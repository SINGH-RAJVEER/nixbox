# NixBox documentation

The README is the short entry point. These documents describe the behavior that matters when using, debugging, changing, or releasing NixBox.

## User documentation

- [User guide](USER_GUIDE.md) covers installation, startup, tabs, controls, settings, package operations, rebuild behavior, and shutdown.
- [Desktop GUI](GUI.md) covers the default `nixbox` package, its pages, how system rebuilds ask for your password, and its limits.
- [Terminal applications](TERMINAL_APPS.md) explains separate flake installation of the GUI, TUI, and CLI.
- [Command line interface](CLI.md) covers the non-interactive subcommands, their flags, the confirmation rules, resuming interrupted work, the nixbox-cli package, and the exit codes.
- [Configuration and stored state](CONFIGURATION.md) lists every setting, default path, environment override, generated file, cache file, and recovery file.
- [Package search](SEARCH.md) explains catalog creation, revision pinning, invalidation, ranking, live fallback, resource limits, and measured performance.
- [Managed files and package operations](MANAGED_FILES.md) explains package manifests, automatic imports, external-package scanning, migration, queuing, rebuild selection, cancellation, recovery, and the mutations NixBox performs.
- [Package options](PACKAGE_OPTIONS.md) explains package options in the terminal UI and the desktop GUI: where options are read from, which ones can be edited, the editors and keys, and how changes are applied.
- [GitHub flake browser](FLAKE_BROWSER.md) explains GitHub authentication, discovery, ranking, detail inspection, module installation, required flake structure, and current limitations.
- [Troubleshooting](TROUBLESHOOTING.md) maps common symptoms to checks and fixes.

## Maintainer documentation

- [Architecture](ARCHITECTURE.md) describes crate ownership, dependencies, the asynchronous event loop, state transitions, data flow, and module boundaries.
- [Development and testing](DEVELOPMENT.md) covers the pinned environment, commands, Cargo features, tests, lint rules, Nix packaging, Jujutsu usage, and release preparation.
- [Release notes](RELEASE_NOTES.md) records published changes and the work planned for the next release from `dev`.
- [Optional telemetry](TELEMETRY_PLAN.md) describes disabled-by-default collection, consent controls, report fields, the shared recipient, receiver deployment, certificate renewal, and validation.
- [NixOS settings tab plan](NIXOS_SETTINGS_PLAN.md) records the implemented browser and simple-type editor, and the remaining phases for complete option discovery, structured editing, validation, and multi-host writes.
- [Deterministic commit messages](COMMIT_MESSAGES.md) describes ordered operation summaries, journal boundaries, and the primary implementations used as references.

## Source of truth

The Rust implementation remains authoritative when documentation and behavior disagree. Settings live in `nixbox-config`, Nix-facing mutations live in `nixbox-nix`, the engine and the queue-and-rebuild `Session` live in `nixbox-core`, the separately installed CLI's command tree lives in `nixbox-cmd`, and the desktop and terminal front-ends live in `nixbox` and `nixbox-tui`.
