#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mode=${1:?expected gui, tui, cli, build, or run}
shift
case "$mode" in
	gui|tui|cli|build|run) ;;
	*) printf 'Unknown VM mode: %s\n' "$mode" >&2; exit 2 ;;
esac

vm_dir="$repo/.vm"
config_dir="$vm_dir/nixos"
mkdir -p "$vm_dir"

# Seed once. Later launches retain edits, manifests, settings, and the disk.
if [[ ! -e "$config_dir/flake.nix" ]]; then
	if [[ -e "$config_dir" ]]; then
		printf 'Incomplete VM configuration at %s; move it aside before seeding.\n' "$config_dir" >&2
		exit 1
	fi
	system=$(nix eval --raw --impure --expr builtins.currentSystem)
	if [[ "$system" != *-linux ]]; then
		printf 'The development VM requires a Linux host.\n' >&2
		exit 1
	fi
	seed_dir=$(mktemp -d "$vm_dir/seed.XXXXXX")
	trap 'rm -rf -- "$seed_dir"' EXIT
	sed -e "s/@system@/$system/g" \
		-e 's|flake:nixpkgs|github:NixOS/nixpkgs/nixos-unstable|' \
		"$repo/vm/seed/flake.nix" > "$seed_dir/flake.nix"
	cp "$repo/vm/seed/configuration.nix" "$repo/vm/guest.nix" "$seed_dir/"
	cp "$repo/flake.lock" "$seed_dir/flake.lock"
	git -C "$seed_dir" init -q -b main
	git -C "$seed_dir" add -A
	git -C "$seed_dir" -c user.name=nixbox -c user.email=nixbox@localhost \
		commit -q -m 'seed the host-edited VM configuration'
	# Publish only the complete seed. Concurrent first launches keep the winner.
	mv -T --no-clobber "$seed_dir" "$config_dir"
	rm -rf -- "$seed_dir"
	trap - EXIT
fi

# Keep preferences, recovery queues, caches, and VCS journals out of the
# user's usual XDG directories. Preserve the host's display/runtime variables.
export XDG_CONFIG_HOME="$vm_dir/config"
export XDG_CACHE_HOME="$vm_dir/cache"
export XDG_DATA_HOME="$vm_dir/data"
export XDG_STATE_HOME="$vm_dir/state"
export NIXBOX_CONFIG_DIR="$config_dir"
export NIXBOX_VM_BUILD="$vm_dir/result"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"
cd "$repo"

case "$mode" in
	gui) exec cargo run -p nixbox --features native -- --target nixos "$@" ;;
	tui) exec cargo run -p nixbox-tui -- --target nixos "$@" ;;
	cli) exec cargo run -p nixbox-cli -- --target nixos "$@" ;;
	build|run)
		nix build --print-build-logs --out-link "$NIXBOX_VM_BUILD" \
			"$config_dir#nixosConfigurations.nixos.config.system.build.vm"
		if [[ "$mode" == run ]]; then
			cd "$vm_dir"
			exec "$NIXBOX_VM_BUILD/bin/run-nixos-vm"
		fi
		;;
esac
