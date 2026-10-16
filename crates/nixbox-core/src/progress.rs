//! A rebuild's progress, read from nix's structured log.
//!
//! A rebuild moves through a fixed set of [`Step`]s: preparing the command,
//! evaluating the configuration, downloading and building what it needs, and
//! activating the result. Downloads and builds run side by side. Their counts
//! are nix's own, the progress its Builds and CopyPaths activities report, so
//! nothing here is estimated. Evaluation and activation have no count, only a
//! duration and what they print.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use nixbox_nix::nix_log::{Activity, Field, INFO, NixLog, Report, store_name};

/// Lines each step keeps. Older lines are dropped first.
pub const STEP_LOG_LIMIT: usize = 5000;

/// Lines that mean a switch has moved on to activation, for when nix's own
/// activities do not already show realisation has ended.
const ACTIVATION_MARKERS: [&str; 3] = [
	"activating the configuration",
	"Starting Home Manager activation",
	"updating GRUB",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKind {
	Prepare,
	Evaluate,
	Download,
	Build,
	Activate,
}

impl StepKind {
	pub const ALL: [Self; 5] = [
		Self::Prepare,
		Self::Evaluate,
		Self::Download,
		Self::Build,
		Self::Activate,
	];

	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Prepare => "Prepare",
			Self::Evaluate => "Evaluate configuration",
			Self::Download => "Download",
			Self::Build => "Build",
			Self::Activate => "Activate",
		}
	}

	const fn index(self) -> usize {
		self as usize
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
	Pending,
	Running,
	Succeeded,
	Failed,
	Cancelled,
	/// Never needed: nothing to download, nothing to build.
	Skipped,
}

impl StepState {
	#[must_use]
	pub const fn ended(self) -> bool {
		matches!(
			self,
			Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped
		)
	}
}

#[derive(Debug, Clone)]
pub struct Step {
	pub kind: StepKind,
	pub state: StepState,
	pub started: Option<Instant>,
	pub ended: Option<Instant>,
	pub lines: VecDeque<String>,
	/// Lines dropped from the front to stay under [`STEP_LOG_LIMIT`], so the
	/// numbers shown beside the rest stay true.
	pub dropped: usize,
}

impl Step {
	const fn new(kind: StepKind) -> Self {
		Self {
			kind,
			state: StepState::Pending,
			started: None,
			ended: None,
			lines: VecDeque::new(),
			dropped: 0,
		}
	}

	/// How long the step ran, or has run so far.
	#[must_use]
	pub fn elapsed(&self, now: Instant) -> Option<Duration> {
		let started = self.started?;
		Some(self.ended.unwrap_or(now).saturating_duration_since(started))
	}

	/// Whether the step belongs in a list of the rebuild's steps: it ran, or
	/// it is one every rebuild goes through and has yet to start.
	#[must_use]
	pub const fn shown(&self) -> bool {
		match self.state {
			StepState::Skipped => false,
			StepState::Pending => matches!(self.kind, StepKind::Evaluate | StepKind::Activate),
			_ => true,
		}
	}

	fn push(&mut self, line: String) {
		self.lines.push_back(line);
		if self.lines.len() > STEP_LOG_LIMIT {
			self.lines.pop_front();
			self.dropped += 1;
		}
	}
}

/// Nix's count of one kind of work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count {
	pub done: u64,
	pub expected: u64,
	pub running: u64,
	pub failed: u64,
}

impl Count {
	/// How much is done, from 0 to 1, or `None` when nothing is expected.
	#[must_use]
	pub fn fraction(&self) -> Option<f32> {
		(self.expected > 0).then(|| self.done.min(self.expected) as f32 / self.expected as f32)
	}

	const fn add(self, other: Self) -> Self {
		Self {
			done: self.done + other.done,
			expected: self.expected + other.expected,
			running: self.running + other.running,
			failed: self.failed + other.failed,
		}
	}

	fn read(fields: &[Field]) -> Self {
		let at = |index: usize| fields.get(index).and_then(Field::int).unwrap_or(0);
		Self {
			done: at(0),
			expected: at(1),
			running: at(2),
			failed: at(3),
		}
	}
}

