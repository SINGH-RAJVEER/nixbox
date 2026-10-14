# Development and testing

## Repository layout

```text
.
├── Cargo.toml
├── Cargo.lock
├── crates
│	├── nixbox
│	├── nixbox-cli
│	├── nixbox-cmd
│	├── nixbox-config
│	├── nixbox-core
│	├── nixbox-nix
│	└── nixbox-tui
├── docs
├── vm
├── flake.nix
├── flake.lock
├── devenv.nix
├── devenv.yaml
├── devenv.lock
├── justfile
└── .github/workflows/publish.yml
```

The root `Cargo.toml` owns the workspace version, Rust edition, shared metadata, dependencies, and Clippy policy. Each crate inherits that metadata. Internal dependencies in `nixbox-cmd`, `nixbox-tui`, `nixbox-cli`, and `nixbox` include both a local path and the current published version, so a release bump must update the workspace version and every internal dependency version.

## Three binaries, one command tree

The workspace publishes three binaries. They are the same program with a different interface behind a bare invocation:

| Package | Binary | Run without a subcommand |
| --- | --- | --- |
| `nixbox` | `nixbox` | opens the desktop GUI |
| `nixbox-tui` | `nixbox-tui` | opens the terminal UI |
| `nixbox-cli` | `nixbox-cli` | prints help |

Each `main` calls `nixbox_cmd::run`, which holds the whole command tree, and passes the interface as an optional `Frontend`: the `--help` text, the default tracing filter, and a `launch` function that opens it. `nixbox-cli` passes none.

```rust
// crates/nixbox-cli/src/main.rs
fn main() -> ExitCode {
	nixbox_cmd::run(env!("CARGO_BIN_NAME"), None)
}
```

`nixbox-cmd` never depends on a front-end. The front-end crates depend on it, so `nixbox-cli` compiles neither `ratatui` nor gpui, and since there are no Cargo features to unify, a workspace build compiles `nixbox-cmd` exactly the way each binary does.

They install under different names on purpose. Two packages producing the same executable collide in one `target/` directory, which cargo warns about and may one day reject, and the second `cargo install` would silently replace the first. `nixbox-cmd` learns which name is running from `env!("CARGO_BIN_NAME")`, so the usage line, the completion script, and hints like "run `nixbox-cli apply`" all name the command the user actually has.

Anything the command line needs belongs below the front-end crates, not in them. Two things moved down for this reason, each with a test in `nixbox-tui` pinning it to its former home:

- `nixbox_config::THEMES` holds the theme names `nixbox config set theme` validates against. `theme::tests::the_palettes_match_the_names_the_settings_file_accepts` asserts the palettes and the names agree.
- `nixbox_core::state` holds `PersistedState` and `InProgress`, the `state.json` format that `nixbox resume` reads. The UI crate re-exports them.

The Nix flake exposes all three as `packages.nixbox`, `packages.nixbox-tui`, and `packages.nixbox-cli`.

## The desktop GUI

`nixbox` is the default package. Its default Cargo build is a small launcher for the matching tagged flake release, which is what `cargo install nixbox` gives users; `--features native` builds the gpui desktop front-end through `gpui-kit`, which pins gpui and gpui-component together.

Building the native feature needs pkg-config, fontconfig, freetype, Wayland, xkbcommon, X11/xcb, and the Vulkan loader, and running it needs the Vulkan loader and the windowing libraries on the library path. The devenv shell provides all of that. The crate stays out of the workspace-wide commands so they do not build the native feature through test dependency unification:

- The root `Cargo.toml` lists every other crate in `default-members`, so a bare `cargo build` or `cargo nextest run` skips it.
- `just build`, `just check`, `just test`, and `just lint` pass `--exclude nixbox`. `just gui`, `just gui-ci`, and `just release-gui` enable `native` and build it on its own. `just gui` enters the pinned devenv shell automatically; run the other GUI recipes inside that shell.
- CI lints and tests the native feature in its own `gui` job, which installs the graphics libraries first.
- The flake's `nixbox-tui` package runs workspace tests with `--exclude nixbox`, so building the TUI never needs the graphics stack.

