# Dendritic configuration support

The implementation supports the layout observed in `~/.config/nixos-dendritic`: a literal `inputs.import-tree ./modules` and flake-parts' classed modules. Discovery lists named NixOS outputs and directly attached Home Manager users. `--host` and `--home-user` select among them; a unique host/user is selected automatically. `nixbox config inspect --json` reports discovery, selected identity, file placement, and diagnostics without writing.

## Generated files

Conventional configurations retain plain NixOS and Home Manager modules. New dendritic files use the same two package filenames as flake-parts modules under the detected tree:

- `<tree>/nixbox/hosts/<hex-host>/nixbox-system-packages.nix` defines a host-specific NixOS classed module and imports it into the selected host module.
- `<tree>/nixbox/hosts/<hex-host>/users/<hex-user>/nixbox-home-packages.nix` defines a host/user-specific Home Manager classed module and imports it into that host's selected user.

The path components encode UTF-8 names as hexadecimal, and generated class names include the same identity to prevent collisions. The import-tree loads native files without changing the root import expression. Root-level dendritic files created by the first implementation retain their existing placement and explicit imports. Normal startup leaves this placement intact. The explicit migration command can convert plain managed modules and relocate root-level flake-parts files after a preview.

Each classed module holds the existing package markers, flake-package markers, external module imports, and configuration/profile mode wiring. The manifest model and marker parser stay the same. Inputs are captured by the outer flake-parts module, so nested module imports and package expressions need no global `specialArgs` or `extraSpecialArgs` changes.

Native files are loaded by the existing tree. For retained root-level files, NixBox uses an explicit flake-parts imports list beside the tree. Defining a classed module and connecting it to the selected host/user are separate steps, both expressed in the generated file.

Settings and Home Manager profile integration retain their existing separate filenames. When used in a dendritic configuration, they are also rendered as flake-parts modules and attached at the appropriate host/user boundary. This change does not consolidate those existing features into the two package files.

## Detection and safety

Settings has a **Detect dendritic configurations** switch, enabled by default, and the CLI exposes `nixbox config set dendritic-detection true` or `false`. Switching reloads file placement, manifests, and output choices without moving or converting configuration files. Off uses conventional entry files and hides dendritic output choices. Changing the mode requires an idle session with no queued, saved, or staged work. A disabled mode refuses writes and rebuilds inside a detected dendritic tree, preventing conventional imports or fallback activation from targeting the wrong layout. Re-enable detection to manage that tree. The mode is part of saved recovery identity.

`nixbox-nix/src/dendritic.rs` parses the root and import-tree files with `rnix`. Discovery is read-only. Inspection reports each source file, its exported classes, and diagnostics for computed imports. It resolves literal constructor aliases, inline root definitions, extra literal root imports, and constructors containing one host-specific classed reference alongside other modules. It requires one literal tree and a statically connected host or standalone Home Manager classed module. Chained `filter`, `filterNot`, `match`, and `matchNot` predicates are evaluated with Nix against relative paths; the proposed managed path must pass those same predicates before a write. Mapping and arbitrary computed trees are refused. Files and directories beginning with `_` are excluded. Symlinks, escaped trees, excessive depth, reserved module-name collisions, and unsupported root imports fail before automatic edits.

The selected output is used for rebuilds, VM builds, options queries, and profile installables. Several hosts may coexist when each has a distinct directly referenced host module. Shared host modules are refused because host-specific attachment cannot be proven. System writes work without selecting among several users; Home Manager writes require that selection. Home Manager profile updates for another account use configuration activation, avoiding the invoking account's local profile.

Sessions and saved recovery state bind to the canonical configuration root, host, and user. Changing that identity requires reloading the engine. Saved work for another identity cannot execute or be overwritten by a new session; select its original target or explicitly discard it. A queue belongs to one identity, with both NixOS and Home Manager scopes allowed. Mixed-host queues remain unsupported. Standalone `homeConfigurations` outputs using `inputs.home-manager.lib.homeManagerConfiguration` and a classed Home Manager module are selectable with `--home-configuration`. Their files live under `<tree>/nixbox/homes/<hex-output>/`, and activation uses that output rather than an attached NixOS user. Legacy saved work without a target identity must be finished or discarded before dendritic use.

