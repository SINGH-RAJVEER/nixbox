# Release notes

This file tracks the user-facing changes in each NixBox version published to crates.io.

## Unreleased

- The desktop NixOS settings editor now uses dropdowns for all enumerated choices and labels boolean toggles Enabled or Disabled. Nullable booleans retain an explicit null choice in a dropdown; controls stage changes until Apply.
- Builds now use every CPU core by default. The flake packages no longer cap compilation at two jobs, and checkout Cargo builds drop the two-job `.cargo/config.toml` default. A positive `buildJobs` package override, `--jobs`, or `CARGO_BUILD_JOBS` still sets a lower limit for machines with little RAM.
- Added hardware-aware filtering to the NixOS browser in the GUI and TUI. Link the selected host to this machine's detected CPU, GPUs, Bluetooth, and battery; known irrelevant settings then disappear from search and editor groups. Configured and staged settings stay accessible, incomplete detection stays unknown, and Show all restores excluded hardware settings.

## 0.4.6 - 2026-10-23

- Added a round GTK-style close button at the top right of the desktop GUI that quits NixBox like Ctrl-Q. Settings has a switch to hide it, saved as `close_button`, and `nixbox config set close-button false` does the same.

## 0.4.5 - 2026-10-23

- Added optional usage counters, disabled by default for fresh and existing installs. GUI/TUI settings and CLI controls disclose the fields, show effective status, and provide offline examples. Settings writes preserve opt-out across stale instances. All interfaces share `https://nixb.duckdns.org:58443/v1/telemetry`; consent version 3 discloses that recipient and requires an explicit choice.
- Added an optional Axum receiver that stores daily SQLite aggregates for 90 days and enforces strict request and capacity limits. The collection protocol is custom JSON.
- Added receiver and Nginx Dockerfiles, isolated Compose deployment, and certificate issuance and renewal. Public HTTPS ingestion is verified on the shared VM without restarting or reconfiguring its existing application. Provider infrastructure logging and backup retention remain outside application verification.

## 0.4.1 - 2026-10-22

- Added opt-in package profiles. `nixbox profile setup` moves a target's packages into an independent Nix profile and rebuilds once; installs and removals then build only the profile. Modules, options and migrations still rebuild. `nixbox profile teardown` moves the packages back into the combined configuration module.
- Made every rebuild set the profile to the configuration's package set, so fresh installs, `nixos-rebuild --rollback`, the boot menu and `nix flake update` carry the profile along. The profile build and the rebuild share one `nixbox.profile` option and produce the same store path.
- Put the profile on PATH through PAM on NixOS, and through `home.sessionPath` and nushell's `env.nu` for Home Manager. When Home Manager runs inside NixOS, setup also adds the profile to the NixOS PAM environment.
- Added `nixbox profile generations` and `nixbox profile rollback [--to <generation>]`, `apply --configuration`, and setup status in `status` and `doctor`.
- Recorded the profile mode in the generated package file, so it travels with the configuration repository.
- Made the desktop NixOS page a settings editor. Selecting an option opens it with its sibling options, each with a control for its type: switches, choice buttons or menus, and text fields for strings, numbers, and string lists. Edits are staged, marked in the list and editor, and applied together as one NixOS rebuild. Options you define yourself, options set by other modules, credentials, and complex types stay read-only with a short reason. Editing is limited to the `nixos` host that nixbox rebuilds.
- Combined flake outputs and nixpkgs packages in `nixbox-home-packages.nix` and `nixbox-system-packages.nix`. Existing configurations upgrade automatically on startup: NixBox merges the old flake-output files, updates imports, preserves profile mode, and deletes the obsolete files. Unexpected custom edits stop the migration without discarding them.
- Tidied the desktop Version control page. The header shows the repository root with a backend tag, plus a subdirectory tag when the configuration is part of a larger repository. Explanations for disabled controls moved into tooltips, and button labels are shorter. Every control is unchanged.

## 0.4.0 - 2026-10-16

