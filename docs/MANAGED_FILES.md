# Managed files and package operations

## Ownership rule

NixBox owns every file whose first line is `# Managed by nixbox. Do not edit by hand.` It parses only its marker blocks and rewrites the whole generated file from an ordered in-memory manifest. Manual changes inside or outside a marker block can disappear on the next operation.

NixBox also edits the target's main module to add an import and may edit `flake.nix` during flake-module installation. Those user-owned files are changed in place. NixBox does not create backups and does not roll changes back after a failed rebuild. Commit the configuration repository before performing a large migration or flake installation.

## Generated package modules

In configuration mode, the default, Home Manager packages are written to `<configuration-root>/nixbox-home-packages.nix`:

```nix
# Managed by nixbox. Do not edit by hand.
{ pkgs, ... }:
{
	home.packages = [
		# nixbox:packages:start
		pkgs.fd
		pkgs.ripgrep
		# nixbox:packages:end
	];
}
```

NixOS packages are written to `<configuration-root>/nixbox-system-packages.nix`:

```nix
# Managed by nixbox. Do not edit by hand.
{ pkgs, ... }:
{
	environment.systemPackages = [
		# nixbox:packages:start
		pkgs.git
		# nixbox:packages:end
	];
}
```

Package names use a `BTreeSet`, so duplicates are removed and generated lines sort lexically. Loading a generated file accepts only non-comment lines between the package markers that begin with `pkgs.`.

## Independent package profiles

`nixbox profile setup` switches one target into profile mode, and `nixbox profile teardown` switches it back. Nothing else changes a target's mode: installs, removals and `apply` keep whatever mode the generated file is in. The mode is the `# nixbox:mode profile` line in the package file. A file without it, including every file written before profile mode existed, is in configuration mode. Keeping the mode in the file rather than in `settings.json` means it travels with the configuration repository to other machines.

In profile mode the package file declares the packages as `nixbox.packages` and builds them into one environment, `nixbox.profile`:

```nix
# Managed by nixbox. Do not edit by hand.
# nixbox:mode profile
{ config, lib, pkgs, ... }:
let
	profile = "${config.home.homeDirectory}/.local/state/nix/profiles/nixbox";
	nix = if (config.nix.package or null) != null then config.nix.package else pkgs.nix;
in
{
	options.nixbox = {
		packages = lib.mkOption { ... };
		profile = lib.mkOption { ... };
	};
	config = {
		nixbox.packages = [
			# nixbox:packages:start
			pkgs.fd
			# nixbox:packages:end
		];
		nixbox.profile = pkgs.buildEnv {
			name = "nixbox-packages";
			paths = config.nixbox.packages;
			...
		};
		home.activation.nixboxProfile = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
			if [ "$(readlink -f ${lib.escapeShellArg profile})" != ${config.nixbox.profile} ]; then
				run mkdir -p ${lib.escapeShellArg (dirOf profile)}
				run ${nix}/bin/nix-env --profile ${lib.escapeShellArg profile} --set ${config.nixbox.profile}
			fi
		'';
		home.sessionPath = [ "${profile}/bin" ];
		xdg.systemDirs.data = [ "${profile}/share" ];
		programs.nushell.extraEnv = lib.mkIf config.programs.nushell.enable ''...'';
		home.file.".local/state/nixbox/profile-ready".text = profile;
	};
}
```

The NixOS file has the same options. Its wiring is `environment.profiles`, a `system.activationScripts.nixboxProfile` entry with the same `readlink` guard, and `/etc/nixbox/profile-ready`. Flake packages share the same `nixbox.packages` list as nixpkgs packages, so they join the same environment.

| Target | Profile | Owner |
| --- | --- | --- |
| Home Manager | `~/.local/state/nix/profiles/nixbox` | You |
| NixOS | `/nix/var/nix/profiles/nixbox` | root, through `sudo` |

### Package changes

A package-only change runs `nix flake lock <configuration-root>` as you, so a build under sudo never rewrites `flake.lock` as root, and then builds the option itself into a new profile generation:

```sh
sudo nix build --no-link --profile /nix/var/nix/profiles/nixbox <configuration-root>#nixosConfigurations.nixos.config.nixbox.profile
nix build --no-link --profile ~/.local/state/nix/profiles/nixbox <configuration-root>#homeConfigurations.<USER>.config.nixbox.profile
nix build --no-link --profile ~/.local/state/nix/profiles/nixbox <configuration-root>#nixosConfigurations.nixos.config.home-manager.users.<USER>.nixbox.profile
```

The Home Manager attribute follows the same detection as the rebuild command: a standalone `homeConfigurations.<USER>` output, or Home Manager inside the `nixos` configuration. A missing configuration fails the build. A failed build leaves the previous generation active, and removing the last package builds an empty environment.

