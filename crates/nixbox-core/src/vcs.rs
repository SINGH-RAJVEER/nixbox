//! Synchronous Git/Jujutsu controls shared by every frontend.
//!
//! Clients must display `Review::status`, `diff`, and `message`, obtain explicit
//! approval, then pass that same review to `commit`. Creation never pushes.
//! Commands use argument arrays. No command runs through a shell.
//!
//! Public synchronous API:
//! - `new` / `for_root`: select one canonical configuration directory.
//! - `detect` / `repository`: discover the nearest repository, preferring JJ at the same root.
//! - `init`: explicitly initialize Git or colocated JJ, without a commit.
//! - `status` / `diff`: inspect all non-ignored config changes, including manual edits.
//! - `journal` / `suggested_message`: read durable successful operations, with option
//!   values omitted and operation descriptions sorted in the proposed message.
//! - `review`: capture status, diff, repository revision and journal, optionally
//!   using an edited message. Clients must obtain approval before `commit`.
//! - `commit`: recheck the review and commit the whole config scope. Git parent
//!   repositories are path-limited; unrelated staged paths cause refusal. JJ
//!   parent repositories are refused because their working-copy commit is shared.
//! - `origin`: inspect only the origin remote.
//! - `push`: push an explicit branch/bookmark, optionally creating or advancing the JJ
//!   `nixbox` bookmark to @- after fast-forward checks. Never force-pushes.
//! - `create_remote`: create an explicitly named public/private GitHub repository
//!   and add origin. Requires a config-root repository and never pushes.
//!
//! Ignored files are not committed. Review diffs may contain secrets from the
//! configuration itself even though journal descriptions omit option values.
//! Calls may block on subprocesses; interactive clients should use a worker.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail, ensure};
use nixbox_config::{Config, Target, settings_path};
use serde::{Deserialize, Serialize};

use crate::Op;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
	Git,
	Jj,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
	pub backend: Backend,
	pub root: PathBuf,
	pub config_root: PathBuf,
}

/// A review is bound to the repository, current files, and pending journal.
/// Public presentation fields are read-only through accessors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
	repository: Repository,
	status: String,
	diff: String,
	message: String,
	identity: String,
	journal: Vec<JournalEntry>,
}

impl Review {
	pub fn repository(&self) -> &Repository {
		&self.repository
	}
	pub fn status(&self) -> &str {
		&self.status
	}
	pub fn diff(&self) -> &str {
		&self.diff
	}
	pub fn message(&self) -> &str {
		&self.message
	}
}

#[derive(Debug)]
pub struct CommitOutcome {
	pub revision: String,
	/// A successful commit stays successful if journal cleanup fails.
	pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Visibility {
	Private,
	Public,
}

/// Only redacted operation descriptions are stored, never option values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
	pub operation: String,
	pub changed_files: Vec<PathBuf>,
}

#[derive(Default, Serialize, Deserialize)]
struct Journal {
	root: PathBuf,
	entries: Vec<JournalEntry>,
}

struct JournalLock(PathBuf);
impl Drop for JournalLock {
	fn drop(&mut self) {
		let _ = fs::remove_file(&self.0);
	}
}

/// Each instance controls exactly one canonical config directory.
pub struct Vcs {
	root: PathBuf,
	journal_path: PathBuf,
}

impl Vcs {
	pub fn new(config: &Config) -> Result<Self> {
		Self::for_root(config.config_root())
	}

