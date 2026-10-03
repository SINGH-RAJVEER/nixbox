//! The operation pipeline an interactive front-end drives.
//!
//! A [`Session`] owns the engine, the queue of pending [`Op`]s, the rebuild
//! that is running, and the build log. Front-ends queue work with
//! [`Session::enqueue`], feed every [`BuildEvent`] from the receiver
//! [`Session::new`] hands back into [`Session::on_build_event`], and render
//! whatever state they like from the public fields. Everything that has to
//! survive a crash is written to `state.json` as it changes.
//!
//! Ops of one scope are written and rebuilt together; other scopes wait in
//! the queue, in order, until the rebuild in front of them ends.

use std::collections::VecDeque;
use std::path::PathBuf;

use nixbox_config::Target;
use nixbox_nix::build::BuildEvent;
use nixbox_nix::scan::ScanTarget;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::engine::Engine;
use crate::op::Op;
use crate::rebuild::{self, Escalation};
use crate::report::LogReporter;
use crate::state::{InProgress, PersistedState};

/// Lines kept in the build log. Older lines are dropped first.
pub const LOG_LIMIT: usize = 1000;

/// The rebuild currently running.
#[derive(Debug)]
struct ActiveBuild {
	scope: Target,
	label: String,
	cancel: Option<oneshot::Sender<()>>,
}

/// What [`Session::enqueue`] did with an op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enqueued {
	/// The same work is already waiting; nothing changed.
	Duplicate,
	/// Added behind the rebuild that is running.
	Queued,
	/// Added and started straight away, as a rebuild with this label.
	Started(String),
	/// Added, but no manifest could be written for it; see the log.
	NotWritten,
}

/// How the rebuild that just ended went, returned from
/// [`Session::on_build_event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildEnded {
	Succeeded {
		label: String,
		/// The label of the next queued rebuild, if one started.
		next: Option<String>,
	},
	Failed {
		label: String,
		error: String,
		next: Option<String>,
	},
	/// The user stopped it. Queued work is left paused, not started.
	Cancelled { label: String, paused: usize },
}

/// What [`Session::restore`] picked back up from the last run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restored {
	Nothing,
	/// A rebuild was killed mid-way; it is running again under this label.
	Rebuild(String),
	/// Queued ops were waiting; this many are queued and draining.
	Queue(usize),
}

pub struct Session {
	/// Settings, both manifests, the external-package scan, and every
	/// mutation of the user's configuration.
	pub engine: Engine,
	/// Ops waiting for their turn, oldest first.
	pub queue: VecDeque<Op>,
	/// Output of the current or most recent rebuild, plus engine notes.
	pub log: Vec<String>,
	/// The last rebuild failure, kept across launches until a rebuild
	/// succeeds.
	pub last_error: Option<String>,
	build: Option<ActiveBuild>,
	build_tx: mpsc::UnboundedSender<BuildEvent>,
	escalation: Escalation,
	runtime: Option<Handle>,
	persistent: bool,
}

impl Session {
	/// Wraps `engine`, returning the session and the receiver its rebuilds
	/// report to. Rebuilds are spawned on the tokio runtime this is called
	/// from, if any; see [`Session::with_runtime`] otherwise.
	#[must_use]
	pub fn new(engine: Engine) -> (Self, mpsc::UnboundedReceiver<BuildEvent>) {
		let (build_tx, build_rx) = mpsc::unbounded_channel();
		let session = Self {
			engine,
			queue: VecDeque::new(),
			log: Vec::new(),
			last_error: None,
			build: None,
			build_tx,
			escalation: Escalation::default(),
			runtime: Handle::try_current().ok(),
			persistent: true,
		};
		(session, build_rx)
	}

	/// Spawns rebuilds on `runtime`, for a front-end whose own event loop
	/// is not tokio.
	#[must_use]
	pub fn with_runtime(mut self, runtime: Handle) -> Self {
		self.runtime = Some(runtime);
		self
	}

	/// Sets how rebuilds that need root obtain it.
	#[must_use]
	pub fn with_escalation(mut self, escalation: Escalation) -> Self {
		self.escalation = escalation;
		self
	}

	/// Keeps the session from reading or writing `state.json`, so tests
	/// never touch the real file.
	#[must_use]
	pub const fn without_persistence(mut self) -> Self {
		self.persistent = false;
		self
	}