/// One activity nix has started.
#[derive(Debug, Clone)]
struct Tracked {
	activity: Activity,
	parent: u64,
	active: bool,
	/// The derivation or path it works on.
	name: String,
	/// The build phase a Build is in.
	phase: Option<String>,
	count: Count,
	/// Bytes a Realise expects to download.
	expected_download: u64,
}

/// What a rebuild has done so far.
#[derive(Debug, Clone, Default)]
pub struct BuildProgress {
	/// Counts rebuilds, so a front-end can tell a new one from the last.
	pub run: u64,
	/// What the rebuild applies, such as `apply 2 queued nixos change(s)`.
	pub label: String,
	/// One per [`StepKind`], in order; empty before the first rebuild.
	pub steps: Vec<Step>,
	pub started: Option<Instant>,
	pub ended: Option<Instant>,
	/// The file evaluation is reading.
	pub evaluating: Option<String>,
	/// The first error nix reported.
	pub error: Option<String>,
	activities: HashMap<u64, Tracked>,
	realising: bool,
}

/// How a rebuild ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
	Succeeded,
	Failed,
	Cancelled,
}

impl BuildProgress {
	/// Forgets the last rebuild and starts preparing `label`.
	pub fn start(&mut self, label: &str) {
		let now = Instant::now();
		*self = Self {
			run: self.run.wrapping_add(1),
			label: label.to_string(),
			steps: StepKind::ALL.into_iter().map(Step::new).collect(),
			started: Some(now),
			..Self::default()
		};
		self.begin(StepKind::Prepare);
	}

	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.steps.is_empty()
	}

	/// True from [`Self::start`] until [`Self::finish`].
	#[must_use]
	pub fn is_running(&self) -> bool {
		self.started.is_some() && self.ended.is_none()
	}

	#[must_use]
	pub fn step(&self, kind: StepKind) -> Option<&Step> {
		self.steps.get(kind.index())
	}

	fn state(&self, kind: StepKind) -> StepState {
		self.step(kind)
			.map_or(StepState::Pending, |step| step.state)
	}

	/// The step output goes to: the latest one running, else the latest
	/// one that ran.
	#[must_use]
	pub fn current(&self) -> Option<StepKind> {
		let running = self
			.steps
			.iter()
			.rev()
			.find(|step| step.state == StepState::Running);
		running
			.or_else(|| self.steps.iter().rev().find(|step| step.started.is_some()))
			.map(|step| step.kind)
	}

	/// The step a failed rebuild failed in.
	#[must_use]
	pub fn failed_step(&self) -> Option<StepKind> {
		self.steps
			.iter()
			.find(|step| step.state == StepState::Failed)
			.map(|step| step.kind)
	}

	fn total(&self, activity: Activity) -> Count {
		self.activities
			.values()
			.filter(|tracked| tracked.activity == activity)
			.fold(Count::default(), |sum, tracked| sum.add(tracked.count))
	}

	/// Derivations built, of those nix expects to build.
	#[must_use]
	pub fn builds(&self) -> Count {
		self.total(Activity::Builds)
	}

	/// Store paths fetched from a cache, of those nix expects to fetch.
	#[must_use]
	pub fn downloads(&self) -> Count {
		self.total(Activity::CopyPaths)
	}

	/// Bytes downloaded for those paths, and the bytes nix expects.
	#[must_use]
	pub fn download_bytes(&self) -> (u64, u64) {
		let done: u64 = self
			.activities
			.values()
			.filter(|tracked| {
				tracked.activity == Activity::FileTransfer
					&& self
						.activities
						.get(&tracked.parent)
						.is_some_and(|parent| parent.activity == Activity::CopyPath)
			})
			.map(|tracked| tracked.count.done)
			.sum();
		let expected: u64 = self
			.activities
			.values()
			.filter(|tracked| tracked.activity == Activity::Realise)
			.map(|tracked| tracked.expected_download)
			.sum();
		(done, expected.max(done))
	}

	/// What is building now, with the phase each build is in.
	#[must_use]
	pub fn building(&self) -> Vec<(&str, Option<&str>)> {
		let mut building: Vec<_> = self
			.activities
			.values()
			.filter(|tracked| tracked.active && tracked.activity == Activity::Build)
			.map(|tracked| (tracked.name.as_str(), tracked.phase.as_deref()))
			.collect();
		building.sort_unstable();
		building
	}

	/// How much of the counted work is done, from 0 to 1. `None` while
	/// nothing is counted, such as during evaluation.
	#[must_use]
	pub fn fraction(&self) -> Option<f32> {
		self.builds().add(self.downloads()).fraction()
	}

	/// A short account of what `kind` has done, for beside its name.
	#[must_use]
	pub fn summary(&self, kind: StepKind) -> Option<String> {
		let step = self.step(kind)?;
		match kind {
			StepKind::Evaluate if step.state == StepState::Running => self.evaluating.clone(),
			StepKind::Download => {
				let paths = self.downloads();
				let (done, expected) = self.download_bytes();
				let mut text = format!("{} of {} paths", paths.done, paths.expected);
				if expected > 0 {
					text.push_str(&format!(", {} of {}", size(done), size(expected)));
				}
				Some(text)
			}
			StepKind::Build => {
				let builds = self.builds();
				let mut text = format!("{} of {}", builds.done, builds.expected);
				if builds.failed > 0 {
					text.push_str(&format!(", {} failed", builds.failed));
				}
				Some(text)
			}
			_ => None,
		}
	}

	fn begin(&mut self, kind: StepKind) {
		let now = Instant::now();
		if let Some(step) = self.steps.get_mut(kind.index())
			&& step.state != StepState::Running
		{
			// Downloads and builds can come back for more work, such as
			// building something evaluation imports; they keep their start.
			step.started.get_or_insert(now);
			step.ended = None;
			step.state = StepState::Running;
		}
	}

	fn end(&mut self, kind: StepKind, state: StepState) {
		if let Some(step) = self.steps.get_mut(kind.index())
			&& step.state == StepState::Running
		{
			step.state = state;
			step.ended = Some(Instant::now());
		}
	}

	fn push(&mut self, kind: StepKind, line: String) {
		if let Some(step) = self.steps.get_mut(kind.index()) {
			step.push(line);
		}
	}

	fn push_current(&mut self, line: String) {
		if let Some(kind) = self.current() {
			self.push(kind, line);
		}
	}

	/// True once realisation has started and every activity of it stopped.
	fn realised(&self) -> bool {
		self.realising
			&& !self.activities.values().any(|tracked| {
				tracked.active
					&& matches!(
						tracked.activity,
						Activity::Realise | Activity::Builds | Activity::CopyPaths
					)
			})
	}

	fn begin_activation(&mut self) {
		for kind in [
			StepKind::Prepare,
			StepKind::Evaluate,
			StepKind::Download,
			StepKind::Build,
		] {
			self.end(kind, StepState::Succeeded);
		}
		self.begin(StepKind::Activate);
	}

	/// Folds in a plain output line: the command, the wrapper's own notes,
	/// and whatever activation prints.
	pub fn on_line(&mut self, line: &str) {
		if !self.is_running() {
			return;
		}
		if self.state(StepKind::Prepare) == StepState::Running && line.starts_with("$ ") {
			self.push(StepKind::Prepare, line.to_string());
			self.end(StepKind::Prepare, StepState::Succeeded);
			self.begin(StepKind::Evaluate);
			return;
		}
		if self.state(StepKind::Activate) == StepState::Pending
			&& self.state(StepKind::Prepare).ended()
			&& (self.realised()
				|| ACTIVATION_MARKERS
					.iter()
					.any(|marker| line.contains(marker)))
		{
			self.begin_activation();
		}
		self.push_current(line.to_string());
	}

	/// Folds in one line of nix's structured log.
	pub fn on_nix(&mut self, log: &NixLog) {
		if !self.is_running() {
			return;
		}
		match log {
			NixLog::Start {
				id,
				parent,
				activity,
				text,
				fields,
				..
			} => {
				let name = fields
					.first()
					.and_then(Field::str)
					.map(|path| store_name(path).to_string())
					.unwrap_or_default();
				self.activities.insert(
					*id,
					Tracked {
						activity: *activity,
						parent: *parent,
						active: true,
						name,
						phase: None,
						count: Count::default(),
						expected_download: 0,
					},
				);
				let lines = log.plain_lines();
				let target = match activity {
					Activity::Realise | Activity::Builds | Activity::CopyPaths => {
						self.realising = true;
						self.end(StepKind::Prepare, StepState::Succeeded);
						self.end(StepKind::Evaluate, StepState::Succeeded);
						None
					}
					Activity::Build => {
						self.begin(StepKind::Build);
						Some(StepKind::Build)
					}
					Activity::CopyPath | Activity::Substitute => {
						self.begin(StepKind::Download);
						Some(StepKind::Download)
					}
					_ => None,
				};
				if text.is_empty() {
					return;
				}
				for line in lines {
					match target {
						Some(kind) => self.push(kind, line),
						None => self.push_current(line),
					}
				}
			}
			NixLog::Stop { id } => {
				let Some(tracked) = self.activities.get_mut(id) else {
					return;
				};
				tracked.active = false;
				let activity = tracked.activity;
				if matches!(activity, Activity::Builds | Activity::CopyPaths) {
					// The last count can still expect work that turned out not to
					// be needed; once stopped, what was done is all there was.
					tracked.count.expected = tracked.count.done + tracked.count.failed;
					tracked.count.running = 0;
				}
				let step = match activity {
					Activity::Builds => StepKind::Build,
					Activity::CopyPaths => StepKind::Download,
					_ => return,
				};
				let busy = self
					.activities
					.values()
					.any(|tracked| tracked.active && tracked.activity == activity);
				if !busy {
					let failed = activity == Activity::Builds && self.builds().failed > 0;
					self.end(
						step,
						if failed {
							StepState::Failed
						} else {
							StepState::Succeeded
						},
					);
				}
			}
			NixLog::Result { id, report, fields } => self.on_result(*id, *report, fields),
			NixLog::Message { level, text } => {
				if *level <= INFO {
					if *level == 0 && self.error.is_none() {
						self.error = Some(text.trim().to_string());
					}
					for line in text.lines() {
						self.push_current(line.to_string());
					}
				} else if self.state(StepKind::Evaluate) == StepState::Running
					&& let Some(file) = text
						.strip_prefix("evaluating file '")
						.and_then(|rest| rest.strip_suffix('\''))
				{
					self.evaluating = Some(short_path(file).to_string());
				}
			}
		}
	}

	fn on_result(&mut self, id: u64, report: Report, fields: &[Field]) {
		let Some(tracked) = self.activities.get_mut(&id) else {
			return;
		};
		match report {
			Report::Progress => {
				tracked.count = Count::read(fields);
				let step = match tracked.activity {
					Activity::Builds => StepKind::Build,
					Activity::CopyPaths => StepKind::Download,
					_ => return,
				};
				if tracked.count.expected > tracked.count.done {
					self.begin(step);
				}
			}
			Report::SetExpected => {
				let kind = fields.first().and_then(Field::int).map(Activity::from_code);
				let expected = fields.get(1).and_then(Field::int).unwrap_or(0);
				if kind == Some(Activity::FileTransfer) {
					tracked.expected_download = expected;
				}
			}
			Report::SetPhase => {
				tracked.phase = fields.first().and_then(Field::str).map(str::to_string);
			}
			Report::BuildLogLine | Report::PostBuildLogLine => {
				let Some(line) = fields.first().and_then(Field::str) else {
					return;
				};
				if tracked.activity == Activity::Build {
					let line = format!("{}> {line}", tracked.name);
					self.push(StepKind::Build, line);
				} else {
					self.push_current(line.to_string());
				}
			}
			_ => {}
		}
	}

	/// Ends every step as the rebuild ended.
	pub fn finish(&mut self, outcome: Outcome) {
		if !self.is_running() {
			return;
		}
		let now = Instant::now();
		self.ended = Some(now);
		let (builds, downloads) = (self.builds(), self.downloads());
		let complete = |count: Count| count.failed == 0 && count.done >= count.expected;
		for step in &mut self.steps {
			step.state = match (step.state, outcome) {
				(StepState::Pending, _) => StepState::Skipped,
				(StepState::Running, Outcome::Succeeded) => StepState::Succeeded,
				(StepState::Running, Outcome::Cancelled) => StepState::Cancelled,
				(StepState::Running, Outcome::Failed) => match step.kind {
					StepKind::Build if complete(builds) => StepState::Succeeded,
					StepKind::Download if complete(downloads) => StepState::Succeeded,
					_ => StepState::Failed,
				},
				(state, _) => state,
			};
			if step.started.is_some() && step.ended.is_none() {
				step.ended = Some(now);
			}
		}
		// A failure between steps, such as setting the system profile after
		// the build, still belongs to the step that last ran.
		if outcome == Outcome::Failed
			&& self.failed_step().is_none()
			&& let Some(step) = self
				.steps
				.iter_mut()
				.filter(|step| step.started.is_some())
				.max_by_key(|step| step.started)
		{
			step.state = StepState::Failed;
		}
	}
}

