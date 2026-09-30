# Desktop GUI

`nixbox-gui` is a desktop window over the same engine as the terminal UI. It reads and writes the same settings, the same managed files, and the same `state.json`, so work queued in one front-end can be resumed in another.

## Installing

The GUI is Linux only for now. Install it from the repository flake to get its native libraries and runtime wrapper without setting up a build environment:

```sh
nix profile install github:SINGH-RAJVEER/nixbox#nixbox-gui
```

The flake package includes a NixBox desktop entry and icon. NixOS and Home Manager installs expose it through their application menus, and a `nix profile` install exposes it in desktops that discover the profile's `share/applications` directory. The entry launches the wrapped GUI binary from its Nix store path.

You can also run it without installing it into your profile:

```sh
nix run github:SINGH-RAJVEER/nixbox#nixbox-gui
```

The crates.io package installs a small launcher that runs the matching tagged flake release. It needs Nix with flakes enabled and fetches or builds the GUI on first run, but it does not need graphics development libraries in your Cargo environment:

```sh
cargo install nixbox-gui
nixbox-gui
```

To compile the native GUI from a checkout, use `devenv shell -- cargo run -p nixbox-gui --features native`. The native build needs `pkg-config` and the Wayland, X11, xkbcommon, Vulkan, fontconfig, and freetype development libraries; the repository's development shell supplies them.

To install it through your own flake, add `inputs.nixbox.url = "github:SINGH-RAJVEER/nixbox";` and include `nixbox.packages.${pkgs.stdenv.hostPlatform.system}.nixbox-gui` in `environment.systemPackages` or `home.packages`. Here `nixbox` is the input available in the enclosing `outputs` function. The package wraps the binary with `gh`, `git`, and `nix` on `PATH`, and the Vulkan loader, Wayland, xkbcommon, and X11 libraries on `LD_LIBRARY_PATH`. It runs on Wayland and X11 and needs a working Vulkan driver.

## Pages

The sidebar lists the pages. The status line at the bottom reports the last thing that happened, and shows the last failed rebuild until you dismiss it.

| Page | What it does |
| --- | --- |
| nixpkgs | Searches the package catalog for your locked nixpkgs revision, falling back to live `nix search` while the catalog is built. Each hit shows where it is already installed and has an Install button. |
| Flakes | Searches GitHub for flakes. Selecting a hit loads its packages, modules, and inputs. Clicking a hit with multiple installable outputs opens a list of exact package and compatible module paths; clicking a path installs that output. A single compatible output can be installed directly. |
| Installed | Packages nixbox manages, flake outputs, and packages declared in your own config, with a filter. Managed packages and flake outputs can be removed; hand-declared packages can be migrated one at a time or all at once. Options on a managed or hand-declared package opens its Options page. |
| Options | One package's module options, read from your configuration, with editors for the simple types. Staged changes are applied as one queued op. See [Package options](PACKAGE_OPTIONS.md). |
| Queue | The running rebuild and every op waiting behind it. An op can be dropped before it runs, and Apply now restarts a queue left paused by a cancelled rebuild. |
| Build | The output of the current or most recent rebuild, with a Cancel button while one runs. The sidebar shows a spinner while a rebuild runs, and a notification appears when it ends. |
| Settings | Install target, channel, and theme, saved immediately to `~/.config/nixbox/settings.json`. |

Ctrl-F focuses the search box of the current page, or the nixpkgs search. Ctrl-Q quits.

Installing, removing, migrating, and applying option changes all queue an op. Ops for one target are written and rebuilt together; the other target's ops wait until that rebuild ends. This is the same `Session` the TUI uses, so the behavior described in [Managed files and package operations](MANAGED_FILES.md) applies unchanged.

## Themes

The GUI reads the same theme setting as the TUI. `default` follows your desktop's light or dark appearance, including changes while NixBox is open. The other themes (`dracula`, `gruvbox`, `nord`, `catppuccin`, `monokai`) use their own dark palettes throughout the window, including text, borders, lists, sidebar, inputs, buttons, and scrollbars. Changing the theme in Settings applies it immediately and saves it for the next launch.

The TUI's input mode setting has no effect in the GUI.

## System rebuilds and your password

A NixOS rebuild runs `sudo nixos-rebuild switch`, which needs your password. A desktop program has no terminal for sudo to ask on, so the GUI runs `sudo -A` with `SUDO_ASKPASS` pointing at its own executable, and sets `NIXBOX_ASKPASS=1` in the rebuild's environment. When sudo starts that executable to ask for a password, it sees the variable and shows a small password window instead of the main window. Authenticate passes the password to sudo; Cancel makes sudo fail, which ends the rebuild with an error.

The password goes only to sudo, through a pipe. The GUI never stores it. sudo's own credential caching still applies, so a second rebuild within sudo's timeout does not ask again.

sudo is setuid, so the loader strips `LD_LIBRARY_PATH` from the environment the password window inherits. The GUI copies it into `NIXBOX_LD_LIBRARY_PATH` for the rebuild, and the password window starts itself again with it restored, so it can load the same Vulkan and windowing libraries as the main window.

sudo stays the parent of `nixos-rebuild`, so Cancel on the Build page stops the whole process group, the same as in the TUI.

A Home Manager rebuild with no standalone `homeConfigurations` output falls back to `nixos-rebuild` and asks the same way.

## Limits

- Running the GUI and the TUI at the same time is not supported. Both write `state.json` and the managed files, and nothing coordinates them.
- The GUI has no command line of its own. Use `nixbox` or `nixbox-cli` for scripting and for `nixbox resume`.
- macOS is not packaged yet.
