default:
    @just --list

# ── Cargo ──────────────────────────────────────────────────────────────────────

# Build debug binary
build:
    cargo build --workspace

# Build release binary
release:
    cargo build --workspace --release

# Type-check without codegen
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Run the TUI (debug build)
run:
    cargo run

# `nixbox` turns on nixbox-cmd's `tui` feature and cargo unifies features
# across a workspace build, so `cargo test --workspace` always compiles the
# UI in. The CLI configuration only gets covered if it is built on its own.

# Lint and test the CLI packages on their own
cli:
    cargo clippy -p nixbox-cli -p nixbox-cmd --all-targets -- -D warnings
    cargo test -p nixbox-cli -p nixbox-cmd

# Build the nixbox-cli release binary
release-cli:
    cargo build --release -p nixbox-cli

# Format all crates
fmt:
    cargo fmt --all

# Lint with clippy
lint:
    cargo clippy --workspace -- -D warnings

# Format + lint
fix: fmt lint

# Remove build artifacts
clean:
    cargo clean

# ── Test VM ────────────────────────────────────────────────────────────────────

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

# Full pre-commit gate: format, lint, test, CLI-only build
ci: fmt lint test cli

# Full gate plus the VM check, which boots a machine and takes minutes
ci-vm: ci vm-test
