# Upgrade old layouts and exercise the installed application inside the guest.

import re
import shlex

UPGRADE_ROOT = "/home/tester/nixbox-upgrade"
UPGRADE_SETTINGS = "/home/tester/nixbox-upgrade-settings"
UPGRADE_REPO = "example/nixbox-upgrade-fixture"


def upgrade(args: str) -> str:
	command = (
		f"env NIXBOX_CONFIG_DIR={UPGRADE_ROOT} XDG_CONFIG_HOME={UPGRADE_SETTINGS} "
		f"nixbox-cli {args}"
	)
	return machine.succeed("su -l tester -c " + shlex.quote(command), timeout=600)


def upgrade_read(name: str) -> str:
	return machine.succeed(f"cat {UPGRADE_ROOT}/{name}")


def upgrade_write(name: str, content: str) -> None:
	parent = name.rsplit("/", 1)[0] if "/" in name else ""
	machine.succeed(f"install -d -o tester -g users {UPGRADE_ROOT}/{parent}")
	command = f"cat > {UPGRADE_ROOT}/{name}"
	machine.succeed(
		"su -l tester -c " + shlex.quote(command) + " <<'NIX'\n" + content + "\nNIX\n"
	)


def upgrade_queue(*ops: dict) -> None:
	state = {"pending_queue": list(ops), "in_progress": None, "last_error": None}
	command = f"cat > {UPGRADE_SETTINGS}/nixbox/state.json"
	machine.succeed(
		"su -l tester -c " + shlex.quote(command)
		+ " <<'JSON'\n" + json.dumps(state) + "\nJSON\n"
	)
	upgrade("resume --no-rebuild --yes")


def upgrade_legacy(home: bool, profile: bool = False) -> str:
	module = "homeModules" if home else "nixosModules"
	option = "nixbox.packages" if profile else ("home.packages" if home else "environment.systemPackages")
	return (
		"# Managed by nixbox. Do not edit by hand.\n"
		"{ inputs, pkgs, ... }:\n{\n"
		"\timports = [\n\t\t# nixbox:flakes:start\n"
		f"\t\tinputs.fixture.{module}.default # github:{UPGRADE_REPO}\n"
		"\t\t# nixbox:flakes:end\n\t];\n"
		f"\t{option} = [\n\t\t# nixbox:flake-packages:start\n"
		f"\t\tinputs.fixture.packages.${{pkgs.system}}.default # github:{UPGRADE_REPO}\n"
		"\t\t# nixbox:flake-packages:end\n\t];\n}\n"
	)


def upgrade_packages_present(home: bool, profile: bool = False) -> None:
	prefix = "homeConfigurations.tester.config" if home else "nixosConfigurations.nixos.config"
	option = "nixbox.packages" if profile else ("home.packages" if home else "environment.systemPackages")
	marker = "home.sessionVariables.NIXBOX_UPGRADE" if home else 'environment.etc."nixbox-upgrade-marker".text'
	query = (
		f"config: {{ paths = map (package: package.outPath) config.{option}; "
		f"marker = config.{marker}; }}"
	)
	command = (
		f"nix eval --impure --json {UPGRADE_ROOT}#{prefix} --apply " + shlex.quote(query)
	)
	result = json.loads(machine.succeed("su -l tester -c " + shlex.quote(command), timeout=600))
	paths = result["paths"]
	assert any(path.endswith("-git-" + git_version) for path in paths), paths
	assert any("-coreutils-" in path for path in paths), paths
	assert result["marker"] == "module-preserved", result


