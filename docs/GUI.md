# Desktop GUI

`nixbox-gui` is a desktop window over the same engine as the terminal UI. It reads and writes the same settings, the same managed files, and the same `state.json`, so work queued in one front-end can be resumed in another.

## Installing

The GUI is Linux only for now and is not on crates.io. Install it from the flake:

```sh
nix profile install github:SINGH-RAJVEER/nixbox#nixbox-gui
```

or add `nixbox-gui` from the flake's overlay to `environment.systemPackages` or `home.packages`. The package wraps the binary with `gh`, `git`, and `nix` on `PATH`, and the Vulkan loader, Wayland, xkbcommon, and X11 libraries on `LD_LIBRARY_PATH`. It runs on Wayland and X11 and needs a working Vulkan driver.

## Pages

The sidebar lists the pages. The status line at the bottom reports the last thing that happened, and shows the last failed rebuild until you dismiss it.

| Page | What it does |
| --- | --- |
| nixpkgs | Searches the package catalog for your locked nixpkgs revision, falling back to live `nix search` while the catalog is built. Each hit shows where it is already installed and has an Install button. |
| Flakes | Searches GitHub for flakes. Selecting a hit loads its packages, modules, and inputs; Install adds the package or module that suits the current target, as the TUI does. |
| Installed | Packages nixbox manages, flake outputs, and packages declared in your own config, with a filter. Managed packages and flake outputs can be removed; hand-declared packages can be migrated one at a time or all at once. |
| Queue | The running rebuild and every op waiting behind it. An op can be dropped before it runs, and Apply now restarts a queue left paused by a cancelled rebuild. |
| Build | The output of the current or most recent rebuild, with a Cancel button while one runs. The sidebar shows a spinner while a rebuild runs, and a notification appears when it ends. |
| Settings | Install target, channel, and theme, saved immediately to `~/.config/nixbox/settings.json`. |

Ctrl-F focuses the search box of the current page, or the nixpkgs search. Ctrl-Q quits.

Installing, removing, and migrating all queue an op. Ops for one target are written and rebuilt together; the other target's ops wait until that rebuild ends. This is the same `Session` the TUI uses, so the behavior described in [Managed files and package operations](MANAGED_FILES.md) applies unchanged.

## Themes

The GUI reads the same theme setting as the TUI. `default` follows your desktop's light or dark appearance. The other themes (`dracula`, `gruvbox`, `nord`, `catppuccin`, `monokai`) are dark, using the TUI palette's background and accent color.

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
