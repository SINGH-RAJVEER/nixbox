# Architecture

## System boundary

NixBox is a local terminal application. It does not run a daemon, expose an HTTP API, receive webhooks, or maintain a remote index. The process reads local configuration, calls the `nix`, `home-manager`, `nixos-rebuild`, `git`, and `gh` executables, writes local Nix and JSON files, and renders a Ratatui interface.

```mermaid
flowchart LR
		User[Terminal user] --> Binary[nixbox binary]
		Binary --> TUI[nixbox-tui]
		TUI --> Config[nixbox-config]
		TUI --> Nix[nixbox-nix]
		Config --> Settings[settings.json]
		Nix --> Target[flake.nix and Nix modules]
		Nix --> Commands[nix, home-manager, nixos-rebuild, git, gh]
		TUI --> State[state.json]
		Nix --> Cache[package-catalog.json]
```

## Workspace crates

The workspace uses Rust edition 2024 and contains four crates. The dependency direction is intentionally simple.

| Crate | Responsibility | Local dependencies |
| --- | --- | --- |
| `nixbox-config` | Settings types, defaults, path resolution, JSON loading, and JSON saving. | None. |
| `nixbox-nix` | Package search, package catalog, generated manifests, source scanning and migration, flake discovery and installation, rebuild command selection, output streaming, and cancellation. | None. |
| `nixbox-tui` | Application state, terminal lifecycle, event handling, asynchronous task scheduling, operation queue, recovery state, Vim input behavior, navigation, themes, and rendering. | `nixbox-config`, `nixbox-nix`. |
| `nixbox` | Clap metadata, tracing setup, Tokio runtime, and the call to `nixbox_tui::run`. | All three library crates are declared, although the entry point directly calls only `nixbox-tui`. |

The crates.io publish order is `nixbox-config`, `nixbox-nix`, `nixbox-tui`, then `nixbox`, because the TUI depends on the two leaf libraries and the binary depends on the TUI.

## Startup and terminal lifecycle

`crates/nixbox/src/main.rs` parses `--help` and `--version`, initializes tracing to stderr with a default `warn` filter, and calls `nixbox_tui::run()` on Tokio's multithreaded runtime.

`run()` loads settings and manifests before changing terminal state. It then enables raw mode, enters the alternate screen, selects a blinking bar cursor, and starts the event loop. On normal return it disables raw mode, restores the cursor shape, leaves the alternate screen, and shows the cursor. Errors returned after terminal initialization still pass through this cleanup path because `run()` stores the event-loop result before restoring the terminal.

## Application state

`App` in `crates/nixbox-tui/src/app.rs` owns the complete live state. Important groups are:

- Configuration and manifests: `config`, `home_manifest`, `nixos_manifest`, and `external_packages`.
- Package search: the input editor, results, selected row, epoch, latest query, catalog handle, loading state, and task handles.
- Flake search: a separate input editor, results, selected row, details, epochs, query, loading flags, and task handles.
- Installed view: filter input and selected combined row.
- UI state: active tab, settings page, status, theme index, spinner frame, and quit flag.
- Build state: output log, active flag, cancellation sender, current label, pending queue, in-progress recovery descriptor, and last error.

The application stores package catalogs in `Arc<PackageCatalog>` because searches move a cloned reference into `spawn_blocking` without copying more than 100,000 package documents.

## Event loop

The event loop redraws the full UI, then waits in `tokio::select!` for one of three inputs:

1. A Crossterm terminal event.
2. An `AppEvent` sent through a bounded channel with capacity 128.
3. An 80 ms spinner tick while any search, catalog, detail, or build task is active.

Terminal events mutate the `App` directly or schedule asynchronous work. Background work reports completion through `AppEvent`. Package search, flake search, and flake detail results include monotonically increasing epochs. The event handler ignores a result when its epoch no longer matches current state, so a slow old request cannot overwrite a newer query.

