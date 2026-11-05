{ lib, ... }:
{
	imports = [ ./hardware-configuration.nix ];
	custom.imported = lib.mkDefault false;
	users.users.alice.active = true;
	services.jobs = [ { name = "configured job"; } ];
	credentials.password = lib.mkForce "fixture-password-never-rendered";
}
