default:
	@just --list

# ── Cargo ──────────────────────────────────────────────────────────────────────

# Build debug binary
build:
	cargo build --workspace --exclude nixbox

# Build release binary
release:
	cargo build --workspace --exclude nixbox --release

# Type-check without codegen
check:
	cargo check --workspace --exclude nixbox

# The desktop front-end is excluded from the workspace-wide recipes: it needs
# the graphics stack, which only the devenv shell provides. `just gui-ci`
# covers it.

# Run all tests
test:
	cargo nextest run --workspace --exclude nixbox

# Run the terminal UI (debug build)
run:
	cargo run -p nixbox-tui

# Build the nixbox-cli release binary
release-cli:
	cargo build --release -p nixbox-cli

# Run the desktop GUI in the pinned devenv shell (debug build)
gui:
	devenv shell -- cargo run -p nixbox --features native

# Build the nixbox desktop release binary
release-gui:
	cargo build --release -p nixbox --features native

# Lint and test the desktop GUI
gui-ci:
	cargo clippy -p nixbox --features native --all-targets -- -D warnings
	cargo nextest run -p nixbox --features native

# Format all crates
fmt:
	cargo fmt --all

# --all-targets so test code is linted too, matching CI.

# Lint with clippy
lint:
	cargo clippy --workspace --exclude nixbox --all-targets -- -D warnings

# Format + lint
fix: fmt lint

# Remove build artifacts
clean:
	cargo clean

# ── Test VM ────────────────────────────────────────────────────────────────────

# Run the checkout's GUI against an isolated VM configuration; Apply builds the VM
[positional-arguments]
vm-gui *args:
	devenv shell -- bash vm/dev.sh gui "$@"

# Run the checkout's TUI against the same VM configuration
[positional-arguments]
vm-tui *args:
	devenv shell -- bash vm/dev.sh tui "$@"

# Run a CLI command against the same VM configuration (defaults to help)
[positional-arguments]
vm-cli *args="--help":
	devenv shell -- bash vm/dev.sh cli "$@"

# Build the configuration edited by vm-gui, vm-tui, and vm-cli
vm-preview-build:
	devenv shell -- bash vm/dev.sh build

# Build and boot that configuration; restart after Apply to see the new system
vm-preview:
	devenv shell -- bash vm/dev.sh run

# Flakes read the git tree, so a file added under vm/ is invisible to the
# build until it is tracked. `path:.` would sidestep that, at the cost of
# copying target/ into the store on every build.

# Build the throwaway NixOS test VM
vm-build:
	nixos-rebuild build-vm --flake .#nixbox-testvm

# Build and boot the test VM (login: tester/tester, quit with ctrl-a x)
vm: vm-build
	./result/bin/run-nixos-vm

# The config dir under ~/.config/nixos is seeded once, by an activation
# script that skips itself if the directory is already there. Exercising
# that path needs the old disk gone first.

# Boot the test VM from a clean disk
vm-fresh: vm-clean vm

# Delete the test VM's disk image and build result
vm-clean:
	rm -f nixos.qcow2 result

# --no-link because vm-build and vm own ./result: without it a bare vm-test
# leaves the check's output there and `just vm` boots the wrong thing.

# Run the automated VM check
vm-test:
	nix build --no-link --print-build-logs ".#checks.$(nix eval --raw --impure --expr builtins.currentSystem).cli"

# ── Dev ────────────────────────────────────────────────────────────────────────

# Enter the devenv shell
dev:
	devenv shell

# Update pinned devenv inputs
dev-update:
	devenv update

# Evaluate the devenv configuration and run its tests
dev-test:
	devenv test

# Full pre-commit gate: format, lint, test
ci: fmt lint test telemetry-ci

# Validate the optional Axum receiver and local telemetry delivery.
telemetry-ci:
	cargo clippy -p nixbox-core --features telemetry-receiver --all-targets -- -D warnings
	cargo nextest run -p nixbox-core --features telemetry-receiver -E 'test(telemetry::)'

# Full gate plus the VM check, which boots a machine and takes minutes
ci-vm: ci vm-test