When the user quits, the event loop aborts package-search, flake-search, flake-detail, and catalog tasks. Rebuild work uses its own process cancellation path and persisted recovery state.

## Package operation flow

```mermaid
sequenceDiagram
		participant U as User
		participant T as TUI
		participant Q as Operation queue
		participant F as Local files
		participant R as Rebuild process
		U->>T: Install, remove, migrate, or install flake output
		T->>Q: Validate and enqueue operation
		Q->>F: Apply every queued operation for the first target
		F->>F: Write generated module and ensure import
		F->>F: git add --intent-to-add where applicable
		Q->>R: Start one rebuild for the target batch
		R-->>T: Stream stdout and stderr
		R-->>T: Finished, failed, or cancelled
		T->>Q: Persist state and continue next target when allowed
```

The queue contains `Install`, `InstallFlake`, `InstallFlakePackage`, `Uninstall`, and `Migrate` variants. `drain_queue` takes the target from the first queued operation, removes every queued operation for that target, applies them in their existing order, and launches one rebuild. Operations for the other target remain queued.

File mutation happens before the rebuild. A failed rebuild does not restore previous files. This matches the recovery model: an interrupted rebuild can run again because the generated files already represent the requested state.

## Module map

### `nixbox-config`

- `lib.rs` defines `Config`, `Target`, `InputMode`, default values, recent-search maintenance, settings persistence, and all target path calculations.

### `nixbox-nix`

- `search.rs` owns live search, the locked-revision catalog, cache serialization, bounded subprocess output, package-attribute normalization, and relevance ranking.
- `manifest.rs` owns generated package and flake-module files, package marker parsing, rendering, relative import paths, and insertion into a main module's `imports` list.
- `scan.rs` detects simple package tokens in selected Nix lists and removes dedicated lines during migration.
- `flakes.rs` calls GitHub through `gh api`, ranks repository candidates, evaluates locked revisions for package and module outputs, caches inspections in memory, reads root `flake.nix` files for input names, and edits a conventional root flake for output installation.
- `build.rs` resolves executables in common Nix profiles, chooses Home Manager or NixOS commands, starts rebuild process groups, forwards output, and cancels a complete process group.
- `lib.rs` exports these modules and their main types and functions.

### `nixbox-tui`

- `app.rs` owns `App`, startup scanning, terminal setup, the event loop, visible-tab rules, combined installed-package state, and task cleanup.
- `handlers.rs` translates key presses and `AppEvent` values into state changes.
- `ops.rs` schedules searches, validates operations, mutates manifests, stages changed files for Git-aware flake evaluation, batches the queue, and starts rebuilds.
- `state.rs` serializes and restores pending operations, interrupted rebuilds, and the previous error.
- `vim.rs` implements Unicode-aware cursor movement, Vim word and WORD motions, selection, deletion, and the two-key `dd` command.
- `nav.rs` handles wrapped row selection, tab movement, and settings entry.
- `theme.rs` defines six palettes and their Ratatui styles.
- `ui/` renders the shared bars, package search, flake search and detail panel, installed list, build log, queue, and settings popup.

## Design constraints

- NixBox uses source-text heuristics instead of a full Nix parser for scanning user files, inserting imports, reading flake input names, and updating the root flake. Package and module eligibility comes from pure Nix evaluation of a locked GitHub revision. The mutation code rejects expressions it cannot handle safely, but conventional file structure is still required.
- Search consistency comes from the target's direct locked nixpkgs revision, not from a server-side index or notification system.
- State persistence is best effort. Settings and manifest failures return errors; queue-state save failures do not stop the TUI.
- Generated nixpkgs package sets use `BTreeSet`, and generated external-flake module and package mappings use `BTreeMap`, so output order is deterministic.
- The build log is bounded to 1,000 lines, live-search stdout to 50 MiB, catalog stdout to 256 MiB, search stderr retention to 64 KiB, package results to 200, and flake results to 20.