- Reworked the desktop Build page into expandable preparation, evaluation, download, build, and activation steps. Each step shows its duration and output; download and build progress uses Nix's reported counts and bytes. The running step opens automatically, and failures retain the failed step and its error.
- Read Nix's structured rebuild output in the shared session while keeping CLI output readable. Build cancellation and persisted recovery continue to use the same rebuild process.
- Added shared batched NixOS option inspection, including values, types, definition priorities, and source ownership.
- Updated the VM integration test to use Delete for uninstall, avoid quitting the TUI from Normal mode, and navigate without racing asynchronous repository detection.
- Updated all seven workspace crates and internal dependencies to `0.4.0`.

## 0.3.7 - 2026-10-15

- Separated interface installations. The GUI package installs only `nixbox-gui`, the TUI only `nixbox-tui`, and the CLI package owns `nixbox`, `nb`, and `nixbox-cli`. GUI and TUI no longer include CLI commands. Bare CLI aliases show help; interface-launch switches and terminal detection are removed. Added copyable TUI/CLI flake installation steps to GUI Settings.
- Redesigned the GUI's NixOS page. The NixOS logo now appears on the tab and in a header that names the host and configuration directory. The page has a category sidebar with match counts, the option list, and details grouped into Value and Source.
- Added a matching read-only NixOS tab to the terminal UI, with the same categories, search, host choice, and on-demand inspection.
- Gave the GUI's search fields Vim cursor shapes: a block in Normal mode, a line in Insert mode, and an outlined box in Visual mode.
- Added migration of a hand-declared package to the other target. In the GUI, hovering a migratable package's Migrate button opens a bubble offering the opposite target, so a font declared for Home Manager can move straight into the NixOS system packages. The TUI and GUI bind it to `o` in the Installed tab, and the TUI footer names the destination while a migratable package is selected. Only the destination rebuilds; the source target drops the package on its next rebuild.

- Installed Jujutsu in CI so the repository integration tests run with their required executable available.
- Updated all workspace crates and internal dependencies to 0.3.7.

## 0.3.6 - 2026-10-15

- Limited each NixBox flake package to two compiler jobs by default, including test compilation. A lower Nix core allowance is preserved; a positive `buildJobs` package override lets users tune the ceiling for their build machine. The GUI, TUI, CLI, and overlay packages share this policy.
- Limited checkout Cargo builds to two jobs by default, with standard `--jobs` and `CARGO_BUILD_JOBS` overrides. Documented explicit limits for native Cargo installations and the separate machine-wide Nix build settings.
- Added package-hook checks for default limits, lower Nix allowances, larger overrides, exported compiler limits, and invalid values. The Cargo launcher continues to inherit the user's Nix settings.
- Updated all workspace crates and internal dependencies to 0.3.6.

## 0.3.5 - 2026-10-11

- Made the desktop GUI the default NixBox. The `nixbox` crate, flake package, and command are now the GUI, which `nixbox-gui` used to be; `cargo install nixbox` installs its Nix-backed launcher, and `nix profile install github:SINGH-RAJVEER/nixbox` installs it with its desktop entry, now `nixbox.desktop`. On macOS, where the GUI is not packaged, the flake's default package and the launcher run the terminal UI.
- Moved the terminal UI to its own `nixbox-tui` crate, flake package, and command. `nixbox-cli` remains the command line with no interface. All three take the same subcommands, so `nixbox install`, `nixbox resume`, and the rest work from the GUI binary without opening a window.
- Removed the `tui` subcommand: a bare `nixbox-tui` opens the terminal UI. The `nixbox-gui` crate and flake package are removed; install `nixbox` instead.
- Updated all workspace crates and internal dependencies to 0.3.5.
- Added a read-only NixOS tab with host selection, category and description search, diagnostic declarations, nested option templates, virtualized rows, and on-demand values, defaults, examples, and source locations. Recursive and embedded full-system schema branches remain visible with partial markers.
- Kept system inspection separate from package targets, package option editors, and queued mutations. System-wide editing and validated application remain in development.
- Kept Ctrl-F on the Options and NixOS pages so it focuses their own search fields. Added live schema fixtures and rendered-window coverage for the NixOS browser.
- Added editable GUI keyboard shortcuts under Settings, saved as `gui_shortcuts` overrides, and the TUI's Normal/Vim input modes to the GUI's search and filter fields.
- Added GUI zoom with Ctrl-+, Ctrl--, and Ctrl-0, in 10% steps between 50% and 200%, scaling text and layout together.
- Highlighted hovered rows in the GUI search and Installed lists.

