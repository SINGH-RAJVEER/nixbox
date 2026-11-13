# User guide

## Before starting

NixBox expects a flake-based configuration and a working `nix` command. By default it treats `~/.config/nixos` as the configuration root, reads `flake.nix` and `flake.lock` there, uses `home.nix` for Home Manager, and prefers a local `configuration.nix` for NixOS. If the configuration root is elsewhere, choose it in GUI Settings, run `nixbox config set config-dir <path>`, or set `NIXBOX_CONFIG_DIR`, which overrides the saved location.

The flake browser calls the GitHub API through `gh`. Run this once before using that tab:

```sh
gh auth login
```

NixOS changes run through `sudo nixos-rebuild switch`, so the active terminal must be able to complete the sudo prompt.

This guide covers the separately installed terminal UI, opened with `nixbox-tui`. It does not include CLI subcommands or install `nixbox` or `nb`. The desktop front-end, opened with `nixbox-gui` or the application menu, is described in [Desktop GUI](GUI.md). See [Terminal applications](TERMINAL_APPS.md) to install the TUI or CLI from the flake.

Both front-ends have a NixOS settings browser for host-wide declarations, category search, nested option templates, and current values. The desktop page also edits simple settings of the `nixos` host, such as booleans, choices, strings, numbers, and string lists, and applies them as one NixOS rebuild; the terminal tab is read-only. The host selector never changes the install target. Package-specific options remain editable through the existing Options page. See [NixOS](#nixos) for the terminal tab and [NixOS settings browser](GUI.md#nixos-settings-browser) for the desktop page.

## Starting NixBox

```sh
nixbox-tui
```

At startup NixBox performs the following work:

1. Loads `~/.config/nixbox/settings.json`, or creates default settings in memory if the file does not exist.
2. Loads the Home Manager and NixOS generated package modules if they exist.
3. Scans the configured Home Manager and NixOS entry files for package declarations outside NixBox's generated files.
4. Restores queued operations or an interrupted rebuild from `~/.config/nixbox/state.json`.
5. Loads the cached package catalog or starts a catalog build for the nixpkgs revision in the configuration's `flake.lock`.

## Tabs

The always-visible tabs are `nixpkgs`, `Flakes`, `Installed`, `NixOS`, and `VCS`. The `Building` tab appears after a rebuild starts or when build output exists. The `Queue` tab appears while operations are waiting.

Use `Tab` and `Shift-Tab` to move between visible tabs. Selection wraps at the first and last row.

### nixpkgs

The nixpkgs tab searches package attributes, package names, and descriptions. Input waits 180 ms after the last edit before searching. NixBox shows at most 200 results and marks packages that are already present in either generated manifests or the scanned configuration files.

Press `Enter` on a result to install it for the target selected in settings. If the package is already managed or queued for that target, NixBox does not add another operation. Before accepting an install, NixBox checks that the catalog still matches the target configuration's locked nixpkgs revision.

### Flakes

The Flakes tab searches GitHub code and repository metadata after a 250 ms input delay, resolves the strongest candidates to their current GitHub revisions, and evaluates those locked revisions with Nix. It keeps repositories that expose derivation packages for the current system or named NixOS or Home Manager modules, and lists flakes with an output named like the query first. Selecting a result loads repository metadata, reads input names from the flake source, and shows every evaluated package and module path plus other top-level output families. Use `PageUp` and `PageDown` to scroll the details.

Press `Enter` to open a picker when the selected flake has multiple installable outputs. Move with the arrow keys or `j` and `k`, press `Enter` on a package or module path to install that exact output, or press `Esc` to close the picker. A flake with one compatible output installs directly. The picker includes every evaluated derivation package under `packages.<current-system>` and modules compatible with the active target. An input that already points at the repository is reused; otherwise a new one is added under a short name such as `zen-browser`, following your root `nixpkgs`. Multiple outputs from the same repository can be installed separately. The command line lists paths with `nixbox flake info <owner/repo>` and installs a specific one with `nixbox flake add <owner/repo> --output '<path>'`.

### Installed

The Installed tab combines packages from four sources: NixBox's Home Manager manifest, NixBox's NixOS manifest, packages scanned from the Home Manager entry file, and packages scanned from the NixOS entry file. The `[hm]` and `[nixos]` labels identify scope. External declarations also show the attribute where the scanner found them.

Press `Delete` in Vim Normal mode on a NixBox-managed package to queue removal. `d` belongs to the filter editor, where `dd` clears the filter. NixBox will not remove an external package directly. Press `m` to migrate one external package or `M` to queue every migratable external package. Press `o` to migrate the selected external package to the other target instead: a font declared in `home.packages` moves into NixBox's NixOS manifest, and a package from `environment.systemPackages` moves into its Home Manager manifest. The footer names the destination while a migratable external package is selected. Declarations in same-line lists and complex expressions remain visible but cannot be migrated automatically.

Between the managed and external packages, a Flakes section lists every flake package and module in the configuration. That covers outputs NixBox wired into `nixbox-home-packages.nix` or `nixbox-system-packages.nix`, and flake packages you declared yourself, such as `inputs.zen-browser.packages.${pkgs.stdenv.hostPlatform.system}.default` in `home.packages`. Each row shows `input#package` (or `input.<module path>` for a module), the scope, and the `github:` repository the input points at. Rows you declared yourself also name the list they sit in. A package that NixBox manages and you also declare yourself appears once. Existing separate flake-output files are merged into the package files automatically on startup, with their imports updated and the obsolete files removed. See [upgrading existing configurations](MANAGED_FILES.md#upgrading-existing-configurations).

Press `m` on a hand-declared flake package to move it into NixBox's generated flake module. `M` includes eligible flake packages in its migrate-all queue. The GUI shows a Migrate button on eligible rows. The existing root flake input remains in place. Migration requires a dedicated line in `home.packages` or `environment.systemPackages`, a dynamic `pkgs.system` or `pkgs.stdenv.hostPlatform.system` selector, and a `github:` input that NixBox can identify. Other package options, literal-system selectors, inline declarations, non-GitHub inputs, and hand-declared module imports need manual changes. Press `Delete` in Vim Normal mode on a flake row to remove it. NixBox deletes the line from whichever file declares it, NixBox's flake module or your own entry file, and then removes the flake's input from `flake.nix` only if nothing references it any more. Removing `llm-agents#chatgpt` while `llm-agents#claude-code` is still listed keeps the `llm-agents` input, and removing the last one drops it.

The Installed tab's text field filters the combined list without changing configuration. It matches flake rows by name or repository.

Press `Enter` on a managed or external package to open its options panel. The panel lists the package's `programs.*` or `services.*` module options, read from your configuration, and lets you stage and apply changes to the simple ones. Flake rows have no options panel yet. See [Package options](PACKAGE_OPTIONS.md).

### NixOS

Choose **Configured options** with the category keys to list settings detected in the selected host's saved configuration and imported modules. Its count follows the search, and configured diagnostic options stay included. Structured settings appear under their parent option, rather than as concrete instances of nested schema templates. Reload after saving external configuration changes.

Press `Alt-H` to link this machine's hardware to the selected host. Hardware filtering then hides settings known to require absent devices. Press `Alt-A` to show all hardware options, or `Alt-H` again to unlink. The association lasts for the session and clears when switching hosts or configuration directories. Configured options and uncertain hardware stay visible. See [Hardware-aware NixOS settings](HARDWARE_FILTERING.md).

The NixOS tab browses the option schema of a host from the configuration flake's `nixosConfigurations` outputs. It reads the hosts the first time you open the tab, then picks `nixos` when it exists or the only host when there is one. A header line names the host and configuration directory. Below it are categories with their match counts, the matching options, and the selected option's details. Terminals narrower than 110 columns hide the category column and name the category in the list title.

The search field matches option paths and descriptions, with exact paths first. Inspecting an option evaluates its effective value, default, example, and definition files, so it runs only when you press `Enter`. The tab is read-only and never changes the install target or queues work. A finished rebuild invalidates the schema, which reloads the next time the tab is open.

| Key | Action |
| --- | --- |
| `Up`, `Down`, or `j`, `k` in Vim Normal mode | Move through options. |
| `Enter` | Inspect the selected option. |
| `Alt-Left`, `Alt-Right`, or `[`, `]` in Vim Normal mode | Previous or next category. |
| `Ctrl-T` | Next host. |
| `Ctrl-D` | Show or hide diagnostic options. |
| `Alt-H` | Use this machine's hardware for the selected host, or unlink it. |
| `Alt-A` | Show all hardware options, or restore relevance filtering. |
| `Ctrl-L`, or `r` in Vim Normal mode | Reload hosts and the schema. |
| `PageUp`, `PageDown` | Scroll the details. |

### Building

The Building tab shows stdout and stderr from the active rebuild. NixBox retains the latest 1,000 lines in memory. Press `u`, when no rebuild is running, to update every flake input and rebuild the system: NixOS, then a standalone Home Manager configuration if the flake has one. See [Updating the system](CLI.md#updating-the-system). Press `c` to cancel the active process group. Cancellation sends `SIGTERM`, waits up to three seconds, and then sends `SIGKILL` if the process has not exited.

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
| `Ctrl-R` | Select the VCS tab from any tab or settings page. |
| `Ctrl-C` | Quit, except while a repository operation is busy. |
| `Esc` | Return to Vim Normal mode when editing; otherwise quit. |

### Normal input mode

Normal input mode keeps the search and filter fields ready for typing. Use the arrow keys to select results and `Enter` to install from the nixpkgs or Flakes tabs, or to open a package's options panel from the Installed tab. The current Normal-mode handler does not expose uninstall or migration keys in the Installed tab, so switch to Vim input mode for those actions.

### Vim input mode

The nixpkgs, Flakes, Installed, and NixOS fields share the same Vim-like text editor and key bindings. This is a focused input editor, not a full Vim command language.

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
| `dd` | Normal | Clear the search or filter field in any of the three tabs. A different key cancels the pending first `d`. |
| `d` or `x` | Visual | Delete the inclusive selection and return to Normal mode. |
| `c` | Visual | Delete the inclusive selection and enter Insert mode. |
| `Esc` | Insert or Visual | Return to Normal mode. |

Use `j` and `k` to move through results in Vim Normal mode. The arrow keys also work while editing. Press `Enter` in the nixpkgs or Flakes tabs to install the selected item, or in the Installed tab to open the selected package's options panel.

## Settings

Press `Ctrl-S` to open settings. Use `Up` and `Down`, or `j` and `k` while Vim input mode is active, then press `Enter` to open or select an option. `Esc` returns to the settings menu and closes it from the top level.

The settings UI changes these values:

- `Input mode` selects Vim or Normal text input.
- `Theme` selects `default`, `dracula`, `gruvbox`, `nord`, `catppuccin`, or `monokai`.
- `Target` selects Home Manager or NixOS for new installs.
- `Channel` selects `nixpkgs-26.05` or `nixpkgs-unstable` for live-search fallback.
- `Usage data` shows the optional collection disclosure, saved choice, effective status, recipient, and an offline example. It defaults to Off. On explicitly consents to sending the disclosed counts to `https://telemetry.rajveer.tech/v1/telemetry`; development builds continue to suppress delivery. During a repository operation, Ctrl-S opens this privacy page directly so collection can still be disabled.

Path overrides are valid settings but are not editable in the TUI. Edit `settings.json` directly to set them.

## Version control tab

Select `VCS` with `Tab` or `Shift-Tab`, or press `Ctrl-R` in either input mode. On the first visit NixBox finds the repository for the configuration directory, preferring JJ over Git, and reviews its changes in the same step. The tab follows the GUI's layout: a header with the backend and repository root, a line with the latest result, the changed files and their diff on the left, and the commit, push, and GitHub remote controls on the right. When the configuration is a subdirectory of a larger repository, the header says so and explains what that limits. Press `r` at any time to look for the repository and review again.

Without a repository, the tab shows only initialization: press `g` for Git or `j` for Jujutsu colocated with Git, then `y` to confirm. Initialization creates no commit.

The review covers the whole configuration, including manual edits and non-ignored untracked files. The file list marks each file as added, modified, deleted, or renamed with its added and removed line counts. Use `Up`/`Down` (or `j`/`k`) to narrow the diff to one file or select `All files` again. `PageUp`/`PageDown` scroll the diff and `Alt-Left`/`Alt-Right` pan long lines. Press `s` to show or hide the raw status output.

The commit message follows the suggestion built from the successful-operation journal, with option values omitted, until you edit it. Press `m` to edit it with typing, `Enter` for a newline, `Left`/`Right`, `Home`/`End`, and `Backspace`/`Delete`; these editor keys work in both input modes. `Esc` or `Ctrl-S` finishes editing and reviews the edited message, because a commit only ever records a reviewed message. Press `c`, check the file count, then `y` to commit the displayed review. Core checks that the repository revision, files, and journal still match it; a stale review or failed commit requires another review. Commit never pushes, and after a commit the message follows the suggestion again.

Each control shows its key, or the reason it cannot run: queued operations or a rebuild in progress, no review yet, a message changed since the review, nothing to commit, a JJ subdirectory, or no origin to push to.

Press `p` to push. Enter an explicit Git branch or JJ bookmark, press `Enter`, and confirm with `y`. For JJ, `Ctrl-B` enables creating or fast-forwarding the `nixbox` bookmark to the last nonempty committed change and fills in the name `nixbox`, the only name it may push. Divergent history is refused, and push never force-pushes.

When no origin exists and the configuration is the repository root, the GitHub remote controls appear. Press `h`, enter an explicit `OWNER/NAME`, toggle visibility with `Ctrl-V` (private each time the form opens), press `Enter`, and confirm with `y` to create the repository with `gh` and add it as origin. This never commits or pushes. Authenticate with `gh auth login` beforehand.

`n` or `Esc` cancels a confirmation. `Esc` closes a form; with no form open it quits, like other tabs. `Tab` and `Shift-Tab` switch tabs and keep the current form and message. Repository subprocesses run on a worker and every action ends with a fresh review. While one runs, the result line shows a spinner, and tab navigation, controls, and exit keys wait for completion, so the TUI cannot start a conflicting configuration change. Operations do not have a cancellation or timeout control. Within VCS, `Ctrl-S` does not open settings.

Git commits in an ancestor repository are limited to the configuration scope; unrelated staged files cause refusal. Core refuses JJ commits and pushes in ancestor repositories. GitHub creation requires a repository rooted at the configuration directory with no existing origin. Ignored files stay excluded. Binary changes use the backend's diff representation.

## Exiting and recovery

`Ctrl-C` requests a clean TUI exit, except while a repository operation is busy. `Esc` exits when it is not first consumed to leave an input, settings, or repository submode. NixBox aborts package-search, flake-search, flake-detail, and catalog tasks during a normal exit.

Queue state is written before rebuilds and after queue changes. If the process exits while a rebuild is active, the next launch repeats the rebuild because the generated files already contain the requested change. If the previous rebuild failed, its error remains in the status line until a later successful build clears it.