	/// True while a rebuild is running.
	#[must_use]
	pub const fn is_building(&self) -> bool {
		self.build.is_some()
	}

	/// The label of the running rebuild.
	#[must_use]
	pub fn build_label(&self) -> Option<&str> {
		self.build.as_ref().map(|build| build.label.as_str())
	}

	/// True when `op`, or the same work, is already waiting in the queue.
	#[must_use]
	pub fn is_queued(&self, op: &Op) -> bool {
		self.queue.iter().any(|queued| queued.duplicates(op))
	}

	/// The part of `op` not already waiting: `None` for a duplicate, and a
	/// migrate op without the names another queued migrate already covers,
	/// so a batch that overlaps an earlier one still queues the rest.
	fn unqueued_part(&self, op: Op) -> Option<Op> {
		let Op::Migrate { mut names, scope } = op else {
			return (!self.is_queued(&op)).then_some(op);
		};
		names.retain(|name| {
			!self.queue.iter().any(|queued| {
				matches!(queued, Op::Migrate { names: queued_names, scope: queued_scope }
                    if *queued_scope == scope && queued_names.contains(name))
			})
		});
		(!names.is_empty()).then_some(Op::Migrate { names, scope })
	}

	/// Folds a [`Op::SetOptions`] into one already waiting for the same
	/// scope, newer values winning, so staged edits share one rebuild.
	/// Hands back any op it did not merge.
	fn merge_settings(&mut self, op: Op) -> Result<(), Op> {
		let Op::SetOptions { changes, scope } = op else {
			return Err(op);
		};
		let waiting = self.queue.iter_mut().find_map(|queued| match queued {
			Op::SetOptions {
				changes,
				scope: queued_scope,
			} if *queued_scope == scope => Some(changes),
			_ => None,
		});
		let Some(waiting) = waiting else {
			return Err(Op::SetOptions { changes, scope });
		};
		for change in changes {
			waiting.retain(|queued| queued.path != change.path);
			waiting.push(change);
		}
		Ok(())
	}

	/// Queues `op` and starts it if nothing else is running.
	pub fn enqueue(&mut self, op: Op) -> Enqueued {
		self.enqueue_all(vec![op])
	}

	/// Queues every op in `ops`, skipping any already waiting, then starts
	/// the queue if nothing else is running.
	pub fn enqueue_all(&mut self, ops: Vec<Op>) -> Enqueued {
		let mut added = false;
		for op in ops {
			let op = match self.merge_settings(op) {
				Ok(()) => {
					added = true;
					continue;
				}
				Err(op) => op,
			};
			if let Some(op) = self.unqueued_part(op) {
				self.queue.push_back(op);
				added = true;
			}
		}
		if !added {
			return Enqueued::Duplicate;
		}
		self.persist();
		if self.is_building() {
			return Enqueued::Queued;
		}
		self.drain().map_or(Enqueued::NotWritten, Enqueued::Started)
	}

	/// Drops the queued op at `index` without applying it.
	pub fn dequeue(&mut self, index: usize) -> Option<Op> {
		let op = self.queue.remove(index)?;
		self.persist();
		Some(op)
	}

	/// Forgets the last rebuild failure once the user has seen it.
	pub fn dismiss_error(&mut self) {
		if self.last_error.take().is_some() {
			self.persist();
		}
	}

	/// Takes the oldest scope in the queue, writes all of its ops, and starts
	/// one rebuild for them. Returns the rebuild's label, or `None` when
	/// something is already running or nothing could be written.
	pub fn drain(&mut self) -> Option<String> {
		loop {
			if self.is_building() {
				return None;
			}
			let Some(scope) = self.queue.front().map(Op::scope) else {
				self.persist();
				return None;
			};

			let mut applied = 0_usize;
			for op in self.take_scope(scope) {
				let mut reporter = LogReporter::new();
				let result = self.engine.apply(&op, &mut reporter);
				self.log.extend(reporter.into_lines());
				match result {
					Ok(()) => applied = applied.saturating_add(1),
					Err(e) => self
						.log
						.push(format!("{}: failed to write manifest: {e}", op.label())),
				}
			}
			if applied == 0 {
				self.log.push(format!(
					"No queued {} changes could be written.",
					scope.label()
				));
				self.persist();
				continue;
			}
			let label = format!("apply {applied} queued {} change(s)", scope.label());
			self.start_rebuild(scope, label.clone());
			return Some(label);
		}
	}

