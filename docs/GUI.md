# Desktop GUI

The `nixbox` package installs `nixbox-gui`, a desktop window over the shared NixBox engine. It contains no CLI commands and does not install `nixbox` or `nb`. Its settings and managed files are shared with the independently installed CLI and TUI.

Open Settings > Terminal applications for step-by-step flake installation commands for the terminal UI and CLI, with a copy button for each command. Nothing is installed automatically. See [Terminal applications](TERMINAL_APPS.md).

## Installing

The GUI is Linux only for now. Install it from the repository flake to get its native libraries and runtime wrapper without setting up a build environment:

```sh
nix profile install github:SINGH-RAJVEER/nixbox
```

The flake package includes a NixBox desktop entry and icon. NixOS and Home Manager installs expose it through their application menus, and a `nix profile` install exposes it in desktops that discover the profile's `share/applications` directory. The entry launches the wrapped GUI binary from its Nix store path.

You can also run it without installing it into your profile:

```sh
nix run github:SINGH-RAJVEER/nixbox
```

The crates.io package installs a small launcher that runs the matching tagged flake release. It needs Nix with flakes enabled and fetches or builds the GUI on first run, but it does not need graphics development libraries in your Cargo environment:

```sh
cargo install nixbox
nixbox-gui
```

To compile and run the native GUI from a checkout, use `just gui`. This command enters the pinned devenv shell automatically, including when invoked from an older shell. The native build needs `pkg-config` and the Wayland, X11, xkbcommon, Vulkan, fontconfig, and freetype development libraries; the repository's development shell supplies them.

The equivalent command is `devenv shell -- cargo run -p nixbox --features native`. Without `--features native`, Cargo builds the launcher for the tagged release, so it runs the published code rather than the checkout's code.

If startup fails with `No GPU adapters found`, run `VK_LOADER_DEBUG=error,warn just gui` to see the Vulkan loader's errors. A `GLIBC_` version error from Mesa or LLVM means the GUI's runtime libraries are older than the host driver requires. For checkout builds, update `devenv update nixpkgs rust-overlay`, enter a fresh shell, and rebuild with `just gui`. The tagged release has its own pinned libraries; updating devenv does not change them. Use the native checkout build to bypass an incompatible release. Do not add another glibc to `LD_LIBRARY_PATH`.

To install it through your own flake, add `inputs.nixbox.url = "github:SINGH-RAJVEER/nixbox";` and include `nixbox.packages.${pkgs.stdenv.hostPlatform.system}.default` in `environment.systemPackages` or `home.packages`. Here `nixbox` is the input available in the enclosing `outputs` function. The package wraps the binary with `gh`, `git`, and `nix` on `PATH`, and the Vulkan loader, Wayland, xkbcommon, and X11 libraries on `LD_LIBRARY_PATH`. It runs on Wayland and X11 and needs a working Vulkan driver.

## Pages