To try it without touching your real configuration or queued work, point the settings, cache, and flake directory somewhere disposable:

```sh
XDG_CONFIG_HOME=/tmp/nbx/config XDG_CACHE_HOME=/tmp/nbx/cache \
	NIXBOX_CONFIG_DIR=/tmp/nbx/nixos just gui
```

`app::tests` opens the real window headlessly with gpui-kit's test support and renders every page with a search hit and a rebuild in progress.

## Development environment

The supported development path is the pinned devenv shell:

```sh
devenv shell
```

`devenv.nix` enables stable Rust with `rustc`, Cargo, Clippy, rustfmt, Rust Analyzer, and Rust source. It supplies Bash, `just`, and `cargo-nextest` for build and test recipes; Nix, `nixd`, and `nil` for Nix development; `nixos-rebuild` and `home-manager` for rebuild commands and the VM recipes; and Git, Jujutsu, GitHub CLI, and OpenSSH for configuration repositories and remotes. The native GUI has pkg-config, fontconfig, freetype, Vulkan, Wayland, xkbcommon, X11, and xcb, with runtime libraries on `LD_LIBRARY_PATH`. `desktop-file-validate` checks the packaged desktop entry. `devenv.lock` pins the Nix inputs. With direnv installed, `.envrc` can enter this environment automatically after `direnv allow`.

Privileged rebuilds use the host's configured sudo wrapper. The VM launcher and NixOS test driver obtain QEMU and their Python environment from the Nix derivations, so they do not require separate shell packages. Running the native GUI requires a desktop session and a working host Vulkan driver.

Using the shell matters on NixOS because a Rustup toolchain can retain linker wrappers that point at garbage-collected Nix store paths. If plain Cargo fails inside a Rust linker wrapper while the devenv build succeeds, treat that as a host toolchain problem rather than changing NixBox source.

## Commands

| Command | Purpose |
| --- | --- |
| `just build` | Build the workspace, except the GUI, in debug mode. |
| `just release` | Build the workspace, except the GUI, with release optimizations. |
| `just check` | Type-check the workspace, except the GUI, without code generation. |
| `just test` | Run the workspace test suite, except the GUI's. |
| `just release-cli` | Build the `nixbox-cli` release binary. |
| `just run` | Start the debug terminal UI. |
| `just fmt` | Format every crate with rustfmt. |
| `just lint` | Run Clippy for the workspace and deny warnings. |
| `just fix` | Run formatting and linting. Despite the name, Clippy is not invoked with automatic fixes. |
| `just gui` | Start the debug desktop GUI. Needs the devenv shell. |
| `just gui-ci` | Lint and test the desktop `nixbox` crate on its own. |
| `just release-gui` | Build the desktop `nixbox` release binary. |
| `just clean` | Remove Cargo build artifacts. |
| `just dev` | Enter `devenv shell`. |
| `just dev-update` | Update pinned devenv inputs. |
| `just dev-test` | Evaluate devenv and run its configured test command. |
| `just ci` | Run format, lint, and test in that order. |
| `just ci-vm` | Run `just ci` and then the automated VM check. |
| `just vm-build` | Build the throwaway NixOS test VM. |
| `just vm` | Build and boot the test VM. |
| `just vm-fresh` | Boot the test VM from a clean disk. |
| `just vm-clean` | Delete the test VM's disk image and build result. |
| `just vm-test` | Run the automated VM check. |

The pre-commit gate is:

```sh
devenv shell -- just ci
```

`devenv test` also runs `just ci` through `enterTest`.

## Tests

Core mutation fixtures isolate both `NIXBOX_CONFIG_DIR` and `XDG_CONFIG_HOME` in temporary directories. Configuration files and VCS journals stay in separate roots, and the fixture restores the caller's environment when it is dropped. This lets the same tests run in a Nix sandbox with an unwritable user settings directory. The flake supplies Git and Jujutsu as test dependencies so repository integration tests run inside the sandbox. Check the packaged build with `nix build --no-link .#nixbox-tui`, which runs the workspace tests; evaluating the flake alone does not run its tests.