	/// Removes every pending op for `scope`, keeping the rest in order.
	fn take_scope(&mut self, scope: Target) -> Vec<Op> {
		let (batch, remaining): (Vec<Op>, Vec<Op>) = std::mem::take(&mut self.queue)
			.into_iter()
			.partition(|op| op.scope() == scope);
		self.queue = remaining.into();
		batch
	}

	/// Starts a rebuild for `scope` without writing anything first, for
	/// changes that are already on disk.
	pub fn start_rebuild(&mut self, scope: Target, label: String) {
		let (cancel_tx, cancel_rx) = oneshot::channel();
		self.build = Some(ActiveBuild {
			scope,
			label,
			cancel: Some(cancel_tx),
		});
		self.log.clear();
		self.persist();

		let config_dir = self.engine.config.home_manager_dir();
		let escalation = self.escalation.clone();
		let session_tx = self.build_tx.clone();
		let task = async move {
			let (tx, mut rx) = mpsc::channel(64);
			let forward = async {
				while let Some(event) = rx.recv().await {
					if session_tx.send(event).is_err() {
						break;
					}
				}
			};
			tokio::join!(
				rebuild::run(config_dir, scope, escalation, tx, cancel_rx),
				forward
			);
		};
		match self.runtime.clone().or_else(|| Handle::try_current().ok()) {
			Some(runtime) => {
				runtime.spawn(task);
			}
			None => {
				let _ = self.build_tx.send(BuildEvent::Finished(Err(
					"no async runtime to run the rebuild on".into(),
				)));
			}
		}
	}

	/// Asks the running rebuild to stop. Returns false when there is nothing
	/// to cancel or it was already asked.
	pub fn cancel_build(&mut self) -> bool {
		self.build
			.as_mut()
			.and_then(|build| build.cancel.take())
			.is_some_and(|cancel| cancel.send(()).is_ok())
	}

	/// Folds one event from the rebuild into the session. Returns how the
	/// rebuild ended once it has; a successful one moves straight on to the
	/// next scope in the queue.
	pub fn on_build_event(&mut self, event: BuildEvent) -> Option<BuildEnded> {
		match event {
			BuildEvent::Line(line) => {
				self.push_log(line);
				None
			}
			BuildEvent::Finished(result) => {
				let label = self.finish_build();
				let ended = match result {
					Ok(()) => {
						self.last_error = None;
						self.engine.refresh_externals();
						self.persist();
						BuildEnded::Succeeded {
							label,
							next: self.drain(),
						}
					}
					Err(error) => {
						self.last_error = Some(format!("{label}: {error}"));
						self.persist();
						BuildEnded::Failed {
							label,
							error,
							next: self.drain(),
						}
					}
				};
				Some(ended)
			}
			BuildEvent::Cancelled => {
				let label = self.finish_build();
				self.persist();
				Some(BuildEnded::Cancelled {
					label,
					paused: self.queue.len(),
				})
			}
		}
	}

	fn finish_build(&mut self) -> String {
		self.build
			.take()
			.map_or_else(|| "build".into(), |build| build.label)
	}

	fn push_log(&mut self, line: String) {
		self.log.push(line);
		if let Some(excess) = self.log.len().checked_sub(LOG_LIMIT)
			&& excess > 0
		{
			self.log.drain(..excess);
		}
	}

	/// Picks up what the last run left in `state.json`: an interrupted
	/// rebuild is run again, otherwise waiting ops are drained.
	pub fn restore(&mut self) -> Restored {
		if !self.persistent {
			return Restored::Nothing;
		}
		let Some(saved) = PersistedState::load() else {
			return Restored::Nothing;
		};
		self.last_error = saved.last_error;
		self.queue.extend(saved.pending_queue);

		if let Some(ip) = saved.in_progress {
			// The manifest was written before the rebuild was killed, so
			// only the rebuild needs to run again.
			let label = if ip.label.starts_with("resume ") {
				ip.label
			} else {
				format!("resume {}", ip.label)
			};
			self.start_rebuild(ip.scope, label.clone());
			return Restored::Rebuild(label);
		}
		if self.queue.is_empty() {
			return Restored::Nothing;
		}
		let queued = self.queue.len();
		self.drain();
		Restored::Queue(queued)
	}

