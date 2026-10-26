# Command line interface

The CLI is installed independently of the GUI and TUI:

```sh
nix profile add github:SINGH-RAJVEER/nixbox#nixbox-cli
```

That package provides `nixbox`, `nb`, and the compatibility name `nixbox-cli`. Bare `nixbox` and `nb` show help and exit successfully. They never launch either interface and have no `--gui` or `--tui` switches. GUI-only and TUI-only installations do not provide these commands or detect terminal invocations. See [Terminal applications](TERMINAL_APPS.md) for profile and declarative flake installation steps.

Every front-end goes through `nixbox-core`. A package installed from the command line and the same package installed in either interface take the identical code path, read the same catalog, and produce the identical files. They also share `state.json`, so a queue built in either interface can be finished with `nixbox resume`, and a rebuild the CLI started and lost can be picked up by either.

Neither interface is required. See [Three packages](#three-packages) for the build that is only the command line.

## Global flags

| Flag | Effect |
| --- | --- |
| `--target`, `-t` | Act on `home-manager` or `nixos` instead of the saved target. Accepts `hm` and `system` as aliases. |
| `--channel`, `-c` | Search a different nixpkgs channel for this invocation only. |
| `--json` | Print JSON instead of a table. |
| `--home-configuration <name>` | Select a standalone dendritic Home Manager output. Mutually exclusive with host/user flags. |
| `--host <name>` | Select a discovered dendritic NixOS output. Required when several hosts exist. |
| `--home-user <name>` | Select a directly attached Home Manager user on that host. Required for Home Manager writes when several users exist. |

None of these are written back to the settings file. `nixbox config set` is the only command that persists a change.

## Reading state

`nixbox config inspect` discovers hosts and users and reports the selected identity, managed-file paths, placement, and unsupported-layout diagnostics without writing configuration or loading migrations. Add `--json` for machine-readable output. Both interactive interfaces also offer Settings > Configuration. Save a selection with `nixbox config set nixos-host <name>` and `nixbox config set home-user <name>`; an empty value returns to automatic selection. Changing the saved host clears the saved user. Changing the saved root clears both. A standalone output can be saved with `nixbox config set home-configuration <name>`. Pending recovery work remains bound to its original root, host, and user; select those again or explicitly discard that work.

For supported dendritic configurations, `status` and `doctor` report `flake.nix` as the integration entry point. `scan` includes packages in connected classed feature modules and marks those external entries read-only. Installs, managed removals, and profile operations use generated flake-parts modules. Unsupported layouts fail with a discovery diagnostic. See [Dendritic support](DENDRITIC_PLAN.md).

`nixbox config set dendritic-detection false` disables native discovery and uses conventional file placement. Set it to `true` to restore automatic dendritic handling. Saved work must be completed or discarded before changing modes. Disabled detection refuses writes or rebuilds in a dendritic tree; it does not convert the tree to a conventional layout.

| Command | Purpose |
| --- | --- |
| `nixbox search <query>` | Search the active channel. `-n` limits the result count. Uses the local package catalog when one is available and falls back to live search otherwise; `--channel` always searches live. |
| `nixbox list` | List the packages NixBox manages for the active target. `-a` covers both. |
| `nixbox scan` | List packages declared by hand in your own configuration that NixBox does not manage. |
| `nixbox status` | Show the active target, the files NixBox owns, whether the import is wired up, and how far package profile setup has got. |
| `nixbox doctor` | Check that Nix, the config directory, Git, the flake, the main config, the import, and `gh` are all in place, and warn when a package profile is set up but not active. |

## Changing the configuration

| Command | Purpose |
| --- | --- |
| `nixbox install <package>...` | Add packages and rebuild. With a package profile set up, update the profile instead. |
| `nixbox remove <package>...` | Drop packages and rebuild, or update the profile. Aliases: `rm`, `uninstall`. |
| `nixbox migrate <package>...` | Move hand-declared packages into the managed file. `-a` migrates everything that can move cleanly. |
| `nixbox apply` | Rewrite the managed file and rebuild without changing the package set. With a package profile, update the profile; `--configuration` rebuilds instead. |
| `nixbox profile <setup\|teardown\|generations\|rollback>` | Move the target's packages into an independent profile and back, list its generations, or roll it back. See [Package profiles](#package-profiles). |
| `nixbox resume` | Finish work a previous run left behind: an interrupted rebuild, or a queue the TUI saved. `--discard` throws it away instead. |

Each of these accepts:

| Flag | Effect |
| --- | --- |
| `--dry-run` | Print what would change and exit without touching anything. |
| `--no-rebuild`, `--no-activate` | Write the configuration change but skip the rebuild or profile update. |
| `--yes`, `-y` | Do not ask before changing the configuration. |

Without a terminal on stdin, a change requires `--yes`. This is deliberate: a script that forgets it stops rather than rebuilding a machine unattended.

Writing happens before the rebuild. If a rebuild fails, the files already reflect the intent, so recovering is `nixbox apply` rather than repeating the original command. With a package profile, a failed module, option or migration change is recovered with `nixbox apply --configuration`; the failure message names the right command.

## Package profiles

By default every package change rebuilds NixOS or Home Manager. `nixbox --target <target> profile setup` moves that target's packages into an independent Nix profile instead: installs and removals then build only the profile and never rebuild the configuration. Building the NixOS profile still asks for sudo; the Home Manager profile belongs to you. Setup rebuilds the configuration once, to put the profile on PATH. Modules, options and migrations still rebuild, because they reach beyond a list of packages.

| Command | Purpose |
| --- | --- |
| `nixbox profile setup` | Rewrite the generated files in profile mode, add the PATH wiring, and rebuild once. Accepts the flags above. |
| `nixbox profile teardown` | Move the packages back into the configuration and rebuild. The profile stays on disk until you delete it. |
| `nixbox profile generations` | List the profile's generations. |
| `nixbox profile rollback [--to <generation>]` | Point the profile at its previous generation, or at the one given. |

Every rebuild of the configuration, and on NixOS every boot, also sets the profile to the configuration's package set, so the profile follows a fresh install, `nixos-rebuild --rollback`, the boot menu and `nix flake update`. A profile rollback is therefore a quick undo, not a permanent one: the generated file still lists the packages, so the next install, `apply`, rebuild or reboot restores them. Remove the packages to make it permanent.

After setup, start a new login session so shells and the desktop see the profile. `status` and `doctor` report the setup as not set up, not activated on this machine, activated but not on this process's PATH, or active. The mode is recorded in the generated package file, so it travels with the configuration repository: on another machine, `nixbox apply --configuration` or any rebuild activates it. [Independent package profiles](MANAGED_FILES.md#independent-package-profiles) describes the files and commands involved.

## Committing configuration changes

`nixbox commit` acts on the saved configuration root, including manual edits and untracked files that are not ignored. It loads settings directly without running Nix or loading package manifests. JJ takes priority when the configuration belongs to a colocated repository. Changing packages or settings never automatically commits or pushes.

| Command | Purpose |
| --- | --- |
| `nixbox commit [--edit]` | Generate the message, print the exact status, full diff, and final message, then ask for approval. Commit that same review. |

The default message is generated deterministically from the shared backend's journal of successful configuration writes. It uses an action-specific subject or target summary, then numbers every applied action in order. Repetitions, reversals, individual option edits, and migrated packages remain visible; option values are omitted. With no recorded operations, the message is `nixbox: update configuration`. See [Version control](VERSION_CONTROL.md#review-and-commit) for an example and external-commit handling.

`commit` accepts `--yes` / `-y` and `--dry-run`. It always shows the full review before confirmation, including with `--yes`. A non-terminal stdin requires `--yes`. A dry run skips confirmation and mutation even with `--yes`. JJ commit dry runs are refused before review because the shared backend's review API snapshots the working copy. Ordinary JJ commit reviews can update snapshot metadata.

Every commit starts with the generated message; there is no `-m` or `--message` override. `--edit` writes the suggestion to a private temporary file and invokes `$EDITOR` directly with the file path as its only argument. Set `EDITOR` to one executable name or path, such as `vim` or `/usr/bin/nano`. Embedded arguments, quotes, shell operators, and variable expansion are not interpreted. Use an executable wrapper if your editor needs flags. Editor failure, invalid UTF-8, and blank messages stop the commit. The temporary file is removed on normal success or error. `--dry-run --edit` skips the editor and shows the unedited message for Git; JJ still refuses the dry run.

The core rejects a commit if repository state, configuration files, or the operation journal changed after review. Git commits are limited to the configuration scope and refuse unrelated staged files. JJ commits require a repository rooted at the configuration directory. Commit output includes the revision and any journal-cleanup warning. Diffs contain the actual configuration contents; message redaction does not redact the diff.

Use Git or JJ directly for repository setup, status, diffs, remotes, bookmarks, and pushing. For example, run `git init` or `jj git init --colocate` in the configuration directory before the first commit.

```sh
nixbox commit
nixbox commit --dry-run  # Git only
EDITOR=vim nixbox commit --edit
nixbox commit --yes
nixbox-cli commit --edit -y
```

`commit` prints text and rejects `--json`. `--target` and `--channel` do not narrow the repository scope.

## Resuming

A rebuild is recorded in `state.json` before it starts and cleared when it reaches a verdict, so a run killed mid-rebuild leaves a marker behind. `nixbox resume` re-runs that rebuild; the configuration was already written, so nothing else needs redoing. The record says whether it was a profile update or a configuration rebuild, and `resume` repeats the same kind.

The TUI queues operations and applies them in a batch, and saves that queue to the same file. `nixbox resume` drains it one target at a time, in the order the queue first mentions each target, with one rebuild per target. `--dry-run` prints the queue without applying it, and `--discard` clears the file.

## Flakes

| Command | Purpose |
| --- | --- |
| `nixbox flake search <query>` | Search GitHub for flakes. Requires `gh auth login`. |
| `nixbox flake info <owner/repo>` | List evaluated package and module paths plus other top-level output families; `--json` includes `available_outputs` and `installable_outputs`. |
| `nixbox flake list` | List the flake modules and packages NixBox manages for the active target. `--json` prints `repo`, `kind` (`module` or `package`), and `output` for each. |
| `nixbox flake add <owner/repo> [--output <path>]` | Add the flake as an input and wire an output into your configuration. `--output` selects an exact path from `flake info` and allows adding another output from the same repository. |
| `nixbox flake remove <owner/repo>` | Drop every output NixBox manages for the flake, and its input once nothing else uses it. Alias: `rm`. |
| `nixbox flake migrate <input>#<package>` | Move an existing hand-declared flake package into NixBox's generated module for the active target. `-a` migrates every eligible flake package in that target. |

Without `--output`, `flake add` installs the flake's first package on the Home Manager target, falling back to a Home Manager module when it has no packages. On the NixOS target it installs a NixOS module when available and otherwise falls back to the first package. Default modules take priority over other named modules. Quote paths containing shell-special characters, for example `nixbox flake add owner/repo --output 'packages.x86_64-linux."my.tool"'`. Packages can be selected for either target; NixOS and Home Manager modules can only be selected for their respective targets. `flake remove` keeps the root input when other files in the configuration still reference it. Outputs are discovered by evaluating the flake.

## Settings and completions

Optional usage collection defaults to off. `nixbox config get telemetry-enabled` reads saved consent and `nixbox config set telemetry-enabled false` disables it. Enabling requires the literal `true` and explicitly consents to sending the disclosed counts to `https://nixb.duckdns.org:58443/v1/telemetry`. Debug, CI, and VM builds still suppress delivery. The CLI never uses an environment variable to enable collection.

`nixbox telemetry status`, also available with `--json`, prints saved consent, effective status, recipient, and the collection disclosure. `nixbox telemetry preview` prints a synthetic report through the actual serializer. Both commands are offline and do not initialize collection. Help, version, completions, and config commands also produce no reports. Other commands can count one dispatch and completed activation outcomes after consent, with at most 100 ms for delivery at exit. CLI aliases all use the same `cli` dimension. See [Optional telemetry](TELEMETRY_PLAN.md) for the exact fields and receiver.

| Command | Purpose |
| --- | --- |
| `nixbox config show` | Print every setting. |
| `nixbox config get <key>` | Print one setting. |
| `nixbox config set <key> <value>` | Change one setting. An empty value clears an optional path. |
| `nixbox config path` | Print the path of the settings file. |
| `nixbox completions <shell>` | Print a completion script. |

## Three packages

Each package installs its own interface, with no executable-name collisions:

| Install | Executables | Bare invocation |
| --- | --- | --- |
| `cargo install nixbox` | `nixbox-gui` | opens the desktop GUI |
| `cargo install nixbox-tui` | `nixbox-tui` | opens the terminal UI |
| `cargo install nixbox-cli` | `nixbox`, `nb`, `nixbox-cli` | the aliases show help; `nixbox-cli` prints help and exits `1` |

The GUI and TUI depend on the shared engine, not `nixbox-cmd`, and do not include CLI commands. Install `nixbox-cli` separately when you want the CLI, with or without either interface. The CLI leaves out Ratatui, Crossterm, gpui, and both front-end crates.

Help, generated completions, and recovery hints use the executable name you ran. All examples here use `nixbox`; `nb` and `nixbox-cli` accept the same commands.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | The command ran and reported a problem. |
| `2` | The arguments were wrong. Clap reports this. |
| `3` | The configuration was written but the rebuild failed. |

`3` is separate from `1` so a script can tell "NixBox could not do it" from "Nix said no". A `3` means the files are already correct and `nixbox apply` will retry the rebuild.

For explicit managed-module conversion, preview with `nixbox config migrate-dendritic --dry-run --json`, then apply with `--yes`. Add `--relocate` to move existing root-level dendritic files into the tree. Host/user or standalone-output flags select the migration destination. This command changes files without activating them.