## 0.3.1 - 2026-10-04

- Published the sandbox build fix in the crate archives: core test fixtures isolate user settings and VCS journals, and restore the caller's environment when dropped. The Nix flake supplies Git and Jujutsu for repository integration tests.
- Updated all workspace crates and internal dependencies to 0.3.1 so Cargo installs and the GUI launcher use the corrected release.

## 0.3.0 - 2026-10-04

- Corrected the 0.3.0 flake tag to fix sandboxed Nix package builds by isolating core test settings and VCS journals from the build user's home, and supplying Git and Jujutsu for repository integration tests. The published crates.io archives are unchanged.
- Added old and new line numbers and within-line word highlighting to the GUI repository diff, using Delta's token alignment. Unpaired lines and changes beyond the alignment limits keep their line colors without word highlighting.
- Added configuration version control pages to the TUI and GUI, with Git and colocated Jujutsu detection, repository initialization, full status and diff review, and editable commit messages.
- Added `nixbox commit` to the CLI, with optional message editing, explicit approval, and a Git dry run. Suggested messages describe successful configuration operations and omit option values.
- Reject stale or empty commit reviews and block repository actions while configuration operations are queued or rebuilding. Configuration mutations wait while a repository command runs.
- Added explicit pushes to `origin` by Git branch or JJ bookmark, optional creation or advancement of the `nixbox` bookmark, and GitHub repository creation with private visibility by default. Repository creation, commits, and pushes remain separate actions.
- Shared Vim search editing across the nixpkgs, Flakes, and Installed TUI fields, including word and WORD motions, selection, and deletion.
- Fixed computed GUI package search results staying invisible because the virtualized list had no usable viewport height. Added window tests for visible results, replacement, clearing, and subsequent results.
- Switched workspace, CLI, and native GUI tests to Cargo Nextest in local recipes and CI, added it to the development shell, and enabled it for Nix flake package checks. Updated the development documentation to match.
- Moved GUI navigation to top tabs and removed the overflow menu that repeated them. A new tab labels setting shows icons and names, icons only, or names only, in GUI Settings and as `nixbox config set tab-labels`.
- Showed the Git or Jujutsu logo on the GUI Version control tab for the repository holding the configuration.
- Centered the GUI tabs in the top row, with NixBox on the left and the install target on the right.
- Made the configuration directory a saved setting, editable in GUI Settings with a folder picker and with `nixbox config set config-dir`. `NIXBOX_CONFIG_DIR` still overrides it.
- Redesigned the GUI Version control page: changed files with change kinds and line counts beside the commit, push, and remote controls, a colored diff that can be narrowed to one file, and a reason under each disabled control. Editing the commit message no longer hides the review.

## 0.2.9 - 2026-09-30

- Applied each selected dark theme's full palette across the GUI instead of changing only its background and accent.
- Made the default GUI theme follow desktop appearance changes while the app is open.
- Listed complete evaluated flake package and named module paths in the CLI, terminal UI, and GUI detail views; the terminal UI can scroll long output inventories.
- Listed installable flake package and named module paths in the CLI, and added exact output selection in the terminal and desktop UIs and with `nixbox flake add --output`. Multiple modules from one repository can now coexist.

## 0.2.8 - 2026-09-28

- Added a NixBox desktop entry and icon to the Linux GUI flake package, so profile and declarative installs can show it in the application menu.
- Added package options to the terminal UI's Installed tab and the desktop GUI's Installed page. `Enter`, or the Options button, on a package lists its `programs.*` or `services.*` options from your own configuration, and simple values can be staged and applied as one rebuild through a new generated settings module.
- Extended the test VM's headless check to the options panel, from opening a package's options to writing, evaluating, and unsetting a value. A full round trip in the interactive VM, `programs.git.lfs.enable` applied with a real `nixos-rebuild switch` and then unset, installed and removed `git-lfs`.

