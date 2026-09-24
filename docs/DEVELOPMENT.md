# Development and testing

## Repository layout

```text
.
├── Cargo.toml
├── Cargo.lock
├── crates
│	├── nixbox
│	├── nixbox-cli
│	├── nixbox-cmd
│	├── nixbox-config
│	├── nixbox-core
│	├── nixbox-nix
│	└── nixbox-tui
├── docs
├── vm
├── flake.nix
├── flake.lock
├── devenv.nix
├── devenv.yaml
├── devenv.lock
├── justfile
└── .github/workflows/publish.yml
```

The root `Cargo.toml` owns the workspace version, Rust edition, shared metadata, dependencies, and Clippy policy. Each crate inherits that metadata. Internal dependencies in `nixbox-tui`, `nixbox-cmd`, `nixbox`, and `nixbox-cli` include both a local path and the current published version, so a release bump must update the workspace version and every internal dependency version.

## Two binaries, one command tree

The workspace publishes two binaries. They are the same program:

| Package | Binary | Terminal UI |
| --- | --- | --- |
| `nixbox` | `nixbox` | yes |
| `nixbox-cli` | `nixbox-cli` | no |

Both are a four-line `main` over `nixbox-cmd`, which holds the whole command tree. The difference is one line of `Cargo.toml`: `nixbox` enables `nixbox-cmd`'s `tui` feature and `nixbox-cli` does not.

```toml
# crates/nixbox-cmd/Cargo.toml
[features]
default = []
tui = ["dep:nixbox-tui"]
```

They install under different names on purpose. Two packages producing the same executable collide in one `target/` directory, which cargo warns about and may one day reject, and the second `cargo install` would silently replace the first. `nixbox-cmd` learns which name is running from `env!("CARGO_BIN_NAME")`, so the usage line, the completion script, and hints like "run `nixbox-cli apply`" all name the command the user actually has.

With the UI off, `nixbox-tui` and its terminal dependencies are not built at all: `cargo tree -p nixbox-cli` reports 60 packages against 108 for `nixbox`. Keeping that working is a constraint on where code goes — anything the command line needs belongs below the UI crate, not in it. Two things moved down for this reason, each with a test in `nixbox-tui` pinning it to its former home:

- `nixbox_config::THEMES` holds the theme names `nixbox config set theme` validates against. `theme::tests::the_palettes_match_the_names_the_settings_file_accepts` asserts the palettes and the names agree.
- `nixbox_core::state` holds `PersistedState` and `InProgress`, the `state.json` format that `nixbox resume` reads. The UI crate re-exports them.

Cargo unifies features across a workspace build, so `cargo test --workspace` always compiles `nixbox-cmd` with `tui` on and proves nothing about the other configuration. `just cli` builds the two CLI packages on their own to cover it, and `just ci` runs that. For the same reason, `cargo build --workspace` produces a `target/debug/nixbox-cli` that does contain the UI code; build it with `cargo build -p nixbox-cli` (or `just release-cli`) when the distinction matters.

The Nix flake exposes both as `packages.nixbox` and `packages.nixbox-cli`.

## Development environment

The supported development path is the pinned devenv shell:

```sh
devenv shell
```

`devenv.nix` enables nightly Rust with `rustc`, Cargo, Clippy, rustfmt, Rust Analyzer, and Rust source. It also installs Nix, `nixd`, `nil`, and `just`. `devenv.lock` pins the Nix inputs. With direnv installed, `.envrc` can enter this environment automatically after `direnv allow`.

Using the shell matters on NixOS because a Rustup toolchain can retain linker wrappers that point at garbage-collected Nix store paths. If plain Cargo fails inside a Rust linker wrapper while the devenv build succeeds, treat that as a host toolchain problem rather than changing NixBox source.

## Commands

| Command | Purpose |
| --- | --- |
| `just build` | Build the complete workspace in debug mode. |
| `just release` | Build the complete workspace with release optimizations. |
| `just check` | Type-check the workspace without code generation. |
| `just test` | Run the workspace test suite. |
| `just cli` | Lint and test `nixbox-cli` and `nixbox-cmd` on their own, which a workspace build does not cover. |
| `just release-cli` | Build the `nixbox-cli` release binary. |
| `just run` | Start the debug TUI. |
| `just fmt` | Format every crate with rustfmt. |
| `just lint` | Run Clippy for the workspace and deny warnings. |
| `just fix` | Run formatting and linting. Despite the name, Clippy is not invoked with automatic fixes. |
| `just clean` | Remove Cargo build artifacts. |
| `just dev` | Enter `devenv shell`. |
| `just dev-update` | Update pinned devenv inputs. |
| `just dev-test` | Evaluate devenv and run its configured test command. |
| `just ci` | Run format, lint, test, and the CLI-only packages in that order. |
| `just ci-vm` | Run `just ci` and then the automated VM check. |
| `just vm-build` | Build the throwaway NixOS test VM. |
| `just vm` | Build and boot the test VM. |
| `just vm-fresh` | Boot the test VM from a clean disk. |
| `just vm-clean` | Delete the test VM's disk image and build result. |
| `just vm-test` | Run the automated VM check. |

