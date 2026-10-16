# NixOS settings tab plan

Status: implementation started on 2026-10-10. Prepared on 2026-10-05 in the `nixos-settings` Jujutsu workspace at `/home/rajveer/Projects/personal/nixbox-nixos-settings`, based on `8f69afe2`.

## Current implementation

The first milestone adds a read-only NixOS tab with independent host selection, category and description search, diagnostic-option visibility, virtualized rows, and lazy value/default/example/provenance inspection. Package Options and the operation queue retain their existing behavior. Inspection state stays available across tab switches and is invalidated after rebuilds or a configuration-directory change. Reload refreshes the host list and schema. Inspection alone never writes settings; only queued edits, described below, do.

Both front-ends share the browser's policy through `nixbox-core::system_options`: the category taxonomy, search ranking with per-category counts, default host choice, and the wording of inspection results. The desktop page shows the NixOS logo with a header, a category sidebar, the option list, and details grouped into Value and Source. The terminal has a matching NixOS tab with the same columns, which loads on first visit and inspects only on `Enter`.

Discovery includes ordinary option declarations and nested schema templates. Expansion stops after four nested submodules and at branches that embed a complete NixOS system, such as container configurations. Those branches remain visible with a partial marker and declaration locations. Concrete named/list instances, complete recursive discovery, source watching, structured-value editing, shared writable host identity, candidate validation, and host-bound queue/recovery records remain to be implemented. Phases 1 and 2 are started, not complete.

The desktop page edits simple types. Selecting an option evaluates its listed siblings in one batched `group` request, which returns each value with its type, highest definition priority, and definition files. `nixbox-core::system_options` decides the group (`sibling_group`, at most `GROUP_LIMIT` paths around the selection) and why an option is locked (`edit_lock`), reusing package options' ownership rules. Editing is limited to the `nixos` host, which the NixOS rebuild applies. Staged edits are queued as one `Op::SetOptions` for the NixOS target, so they share the package options' settings module, queue, review, and recovery. The terminal tab remains read-only.

The live schema fixture runs with `NIXBOX_TEST_NIXPKGS` pointing to a local nixpkgs store source. Run `cargo test -p nixbox-nix live_schema_and_inspection -- --ignored` inside the development shell. It checks disabled services, custom declarations, nested templates, failing metadata/values, null, bounded previews, quoted output names, and credential fields that must not evaluate. The default CI gates also cover the browser's filtering and rendered list visibility.

## Outcome and coverage

Add a top-level **NixOS** tab to the desktop GUI that lets a user configure the selected NixOS host, browse its complete option schema, and stage settings before reviewing and rebuilding. Keep the existing **Settings** tab for NixBox preferences and the package **Options** page for package-specific navigation. The NixOS tab works independently of the package install target, so selecting Home Manager elsewhere does not redirect system edits.

"All settings" means every option declared by the selected configuration's locked NixOS modules, including imported third-party modules and local custom modules. Categories help people find settings, but the schema determines coverage. An option remains discoverable when its value cannot be evaluated or when NixBox cannot provide a form editor. Options from a module that the host has not imported become available after importing that module and refreshing.

Complete discovery and complete form editing are separate milestones. The first usable release provides the full schema browser and editors for simple types. Subsequent phases add named instances, structured values, package references, source migration, and an advanced Nix expression editor. Read-only options remain read-only. Internal and hidden declarations are available in an explicit diagnostic view, with aliases labeled and redirected where possible.

The initial product configures an existing local flake-based NixOS installation. A later phase adds a non-flake configuration adapter. Disk installation, formatting disks, and deployment to remote machines are separate features. Boot, filesystem, and storage configuration options are still included in this tab.

## What the repository already provides