/// Bytes as nix prints them.
#[must_use]
pub fn size(bytes: u64) -> String {
	const KIB: f64 = 1024.;
	let bytes = bytes as f64;
	if bytes >= KIB * KIB * KIB {
		format!("{:.1} GiB", bytes / (KIB * KIB * KIB))
	} else if bytes >= KIB * KIB {
		format!("{:.1} MiB", bytes / (KIB * KIB))
	} else {
		format!("{:.0} KiB", bytes / KIB)
	}
}

/// A duration the way a step list shows it: `0s`, `42s`, `3m 5s`, `1h 2m`.
#[must_use]
pub fn duration(elapsed: Duration) -> String {
	let seconds = elapsed.as_secs();
	match seconds {
		0..60 => format!("{seconds}s"),
		60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
		_ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
	}
}

/// A store path without its store prefix: `nixos/modules/x.nix` for a file
/// inside `/nix/store/<hash>-source/nixos/modules/x.nix`.
fn short_path(path: &str) -> &str {
	path.strip_prefix("/nix/store/")
		.and_then(|rest| rest.split_once('/'))
		.map_or(path, |(_, inner)| inner)
}

#[cfg(test)]
mod tests {
	use super::*;

	const FIXTURE: &str = include_str!("../tests/fixtures/nix-log.jsonl");

