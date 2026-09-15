# NixBox documentation

The README is the short entry point. These documents describe the behavior that matters when using, debugging, changing, or releasing NixBox.

## User documentation

- [User guide](USER_GUIDE.md) covers installation, startup, tabs, controls, settings, package operations, rebuild behavior, and shutdown.
- [Configuration and stored state](CONFIGURATION.md) lists every setting, default path, environment override, generated file, cache file, and recovery file.
- [Package search](SEARCH.md) explains catalog creation, revision pinning, invalidation, ranking, live fallback, resource limits, and measured performance.
- [Managed files and package operations](MANAGED_FILES.md) explains package manifests, automatic imports, external-package scanning, migration, queuing, rebuild selection, cancellation, recovery, and the mutations NixBox performs.
- [GitHub flake browser](FLAKE_BROWSER.md) explains GitHub authentication, discovery, ranking, detail inspection, module installation, required flake structure, and current limitations.
- [Troubleshooting](TROUBLESHOOTING.md) maps common symptoms to checks and fixes.

## Maintainer documentation

- [Architecture](ARCHITECTURE.md) describes crate ownership, dependencies, the asynchronous event loop, state transitions, data flow, and module boundaries.
- [Development and testing](DEVELOPMENT.md) covers the pinned environment, commands, tests, lint rules, Nix packaging, Jujutsu usage, and release preparation.
- [Release notes](RELEASE_NOTES.md) records published changes and the work planned for the next release from `dev`.

## Source of truth

The Rust implementation remains authoritative when documentation and behavior disagree. Settings live in `nixbox-config`, Nix-facing mutations live in `nixbox-nix`, application state and event handling live in `nixbox-tui`, and `crates/nixbox` only starts the application.