The pre-commit gate is:

```sh
devenv shell -- just ci
```

`devenv test` also runs `just ci` through `enterTest`.

## Tests

The workspace has unit and asynchronous tests in every functional module. The tests cover settings defaults and serialization, package and flake manifest rendering, import insertion, package scanning and removal, search parsing and ranking, lock-file catalog resolution, bounded process output, build cancellation, GitHub result parsing and flake mutation, application epochs and state transitions, keyboard handling, queue batching, recovery serialization, and Unicode-aware Vim motions.

Five tests are ignored during a normal run because they depend on live machine or network state:

- `scan::tests::real_config_detection` reads the maintainer's live Nix configuration paths.
- The ignored Zen Browser flake search test requires authenticated GitHub access and consumes API quota.
- The ignored Bun flake search test requires authenticated GitHub access, Nix evaluation, and GitHub search quota.
- `flakes::tests::for_repo_evaluates_the_flake_it_names` requires authenticated GitHub access and Nix evaluation.
- The ignored live search test requires Nix and a configured nixpkgs registry.

Run ignored tests deliberately and one at a time after reading their source:

```sh
cargo test --workspace -- --ignored --nocapture
```

The project does not contain end-to-end terminal snapshot tests. Unit tests cannot prove that an arbitrary user flake will rebuild successfully, especially where source-text mutation heuristics are involved, which is what the disposable NixOS VM below is for.

## Testing in a VM

NixBox edits a NixOS configuration and runs rebuilds, so the useful way to try a change is against a throwaway machine rather than your own:

```sh
just vm         # build and boot the test VM
just vm-test    # run the automated check
just vm-clean   # delete the VM's disk image and build result
```

The VM logs in as `tester` (password `tester`, passwordless sudo) with NixBox already on `PATH`. Quit QEMU with `Ctrl-a x`. Inside, `~/.config/nixos` is seeded with a git-tracked flake that exposes `nixosConfigurations.nixos` — which is what NixBox rebuilds — and a `configuration.nix` that declares `hello` by hand, so `nixbox scan` and `nixbox migrate` have something to work with. The VM keeps its state in `nixos.qcow2`; delete it for a clean boot.

A handful of settings in `vm/guest.nix` are what make the guest usable, and none of them are optional:

| setting | why |
| --- | --- |
| `virtualisation.writableStore` | Layers a writable overlay over the host's read-only store. Without it nothing inside the VM can rebuild. |
| `virtualisation.writableStoreUseTmpfs = false` | Puts that overlay on disk instead of in RAM, so an inner rebuild does not run the VM out of memory. |
| `virtualisation.qemu.enableSharedMemory` | virtiofs is vhost-user, so the daemon must be able to map the guest's RAM. Without it every virtiofs mount hangs in uninterruptible sleep and the boot strands in stage 1. The NixOS test driver sets this itself, so only the interactive VM ever needed it spelled out. |
| `boot.loader.grub.enable = false` | QEMU is handed the kernel directly, so there is nothing to install. `grub-install` refuses this disk, and it runs before activation, so leaving GRUB on would fail every rebuild after having built the whole system. |
| `nix.settings.flake-registry` | Locking a flake input resolves indirect references against the global registry only, not the system one, so the default would send every rebuild to channels.nixos.org. It points at the same pin `nix.registry` generates. |

`vm/guest.nix` is shared by the outer VM and the copy seeded inside it. A rebuild started in the guest stops any unit the new configuration does not declare, so the two have to agree about `virtualisation` or the switch would unmount the Nix store out from under itself.

`just vm-test` boots the same machine headless and drives it with the NixOS test driver. Those VMs have no network, so it covers what works offline: `status`, `doctor`, `scan`, `migrate`, `list`, `remove`, `apply`, `config`, the confirmation guard, `--dry-run`, and — because the guest registry pins nixpkgs to a store path — `search` and `install` as far as the managed file. A real rebuild is the one thing it cannot do, since anything outside the system closure would have to come from a substituter, so `just vm` is where the `install`/`migrate`/`remove` round trip gets exercised against an actual `nixos-rebuild switch`.

Only `nixos-rebuild build-vm` is ever used here; it builds and never activates, so your own system is untouched. Do not run `nixos-rebuild switch` against `.#nixbox-testvm` — that would apply the test machine's configuration, passwordless sudo and all, to your host.

## Clippy policy

