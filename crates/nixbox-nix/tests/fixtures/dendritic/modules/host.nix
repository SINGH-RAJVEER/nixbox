{ config, inputs, ... }:
{
	imports = [ inputs.flake-parts.flakeModules.modules ];
	systems = [ "x86_64-linux" ];
	flake.nixosConfigurations.nixos = inputs.nixpkgs.lib.nixosSystem {
		modules = [ config.flake.modules.nixos.nixos ];
	};
	flake.modules.nixos.nixos = {
		imports = [ config.flake.modules.nixos.desktop ];
		home-manager.users.rajveer = config.flake.modules.homeManager.desktop;
	};
}
