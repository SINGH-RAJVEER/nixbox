# Version control

NixBox can manage a Git or Jujutsu repository for the whole configuration directory. It includes manual edits and new files under that root, not only NixBox-generated modules. Ignored files are excluded. Initialization, commit, push, and GitHub repository creation are explicit actions; installing packages or rebuilding never publishes your configuration.

## Choose the configuration directory

Set `NIXBOX_CONFIG_DIR` to the configuration directory you want to version. By default it is `$XDG_CONFIG_HOME/nixos`, normally `~/.config/nixos`. Keep the main NixOS and Home Manager files inside it. The `/etc/nixos/configuration.nix` fallback and external main-file overrides are not included in a commit of another root. Pending journal entries for such files cause commit to refuse with an error.

Detection finds the nearest repository. A colocated JJ repository uses JJ controls. Git repositories in parent directories support config-only commits, but unrelated staged files must be unstaged first. JJ commits and pushes in parent repositories are refused; use a dedicated config repository instead. Initialization refuses to nest a new repository inside a detected one.

## Review and commit

Open the **Version control** tab in the GUI or the **VCS** tab in the TUI. `Ctrl-R` selects the TUI tab, and `Tab` / `Shift-Tab` cycles through it alongside the other tabs. Initialize with Git or colocated JJ if no repository exists. From the CLI, use Git or JJ directly for initialization, inspection, remotes, bookmarks, and pushing. NixBox exposes only `nixbox commit`, with no `vcs` subcommand.

Review the full status and diff before committing. The proposed message starts with `nixbox: update configuration` and sorts descriptions of successful app operations since the last app commit. Descriptions include targets and package or flake identifiers; option values are omitted. You can edit the message before approval. Manual-only changes use the default subject. The journal records operations rather than calculating a net semantic diff, so opposite edits can both appear in the message.

The backend rejects a blank message, an empty diff, or a review that no longer matches the files, revision, and journal. A successful commit clears the pending journal even if a later push fails. Retry push separately; do not create another commit just to retry publication. External commits do not clear NixBox's journal.

Git and JJ need your configured name and email. NixBox does not supply an identity or change your identity settings.

## Push or create origin

Push an explicit local Git branch or JJ bookmark to `origin`. There is no force-push action. For JJ, opt into **Create/advance nixbox bookmark** to create or fast-forward advance `nixbox` to the last committed change at `@-`. Without that opt-in, push uses the existing bookmark. Resolve divergent history outside NixBox before retrying.

If no origin exists, enter an explicit GitHub repository name and visibility. This action requires installed and authenticated `gh`, defaults to private, creates the GitHub repository, and adds origin. It neither commits nor pushes. If adding origin fails after creation, the GitHub repository remains; inspect it before retrying.

Review diffs can contain secrets, even though suggested messages omit option values. A private repository does not make committing credentials safe. Use ignore rules or move secrets outside the repository before approving a commit.

## Dependencies and limitations

Git controls require `git`; JJ controls require `jj`. GitHub creation alone requires `gh`. Nix packages include these executables in the wrapper path. Network authentication remains the user's responsibility.

Interactive clients run repository operations on workers and report errors in the version control tab. Subprocesses have no cancellation or timeout control. App mutations and commits share a journal lock, but external editors and VCS commands do not; avoid concurrent writes while committing. After a crash, inspect any `.lock` file under the journal directory before removing it.

See the [CLI reference](CLI.md), [TUI guide](USER_GUIDE.md), and [GUI guide](GUI.md) for individual controls.
