#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mode=${1:?expected seed, cli, build, or run}
shift
vm_dir="$repo/.vm/dendritic"
config_dir="$vm_dir/nixos"

if [[ "$mode" == seed ]]; then
	source_dir=${1:?expected the source dendritic configuration directory}
	if [[ -e "$config_dir" ]]; then
		printf 'VM configuration already exists at %s; it was left intact.\n' "$config_dir" >&2
		exit 1
	fi
	[[ -f "$source_dir/flake.nix" && -d "$source_dir/modules" ]]
	mkdir -p "$vm_dir"
	seed_dir=$(mktemp -d "$vm_dir/seed.XXXXXX")
	trap 'rm -rf -- "$seed_dir"' EXIT
	# Exclude VCS metadata so generated-file visibility cannot touch the source
	# repository through a copied worktree pointer. The copy uses a path flake.
	tar -C "$source_dir" --exclude=.git --exclude=.jj --exclude=.direnv \
		--exclude=.devenv --exclude=result --exclude='*.qcow2' -cf - . \
		| tar -C "$seed_dir" -xf -
	if [[ -e "$seed_dir/modules/nixbox-vm.nix" ]]; then
		printf 'The source already has modules/nixbox-vm.nix; choose a copy without that reserved test filename.\n' >&2
		exit 1
	fi
	cp "$repo/vm/dendritic.nix" "$seed_dir/modules/nixbox-vm.nix"
	mv -T --no-clobber "$seed_dir" "$config_dir"
	rmdir "$seed_dir" 2>/dev/null || true
	trap - EXIT
	jj git init --colocate "$config_dir"
	printf 'Seeded disposable dendritic configuration at %s\n' "$config_dir"
	exit 0
fi

[[ -f "$config_dir/flake.nix" ]] || { printf 'Seed a dendritic configuration first.\n' >&2; exit 1; }
export XDG_CONFIG_HOME="$vm_dir/preferences"
export XDG_CACHE_HOME="$vm_dir/cache"
export XDG_DATA_HOME="$vm_dir/data"
export XDG_STATE_HOME="$vm_dir/state"
export NIXBOX_CONFIG_DIR="$config_dir"
# Every CLI activation builds the guest. It never switches the host or
# changes a package profile on the host.
export NIXBOX_VM_BUILD="$vm_dir/result"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"
cd "$repo"
case "$mode" in
	cli) exec cargo run -p nixbox-cli -- "$@" ;;
	build|run)
		nix build --print-build-logs --max-jobs 1 --cores 2 --out-link "$NIXBOX_VM_BUILD" \
			"path:$config_dir#nixosConfigurations.nixos.config.system.build.vm"
		if [[ "$mode" == run ]]; then
			cd "$vm_dir"
			export NIX_DISK_IMAGE="$vm_dir/nixos.qcow2"
			# Keep writable shares and runner state inside the disposable area.
			# Shell overrides must not add host devices, shares, or boot options.
			unset QEMU_OPTS QEMU_NET_OPTS QEMU_KERNEL_PARAMS NIXPKGS_QEMU_KERNEL_nixos
			export TMPDIR="$vm_dir/tmp"
			export USE_TMPDIR=1
			export SHARED_DIR="$vm_dir/shared"
			mkdir -p "$TMPDIR" "$SHARED_DIR"
			exec "$NIXBOX_VM_BUILD/bin/run-nixos-vm"
		fi
		;;
	*) printf 'Unknown dendritic VM mode: %s\n' "$mode" >&2; exit 2 ;;
esac