	pub fn for_root(root: impl AsRef<Path>) -> Result<Self> {
		let root =
			resolved_path(&std::path::absolute(root)?).context("locating configuration root")?;
		// Stable FNV-1a, with the full canonical path checked on every read.
		let hash = root
			.as_os_str()
			.as_encoded_bytes()
			.iter()
			.fold(0xcbf29ce484222325u64, |hash, byte| {
				(hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
			});
		let directory = settings_path()?
			.parent()
			.context("settings directory")?
			.join("vcs-journal");
		ensure!(
			!directory.starts_with(&root),
			"journal directory must be outside the configuration root"
		);
		Ok(Self {
			root,
			journal_path: directory.join(format!("{hash:016x}.json")),
		})
	}

	/// Stops at the nearest repository boundary; JJ wins only at the same root.
	pub fn detect(&self) -> Result<Option<Repository>> {
		for parent in self.root.ancestors() {
			if parent.join(".jj").is_dir() {
				let root = text(run(parent, "jj", &["root"])?)?;
				return Ok(Some(Repository {
					backend: Backend::Jj,
					root: fs::canonicalize(root.trim())?,
					config_root: self.root.clone(),
				}));
			}
			if parent.join(".git").exists() {
				let root = text(run(parent, "git", &["rev-parse", "--show-toplevel"])?)?;
				return Ok(Some(Repository {
					backend: Backend::Git,
					root: fs::canonicalize(root.trim())?,
					config_root: self.root.clone(),
				}));
			}
		}
		let output = Command::new("git")
			.current_dir(
				self.root
					.ancestors()
					.find(|path| path.is_dir())
					.context("locating existing configuration ancestor")?,
			)
			.args(["rev-parse", "--show-toplevel"])
			.output();
		match output {
			Ok(output) if output.status.success() => Ok(Some(Repository {
				backend: Backend::Git,
				root: fs::canonicalize(text(output)?.trim())?,
				config_root: self.root.clone(),
			})),
			Ok(_) => Ok(None),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
			Err(error) => Err(error.into()),
		}
	}

	/// Explicitly initializes Git or colocated JJ; refuses any existing repo.
	pub fn init(&self, backend: Backend) -> Result<Repository> {
		ensure!(
			self.detect()?.is_none(),
			"configuration already belongs to a repository"
		);
		fs::create_dir_all(&self.root).context("creating configuration root")?;
		match backend {
			Backend::Git => {
				run(&self.root, "git", &["init"])?;
			}
			Backend::Jj => {
				run(&self.root, "jj", &["git", "init", "--colocate"])?;
			}
		}
		self.repository()
	}

	pub fn journal(&self) -> Result<Vec<JournalEntry>> {
		Ok(self.read_journal()?.entries)
	}

	pub fn suggested_message(&self) -> Result<String> {
		let mut operations: Vec<_> = self
			.journal()?
			.into_iter()
			.map(|entry| entry.operation)
			.collect();
		operations.sort();
		if operations.is_empty() {
			return Ok("nixbox: update configuration".into());
		}
		Ok(format!(
			"nixbox: update configuration\n\n{}",
			operations
				.into_iter()
				.map(|op| format!("- {op}"))
				.collect::<Vec<_>>()
				.join("\n")
		))
	}

	/// Reviews the entire config, including manual edits and untracked files.
	/// An override is used verbatim, except that blank messages are rejected.
	pub fn review(&self, message: Option<&str>) -> Result<Review> {
		let repository = self.repository()?;
		let message = message
			.map(str::to_owned)
			.map(Ok)
			.unwrap_or_else(|| self.suggested_message())?;
		ensure!(!message.trim().is_empty(), "commit message cannot be blank");
		let (status, diff, identity) = self.inspect(&repository)?;
		Ok(Review {
			repository,
			status,
			diff,
			message,
			identity,
			journal: self.journal()?,
		})
	}

	fn inspect(&self, repository: &Repository) -> Result<(String, String, String)> {
		let (status, diff, identity) = match repository.backend {
			Backend::Git => {
				let scope = git_scope(repository)?;
				let status = text(run(
					&repository.root,
					"git",
					&["status", "--short", "--untracked-files=all", "--", &scope],
				)?)?;
				let head = run(&repository.root, "git", &["rev-parse", "--verify", "HEAD"]);
				let identity = match head {
					Ok(output) => text(output)?,
					Err(_) => String::new(),
				};
				let mut diff = if identity.is_empty() {
					text(run(
						&repository.root,
						"git",
						&[
							"diff",
							"--cached",
							"--no-ext-diff",
							"--no-textconv",
							"--binary",
							"--",
							&scope,
						],
					)?)?
				} else {
					text(run(
						&repository.root,
						"git",
						&[
							"diff",
							"HEAD",
							"--no-ext-diff",
							"--no-textconv",
							"--binary",
							"--",
							&scope,
						],
					)?)?
				};
				if identity.is_empty() {
					diff.push_str(&text(run(
						&repository.root,
						"git",
						&[
							"diff",
							"--no-ext-diff",
							"--no-textconv",
							"--binary",
							"--",
							&scope,
						],
					)?)?);
				}
				let untracked = run(
					&repository.root,
					"git",
					&[
						"ls-files",
						"--others",
						"--exclude-standard",
						"-z",
						"--",
						&scope,
					],
				)?;
				for path in paths(&untracked.stdout)? {
					let output = git_command(&repository.root)
						.args([
							"diff",
							"--no-index",
							"--no-ext-diff",
							"--no-textconv",
							"--binary",
							"--",
							"/dev/null",
						])
						.arg(path)
						.output()?;
					ensure!(
						output.status.success() || output.status.code() == Some(1),
						"cannot review untracked file"
					);
					diff.push_str(&text(output)?);
				}
				(status, diff, identity)
			}
			Backend::Jj => {
				let status = text(run(&self.root, "jj", &["status", "."])?)?;
				let diff = text(run(&self.root, "jj", &["diff", "--git", "."])?)?;
				let identity = text(run(
					&self.root,
					"jj",
					&["log", "--no-graph", "-r", "@", "-T", "commit_id"],
				)?)?;
				(status, diff, identity)
			}
		};
		Ok((status, diff, identity))
	}

	/// Commits only after the client has shown and approved `review`.
	/// Rechecks the review, refuses empty changes and unrelated Git staging.
	pub fn commit(&self, review: &Review) -> Result<CommitOutcome> {
		let _lock = self.lock()?;
		ensure!(
			self.review(Some(&review.message))? == *review,
			"configuration changed since review; review again"
		);
		for entry in &review.journal {
			for path in &entry.changed_files {
				ensure!(
					path.is_absolute() && resolved_path(path)?.starts_with(&self.root),
					"pending operation '{}' changed {} outside configuration root {}; commit those files separately and reconcile the pending journal before committing here",
					entry.operation,
					path.display(),
					self.root.display()
				);
			}
		}
		ensure!(!review.diff.trim().is_empty(), "no changes to commit");
		let repo = &review.repository;
		let revision = match repo.backend {
			Backend::Git => {
				let staged = run(
					&repo.root,
					"git",
					&["diff", "--cached", "--name-only", "--no-renames", "-z"],
				)?;
				for path in paths(&staged.stdout)? {
					ensure!(
						repo.root.join(path).starts_with(&self.root),
						"unrelated Git staged files; unstage them first"
					);
				}
				let scope = git_scope(repo)?;
				run(&repo.root, "git", &["add", "--all", "--", &scope])?;
				run(
					&repo.root,
					"git",
					&[
						"commit",
						"--cleanup=verbatim",
						"--only",
						"-m",
						&review.message,
						"--",
						&scope,
					],
				)?;
				text(run(&repo.root, "git", &["rev-parse", "HEAD"])?)?
					.trim()
					.to_owned()
			}
			Backend::Jj => {
				ensure!(
					repo.root == self.root,
					"JJ commits in parent repositories are refused, including unrelated working-copy files"
				);
				run(&self.root, "jj", &["commit", "-m", &review.message])?;
				text(run(
					&self.root,
					"jj",
					&["log", "--no-graph", "-r", "@-", "-T", "commit_id"],
				)?)?
				.trim()
				.to_owned()
			}
		};
		let warning = self
			.write_journal(&Journal {
				root: self.root.clone(),
				entries: Vec::new(),
			})
			.err()
			.map(|error| format!("Commit succeeded but journal cleanup failed: {error:#}"));
		Ok(CommitOutcome { revision, warning })
	}

	/// Returns only origin, never another remote as a substitute.
	pub fn origin(&self) -> Result<Option<String>> {
		let repo = self.repository()?;
		let remotes = match repo.backend {
			Backend::Git => text(run(&repo.root, "git", &["remote"])?)?,
			Backend::Jj => text(run(&repo.root, "jj", &["git", "remote", "list"])?)?,
		};
		if !remotes
			.lines()
			.any(|line| line.split_whitespace().next() == Some("origin"))
		{
			return Ok(None);
		}
		match repo.backend {
			Backend::Git => Ok(Some(
				text(run(&repo.root, "git", &["remote", "get-url", "origin"])?)?
					.trim()
					.to_owned(),
			)),
			Backend::Jj => Ok(remotes.lines().find_map(|line| {
				line.strip_prefix("origin ")
					.map(|url| url.trim().to_owned())
			})),
		}
	}

	/// Explicitly pushes the named Git branch or JJ bookmark to origin.
	/// `create_nixbox` explicitly creates or fast-forward advances `nixbox` to @-.
	/// Clients must label this opt-in as "Create/advance nixbox bookmark".
	pub fn push(&self, name: &str, create_nixbox: bool) -> Result<()> {
		validate_name(name)?;
		ensure!(self.origin()?.is_some(), "origin is not configured");
		let repo = self.repository()?;
		match repo.backend {
			Backend::Git => {
				run(&repo.root, "git", &["check-ref-format", "--branch", name])?;
				run(
					&repo.root,
					"git",
					&["show-ref", "--verify", &format!("refs/heads/{name}")],
				)?;
				run(
					&repo.root,
					"git",
					&[
						"push",
						"--no-follow-tags",
						"origin",
						&format!("refs/heads/{name}:refs/heads/{name}"),
					],
				)?;
			}
			Backend::Jj => {
				ensure!(
					repo.root == self.root,
					"JJ push in parent repositories is refused"
				);
				let local = format!("bookmarks(exact:\"{name}\")");
				let target = text(run(
					&repo.root,
					"jj",
					&["log", "--no-graph", "-r", &local, "-T", "commit_id"],
				)?)?;
				ensure!(
					target.trim().is_empty()
						|| target.trim().len() == 40
						|| target.trim().len() == 64,
					"bookmark must resolve to exactly one commit"
				);
				let destination = if create_nixbox { "@-" } else { &local };
				if create_nixbox {
					ensure!(
						name == "nixbox",
						"only the nixbox bookmark can be created/advanced automatically"
					);
					let committed = text(run(
						&repo.root,
						"jj",
						&[
							"log",
							"--no-graph",
							"-r",
							"@- ~ root()",
							"-T",
							"if(empty, \"\", description)",
						],
					)?)?;
					ensure!(
						!committed.trim().is_empty(),
						"nixbox bookmark needs a nonempty committed change at @-"
					);
					let diverged = format!("{local} ~ ancestors(@-)");
					ensure!(
						text(run(
							&repo.root,
							"jj",
							&["log", "--no-graph", "-r", &diverged, "-T", "commit_id"]
						)?)?
						.trim()
						.is_empty(),
						"nixbox bookmark diverged from @-; reconcile it before advancing"
					);
				} else {
					ensure!(
						!target.trim().is_empty(),
						"bookmark must resolve to exactly one commit"
					);
				}
				let non_forward = format!(
					"remote_bookmarks(exact:\"{name}\", remote=exact:\"origin\") ~ ancestors({destination})"
				);
				ensure!(
					text(run(
						&repo.root,
						"jj",
						&["log", "--no-graph", "-r", &non_forward, "-T", "commit_id"]
					)?)?
					.trim()
					.is_empty(),
					"non-fast-forward JJ push refused; fetch and reconcile first"
				);
				if create_nixbox {
					let action = if target.trim().is_empty() {
						"create"
					} else {
						"set"
					};
					run(&repo.root, "jj", &["bookmark", action, name, "-r", "@-"])?;
				}
				run(
					&repo.root,
					"jj",
					&[
						"git",
						"push",
						"--remote",
						"origin",
						"--bookmark",
						&format!("exact:{name}"),
					],
				)?;
			}
		}
		Ok(())
	}

	/// Creates an explicitly named GitHub repo and adds origin, without pushing.
	/// Requires a config-root repo and no existing origin. Name is NAME or OWNER/NAME.
	pub fn create_remote(&self, name: &str, visibility: Visibility) -> Result<()> {
		ensure!(
			!name.is_empty()
				&& name.split('/').count() <= 2
				&& name.split('/').all(|part| !part.is_empty()
					&& part
						.bytes()
						.all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
					&& !part.starts_with('-')),
			"expected explicit repository NAME or OWNER/NAME"
		);
		let repo = self.repository()?;
		ensure!(
			repo.root == self.root,
			"remote creation in parent repositories is refused"
		);
		ensure!(self.origin()?.is_none(), "origin already exists");
		let flag = match visibility {
			Visibility::Private => "--private",
			Visibility::Public => "--public",
		};
		// gh creates only the remote; adding origin separately also works with JJ.
		run(&self.root, "gh", &["repo", "create", name, flag])?;
		let url = text(run(
			&self.root,
			"gh",
			&["repo", "view", name, "--json", "url", "--jq", ".url"],
		)?)?;
		match repo.backend {
			Backend::Git => {
				run(&self.root, "git", &["remote", "add", "origin", url.trim()])?;
			}
			Backend::Jj => {
				run(
					&self.root,
					"jj",
					&["git", "remote", "add", "origin", url.trim()],
				)?;
			}
		}
		Ok(())
	}

	pub fn repository(&self) -> Result<Repository> {
		self.detect()?
			.context("no repository; initialize one explicitly")
	}

	pub fn status(&self) -> Result<String> {
		Ok(self.inspect(&self.repository()?)?.0)
	}
	pub fn diff(&self) -> Result<String> {
		Ok(self.inspect(&self.repository()?)?.1)
	}

	fn lock(&self) -> Result<JournalLock> {
		let directory = self.journal_path.parent().context("journal directory")?;
		fs::create_dir_all(directory)?;
		let path = self.journal_path.with_extension("lock");
		OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&path)
			.context("VCS journal is busy, or has a stale lock")?;
		Ok(JournalLock(path))
	}