## 0.2.7 - 2026-09-27

- Made `cargo install nixbox-gui` install a small launcher that runs the matching flake release, without requiring graphics development libraries in the Cargo build environment. Native GUI builds remain available with the `native` feature.
- Exposed named CLI and GUI flake apps alongside the existing default app.

## 0.2.6 - 2026-09-27

- Published `nixbox-gui` as a separate Linux desktop crate. It can now be installed with `cargo install nixbox-gui` when the graphics development libraries are available.
- Updated Ratatui, Crossterm, and tui-input, replacing the vulnerable `lru` 0.12 dependency with a patched version.
- Made the release workflow wait for the GUI checks before publishing all eight crates.

## 0.2.5 - 2026-09-27

- Added a Linux desktop GUI built with GPUI. It shares NixBox's queue, settings, and rebuild engine with the terminal and command-line front ends, and uses a password window for sudo rebuilds.
- Added migration of eligible hand-declared flake packages into NixBox's generated modules from the CLI, terminal UI, and desktop GUI.
- Extended the test VM's headless check to flake wiring, applying queued flake changes offline with `nixbox resume`. It covers the `{ ... }@inputs` form, input naming and `follows`, two packages from one flake, skipping a package the entry file already declares, keeping an input until its last package is removed, and `nixbox flake remove`.
- Drove the real TUI in the VM through tmux to confirm the Installed tab lists flake packages from both the managed files and hand-written configuration, and that `d` removes a hand-written one.

## 0.2.4 - 2026-09-24

Flake packages, wherever they are declared, now show up in the Installed tab and can be removed there, and flake installation follows hand-written configurations.

- Made flake installation reuse a root input that already points at the selected repository, and name new inputs the way they are usually written by hand (`zen-browser` rather than `"0xc000022070/zen-browser-flake"`), following the root `nixpkgs` when there is one.
- Accepted `outputs = { ... }@inputs:` and `outputs = inputs:` in the root flake, not only `outputs = inputs@{ ... }:`.
- Supported Home Manager installed as a NixOS module: `inputs` is passed through `specialArgs` and `home-manager.extraSpecialArgs` instead of failing on a missing `homeManagerConfiguration`.
- Changed the Home Manager target to install a flake's package into `home.packages` before considering its Home Manager module, which does nothing until its options are set.
- Switched generated package references from the deprecated `pkgs.system` to `pkgs.stdenv.hostPlatform.system`. Files written by earlier releases are still read.
- Kept a flake's root input on removal when other files in the configuration still reference it.
- Added a Flakes section to the Installed tab listing every flake package and module, both those NixBox manages and flake packages declared by hand in `home.nix` or `configuration.nix`, with the repository each comes from. `d` removes one from whichever file declares it, and the flake's input goes once its last package is removed.
- Stopped installing a flake package that the entry file already declares, instead of listing it twice.
- Fixed the generated flake module keeping only one package per flake: installing a second package from the same flake replaced the first.
- Changed `nixbox flake list` to show packages as well as modules. Its `--json` rows now carry `repo`, `kind`, and `output` instead of `repo` and `module`.

## 0.2.3 - 2026-09-16

The command line release. Neither 0.2.1 nor 0.2.2 reached crates.io, because the publish workflow could not authenticate; everything they described ships here.

