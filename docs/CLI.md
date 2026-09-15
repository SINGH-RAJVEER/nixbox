# Command line interface

NixBox is a TUI first, and running `nixbox` with no subcommand still starts it. Every operation the TUI performs is also available as a subcommand, so the same work can be scripted, run over SSH, or put in a shell alias.

Both front-ends go through `nixbox-core`. A package installed from the command line and the same package installed in the TUI take the identical code path and produce the identical files.

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
| `nixbox search <query>` | Search the active channel. `-n` limits the result count. |
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

Each of these accepts:

| Flag | Effect |
| --- | --- |
| `--dry-run` | Print what would change and exit without touching anything. |
| `--no-rebuild` | Write the configuration change but skip the rebuild. |
| `--yes`, `-y` | Do not ask before changing the configuration. |

Without a terminal on stdin, a change requires `--yes`. This is deliberate: a script that forgets it stops rather than rebuilding a machine unattended.

Writing happens before the rebuild. If a rebuild fails, the files already reflect the intent, so recovering is `nixbox apply` rather than repeating the original command.

## Flakes

| Command | Purpose |
| --- | --- |
| `nixbox flake search <query>` | Search GitHub for flakes. Requires `gh auth login`. |
| `nixbox flake info <owner/repo>` | Show what a flake publishes, including the modules and packages it actually evaluates to. |
| `nixbox flake list` | List the flake outputs NixBox manages for the active target. |
| `nixbox flake add <owner/repo>` | Add the flake as an input and wire an output into your configuration. |
| `nixbox flake remove <owner/repo>` | Drop the flake's output and its input. Alias: `rm`. |

`flake add` installs the default module for the active target when the flake publishes one, and otherwise falls back to the flake's first package. Both come from evaluating the flake rather than from the names of its outputs, so a flake whose outputs cannot be imported is refused instead of producing a configuration that will not build.

## Settings and completions

| Command | Purpose |
| --- | --- |
| `nixbox config show` | Print every setting. |
| `nixbox config get <key>` | Print one setting. |
| `nixbox config set <key> <value>` | Change one setting. An empty value clears an optional path. |
| `nixbox config path` | Print the path of the settings file. |
| `nixbox completions <shell>` | Print a completion script. |

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | The command ran and reported a problem. |
| `2` | The arguments were wrong. Clap reports this. |
| `3` | The configuration was written but the rebuild failed. |

`3` is separate from `1` so a script can tell "NixBox could not do it" from "Nix said no". A `3` means the files are already correct and `nixbox apply` will retry the rebuild.
