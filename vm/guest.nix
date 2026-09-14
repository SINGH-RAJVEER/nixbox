# Configuration shared by the VM as it is built on the host and by the copy
# seeded inside the guest.
#
# Both sides must agree: nixbox runs `nixos-rebuild switch` from inside the
# VM, and a switch stops any unit the new configuration no longer declares.
# If the inner configuration disagreed about `virtualisation`, the switch
# would tear the Nix store mount out from under the running system. Keeping
# every VM setting here — rather than in `virtualisation.vmVariant`, which
# only applies to the outer build — is what keeps the two identical.
{
	lib,
	pkgs,
	nixpkgsFlake,
	...
}:
let
	# The same mapping `nix.registry` writes to /etc/nix/registry.json, as a
	# standalone file. Referring to the generated one would make nix.settings
	# depend on environment.etc, which depends back on nix.settings.
	registry = pkgs.writeText "nixbox-testbed-registry.json" (builtins.toJSON {
		version = 2;
		flakes = [
			{
				from = {
					id = "nixpkgs";
					type = "indirect";
				};
				to = {
					type = "path";
					path = nixpkgsFlake.outPath;
				}
				// lib.filterAttrs (name: _: builtins.elem name [
					"lastModified"
					"narHash"
					"rev"
					"revCount"
				]) nixpkgsFlake;
				exact = true;
			}
		];
	});
in
{
	# nixbox rebuilds `<config dir>#nixos`, so the attribute and the host name
	# both have to be `nixos`.
	networking.hostName = "nixos";

	# QEMU is handed the kernel and initrd directly, so there is no boot
	# loader to install. Leaving GRUB enabled would not just be redundant:
	# `grub-install` refuses this disk, and it runs before activation, so
	# every `nixos-rebuild switch` nixbox starts would fail after having
	# built the whole system.
	boot.loader.grub.enable = false;

	users.users.tester = {
		isNormalUser = true;
		password = "tester";
		extraGroups = [ "wheel" ];
		uid = 1000;
	};
	users.users.root.password = "root";
	# nixbox shells out to `sudo nixos-rebuild switch`.
	security.sudo.wheelNeedsPassword = false;
	services.getty.autologinUser = "tester";

	# Keeps the nixbox symlink seeded into ~/.local/bin on PATH, including
	# after a rebuild started from inside the VM.
	environment.localBinInPath = true;

	environment.systemPackages = [
		pkgs.git
		pkgs.jq
		pkgs.vim
	];

	nix.settings = {
		# Locking a flake input resolves indirect references against the
		# global registry only — the system registry below is not consulted,
		# so leaving this at its default would make every rebuild reach for
		# channels.nixos.org. Point it at the very file the pin generates.
		flake-registry = registry;
		experimental-features = [
			"nix-command"
			"flakes"
		];
		trusted-users = [
			"root"
			"tester"
		];
	};

	# Resolve `flake:nixpkgs` to the copy already in the host store, so the
	# guest never downloads a second nixpkgs to rebuild itself.
	nix.registry.nixpkgs.flake = nixpkgsFlake;
	nix.nixPath = [ "nixpkgs=${nixpkgsFlake}" ];

	virtualisation = {
		# virtiofs is vhost-user, so the daemon has to be able to map the
		# guest's RAM. Without a shared memory backend every virtiofs mount
		# hangs in uninterruptible sleep — including the Nix store overlay,
		# which strands the boot in stage 1. `runNixOSTest` turns this on
		# itself, so only the interactive VM ever noticed.
		qemu.enableSharedMemory = true;
		memorySize = 6144;
		cores = 4;
		# MiB. Inner rebuilds need room for a second system closure.
		diskSize = 16384;
		# Serial console; quit QEMU with Ctrl-a x.
		graphics = false;
		# Without this the store is the host's, read-only, and nothing inside
		# the VM can rebuild anything.
		writableStore = true;
		# The overlay defaults to a tmpfs, which an inner rebuild fills until
		# the VM runs out of memory. Put it on the VM's own disk instead.
		writableStoreUseTmpfs = false;
		forwardPorts = [
			{
				from = "host";
				host.port = 2222;
				guest.port = 22;
			}
		];
	};

	system.stateVersion = lib.trivial.release;
}
