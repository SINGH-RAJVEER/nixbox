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

To compile and run the native GUI from a checkout, use `just gui`. This command enters the pinned devenv shell automatically, including when invoked from an older shell. The native build needs `pkg-config` and the Wayland, X11, xkbcommon, Vulkan, fontconfig, and freetype development libraries; the repository's development shell supplies them.

To install it through your own flake, add `inputs.nixbox.url = "github:SINGH-RAJVEER/nixbox";` and include `nixbox.packages.${pkgs.stdenv.hostPlatform.system}.nixbox-gui` in `environment.systemPackages` or `home.packages`. Here `nixbox` is the input available in the enclosing `outputs` function. The package wraps the binary with `gh`, `git`, and `nix` on `PATH`, and the Vulkan loader, Wayland, xkbcommon, and X11 libraries on `LD_LIBRARY_PATH`. It runs on Wayland and X11 and needs a working Vulkan driver.

## Pages

The GUI uses [GPUI Kit](https://gpui-kit.com/) components for tabs, buttons, inputs, and other controls. Tabs centered along the top of the window list the pages, with an underline marking the active tab. NixBox sits at the left of the same row and the current install target at the right. In smaller windows, the tabs scroll horizontally. The tabs show icons and names by default; Settings switches them to icons only, which name themselves on hover, or names only. The Version control tab shows the Git or Jujutsu logo for the repository holding your configuration, or a branch icon when there is none. Queue shows the number of waiting operations, and Installed stays selected while a package's Options page is open. The status line at the bottom reports the last thing that happened, and shows the last failed rebuild until you dismiss it.

| Page | What it does |
| --- | --- |
| nixpkgs | Searches the package catalog for your locked nixpkgs revision, falling back to live `nix search` while the catalog is built. Each hit shows where it is already installed and has an Install button. |
| Flakes | Searches GitHub for flakes. Selecting a hit loads its packages, modules, and inputs. Clicking a hit with multiple installable outputs opens a list of exact package and compatible module paths; clicking a path installs that output. A single compatible output can be installed directly. |
| Installed | Packages nixbox manages, flake outputs, and packages declared in your own config, with a filter. Managed packages and flake outputs can be removed; hand-declared packages can be migrated one at a time or all at once. Options on a managed or hand-declared package opens its Options page. |
| Options | One package's module options, read from your configuration, with editors for the simple types. Staged changes are applied as one queued op. See [Package options](PACKAGE_OPTIONS.md). |
| Queue | The running rebuild and every op waiting behind it. An op can be dropped before it runs, and Apply now restarts a queue left paused by a cancelled rebuild. |
| Build | The output of the current or most recent rebuild, with a Cancel button while one runs. The Build tab shows a spinner while a rebuild runs, and a notification appears when it ends. |
| Version control | Changed files and their full diff beside an editable commit message, local commits, explicit pushes, and optional GitHub repository creation. |
| Settings | Install target, channel, theme, tab labels, and the configuration directory, saved immediately to `~/.config/nixbox/settings.json`. |

Ctrl-F focuses the search box of the current page, or the nixpkgs search. Ctrl-Q quits.

The nixpkgs results list fills the available space below the search box and scrolls through matches. Completed searches replace the displayed rows on the current page; clearing the search removes them.

Installing, removing, migrating, and applying option changes all queue an op. Ops for one target are written and rebuilt together; the other target's ops wait until that rebuild ends. This is the same `Session` the TUI uses, so the behavior described in [Managed files and package operations](MANAGED_FILES.md) applies unchanged.

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