with subtest("seed previous-version Home Manager and NixOS files with a local flake"):
	machine.succeed(f"install -d -o tester -g users {UPGRADE_ROOT} {UPGRADE_SETTINGS}/nixbox")
	upgrade("config set channel flake:nixpkgs")
	machine.succeed(f"cp /home/tester/.config/nixos/guest.nix {UPGRADE_ROOT}/guest.nix")
	machine.succeed(f"chown tester:users {UPGRADE_ROOT}/guest.nix")
	upgrade_system = machine.succeed("nix eval --impure --raw --expr builtins.currentSystem").strip()
	upgrade_home_manager = machine.succeed("readlink -f /etc/nixbox-test-home-manager").strip()
	git_version = machine.succeed("nix eval --impure --raw --expr '(import <nixpkgs> {}).git.version'").strip()
	upgrade_write("fixture/flake.nix", """{
	inputs.nixpkgs.url = "flake:nixpkgs";
	outputs = { nixpkgs, ... }: {
		packages.@SYSTEM@ = {
			default = nixpkgs.legacyPackages.@SYSTEM@.coreutils;
			extra = nixpkgs.legacyPackages.@SYSTEM@.bash;
		};
		nixosModules.default = { ... }: {
			environment.etc."nixbox-upgrade-marker".text = "module-preserved";
		};
		homeModules.default = { ... }: {
			home.sessionVariables.NIXBOX_UPGRADE = "module-preserved";
		};
	};
}""".replace("@SYSTEM@", upgrade_system))
	upgrade_write("flake.nix", """{
	inputs = {
		nixpkgs.url = "flake:nixpkgs";
		home-manager.url = "path:@HOME_MANAGER@";
		home-manager.inputs.nixpkgs.follows = "nixpkgs";
		fixture.url = "github:example/nixbox-upgrade-fixture";
		fixture.inputs.nixpkgs.follows = "nixpkgs";
	};
	outputs = inputs@{ nixpkgs, home-manager, ... }: {
		nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
			system = "@SYSTEM@";
			specialArgs = { inherit inputs; nixpkgsFlake = nixpkgs; };
			modules = [
				({ modulesPath, ... }: { imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ]; })
				./guest.nix ./configuration.nix
			];
		};
		homeConfigurations.tester = home-manager.lib.homeManagerConfiguration {
			pkgs = nixpkgs.legacyPackages.@SYSTEM@;
			extraSpecialArgs = { inherit inputs; };
			modules = [ ./home.nix ];
		};
	};
}""".replace("@SYSTEM@", upgrade_system).replace("@HOME_MANAGER@", upgrade_home_manager))
	for home in [True, False]:
		tag = "home" if home else "system"
		option = "home.packages" if home else "environment.systemPackages"
		upgrade_write(f"nixbox-{tag}-packages.nix", (
			"# Managed by nixbox. Do not edit by hand.\n{ pkgs, ... }:\n{\n"
			f"\t{option} = [\n\t\t# nixbox:packages:start\n\t\tpkgs.git\n"
			"\t\t# nixbox:packages:end\n\t];\n}\n"
		))
		upgrade_write(f"nixbox-{tag}-flakes.nix", upgrade_legacy(home))
		body = '\n\thome.username = "tester";\n\thome.homeDirectory = "/home/tester";\n\thome.stateVersion = "26.05";\n' if home else "\n"
		upgrade_write("home.nix" if home else "configuration.nix", (
			"{ ... }:\n{\n\timports = [\n"
			f"\t\t./nixbox-{tag}-packages.nix\n\t\t./nixbox-{tag}-flakes.nix\n"
			"\t];" + body + "}\n"
		))
	machine.succeed(f"su -l tester -c 'git -C {UPGRADE_ROOT} init -q -b main && git -C {UPGRADE_ROOT} add -A'")
	machine.succeed(
		f"su -l tester -c 'nix flake lock {UPGRADE_ROOT} --override-input fixture path:{UPGRADE_ROOT}/fixture'",
		timeout=600,
	)
	machine.succeed(f"su -l tester -c 'git -C {UPGRADE_ROOT} add -N flake.lock'")
	for home in [True, False]:
		upgrade_packages_present(home)
	upgrade_input_text = upgrade_read("flake.nix")
	upgrade_lock_text = upgrade_read("flake.lock")

with subtest("opening the application upgrades both targets and deletes obsolete files"):
	upgrade("list --all")
	for home in [True, False]:
		tag = "home" if home else "system"
		machine.fail(f"test -e {UPGRADE_ROOT}/nixbox-{tag}-flakes.nix")
		combined = upgrade_read(f"nixbox-{tag}-packages.nix")
		assert "pkgs.git" in combined and "inputs.fixture.packages." in combined, combined
		assert f"# github:{UPGRADE_REPO}" in combined, combined
		entry = upgrade_read("home.nix" if home else "configuration.nix")
		assert "-flakes.nix" not in entry, entry
		assert entry.count(f"nixbox-{tag}-packages.nix") == 1, entry
		upgrade_packages_present(home)
	assert upgrade_read("flake.nix") == upgrade_input_text
	assert upgrade_read("flake.lock") == upgrade_lock_text
	upgrade_files = ["home.nix", "configuration.nix", "nixbox-home-packages.nix", "nixbox-system-packages.nix"]
	upgrade_snapshot = {name: upgrade_read(name) for name in upgrade_files}
	upgrade("list --all")
	assert {name: upgrade_read(name) for name in upgrade_files} == upgrade_snapshot