For `flake-file` configurations, NixBox stores its input declarations in `<tree>/nixbox/inputs.nix` and runs the configured `write-flake` app from the configuration root. Generation failure restores that source, `flake.nix`, and `flake.lock`. Existing input names are reused, and inputs declared by the user remain user-owned. Hand-maintained flakes retain direct root-input editing. External NixOS/Home Manager module imports belong to their matching nested class; arbitrary `flakeModules.*` installation remains a separate feature.

A generated module and its root import are prepared and syntax-checked before writing. The writer checks for stale contents and restores the module when the root write fails. Engine operations restore their recorded configuration files and in-memory manifests on an error in dendritic mode. Rebuild failures retain the written configuration, matching existing activation semantics.

Existing conventional modules require explicit conversion:

```sh
nixbox --host workstation --home-user alice config migrate-dendritic --dry-run --json
nixbox --host workstation --home-user alice config migrate-dendritic --yes
nixbox --host workstation --home-user alice config migrate-dendritic --relocate --dry-run --json
nixbox --host workstation --home-user alice config migrate-dendritic --relocate --yes
```

The preview includes exact before/after contents. Apply holds the configuration writer lock and makes newly created modules visible to Git-backed flakes. Conversion creates native modules, merges recognised legacy `*-flakes.nix` outputs, removes literal old imports, and deletes the old owned files. Custom configuration-mode declarations go into an editable `nixbox-*-custom.nix` feature module. Relative paths are rebased to preserve their meaning. Exact generated profile modules are supported; custom profile boilerplate, custom legacy flake files, interpolated paths, shared imports, and destination collisions require manual extraction first. A changed file or symlink invalidates the preview. Apply validates every result before writing and rolls back files already changed if a write fails. No rebuild or profile activation runs during migration.

## Existing packages

Read-only scanning follows literal classed module references from the selected host and its Home Manager module. It records the relative feature filename, package attribute, and source line. Unconnected module exports do not count as installed packages. External flake packages are listed using their existing root input aliases.

Dedicated package lines in exclusively connected source modules can be migrated or removed. The planner checks other hosts and users before offering an edit; declarations shared with another configuration stay read-only. Inline or computed entries stay read-only. Flake-package migration preserves the existing root input alias and changes only the selected classed source. Engine rollback covers the actual source edits and leaves unrelated concurrent edits intact. Managed packages and managed flake outputs remain editable. Scanning does not evaluate arbitrary imports or claim complete coverage of computed expressions.

## Disposable VM workflow

```sh
just vm-dendritic-seed ~/.config/nixos-dendritic
just vm-dendritic-cli --target nixos install hello --no-rebuild --yes
just vm-dendritic-cli --target home-manager install cowsay --no-rebuild --yes
just vm-dendritic-build
just vm-dendritic
```

The source is copied once into `.vm/dendritic/nixos`, excluding VCS metadata and build state. The copy gets its own colocated JJ repository. Reseeding refuses to overwrite it. Preferences, caches, journals, recovery state, VM output, and disk are isolated under `.vm/dendritic`. Activations through this launcher build the guest and do not switch the host or modify a host package profile. Builds use one job and two cores.

The seed adds `modules/nixbox-vm.nix`, a VM-only flake-parts module for the observed `flake.modules.nixos.nixos` host module. On recent NixOS, initrd activation precedes QEMU's stage-2 store registration. The helper registers the shared closure before `nixboxProfile` activation so the system profile can be created on a fresh guest disk. It does not affect the normal host configuration.

The copied configuration retains its original guest credentials and VM settings. No test login or SSH key is installed by the launcher. For automated checks in this development session, a separate VM-only overlay in the disposable copy supplies an ephemeral SSH key and a localhost port forward.