- Added a full command line interface. Every operation the TUI performs is now also a subcommand: `search`, `install`, `remove`, `migrate`, `apply`, `list`, `scan`, `status`, `doctor`, `flake`, `config`, and `completions`. Running `nixbox` with no subcommand still starts the TUI.
- Added `--json` output to the read-only commands, `--target` and `--channel` overrides that apply to a single invocation, and `--dry-run`, `--no-rebuild`, and `--yes` to every command that changes configuration.
- Refused an unconfirmed change when stdin is not a terminal, so a script that omits `--yes` stops instead of rebuilding a machine unattended.
- Separated the exit code for a failed rebuild from the exit code for a failed command, so a caller can tell a written-but-unbuilt configuration from a refused one. The former is recoverable with `nixbox apply`.
- Extracted the engine behind both front-ends into a new `nixbox-core` crate. A change made from the command line and the same change made in the TUI now take the identical code path and persist the identical `state.json` queue.
- Extended flake installation to the command line, including removal of a flake's input and generated output, which the TUI does not offer.
- Split the program into two published packages. `nixbox` is unchanged; the new `nixbox-cli` is the same subcommands with no terminal UI, installing as `nixbox-cli` so both can sit on one machine. It pulls 60 dependency packages against 108, leaving out `ratatui`, `crossterm`, and `tui-input`.
- Moved the command tree into a new `nixbox-cmd` crate shared by both binaries, so the two can never drift apart. Each binary is a four-line `main`; the terminal UI is a Cargo feature that only `nixbox` turns on.
- Made help text, generated completion scripts, and hints like "run `nixbox apply`" use the name the running binary was installed as.
- Moved the theme names and the `state.json` format out of the UI crate into `nixbox-config` and `nixbox-core`, so `nixbox config set theme` and `nixbox resume` work in a build with no terminal UI.
- Routed `nixbox search` and `nixbox install` through the local package catalog, matching the TUI. An explicit `--channel` still searches the channel live, and a missing catalog falls back to live search with a note.
- Added `nixbox resume`, which finishes an interrupted rebuild or drains a queue left by the TUI, one target at a time. `--dry-run` prints the queue and `--discard` clears it.
- Recorded a command-line rebuild in `state.json` before it starts, so a run killed mid-rebuild can be resumed from either front-end.
- Added a disposable NixOS virtual machine for testing NixBox against a real configuration and a real `nixos-rebuild switch` without touching the host, plus an automated headless check over the parts that work without a network.
- Gated crate publishing on a CI `test` job that runs `cargo fmt --all --check`, Clippy with `-D warnings`, and the full test suite on the stable toolchain. The publish job now declares `needs: test` and no longer runs when that gate fails.
- Extended the workflow to pull requests targeting `master` so the same gate reports before a release merge instead of after it.

## 0.2.1 - 2026-09-15

- Replaced a separate `nix search` evaluation for every query with a local package catalog built from the exact `nixpkgs` revision in the target configuration's `flake.lock`.
- Cached the catalog at `~/.cache/nixbox/package-catalog.json` and rebuild it when the locked `nixpkgs` revision changes. Search falls back to the configured live channel if the catalog cannot be prepared.
- Reduced measured warm searches from 15.9 seconds or more to 53-75 ms in a debug build. Loading an existing 112,847-package catalog took 425 ms in the same test.
- Added local relevance ranking across package attributes, names, and descriptions while retaining nested attributes such as `python312Packages.black`.
- Changed GitHub flake discovery to evaluate the strongest candidates at their locked GitHub revision, reject repositories without derivation packages or default modules, and rank exact package outputs above weak repository matches.
- Added installation of the highest-ranked package output when a selected flake does not publish a default module for the active target, and preserved `homeModules.default` as a distinct Home Manager module path.
- Added an installable Nix flake with packages, a runnable app, and an overlay for `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`.
- Expanded Vim-style editing in search fields with distinct word and WORD motions, end-of-word motions, and `dd` line deletion.
- Enabled stricter workspace-wide Clippy checks, including bans on unchecked indexing, arithmetic, panics, `unwrap`, and `expect`.
- Switched the pinned devenv Rust toolchain to the nightly channel.

## 0.2.0 - 2026-08-13

- Added a GitHub flake browser for finding repositories that expose Nix flake outputs.
- Ranked flake discovery results so repositories with usable NixOS and Home Manager modules appear before weak matches.
- Added target-aware installation of flake inputs and modules for NixOS and Home Manager configurations.
- Batched queued changes for the same target into one rebuild instead of rebuilding after every individual edit.
- Corrected package metadata and moved the accumulated flake-management work to the `0.2.0` release line.

## 0.1.9 - 2026-08-09

