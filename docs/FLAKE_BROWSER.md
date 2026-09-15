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

Candidates deduplicate by repository and receive an initial score. NixBox keeps the best 12 candidates, evaluates them concurrently with `nix flake show --json --no-write-lock-file github:<owner>/<repository>`, and gives each evaluation 15 seconds. It records top-level output names, package nodes whose reported type is `derivation`, and conventional default NixOS and Home Manager modules. A candidate is dropped when evaluation fails, times out, or finds none of those outputs. Successful inspections are cached in memory for the rest of the process.

Initial ranking favors an upstream reference extracted from code, then repository-search matches, then the repository that contained a code match. Within those groups, an exact normalized repository-name match ranks above a prefix or substring match. Repository search adds a small star-count contribution capped at 10,000 stars. After evaluation, a matching package attribute or package name adds another score contribution. Package ordering favors the `default` attribute, then exact, prefix, and substring matches, then version and attribute name. The final result list is capped at 20, although at most 12 inspected candidates can reach it in the current implementation.

Normalization removes non-alphanumeric characters and lowercases the remaining text. A query for `zen-browser` can therefore match a repository named `zen_browser`.

## Detail inspection

Selecting a result fetches repository metadata and raw `flake.nix` concurrently. The detail panel shows description, stars, archived state, default branch, last push time, topics, homepage, repository URL, detected inputs, evaluated output categories, and up to four derivation packages with their attribute, name, and version.

Input detection is textual: it reads identifier-like names from the first balanced block after `inputs`, so unusual input construction can be missed. Package and module detection comes from the `nix flake show` JSON produced during search. Output labels combine those evaluated package and module results with the top-level attribute names returned for `overlays`, `devShells`, `apps`, and `formatter`.

Search results and details remain in memory only. NixBox does not clone repositories or persist flake-browser results.

## Output installation

Pressing `Enter` chooses one output for the active target:

| Priority | Active target | Condition | Generated expression |
| --- | --- | --- | --- |
| 1 | Home Manager | Evaluation finds `homeManagerModules.default` or `homeModules.default`. | The exact detected path under `inputs."<owner>/<repository>"`. |
| 1 | NixOS | Evaluation finds `nixosModules.default`. | `inputs."<owner>/<repository>".nixosModules.default`. |
| 2 | Either | No matching default module exists, but the flake has packages for the current system. | The first ranked `inputs."<owner>/<repository>".packages.${pkgs.system}."<attribute>"`. |

For Home Manager, the evaluator preserves whether the repository exports `homeManagerModules.default` or `homeModules.default`, and installation writes that exact path. When installing a package, the first evaluated package is already sorted to favor the `default` attribute, then exact, prefix, and substring query matches.

The queued installer performs these mutations:

1. Read `<configuration-root>/flake.nix`.
2. Require the outputs function to contain `outputs = inputs@` so the complete input set has a stable binding.
3. Add `"<owner>/<repository>".url = "github:<owner>/<repository>";` to the root `inputs = { ... };` block unless an input with that exact quoted key already has a URL.
4. Find `homeManagerConfiguration` or `nixosSystem` and add `extraSpecialArgs = { inherit inputs; };` or `specialArgs = { inherit inputs; };` unless the source already contains that setting name.
5. Add the selected module path or package attribute to `nixbox-home-flakes.nix` or `nixbox-system-flakes.nix`.
6. Ensure the target entry module imports the generated flake module.
7. Mark `flake.nix`, the generated module, and the target entry file with `git add --intent-to-add` when they are inside a Git work tree.
8. Run the target rebuild through the normal queue.

The generated Home Manager flake module has this shape when one module and one package have been selected in separate operations:

```nix
# Managed by nixbox. Do not edit by hand.
{ inputs, pkgs, ... }:
{
	imports = [
		# nixbox:flakes:start
		inputs."owner/repository".homeManagerModules.default
		# nixbox:flakes:end
	];
	home.packages = [
		# nixbox:flake-packages:start
		inputs."owner/package".packages.${pkgs.system}."default"
		# nixbox:flake-packages:end
	];
}
```

The quoted owner and repository string is also the key in the root flake inputs set. Quoted attribute names containing `/` are valid Nix.

## Required root-flake shape

The text editor expects conventional literal text similar to:

```nix
{
	inputs = {
		nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
	};

	outputs = inputs@{ self, nixpkgs, ... }: {
		nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
			specialArgs = { inherit inputs; };
			modules = [ ./configuration.nix ];
		};
	};
}
```

The installer searches for literal strings such as `inputs = {`, `outputs = inputs@`, `nixosSystem`, and `homeManagerConfiguration`. It does not parse Nix strings or comments while finding braces, and it does not understand helper functions that move constructor arguments elsewhere. Commit the configuration before installation and inspect the diff afterward.

## Current limitations

- There is no flake-output uninstall command. Remove the generated mapping and root input through a controlled code change or manual configuration edit.
- Only GitHub repositories are discoverable and installable in this tab.
- Only root `flake.nix` files qualify.
- Only default NixOS and Home Manager module outputs can be installed; named non-default modules cannot be selected.
- The UI automatically chooses the first ranked package when no target-compatible default module exists; it does not let the user select another package attribute.
- Search and metadata use GitHub API quota from the authenticated `gh` account.
- Package inspection uses the derivation metadata that `nix flake show` reports for the current system. Outputs for other systems appear as empty objects and do not qualify.
- Search evaluates up to 12 remote flakes concurrently. Uncached searches can therefore take as long as the slowest successful evaluation, and failed candidates are omitted instead of reported individually.
- Root-flake mutation is not transactional and has no automatic backup or rollback.
