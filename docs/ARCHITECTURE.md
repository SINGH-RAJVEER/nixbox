# Architecture

## System boundary

NixBox is a local application with three front-ends: a terminal UI, a command line, and a desktop GUI. It does not run a daemon, expose an HTTP API, receive webhooks, or maintain a remote index. The process reads local configuration, calls the `nix`, `home-manager`, `nixos-rebuild`, `git`, and `gh` executables, writes local Nix and JSON files, and renders a Ratatui or gpui interface.

```mermaid
flowchart LR
		GUIUser[Desktop user] --> GUI[nixbox]
		User[Terminal user] --> TUI[nixbox-tui]
		Script[Scripts and SSH] --> CLI[nixbox-cli]
		GUI --> Cmd[nixbox-cmd]
		TUI --> Cmd
		CLI --> Cmd
		GUI --> Core[nixbox-core]
		TUI --> Core
		Cmd --> Core
		Core --> Config[nixbox-config]
		Core --> Nix[nixbox-nix]
		TUI --> Config
		TUI --> Nix
		Config --> Settings[settings.json]
		Nix --> Target[flake.nix and Nix modules]
		Nix --> Commands[nix, home-manager, nixos-rebuild, git, gh]
		Core --> State[state.json]
		Nix --> Cache[package-catalog.json]
```

## Workspace crates

The workspace uses Rust edition 2024 and contains seven crates. The dependency direction is intentionally simple.

| Crate | Responsibility | Local dependencies |
| --- | --- | --- |
| `nixbox-config` | Settings types, defaults, path resolution, JSON loading, JSON saving, and the list of theme names. | None. |
| `nixbox-nix` | Package search, package catalog, generated manifests, source scanning and migration, flake discovery and installation, rebuild command selection, output streaming, and cancellation. | None. |
| `nixbox-core` | The headless engine: the `Op` every front-end queues, the persisted `state.json` format, manifest mutation, import wiring, external-package scanning, flake input and output installation, rebuild command selection and privilege escalation, the NixOS settings browser's shared categories and search, and the `Session` that drives queued ops through rebuilds for the interactive front-ends. Reports progress through a `Reporter` rather than writing to a terminal. | `nixbox-config`, `nixbox-nix`. |
| `nixbox-cmd` | Clap command tree, the non-interactive commands, output rendering, and tracing setup. `run` takes an optional `Frontend` and opens it when no subcommand is given. Shared by all three binaries. | `nixbox-config`, `nixbox-core`, `nixbox-nix`. |
| `nixbox-tui` | The `nixbox-tui` binary, which passes its terminal front-end to `nixbox_cmd::run`, and that front-end: terminal lifecycle, event handling, search scheduling, Vim input behavior, navigation, themes, and rendering. The queue and rebuilds are the `Session`'s. | `nixbox-cmd`, `nixbox-config`, `nixbox-core`, `nixbox-nix`. |
| `nixbox-cli` | The `nixbox-cli` binary: `nixbox_cmd::run` with no front-end. | `nixbox-cmd`. |
| `nixbox` | The `nixbox` binary and the default package. Its default build is a small Nix-backed launcher. The `native` feature builds the gpui desktop front-end over the same `Session`, passes it to `nixbox_cmd::run`, and makes the binary sudo's askpass helper for its system rebuilds. | `nixbox-cmd`, `nixbox-config`, `nixbox-core`, `nixbox-nix` with `native`. |

The crates.io publish order is `nixbox-config`, `nixbox-nix`, `nixbox-core`, `nixbox-cmd`, `nixbox-tui`, `nixbox-cli`, then `nixbox`. The engine sits on the two leaf libraries, the command tree sits on the engine, and every binary sits on the command tree.