- Added Normal, Insert, and Visual modes to package search fields.
- Added Vim-style character and word movement, start and end movement, append, character deletion, delete-to-end, and visual delete or change operations.
- Persisted the selected input mode in NixBox settings.
- Reworked the footer and settings UI to show commands for the active mode.

## 0.1.8 - 2026-07-17

- Ranked package matches by exact name, prefix, whole token, substring, and description matches instead of relying on nixpkgs output order.
- Kept only the 200 most relevant matches and told the user when a query reached that limit.
- Added cancellation for active rebuilds. NixBox terminates the full process group and escalates from `SIGTERM` to `SIGKILL` if needed.
- Resolved common channel names such as `nixpkgs-unstable` to explicit GitHub flake references.

## 0.1.7 - 2026-07-17

- Fixed searches that could hang after Nix wrote more than 64 KiB of evaluation output to stderr. NixBox now drains the stream while retaining only a bounded error message.
- Reduced Nix evaluation noise by running package searches with `--quiet`.
- Tightened search-task cleanup and added regression tests around bounded process output.
- Replaced the development flake with a pinned devenv environment containing Rust, Nix, `just`, `nixd`, and `nil`.
- Added `just ci` and `devenv test` as the repository's format, lint, and test checks.

## 0.1.6 - 2026-06-05

- Replaced full search-result materialization with bounded JSON parsing and a maximum of 200 displayed results.
- Added limits for captured search output and error text so broad nixpkgs searches cannot grow memory without bound.
- Cancelled obsolete search tasks when a newer query supersedes them or the application exits.
- Added tests for result limits, missing descriptions, attribute parsing, and rebuild output forwarding.

## 0.1.5 - 2026-05-23

- Persisted pending package operations, the active rebuild, and the last error in `~/.config/nixbox/state.json`.
- Restored queued operations after a restart and retried rebuilds interrupted by a crash or terminated session.
- Kept install, uninstall, and migration operations ordered across rebuilds.
- Added crates.io publishing automation that skips crate versions which are already available.

## 0.1.4 - 2026-05-19

- Added a search field to the Installed tab.
- Filtered both NixBox-managed and externally declared packages as the user types.
- Kept selection within the filtered result set and added a clear empty state when no installed package matches.

## 0.1.3 - 2026-05-19

- Split managed packages into `nixbox-home-packages.nix` and `nixbox-system-packages.nix`.
- Automatically inserted the appropriate managed file into the target Nix configuration's `imports` list without changing its existing layout.
- Detected whether Home Manager runs as a standalone flake output or as a NixOS module and selected the matching rebuild command.
- Scanned existing Nix configuration files for packages and added commands to migrate one or all supported declarations into NixBox management.
- Added configurable paths for Home Manager and NixOS entry files.
- Added direnv and Rust Analyzer support to the development environment.

## 0.1.2 - 2026-05-19

- Added an Installed tab that distinguishes NixBox-managed packages from packages declared elsewhere in the configuration.
- Added scanning for package declarations in Home Manager and NixOS files, including common multiline and `with pkgs` forms.
- Made system-package detection more tolerant of real-world Nix formatting.
- Split the TUI into focused handlers, navigation, operations, state, and UI modules to make later queue and migration work safer.
- Improved status and queue presentation and made installed packages clearer in search results.

## 0.1.1 - 2026-05-12

- Added a dedicated empty state when a package search returns no matches.
- Moved the workspace to Rust edition 2024.
- Corrected the Rust CodeQL workflow so it builds the Cargo workspace directly.
- Removed obsolete VM development commands and completed Apache-2.0 package metadata.

## 0.1.0 - 2026-05-12

- Rebuilt the original ArchBox experiment as NixBox, a terminal package manager for NixOS and Home Manager.
- Added live package search through `nix search --json` with a configurable nixpkgs channel.
- Added a Ratatui interface for searching, selecting, installing, and removing packages.
- Added managed Nix package files so NixBox could update package declarations without rewriting the rest of the user's configuration.
- Added Home Manager and NixOS targets with rebuild output streamed into the interface.
- Persisted the selected channel, target, paths, and theme in `~/.config/nixbox/settings.json`.
- Added Cargo and Nix packaging for the four-crate Rust workspace.
