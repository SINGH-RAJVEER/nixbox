# Managed files and package operations

## Ownership rule

NixBox owns every file whose first line is `# Managed by nixbox. Do not edit by hand.` It parses only its marker blocks and rewrites the whole generated file from an ordered in-memory manifest. Manual changes inside or outside a marker block can disappear on the next operation.

NixBox also edits the target's main module to add an import and may edit `flake.nix` during flake-module installation. Those user-owned files are changed in place. NixBox does not create backups and does not roll changes back after a failed rebuild. Commit the configuration repository before performing a large migration or flake installation.

## Generated package modules

Home Manager packages are written to `<configuration-root>/nixbox-home-packages.nix`:

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

## Generated flake-output modules

`nixbox-home-flakes.nix` and `nixbox-system-flakes.nix` store outputs selected from external flake inputs. Each file has an import marker block backed by a `BTreeMap` from the `owner/repository` key to a module output path. The manifest format also has a package marker block backed by a second `BTreeMap` from the repository to its set of package attributes, so one flake can contribute several packages; Home Manager renders that block as `home.packages`, while NixOS renders it as `environment.systemPackages`. A third map records the root flake input each repository is referenced through, such as `zen-browser`.

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
		# nixbox:flake-packages:start
		inputs.package.packages.${pkgs.stdenv.hostPlatform.system}.default # github:owner/package
		# nixbox:flake-packages:end
	];
}
```

Input and package attribute names are quoted and escaped only when they are not plain Nix identifiers. Each line ends with a `# github:<owner>/<repository>` comment that ties it back to its repository. The loader also reads lines written by earlier releases, which used the quoted repository as the input name, had no trailing comment, and used `${pkgs.system}`; those keep working until the flake is reinstalled. On the Home Manager target the Flakes tab enqueues a flake-package operation whenever the selected repository has packages, and on the NixOS target when it has no default module, using the first package from the evaluated and ranked package list.

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

When the queue drains, NixBox adds the package attribute to the target manifest, writes the generated module, adds its import, marks changed paths for Git visibility, and starts a rebuild. A build already in progress leaves the new operation queued.

## Removing a package

The Installed tab allows removal only for packages already in a NixBox manifest. NixBox rejects direct removal of an external declaration and asks the user to migrate it first. A queued uninstall removes the attribute from the target set, rewrites the generated module, and rebuilds the target.

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

`M` queues separate Home Manager and NixOS migration operations. Queue batching produces at most one rebuild per affected target for that migration run.

## Queue and recovery

Each queued operation records its target. `drain_queue` takes the target of the first item, extracts every queued operation for that target, applies them in order, and starts one rebuild if at least one file mutation succeeded. It leaves operations for the other target in the queue.

Before a rebuild starts, NixBox stores the active target and label in `state.json`. The generated files have already been written at this point. On restart, an in-progress descriptor causes NixBox to run the same target rebuild again before draining the pending queue. If there is no interrupted rebuild, queued operations resume immediately.

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