	fn running() -> BuildProgress {
		let mut progress = BuildProgress::default();
		progress.start("apply 1");
		progress.on_line("$ sudo nixos-rebuild switch --flake /etc/nixos#nixos");
		progress
	}

	fn feed(progress: &mut BuildProgress, log: &str) {
		for line in log.lines() {
			match NixLog::parse(line) {
				Some(log) => progress.on_nix(&log),
				None => progress.on_line(line),
			}
		}
	}

	#[test]
	fn the_command_ends_preparation_and_starts_evaluation() {
		let progress = running();

		assert_eq!(progress.state(StepKind::Prepare), StepState::Succeeded);
		assert_eq!(progress.state(StepKind::Evaluate), StepState::Running);
		assert_eq!(progress.current(), Some(StepKind::Evaluate));
		assert_eq!(progress.fraction(), None);
	}

	#[test]
	fn a_real_log_counts_downloads_and_builds_then_activates() {
		let mut progress = running();
		feed(&mut progress, "building the system configuration...");
		feed(&mut progress, FIXTURE);

		assert_eq!(progress.state(StepKind::Evaluate), StepState::Succeeded);
		assert_eq!(progress.state(StepKind::Download), StepState::Succeeded);
		assert_eq!(progress.state(StepKind::Build), StepState::Succeeded);
		assert_eq!(
			progress.downloads(),
			Count {
				done: 1,
				expected: 1,
				running: 0,
				failed: 0
			}
		);
		assert_eq!(progress.builds().done, 1);
		// Nix still expected a second build when it stopped; it was not needed.
		assert_eq!(progress.builds().expected, 1);
		assert_eq!(progress.download_bytes(), (1_607_088, 1_607_088));
		assert_eq!(progress.fraction(), Some(1.));
		assert!(progress.building().is_empty());

		let build = progress.step(StepKind::Build).unwrap();
		assert!(
			build
				.lines
				.iter()
				.any(|line| line == "nixbox-probe-1792078560997766440> hello from build")
		);
		let download = progress.step(StepKind::Download).unwrap();
		assert!(download.lines[0].starts_with("copying path '/nix/store/"));
		let evaluate = progress.step(StepKind::Evaluate).unwrap();
		assert!(
			evaluate
				.lines
				.iter()
				.any(|line| line == "building the system configuration...")
		);

		feed(
			&mut progress,
			"activating the configuration...\nsetting up /etc...",
		);
		assert_eq!(progress.state(StepKind::Activate), StepState::Running);
		assert_eq!(
			progress.step(StepKind::Activate).unwrap().lines.len(),
			2,
			"activation output belongs to activation"
		);
		progress.finish(Outcome::Succeeded);
		assert!(
			progress
				.steps
				.iter()
				.all(|step| step.state == StepState::Succeeded)
		);
	}