	fn read_journal(&self) -> Result<Journal> {
		match fs::read(&self.journal_path) {
			Ok(bytes) => {
				let journal: Journal = serde_json::from_slice(&bytes)?;
				ensure!(journal.root == self.root, "journal scope mismatch");
				Ok(journal)
			}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Journal {
				root: self.root.clone(),
				entries: Vec::new(),
			}),
			Err(error) => Err(error.into()),
		}
	}

	fn write_journal(&self, journal: &Journal) -> Result<()> {
		let temporary = self.journal_path.with_extension("tmp");
		let mut file = OpenOptions::new()
			.create(true)
			.truncate(true)
			.write(true)
			.open(&temporary)?;
		file.write_all(&serde_json::to_vec(journal)?)?;
		file.sync_all()?;
		fs::rename(temporary, &self.journal_path)?;
		fs::File::open(self.journal_path.parent().context("journal directory")?)?.sync_all()?;
		Ok(())
	}
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result<Output> {
	let mut command = if program == "git" {
		git_command(root)
	} else {
		let mut command = Command::new(program);
		command
			.current_dir(root)
			.env("NO_COLOR", "1")
			.env("GIT_TERMINAL_PROMPT", "0");
		if program == "jj" {
			command.args(["--no-pager", "--color", "never"]);
		}
		command
	};
	let output = command
		.args(args)
		.output()
		.with_context(|| format!("running {program}"))?;
	if !output.status.success() {
		bail!(
			"{program} failed: {}",
			String::from_utf8_lossy(&output.stderr).trim()
		);
	}
	Ok(output)
}