Installs, removals and flake packages update only the profile. Module installations, option changes, migrations, removal of a module or of a hand-declared flake package, setup and teardown rebuild the configuration. A batch containing any of them rebuilds once and builds nothing separately, because the rebuild sets the profile itself.

### Rebuilds set the profile

The activation script sets the profile to `nixbox.profile` whenever the profile points anywhere else. It runs on every rebuild and, on NixOS, at every boot. The package build and the rebuild use the same option, so they produce the same store path, and an ordinary rebuild leaves the profile alone. In return the configuration stays the source of truth:

- On a fresh machine, the first rebuild installs every package.
- `nixos-rebuild --rollback`, the boot menu, and `home-manager` generation switches restore the matching package set.
- After `nix flake update`, the rebuild moves the profile to the new nixpkgs.

`nixbox profile rollback` and `--to` switch the profile's own generations with `nix-env`. That undo lasts until the next install, `apply`, rebuild or reboot: NixOS runs activation scripts at boot, and so does the Home Manager service when Home Manager runs inside NixOS. Each sets the profile from the generated file again.

### PATH and desktop entries

On NixOS, `environment.profiles` adds the profile to the PAM environment, which every shell and the desktop session inherit.

Home Manager's `home.sessionPath` only reaches shells that source `hm-session-vars.sh`, such as bash, zsh and fish, so the generated file also adds the profile to nushell's `env.nu`. When Home Manager runs inside NixOS, setup also writes `nixbox-home-integration.nix` and imports it from the NixOS main file:

```nix
# Managed by nixbox. Do not edit by hand.
# Puts the nixbox Home Manager profile on every user's PATH through PAM,
# which shells that skip Home Manager's session variables still read.
{ ... }:
{
	environment.profiles = [ "$HOME/.local/state/nix/profiles/nixbox" ];
}
```

Setup fails before writing anything when that NixOS main file is missing. Teardown empties the module and keeps its import.

Start a new login session after setup. `status` and `doctor` report one of four states: not set up, set up but not activated on this machine, activated but not on this process's PATH, or active. The check reads the profile-ready marker and the PATH of the running NixBox process, so a shell started before setup, `sudo`, or a desktop launcher can differ from a fresh login.

### Teardown

Teardown rewrites the combined package module in configuration mode and rebuilds. Nixpkgs packages, flake packages and flake module imports stay in that one file. The profile and its generations stay on disk and off PATH. Delete them with `rm ~/.local/state/nix/profiles/nixbox*` or `sudo rm /nix/var/nix/profiles/nixbox*`, then collect garbage.

## Flake outputs in package modules

`nixbox-home-packages.nix` and `nixbox-system-packages.nix` also store outputs selected from external flake inputs. Each file can have an import marker block backed by a `BTreeMap` from the `owner/repository` key to a module output path. The manifest format also has a package marker block backed by a second `BTreeMap` from the repository to its set of package attributes, so one flake can contribute several packages; Home Manager renders that block as `home.packages`, while NixOS renders it as `environment.systemPackages`. A third map records the root flake input each repository is referenced through, such as `zen-browser`.

The input declarations stay in the root `flake.nix`. The package modules reference them as `inputs.<name>` and request the `inputs` argument only when flake outputs are present. Module outputs go in a top-level `imports` list. Nixpkgs and flake packages share one package list, with separate marker blocks so changing either preserves the other. A combined Home Manager file looks like this:

```nix
# Managed by nixbox. Do not edit by hand.
{ inputs, pkgs, ... }:
{
	imports = [
		# nixbox:flakes:start
		inputs.module.homeManagerModules.default # github:owner/module
		# nixbox:flakes:end
	];
	home.packages = [
		# nixbox:packages:start
		pkgs.fd
		# nixbox:packages:end
		# nixbox:flake-packages:start
		inputs.package.packages.${pkgs.stdenv.hostPlatform.system}.default # github:owner/package
		# nixbox:flake-packages:end
	];
}
```

Input and package attribute names are quoted and escaped only when they are not plain Nix identifiers. Each line ends with a `# github:<owner>/<repository>` comment that ties it back to its repository. The loader also reads lines written by earlier releases, which used the quoted repository as the input name, had no trailing comment, and used `${pkgs.system}`; those are accepted during the automatic upgrade and rewritten with the current system selector. On the Home Manager target the Flakes tab enqueues a flake-package operation whenever the selected repository has packages, and on the NixOS target when it has no default module, using the first package from the evaluated and ranked package list.

### Upgrading existing configurations

When the shared engine starts, NixBox automatically folds `nixbox-home-flakes.nix` into `nixbox-home-packages.nix` and `nixbox-system-flakes.nix` into `nixbox-system-packages.nix`. This runs for both targets in the CLI, TUI and GUI, including when opening the application or listing packages. It preserves nixpkgs packages, flake packages, module outputs, repository comments, input names and profile mode. Input declarations and `flake.lock` stay unchanged, and the migration itself does not build or activate anything.

