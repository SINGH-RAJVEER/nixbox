# Automated VM check for the nixbox CLI.
#
# The script it runs lives in ./test-script.py — the test driver type-checks
# it, so keeping it in a real .py file means editors and the checker see the
# same thing.
{
	nixbox,
	nixboxCli,
	nixpkgsFlake,
	testers,
}:
testers.runNixOSTest {
	name = "nixbox-cli";

	node.specialArgs = { inherit nixbox nixboxCli nixpkgsFlake; };

	nodes.machine =
		{ pkgs, ... }:
		{
			imports = [ ./host.nix ];
			# Gives the check a terminal to run the TUI in and read it back
			# from. Kept out of host.nix, which the interactive VM shares.
			environment.systemPackages = [ pkgs.tmux ];
		};

	testScript = builtins.readFile ./test-script.py;
}