fn git_command(root: &Path) -> Command {
	let mut command = Command::new("git");
	command
		.current_dir(root)
		.args(["--no-pager", "-c", "color.ui=false"])
		.env("GIT_LITERAL_PATHSPECS", "0")
		.env("GIT_GLOB_PATHSPECS", "0")
		.env("GIT_NOGLOB_PATHSPECS", "0")
		.env("GIT_ICASE_PATHSPECS", "0")
		.env("GIT_TERMINAL_PROMPT", "0");
	command
}

fn text(output: Output) -> Result<String> {
	Ok(String::from_utf8(output.stdout)?)
}

fn paths(bytes: &[u8]) -> Result<Vec<PathBuf>> {
	bytes
		.split(|byte| *byte == 0)
		.filter(|path| !path.is_empty())
		.map(|path| {
			// Reject non-UTF8 paths rather than silently scope them incorrectly.
			Ok(PathBuf::from(std::str::from_utf8(path)?))
		})
		.collect()
}

fn git_scope(repo: &Repository) -> Result<String> {
	let relative = repo.config_root.strip_prefix(&repo.root)?;
	if relative.as_os_str().is_empty() {
		return Ok(":(top)".into());
	}
	Ok(format!(
		":(top,literal){}",
		relative.to_str().context("non-UTF8 config root")?
	))
}

fn validate_name(name: &str) -> Result<()> {
	ensure!(
		!name.is_empty()
			&& !name.starts_with('-')
			&& name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte)),
		"invalid branch/bookmark name"
	);
	Ok(())
}

pub(crate) fn operation_files(config: &Config) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
	let mut files = vec![config.flake_file(), config.config_root().join("flake.lock")];
	for scope in [Target::HomeManager, Target::NixosSystem] {
		files.extend([
			config.main_file_for(scope),
			config.managed_file_for(scope),
			config.flake_manifest_for(scope),
			config.settings_file_for(scope),
		]);
	}
	files
		.into_iter()
		.map(|path| {
			let bytes = fs::read(&path).ok();
			(path, bytes)
		})
		.collect()
}

/// Internal guard held from before configuration writes through journal recording.
pub(crate) struct AppliedJournal {
	vcs: Vcs,
	_lock: JournalLock,
}

impl AppliedJournal {
	pub(crate) fn acquire(config: &Config) -> Result<Self> {
		let vcs = Vcs::new(config)?;
		let lock = vcs.lock()?;
		Ok(Self { vcs, _lock: lock })
	}

	pub(crate) fn record(
		&self,
		config: &Config,
		op: &Op,
		before: BTreeMap<PathBuf, Option<Vec<u8>>>,
	) -> Result<()> {
		let changed_files: Vec<_> = operation_files(config)
			.into_iter()
			.filter_map(|(path, bytes)| {
				if before.get(&path) == Some(&bytes) {
					None
				} else {
					Some(path)
				}
			})
			.map(|path| resolved_path(&path))
			.collect::<Result<_>>()?;
		if changed_files.is_empty() {
			return Ok(());
		}
		let mut journal = self.vcs.read_journal()?;
		journal.entries.push(JournalEntry {
			operation: operation_description(op),
			changed_files,
		});
		self.vcs.write_journal(&journal)
	}
}

// Resolve existing ancestors too, so deleted files and symlinked directories
// cannot conceal journal paths outside the configuration root.
fn resolved_path(path: &Path) -> Result<PathBuf> {
	match fs::canonicalize(path) {
		Ok(path) => Ok(path),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			let parent = path.parent().context("journal path has no parent")?;
			let mut resolved = resolved_path(parent)?;
			match path.components().next_back() {
				Some(std::path::Component::ParentDir) => {
					resolved.pop();
				}
				Some(std::path::Component::Normal(name)) => resolved.push(name),
				_ => bail!("cannot resolve path {}", path.display()),
			}
			match fs::canonicalize(&resolved) {
				Ok(path) => Ok(path),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(resolved),
				Err(error) => Err(error.into()),
			}
		}
		Err(error) => Err(error.into()),
	}
}