The upgrade scans Nix files under the configured root and the configured Home Manager and NixOS entry files. It updates literal path entries in import lists, including relative paths in nested files, and removes redundant imports when the combined module is already in that binding. Comments, unrelated strings and other configuration text stay in place. The old generated files are deleted after the combined contents and import edits have been written. Missing package files are created, and repeated startup does not duplicate entries. Newly created package files are made visible to Git-backed flakes.

Before writing, NixBox checks that the generated files contain only recognised generated code and that the import edits are supported. Unexpected hand edits, incomplete markers, conflicting input names, string or computed imports, and imports through symlinked Nix files stop the upgrade with an error instead of deleting the source files. Move those custom declarations into a user-owned module or use literal import paths, then start NixBox again. References outside the configured root and entry files must be updated separately. A write failure restores the files already changed during that target's migration.

`just vm-test` boots a guest and exercises this upgrade with the packaged CLI, real Home Manager and NixOS modules, and a local flake fixture. It checks migration in configuration and profile mode, package and flake-output edits after migration, repeated startup, and profile builds that install runnable commands inside the guest.

## Generated settings modules

Package options set from the [options panel](PACKAGE_OPTIONS.md) are written to `<configuration-root>/nixbox-home-settings.nix` for Home Manager and `<configuration-root>/nixbox-system-settings.nix` for NixOS:

```nix
# Managed by nixbox. Do not edit by hand.
{ ... }:
{
	# nixbox:settings:start
	programs.git.ignores = [ ".direnv" "*.swp" ];
	programs.git.signing.format = "ssh";
	# nixbox:settings:end
}
```

Each option is one `path = value;` line, sorted by path. Path segments that are not plain Nix identifiers are quoted. Values are limited to `null`, booleans, integers, floats, strings, and lists of strings. Strings escape `\`, `"`, newlines, tabs, and `${`, so a value can never become an interpolation. When loading, NixBox accepts only lines it could have written itself and drops everything else on the next write, including hand-added lines inside the markers.

