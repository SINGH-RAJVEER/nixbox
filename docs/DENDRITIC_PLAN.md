# Dendritic configuration support

The implementation supports the layout observed in `~/.config/nixos-dendritic`: a literal `inputs.import-tree ./modules` and flake-parts' classed modules. Discovery lists named NixOS outputs and directly attached Home Manager users. `--host` and `--home-user` select among them; a unique host/user is selected automatically. `nixbox config inspect --json` reports discovery, selected identity, file placement, and diagnostics without writing.

## Generated files

Conventional configurations retain plain NixOS and Home Manager modules. New dendritic files use the same two package filenames as flake-parts modules under the detected tree:

- `<tree>/nixbox/hosts/<hex-host>/nixbox-system-packages.nix` defines a host-specific NixOS classed module and imports it into the selected host module.
- `<tree>/nixbox/hosts/<hex-host>/users/<hex-user>/nixbox-home-packages.nix` defines a host/user-specific Home Manager classed module and imports it into that host's selected user.

The path components encode UTF-8 names as hexadecimal, and generated class names include the same identity to prevent collisions. The import-tree loads native files without changing the root import expression. Root-level dendritic files created by the first implementation retain their existing placement and explicit imports. No automatic relocation or conventional-module conversion occurs.

Each classed module holds the existing package markers, flake-package markers, external module imports, and configuration/profile mode wiring. The manifest model and marker parser stay the same. Inputs are captured by the outer flake-parts module, so nested module imports and package expressions need no global `specialArgs` or `extraSpecialArgs` changes.

Native files are loaded by the existing tree. For retained root-level files, NixBox uses an explicit flake-parts imports list beside the tree. Defining a classed module and connecting it to the selected host/user are separate steps, both expressed in the generated file.

Settings and Home Manager profile integration retain their existing separate filenames. When used in a dendritic configuration, they are also rendered as flake-parts modules and attached at the appropriate host/user boundary. This change does not consolidate those existing features into the two package files.

## Detection and safety

`nixbox-nix/src/dendritic.rs` parses the root and import-tree files with `rnix`. Discovery is read-only. It requires a literal flake-parts constructor, one literal tree, and a statically connected `config.flake.modules.nixos.<name>` host reference. Files and directories beginning with `_` are excluded. Symlinks, escaped trees, excessive depth, reserved module-name collisions, and unsupported root imports fail before automatic edits.

The selected output is used for rebuilds, VM builds, options queries, and profile installables. Several hosts may coexist when each has a distinct directly referenced host module. Shared host modules are refused because host-specific attachment cannot be proven. System writes work without selecting among several users; Home Manager writes require that selection. Home Manager profile updates for another account use configuration activation, avoiding the invoking account's local profile.

Sessions and saved recovery state bind to the canonical configuration root, host, and user. Changing that identity requires reloading the engine. Saved work for another identity cannot execute or be overwritten by a new session; select its original target or explicitly discard it. A queue belongs to one identity, with both NixOS and Home Manager scopes allowed. Mixed-host queues and standalone Home Manager outputs remain unsupported. Legacy saved work without a target identity must be finished or discarded before dendritic use.

Generated-flake declarations through `flake-file` are rejected. Input insertion currently uses the existing hand-maintained root-flake backend. Existing input names are reused. External NixOS/Home Manager module imports belong to their matching nested class; arbitrary `flakeModules.*` installation remains a separate feature.

A generated module and its root import are prepared and syntax-checked before writing. The writer checks for stale contents and restores the module when the root write fails. Engine operations restore their recorded configuration files and in-memory manifests on an error in dendritic mode. Rebuild failures retain the written configuration, matching existing activation semantics.

Existing conventional managed modules are not automatically converted. NixBox refuses to overwrite them as a different module class. Legacy `*-flakes.nix` migration must also be completed before dendritic integration. Conversion requires a future migration that updates all old imports and preserves custom declarations.

## Existing packages

Read-only scanning follows literal classed module references from the selected host and its Home Manager module. It records the relative feature filename, package attribute, and source line. Unconnected module exports do not count as installed packages. External flake packages are listed using their existing root input aliases.

External entries are read-only. Migration and removal from handwritten dendritic feature modules are disabled until source edits can preserve their scope and exact syntax. Managed packages and managed flake outputs remain editable. Scanning does not evaluate arbitrary imports or claim complete coverage of computed expressions.

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

Next work is standalone Home Manager selection, additional constructor/import shapes, complete source diagnostics, safe migration from handwritten feature modules, conversion or relocation of existing managed modules, a generated-flake input backend, and a target picker in the GUI/TUI. Existing interfaces use the saved selection; the CLI provides per-invocation selection. Extend the existing fixture and VM checks as those capabilities are added.

## References

- [Dendritic pattern](https://github.com/mightyiam/dendritic)
- [flake-parts module classes](https://flake.parts/options/flake-parts-modules.html)
- [import-tree](https://github.com/denful/import-tree)
- [flake-file](https://github.com/denful/flake-file)