`nixbox` is left out of `default-members`, `just ci`, and the workspace-wide checks because its native build needs the graphics stack (Wayland, xkbcommon, X11, Vulkan, fontconfig); `just gui-ci` and a separate CI job enable `native` and cover it. Publishing waits for both the workspace and GUI checks. The flake package also enables `native`, while the default Cargo build needs only the Rust toolchain and runs that flake package through Nix. On macOS, where the GUI is not packaged, the launcher runs the flake's `nixbox-tui` instead.

The three are separate packages so each installs under its own name and pulls in only its own interface. `nixbox-cli` excludes `ratatui`, `crossterm`, `tui-input`, gpui, and both front-end crates. The front-end is a `Frontend` value rather than a Cargo feature, so `nixbox-cmd` has no features and a workspace build compiles it the same way every binary does.

Nothing the command line needs lives in the UI crate: the theme names the settings file accepts are declared in `nixbox-config` and the persisted-state format in `nixbox-core`, each with a test in the UI crate asserting the two stay in step.

The three binaries install under different names, so one profile can hold all of them. `nixbox-cmd` learns which one is running from `env!("CARGO_BIN_NAME")`, passed into `run`, and uses it for the usage line, the generated completion script, and every message that tells the user what to run next.

Every front-end goes through the same engine, so a change applied by `nixbox install`, in the TUI, or in the GUI takes the identical code path. The TUI and the GUI also share the `Session`, so queue batching, rebuild cancellation, the build log, and recovery behave the same in both. The engine is also what keeps `state.json` compatible between them: the queue holds `nixbox_core::Op` values verbatim, and the TUI's `QueuedOp` is a re-export of that type rather than a parallel definition.

## Configuration version control

`nixbox-core::vcs` owns synchronous Git/Jujutsu discovery, initialization, review, commit, origin inspection, explicit push, and optional GitHub repository creation through `gh`. All frontends share this backend. The TUI and GUI dispatch subprocess calls to workers rather than blocking rendering.

`Engine::apply` takes a root-scoped journal lock before file mutation and records successful operations with changed paths before releasing it. Commit holds the same lock, revalidates the displayed review, then commits and clears the journal. Inspection does not depend on journal health. External programs do not honor this lock, so users should avoid editing config files concurrently with a commit.

The nearest repository boundary owns the config; colocated JJ takes precedence over Git at the same root. Git ancestor repositories support path-scoped commits and refuse unrelated staged files. JJ commits in ancestor repositories are refused because the working-copy change can include unrelated files. See [Version control](VERSION_CONTROL.md).

## Startup and terminal lifecycle

`crates/nixbox-cmd/src/lib.rs` parses the command tree and initializes tracing to stderr, driven by a `main` in whichever binary crate was built. With no subcommand it calls the binary's `Frontend::launch` outside any async runtime, because gpui drives its own event loop; the terminal front-end starts a Tokio runtime around `nixbox_tui::run()` itself. With a subcommand it runs that command on Tokio's multithreaded runtime and returns its exit code. The default tracing filter is `warn` for commands and the front-end's own `log_filter` for an interface: `warn` for the TUI, `error` for the GUI, whose renderer warns about every Vulkan driver it skips. Before a command it also restores the default `SIGPIPE` disposition, so piping output into `head` ends the process quietly instead of panicking on a broken pipe.

`run()` loads settings and manifests before changing terminal state. It then enables raw mode, enters the alternate screen, selects a blinking bar cursor, and starts the event loop. On normal return it disables raw mode, restores the cursor shape, leaves the alternate screen, and shows the cursor. Errors returned after terminal initialization still pass through this cleanup path because `run()` stores the event-loop result before restoring the terminal.

## Application state

`App` in `crates/nixbox-tui/src/app.rs` owns the complete live state. Important groups are:

- Package search: the input editor, results, selected row, epoch, latest query, catalog handle, loading state, and task handles.
- Flake search: a separate input editor, results, selected row, details, epochs, query, loading flags, and task handles.
- Installed view: filter input and selected combined row.
- Options panel: the open `OptionsPanel` (package, scope, evaluated options, filter, staged edits, and the active editor), a cache of evaluated option sets keyed by scope and package, an epoch, and the evaluation task handle.
- UI state: active tab, settings page, status, theme index, spinner frame, and quit flag.
- `session`: a `nixbox_core::Session` holding the engine, the pending queue, the build log, the running rebuild and its cancel sender, and the last error.

The application stores package catalogs in `Arc<PackageCatalog>` because searches move a cloned reference into `spawn_blocking` without copying more than 100,000 package documents.

## Event loop

The event loop redraws the full UI, then waits in `tokio::select!` for one of three inputs:

1. A Crossterm terminal event.
2. An `AppEvent` sent through a bounded channel with capacity 128.
3. A `BuildEvent` from the receiver `Session::new` returned, handed to `Session::on_build_event`.
4. An 80 ms spinner tick while any search, catalog, detail, or build task is active.

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

The queue contains `Install`, `InstallFlake`, `InstallFlakePackage`, `UninstallFlake`, `UninstallFlakeOutput`, `Uninstall`, `Migrate`, `MigrateAcross`, `MigrateFlakePackage`, and `SetOptions` variants. `MigrateAcross` belongs to its destination target even though it also edits the source target's main file. `Session::enqueue` skips an op when the same work is already waiting (`Op::duplicates`). `Session::drain` takes the target from the first queued operation, removes every queued operation for that target, applies them in their existing order, and launches one rebuild. Operations for the other target remain queued.

File mutation happens before the rebuild. A failed rebuild does not restore previous files. This matches the recovery model: an interrupted rebuild can run again because the generated files already represent the requested state.

## Module map

### `nixbox-config`

- `lib.rs` defines `Config`, `Target`, `InputMode`, default values, recent-search maintenance, settings persistence, and all target path calculations.

### `nixbox-nix`

- `search.rs` owns live search, the locked-revision catalog, cache serialization, bounded subprocess output, package-attribute normalization, and relevance ranking.
- `manifest.rs` owns generated package and flake-module files, package marker parsing, rendering, relative import paths, and insertion into a main module's `imports` list.
- `settings.rs` owns the generated settings modules: `SettingValue`, the `SettingsManifest` map from option path to value, rendering, and the strict line parser.
- `options.rs` and `options.nix` evaluate a package's module options from the user's flake. The query goes to Nix as JSON in an environment variable, and the result is decoded into an `OptionSet` of `OptionEntry` values with an `OptionKind`, the evaluated value, and the files that define each option.
- `system_options.rs` and `system_options.nix` discover NixOS output names, index host-wide schema metadata, and inspect one option on demand. `NixosConfiguration` binds each inspection to its root and output key, independently of the package-operation target. Schema traversal does not read option values or defaults; it includes nested templates and flags bounded or failed submodule expansion. Selected-value previews have bounded depth and size and withhold credential fields.
- `scan.rs` detects simple package tokens in selected Nix lists and removes dedicated lines during migration.
- `flakes.rs` calls GitHub through `gh api`, ranks repository candidates, evaluates locked revisions for package and module outputs, caches inspections in memory, reads root `flake.nix` files for input names, and edits a conventional root flake for output installation.
- `build.rs` resolves executables in common Nix profiles, chooses Home Manager or NixOS commands, starts rebuild process groups, forwards output, and cancels a complete process group.
- `lib.rs` exports these modules and their main types and functions.

### `nixbox-core`

