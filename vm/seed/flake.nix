# Seeded into the VM at ~/.config/nixos. This is the flake nixbox rebuilds,
# so it has to expose `nixosConfigurations.nixos`.
#
# The `inputs = { ... }` block and the `outputs = inputs@{ ... }` binding are
# both load-bearing: `nixbox flake add` looks for exactly those two shapes
# before it will wire a flake module in.
{
	description = "Throwaway NixOS configuration for the nixbox test VM";

	inputs = {
		# Resolved through the registry to the nixpkgs already in the store,
		# so rebuilding inside the VM downloads nothing.
		nixpkgs.url = "flake:nixpkgs";
	};

	outputs =
		inputs@{ self, nixpkgs }:
		{
			nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
				system = "@system@";
				specialArgs = {
					inherit inputs;
					nixpkgsFlake = nixpkgs;
				};
				modules = [
					(
						{ modulesPath, ... }:
						{
							imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ];
						}
					)
					./guest.nix
					./configuration.nix
				];
			};
		};
}
