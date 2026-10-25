# Configuration and stored state

## Settings file

NixBox stores user settings at `$XDG_CONFIG_HOME/nixbox/settings.json`. On a conventional Linux setup this resolves to `~/.config/nixbox/settings.json`. A missing file uses defaults. An unreadable or invalid file stops startup with an error rather than silently replacing it.

The complete shape is:

```json
{
	"channel": "nixpkgs-26.05",
	"target": "nixos-system",
	"theme": "default",
	"input_mode": "vim",
	"tab_labels": "icons-and-names",
	"close_button": true,
	"telemetry": {
		"enabled": false,
		"consent_version": 0,
		"revision": 0
	},
	"config_dir": null,
	"recent_searches": [],
	"home_manager_main_file": null,
	"nixos_main_file": null,
	"nixos_host": null,
	"home_user": null,
	"home_configuration": null
}
```

| Field | Default | Meaning |
| --- | --- | --- |
| `channel` | `nixpkgs-26.05` | Flake reference used when the revision-pinned catalog is unavailable and NixBox falls back to live `nix search`. The settings UI also offers `nixpkgs-unstable`. |
| `target` | `nixos-system` | Scope for new package and flake installs. Valid values are `home-manager` and `nixos-system`. |
| `theme` | `default` | Active color palette. Valid built-in names are `default`, `dracula`, `gruvbox`, `nord`, `catppuccin`, and `monokai`. An unknown value displays the default theme until another theme is selected. |
| `input_mode` | `vim` | Text editing behavior. Valid values are `vim` and `normal`. |
| `tab_labels` | `icons-and-names` | What the desktop GUI's navigation tabs show. Valid values are `icons`, `icons-and-names`, and `names`. `nixbox config set tab-labels` accepts the same values. |
| `close_button` | `true` | Whether the desktop GUI shows a GTK-style close button at the top right. `nixbox config set close-button` accepts `true` or `false`. |
| `telemetry.enabled` | `false` | Saved telemetry consent. Missing fields on upgrades stay disabled. The shared recipient is `https://nixb.duckdns.org:58443/v1/telemetry`. |
| `telemetry.consent_version` | `0` | Version of the disclosed collection contract. A saved enablement is valid only for the current version, `3`, which discloses the selected recipient. |
| `telemetry.revision` | `0` | Local counter advanced by explicit consent changes, so queued work can detect a disable/enable cycle. It is never part of a report. |
| `config_dir` | `null` | Optional configuration root. `null` uses `~/.config/nixos`; `NIXBOX_CONFIG_DIR` overrides it. |
| `recent_searches` | `[]` | The schema can retain up to 20 non-empty package queries, newest first, with duplicates moved to the front. The current TUI neither adds queries to this field nor displays a recent-search screen. |
| `home_manager_main_file` | `null` | Optional absolute or relative path to the Home Manager file that NixBox scans and imports its generated Home Manager modules into. |
| `nixos_main_file` | `null` | Optional absolute or relative path to the NixOS file that NixBox scans and imports its generated NixOS modules into. |

NixBox saves settings immediately after selecting an input mode, theme, tab labels, close button, configuration location, target, or channel. Writers use a persistent `settings.lock` sibling to serialize updates, write formatted JSON to a private temporary file, synchronize it, and atomically replace the settings file. A linked settings file keeps its symlink; writers update and lock the resolved target. Invalid existing settings are never overwritten by a preference save.

Ordinary saves preserve the latest telemetry consent from disk, even if an older open instance still holds an enabled snapshot. The shared `Config::set_telemetry_enabled` function is the only application path that persists explicit consent changes, and it updates consent without overwriting other saved preferences. Failed opt-out saves leave that instance disabled but do not confirm an opt-out for other instances. Removing the settings file also makes a later ordinary save use disabled telemetry defaults.

The optional `nixos_host`, `home_user`, and `home_configuration` fields default to automatic selection. Standalone `home_configuration` is mutually exclusive with a NixOS host. These saved names are separate from the install scope. CLI global flags override them for one invocation. Resolved tree paths and legacy-placement flags are runtime discovery data and are never saved.

The consent foundation honors `NIXBOX_NO_TELEMETRY=1` and `DO_NOT_TRACK=1` as disabling overrides. Environment variables cannot enable collection. Debug and test builds, a present `CI` variable, and a present `NIXBOX_VM_BUILD` variable suppress delivery. These overrides leave saved consent unchanged.

GUI Settings, TUI Settings > Usage data, and `nixbox config set telemetry-enabled true|false` share the consent function. `nixbox telemetry status` shows saved consent, effective status, disclosure, and recipient. `nixbox telemetry preview` prints a synthetic example without collecting or sending anything. All interfaces use the fixed HTTPS recipient `https://nixb.duckdns.org:58443/v1/telemetry`; it has no runtime environment override. Collection requires explicit saved consent and remains suppressed in debug, CI, and VM builds. Version 1 and 2 consent need renewal because it did not disclose this recipient. The collector and optional Axum receiver are implemented and verified against controlled local receivers. See the [telemetry implementation plan](TELEMETRY_PLAN.md) for the allowed fields, Docker/Nginx deployment, and remaining public-service checks.

## Configuration root

