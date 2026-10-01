# Command line interface

Running `nixbox` with no subcommand starts the terminal UI. Package operations are also available as subcommands, so they can be scripted, run over SSH, or put in a shell alias.

Both front-ends go through `nixbox-core`. A package installed from the command line and the same package installed in the TUI take the identical code path, read the same catalog, and produce the identical files. They also share `state.json`, so a queue built in the TUI can be finished with `nixbox resume`, and a rebuild the CLI started and lost can be picked up by either.

The terminal UI is optional. See [Two packages](#two-packages) for the build that is only the command line.

## Global flags

| Flag | Effect |
| --- | --- |
| `--target`, `-t` | Act on `home-manager` or `nixos` instead of the saved target. Accepts `hm` and `system` as aliases. |
| `--channel`, `-c` | Search a different nixpkgs channel for this invocation only. |
| `--json` | Print JSON instead of a table. |

None of these are written back to the settings file. `nixbox config set` is the only command that persists a change.

## Reading state

| Command | Purpose |
| --- | --- |
| `nixbox search <query>` | Search the active channel. `-n` limits the result count. Uses the local package catalog when one is available and falls back to live search otherwise; `--channel` always searches live. |
| `nixbox list` | List the packages NixBox manages for the active target. `-a` covers both. |
| `nixbox scan` | List packages declared by hand in your own configuration that NixBox does not manage. |
| `nixbox status` | Show the active target, the files NixBox owns, and whether the import is wired up. |
| `nixbox doctor` | Check that Nix, the config directory, Git, the flake, the main config, the import, and `gh` are all in place. |

## Changing the configuration

| Command | Purpose |
| --- | --- |
| `nixbox install <package>...` | Add packages and rebuild. |
| `nixbox remove <package>...` | Drop packages and rebuild. Aliases: `rm`, `uninstall`. |
| `nixbox migrate <package>...` | Move hand-declared packages into the managed file. `-a` migrates everything that can move cleanly. |
| `nixbox apply` | Rewrite the managed file and rebuild without changing the package set. |
| `nixbox resume` | Finish work a previous run left behind: an interrupted rebuild, or a queue the TUI saved. `--discard` throws it away instead. |

Each of these accepts:

| Flag | Effect |
| --- | --- |
| `--dry-run` | Print what would change and exit without touching anything. |
| `--no-rebuild` | Write the configuration change but skip the rebuild. |
| `--yes`, `-y` | Do not ask before changing the configuration. |

Without a terminal on stdin, a change requires `--yes`. This is deliberate: a script that forgets it stops rather than rebuilding a machine unattended.

Writing happens before the rebuild. If a rebuild fails, the files already reflect the intent, so recovering is `nixbox apply` rather than repeating the original command.

## Committing configuration changes

`nixbox commit` acts on the saved configuration root, including manual edits and untracked files that are not ignored. It loads settings directly without running Nix or loading package manifests. JJ takes priority when the configuration belongs to a colocated repository. Changing packages or settings never automatically commits or pushes.

| Command | Purpose |
| --- | --- |
| `nixbox commit [--edit]` | Generate the message, print the exact status, full diff, and final message, then ask for approval. Commit that same review. |

The default message is generated deterministically from the shared backend's journal of successful operations. Descriptions are sorted and omit option values. With no recorded operations, the message is `nixbox: update configuration`.

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

A rebuild is recorded in `state.json` before it starts and cleared when it reaches a verdict, so a run killed mid-rebuild leaves a marker behind. `nixbox resume` re-runs that rebuild; the configuration was already written, so nothing else needs redoing.

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

| Command | Purpose |
| --- | --- |
| `nixbox config show` | Print every setting. |
| `nixbox config get <key>` | Print one setting. |
| `nixbox config set <key> <value>` | Change one setting. An empty value clears an optional path. |
| `nixbox config path` | Print the path of the settings file. |
| `nixbox completions <shell>` | Print a completion script. |

## Two packages

NixBox publishes two binaries. They are the same program with the same subcommands:

| Install | Command | Terminal UI |
| --- | --- | --- |
| `cargo install nixbox` | `nixbox` | yes |
| `cargo install nixbox-cli` | `nixbox-cli` | no |

`nixbox-cli` exists for machines that will never run a UI: servers, CI, containers, and anything reached over SSH. It leaves out `ratatui`, `crossterm`, and `tui-input` entirely.

The two install under different names, so one machine can have both. Everything else is identical, including `nixbox config set theme`, because the theme names live in `nixbox-config` rather than in the UI. The only visible differences are that `nixbox-cli` has no `tui` subcommand, and that running it with no subcommand prints help and exits `1` instead of opening a UI.

Help text, generated completions, and hints like "run `nixbox-cli apply`" all use whichever name you installed.

The Nix flake exposes both:

```sh
nix profile install github:SINGH-RAJVEER/nixbox#nixbox
nix profile install github:SINGH-RAJVEER/nixbox#nixbox-cli
```

Examples in this document say `nixbox`; substitute `nixbox-cli` if that is what you installed.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | The command ran and reported a problem. |
| `2` | The arguments were wrong. Clap reports this. |
| `3` | The configuration was written but the rebuild failed. |

`3` is separate from `1` so a script can tell "NixBox could not do it" from "Nix said no". A `3` means the files are already correct and `nixbox apply` will retry the rebuild.
