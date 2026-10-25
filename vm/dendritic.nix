# VM-only support for testing package-profile activation on a fresh disk.
{ ... }:
{
	flake.modules.nixos.nixos.virtualisation.vmVariant =
		{ config, ... }:
		{
			# Recent NixOS boots run activation in the initrd, before the QEMU
			# module's stage-2 register-nix-paths service. nix-env needs those
			# paths registered even though the shared store is already mounted.
			# Activation visits attribute names in order. Bootstrap first, since
			# a generated string-valued script cannot merge a separate deps set.
			system.activationScripts."000-nixboxStoreRegistration".text = ''
				if [[ "$(cat /proc/cmdline)" =~ regInfo=([^[:space:]]*) ]]; then
					${config.nix.package}/bin/nix-store --load-db < "''${BASH_REMATCH[1]}"
				fi
			'';
		};
}