	#[test]
	fn evaluation_reports_the_file_it_reads() {
		let mut progress = running();
		feed(
			&mut progress,
			r#"@nix {"action":"msg","level":4,"msg":"evaluating file '/nix/store/fd5qgysvbfx27wj651swlr899w6jnbwd-source/nixos/modules/system/activation/top-level.nix'"}"#,
		);

		assert_eq!(
			progress.summary(StepKind::Evaluate).as_deref(),
			Some("nixos/modules/system/activation/top-level.nix")
		);
	}

	#[test]
	fn an_evaluation_error_fails_evaluation_and_skips_the_rest() {
		let mut progress = running();
		feed(
			&mut progress,
			"@nix {\"action\":\"msg\",\"level\":0,\"msg\":\"\\u001b[31;1merror:\\u001b[0m undefined variable 'foo'\"}",
		);
		progress.finish(Outcome::Failed);

		assert_eq!(progress.failed_step(), Some(StepKind::Evaluate));
		assert_eq!(
			progress.error.as_deref(),
			Some("error: undefined variable 'foo'")
		);
		assert_eq!(progress.state(StepKind::Build), StepState::Skipped);
		assert!(!progress.step(StepKind::Build).unwrap().shown());
		let shown: Vec<StepKind> = progress
			.steps
			.iter()
			.filter(|step| step.shown())
			.map(|step| step.kind)
			.collect();
		assert_eq!(shown, [StepKind::Prepare, StepKind::Evaluate]);
	}