The run launcher clears inherited QEMU device, network, kernel, and kernel-path overrides. It pins the writable shared directory and temporary exchange files under `.vm/dendritic`; the generated runner shares the host Nix store read-only. In the isolation check, a guest-only configuration file and a new guest profile did not appear on the host. Before/after fingerprints of 293 host paths matched, covering the active and booted system links, Nix profiles, source configurations, and NixBox settings/state. Evidence is in `.vm/dendritic/isolation-test`. The test used the existing VM build and shut the guest down afterward.

## Validation and remaining work

Focused engine tests cover both targets, configuration/profile mode round trips, external module imports, lexical inputs, repeated writes, rollback after a failed install, excluded files, import comments, class-aware scanning, independent files for named hosts and multiple users, recovery identity, and root-placement compatibility. The workspace CI gate covers conventional behavior as well.

On 2026-10-25, the disposable copy of the real configuration built and booted from a fresh disk. Guest checks reported a running system, active Home Manager, `hello` from the system profile, and `cowsay` from the Home Manager profile. A direct `Engine::apply` probe also imported the existing Home Manager input's NixOS module and evaluated successfully. The GitHub browser's authenticated output lookup was unavailable because this environment's `gh` was not authenticated; browser discovery was not verified by that engine probe.

Local evidence is under `.vm/dendritic`: `build-fresh-fix.log`, `boot-fixed.log`, `guest-fixed-check.log`, `probe.log`, and the CI logs. These ignored artifacts are local evidence, not published test fixtures. The repository fixtures and core tests cover deterministic integration behavior.

The native-placement follow-up passed the workspace gate (311 tests, six skipped, plus 14 telemetry and four version checks) and GUI gate (34 tests). A fresh copy under `.vm/dendritic/native-check` built and booted with both managed package modules inside the tree. Guest checks confirmed a running system, active Home Manager, system `hello`, user `cowsay`, and a read-only shared store. The guest shut down after testing. A second output named `test-server` accepted its own native package module; real Nix evaluation showed `cowsay` only in that host's system packages and `hello` only in the original host's system packages. The root flake remained byte-identical to the source. Before/after fingerprints of 293 host paths matched, and the guest-only sentinel was absent on the host. Logs and `result.json` are retained in that ignored directory.

The documented workstreams now have implementations: standalone Home Manager, broader static discovery, source diagnostics, exclusive-source migration, managed-module conversion and relocation, `flake-file` inputs, and target pickers in both interfaces. GUI Settings and TUI Settings > Configuration use the shared selector. Ambiguous layouts can open an interface for selection. Switching is refused while queued or saved work still belongs to the current identity. The CLI also supports per-invocation selection.

The safety limits remain explicit. A queue belongs to one root/host/user identity. Shared host modules, arbitrary computed constructors, import-tree mapping, symlinked trees, and source edits that cannot be assigned to one configuration are refused. These are not silently treated as conventional layouts.

On 2026-10-26, a disposable fixture built a standalone Home Manager activation package without activating it. Real Nix evaluation retained a custom host name after conventional-module conversion. The official `flake-file` generator completed input add/remove; a cached dependency override avoided GitHub rate limiting during that test. Controlled generator failure restored the flake, lock, and owned input module byte-for-byte. Filtered-tree checks covered a successful native write and rejection of a path excluded by the predicate. Evidence is retained under `.vm/dendritic/roadmap-real`.

## References

- [Dendritic pattern](https://github.com/mightyiam/dendritic)
- [flake-parts module classes](https://flake.parts/options/flake-parts-modules.html)
- [import-tree](https://github.com/denful/import-tree)
- [flake-file](https://github.com/denful/flake-file)

Final validation on 2026-10-26 passed the workspace CI gate with 319 tests, six skipped, 14 telemetry checks, and four version checks. The GUI gate passed 35 tests, including a rendered configuration-picker check. Relocation of the existing VM configuration preserved its evaluated system-profile derivation. The final host audit compared 289 paths with no differences. The earlier fresh-boot VM checks remain recorded above; the standalone activation package was built and never activated.