with subtest("package and flake operations preserve each other after upgrading"):
	for home in [True, False]:
		target = "home-manager" if home else "nixos"
		scope = "home-manager" if home else "nixos-system"
		tag = "home" if home else "system"
		upgrade(f"--target {target} install bash --no-rebuild --yes")
		upgrade_queue({"InstallFlakePackage": {"repo": UPGRADE_REPO, "package": "extra", "scope": scope}})
		text = upgrade_read(f"nixbox-{tag}-packages.nix")
		assert "pkgs.bash" in text and ".extra # github:" in text, text
		upgrade(f"--target {target} remove bash --no-rebuild --yes")
		text = upgrade_read(f"nixbox-{tag}-packages.nix")
		assert "pkgs.bash" not in text and ".extra # github:" in text, text
		upgrade_queue({"UninstallFlakeOutput": {"input": "fixture", "output": {"Package": "extra"}, "scope": scope}})
		text = upgrade_read(f"nixbox-{tag}-packages.nix")
		assert ".extra # github:" not in text and "pkgs.git" in text, text
		upgrade_packages_present(home)

with subtest("real profile builds install both package sources inside the guest"):
	for home in [True, False]:
		target = "home-manager" if home else "nixos"
		upgrade(f"--target {target} profile setup --no-rebuild --yes")
		upgrade_packages_present(home, profile=True)
		# These run the actual profile build and switch its generation.
		upgrade(f"--target {target} remove git --yes")
		upgrade(f"--target {target} install git --yes")
		profile = "/home/tester/.local/state/nix/profiles/nixbox" if home else "/nix/var/nix/profiles/nixbox"
		machine.succeed(f"{profile}/bin/git --version")
		machine.succeed(f"{profile}/bin/ls --version")

with subtest("an upgrade preserves existing profile mode and its installed generation"):
	upgrade_profiles = machine.succeed("readlink -f /home/tester/.local/state/nix/profiles/nixbox /nix/var/nix/profiles/nixbox")
	for home in [True, False]:
		tag = "home" if home else "system"
		name = f"nixbox-{tag}-packages.nix"
		text = upgrade_read(name)
		text = re.sub(r"\timports = \[\n.*?\t\];\n", "", text, flags=re.S)
		text = re.sub(r"^[ \t]*# nixbox:flake-packages:start\n.*?^[ \t]*# nixbox:flake-packages:end\n", "", text, flags=re.S | re.M)
		upgrade_write(name, text)
		upgrade_write(f"nixbox-{tag}-flakes.nix", upgrade_legacy(home, profile=True))
		main = "home.nix" if home else "configuration.nix"
		upgrade_write(main, upgrade_read(main).replace("imports = [", f"imports = [ ./nixbox-{tag}-flakes.nix", 1))
		upgrade_packages_present(home, profile=True)
	upgrade("list --all")
	for home in [True, False]:
		tag = "home" if home else "system"
		assert "# nixbox:mode profile" in upgrade_read(f"nixbox-{tag}-packages.nix")
		machine.fail(f"test -e {UPGRADE_ROOT}/nixbox-{tag}-flakes.nix")
		upgrade_packages_present(home, profile=True)
	assert machine.succeed("readlink -f /home/tester/.local/state/nix/profiles/nixbox /nix/var/nix/profiles/nixbox") == upgrade_profiles

with subtest("profile teardown retains the merged packages and modules"):
	for home in [True, False]:
		target = "home-manager" if home else "nixos"
		tag = "home" if home else "system"
		upgrade(f"--target {target} profile teardown --no-rebuild --yes")
		assert "# nixbox:mode profile" not in upgrade_read(f"nixbox-{tag}-packages.nix")
		upgrade_packages_present(home)
