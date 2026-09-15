# Release notes

This file tracks the user-facing changes in each NixBox version published to crates.io.

## Unreleased

This is the release that will be cut when the current `dev` branch is merged. The version and release date are not set yet.

- No changes yet.

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
