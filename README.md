# NixBox

NixBox is a terminal package manager for NixOS and Home Manager. It searches the nixpkgs revision locked by your configuration, writes selected packages into generated Nix modules, imports those modules into your configuration, and runs the matching rebuild command.

## What it does

- Builds and caches a searchable package catalog from the direct `nixpkgs` input in your configuration's `flake.lock`.
- Installs packages into separate generated modules for Home Manager and NixOS.
- Finds supported package declarations in existing configuration files and can move simple declarations into NixBox management.
- Searches GitHub for repositories with a root `flake.nix` and can install a default NixOS or Home Manager module, or a package output, from a conventional flake.
- Queues changes by target, combines changes that can share a rebuild, streams build output, and restores interrupted work after restart.
- Supports normal text input or Vim-style Normal, Insert, and Visual modes.

## Requirements

- A working Nix installation with flakes enabled.
- A flake-based NixOS or Home Manager configuration. NixBox looks in `~/.config/nixos` unless `NIXBOX_CONFIG_DIR` changes that location.
- `sudo` access for NixOS rebuilds.
- An authenticated GitHub CLI session from `gh auth login` if you use the flake browser.

## Install

Install the published crate:

```sh
cargo install nixbox
```

Run NixBox directly from its flake:

```sh
nix run github:SINGH-RAJVEER/nix-box
```

Add NixBox to another flake:

```nix
{
	inputs.nixbox.url = "github:SINGH-RAJVEER/nix-box";

	outputs = inputs@{ self, nixpkgs, nixbox, ... }: {
		nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
			specialArgs = { inherit inputs; };
			modules = [
				./configuration.nix
				({ pkgs, ... }: {
					environment.systemPackages = [ nixbox.packages.${pkgs.system}.default ];
				})
			];
		};
	};
}
```

The flake also exports `overlays.default`, which adds `pkgs.nixbox`:

```nix
nixpkgs.overlays = [ inputs.nixbox.overlays.default ];
environment.systemPackages = [ pkgs.nixbox ];
```

Supported flake package systems are `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`.

## Run

```sh
nixbox
```

The binary accepts Clap's generated `--help` and `--version` flags. It has no headless subcommands; a normal invocation starts the TUI.

## First run

NixBox loads both generated package modules, scans the configured Home Manager and NixOS entry files, restores queued work from `~/.config/nixbox/state.json`, and starts preparing the package catalog. If the catalog cache does not match the locked nixpkgs revision, Nix evaluates the full package set once. Later searches use the cache without starting Nix.

Installing a package writes a generated module before the rebuild starts. NixBox does not roll back that file if the rebuild fails, so the generated module continues to describe the requested state and the failure remains visible on the next launch.

## Documentation

- [Documentation index](docs/README.md)
- [User guide](docs/USER_GUIDE.md)
- [Configuration and stored state](docs/CONFIGURATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Package search](docs/SEARCH.md)
- [Managed files and package operations](docs/MANAGED_FILES.md)
- [GitHub flake browser](docs/FLAKE_BROWSER.md)
- [Development and testing](docs/DEVELOPMENT.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Release notes](docs/RELEASE_NOTES.md)

## Development

Use the pinned devenv shell so Cargo, Rust, Nix, and Clippy come from one toolchain:

```sh
devenv shell
just ci
```

See the [development guide](docs/DEVELOPMENT.md) for repository layout, commands, tests, lint policy, Nix packaging, Jujutsu usage, and release preparation.

## License

NixBox is licensed under Apache-2.0. See [LICENSE](LICENSE).
