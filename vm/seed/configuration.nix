# The hand-written configuration inside the VM — the file nixbox scans for
# packages you declared yourself, and the one it adds a single `imports`
# entry to.
#
# `hello` is here so `nixbox scan` has something to find and `nixbox migrate`
# has something to move.
{ pkgs, ... }:
{
	imports = [ ];

	environment.systemPackages = with pkgs; [
		hello
	];
}