	#[test]
	fn a_failed_build_fails_the_build_step() {
		let mut progress = running();
		feed(
			&mut progress,
			r#"@nix {"action":"start","id":1,"level":0,"parent":0,"text":"","type":104}
@nix {"action":"result","fields":[0,2,1,0],"id":1,"type":105}
@nix {"action":"start","fields":["/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-broken-1.0.drv","",1,1],"id":2,"level":3,"parent":0,"text":"building '/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-broken-1.0.drv'","type":105}
@nix {"action":"result","fields":["buildPhase"],"id":2,"type":104}"#,
		);
		assert_eq!(progress.building(), [("broken-1.0", Some("buildPhase"))]);
		assert_eq!(progress.fraction(), Some(0.));
		assert_eq!(progress.summary(StepKind::Build).as_deref(), Some("0 of 2"));

		feed(
			&mut progress,
			r#"@nix {"action":"result","fields":[0,2,0,1],"id":1,"type":105}
@nix {"action":"stop","id":2}"#,
		);
		progress.finish(Outcome::Failed);
		assert_eq!(progress.failed_step(), Some(StepKind::Build));
		assert_eq!(progress.state(StepKind::Activate), StepState::Skipped);
	}

	#[test]
	fn cancelling_marks_the_running_step() {
		let mut progress = running();
		progress.finish(Outcome::Cancelled);

		assert_eq!(progress.state(StepKind::Evaluate), StepState::Cancelled);
		assert!(!progress.is_running());
		// Nothing after the end changes it.
		progress.on_line("activating the configuration...");
		assert_eq!(progress.state(StepKind::Activate), StepState::Skipped);
	}

	#[test]
	fn step_logs_are_capped_but_keep_their_numbering() {
		let mut progress = running();
		for n in 0..STEP_LOG_LIMIT + 3 {
			progress.on_line(&n.to_string());
		}

		let evaluate = progress.step(StepKind::Evaluate).unwrap();
		assert_eq!(evaluate.lines.len(), STEP_LOG_LIMIT);
		assert_eq!(evaluate.dropped, 3);
		assert_eq!(evaluate.lines.front().map(String::as_str), Some("3"));
	}

	#[test]
	fn sizes_and_durations_read_like_nix_and_a_step_list() {
		assert_eq!(size(1_607_088), "1.5 MiB");
		assert_eq!(size(512), "0 KiB");
		assert_eq!(duration(Duration::from_secs(42)), "42s");
		assert_eq!(duration(Duration::from_secs(185)), "3m 5s");
		assert_eq!(duration(Duration::from_secs(3720)), "1h 2m");
	}
}