NixBox writes an option only when no other file under the configuration root defines it, because a second definition of a non-mergeable value fails evaluation. See [What can be edited](PACKAGE_OPTIONS.md#what-can-be-edited).

## Automatic imports

After writing a generated module, NixBox ensures the target entry file imports it. It calculates a relative path when possible and checks for the complete path token before editing, which makes repeated insertion idempotent and avoids treating `.nix.bak` as the same import.

If an `imports = [ ... ]` list exists outside a quoted string or line comment, NixBox inserts the path immediately after `[`. It detects whether the list is multiline and copies the indentation of its first non-empty entry. If no imports list exists, NixBox looks for the outer module body after the function-argument colon and inserts a new list near the opening brace.

The import editor is a text scanner, not a Nix parser. It understands line comments and double-quoted strings during keyword search, but unusual bindings, indented strings, computed attributes, or unconventional module structure may prevent insertion or select the wrong syntactic location. NixBox reports a warning when it cannot add the import. If the main file is missing, it leaves the generated file in place and tells the user to import it manually.

`ensure_home_nix` exists in `nixbox-nix` and can generate a minimal Home Manager file, but the current TUI startup and operation flow do not call it.

## Git visibility

Nix flakes ignore untracked files in a Git work tree. After mutation, NixBox checks whether each changed path is inside Git and runs this equivalent command:

```sh
git -C <parent-directory> add --intent-to-add -- <path>
```

This does not commit the file or stage its full contents. It makes a newly generated path visible to flake evaluation. Failure is recorded as a warning in the build log. Outside a Git work tree, the step is skipped. A colocated Jujutsu repository normally exposes the Git work tree needed by this check, but NixBox invokes `git`, not `jj`.

## Installing a package

Pressing `Enter` in package search performs these checks before queuing work:

1. If a local catalog exists, verify that it still matches `flake.lock`. A mismatch starts catalog refresh and postpones installation.
2. Require a selected result.
3. Reject a package already managed or already queued for the selected target.
4. Append an `Install` operation to the queue and persist recovery state.

When the queue drains, NixBox adds the package attribute to the target manifest, writes the generated module, adds its import, marks changed paths for Git visibility, and starts a rebuild, or a profile build for a target in profile mode. A build already in progress leaves the new operation queued.

## Removing a package

The Installed tab allows removal only for packages already in a NixBox manifest. NixBox rejects direct removal of an external declaration and asks the user to migrate it first. A queued uninstall removes the attribute from the target set, rewrites the generated module, and rebuilds the target or its profile.

Removing a package does not remove an empty generated module or its import. The generated file remains valid with an empty package marker block.

## External-package scanning

On startup and after successful rebuilds, NixBox scans the configured Home Manager and NixOS entry files. It searches assignment lists whose final attribute segment ends in `Packages`, `packages`, `Portals`, `portals`, `Themes`, or `themes`. Lists containing `with pkgs;` also qualify when their attribute falls within the selected target scope. Names containing `exclude` or `disable` are rejected.

Home Manager scanning accepts `home.*` attributes and a bare `packages` attribute. NixOS scanning rejects `home.*` attributes and accepts other package-like paths such as `environment.systemPackages`, `fonts.packages`, `xdg.portal.extraPortals`, and `hardware.graphics.extraPackages`.

The scanner accepts `pkgs.<attribute>` tokens and bare tokens inside `with pkgs;` lists. Dotted attributes are supported. It rejects strings, function calls, nested lists, interpolations, `inputs.*`, `self.*`, invalid Nix identifiers, and entries with extra tokens on the same dedicated line.

Same-line declarations such as `[ pkgs.git pkgs.curl ]` are detected for display but marked non-migratable because deleting one token without a parser could damage the expression. Dedicated one-token lines at the outer list depth are migratable.

The scanner strips text after `#` as a comment without parsing string context. A `#` inside a quoted string can therefore confuse detection, although string expressions are rejected as package entries.

## Migrating external packages

Migration rescans the source file, removes every dedicated line associated with the requested package names, adds the names to the generated target manifest, writes the generated module, and rebuilds. The remover preserves all other lines and whether the file ended with a newline.

Migration is not transactional across the source file and generated module. If removing source lines succeeds and a later write or rebuild fails, the files remain in their latest state. Use version control as the rollback mechanism.

Flake-package migration records the existing GitHub input and package attribute in the generated flake module, adds its import, then removes the dedicated source line. It preserves the root input and other outputs. Inline entries, non-GitHub inputs, and hand-declared module imports are not migrated automatically. If a write fails between these steps, inspect both files before retrying.

Migrating to the other target removes the dedicated lines from the source target's main file and adds the names to the other target's generated manifest, in one operation that belongs to the destination. NixBox imports the destination's generated module before it edits the source, and refuses the move when the destination has no main file, so a Home Manager-only machine never loses a declaration to a NixOS target it does not have. The GUI bubble and the TUI `o` hint only appear when the other target exists. Only the destination rebuilds. The source target keeps the package installed until its next rebuild, which picks up the removed lines.

`M` queues separate Home Manager and NixOS migration operations. Queue batching produces at most one rebuild per affected target for that migration run.

## Queue and recovery

Each queued operation records its target. `drain_queue` takes the target of the first item, extracts every queued operation for that target, applies them in order, and starts one rebuild if at least one file mutation succeeded. It leaves operations for the other target in the queue.

Before a rebuild starts, NixBox stores the active target, label and activation in `state.json`: `packages` for a profile build, `configuration` for a rebuild. Records without one, and the `packages-and-configuration` value an unreleased build wrote, mean a rebuild. The generated files have already been written at this point. On restart, an in-progress descriptor causes NixBox to run the same activation again before draining the pending queue. If there is no interrupted rebuild, queued operations resume immediately.

Successful rebuilds clear the previous error, rescan external packages, persist state, and continue the queue. Failed rebuilds keep the changed files, save the error, and continue draining queued work. Cancellation clears the active rebuild descriptor but leaves pending operations paused.

## Rebuild command selection

For the NixOS target, NixBox runs:

```sh
sudo nixos-rebuild switch --flake <configuration-root>#nixos
```

For the Home Manager target, NixBox first evaluates whether the root flake exports `homeConfigurations.<USER>`. If it does, NixBox uses an installed `home-manager` executable when available:

```sh
home-manager switch --flake <configuration-root>#<USER>
```

If `home-manager` is not found in common Nix profiles, NixBox falls back to:

```sh
nix run nixpkgs#home-manager -- switch --flake <configuration-root>#<USER>
```

If no standalone Home Manager output exists or the detection evaluation fails, NixBox assumes Home Manager is integrated as a NixOS module and uses the NixOS rebuild command.

Executable discovery checks `/run/wrappers/bin`, the user's Nix profile, the per-user system profile, the current system profile, and the default Nix profile before falling back to `PATH`.

## Build output and cancellation

The child process runs in a new Unix process group so wrapper programs and their descendants can be cancelled together. Separate tasks read newline-delimited stdout and stderr and send both streams to the TUI. Blank lines are retained because the build reader trims only line endings. The UI keeps the newest 1,000 lines.

Cancellation sends `SIGTERM` to the negative process-group ID. After three seconds it sends `SIGKILL` and reaps the child. This implementation depends on Unix process-group APIs and is not portable to native Windows.