fn operation_description(op: &Op) -> String {
	let detail = match op {
		Op::Install { hit, .. } => format!("install {} version {}", hit.attr, hit.version),
		Op::Uninstall { name, .. } => format!("uninstall {name}"),
		Op::InstallFlake { repo, module, .. } => format!("install flake {repo} module {module}"),
		Op::InstallFlakePackage { repo, package, .. } => {
			format!("install flake {repo} package {package}")
		}
		Op::UninstallFlake { repo, .. } => format!("uninstall flake {repo}"),
		Op::UninstallFlakeOutput { input, output, .. } => {
			format!("uninstall flake input {input} output {output:?}")
		}
		Op::Migrate { names, .. } => {
			let mut names = names.clone();
			names.sort();
			format!("migrate packages {}", names.join(", "))
		}
		Op::MigrateFlakePackage { input, package, .. } => {
			format!("migrate flake input {input} package {package}")
		}
		Op::SetOptions { changes, .. } => {
			let mut changes: Vec<_> = changes
				.iter()
				.map(|change| {
					format!(
						"{} {}",
						if change.value.is_some() {
							"set"
						} else {
							"unset"
						},
						nixbox_nix::settings::option_path_display(&change.path)
					)
				})
				.collect();
			changes.sort();
			format!("options {}", changes.join(", "))
		}
	};
	format!(
		"{} [{}]",
		detail.replace(['\n', '\r'], " "),
		op.scope().label()
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Engine, OptionChange, SilentReporter};
	use nixbox_nix::{Manifest, settings::SettingValue};

	struct TempRepo {
		vcs: Vcs,
		_dir: crate::tests::TempConfigDir,
	}

	fn repo(label: &str) -> TempRepo {
		let dir = crate::tests::temp_dir(label);
		let root = dir.config().config_root().join("repo");
		fs::create_dir(&root).unwrap();
		let mut vcs = Vcs::for_root(&root).unwrap();
		vcs.journal_path = dir.config().config_root().join("journal.json");
		TempRepo { vcs, _dir: dir }
	}

	fn integration_ready(root: &Path, backend: Backend) -> bool {
		let programs: &[&str] = if backend == Backend::Jj {
			&["git", "jj"]
		} else {
			&["git"]
		};
		for program in programs {
			if run(root, program, &["--version"]).is_err() {
				eprintln!("SKIP VCS integration test: {program} is unavailable");
				return false;
			}
		}
		let identity: &[&[&str]] = if backend == Backend::Jj {
			&[
				&["config", "get", "user.name"],
				&["config", "get", "user.email"],
			]
		} else {
			&[
				&["var", "GIT_AUTHOR_IDENT"],
				&["var", "GIT_COMMITTER_IDENT"],
			]
		};
		let program = if backend == Backend::Jj { "jj" } else { "git" };
		for args in identity {
			if !run(root, program, args)
				.and_then(text)
				.is_ok_and(|value| !value.trim().is_empty())
			{
				eprintln!(
					"SKIP VCS integration test: {program} user identity is not configured; configure your own name/email to run this test"
				);
				return false;
			}
		}
		true
	}

	fn bare_remote(temp: &TempRepo) -> PathBuf {
		let remote = temp.vcs.root.parent().unwrap().join("remote.git");
		fs::create_dir(&remote).unwrap();
		run(&remote, "git", &["init", "--bare"]).unwrap();
		remote
	}

	#[test]
	fn git_push_is_explicit_and_origin_creation_is_guarded() {
		let temp = repo("vcs-git-push");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Git) {
			return;
		}
		vcs.init(Backend::Git).unwrap();
		assert_eq!(vcs.origin().unwrap(), None);
		assert!(vcs.push("nixbox", false).is_err());
		assert!(vcs.create_remote("", Visibility::Private).is_err());
		assert!(vcs.create_remote("--bad", Visibility::Public).is_err());
		assert!(
			vcs.create_remote("owner/repo/extra", Visibility::Public)
				.is_err()
		);
		let remote = bare_remote(&temp);
		run(
			&vcs.root,
			"git",
			&["remote", "add", "origin", remote.to_str().unwrap()],
		)
		.unwrap();
		assert_eq!(vcs.origin().unwrap().as_deref(), remote.to_str());
		assert!(
			vcs.create_remote("explicit-name", Visibility::Private)
				.unwrap_err()
				.to_string()
				.contains("origin already exists")
		);
		fs::write(vcs.root.join("home.nix"), "first\n").unwrap();
		let committed = vcs.commit(&vcs.review(None).unwrap()).unwrap();
		run(&vcs.root, "git", &["config", "push.followTags", "true"]).unwrap();
		run(
			&vcs.root,
			"git",
			&["tag", "-a", "unrelated-tag", "-m", "tag stays local"],
		)
		.unwrap();
		run(&vcs.root, "git", &["branch", "nixbox", &committed.revision]).unwrap();
		assert!(
			run(
				&remote,
				"git",
				&["show-ref", "--verify", "refs/heads/nixbox"]
			)
			.is_err()
		);
		assert!(vcs.push("missing", false).is_err());
		assert!(vcs.push("--all", false).is_err());
		vcs.push("nixbox", false).unwrap();
		assert!(
			run(
				&remote,
				"git",
				&["show-ref", "--verify", "refs/tags/unrelated-tag"]
			)
			.is_err()
		);
		assert_eq!(
			text(run(&remote, "git", &["rev-parse", "refs/heads/nixbox"]).unwrap())
				.unwrap()
				.trim(),
			committed.revision
		);
		run(
			&vcs.root,
			"git",
			&["symbolic-ref", "HEAD", "refs/heads/nixbox"],
		)
		.unwrap();
		fs::write(vcs.root.join("home.nix"), "second\n").unwrap();
		let second = vcs.commit(&vcs.review(None).unwrap()).unwrap();
		vcs.push("nixbox", false).unwrap();
		// Simulate a local branch behind origin, without touching the worktree.
		run(
			&vcs.root,
			"git",
			&["update-ref", "refs/heads/nixbox", &committed.revision],
		)
		.unwrap();
		assert!(vcs.push("nixbox", false).is_err());
		assert_eq!(
			text(run(&remote, "git", &["rev-parse", "refs/heads/nixbox"]).unwrap())
				.unwrap()
				.trim(),
			second.revision
		);
	}

	#[test]
	fn jj_push_creates_only_explicit_bookmark_and_refuses_non_forward() {
		let temp = repo("vcs-jj-push");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Jj) {
			return;
		}
		vcs.init(Backend::Jj).unwrap();
		let remote = bare_remote(&temp);
		run(
			&vcs.root,
			"jj",
			&["git", "remote", "add", "origin", remote.to_str().unwrap()],
		)
		.unwrap();
		assert_eq!(vcs.origin().unwrap().as_deref(), remote.to_str());
		assert!(vcs.push("nixbox", true).is_err());
		fs::write(vcs.root.join("home.nix"), "first\n").unwrap();
		let first = vcs.commit(&vcs.review(None).unwrap()).unwrap();
		assert!(
			text(run(&vcs.root, "jj", &["bookmark", "list"]).unwrap())
				.unwrap()
				.is_empty()
		);
		assert!(vcs.push("other", true).is_err());
		vcs.push("nixbox", true).unwrap();
		assert_eq!(
			text(run(&remote, "git", &["rev-parse", "refs/heads/nixbox"]).unwrap())
				.unwrap()
				.trim(),
			first.revision
		);
		fs::write(vcs.root.join("home.nix"), "second\n").unwrap();
		let second = vcs.commit(&vcs.review(None).unwrap()).unwrap();
		// Without the opt-in, pushing keeps the existing bookmark in place.
		vcs.push("nixbox", false).unwrap();
		assert_eq!(
			text(run(&remote, "git", &["rev-parse", "refs/heads/nixbox"]).unwrap())
				.unwrap()
				.trim(),
			first.revision
		);
		// The explicit create/advance opt-in supports repeated commits and pushes.
		vcs.push("nixbox", true).unwrap();
		assert_eq!(
			text(run(&remote, "git", &["rev-parse", "refs/heads/nixbox"]).unwrap())
				.unwrap()
				.trim(),
			second.revision
		);
		run(&vcs.root, "jj", &["bookmark", "delete", "nixbox"]).unwrap();
		run(
			&vcs.root,
			"jj",
			&["bookmark", "set", "nixbox", "-r", &first.revision],
		)
		.unwrap();
		assert!(
			vcs.push("nixbox", false)
				.unwrap_err()
				.to_string()
				.contains("non-fast-forward")
		);
		assert_eq!(
			text(run(&remote, "git", &["rev-parse", "refs/heads/nixbox"]).unwrap())
				.unwrap()
				.trim(),
			second.revision
		);
		// A divergent committed change must not move the local bookmark either.
		run(&vcs.root, "jj", &["new", &first.revision]).unwrap();
		fs::write(vcs.root.join("home.nix"), "divergent\n").unwrap();
		vcs.commit(&vcs.review(Some("divergent configuration")).unwrap())
			.unwrap();
		assert!(
			vcs.push("nixbox", true)
				.unwrap_err()
				.to_string()
				.contains("non-fast-forward")
		);
		assert_eq!(
			text(
				run(
					&vcs.root,
					"jj",
					&["log", "--no-graph", "-r", "nixbox", "-T", "commit_id"]
				)
				.unwrap()
			)
			.unwrap()
			.trim(),
			first.revision
		);
		run(
			&vcs.root,
			"jj",
			&["bookmark", "set", "nixbox", "-r", &second.revision],
		)
		.unwrap();
		assert!(
			vcs.push("nixbox", true)
				.unwrap_err()
				.to_string()
				.contains("diverged")
		);
		assert_eq!(
			text(
				run(
					&vcs.root,
					"jj",
					&["log", "--no-graph", "-r", "nixbox", "-T", "commit_id"]
				)
				.unwrap()
			)
			.unwrap()
			.trim(),
			second.revision
		);
	}

	#[test]
	fn redacts_values_and_sorts_every_option_and_migration() {
		let op = Op::SetOptions {
			changes: vec![
				OptionChange {
					path: vec!["z".into()],
					value: Some(SettingValue::Str("secret-token".into())),
				},
				OptionChange {
					path: vec!["a".into()],
					value: None,
				},
			],
			scope: Target::HomeManager,
		};
		let description = operation_description(&op);
		assert_eq!(description, "options set z, unset a [home-manager]");
		assert!(!description.contains("secret-token"));
		assert_eq!(
			operation_description(&Op::Migrate {
				names: vec!["z".into(), "a".into()],
				scope: Target::NixosSystem
			}),
			"migrate packages a, z [nixos]"
		);
	}

	#[test]
	fn git_reviews_manual_files_commits_override_and_rejects_stale_or_empty() {
		let temp = repo("vcs-git");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Git) {
			return;
		}
		assert!(vcs.detect().unwrap().is_none());
		vcs.init(Backend::Git).unwrap();
		fs::write(vcs.root.join("manual.nix"), "first\n").unwrap();
		let review = vcs.review(Some("manual config update")).unwrap();
		assert!(review.diff().contains("+first"));
		assert_eq!(review.message(), "manual config update");
		fs::write(vcs.root.join("manual.nix"), "second\n").unwrap();
		assert!(
			vcs.commit(&review)
				.unwrap_err()
				.to_string()
				.contains("review again")
		);
		let verbatim = "manual config update\n\n# keep this line\ntrailing spaces  \n\n";
		run(&vcs.root, "git", &["config", "commit.cleanup", "strip"]).unwrap();
		let result = vcs.commit(&vcs.review(Some(verbatim)).unwrap()).unwrap();
		assert!(result.warning.is_none());
		assert_eq!(
			text(run(&vcs.root, "git", &["log", "-1", "--format=%B"]).unwrap()).unwrap(),
			format!("{verbatim}\n")
		);
		fs::remove_file(vcs.root.join("manual.nix")).unwrap();
		fs::write(vcs.root.join("added.nix"), "new manual config\n").unwrap();
		let manual_review = vcs.review(None).unwrap();
		assert!(manual_review.diff().contains("-second"));
		assert!(manual_review.diff().contains("+new manual config"));
		vcs.commit(&manual_review).unwrap();
		assert_eq!(
			text(run(&vcs.root, "git", &["ls-tree", "-r", "--name-only", "HEAD"]).unwrap())
				.unwrap()
				.trim(),
			"added.nix"
		);
		assert!(
			vcs.commit(&vcs.review(None).unwrap())
				.unwrap_err()
				.to_string()
				.contains("no changes")
		);
		assert!(vcs.review(Some(" \n")).is_err());
		assert!(vcs.init(Backend::Jj).is_err());
	}

	#[test]
	fn git_parent_scope_refuses_unrelated_staging_and_keeps_manual_outside_edits() {
		let temp = repo("vcs-parent-git");
		let parent = &temp.vcs.root;
		if !integration_ready(parent, Backend::Git) {
			return;
		}
		run(parent, "git", &["init"]).unwrap();
		let config = parent.join("config[1]");
		fs::create_dir(&config).unwrap();
		let mut vcs = Vcs::for_root(&config).unwrap();
		vcs.journal_path = temp.vcs.journal_path.clone();
		fs::write(config.join("home.nix"), "config\n").unwrap();
		fs::write(parent.join("outside"), "outside\n").unwrap();
		assert_eq!(vcs.detect().unwrap().unwrap().root, *parent);
		let review = vcs.review(None).unwrap();
		assert!(!review.diff().contains("+outside"));
		run(parent, "git", &["add", "outside"]).unwrap();
		assert!(
			vcs.commit(&review)
				.unwrap_err()
				.to_string()
				.contains("unrelated Git staged")
		);
		run(parent, "git", &["rm", "--cached", "outside"]).unwrap();
		vcs.commit(&vcs.review(None).unwrap()).unwrap();
		assert_eq!(
			text(run(parent, "git", &["ls-tree", "-r", "--name-only", "HEAD"]).unwrap())
				.unwrap()
				.trim(),
			"config[1]/home.nix"
		);
		assert!(vcs.status().unwrap().is_empty());
		assert!(
			text(run(parent, "git", &["status", "--short"]).unwrap())
				.unwrap()
				.contains("outside")
		);
	}

	#[test]
	fn jj_prefers_colocated_repository_commits_and_refuses_parent_scope() {
		let temp = repo("vcs-jj");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Jj) {
			return;
		}
		vcs.init(Backend::Jj).unwrap();
		assert_eq!(vcs.detect().unwrap().unwrap().backend, Backend::Jj);
		fs::write(vcs.root.join("manual.nix"), "manual\n").unwrap();
		let review = vcs.review(Some("record manual configuration")).unwrap();
		assert!(review.diff().contains("+manual"));
		vcs.commit(&review).unwrap();
		assert!(vcs.commit(&vcs.review(None).unwrap()).is_err());
		assert!(vcs.diff().unwrap().is_empty());
		let child = vcs.root.join("config");
		fs::create_dir(&child).unwrap();
		fs::write(child.join("home.nix"), "child\n").unwrap();
		fs::write(vcs.root.join("unrelated.nix"), "unrelated\n").unwrap();
		let mut child_vcs = Vcs::for_root(&child).unwrap();
		child_vcs.journal_path = vcs.journal_path.clone();
		// The journal is scoped to its original root, so use a separate file.
		child_vcs.journal_path.set_file_name("child-journal.json");
		let review = child_vcs.review(None).unwrap();
		assert!(review.diff().contains("+child"));
		assert!(!review.diff().contains("+unrelated"));
		assert!(
			child_vcs
				.commit(&review)
				.unwrap_err()
				.to_string()
				.contains("parent repositories")
		);
	}

	#[test]
	fn detection_stops_at_nested_git_and_jj_boundaries() {
		let temp = repo("vcs-nested-detect");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Jj) {
			return;
		}
		vcs.init(Backend::Jj).unwrap();
		let git_root = vcs.root.join("nested-git");
		fs::create_dir(&git_root).unwrap();
		run(&git_root, "git", &["init"]).unwrap();
		let child = git_root.join("config");
		fs::create_dir(&child).unwrap();
		let detected = Vcs::for_root(&child).unwrap().repository().unwrap();
		assert_eq!(detected.backend, Backend::Git);
		assert_eq!(detected.root, git_root);
		let jj_root = child.join("nested-jj");
		fs::create_dir(&jj_root).unwrap();
		run(&jj_root, "jj", &["git", "init", "--colocate"]).unwrap();
		let detected = Vcs::for_root(&jj_root).unwrap().repository().unwrap();
		assert_eq!(detected.backend, Backend::Jj);
		assert_eq!(detected.root, jj_root);
	}

	#[test]
	fn commit_preserves_journal_when_operation_touched_external_paths() {
		let temp = repo("vcs-external-journal");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Git) {
			return;
		}
		vcs.init(Backend::Git).unwrap();
		fs::write(vcs.root.join("home.nix"), "config\n").unwrap();
		let outside = vcs.root.parent().unwrap().join("external.nix");
		fs::write(&outside, "external\n").unwrap();
		let entries = vec![JournalEntry {
			operation: "install fd [home-manager]".into(),
			changed_files: vec![vcs.root.join("home.nix"), outside.clone()],
		}];
		{
			let _lock = vcs.lock().unwrap();
			vcs.write_journal(&Journal {
				root: vcs.root.clone(),
				entries: entries.clone(),
			})
			.unwrap();
		}
		let review = vcs.review(None).unwrap();
		let error = vcs.commit(&review).unwrap_err().to_string();
		assert!(error.contains("outside configuration root"));
		assert!(error.contains(outside.to_str().unwrap()));
		assert_eq!(vcs.journal().unwrap(), entries);
		assert!(run(&vcs.root, "git", &["rev-parse", "--verify", "HEAD"]).is_err());
		assert!(
			text(run(&vcs.root, "git", &["diff", "--cached", "--name-only"]).unwrap())
				.unwrap()
				.is_empty()
		);
		// Deleted external files still must not be silently dropped.
		fs::remove_file(outside).unwrap();
		assert!(
			vcs.commit(&review)
				.unwrap_err()
				.to_string()
				.contains("outside configuration root")
		);
		assert_eq!(vcs.journal().unwrap(), entries);
	}

	#[test]
	fn status_and_diff_do_not_depend_on_journal_readability() {
		let temp = repo("vcs-inspection");
		let vcs = &temp.vcs;
		if !integration_ready(&vcs.root, Backend::Git) {
			return;
		}
		vcs.init(Backend::Git).unwrap();
		fs::write(vcs.root.join("home.nix"), "config\n").unwrap();
		fs::write(&vcs.journal_path, "invalid journal").unwrap();
		assert!(vcs.status().unwrap().contains("home.nix"));
		assert!(vcs.diff().unwrap().contains("+config"));
		assert!(vcs.review(None).is_err());
	}

	#[test]
	fn engine_first_install_creates_missing_root_and_records_journal() {
		let dir = crate::tests::temp_dir("vcs-first-install");
		let config = dir.config();
		let root = config.config_root();
		fs::remove_dir(&root).unwrap();
		let mut engine =
			Engine::from_parts(config, Manifest::default(), Manifest::default(), Vec::new());
		engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit("fd"),
					scope: Target::HomeManager,
				},
				&mut SilentReporter,
			)
			.unwrap();
		assert!(engine.is_tracked("fd", Target::HomeManager));
		assert!(
			engine
				.config
				.managed_file_for(Target::HomeManager)
				.is_file()
		);
		let vcs = Vcs::new(&engine.config).unwrap();
		let entries = vcs.journal().unwrap();
		assert_eq!(entries.len(), 1);
		assert_eq!(
			entries[0].operation,
			"install fd version 1.0 [home-manager]"
		);
		assert!(!entries[0].changed_files.is_empty());
		assert!(
			entries[0]
				.changed_files
				.iter()
				.all(|path| path.starts_with(&root))
		);
		fs::remove_file(vcs.journal_path).unwrap();
	}

	#[test]
	fn missing_root_is_created_only_by_explicit_init() {
		let dir = crate::tests::temp_dir("vcs-missing-init");
		let parent = dir.config().config_root();
		let root = parent.join("missing/config");
		let vcs = Vcs::for_root(&root).unwrap();
		assert!(vcs.detect().unwrap().is_none());
		assert!(vcs.journal().unwrap().is_empty());
		assert!(vcs.status().is_err());
		assert!(vcs.diff().is_err());
		assert!(!parent.join("missing").exists());
		let repository = vcs.init(Backend::Git).unwrap();
		assert_eq!(repository.root, fs::canonicalize(&root).unwrap());
		assert_eq!(repository.config_root, repository.root);
	}

	#[test]
	fn missing_root_detects_parent_repository_and_init_does_not_create_it() {
		let temp = repo("vcs-missing-parent");
		temp.vcs.init(Backend::Git).unwrap();
		let root = temp.vcs.root.join("missing/config");
		let vcs = Vcs::for_root(&root).unwrap();
		let repository = vcs.repository().unwrap();
		assert_eq!(repository.root, temp.vcs.root);
		assert_eq!(repository.config_root, root);
		assert!(vcs.status().unwrap().is_empty());
		assert!(vcs.init(Backend::Git).is_err());
		assert!(!temp.vcs.root.join("missing").exists());
	}

	#[test]
	#[cfg(unix)]
	fn prospective_root_resolves_symlinks_and_missing_parent_components() {
		let dir = crate::tests::temp_dir("vcs-prospective-path");
		let parent = dir.config().config_root();
		let target = parent.join("target/nested");
		fs::create_dir_all(&target).unwrap();
		std::os::unix::fs::symlink(&target, parent.join("link")).unwrap();
		let vcs = Vcs::for_root(parent.join("link/../missing/../config")).unwrap();
		assert_eq!(
			vcs.root,
			fs::canonicalize(parent.join("target"))
				.unwrap()
				.join("config")
		);
		assert!(!vcs.root.exists());
		assert!(!parent.join("target/missing").exists());
		let vcs = Vcs::for_root(parent.join("missing/../link/../config")).unwrap();
		assert_eq!(
			vcs.root,
			fs::canonicalize(parent.join("target"))
				.unwrap()
				.join("config")
		);
		assert!(!parent.join("missing").exists());
		let relative = Path::new("nixbox-prospective-missing/../nixbox-prospective-config");
		let vcs = Vcs::for_root(relative).unwrap();
		assert_eq!(
			vcs.root,
			fs::canonicalize(std::env::current_dir().unwrap())
				.unwrap()
				.join("nixbox-prospective-config")
		);
		assert!(!vcs.root.exists());
	}

	#[test]
	fn engine_journals_early_return_success_and_not_failure() {
		let dir = crate::tests::temp_dir("vcs-apply");
		let config = dir.config();
		let vcs = Vcs::new(&config).unwrap();
		let mut engine =
			Engine::from_parts(config, Manifest::default(), Manifest::default(), Vec::new());
		let op = Op::SetOptions {
			changes: vec![OptionChange {
				path: vec!["programs".into(), "git".into(), "enable".into()],
				value: Some(SettingValue::Bool(true)),
			}],
			scope: Target::HomeManager,
		};
		engine.apply(&op, &mut SilentReporter).unwrap();
		let entries = vcs.journal().unwrap();
		assert_eq!(entries.len(), 1);
		assert_eq!(
			entries[0].operation,
			"options set programs.git.enable [home-manager]"
		);
		assert!(!entries[0].changed_files.is_empty());
		engine.apply(&op, &mut SilentReporter).unwrap();
		assert_eq!(
			vcs.journal().unwrap(),
			entries,
			"no-op apply must not add a journal entry"
		);
		// Reloading a backend retains the applied operation.
		assert!(
			Vcs::new(&engine.config)
				.unwrap()
				.suggested_message()
				.unwrap()
				.contains("programs.git.enable")
		);
		fs::remove_file(engine.config.settings_file_for(Target::HomeManager)).unwrap();
		fs::create_dir(engine.config.settings_file_for(Target::HomeManager)).unwrap();
		assert!(engine.apply(&op, &mut SilentReporter).is_err());
		assert_eq!(vcs.journal().unwrap(), entries);
		fs::remove_file(vcs.journal_path).unwrap();
	}

	#[test]
	fn busy_journal_refuses_apply_before_any_writes_or_manifest_mutation() {
		let dir = crate::tests::temp_dir("vcs-journal-failure");
		let config = dir.config();
		let vcs = Vcs::new(&config).unwrap();
		let lock = vcs.lock().unwrap();
		let mut engine =
			Engine::from_parts(config, Manifest::default(), Manifest::default(), Vec::new());
		let before = operation_files(&engine.config);
		let mut reporter = crate::LogReporter::new();
		let error = engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit("fd"),
					scope: Target::HomeManager,
				},
				&mut reporter,
			)
			.unwrap_err();
		assert!(error.to_string().contains("busy"));
		assert_eq!(operation_files(&engine.config), before);
		assert!(!engine.is_tracked("fd", Target::HomeManager));
		assert!(vcs.journal().unwrap().is_empty());
		drop(lock);
	}

	#[test]
	#[cfg(unix)]
	fn recording_keeps_guard_and_remembers_external_symlink_target() {
		let dir = crate::tests::temp_dir("vcs-guard-symlink");
		let config = dir.config();
		let vcs = Vcs::new(&config).unwrap();
		let outside = dir.config().config_root().with_extension("external.nix");
		fs::write(&outside, "before\n").unwrap();
		let linked = config.settings_file_for(Target::HomeManager);
		std::os::unix::fs::symlink(&outside, &linked).unwrap();
		let journal = AppliedJournal::acquire(&config).unwrap();
		let before = operation_files(&config);
		fs::write(&linked, "after\n").unwrap();
		journal
			.record(
				&config,
				&Op::SetOptions {
					changes: Vec::new(),
					scope: Target::HomeManager,
				},
				before,
			)
			.unwrap();
		assert!(
			vcs.lock().is_err(),
			"recording must retain the original guard"
		);
		fs::remove_file(linked).unwrap();
		assert_eq!(
			vcs.journal().unwrap()[0].changed_files,
			vec![fs::canonicalize(&outside).unwrap()]
		);
		drop(journal);
		let _lock = vcs.lock().unwrap();
		fs::remove_file(&vcs.journal_path).unwrap();
		fs::remove_file(outside).unwrap();
	}

	#[test]
	fn journal_write_failure_warns_after_apply_and_releases_guard() {
		let dir = crate::tests::temp_dir("vcs-write-failure");
		let config = dir.config();
		let vcs = Vcs::new(&config).unwrap();
		fs::create_dir_all(vcs.journal_path.parent().unwrap()).unwrap();
		let temporary = vcs.journal_path.with_extension("tmp");
		fs::create_dir(&temporary).unwrap();
		let mut engine =
			Engine::from_parts(config, Manifest::default(), Manifest::default(), Vec::new());
		let mut reporter = crate::LogReporter::new();
		engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit("fd"),
					scope: Target::HomeManager,
				},
				&mut reporter,
			)
			.unwrap();
		assert!(engine.config.managed_file_for(Target::HomeManager).exists());
		assert!(
			reporter
				.lines()
				.iter()
				.any(|line| line.contains("journal could not be saved"))
		);
		let _lock = vcs.lock().unwrap();
		fs::remove_dir(temporary).unwrap();
	}

	#[test]
	fn messages_are_sorted_and_journals_are_scope_checked() {
		let temp = repo("vcs-message");
		let vcs = &temp.vcs;
		let _lock = vcs.lock().unwrap();
		vcs.write_journal(&Journal {
			root: vcs.root.clone(),
			entries: vec![
				JournalEntry {
					operation: "uninstall z [nixos]".into(),
					changed_files: Vec::new(),
				},
				JournalEntry {
					operation: "install a [home-manager]".into(),
					changed_files: Vec::new(),
				},
			],
		})
		.unwrap();
		assert_eq!(
			vcs.suggested_message().unwrap(),
			"nixbox: update configuration\n\n- install a [home-manager]\n- uninstall z [nixos]"
		);
		vcs.write_journal(&Journal {
			root: vcs.root.join("wrong"),
			entries: Vec::new(),
		})
		.unwrap();
		assert!(vcs.journal().is_err());
	}
}