- `op.rs` defines `Op`, the unit of work both front-ends queue. Its variant and field names are an on-disk format, because the queue is persisted verbatim in `state.json`.
- `state.rs` defines `PersistedState` and `InProgress`, the `state.json` format, and its load, save, and clear operations. It lives here rather than in the UI crate because `nixbox resume` reads the same file.
- `engine.rs` owns `Engine`, which holds the settings and both manifests, applies an `Op`, writes the managed file, wires the import into the main config, stages files for Git-aware flake evaluation, and installs or removes flake inputs and outputs.
- `options.rs` defines `OptionChange`, picks the configuration an option query evaluates (`load_options`), and applies `Op::SetOptions` by writing the settings module and wiring its import. It also holds the editing rules both front-ends share: why an option is locked, the values a choice offers, how typed text is read as a value, how an evaluated value is summarized, which options NixBox sets (`Engine::sets_option`), and what the queue holds for an option (`Session::queued_option`).
- `rebuild.rs` selects the rebuild command for a target, reports when a Home Manager rebuild falls back to `nixos-rebuild`, applies the `Escalation` (plain `sudo`, or `sudo -A` with an askpass helper), and `run` resolves and runs the rebuild, racing the cancel signal. The CLI and the `Session` both use `run`.
- `session.rs` owns `Session`, the operation pipeline behind the TUI and the GUI: deduplicated enqueueing, merging of waiting `SetOptions` ops per scope, per-target batching, one rebuild at a time, cancellation, the capped build log, `state.json` persistence, and restoring an interrupted run. Rebuilds are spawned on a tokio runtime handle, so a front-end whose own loop is not tokio can still drive it.
- `search.rs` searches the package catalog when it is ready and falls back to live `nix search`, and names the catalog cache path.
- `system_options.rs` is the NixOS settings browser's shared policy: the `Category` taxonomy, `filter_options` (path and description search, exact paths first, with per-category counts), `default_host`, and the text both front-ends show for inspected values, read-only options, and partial schema branches.
- `report.rs` defines the `Reporter` trait plus a silent and a log-collecting implementation, which is how the same engine feeds the TUI's log pane and the CLI's stderr.

### `nixbox-cmd`

- `lib.rs` defines `Frontend`, records the running binary's name, parses arguments, starts tracing, and either opens the front-end or restores the default `SIGPIPE` disposition and dispatches.
- `cli.rs` defines the Clap command tree, the global `--target`, `--channel`, and `--json` flags, and dispatch. A binary with a front-end opens it before dispatch, so a bare invocation that reaches `cli.rs` is `nixbox-cli`: it prints help and exits `1`.
- `apply.rs` is the shared path for every command that changes configuration: confirmation, the dry run, writing, the rebuild, and the exit code.
- `commands/` holds one module per subcommand.
- `commands/mod.rs` also holds `search_packages`, the shared lookup behind `search` and `install`: it prefers the cached package catalog and falls back to live search, and an explicit `--channel` always searches live.
- `commands/resume.rs` finishes an interrupted rebuild and drains a queue the TUI left, one target at a time.
- `render.rs` renders tables and field lists for the non-JSON output.

### `nixbox-cli`

- `main.rs` hands `env!("CARGO_BIN_NAME")` and no front-end to `nixbox_cmd::run`.


### `nixbox-tui`

- `main.rs` is the `nixbox-tui` binary: it hands its `Frontend`, which starts a Tokio runtime around `app::run`, to `nixbox_cmd::run`.
- `app.rs` owns `App`, startup scanning, terminal setup, the event loop, visible-tab rules, combined installed-package state, and task cleanup.
- `handlers.rs` translates key presses and `AppEvent` values into state changes.
- `ops.rs` schedules searches, prepares the package catalog, turns the selected row into an `Op`, and reports what the `Session` did with it.
- `state.rs` turns what `Session::restore` picked up into the starting tab and status line.
- `vim.rs` implements Unicode-aware cursor movement, Vim word and WORD motions, selection, deletion, and the two-key `dd` command. Its shared event handler owns text editing for the nixpkgs, Flakes, Installed, and NixOS fields; `handlers.rs` handles result navigation, tab actions, and refreshing results after edits.
- `nixos.rs` holds the NixOS tab: host and schema discovery on first visit, inspection on `Enter`, epoch-guarded results, and invalidation after a rebuild. It never queues work.
- `nav.rs` handles wrapped row selection, tab movement, and settings entry.
- `theme.rs` defines the Ratatui styles for the six palettes named in `nixbox-config::THEMES`, with a test asserting the two lists match.
- `ui/` renders the shared bars, package search, flake search and detail panel, installed list, NixOS browser, build log, queue, and settings popup. The NixOS browser builds only the option rows on screen.

