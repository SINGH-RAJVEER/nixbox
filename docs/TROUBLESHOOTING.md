# Troubleshooting

## Start with the supported environment

When a build, linker, Clippy, or dependency problem appears in a checkout, reproduce it with the pinned environment before changing source:

```sh
devenv shell -- cargo build --workspace
devenv shell -- just ci
```

If the command succeeds there and fails with plain Cargo, the host Rust toolchain or environment is the likely cause.

## Rust linker wrapper points to a missing Nix store path

An error such as `ld.lld: ... /nix/store/.../nix-support/ld-wrapper.sh: No such file or directory` means the active Rustup toolchain contains a linker wrapper tied to a Nix store path that no longer exists. It is not a Rust source or NixBox linker-flag error when the same build succeeds in devenv.

Use the repository toolchain immediately:

```sh
nix develop --command cargo run
```

or:

```sh
devenv shell
cargo run
```

Repairing plain Cargo usually requires reinstalling the affected Rustup toolchain after the current Nix-provided Rustup is active. Removing a toolchain is destructive, so inspect `rustup show active-toolchain` and preserve required components before doing it.

## Package catalog is unavailable

The status line includes the catalog error and says live search will be used. Check the target configuration root and lock structure:

```sh
test -f "${NIXBOX_CONFIG_DIR:-$HOME/.config/nixos}/flake.lock"
jq '.root, .nodes[.root].inputs.nixpkgs' "${NIXBOX_CONFIG_DIR:-$HOME/.config/nixos}/flake.lock"
```

The root must have a direct `nixpkgs` input whose locked node contains `type`, `owner`, `repo`, and `rev`. Only `github` and `gitlab` locked types are supported by the catalog builder. Other layouts fall back to the channel in settings.

To force a clean catalog rebuild, move the cache aside rather than deleting it immediately:

```sh
mv ~/.cache/nixbox/package-catalog.json ~/.cache/nixbox/package-catalog.json.backup
```

The next start will evaluate the full locked package set.

## Search is slow again

Check the status line. `Preparing package catalog` means NixBox is performing the one-time full evaluation or reloading the cache. `Package catalog unavailable; using live search` means every query starts `nix search` and inherits its variable latency.

Common causes are a changed `flake.lock`, a missing direct root nixpkgs input, an unsupported lock type, an unreadable cache directory, a catalog output larger than 256 MiB, or a Nix evaluation failure. Run the locked full search directly to expose the Nix error:

```sh
nix --quiet search --json --extra-experimental-features "nix-command flakes" <locked-flake-reference> '^' >/dev/null
```

Replace `<locked-flake-reference>` with the exact GitHub or GitLab revision from the error or lock file. Do not use the moving unstable branch when testing a catalog tied to a revision.

## Search results changed after a lock update

This is expected. NixBox invalidates the catalog when the exact locked nixpkgs revision changes. It postpones package installation until the replacement catalog is ready so a selected attribute comes from the same revision the target will evaluate.

## GitHub flake search fails

Verify the GitHub CLI and authentication:

```sh
command -v gh
gh auth status
gh api user --jq .login
```

The flake browser consumes GitHub code-search and repository-search quota. API errors, insufficient scopes, rate limits, and network failures appear in the status line. Package search does not depend on `gh`.

## A known flake does not appear

The repository must have a readable `flake.nix` at its root and expose at least one derivation package for the current system or a conventional default NixOS or Home Manager module. NixBox evaluates only the 12 strongest GitHub candidates at their resolved GitHub revision. Normal candidates have a 15-second evaluation timeout; upstream `github:` references found in other root flakes have 45 seconds. Candidates that fail evaluation or exceed the timeout are dropped. Search ranking favors repository-name matches and upstream references. Repositories with only a nested flake are excluded. GitHub indexing delay can also prevent a recent file from appearing in code search, although repository search can still find the repository.

## Flake details show the wrong capabilities

NixBox uses a pure, locked `nix eval` expression to inspect top-level output names, derivation package attributes for the current system, and conventional default module attributes. Input names are still inferred from source text. An evaluation failure removes the repository from the results. Inspect the current-system packages when the result differs from expectation:

