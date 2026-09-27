# Package options

Press `Enter` on a package in the terminal UI's Installed tab, or click Options on its row in the desktop GUI's Installed page, to open its options. The panel lists the module options your configuration evaluates for that package, such as `programs.git.*`, with their current values, and lets you change the simple ones. Changes are staged in the panel and applied together as one queued operation and one rebuild.

Both front-ends share the evaluation, the editing rules, and the queued operation described here. The command line does not have an options command yet.

## Where the options come from

NixBox evaluates your own flake with `nix eval --impure`, so the panel shows the options and values of the configuration you actually build, not a generic option index.

| Row scope | Configuration evaluated |
| --- | --- |
| `[nixos]` | `nixosConfigurations.nixos` |
| `[hm]`, standalone Home Manager | `homeConfigurations.<USER>` |
| `[hm]`, Home Manager as a NixOS module | `home-manager.users.<USER>` inside `nixosConfigurations.nixos` |

The Home Manager source is picked the same way as the rebuild command (see [Rebuild command selection](MANAGED_FILES.md#rebuild-command-selection)). The Home Manager module case reads options through the `valueMeta` of `home-manager.users`, which needs a recent nixpkgs.

To find a package's options, NixBox tries `programs.<name>` and `services.<name>`, where `<name>` is the last segment of the package attribute. It then tries the same name without a trailing version, so `nodejs_22` also tries `nodejs`. If none of these exist, it looks for a `programs.*` or `services.*` module whose `package` option defaults to `pkgs.<attribute>`, which is how `neovim-unwrapped` finds `programs.neovim`. Packages with no such module show "No module options found".

Internal options, hidden options such as renamed aliases, the per-name templates of `attrsOf submodule` options, and the sub-options of submodule-typed options such as `services.resolved.settings` are left out. The submodule option itself is listed, read-only. Each option's default, example, description, type, value, and defining files are evaluated separately, so one that throws shows as unavailable without hiding the rest.

Results are cached per package and scope while NixBox runs. The cache is cleared whenever a rebuild ends, and an open panel reloads. Press `r` (or `Ctrl+r`) to read the options again, and to retry after an evaluation error. One evaluation is limited to 120 seconds.

## What can be edited

An option is editable when all of the following hold:

- It is not read-only.
- Its type is one NixBox can write and read back: `bool`, the integer types, `float`, `str` and its constrained forms, `enum` of strings, `list of string`, or `null or` any of these.
- No file in your configuration, other than NixBox's settings module, defines it.
- No upstream module defines it at normal priority. String lists are exempt, because list definitions merge.

The last rule prevents broken rebuilds. If `configuration.nix` sets `programs.git.enable` and NixBox also wrote it, the module system would reject the two definitions for any type that does not merge. Such rows are dimmed and show `set in <file>`. Change those options in your own file, or remove your definition first. An upstream module can also set an option, for example when one program's module configures another. Definitions at `mkDefault` or option-default priority do not lock an option, because NixBox's plain definition overrides them. A definition at normal priority does lock it, and the row shows `set by another module`.

Other types, such as attribute sets, packages, submodules, and multi-line `lines` strings, are shown read-only with their evaluated value.

## Editing in the terminal UI

| Option type | `Enter` does |
| --- | --- |
| `bool` | Toggles the value. |
| `enum`, or `null or bool` | Opens a list of the allowed values, with `null` first when the option accepts it. |
| String and number types | Opens a text field. For `null or` types, an empty field means `null`. |
| `list of string` | Opens a text field of comma-separated items. An empty field means `[ ]`. Items that contain commas cannot be entered this way. |

In the editors, `Enter` stages the value and `Esc` cancels. A staged row is marked `*`. A row whose change is waiting in the queue is marked `~` and shows the queued value.

| Key (Vim input mode) | Key (Normal input mode) | Action |
| --- | --- | --- |
| `j`, `k`, arrows | arrows | Move. |
| `/` | type | Filter options by path. `/` starts a new filter and `i` edits the current one. |
| `Enter` | `Enter` | Edit the selected option. |
| `u` | `Ctrl+u` | Stage removal of NixBox's value, returning the option to its default. On an option NixBox does not set, drop its staged change instead. |
| `U` | `Ctrl+z` | Discard every staged change. |
| `w` | `Ctrl+w` | Apply the staged changes. |
| `r` | `Ctrl+r` | Read the options again. |
| `Esc` | `Esc` | Close the panel. With staged changes, the first `Esc` warns and a second one discards them. |

`Tab`, `Shift+Tab`, and `Ctrl+s` keep their usual meaning. The panel stays open when you switch tabs and come back.

## Editing in the desktop GUI

The Options page lists the options on the left and the selected option on the right, with its type, value, default, example, status, and description. A filter box narrows the list. Tags mark options that are staged, queued, or set by NixBox, and locked options say why.

| Option type | Editor |
| --- | --- |
| `bool` | A switch. |
| `enum`, or `null or bool` | A row of buttons for the allowed values, with `null` first when the option accepts it. Enum values are shown quoted, so the string `"true"` is distinct from the boolean `true`. |
| String, number, and `list of string` types | A text field, read the same way as in the terminal UI. Press `Enter` or Stage. |

Unset stages removal of NixBox's value. Discard drops every staged change, Apply queues them as one operation, and Reload reads the options again. Back returns to the Installed page; with staged changes, the first click warns and a second one discards them. The Installed item in the sidebar stays highlighted while the Options page is open.

## Applying

Applying queues one `SetOptions` operation carrying every staged change for the panel's scope. If a `SetOptions` for the same scope is already waiting behind a running rebuild, the new changes merge into it and newer values win, so several rounds of edits still produce one rebuild.

When the operation runs, NixBox writes the values into `nixbox-home-settings.nix` or `nixbox-system-settings.nix` and imports that file into the target's entry file. It marks the files for Git visibility and rebuilds, like any other operation. See [Generated settings modules](MANAGED_FILES.md#generated-settings-modules) for the file format.

A value that passes the panel's checks can still fail the rebuild, for example an integer outside a port range. The file keeps the value, as with any failed rebuild. Open the panel again and change or unset it.
