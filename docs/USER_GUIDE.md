# User guide

## Before starting

NixBox expects a flake-based configuration and a working `nix` command. By default it treats `~/.config/nixos` as the configuration root, reads `flake.nix` and `flake.lock` there, uses `home.nix` for Home Manager, and prefers a local `configuration.nix` for NixOS. Set `NIXBOX_CONFIG_DIR` before starting NixBox if the configuration root is elsewhere.

The flake browser calls the GitHub API through `gh`. Run this once before using that tab:

```sh
gh auth login
```

NixOS changes run through `sudo nixos-rebuild switch`, so the active terminal must be able to complete the sudo prompt.

## Starting NixBox

```sh
nixbox
```

At startup NixBox performs the following work:

1. Loads `~/.config/nixbox/settings.json`, or creates default settings in memory if the file does not exist.
2. Loads the Home Manager and NixOS generated package modules if they exist.
3. Scans the configured Home Manager and NixOS entry files for package declarations outside NixBox's generated files.
4. Restores queued operations or an interrupted rebuild from `~/.config/nixbox/state.json`.
5. Loads the cached package catalog or starts a catalog build for the nixpkgs revision in the configuration's `flake.lock`.

## Tabs

The always-visible tabs are `nixpkgs`, `Flakes`, and `Installed`. The `Building` tab appears after a rebuild starts or when build output exists. The `Queue` tab appears while operations are waiting.

Use `Tab` and `Shift-Tab` to move between visible tabs. Selection wraps at the first and last row.

### nixpkgs

The nixpkgs tab searches package attributes, package names, and descriptions. Input waits 180 ms after the last edit before searching. NixBox shows at most 200 results and marks packages that are already present in either generated manifests or the scanned configuration files.

Press `Enter` on a result to install it for the target selected in settings. If the package is already managed or queued for that target, NixBox does not add another operation. Before accepting an install, NixBox checks that the catalog still matches the target configuration's locked nixpkgs revision.

### Flakes

The Flakes tab searches GitHub code and repository metadata after a 250 ms input delay, resolves the strongest candidates to their current GitHub revisions, and evaluates those locked revisions with Nix. It keeps repositories that expose derivation packages for the current system or a conventional default NixOS or Home Manager module. Selecting a result loads repository metadata, reads input names from the flake source, and shows output categories plus up to four evaluated package attributes.

Press `Enter` to install the selected repository's preferred output. NixBox chooses `homeManagerModules.default` or `homeModules.default` for the Home Manager target and `nixosModules.default` for the NixOS target. When the matching default module does not exist, it installs the first evaluated package instead. Package ranking favors the `default` attribute and then matches package attributes against the search query. NixBox evaluates only `packages.<current-system>`, so packages offered here match the active system.

### Installed

The Installed tab combines packages from four sources: NixBox's Home Manager manifest, NixBox's NixOS manifest, packages scanned from the Home Manager entry file, and packages scanned from the NixOS entry file. The `[hm]` and `[nixos]` labels identify scope. External declarations also show the attribute where the scanner found them.

Press `d` or `Delete` on a NixBox-managed package to queue removal. NixBox will not remove an external package directly. Press `m` to migrate one external package or `M` to queue every migratable external package. Declarations in same-line lists and complex expressions remain visible but cannot be migrated automatically.

The Installed tab's text field filters the combined list without changing configuration.

### Building

The Building tab shows stdout and stderr from the active rebuild. NixBox retains the latest 1,000 lines in memory. Press `c` to cancel the active process group. Cancellation sends `SIGTERM`, waits up to three seconds, and then sends `SIGKILL` if the process has not exited.

### Queue

The Queue tab is read-only. Operations are grouped by target because Home Manager and NixOS may need different rebuild commands. NixBox applies every queued operation for the target at the front of the queue, writes the resulting files, and starts one rebuild for that batch. Work for the other target waits until the current rebuild finishes successfully or fails. Cancelling a rebuild pauses the remaining queue.

## Input modes

Choose Vim mode or Normal mode under `Ctrl-S`, then `Input mode`. NixBox saves the choice immediately.

### Global keys

| Key | Action |
| --- | --- |
| `Tab` | Move to the next visible tab. |
| `Shift-Tab` | Move to the previous visible tab. |
| `Ctrl-S` | Open or close settings. |
| `Ctrl-C` | Quit from any mode. |
| `Esc` | Return to Vim Normal mode when editing; otherwise quit. |

### Normal input mode

Normal input mode keeps the search and filter fields ready for typing. Use the arrow keys to select results and `Enter` to install from the nixpkgs or Flakes tabs. The current Normal-mode handler does not expose uninstall or migration keys in the Installed tab, so switch to Vim input mode for those actions.

### Vim input mode

The nixpkgs, Flakes, and Installed fields support Vim-like text editing. This is a focused input editor, not a full Vim command language.

| Key | Mode | Action |
| --- | --- | --- |
| `i` or `/` | Normal | Enter Insert mode before the cursor. |
| `a` | Normal | Enter Insert mode after the cursor. |
| `I` | Normal | Enter Insert mode at the start. |
| `A` | Normal | Enter Insert mode at the end. |
| `v` | Normal | Enter Visual mode. |
| `h`, `l`, `Left`, `Right` | Normal or Visual | Move by one character. |
| `b`, `w` | Normal or Visual | Move by Vim word boundaries. Letters, digits, and underscores form one class; punctuation forms another. |
| `B`, `W` | Normal or Visual | Move by whitespace-delimited WORD boundaries. |
| `e`, `E` | Normal or Visual | Move to the end of the current or next word or WORD. |
| `0`, `$` | Normal or Visual | Move to the start or end. |
| `x` | Normal | Delete the character under the cursor. |
| `D` | Normal | Delete from the cursor through the end. |
| `dd` | Normal | Clear the nixpkgs or Flakes search field. In the Installed tab, `d` means uninstall. |
| `d` or `x` | Visual | Delete the inclusive selection and return to Normal mode. |
| `c` | Visual | Delete the inclusive selection and enter Insert mode. |
| `Esc` | Insert or Visual | Return to Normal mode. |

Use `j` and `k` to move through results in Vim Normal mode. The arrow keys also work while editing. Press `Enter` in the nixpkgs or Flakes tabs to install the selected item.

## Settings

Press `Ctrl-S` to open settings. Use `Up` and `Down`, or `j` and `k` while Vim input mode is active, then press `Enter` to open or select an option. `Esc` returns to the settings menu and closes it from the top level.

The settings UI changes these values:

- `Input mode` selects Vim or Normal text input.
- `Theme` selects `default`, `dracula`, `gruvbox`, `nord`, `catppuccin`, or `monokai`.
- `Target` selects Home Manager or NixOS for new installs.
- `Channel` selects `nixpkgs-26.05` or `nixpkgs-unstable` for live-search fallback.

Path overrides are valid settings but are not editable in the TUI. Edit `settings.json` directly to set them.

## Exiting and recovery

`Ctrl-C` always requests a clean TUI exit. `Esc` exits when it is not first consumed to leave an input or settings submode. NixBox aborts package-search, flake-search, flake-detail, and catalog tasks during a normal exit.

Queue state is written before rebuilds and after queue changes. If the process exits while a rebuild is active, the next launch repeats the rebuild because the generated files already contain the requested change. If the previous rebuild failed, its error remains in the status line until a later successful build clears it.
