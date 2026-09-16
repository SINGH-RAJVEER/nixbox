# Driven by vm/test.nix against the VM that vm/host.nix describes.
#
# The test VM has no network, so everything here is scoped to what works
# offline. `nixbox search` counts: it resolves against the nixpkgs pinned
# into the guest registry. A real rebuild does not — it needs a substituter
# for anything outside the system closure — so it belongs in the interactive
# VM (`just vm`) rather than here.

start_all()
machine.wait_for_unit("multi-user.target")


def nixbox(args: str) -> str:
	return machine.succeed("su -l tester -c 'nixbox " + args + "'")


def nixbox_fails(args: str) -> str:
	return machine.fail("su -l tester -c 'nixbox " + args + "'")


def managed() -> str:
	return machine.succeed("cat /home/tester/.config/nixos/nixbox-system-packages.nix")


def main_config() -> str:
	return machine.succeed("cat /home/tester/.config/nixos/configuration.nix")


def declared_in_main_config() -> str:
	# Comments in the seeded file mention the package by name, so only the
	# code is meaningful when asking what is still declared.
	lines = main_config().splitlines()
	return "\n".join(line for line in lines if not line.lstrip().startswith("#"))


with subtest("the testbed was seeded"):
	machine.succeed("test -d /home/tester/.config/nixos/.git")
	machine.succeed("test -f /home/tester/.config/nixos/flake.nix")
	machine.succeed("test -f /home/tester/.config/nixos/guest.nix")

with subtest("status reports the seeded configuration"):
	status = nixbox("status")
	assert "nixos" in status, status
	assert "/home/tester/.config/nixos" in status, status

with subtest("doctor passes on a well-formed testbed"):
	doctor = nixbox("doctor")
	assert "fail" not in doctor, doctor

with subtest("scan finds the hand-declared package"):
	scan = nixbox("scan")
	assert "hello" in scan, scan
	assert "environment.systemPackages" in scan, scan

with subtest("migrate moves it into the managed file"):
	nixbox("migrate hello --no-rebuild --yes")
	assert "pkgs.hello" in managed(), managed()
	assert "hello" not in declared_in_main_config(), main_config()

with subtest("the managed file is imported and tracked by git"):
	assert "./nixbox-system-packages.nix" in main_config(), main_config()
	tracked = machine.succeed("su -l tester -c 'git -C ~/.config/nixos ls-files'")
	assert "nixbox-system-packages.nix" in tracked, tracked

with subtest("list reports what was migrated, as text and as json"):
	assert "hello" in nixbox("list")
	parsed = machine.succeed("su -l tester -c 'nixbox list --json' | jq -r '.[0].name'")
	assert parsed.strip() == "hello", parsed

with subtest("a change without a terminal needs --yes"):
	nixbox_fails("remove hello --no-rebuild < /dev/null")
	assert "hello" in nixbox("list")

with subtest("a dry run writes nothing"):
	nixbox("remove hello --no-rebuild --dry-run")
	assert "hello" in nixbox("list")

with subtest("remove drops it again"):
	nixbox("remove hello --no-rebuild --yes")
	assert "pkgs.hello" not in managed(), managed()

with subtest("removing something nixbox does not manage is an error"):
	nixbox_fails("remove nosuchpackage --no-rebuild --yes")

with subtest("apply rewrites a managed file that went missing"):
	machine.succeed("rm /home/tester/.config/nixos/nixbox-system-packages.nix")
	nixbox("apply --no-rebuild --yes")
	machine.succeed("test -f /home/tester/.config/nixos/nixbox-system-packages.nix")

with subtest("settings round-trip through the config command"):
	nixbox("config set channel nixpkgs-unstable")
	assert nixbox("config get channel").strip() == "nixpkgs-unstable"
	nixbox_fails("config set theme nosuchtheme")

with subtest("completions are generated for a real shell"):
	assert "_nixbox" in nixbox("completions bash")

with subtest("search resolves against the nixpkgs pinned into the registry"):
	nixbox("config set channel flake:nixpkgs")
	assert "hello" in nixbox("search hello")

with subtest("install resolves a name and declares it"):
	nixbox("install hello --no-rebuild --yes")
	assert "pkgs.hello" in managed(), managed()
	assert "hello" in nixbox("list")
	nixbox("remove hello --no-rebuild --yes")

# Shaped exactly as the TUI writes it: `nixbox-core::Op` is serialized
# verbatim, so this doubles as a check that the on-disk format has not moved.
QUEUED_STATE = """{
  "pending_queue": [
    {
      "Install": {
        "hit": {
          "attr": "hello",
          "pname": "hello",
          "version": "2.12.2",
          "description": "greeting"
        },
        "scope": "nixos-system"
      }
    }
  ],
  "in_progress": null,
  "last_error": null
}"""

with subtest("resume is a no-op with nothing saved"):
	# Progress goes to stderr, so it has to be folded in to be asserted on.
	assert "Nothing to resume" in nixbox("resume 2>&1")

with subtest("resume applies a queue the TUI could have left behind"):
	machine.succeed(
		"su -l tester -c 'cat > ~/.config/nixbox/state.json' <<'JSON'\n"
		+ QUEUED_STATE
		+ "\nJSON\n"
	)
	nixbox("resume --no-rebuild --yes")
	assert "pkgs.hello" in managed(), managed()
	# Applied work is dropped, so a second resume has nothing to do.
	machine.fail("test -f /home/tester/.config/nixbox/state.json")
	nixbox("remove hello --no-rebuild --yes")

with subtest("nixbox-cli is the same program without the UI"):
	# Both binaries are on PATH, so this compares them on one machine rather
	# than trusting that the shared command tree behaves the same.
	cli = machine.succeed("su -l tester -c 'nixbox-cli --help'")
	assert "Usage: nixbox-cli" in cli, cli
	assert "no terminal UI" in cli, cli
	# The `tui` subcommand is feature-gated away. The message matters as
	# much as the failure: a nixbox-cli that still carried the UI would
	# also fail here, by having no terminal to open.
	rejected = machine.fail("su -l tester -c 'nixbox-cli tui' 2>&1")
	assert "unrecognized subcommand 'tui'" in rejected, rejected
	bare = machine.fail("su -l tester -c 'nixbox-cli 2>&1'")
	assert "This is nixbox-cli" in bare, bare
	# Hints name the binary that was actually run.
	assert "nixbox-cli" in machine.succeed(
		"su -l tester -c 'nixbox-cli completions bash' | tail -5"
	)
	# And it does real work against the same managed file.
	machine.succeed("su -l tester -c 'nixbox-cli install hello --no-rebuild --yes'")
	assert "pkgs.hello" in managed(), managed()
	assert "hello" in nixbox("list")
	machine.succeed("su -l tester -c 'nixbox-cli remove hello --no-rebuild --yes'")

with subtest("resume --discard throws the queue away instead"):
	machine.succeed(
		"su -l tester -c 'cat > ~/.config/nixbox/state.json' <<'JSON'\n"
		+ QUEUED_STATE
		+ "\nJSON\n"
	)
	nixbox("resume --discard")
	machine.fail("test -f /home/tester/.config/nixbox/state.json")
	assert "hello" not in nixbox("list")
