# Automated VM check for the nixbox CLI.
#
# The script it runs lives in ./test-script.py — the test driver type-checks
# it, so keeping it in a real .py file means editors and the checker see the
# same thing.
{
	nixboxTui,
	nixboxCli,
	nixpkgsFlake,
	testers,
}:
testers.runNixOSTest {
	name = "nixbox-cli";

	node.specialArgs = { inherit nixboxTui nixboxCli nixpkgsFlake; };

	nodes.machine =
		{ pkgs, ... }:
		{
			imports = [ ./host.nix ];
			# Gives the check a terminal to run the TUI in and read it back
			# from. Kept out of host.nix, which the interactive VM shares.
			environment.systemPackages = [ pkgs.tmux ];
			# Exercise the Home Manager target with its real module system,
			# using the version pinned by this test's nixpkgs, entirely offline.
			environment.etc."nixbox-test-home-manager".source = pkgs.home-manager.src;
			# The guest has no substituters. Populate the real profile closures
			# before boot so `nix build --profile` can update generations there.
			system.extraDependencies = let
				profile = paths: pkgs.buildEnv {
					name = "nixbox-packages";
					inherit paths;
					pathsToLink = [ "/bin" "/sbin" "/share" "/lib" "/libexec" "/etc" ];
					extraOutputsToInstall = [ "man" ];
				};
			in [ (profile [ pkgs.coreutils ]) (profile [ pkgs.git pkgs.coreutils ]) ];
		};

	# Run upgrade behavior immediately after boot, before the existing CLI
	# and TUI checks, so fixture failures do not repeat unrelated scenarios.
	testScript = builtins.replaceStrings
		[ "machine.wait_for_unit(\"multi-user.target\")\n" ]
		[ ("machine.wait_for_unit(\"multi-user.target\")\n" + builtins.readFile ./upgrade-test.py + "\n") ]
		(builtins.readFile ./test-script.py);
}