| Existing module | Reuse | Gap to close |
| --- | --- | --- |
| `nixbox-nix/src/options.rs` and `options.nix` | Evaluated option types, descriptions, values, definition locations, timeout, and bounded value rendering. | Queries are package-based. The expression excludes named templates and nested submodule options. |
| `nixbox-core/src/options.rs` | Shared parsing, editability rules, formatting, changes, and settings access. | `load_options` fixes the host to `nixos`. Ownership rules need explicit unknown and mixed-definition states. |
| `nixbox-nix/src/settings.rs` | Typed Nix serialization and generated module load/write. | Only null, booleans, numbers, strings, and string lists round-trip. Paths, structured values, references, and expressions need representation. |
| `Op::SetOptions` and `Session` | Persistent queue, coalesced edits, rebuild progress, cancellation, and resume. | Batching currently keys only on Home Manager versus NixOS and starts work immediately when idle. Validation and selected-host identity must reach execution and recovery. |
| `nixbox-core/src/rebuild.rs` and `nixbox-nix/src/build.rs` | Shared rebuild commands and GUI askpass escalation. | The NixOS invocation fixes `#nixos` and `switch`; the tab needs explicit host and action. |
| `nixbox/src/options.rs` and `ui/options.rs` | Staged changes, selection, filtering, type editors, and reload behavior. | Package-bound view state cannot serve as the system-wide browser state. |
| `nixbox/src/ui/mod.rs` | Horizontal GPUI Kit navigation, tab label modes, queue count, and build status. | Add a distinct tab without changing package Options navigation. |

Keep the work in the existing crates. Extend the shared options module so callers can discover a target, list its schema, inspect an option, and prepare a change batch through a small interface. GUI code should not decide Nix definition priorities or construct rebuild commands.

## Tab layout and navigation

Use GPUI Kit's native controls, which already implement the application's shadcn-style design language. Use flat lists, whitespace, and separators. Avoid nested cards.

```text
NixBox   nixpkgs | Flakes | Installed | NixOS | Queue | Build | Version control | Settings

NixOS     Host: workstation     Configuration: ~/.config/nixos     Reload
Search settings...             All | Editable | Changed | Managed | Advanced

Categories           Settings                           Selected option
General              Automatic time zone        Off     services.automatic-timezoned.enable
Network              Time zone                  ...     Type, description, default, example
Desktop              Bluetooth                  On      Effective / managed / staged values
Hardware             ...                                Definition files and editability
Users                                                   Editor / source action
Boot and storage
Services
Nix
All options

3 staged changes                                  Discard all | Review changes
```

Preserve selection, filter, category, and staged values when switching tabs. Ctrl-F focuses this tab's search field. Boolean rows may have inline switches; selecting any row opens the full details and explanation of its status. A switch stages a value and visibly marks the row. It never activates a system change on click.

Search both option paths and descriptions, with exact path matches ranked first. Search the entire schema by default and make any category restriction explicit. "All options" uses the same index and includes uncategorized custom namespaces. Filters distinguish configured options, NixBox-managed values, drafts, queued work, unsupported editors, and read-only options. Diagnostic options are hidden by default, with a separate toggle to include them.

At smaller sizes, collapse the category list into a selector and open option details as a full-width subpage. Keep the existing top navigation and label modes working. Virtualize option rows and avoid evaluating current values during rendering.

## Categories

This is the starting navigation taxonomy. Prefix matching and a small set of curated shortcuts organize the schema; they do not enumerate its contents. Where an option fits several categories, shortcuts point to one canonical option identity. Names and paths are examples to resolve against the selected host's schema, not paths to assume exist on every release.

| Category | Settings to include | Example namespaces |
| --- | --- | --- |
| General, region, and time | Host name, time zone, locale, console language, keyboard layout, fonts, documentation, accessibility-related system settings. | `networking.hostName`, `time`, `i18n`, `console`, `fonts`, `documentation` |
| Network and connectivity | NetworkManager, networkd, interfaces, IP addressing, DNS, Wi-Fi, proxies, VPNs, firewall ports, host mappings, SSH. | `networking`, `systemd.network`, `services.resolved`, `services.openssh` |
| Desktop and login | Available desktop environments, display managers, Wayland/X11, window managers, session packages, portals, input, printing, scanning. | `services.desktopManager`, `services.displayManager`, `services.xserver`, `programs`, `xdg.portal`, `services.printing`, `hardware.sane` |
| Audio and peripherals | PipeWire, PulseAudio where declared, Bluetooth, input devices, removable media, firmware, and device rules. | `services.pipewire`, `services.pulseaudio`, `hardware.bluetooth`, `services.blueman`, `services.udev`, `hardware` |
| Graphics and power | Graphics support, GPU drivers, acceleration, power profiles, suspend, hibernate, battery and thermal settings. | `hardware.graphics`, `hardware.nvidia`, `services.xserver.videoDrivers`, `powerManagement`, `services.power-profiles-daemon`, `services.tlp` |
| Users and security | Users, groups, login shells, sudo, polkit, PAM, authorized SSH keys, credential-file references, audit, and security modules. | `users`, `security`, `services.openssh` |
| Boot and storage | Boot loaders, kernel settings, initrd, filesystems, swap, encrypted devices, RAID, trim, mount definitions, and filesystem services. | `boot`, `fileSystems`, `swapDevices`, `systemd.mounts`, `services.fstrim` |
| Programs and environment | System packages, program modules, shells, environment variables, session variables, and declarative files. | `environment`, `programs` |
| Services and scheduling | Every service module, systemd units and timers, logs, backups, databases, web servers, discovery, and monitoring. | `services`, `systemd`, `system.activationScripts` |
| Virtualization and containers | Podman, Docker, libvirt, virtual machines, NixOS containers, and guest integration. | `virtualisation`, `containers` |
| Nix and maintenance | Daemon settings, experimental features, substituters, trusted keys, garbage collection, optimization, nixpkgs settings, system upgrades. | `nix`, `nixpkgs`, `system.autoUpgrade` |
| System and advanced | State version, specialisations, assertions, warnings, custom modules, and every declaration not covered above. | `system`, `specialisation`, `assertions`, `warnings`, custom prefixes |