The workspace denies Clippy's `pedantic` and `nursery` groups and adds explicit bans on `unwrap`, `expect`, unchecked indexing and slicing, arithmetic side effects, unreachable code, unimplemented code, unchecked time subtraction, `todo!`, string slicing, panics in result-returning functions, `panic!`, process exit, and unchecked `as` conversions.

`clippy.toml` allows unwrap, expect, panic, and indexing or slicing in tests. Individual source locations may carry narrow lint allowances where a checked invariant cannot be expressed cleanly under the workspace policy.

Run Clippy from the same toolchain as rustc. A Clippy driver from a different Rust version can fail before analyzing the project.

## Logging

The binary writes tracing output to stderr and defaults to the `warn` filter. Set `RUST_LOG` to increase detail:

```sh
RUST_LOG=debug nixbox
```

The current code has limited tracing calls, so the TUI status line and Building tab often contain more operational detail than the debug log.

## Nix package

The repository flake exports:

- `packages.<system>.nixbox` and `packages.<system>.default`.
- `apps.<system>.default`.
- `overlays.default`, which adds `pkgs.nixbox`.

Supported systems are `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`. `buildRustPackage` builds only the `nixbox` package target but runs workspace tests. The source set includes Cargo metadata, the license, README, and all crates. The installed executable is wrapped with `gh`, `git`, and `nix` on `PATH`.

Useful checks are:

```sh
nix flake check
nix build .#nixbox
nix run . -- --version
```

The Cargo-installed binary does not receive the Nix wrapper, so users must provide its external commands themselves.

## Version control

This repository uses Jujutsu with a colocated Git repository. Inspect both the working copy and bookmark state before changing or publishing work:

```sh
jj status
jj bookmark list -a
git status --short
```

Create focused commits and move `dev` to the completed commit when appropriate:

```sh
jj commit -m "docs: describe package catalog"
jj bookmark move dev --to @-
```

Do not describe a local commit as pushed. Compare `dev`, `master`, and their remote bookmarks before release work.

## Release preparation

Published crates must use one version across the workspace. Before merging a release into `master`:

1. Choose the next semantic version and replace `workspace.package.version` in the root manifest.
2. Update every internal dependency version in `crates/nixbox-core/Cargo.toml`, `crates/nixbox-tui/Cargo.toml`, and `crates/nixbox/Cargo.toml`.
3. Regenerate `Cargo.lock` with the pinned development toolchain.
4. Move the current `Unreleased` notes to a dated version heading and add a fresh empty `Unreleased` section.
5. Run `just ci`, `nix flake check`, and a release build.
6. Inspect the Jujutsu diff and commit only the intended release files.
7. Merge the verified `dev` commit into `master` through the project's normal review workflow.

`.github/workflows/publish.yml` runs on pushes to `master` and on pull requests targeting `master`. It contains two jobs.

The `test` job mirrors the local `just ci` gate on the stable toolchain: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked`, followed by the same two checks `just cli` runs and a step that builds each binary package separately and confirms `nixbox-cli` rejects `tui` with `unrecognized subcommand`. It runs on both triggers, so a pull request reports the same gate before the merge happens.

The `publish` job declares `needs: test` and is restricted to `push` events, so it never runs from a pull request and never starts unless the `test` job succeeded. It checks the locked workspace, then publishes `nixbox-config`, `nixbox-nix`, `nixbox-core`, `nixbox-tui`, `nixbox-cmd`, `nixbox`, and `nixbox-cli` in dependency order. A `concurrency` group keeps two pushes from racing up that chain, and never cancels a run in flight, because a half-published chain is worse than a queued one. It treats an already-published version as a skip, so re-running a failed publish is safe and does not require another version bump. Publishing requires the `CARGO_REGISTRY_TOKEN` repository secret; an expired or revoked token fails the upload with `403 Forbidden: authentication failed`.

The workspace denies every `pedantic` and `nursery` lint, and those sets change between Rust releases. The development shell runs nightly while this workflow runs stable, so a lint can fire in one and not the other. Run the gate on stable before a release if the local shell is on a different channel.

The current `dev` manifest says `0.2.4`. Bump it again before the next merge, because publishing skips any version that already exists on crates.io.

Nothing after `0.2.0` reached crates.io before `0.2.3`: the `0.2.1` and `0.2.2` publishes both failed on an expired token, which is why `nixbox-core` had no released version at all despite two attempts. A failed publish leaves no trace on crates.io, so a version that failed to upload can be reused; a version that uploaded cannot, even after a yank.

## Documentation style

Project Markdown keeps each prose paragraph, list item, and table row on one source line. Do not insert hard line breaks to satisfy a column limit. Code blocks retain the line structure required by the code or command examples.

Use sentence-case headings, state exact paths and commands, and document destructive behavior and parser limitations next to the feature that triggers them.
