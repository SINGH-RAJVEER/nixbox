# GitHub flake browser

## Purpose and requirements

The Flakes tab finds GitHub repositories with a root `flake.nix`, shows repository and evaluated flake properties, and can wire a default NixOS or Home Manager module or a package output into the target configuration.

NixBox delegates authentication and API access to the GitHub CLI. Install `gh` and authenticate before opening the tab:

```sh
gh auth login
gh auth status
```

The Nix package exported by this repository wraps `nixbox` with `gh`, `git`, and `nix` on `PATH`. A Cargo installation relies on those commands already being available.

## Search pipeline

After 250 ms without another input edit, NixBox runs GitHub code search and repository search concurrently through `gh api`.

Code search looks for the user's query in files named `flake.nix` at repository root and asks GitHub for text-match fragments. Repository search requires both the user's query and the word `flake` in repository names, descriptions, or topics while excluding archived repositories. This keeps dedicated project flakes ahead of same-named repositories and unrelated projects that merely use the queried package. Each GitHub request asks for up to 20 matches.

Text fragments are also scanned for `github:<owner>/<repository>` references. This lets a consumer configuration point NixBox toward the upstream project flake. Branch or path suffixes after the first owner and repository segments are discarded for candidate identity.

Candidates deduplicate by repository and receive an initial score. NixBox keeps the best 12 candidates, resolves each repository's `HEAD` commit through GitHub, and evaluates that locked revision with a pure `nix eval`. Normal candidates receive 15 seconds. Upstream references extracted from flake source receive 45 seconds because large canonical flakes can take longer on a cold Nix cache. Evaluation records top-level output names, derivation package attributes under `packages.<current-system>`, and conventional default NixOS and Home Manager modules. A candidate is dropped when evaluation fails, times out, or finds none of those outputs. Successful inspections are cached in memory for the rest of the process.

Initial ranking favors an upstream reference extracted from code, then repository-search matches, then the repository that contained a code match. Within those groups, an exact normalized repository-name match ranks above a prefix or substring match. Repository search adds a small star-count contribution capped at 10,000 stars. After evaluation, a matching package attribute adds another score contribution. Package ordering favors the `default` attribute, then exact, prefix, and substring matches, then the attribute name. The final result list is capped at 20, although at most 12 inspected candidates can reach it in the current implementation.

Normalization removes non-alphanumeric characters and lowercases the remaining text. A query for `zen-browser` can therefore match a repository named `zen_browser`.

## Detail inspection

Selecting a result fetches repository metadata and raw `flake.nix` concurrently. The detail panel shows description, stars, archived state, default branch, last push time, topics, homepage, repository URL, detected inputs, evaluated output categories, and up to four derivation package attributes.

Input detection is textual: it reads identifier-like names from the first balanced block after `inputs`, so unusual input construction can be missed. Package and module detection comes from pure evaluation of the locked GitHub revision. Output labels combine those evaluated package and module results with top-level attribute names for `overlays`, `devShells`, `apps`, and `formatter`.

Search results and details remain in memory only. NixBox does not clone repositories or persist flake-browser results.

## Output installation

Pressing `Enter` chooses one output for the active target:

| Priority | Active target | Condition | Generated expression |
| --- | --- | --- | --- |
| 1 | Home Manager | The flake has packages for the current system. | The first ranked `inputs.<input>.packages.${pkgs.stdenv.hostPlatform.system}.<attribute>`, added to `home.packages`. |
| 2 | Home Manager | No packages, but evaluation finds `homeManagerModules.default` or `homeModules.default`. | The exact detected module path under `inputs.<input>`. |
| 1 | NixOS | Evaluation finds `nixosModules.default`. | `inputs.<input>.nixosModules.default`. |
| 2 | NixOS | No default module, but the flake has packages for the current system. | The first ranked package, added to `environment.systemPackages`. |

Home Manager prefers packages because a Home Manager module installs nothing until its options are set, while a package in `home.packages` is usable after one rebuild. The first evaluated package is already sorted to favor the `default` attribute, then exact, prefix, and substring query matches.

The queued installer performs these mutations:

1. Read `<configuration-root>/flake.nix` and require the outputs function to bind the whole input set as `inputs`, written as `outputs = inputs@{ ... }:`, `outputs = { ... }@inputs:`, or `outputs = inputs:`.
2. Look for an existing root input whose URL is `github:<owner>/<repository>`, compared case-insensitively and ignoring any branch or query suffix. If one exists, its name is reused and `flake.nix` is left alone for this step.
3. Otherwise add a new input under a short name derived from the repository: a `.nix` suffix, a `-flake` suffix, and a `nix-flake-` or `flake-` prefix are dropped, so `0xc000022070/zen-browser-flake` becomes `zen-browser` and `numtide/llm-agents.nix` becomes `llm-agents`. When that name is taken by another repository, `<owner>-<name>` is used. When the root flake has a `nixpkgs` input, the new input follows it. The entry copies the indentation and blank-line spacing of the existing inputs, and both the `inputs = { ... };` block and root-level `inputs.<name>.url = ...;` layouts are supported.
4. Make `inputs` reachable from the target's modules:
	- NixOS: ensure `nixosSystem` has `specialArgs` that inherits `inputs`.
	- Home Manager with a standalone `homeManagerConfiguration`: ensure its `extraSpecialArgs` inherits `inputs`.
	- Home Manager as a NixOS module (`home-manager.nixosModules.home-manager`): ensure `nixosSystem` has `specialArgs` with `inputs`, then ensure `home-manager.extraSpecialArgs` inherits `inputs`, either in `flake.nix` or in the NixOS entry file (usually `configuration.nix`). When it is missing, it is added to the `home-manager = { ... };` block or next to `home-manager.users`, and `inputs` is added to that file's module arguments.
	An existing setting that lacks `inputs` gets `inherit inputs;` added to it. Settings bound to something other than a literal set are left alone.
5. Add the selected module path or package attribute to `nixbox-home-flakes.nix` or `nixbox-system-flakes.nix`.
6. Ensure the target entry module imports the generated flake module.
7. Mark every touched file with `git add --intent-to-add` when it is inside a Git work tree.
8. Run the target rebuild through the normal queue.

For a configuration that wires `inputs` into Home Manager through the NixOS module and lists flake packages in `home.nix`, the result matches what a hand edit would produce. Installing `oxcl/nix-flake-helium-browser` adds this to `flake.nix`:

```nix
helium-browser = {
	url = "github:oxcl/nix-flake-helium-browser";
	inputs.nixpkgs.follows = "nixpkgs";
};
```

The generated Home Manager flake module then has this shape when one module and one package have been selected in separate operations:

```nix
# Managed by nixbox. Do not edit by hand.
{ inputs, pkgs, ... }:
{
	imports = [
		# nixbox:flakes:start
		inputs.some-module.homeManagerModules.default # github:owner/some-module
		# nixbox:flakes:end
	];
	home.packages = [
		# nixbox:flake-packages:start
		inputs.helium-browser.packages.${pkgs.stdenv.hostPlatform.system}.default # github:oxcl/nix-flake-helium-browser
		# nixbox:flake-packages:end
	];
}
```

The trailing comment records which repository each line belongs to, so the input can be found again when the flake is removed.

## Duplicate outputs

Installing a package that your target entry file already declares, through the same input, changes nothing: NixBox reports that the package is already declared and does not add a second copy to the generated module. Because an existing input for the repository is always reused, the two declarations would name the same package anyway.

## Output removal

The Installed tab lists every flake output in the configuration and removes one at a time: see the Installed section of the user guide. The package line is deleted from NixBox's generated module, from your entry file, or from both when both declare it.

`nixbox flake remove <owner>/<repository>` drops all of the repository's lines from the generated module at once.

Either way, NixBox then removes the root input only when nothing else uses it: the input stays when the flake's outputs mention it, another input `follows` it, or any `.nix` file under the configuration root other than `flake.nix` still references `inputs.<input>`. An input you declared yourself and also use in `home.nix` therefore survives removal of the nixbox-managed copy.

## Required root-flake shape

The text editor expects conventional literal text similar to:

```nix
{
	inputs = {
		nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
	};

	outputs = { self, nixpkgs, ... }@inputs: {
		nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
			specialArgs = { inherit inputs; };
			modules = [ ./configuration.nix ];
		};
	};
}
```

The installer reads the root attribute set of `flake.nix` with a small scanner that skips strings and comments, and searches for the literal constructor names `nixosSystem` and `homeManagerConfiguration`. It does not understand helper functions that move constructor arguments elsewhere. Commit the configuration before installation and inspect the diff afterward.

## Current limitations

- The Flakes tab has no uninstall key. Remove flake outputs from the Installed tab or with `nixbox flake remove`.
- Only flake packages are picked up from your own entry files. Module imports you wrote yourself, such as `inputs.zen-browser.homeModules.twilight`, are not listed, because removing one usually leaves options behind that no longer exist.
- Only GitHub repositories are discoverable and installable in this tab.
- Only root `flake.nix` files qualify.
- Only default NixOS and Home Manager module outputs can be installed; named non-default modules cannot be selected. On Home Manager, a module is only chosen when the flake has no packages.
- The UI automatically chooses the first ranked package when no target-compatible default module exists; it does not let the user select another package attribute.
- Search and metadata use GitHub API quota from the authenticated `gh` account.
- Package inspection evaluates only `packages.<current-system>` and rejects entries whose `type` is not `derivation`.
- Search evaluates up to 12 remote flakes concurrently. Uncached searches can therefore take as long as the slowest successful evaluation, and failed candidates are omitted instead of reported individually.
- Root-flake mutation is not transactional and has no automatic backup or rollback.
