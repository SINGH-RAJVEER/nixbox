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
	"recent_searches": [],
	"home_manager_main_file": null,
	"nixos_main_file": null
}
```

| Field | Default | Meaning |
| --- | --- | --- |
| `channel` | `nixpkgs-26.05` | Flake reference used when the revision-pinned catalog is unavailable and NixBox falls back to live `nix search`. The settings UI also offers `nixpkgs-unstable`. |
| `target` | `nixos-system` | Scope for new package and flake installs. Valid values are `home-manager` and `nixos-system`. |
| `theme` | `default` | Active color palette. Valid built-in names are `default`, `dracula`, `gruvbox`, `nord`, `catppuccin`, and `monokai`. An unknown value displays the default theme until another theme is selected. |
| `input_mode` | `vim` | Text editing behavior. Valid values are `vim` and `normal`. |
| `recent_searches` | `[]` | The schema can retain up to 20 non-empty package queries, newest first, with duplicates moved to the front. The current TUI neither adds queries to this field nor displays a recent-search screen. |
| `home_manager_main_file` | `null` | Optional absolute or relative path to the Home Manager file that NixBox scans and imports its generated Home Manager modules into. |
| `nixos_main_file` | `null` | Optional absolute or relative path to the NixOS file that NixBox scans and imports its generated NixOS modules into. |

NixBox saves settings immediately after selecting an input mode, theme, target, or channel. It writes formatted JSON directly to the settings path.

## Configuration root

`NIXBOX_CONFIG_DIR` changes the directory used for the target configuration. If the variable is unset, NixBox uses `$XDG_CONFIG_HOME/nixos`, normally `~/.config/nixos`.

The configuration root determines these paths:

| Path | Purpose |
| --- | --- |
| `<root>/flake.nix` | Root flake modified by flake-module installation. |
| `<root>/flake.lock` | Source of the exact nixpkgs revision used to build and validate the package catalog. |
| `<root>/home.nix` | Default Home Manager entry file. |
| `<root>/configuration.nix` | Preferred NixOS entry file when it exists. |
| `<root>/nixbox-home-packages.nix` | Generated Home Manager package module. |
| `<root>/nixbox-system-packages.nix` | Generated NixOS package module. |
| `<root>/nixbox-home-flakes.nix` | Generated Home Manager imports and the reserved external-flake package block. |
| `<root>/nixbox-system-flakes.nix` | Generated NixOS imports and the reserved external-flake package block. |

If `<root>/configuration.nix` does not exist and `nixos_main_file` is unset, NixBox uses `/etc/nixos/configuration.nix`. The generated files still stay under the configuration root.

`NIXBOX_CONFIG_DIR` does not move `settings.json`, `state.json`, or the package catalog cache.

## Package catalog cache

The catalog lives at `$XDG_CACHE_HOME/nixbox/package-catalog.json`, normally `~/.cache/nixbox/package-catalog.json`. It contains a format version, the locked flake reference and revision, and all parsed package hits. NixBox accepts the cache only when its format and source match the current lock file.

NixBox writes a new catalog to `package-catalog.json.tmp` and renames it over the cache after serialization succeeds. Removing the cache is safe; the next launch rebuilds it from the locked nixpkgs revision.

## Recovery state

NixBox stores unfinished work at `$XDG_CONFIG_HOME/nixbox/state.json`, normally `~/.config/nixbox/state.json`. The file may contain queued operations, one in-progress rebuild descriptor, and the last rebuild error.

When state becomes empty, NixBox removes the file. Load and save failures are intentionally non-fatal. A malformed state file is ignored, so it may need manual inspection if expected recovery does not happen.

Both front-ends read and write this one file. Starting the TUI restores whatever it holds, and `nixbox resume` does the same from the command line: it re-runs an interrupted rebuild and applies a queued batch one target at a time. `nixbox resume --dry-run` prints the contents without acting on them, and `nixbox resume --discard` clears the file.

The format is defined by `nixbox-core`, not by the UI crate, so `nixbox-cli` reads the same file.

## Generated Nix files

Generated files begin with `# Managed by nixbox. Do not edit by hand.` NixBox rewrites each generated file from its parsed in-memory manifest rather than preserving arbitrary manual edits. Keep custom Nix expressions in your own modules and import the generated files.

See [Managed files and package operations](MANAGED_FILES.md) for the exact generated forms and mutation rules.