Tests run under [cargo-nextest](https://nexte.st), which runs each test in its own process. Use `cargo nextest run` rather than `cargo test`; the `just` recipes, CI, and the flake's check phase all do. Nextest does not run doctests, and the crates have none, so add a regular test instead of a doc example when something needs coverage.

The workspace has unit and asynchronous tests in every functional module. The tests cover settings defaults and serialization, package and flake manifest rendering, import insertion, package scanning and removal, search parsing and ranking, lock-file catalog resolution, bounded process output, build cancellation, GitHub result parsing and flake mutation, application epochs and state transitions, keyboard handling, queue batching and deduplication in the `Session`, askpass escalation, recovery serialization, and Unicode-aware Vim motions.

Five tests are ignored during a normal run because they depend on live machine or network state:

- `scan::tests::real_config_detection` reads the maintainer's live Nix configuration paths.
- The ignored Zen Browser flake search test requires authenticated GitHub access and consumes API quota.
- The ignored Bun flake search test requires authenticated GitHub access, Nix evaluation, and GitHub search quota.
- `flakes::tests::for_repo_evaluates_the_flake_it_names` requires authenticated GitHub access and Nix evaluation.
- The ignored live search test requires Nix and a configured nixpkgs registry.

Run ignored tests deliberately and one at a time after reading their source:

```sh
cargo nextest run --workspace --exclude nixbox --run-ignored only --no-capture
```

The project does not contain end-to-end terminal snapshot tests. Unit tests cannot prove that an arbitrary user flake will rebuild successfully, especially where source-text mutation heuristics are involved, which is what the disposable NixOS VM below is for.

## Testing in a VM

Run the checkout's front-ends on your Linux host against one isolated VM configuration:

```sh
just vm-gui                      # open the native desktop GUI
just vm-tui                      # open the terminal UI
just vm-cli                      # show CLI help
just vm-cli install ripgrep --yes # edit the configuration and build its VM
just vm-preview-build            # build without opening a front-end
just vm-preview                  # build and boot the edited configuration
```

These recipes seed `.vm/nixos` once from `vm/seed` and `vm/guest.nix`, using the repository's locked nixpkgs revision and the host's Linux architecture. They run the current checkout's binaries with the NixOS target. GUI, TUI, and CLI share that configuration, with separate preferences, recovery state, VCS journals, and caches under `.vm`. Later launches preserve your edits and settings. `.vm` is ignored by version control.

Apply writes the managed files and runs `nix build <configuration>#nixosConfigurations.nixos.config.system.build.vm`, producing `.vm/result`. It needs no sudo. Boot with `just vm-preview`; stop and restart the guest after another Apply to load the rebuilt system. The preview keeps its disk at `.vm/nixos.qcow2`. The NixOS settings browser remains read-only; package installs, package options, and flake modules use the usual apply workflow. This seeded configuration has no Home Manager module.

The launchers set `NIXBOX_CONFIG_DIR` and the XDG configuration, cache, data, and state directories. They also set `NIXBOX_VM_BUILD` to the absolute output-link path, so every shared rebuild path, including Home Manager fallback and recovery, builds this VM. See [Configuration](CONFIGURATION.md) for the environment variable.

The existing guest-side workflow is separate:

NixBox edits a NixOS configuration and runs rebuilds, so the useful way to try a change is against a throwaway machine rather than your own:

```sh
just vm         # build and boot the test VM
just vm-test    # run the automated check
just vm-clean   # delete the VM's disk image and build result
```

The VM logs in as `tester` (password `tester`, passwordless sudo) with NixBox already on `PATH`. Quit QEMU with `Ctrl-a x`. Inside, `~/.config/nixos` is seeded with a git-tracked flake that exposes `nixosConfigurations.nixos` — which is what NixBox rebuilds — and a `configuration.nix` that declares `hello` by hand, so `nixbox scan` and `nixbox migrate` have something to work with. The VM keeps its state in `nixos.qcow2`; delete it for a clean boot.

A handful of settings in `vm/guest.nix` are what make the guest usable, and none of them are optional:

| setting | why |
| --- | --- |
| `virtualisation.writableStore` | Layers a writable overlay over the host's read-only store. Without it nothing inside the VM can rebuild. |
| `virtualisation.writableStoreUseTmpfs = false` | Puts that overlay on disk instead of in RAM, so an inner rebuild does not run the VM out of memory. |
| `virtualisation.qemu.enableSharedMemory` | virtiofs is vhost-user, so the daemon must be able to map the guest's RAM. Without it every virtiofs mount hangs in uninterruptible sleep and the boot strands in stage 1. The NixOS test driver sets this itself, so only the interactive VM ever needed it spelled out. |
| `boot.loader.grub.enable = false` | QEMU is handed the kernel directly, so there is nothing to install. `grub-install` refuses this disk, and it runs before activation, so leaving GRUB on would fail every rebuild after having built the whole system. |
| `nix.settings.flake-registry` | Locking a flake input resolves indirect references against the global registry only, not the system one, so the default would send every rebuild to channels.nixos.org. It points at the same pin `nix.registry` generates. |

`vm/guest.nix` is shared by the outer VM and the copy seeded inside it. A rebuild started in the guest stops any unit the new configuration does not declare, so the two have to agree about `virtualisation` or the switch would unmount the Nix store out from under itself.

`just vm-test` boots the same machine headless and drives it with the NixOS test driver. Those VMs have no network, so it covers what works offline: `status`, `doctor`, `scan`, `migrate`, `list`, `remove`, `apply`, `config`, the confirmation guard, `--dry-run`, and — because the guest registry pins nixpkgs to a store path — `search` and `install` as far as the managed file. It also drives the TUI's options panel through tmux: opening `git`'s options, staging and applying `programs.git.lfs.enable`, checking the settings module, its import, and Git tracking, confirming that `nix eval` reads the value back, and unsetting it again. A real rebuild is the one thing it cannot do, since anything outside the system closure would have to come from a substituter, so `just vm` is where the `install`/`migrate`/`remove` round trip gets exercised against an actual `nixos-rebuild switch`.

Only `nixos-rebuild build-vm` is ever used here; it builds and never activates, so your own system is untouched. Do not run `nixos-rebuild switch` against `.#nixbox-testvm` — that would apply the test machine's configuration, passwordless sudo and all, to your host.

## Clippy policy

The workspace denies Clippy's `pedantic` and `nursery` groups and adds explicit bans on `unwrap`, `expect`, unchecked indexing and slicing, arithmetic side effects, unreachable code, unimplemented code, unchecked time subtraction, `todo!`, string slicing, panics in result-returning functions, `panic!`, process exit, and unchecked `as` conversions.

`clippy.toml` allows unwrap, expect, panic, and indexing or slicing in tests. Individual source locations may carry narrow lint allowances where a checked invariant cannot be expressed cleanly under the workspace policy.

Run Clippy from the same toolchain as rustc. A Clippy driver from a different Rust version can fail before analyzing the project.

## Logging

Every binary writes tracing output to stderr. Commands and the terminal UI default to the `warn` filter, and the desktop GUI to `error`. Set `RUST_LOG` to increase detail:

```sh
RUST_LOG=debug nixbox-tui
```

The current code has limited tracing calls, so the TUI status line and Building tab often contain more operational detail than the debug log.

## Nix package

The repository flake exports:

- `packages.<system>.nixbox`, the desktop GUI, on Linux systems only. Its wrapper also puts the Vulkan loader, Wayland, xkbcommon, and X11 libraries on `LD_LIBRARY_PATH`, and it installs a desktop entry and icon.
- `packages.<system>.nixbox-tui` and `packages.<system>.nixbox-cli`.
- `packages.<system>.default`, which is `nixbox` on Linux and `nixbox-tui` on macOS.
- `apps.<system>.default`, plus an app for each package and `apps.<system>.nb` on Linux for the short command bundled with the GUI.
- `overlays.default`, which adds `pkgs.nixbox`, `pkgs.nixbox-tui`, and `pkgs.nixbox-cli`.

Supported systems are `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`. Each package builds its own binary targets; the GUI package installs both `nixbox` and `nb` from the same entry point. `nixbox-tui` runs the workspace tests except the GUI's, `nixbox` runs the GUI's with `native`, and `nixbox-cli` runs its own and `nixbox-cmd`'s. The source set includes Cargo metadata, the license, README, and all crates. Every installed executable is wrapped with `gh`, `git`, and `nix` on `PATH`. The test VM and `checks.<system>.cli` use `nixbox-tui`, since the VM has no display.

Useful checks are:

```sh
nix flake check
nix build .#nixbox .#nixbox-tui .#nixbox-cli
nix run . -- --version
```

The Cargo-installed binary does not receive the Nix wrapper, so users must provide its external commands themselves.

## Version control

This repository uses Jujutsu with a colocated Git repository. Inspect both the working copy and bookmark state before changing or publishing work:

```sh
jj status
jj bookmark list -a
git status --short
```

Create focused commits and move `dev` to the completed commit when appropriate:

```sh
jj commit -m "docs: describe package catalog"
jj bookmark move dev --to @-
```

Do not describe a local commit as pushed. Compare `dev`, `master`, and their remote bookmarks before release work.

## Release preparation

Published crates must use one version across the workspace. Before merging a release into `master`:

1. Choose the next semantic version and replace `workspace.package.version` in the root manifest.
2. Update every internal dependency version in the crate manifests to match the workspace version, including the `nixbox-core` test dependencies.
3. Regenerate `Cargo.lock` with the pinned development toolchain.
4. Move the current `Unreleased` notes to a dated version heading and add a fresh empty `Unreleased` section.
5. Run `just ci`, `nix flake check`, and a release build.
6. Inspect the Jujutsu diff and commit only the intended release files.
7. Merge the verified `dev` commit into `master` through the project's normal review workflow.

`.github/workflows/publish.yml` runs on pushes to `master` and on pull requests targeting `master`. It contains three jobs.

The `test` job mirrors the local `just ci` gate on the stable toolchain: `cargo fmt --all --check`, `cargo clippy --workspace --exclude nixbox --all-targets --locked -- -D warnings`, and `cargo nextest run --workspace --exclude nixbox --locked`, followed by a step that builds `nixbox-tui` and `nixbox-cli` and checks their names, help, and that a bare `nixbox-cli` prints help and fails, and a dependency-free build and argument-forwarding check for both `nixbox` and `nb` launchers. The separate `gui` job installs the graphics development libraries, then lints and tests `nixbox` with `native` enabled. Both run on pull requests, so a release reports the gates before the merge happens.

The `publish` job declares `needs: [test, gui]` and is restricted to `push` events, so it never runs from a pull request and never starts unless both checks succeeded. It installs the graphics development libraries, checks the locked workspace including the GUI, creates the `v<version>` tag for the flake launcher, then publishes `nixbox-config`, `nixbox-nix`, `nixbox-core`, `nixbox-cmd`, `nixbox-tui`, `nixbox-cli`, and `nixbox` in dependency order. A later push at the same version does not move the tag or republish. A `concurrency` group keeps two pushes from racing up that chain, and never cancels a run in flight, because a half-published chain is worse than a queued one. It treats an already-published version as a skip, so re-running a failed publish is safe and does not require another version bump. Publishing requires the `CARGO_REGISTRY_TOKEN` repository secret; an expired or revoked token fails the upload with `403 Forbidden: authentication failed`.

The workspace denies every `pedantic` and `nursery` lint, and those sets change between Rust releases. The development shell and release workflow both run stable. The shell pins its toolchain through `devenv.lock`, so refresh the pinned inputs when CI advances to a newer stable release.

Before merging a release into `master`, confirm the workspace version is newer than every published crate version. Publishing skips a version that already exists on crates.io.

Nothing after `0.2.0` reached crates.io before `0.2.3`: the `0.2.1` and `0.2.2` publishes both failed on an expired token, which is why `nixbox-core` had no released version at all despite two attempts. A failed publish leaves no trace on crates.io, so a version that failed to upload can be reused; a version that uploaded cannot, even after a yank.

## Documentation style

Project Markdown keeps each prose paragraph, list item, and table row on one source line. Do not insert hard line breaks to satisfy a column limit. Code blocks retain the line structure required by the code or command examples.

Use sentence-case headings, state exact paths and commands, and document destructive behavior and parser limitations next to the feature that triggers them.