	/// One migrate op per scope covering every external package that can be
	/// moved into a managed file and is not already queued for migration.
	#[must_use]
	pub fn migrate_all_ops(&self) -> Vec<Op> {
		let mut home = Vec::new();
		let mut nixos = Vec::new();
		for package in self
			.engine
			.external_packages
			.iter()
			.filter(|p| p.migratable)
		{
			match package.scope {
				ScanTarget::HomeManager => home.push(package.name.clone()),
				ScanTarget::Nixos => nixos.push(package.name.clone()),
			}
		}
		let mut ops: Vec<Op> = [(home, Target::HomeManager), (nixos, Target::NixosSystem)]
			.into_iter()
			.filter(|(names, _)| !names.is_empty())
			.filter_map(|(names, scope)| self.unqueued_part(Op::Migrate { names, scope }))
			.collect();
		ops.extend(
			self.engine
				.flakes
				.iter()
				.filter(|flake| flake.migratable())
				.filter_map(|flake| {
					let nixbox_nix::manifest::FlakeOutput::Package(package) = &flake.output else {
						return None;
					};
					self.unqueued_part(Op::MigrateFlakePackage {
						input: flake.input.clone(),
						package: package.clone(),
						scope: flake.scope,
					})
				}),
		);
		ops
	}

	/// Writes the part of the session that must survive a crash. Failing to
	/// is swallowed: it only costs the ability to resume.
	pub fn persist(&self) {
		if !self.persistent {
			return;
		}
		let snapshot = PersistedState {
			pending_queue: self.queue.iter().cloned().collect(),
			in_progress: self.build.as_ref().map(|build| InProgress {
				scope: build.scope,
				label: build.label.clone(),
			}),
			last_error: self.last_error.clone(),
		};
		let _ = snapshot.save();
	}

	/// The directory the rebuild evaluates, for display.
	#[must_use]
	pub fn config_dir(&self) -> PathBuf {
		self.engine.config.home_manager_dir()
	}

	/// Marks a rebuild as running without spawning one, so front-end tests
	/// can exercise their "busy" paths. Returns the cancel signal.
	#[cfg(any(test, feature = "test-util"))]
	pub fn fake_build(&mut self, scope: Target, label: &str) -> oneshot::Receiver<()> {
		let (cancel_tx, cancel_rx) = oneshot::channel();
		self.build = Some(ActiveBuild {
			scope,
			label: label.into(),
			cancel: Some(cancel_tx),
		});
		cancel_rx
	}
}

#[cfg(test)]
mod tests {
	use super::{BuildEnded, Enqueued, Session};
	use crate::engine::Engine;
	use crate::op::Op;
	use crate::tests::{hit, temp_dir};
	use nixbox_config::Target;
	use nixbox_nix::build::BuildEvent;
	use nixbox_nix::manifest::Manifest;

	fn session(config: nixbox_config::Config) -> Session {
		Session::new(Engine::from_parts(
			config,
			Manifest::default(),
			Manifest::default(),
			Vec::new(),
		))
		.0
		.without_persistence()
	}

	fn install(attr: &str, scope: Target) -> Op {
		Op::Install {
			hit: hit(attr),
			scope,
		}
	}

	#[test]
	fn duplicate_ops_are_not_queued_twice() {
		let dir = temp_dir("session-dup");
		let mut session = session(dir.config());
		let _cancel = session.fake_build(Target::HomeManager, "busy");

		assert_eq!(
			session.enqueue(install("fd", Target::HomeManager)),
			Enqueued::Queued
		);
		assert_eq!(
			session.enqueue(install("fd", Target::HomeManager)),
			Enqueued::Duplicate
		);
		assert_eq!(session.queue.len(), 1);
	}

	#[test]
	fn taking_a_scope_batches_all_of_its_pending_operations() {
		let dir = temp_dir("session-scope");
		let mut session = session(dir.config());
		session
			.queue
			.push_back(install("ripgrep", Target::HomeManager));
		session.queue.push_back(install("fd", Target::NixosSystem));
		session.queue.push_back(Op::Uninstall {
			name: "neovim".into(),
			scope: Target::HomeManager,
		});

		let batch = session.take_scope(Target::HomeManager);

		assert_eq!(batch.len(), 2);
		assert!(batch.iter().all(|op| op.scope() == Target::HomeManager));
		assert_eq!(session.queue.len(), 1);
		assert!(matches!(
			session.queue.front(),
			Some(Op::Install { hit, scope: Target::NixosSystem }) if hit.attr == "fd"
		));
	}

