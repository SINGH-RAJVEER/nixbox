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

The terminal UI is an optional feature. For the command line alone, which pulls in 59 dependency packages instead of 107:

```sh
cargo install nixbox --no-default-features
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

The flake exposes the CLI-only build as `packages.nixbox-cli`, alongside the default `packages.nixbox`.

The flake also exports `overlays.default`, which adds `pkgs.nixbox` and `pkgs.nixbox-cli`:

```nix
nixpkgs.overlays = [ inputs.nixbox.overlays.default ];
environment.systemPackages = [ pkgs.nixbox ];
```

Supported flake package systems are `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`.

## Run

```sh
nixbox
```

With no subcommand this starts the TUI. Every operation it performs is also a subcommand, so the same work can be scripted or run over SSH. A build without the `tui` feature has the subcommands and no TUI; there, a bare `nixbox` prints help.

## First run

NixBox loads both generated package modules, scans the configured Home Manager and NixOS entry files, restores queued work from `~/.config/nixbox/state.json`, and starts preparing the package catalog. If the catalog cache does not match the locked nixpkgs revision, Nix evaluates the full package set once. Later searches use the cache without starting Nix.

Installing a package writes a generated module before the rebuild starts. NixBox does not roll back that file if the rebuild fails, so the generated module continues to describe the requested state and the failure remains visible on the next launch.

## Documentation

- [Documentation index](docs/README.md)
- [User guide](docs/USER_GUIDE.md)
- [Command line interface](docs/CLI.md)
- [Configuration and stored state](docs/CONFIGURATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Package search](docs/SEARCH.md)
- [Managed files and package operations](docs/MANAGED_FILES.md)
- [GitHub flake browser](docs/FLAKE_BROWSER.md)
- [Development and testing](docs/DEVELOPMENT.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Release notes](docs/RELEASE_NOTES.md)

## CLI

Running `nixbox` with no arguments opens the TUI. Every subcommand does the same work headlessly, against the same managed files.

```sh
nixbox search ripgrep            # search the configured channel
nixbox install ripgrep           # add it, wire the import, rebuild
nixbox remove ripgrep            # drop it and rebuild
nixbox list                      # what NixBox manages
nixbox scan                      # packages you declared by hand
nixbox migrate htop              # move one of them into the managed file
nixbox apply                     # rewrite the managed file and rebuild
nixbox resume                    # finish an interrupted rebuild or a queued batch
nixbox status                    # target, paths, and how they are wired
nixbox doctor                    # check what NixBox depends on
```

Flake modules work the same way:

```sh
nixbox flake search nix-index    # needs `gh auth login`
nixbox flake info Mic92/nix-index-database
nixbox flake add Mic92/nix-index-database
nixbox flake list
nixbox flake remove Mic92/nix-index-database
```

Settings can be read and changed without opening the TUI:

```sh
nixbox config show
nixbox config set channel nixpkgs-unstable
nixbox config path
nixbox completions fish > ~/.config/fish/completions/nixbox.fish
```

### Flags

| flag                 | effect                                                          |
| -------------------- | --------------------------------------------------------------- |
| `-t`, `--target`     | act on `home-manager` or `nixos` for this invocation only        |
| `-c`, `--channel`    | use another channel for this invocation only                     |
| `--json`             | machine-readable output instead of a table                       |
| `--dry-run`          | print the plan and stop, writing nothing                         |
| `--no-rebuild`       | write the configuration but skip the rebuild                     |
| `-y`, `--yes`        | do not ask before changing the configuration                     |

Anything that changes your configuration asks first. Without a terminal to ask at, it refuses unless you pass `--yes`, so a script can never trigger a rebuild by accident. `--target` and `--channel` never touch `settings.json` — only `nixbox config set` does.

The configuration is always written before the rebuild starts. If a rebuild fails or you interrupt it with Ctrl-C, your files already hold the change: fix the problem and run `nixbox apply` to finish.

Both front-ends share `~/.config/nixbox/state.json`. `nixbox resume` picks up a rebuild that was killed partway, or applies a batch you queued in the TUI and never ran; `--dry-run` shows it first and `--discard` throws it away.

### Exit codes

| code | meaning                                                      |
| ---- | ------------------------------------------------------------ |
| 0    | success                                                       |
| 1    | NixBox could not do what you asked, or the run was cancelled  |
| 2    | the command line could not be parsed                          |
| 3    | the configuration was written but the rebuild failed          |

Structured output goes to stdout and progress to stderr, so `nixbox list --json | jq` works while a rebuild is streaming.

## Development

Use the pinned devenv shell so Cargo, Rust, Nix, and Clippy come from one toolchain:

```sh
devenv shell
just ci
```

See the [development guide](docs/DEVELOPMENT.md) for repository layout, commands, tests, lint policy, Nix packaging, Jujutsu usage, and release preparation.

## License

NixBox is licensed under Apache-2.0. See [LICENSE](LICENSE).
