# NixBox

A NixOS TUI package manager. Search a nixpkgs channel, pick a package, and NixBox writes it into your home-manager or NixOS config and runs the rebuild — without ever leaving the terminal.

## What it does

- Live search against `nix search --json` over a configurable flake input (default `nixpkgs`).
- Maintains two managed files in your config directory — `nixbox-home-packages.nix` and `nixbox-system-packages.nix` — and owns them end-to-end. Your hand-written config is never touched outside of a single `imports` line.
- On install/uninstall it updates the managed file, makes sure it's imported by your `home.nix` / `configuration.nix`, then runs the appropriate rebuild and streams the output into the TUI.
- Works whether your home-manager is exposed as a standalone `homeConfigurations.<user>` flake output, or wired in as a NixOS module — NixBox auto-detects which one you have and picks the right rebuild command.
- Scans your existing config for externally-declared packages and lets you "migrate" them into the managed file with `m` (or `M` for all of them).
- Settings (channel, target, theme, paths) persist in `~/.config/nixbox/settings.json`.

## How it wires itself in

The first time you install or migrate a package, NixBox does three things automatically:

1. Writes the managed file (`nixbox-home-packages.nix` for home-manager, `nixbox-system-packages.nix` for NixOS).
2. Inserts `./nixbox-home-packages.nix` (or `…-system-…`) into the `imports = [ … ]` list of your `home.nix` / `configuration.nix`. Existing imports-list style is preserved, and the insertion is idempotent.
3. Stages the managed file with `git add -N` if your config dir is a git work tree, so flakes (which ignore untracked files) can actually evaluate it.

If you want to override where NixBox looks for the "main" config file, set `home_manager_main_file` or `nixos_main_file` in `~/.config/nixbox/settings.json`.

Inside the managed file, NixBox owns everything between `# nixbox:packages:start` and `# nixbox:packages:end`. Don't edit those by hand.

## Install

```sh
cargo install nixbox
```

NixBox requires a working Nix installation and a configured NixOS or home-manager flake. The `nix` and rebuild commands are executed locally, so make sure the selected flake can be evaluated before installing packages.

### Install from the flake

Add NixBox to your flake inputs:

```nix
inputs.nixbox.url = "github:SINGH-RAJVEER/nix-box";
```

Then add its package to either a NixOS system package list:

```nix
environment.systemPackages = [
  inputs.nixbox.packages.${pkgs.system}.default
];
```

or a Home Manager package list:

```nix
home.packages = [
  inputs.nixbox.packages.${pkgs.system}.default
];
```

If your configuration uses overlays, apply the included overlay and refer to the package as `pkgs.nixbox`:

```nix
nixpkgs.overlays = [ inputs.nixbox.overlays.default ];

environment.systemPackages = [ pkgs.nixbox ];
# or: home.packages = [ pkgs.nixbox ];
```

You can also try it without installing it:

```sh
nix run github:SINGH-RAJVEER/nix-box
```

Or build from source:

```sh
devenv shell
cargo build --release
```

## Run

```sh
nixbox          # if cargo-installed
devenv shell -- just run  # from a checkout
```

NixBox stores its settings and managed package files separately from this repository. By default, settings are written to `~/.config/nixbox/settings.json`; paths and the active target can be changed from the settings screen with `Ctrl-S`.

## Development

The reproducible development environment is managed by [devenv](https://devenv.sh/):

```sh
devenv shell  # Rust toolchain, Nix tooling, and just
just ci       # format, lint, and test the workspace
devenv test   # evaluate the environment and run just ci
devenv update # update pinned inputs
```

With direnv installed, run `direnv allow` once to activate the environment automatically.

## Layout

Cargo workspace:

- `crates/nixbox` — binary entrypoint
- `crates/nixbox-tui` — ratatui app, search / installed / build views
- `crates/nixbox-nix` — `nix search` wrapper, managed-file writer, import inserter, rebuild runner
- `crates/nixbox-config` — persisted user settings (channel, target, theme, input mode, path overrides)

## Keys

| key                   | action                                      |
| --------------------- | ------------------------------------------- |
| `/` / `i` / `a`       | enter insert mode in a search bar           |
| `v`                   | enter visual mode in a search bar           |
| `h` `l` / `b` `w`     | move by character / word                    |
| `B` `W` / `e` `E`       | move by WORD / to end of word               |
| `0` / `$`             | move to start / end                          |
| `x` / `D` / `dd`      | delete character / to end / whole line      |
| `d` / `x` / `c`       | delete or change a visual selection         |
| `↑` `↓` / `k` `j`     | move package selection                      |
| Enter                 | install selected package                    |
| `d` / Delete          | uninstall selected (Installed tab)          |
| `m` / `M`             | migrate selected / all external packages    |
| `c`                   | cancel active build (Building tab)          |
| Tab                   | next tab (Search → Installed → Build)       |
| Shift-Tab             | previous tab                                |
| Ctrl-S                | open settings                               |
| Esc / Ctrl-C          | return to normal mode / quit                |
