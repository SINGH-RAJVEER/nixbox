# The outer half of the test VM: everything that only makes sense on the
# machine building it. The seeded copy inside the guest imports `guest.nix`
# alone, so nothing here can drift into the configuration nixbox rebuilds.
{
	lib,
	pkgs,
	modulesPath,
	nixbox,
	nixboxCli,
	...
}:
let
	configDir = "/home/tester/.config/nixos";

	# What lands in the guest's config directory. `guest.nix` is copied in so
	# the inner configuration is built from the same settings as the outer one.
	seed = pkgs.runCommand "nixbox-testbed-config" { } ''
		mkdir -p $out
		substitute ${./seed/flake.nix} $out/flake.nix \
			--subst-var-by system ${pkgs.stdenv.hostPlatform.system}
		cp ${./seed/configuration.nix} $out/configuration.nix
		cp ${./guest.nix} $out/guest.nix
	'';
in
{
	imports = [
		# `nixos-rebuild build-vm` adds this itself; importing it explicitly
		# means `nix flake check` can evaluate the configuration too.
		"${modulesPath}/virtualisation/qemu-vm.nix"
		./guest.nix
	];

	system.activationScripts.nixboxTestbed = lib.stringAfter [ "users" ] ''
		# Outside the seed guard: a rebuild started inside the VM drops
		# anything the inner configuration does not declare, and it has no way
		# to refer to this nixbox build.
		# `install -d` only applies -o/-g to the last component, so every
		# parent has to be named or it stays root-owned — which is enough to
		# stop nixbox from writing its own settings file.
		install -d -o tester -g users -m 0755 /home/tester/.config
		install -d -o tester -g users -m 0755 /home/tester/.local
		install -d -o tester -g users -m 0755 /home/tester/.local/bin
		ln -sfn ${lib.getExe nixbox} /home/tester/.local/bin/nixbox
		chown -h tester:users /home/tester/.local/bin/nixbox
		# The other published binary, so the test can prove it behaves the
		# same on a real machine.
		ln -sfn ${lib.getExe nixboxCli} /home/tester/.local/bin/nixbox-cli
		chown -h tester:users /home/tester/.local/bin/nixbox-cli

		if [ ! -e ${configDir}/.git ]; then
			# Stays root-owned until the commit lands. Activation runs from
			# the initrd, where git has no SUDO_UID to fall back on, so a
			# worktree owned by anyone else is "dubious ownership" and every
			# git command after `init` fails with status 128.
			install -d -m 0755 ${configDir}
			cp -r ${seed}/. ${configDir}/
			chmod -R u+w ${configDir}
			# Flakes ignore untracked files, which is the whole reason nixbox
			# runs `git add -N`. Without a repo here the rebuild would not see
			# the configuration at all.
			${pkgs.git}/bin/git -C ${configDir} init -q -b main
			${pkgs.git}/bin/git -C ${configDir} add -A
			${pkgs.git}/bin/git -C ${configDir} \
				-c user.name=nixbox -c user.email=nixbox@localhost \
				commit -q -m "seed the testbed configuration"
			chown -R tester:users ${configDir}
		fi
	'';
}