### `nixbox`

- `main.rs` picks the launcher or, with `native`, `native.rs`. The launcher runs `nix run` on the matching tagged flake release, forwarding every argument.
- `native.rs` hands its `Frontend` to `nixbox_cmd::run`. Opening it starts a multithreaded tokio runtime for the nix work, builds a `Session` with `Escalation::Askpass` pointing at its own executable, and opens the window. Started by sudo as the askpass helper (`NIXBOX_ASKPASS` set), it shows the password prompt instead, before the command line is parsed, because sudo passes the prompt text as the only argument.

- `askpass.rs` is that prompt: it prints the password to stdout for sudo, or exits non-zero on cancel so the rebuild stops.
- `app.rs` owns `NixboxApp`, the window state. Searches and flake lookups run on the tokio runtime; gpui tasks await their join handles, debounce input, and discard stale results by epoch, as the TUI does. A gpui task feeds `BuildEvent`s to the `Session` and raises notifications when a rebuild ends.
- `options.rs` holds the Options page's state (`OptionsView`): the package and scope, the evaluated options, the selected option, and staged edits, plus the actions that load, stage, unset, discard, and apply them. Evaluations run on the tokio runtime and are cached per scope and package until a rebuild ends, as in the TUI.
- `nixos.rs` holds the separate read-only `NixosView`, with hosts, schema, filtered indices, per-category counts, selected option, and lazy inspection tasks. It aborts superseded Tokio jobs as well as rejecting stale results by epoch. Build completion and configuration-directory changes invalidate it. It never enqueues a mutation, and its host selection never redirects package work. Edits to source files outside the application need explicit Reload.
- `model.rs` computes the Installed page's filtered sections and a search hit's installed scopes without gpui, so they are unit tested.
- `theme.rs` maps the theme names in `nixbox-config::THEMES` onto gpui-component: `default` follows the desktop appearance, the others are dark with the TUI palette's background and accent.
- `ui/` renders the horizontal top tabs, status line, and the nixpkgs, Flakes, Installed, Options, NixOS, Queue, Build, Version control, and Settings pages with GPUI Kit components, as flat lists separated by rules. `ui/nixos.rs` lays out the logo header, category sidebar, virtualized schema rows, and details, and switches to wrapped categories and full-width details in small windows. `ui/logo.rs` holds the brand logos, and `ui/cursor.rs` draws the Vim Normal block and Visual outline cursors over the filter inputs, whose own line caret serves Insert mode.

## Design constraints

- NixBox uses source-text heuristics instead of a full Nix parser for scanning user files, inserting imports, reading flake input names, and updating the root flake. Package and module eligibility comes from pure Nix evaluation of a locked GitHub revision. The mutation code rejects expressions it cannot handle safely, but conventional file structure is still required.
- Search consistency comes from the target's direct locked nixpkgs revision, not from a server-side index or notification system.
- State persistence is best effort. Settings and manifest failures return errors; queue-state save failures do not stop the TUI or the GUI.
- Nothing stops two front-ends running at once. Both would write `state.json` and the managed files, so running the TUI and the GUI side by side is not supported.
- Generated nixpkgs package sets use `BTreeSet`, and generated external-flake module and package mappings use `BTreeMap`, so output order is deterministic.
- The build log is bounded to 1,000 lines, live-search stdout to 50 MiB, catalog stdout to 256 MiB, search stderr retention to 64 KiB, package results to 200, and flake results to 20.
