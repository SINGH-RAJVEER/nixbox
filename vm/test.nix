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

	nodes.machine = {
		imports = [ ./host.nix ];
	};

	testScript = builtins.readFile ./test-script.py;
}