```sh
nix eval --json 'github:<owner>/<repository>#packages.<system>' --apply builtins.attrNames
```

## Flake installation says the root must bind `inputs`

The installer requires the outputs function to bind the full input set as `inputs`. Any of these forms work:

```nix
outputs = inputs@{ self, nixpkgs, ... }: { };
outputs = { self, nixpkgs, ... }@inputs: { };
outputs = inputs: { };
```

## Flake installation cannot find inputs or a constructor

The text editor looks for the root `inputs` set (either `inputs = { ... };` or root-level `inputs.<name>` bindings), then `homeManagerConfiguration` or `nixosSystem`. When Home Manager runs as a NixOS module, it also needs a `home-manager = { ... };` block or a `home-manager.users` line in the NixOS entry file to attach `extraSpecialArgs` to. Helper functions, inherited input sets, different constructor spellings, or arguments built in another file are outside the current installer. Do not reshape a working flake merely to satisfy automation unless the new structure is acceptable on its own. Add the input, special arguments, generated-module import, and module path manually instead.

## A Home Manager flake falls back to nixos-rebuild

NixBox evaluates whether the root flake has `homeConfigurations.<USER>`. Missing output, a username mismatch, or any evaluation failure returns false and selects `sudo nixos-rebuild switch`. Check the exported names:

```sh
nix eval --json .#homeConfigurations --apply builtins.attrNames
printf '%s\n' "$USER"
```

If Home Manager is intentionally integrated into `nixosConfigurations`, the fallback is correct.

## A managed file exists but the rebuild cannot see it

Git-backed flakes ignore untracked paths. NixBox runs `git add --intent-to-add` for changed generated and entry files, but that can fail if Git is missing, permissions block the index, or the path is outside the discovered work tree.

Inspect the target repository and add the file explicitly:

```sh
git -C ~/.config/nixos status --short
git -C ~/.config/nixos add --intent-to-add -- nixbox-home-packages.nix
```

Use the correct generated filename and configuration root for the active target.

## NixBox could not add the generated import

The generated module remains on disk, but the main entry file was missing or its structure did not match the text editor. Add the import manually:

```nix
{
	imports = [
		./nixbox-home-packages.nix
	];
}
```

Use `nixbox-system-packages.nix`, `nixbox-home-flakes.nix`, or `nixbox-system-flakes.nix` for the corresponding operation.

## An external package is marked inline and cannot migrate

NixBox detects packages in same-line lists but refuses to remove one token automatically. Put each package on its own line, restart NixBox or complete another successful rebuild to trigger a rescan, then migrate it:

```nix
home.packages = with pkgs; [
	ripgrep
	fd
];
```

Complex expressions remain unsupported even when moved to a dedicated line. Keep those expressions in a hand-written module.

## A rebuild failed but files still changed

This is expected under the current operation model. NixBox writes manifests and source migrations before starting the rebuild and does not roll them back. Inspect the configuration repository diff, correct the Nix error, and rebuild again. If the change should be abandoned, restore the relevant files through version control rather than editing only the generated marker block.

## A cancelled or crashed operation returns after restart

An interrupted rebuild is stored in `~/.config/nixbox/state.json` and resumes because its file changes were already written. Cancelling through the Building tab clears the in-progress descriptor and pauses queued work. Killing the process externally may leave the descriptor, causing the next launch to retry.

Inspect the file if recovery does not match expectations:

```sh
jq . ~/.config/nixbox/state.json
```

Move it aside only after checking that the target files and system state are consistent:

```sh
mv ~/.config/nixbox/state.json ~/.config/nixbox/state.json.backup
```

## The terminal remains in raw mode

A hard kill can prevent terminal cleanup. Restore a usable shell with:

```sh
reset
```

If ordinary errors leave the terminal broken, that is a cleanup bug because `run()` is designed to restore raw mode and the alternate screen after the event loop returns.

## Increase logging

Run with a tracing filter:

```sh
RUST_LOG=debug nixbox
```

Tracing goes to stderr. The Building tab separately shows rebuild stdout and stderr and retains only the newest 1,000 lines.