NixBox detects the supported import-tree/flake-parts layout under this root. Named NixOS outputs and their directly attached Home Manager users are discovered from classed modules. `nixos_host` and `home_user` select a target; a unique host/user is selected automatically. Multiple hosts require an explicit host, and Home Manager writes require an explicit user when several are attached. New generated flake-parts modules live under the import-tree with a separate directory for each host/user. Existing root-level dendritic managed files retain their placement until explicit migration. `home_configuration` selects a standalone Home Manager output and clears the host selection. GUI/TUI Settings > Configuration lists discovered targets. Conventional main-file settings continue to describe conventional configurations. See [Dendritic support](DENDRITIC_PLAN.md).

The configuration root is, in order: `NIXBOX_CONFIG_DIR` when it is set, then the saved `config_dir` setting, then `$XDG_CONFIG_HOME/nixos`, normally `~/.config/nixos`. Change the saved location in GUI Settings or with `nixbox config set config-dir <path>`; an empty value returns to the default.

The configuration root determines these paths:

| Path | Purpose |
| --- | --- |
| `<root>/flake.nix` | Root flake modified by flake-module installation. |
| `<root>/flake.lock` | Source of the exact nixpkgs revision used to build and validate the package catalog. |
| `<root>/home.nix` | Default Home Manager entry file. |
| `<root>/configuration.nix` | Preferred NixOS entry file when it exists. |
| `<root>/nixbox-home-packages.nix` | Generated Home Manager nixpkgs packages, flake packages and module imports. |
| `<root>/nixbox-system-packages.nix` | Generated NixOS nixpkgs packages, flake packages and module imports. |
| `<root>/nixbox-home-settings.nix` | Home Manager package options set from the options panel. |
| `<root>/nixbox-system-settings.nix` | NixOS package options set from the options panel. |

If `<root>/configuration.nix` does not exist and `nixos_main_file` is unset, NixBox uses `/etc/nixos/configuration.nix`. The generated files still stay under the configuration root.

Older `nixbox-home-flakes.nix` and `nixbox-system-flakes.nix` files are [merged automatically on startup](MANAGED_FILES.md#upgrading-existing-configurations), then deleted after their imports have been updated.

`NIXBOX_CONFIG_DIR` does not move `settings.json`, `state.json`, or the package catalog cache.

For host-side VM development, `NIXBOX_VM_BUILD=<absolute-output-link>` replaces rebuilds with `nix build <root>#nixosConfigurations.nixos.config.system.build.vm --out-link <absolute-output-link>`. It applies to both targets and recovery, builds without sudo, and leaves activation to a later VM boot. A target in package-profile mode gets the same VM build: package changes do not touch the host's package profile, and `nixbox profile rollback` refuses to run. The `just vm-gui`, `just vm-tui`, and `just vm-cli` recipes set it together with isolated XDG directories. See [Development](DEVELOPMENT.md#testing-in-a-vm) for launch and boot commands.

## Version control journal

Repository controls use the configuration root above, not the directory containing NixBox's own settings. Successful operations that change files are recorded in `$XDG_CONFIG_HOME/nixbox/vcs-journal/<root-hash>.json`. The hash is stable for a canonical root path, and each journal also stores that full path to verify its scope. The journal stores operation descriptions and changed paths, not option values.

Commit-message suggestions enumerate pending operations in application order, preserving repetitions and reversals. New entries include a redacted subject summary and target; option and migration batches record each action separately. The journal also stores a commit boundary. A successful app commit clears it; pushing and JJ snapshots do not. An external commit reconciles it only after forward history and a clean configuration establish that no scoped changes remain; uncertain history stays available for review. Keep main-file overrides inside the configuration root. A pending operation that touched an external file prevents committing rather than silently omitting that change. See [Version control](VERSION_CONTROL.md) for repository scope and safety rules.

## Package catalog cache

The catalog lives at `$XDG_CACHE_HOME/nixbox/package-catalog.json`, normally `~/.cache/nixbox/package-catalog.json`. It contains a format version, the locked flake reference and revision, and all parsed package hits. NixBox accepts the cache only when its format and source match the current lock file.

NixBox writes a new catalog to `package-catalog.json.tmp` and renames it over the cache after serialization succeeds. Removing the cache is safe; the next launch rebuilds it from the locked nixpkgs revision.

## Recovery state

NixBox stores unfinished work at `$XDG_CONFIG_HOME/nixbox/state.json`, normally `~/.config/nixbox/state.json`. The file may contain queued operations, one in-progress rebuild descriptor, and the last rebuild error.

When state becomes empty, NixBox removes the file. Load and save failures are intentionally non-fatal. A malformed state file is ignored, so it may need manual inspection if expected recovery does not happen.

Every front-end reads and writes this one file. Starting the TUI restores whatever it holds, and `nixbox resume` does the same from the command line: it re-runs an interrupted rebuild and applies a queued batch one target at a time. `nixbox resume --dry-run` prints the contents without acting on them, and `nixbox resume --discard` clears the file.

The format is defined by `nixbox-core`, not by the UI crate, so `nixbox-cli` reads the same file.

## Generated Nix files

Generated files begin with `# Managed by nixbox. Do not edit by hand.` NixBox rewrites each generated file from its parsed in-memory manifest rather than preserving arbitrary manual edits. Keep custom Nix expressions in your own modules and import the generated files.

See [Managed files and package operations](MANAGED_FILES.md) for the exact generated forms and mutation rules.
