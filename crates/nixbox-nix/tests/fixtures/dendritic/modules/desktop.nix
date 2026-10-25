{ ... }:
{
	flake.modules.nixos.desktop = {
		environment.systemPackages = [ ];
	};
	flake.modules.homeManager.desktop = { pkgs, ... }: {
		home.username = "rajveer";
		home.homeDirectory = "/home/rajveer";
		home.stateVersion = "26.05";
		home.packages = [ pkgs.git ];
	};
}