The GUI uses [GPUI Kit](https://gpui-kit.com/) components for tabs, buttons, inputs, and other controls. Tabs centered along the top of the window list the pages, with an underline marking the active tab. NixBox sits at the left of the same row and the current install target at the right. In smaller windows, the tabs scroll horizontally. The tabs show icons and names by default; Settings switches them to icons only, which name themselves on hover, or names only. The Version control tab shows the Git or Jujutsu logo for the repository holding your configuration, or a branch icon when there is none. Queue shows the number of waiting operations, and Installed stays selected while a package's Options page is open. The status line at the bottom reports the last thing that happened, and shows the last failed rebuild until you dismiss it.

| Page | What it does |
| --- | --- |
| nixpkgs | Searches the package catalog for your locked nixpkgs revision, falling back to live `nix search` while the catalog is built. Each hit shows where it is already installed and has an Install button. |
| Flakes | Searches GitHub for flakes. Selecting a hit loads its packages, modules, and inputs. Clicking a hit with multiple installable outputs opens a list of exact package and compatible module paths; clicking a path installs that output. A single compatible output can be installed directly. |
| Installed | Packages nixbox manages, flake outputs, and packages declared in your own config, with a filter. Managed packages and flake outputs can be removed; hand-declared packages can be migrated one at a time or all at once. Hovering a hand-declared package's Migrate button opens a bubble that migrates it to the other target instead, for example a font declared for Home Manager into the NixOS system packages. Options on a managed or hand-declared package opens its Options page. |
| Options | One package's module options, read from your configuration, with editors for the simple types. Staged changes are applied as one queued op. See [Package options](PACKAGE_OPTIONS.md). |
| NixOS | A read-only browser of the selected host's option schema, with categories, search by path and description, nested templates, and on-demand values and declaration locations. |
| Queue | The running rebuild and every op waiting behind it. An op can be dropped before it runs, and Apply now restarts a queue left paused by a cancelled rebuild. |
| Build | The output of the current or most recent rebuild, with a Cancel button while one runs. The Build tab shows a spinner while a rebuild runs, and a notification appears when it ends. |
| Version control | Changed files and their full diff beside an editable commit message, local commits, explicit pushes, and optional GitHub repository creation. |
| Settings | Input mode, editable keyboard shortcuts, install target, channel, theme, tab labels, and the configuration directory, saved to `~/.config/nixbox/settings.json`. |

Zoom scales text and layout together, carries across theme changes, and resets when the GUI restarts.

## NixOS settings browser

Open **NixOS** to inspect settings across the system without opening an installed package. The tab's icon and page header show the NixOS logo. The header names the selected host and configuration directory, with the host picker and Reload on the right. Below it are the search field and the Diagnostic options switch, then three columns: categories with their match counts, the matching options, and the selected option's details. Details group the effective value, default, and example under Value, and the definition and declaration files under Source.

Host names come from the configuration flake's `nixosConfigurations` outputs. The browser selects `nixos` when available, or the only host when there is one. Otherwise choose a host. The host picker appears when there is more than one host or none is selected yet. This choice only affects inspection; it does not change the install target or the host used by existing package operations.

Categories group the selected host's declarations, including disabled services and imported custom modules. All options includes uncategorized namespaces. Search matches full paths and descriptions, with exact paths first. Diagnostic options includes internal or hidden declarations. Select an option to fetch its effective value, default, example, and definition locations separately from schema discovery. Unavailable values, null, nested templates, and withheld credential fields have different statuses. Previews limit nested depth, collection length, and long strings; sensitive attribute names are also withheld inside structured previews.

Nested schema entries describe fields such as `users.users.<name>.shell`, but concrete instance inspection is not available yet. Expansions deeper than four submodules and embedded full-system configurations have a partial marker instead of silently disappearing. Their declarations remain visible. The tab currently offers inspection only. Editing, review, validation, and host-specific writes are the next implementation work; see the [implementation plan](NIXOS_SETTINGS_PLAN.md).

Reload discovers hosts and options again. Switching hosts cancels older evaluations, and stale results cannot replace the current selection. Search, category, and inspection state survive tab switches. A finished rebuild invalidates the schema, including failed or cancelled rebuilds; changing the configuration directory resets it. Source edits and lock changes made outside NixBox need an explicit Reload. In windows narrower than 900 pixels, categories wrap above the list and selecting an option opens its details as a full-width page with Back to options.

The nixpkgs results list fills the available space below the search box and scrolls through matches. Completed searches replace the displayed rows on the current page; clearing the search removes them.

Installing, removing, migrating, and applying option changes all queue an op. Ops for one target are written and rebuilt together; the other target's ops wait until that rebuild ends. This is the same `Session` the TUI uses, so the behavior described in [Managed files and package operations](MANAGED_FILES.md) applies unchanged.

## Keyboard

Settings has the same Normal/Vim input mode choice as the TUI. Normal mode types directly into search and filter fields. Vim starts those fields in Normal mode, with Insert and Visual modes shown in the status line. The cursor shape follows the mode: a block over the character in Normal mode, a line in Insert mode, and an outlined box around the moving end of the selection in Visual mode. Clicking a field or pressing Ctrl-F enters Insert mode; Esc returns to Normal. Other text fields, including option values, commit messages, paths, and shortcut editors, keep ordinary text editing.

| Default shortcut | Action |
| --- | --- |
| Tab / Shift-Tab, Ctrl-Tab / Ctrl-Shift-Tab | Next / previous page. Plain Tab traverses controls while editing other text fields. |
| Ctrl-S | Open Settings or return to the previous page. |
| Ctrl-R | Open version control. |
| Ctrl-F | Focus search/filter on nixpkgs, Flakes, Installed, Options, or NixOS; open nixpkgs search from other pages. |
| Up / Down, j / k in Vim Normal mode | Select a result, option, output choice, or queued operation. The selected row stays visible. |
| Enter | Install a search result or flake output, open an installed package's options, or focus an option value editor. |
| Delete, m, o, M in Installed | Remove the selected managed package/flake, migrate a declaration, migrate a declaration to the other target, or migrate all eligible declarations. Letter keys require Vim Normal mode or focus outside a text field; Delete edits text while typing. |
| Ctrl-W / w, Ctrl-U / u, Ctrl-Z / U, r in Options | Apply, unset, discard, or reload. Plain letters act in Vim Normal mode. |
| c in Build | Cancel the running build. |
| Ctrl-Enter / Delete in Queue | Apply the queue / drop its selected operation. |
| PageUp / PageDown | Scroll the installed list, flake details, options, queue, build log, or diff. |
| r, e, p, h in Version control | Refresh review, edit the commit message, edit the push branch/bookmark, or edit GitHub repository details. |
| Ctrl-Enter in Version control | Submit the commit, push, or remote-creation form selected with e, p, or h. The existing review and repository checks still apply. |
| g / j, Ctrl-B, Ctrl-V, Alt-Left / Alt-Right in Version control | Initialize Git/JJ when no repository exists, toggle the nixbox bookmark, show raw status, or pan the diff. |
| Ctrl-Q | Quit. Ctrl-C also quits outside text fields; inside them it copies. |
| Ctrl-+ / Ctrl-=, Ctrl--, Ctrl-0 | Zoom in, out, or reset, in 10% steps between 50% and 200%. |

Vim editing includes i/a/I/A for insertion, h/l and arrows for movement, w/b/e and W/B/E for word/WORD motions, 0/$ for the start/end, v for inclusive Visual selection, x or d for deleting that selection, c for changing it, x for deleting a character, D for deleting to the end, and dd for clearing the input. On the Options page, w applies staged changes in Normal mode and moves by word in Visual mode. Esc also closes the flake output picker, returns from Options or Settings, or leaves another text field; it does not quit the GUI.

Edit any shortcut under **Settings > Keyboard shortcuts**. Separate alternatives with commas, such as `ctrl-f, f2`, and sequence keys with spaces, such as `d d`. Uppercase letters mean Shift. Clear a field to disable the action. Enter or **Apply** saves the binding immediately; **Reset** restores its default. Invalid keys and overlapping bindings are refused, including prefixes of another sequence. The status line explains the conflict. Page-specific actions take precedence over Vim motions, matching the TUI.

Bindings persist as `gui_shortcuts` overrides in `settings.json` and take effect without restarting. These overrides apply only to the GUI; `input_mode` is shared with the TUI.

## Configuration location

Settings shows the configuration directory nixbox edits and rebuilds. Type a path, where a leading `~` means your home directory, and press Enter or **Apply**, or pick a directory with **Browse**. **Use default** returns to `~/.config/nixos`. The directory must exist; NixBox warns when it has no `flake.nix`. Changing it reloads installed packages, flakes, and options from the new directory and restarts repository detection. It is refused while operations are queued or a rebuild runs, since those write to the current directory. When `NIXBOX_CONFIG_DIR` is set, it overrides the saved location and the field is read-only.

## Configuration repository

Open **Version control** in the top tabs to review the configuration repository. The header shows the repository's logo and root, the result of the last repository command, and anything currently blocking a commit, such as pending queued work. The left side lists the changed files with their change kind and added and removed line counts, above the diff. Click a file to narrow the diff to it, and **Show all files** to return; **Show status** adds the raw status output. Diff lines show old and new line numbers, are colored by kind, and scroll horizontally. Similar removed and added lines highlight the changed words using Delta token alignment. Unpaired lines, lines longer than 512 bytes, and blocks beyond the alignment limits keep their line colors without word highlighting. The right side holds the commit, push, and GitHub remote controls, so they stay visible however long the diff is; a disabled control says why. Git and JJ repositories are detected for the configuration directory. Colocated JJ takes priority over Git. If no repository exists, choose **Initialize Git** or **Initialize Jujutsu with Git**. Initialization does not commit or push.

**Refresh review** shows the complete status, diff and commit message, including manual edits and untracked files that are not ignored. The message is suggested from the journal of successful configuration operations. Option values are omitted from that suggestion. The diff shows the actual file contents. You can edit the multiline message. Refresh keeps your edits, including a deliberately cleared message; a blank message cannot be reviewed or committed. Untouched generated messages follow updated suggestions. Editing while a refresh runs keeps the newer draft and requires another refresh. Editing the message after a review disables the commit until you refresh, so the committed message is always the reviewed one.

Read the review, then click **Commit locally**, which names the number of files. The backend validates that the files, repository revision and operation journal still match that review before committing. A failed or stale commit requires a new explicit refresh. Nothing pushes automatically. Repository actions are blocked while operations are queued or a rebuild runs, including a paused queue. Configuration mutations are blocked while a repository command runs. Finish the queued work, or drop it, before reviewing again.

To push, enter an explicit local Git branch or JJ bookmark and click **Push branch to origin** or **Push bookmark to origin**. Only `origin` is used. For JJ, the optional **Create or advance the nixbox bookmark** checkbox fills in `nixbox` as the bookmark and creates or fast-forward advances only `nixbox` to the last nonempty committed change. Divergent history is refused. Pushes never force-update the remote.

If there is no origin, you can optionally create a GitHub repository with authenticated `gh`. Enter both an explicit owner and repository name, choose Private or Public, and click **Create repository and add origin**. Private is the default. This creates the remote and adds origin without pushing. Use the separate push control to publish a named branch or bookmark.

Git parent repositories are reviewed and committed only within the configuration directory; unrelated staged paths cause refusal. JJ parent repositories can be reviewed, but commit and push are refused because the working-copy commit is shared. GitHub repository creation requires a repository rooted at the configuration directory. Git, JJ and `gh` subprocesses run on background workers; results and errors appear on the Version control page. Opening this page refreshes the review when no writes are pending. Finishing a rebuild refreshes it while this page is open. Settings does not start repository commands. JJ must be on the GUI's `PATH`; the Nix package wrapper includes it.

## Themes

The GUI reads the same theme setting as the TUI. `default` follows your desktop's light or dark appearance, including changes while NixBox is open. The other themes (`dracula`, `gruvbox`, `nord`, `catppuccin`, `monokai`) use their own dark palettes throughout the window, including text, borders, lists, tabs, inputs, buttons, and scrollbars. Changing the theme in Settings applies it immediately and saves it for the next launch.


## System rebuilds and your password

A NixOS rebuild runs `sudo nixos-rebuild switch`, which needs your password. A desktop program has no terminal for sudo to ask on, so the GUI runs `sudo -A` with `SUDO_ASKPASS` pointing at its own executable, and sets `NIXBOX_ASKPASS=1` in the rebuild's environment. When sudo starts that executable to ask for a password, it sees the variable and shows a small password window instead of the main window. Authenticate passes the password to sudo; Cancel makes sudo fail, which ends the rebuild with an error.

The password goes only to sudo, through a pipe. The GUI never stores it. sudo's own credential caching still applies, so a second rebuild within sudo's timeout does not ask again.

sudo is setuid, so the loader strips `LD_LIBRARY_PATH` from the environment the password window inherits. The GUI copies it into `NIXBOX_LD_LIBRARY_PATH` for the rebuild, and the password window starts itself again with it restored, so it can load the same Vulkan and windowing libraries as the main window.

sudo stays the parent of `nixos-rebuild`, so Cancel on the Build page stops the whole process group, the same as in the TUI.

A Home Manager rebuild with no standalone `homeConfigurations` output falls back to `nixos-rebuild` and asks the same way.

## Limits

- Running the GUI and the TUI at the same time is not supported. Both write `state.json` and the managed files, and nothing coordinates them.
- `nixbox-gui` only opens the GUI. CLI commands belong to the separately installed `nixbox-cli` package and are invoked as `nixbox`, `nb`, or `nixbox-cli`.
- The GUI is Linux only. On macOS the flake's default package remains the independently installed terminal UI; the GUI Cargo launcher does not fall back to another interface.