Home Manager options have their own source and user identity. Keep the existing package editor working for them. A future user-settings view can reuse the browser; the NixOS tab must not treat `home-manager.users` options as interchangeable with system options.

## Option discovery and metadata

Split schema discovery from inspecting current values. Index option paths, type structure, visibility, descriptions, declaration files, defaults, examples, and nested templates. Fetch current values and definition provenance on demand for selected options or a bounded batch of visible rows. Do not deeply evaluate the entire `config` tree to populate a list.

Use the target's own library and evaluated `options`, so the locked release and its custom declarations remain authoritative. Nixpkgs' documentation traversal already describes sub-options through `getSubOptions`; adapt that approach instead of extending the current plain attribute lookup to template paths. Preserve named and list templates as schema entries. Resolve concrete instances through type-aware traversal and evaluated value metadata where available. These mechanisms need capability checks against the locked nixpkgs version. [Nixpkgs option traversal](https://github.com/NixOS/nixpkgs/blob/master/lib/options.nix), [Nixpkgs type implementation](https://github.com/NixOS/nixpkgs/blob/master/lib/types.nix).

For example, `users.users.<name>.shell` is a template, while `users.users.alice.shell` is a concrete setting. Show both the template documentation and configured instances. "Add user" asks for a name and opens that instance's declared fields. A list of submodules needs index-aware editing and writes a coherent list value; it must not generate a dotted numeric Nix attribute path.

Each field independently reports available, absent, failed, or truncated. An unavailable current value is distinct from `null`, an empty value, or a default. Keep description/default failures local to an entry. A failure to evaluate the host itself produces a page-level diagnostic and any previous schema is marked stale and read-only. A cached schema alone cannot recover every broken module evaluation; provide links to the affected source and build log.

Use structured query arguments and attribute path segments. Do not interpolate host names, user names, option paths, or editor text into shell commands or Nix source. Match ownership against the exact managed file and resolved configuration files; matching only a file basename is insufficient.

Start with the existing 120-second evaluation timeout, cancellation on dropped tasks, and epoch-based stale-result rejection. Cache by configuration root, selected output, source, lock identity, and configuration revision. Invalidate after managed writes, rebuild completion including failures, lock changes, source changes, target changes, and explicit Reload. Mark draft reviews stale when the configuration changes. Persistent metadata caching is optional; persist neither complete current values nor credential contents.

Track discovery counts and failures internally so "all options" can be verified against the selected target's schema. Unsupported traversal must appear as an unresolved schema branch, never disappear silently.

## Value editors

| Type | Editor and serialization | Delivery |
| --- | --- | --- |
| Boolean | Switch with inherited/effective value visible. Unset is a separate action. | First usable release. |
| String enum | Searchable select for declared choices. Preserve string values even when they look like booleans or numbers. | First usable release. |
| Integer and float | Numeric field with available bounds and exact type checking through evaluation. Reject non-finite values. | First usable release. |
| String and constrained string | Text input with type hints, preserving whitespace when the type permits it. | First usable release. |
| Nullable type | Explicit "Use null" choice, independent of empty text and Unset. | First usable release for supported inner types. |
| String list | Add/remove/reorder rows, preserving empty strings and commas inside an item. Reuse existing serializer. | First usable release. |
| Multiline string and `lines` | Multiline text editor that distinguishes literal text from a Nix expression. | Structured-editor phase. |
| Path | Explicit path reference with source-relative resolution. Keep paths distinct from strings. | Structured-editor phase. |
| List and attribute set | Recursive editors using the declared element type, with key quoting and ordering rules. | Structured-editor phase. |
| Named submodule | Create/remove named instances and render their declared child fields. | Structured-editor phase. |
| List of submodules | Edit child fields and list order through the parent value. Preserve source-owned entries and require reviewed migration before replacing them. | Structured-editor phase. |
| Package | Picker based on the target's evaluated package universe. Store an expression/reference, not an evaluated store path. | Reference-editor phase. |
| Either/coerced/custom type | Choose a supported form when unambiguous; otherwise use an explicit expression or source editor. | Reference-editor phase. |
| Function or arbitrary Nix expression | Explicit advanced editor, parsed as Nix and evaluated in the selected module context. Never infer source from an evaluated result. | Advanced-editor phase. |
| Read-only or internal value | Inspect metadata and source, with an explanation. No editable switch. | All phases. |

Structured editors need a recursive type description with bounded expansion and explicit recursion markers. Do not derive merge behavior solely from the rendered type name. Nix's type check and a candidate configuration evaluation are the final authority for allowed values.

Extend `SettingValue` with typed data and reference forms as needed, preserving old serialized variants. Add a separately tagged expression representation only in the advanced phase. Generated files must load every form they write. Multiline or nested syntax requires a real Nix parser or an equally reliable structured format; the existing line parser must not silently skip richer values. Evaluate `rnix` as the parser dependency at that phase.

Literal string input must escape interpolation. Expression input deliberately preserves it and has a distinct mode and review. Nix treats `${...}` as interpolation and requires escaping it when it should remain literal. [Nix string literals](https://nix.dev/manual/nix/2.35/language/string-literals.html).

## Existing definitions and ownership

Show the effective value, NixBox's own contribution, a staged contribution, any queued contribution, and every known definition location as separate facts. A list's merged effective value must not become the next NixBox-owned list by default; doing that would duplicate contributions from other modules. "Remove NixBox value" removes only its contribution and may expose a different inherited value. It does not promise to restore the option's declared default.

Preserve the current conservative editing rule in the first release. User-defined settings stay locked with an Open source action. Do not automatically insert `mkForce` to make a toggle appear to work. Definitions have override priorities, and merge behavior depends on the option type. [NixOS module priorities](https://nixos.org/manual/nixos/stable/#sec-option-definitions-setting-priorities).

The source-migration phase adds "Manage with NixBox" for a supported literal assignment whose location is unambiguous. Parse its source, preview removal from that file plus addition to the managed module, and evaluate the whole candidate configuration. Preserve comments and unrelated content. Conditional definitions, shared modules, imported files outside the editable root, `mkMerge`, and expression-valued assignments use the source editor until a reliable transformation exists. Never remove a definition merely because its evaluated value matches the requested value.

Ownership metadata that could not be read produces "Definition source unavailable" and blocks managed editing. Mixed NixBox and upstream definitions require inspection of all relevant definitions and priorities; the existence of a NixBox definition does not prove the setting is safe to overwrite. A staged value that an upstream override prevents from taking effect must fail review or clearly report its contribution-only effect.

Credential fields use file references where their schema allows them. Known password, private-key, token, and secret fields are excluded from bulk value fetching, default previews, logs, and persisted drafts. Do not rely on option naming to prove all other fields are non-sensitive. The advanced editor warns before persisting literal credential material and exposes the actual file changes through an explicit review.

## Host identity and managed files

Add a shared configuration identity containing the canonical root, target kind, NixOS output key, and Home Manager output/user identity when relevant. Host selection reads `nixosConfigurations` keys; it does not use `networking.hostName` as an output key. The initial selected host remains `nixos` when that output exists, to preserve current behavior. A single different output may be selected automatically; multiple outputs require choosing one before editing.

Use this identity consistently for reads, caches, drafts, generated-file ownership, queued work, rebuild commands, and recovery. Add configuration selection to app preferences through backward-compatible defaults. Selection of a different root is blocked while existing work uses the old root, matching today's behavior. Inspecting another host may remain available, but every edit batch stays bound to its original host.

First support one writable selected host per session. Reuse `nixbox-system-settings.nix` only when its import is verified to affect that host alone, or when the user explicitly chooses a shared configuration. If wiring is shared or unclear, browsing works and writes remain disabled until a host-specific import is established. Never infer an import destination from a filename alone or fall back to an unrelated `/etc/nixos/configuration.nix`.

In the multiple-host phase, use distinct managed settings modules and manifest state per output, with collision-safe filenames and explicit import mappings. Preview the import changes and verify which outputs consume each file. Do not silently split or duplicate an existing shared module. Provide a reviewed migration path for current single-host settings files.

Extend queue and recovery records to carry the bound identity and rebuild action. Preserve decoding of old `SetOptions` and `InProgress` records. Treat missing host identity as the legacy `nixos` target, and require an explicit resume review before executing legacy work if its root cannot be verified. Keep raw unresolved recovery data available instead of dropping it after a schema upgrade.

## Review, validation, and rebuild behavior

The tab's workflow is edit, stage, review, then apply. Draft staging does not enqueue an operation, because `Session::enqueue` currently starts an idle queue immediately.

1. Read a source snapshot and its configuration identity. Build a draft of sets, unsets, and any reviewed source migrations. Retain drafts across tabs; changing targets keeps them attached to their original identity.
2. Review the complete changed-file diff, including the generated module and any import changes. Explain each old/new contribution and distinguish it from the effective value.
3. Validate against an isolated snapshot containing the exact candidate files. Replace the existing managed module in that snapshot so Unset actually removes a contribution. Appending an override module cannot simulate all removals correctly. Preserve local flake inputs, relative paths, and the files Nix will include without changing the live repository index.
4. Parse and type-check candidate values, evaluate affected options, and evaluate the system build derivation with assertions and warnings. Keep validation asynchronous and cancellable. A passing evaluation does not prove the system builds or boots.
5. Queue the reviewed batch with its source fingerprint and action. Immediately before writing, validate the final coalesced batch against the current sources. If another edit, imported module, or earlier queued operation changes the inputs, stop and require a new review rather than applying a stale candidate.
6. Write only reviewed files, atomically per file, retain a recovery journal for a partial multi-file failure, and run one rebuild for that configuration and action. Repository commands and config mutation use the existing mutual exclusion rules. Bind queued work to its target even when the visible tab changes.
7. Report file-write, evaluation, build, activation, and reboot-pending states separately. Refresh the schema and ownership state after completion, cancellation, or failure. Clear only drafts that were accepted into the queue; failed validation preserves them.

The GUI offers these rebuild actions once the shared runner supports them. NixOS distinguishes building from activation and boot selection. [Official nixos-rebuild documentation](https://wiki.nixos.org/wiki/Nixos-rebuild).

| Action | Meaning |
| --- | --- |
| Build only | Writes reviewed configuration and builds it without activation. |
| Test now | Activates the candidate without making it the boot default. |
| Apply now | Runs `switch`, activates the candidate, and makes it the boot default. |
| Apply on next boot | Runs `boot`, updating the boot default without live activation. |

Offer Apply now as the familiar choice, while boot-related edits can suggest Apply on next boot. Do not claim every option provides exact restart or reboot metadata. Show known guidance for curated cases and the actual build/activation output.

Coalesce settings only when root, output, scope, and rebuild action match. If edits from package Options and the NixOS tab combine, the final batch must pass the same validation. Operations for different identities or actions cannot share a rebuild. Validate the full batch of interacting configuration changes, not just the final scalar edited in the page. A failure pauses that batch and exposes the reason instead of losing queued work.

After a failed or cancelled rebuild, show that the files were written and the new system is not confirmed active. Offer retry, reviewed removal/reversion of NixBox contributions, and an explicit previous-generation action. A system-generation rollback does not revert source files; source recovery is separate. Cancellation is not proof that an activation script made no changes. Restore journaled file content only when the live file still matches NixBox's written version.

## Implementation phases

| Phase | Work | Completion gate |
| --- | --- | --- |
| 1. Shared host selection and discovery | Remove the fixed host from option reads and rebuild construction, add the configuration identity, discover complete schemas including templates, and inspect values lazily. Establish writable import mapping for one host. | A fixture with an output named `workstation` and a custom module discovers ordinary, nested, hidden, and unavailable options. Reads and rebuilds use the same output. Unsupported schema branches remain visible. |
| 2. First usable NixOS tab | Add the top tab, categories, global search, details, virtualized rows, simple editors, drafts, complete review, candidate validation, host-bound queue records, and switch/build actions. | A user finds and changes an editable system option without installing a related package. The candidate validates, the reviewed file is written, and exactly one rebuild targets the selected host. Unsupported editors and source-owned settings explain the next action. |
| 3. Source ownership and migration | Add safe literal migration with parser-based edits, full definition provenance, mixed-definition handling, and contribution-aware merged-value editors. | A hand-written literal moves into NixBox management without changing its effective value. Ambiguous or shared source fails safely with an actionable source link. |
| 4. Structured editors | Add paths, multiline strings, generic lists/attributes, named submodules, and list submodules. Upgrade generated-module parsing and round trips. | Users, groups, firewall port lists, filesystem entries, mount lists, and nested service settings can be edited with the same draft and validation workflow. Literal text and dynamic keys round-trip. |
| 5. References and advanced expressions | Add target-pinned package selection, reference serialization, custom/either-type handling, and explicit parsed expression editing. | Package references rebuild against the selected target. Expression values retain their source rather than a store path or JSON approximation. Every writable discovered option has a form or source/expression route with a stated limitation. |
| 6. Complete local configuration support | Add reviewed multiple-host wiring, per-host manifests, build/test/switch/boot actions, recovery and generation controls, then a non-flake adapter. | Two hosts retain separate values and queued work through restart. Non-flake reads and rebuilds use the same explicit configuration entry. Recovery distinguishes files, active generation, and next-boot generation. |

Phases 1 and 2 are the first usable release. They do not satisfy the complete editing goal alone. Phases 3 through 6 remain part of this feature's planned scope. Guided setup shortcuts can follow, but they must emit ordinary option changes into the same review workflow. Changing `system.stateVersion` is an explicit advanced edit, not an automatic release-upgrade step.

## Focused verification

Planning-only changes require Markdown/link inspection and a JJ diff check. Implementation verification should extend the existing option, generated-file, Session, rebuild-command, and GPUI tests rather than create a parallel test harness.

- Add schema fixtures for a host with a non-default output key, disabled-service declarations, imported custom options, hidden options, named submodules, lists of submodules, and values/defaults that fail independently. Compare discovered paths and templates with that fixture's full schema.
- Verify existing package namespace queries and Home Manager queries still return the same results and preserve their ownership rules.
- Verify generated files round-trip typed values, quoted dynamic keys, multiline text, literal interpolation, empty strings, null, and Unset. Test contribution editing for merged lists and mixed module priorities.
- Verify migration changes only the reviewed assignment, preserves comments, and refuses ambiguous or externally shared definitions.
- Verify candidate validation catches assertion failures and conflicting definitions before live writes; source changes invalidate reviews; coalesced work is revalidated; different hosts/actions stay separate; legacy state decodes and new bound work resumes correctly.
- Render GUI frames for the new top tab, search appearance/replacement/clearing, supported controls, lock explanations, staged badges, persistent drafts, and small-window navigation. Assert visible bounds so a collapsed list cannot pass as a working browser.
- Use a disposable NixOS VM for build, test, switch, boot, cancellation, and recovery behavior. The developer's own host is not the integration fixture.
- Run `just ci` and `devenv shell -- just gui-ci` for implementation changes. At the packaging milestone, build the actual affected flake packages. A release's visual sign-off includes a native GUI session; a model assertion or headless layout test alone does not prove the tab looks right.

## Documentation and integration

Keep the current implementation status separate from the proposed phases. During implementation, update `GUI.md`, `PACKAGE_OPTIONS.md`, `CONFIGURATION.md`, `MANAGED_FILES.md`, `ARCHITECTURE.md`, `USER_GUIDE.md`, `TROUBLESHOOTING.md`, and release notes where each milestone changes behavior. Describe shipped coverage separately from later editors and adapters.

Develop in the `nixos-settings` JJ workspace. Preserve the default workspace and the existing `package-options` workspace. Merge, commit, push, release, and host activation remain separate steps.