	#[tokio::test]
	async fn cancel_signals_the_running_rebuild_once() {
		let dir = temp_dir("session-cancel");
		let mut session = session(dir.config());
		let cancel = session.fake_build(Target::HomeManager, "busy");

		assert!(session.cancel_build());
		assert!(!session.cancel_build());
		cancel.await.expect("rebuild should be signalled");
		assert!(session.is_building());
	}

	#[test]
	fn cancelling_leaves_queued_work_paused() {
		let dir = temp_dir("session-paused");
		let mut session = session(dir.config());
		let _cancel = session.fake_build(Target::HomeManager, "apply 1");
		session.enqueue(install("fd", Target::NixosSystem));

		let ended = session.on_build_event(BuildEvent::Cancelled);

		assert_eq!(
			ended,
			Some(BuildEnded::Cancelled {
				label: "apply 1".into(),
				paused: 1
			})
		);
		assert!(!session.is_building());
		assert_eq!(session.queue.len(), 1);
	}

	#[test]
	fn a_failed_rebuild_is_remembered_until_one_succeeds() {
		let dir = temp_dir("session-failed");
		let mut session = session(dir.config());
		let _cancel = session.fake_build(Target::HomeManager, "apply 1");

		let ended = session.on_build_event(BuildEvent::Finished(Err("exit 1".into())));

		assert_eq!(
			ended,
			Some(BuildEnded::Failed {
				label: "apply 1".into(),
				error: "exit 1".into(),
				next: None
			})
		);
		assert_eq!(session.last_error.as_deref(), Some("apply 1: exit 1"));

		let _cancel = session.fake_build(Target::HomeManager, "apply 2");
		session.on_build_event(BuildEvent::Finished(Ok(())));
		assert!(session.last_error.is_none());
	}

	#[test]
	fn the_build_log_is_capped() {
		let dir = temp_dir("session-log");
		let mut session = session(dir.config());
		for n in 0..super::LOG_LIMIT + 5 {
			session.on_build_event(BuildEvent::Line(n.to_string()));
		}

		assert_eq!(session.log.len(), super::LOG_LIMIT);
		assert_eq!(session.log.first().map(String::as_str), Some("5"));
	}

	#[test]
	fn an_overlapping_migrate_queues_only_the_new_names() {
		let dir = temp_dir("session-migrate");
		let mut session = session(dir.config());
		let _cancel = session.fake_build(Target::HomeManager, "busy");
		let migrate = |names: &[&str]| Op::Migrate {
			names: names.iter().map(|name| (*name).to_string()).collect(),
			scope: Target::HomeManager,
		};

		session.enqueue(migrate(&["git"]));
		assert_eq!(
			session.enqueue_all(vec![migrate(&["git", "fd"])]),
			Enqueued::Queued
		);
		assert_eq!(session.enqueue(migrate(&["fd"])), Enqueued::Duplicate);

		let queued: Vec<&str> = session
			.queue
			.iter()
			.flat_map(|op| match op {
				Op::Migrate { names, .. } => names.iter().map(String::as_str).collect(),
				_ => Vec::new(),
			})
			.collect();
		assert_eq!(queued, ["git", "fd"]);
	}

	#[test]
	fn option_edits_for_a_scope_merge_into_the_waiting_op() {
		use crate::options::OptionChange;
		use nixbox_nix::settings::SettingValue;

		let dir = temp_dir("session-settings");
		let mut session = session(dir.config());
		let _cancel = session.fake_build(Target::NixosSystem, "busy");
		let set = |name: &str, value: Option<bool>| Op::SetOptions {
			changes: vec![OptionChange {
				path: vec!["programs".into(), "git".into(), name.into()],
				value: value.map(SettingValue::Bool),
			}],
			scope: Target::HomeManager,
		};

		assert_eq!(session.enqueue(set("enable", Some(true))), Enqueued::Queued);
		assert_eq!(session.enqueue(set("lfs", Some(true))), Enqueued::Queued);
		assert_eq!(session.enqueue(set("enable", None)), Enqueued::Queued);

		assert_eq!(session.queue.len(), 1);
		let Some(Op::SetOptions { changes, .. }) = session.queue.front() else {
			panic!("expected one settings op");
		};
		let summary: Vec<String> = changes.iter().map(OptionChange::label).collect();
		assert_eq!(
			summary,
			["programs.git.lfs = true", "unset programs.git.enable"]
		);
	}
}
