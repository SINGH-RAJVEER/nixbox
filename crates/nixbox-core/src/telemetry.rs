//! Opt-in usage counters. Construction is inert; only explicit front-end startup arms collection.

#[cfg(feature = "telemetry-receiver")]
pub mod receiver;
mod schema;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use nixbox_config::{Config, TelemetrySettings, TelemetryStatus};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

pub use schema::*;

/// Shared by every interface and install method. Changes require fresh consent.
/// There is deliberately no runtime endpoint override in the application.
pub const RECIPIENT: Option<&str> = Some("https://nixb.duckdns.org:58443/v1/telemetry");
pub const DISCLOSURE: &str = "Optional counts of app starts and completed rebuild/profile outcomes, grouped by release, interface, OS and CPU architecture. Includes target, outcome and coarse duration. No IDs, search text, package names, paths, configuration values or logs. The receiver sees the connection IP but stores only daily totals for 90 days. No retries or offline history.";

type ConsentSource = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

pub fn status(settings: TelemetrySettings) -> TelemetryStatus {
	let status = settings.status();
	if status == TelemetryStatus::Enabled && RECIPIENT.is_none() {
		TelemetryStatus::ReceiverNotConfigured
	} else {
		status
	}
}

pub fn status_label(settings: TelemetrySettings) -> &'static str {
	match status(settings) {
		TelemetryStatus::Enabled => "Enabled",
		TelemetryStatus::Disabled => "Off",
		TelemetryStatus::ConsentRequired => "Off: consent needs updating",
		TelemetryStatus::DisabledByEnvironment => "Off: NIXBOX_NO_TELEMETRY=1",
		TelemetryStatus::DisabledByDoNotTrack => "Off: DO_NOT_TRACK=1",
		TelemetryStatus::DevelopmentBuild => "Off: development build",
		TelemetryStatus::ContinuousIntegration => "Off: CI environment",
		TelemetryStatus::VirtualMachine => "Off: VM development",
		TelemetryStatus::ReceiverNotConfigured => "Off: receiver not configured",
	}
}

/// Consent cannot be granted for an undisclosed recipient.
pub fn set_enabled(config: &mut Config, enabled: bool) -> Result<()> {
	ensure!(
		!enabled || RECIPIENT.is_some(),
		"telemetry receiver is not configured; collection remains off"
	);
	config.set_telemetry_enabled(enabled)
}

#[derive(Clone, Default)]
pub struct Telemetry {
	inner: Option<Arc<Inner>>,
}

impl std::fmt::Debug for Telemetry {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.debug_struct("Telemetry").finish_non_exhaustive()
	}
}

struct Inner {
	interface: Interface,
	runtime: Handle,
	source: ConsentSource,
	destination: Option<Destination>,
	running: Mutex<Option<Running>>,
}

#[derive(Clone)]
struct Destination {
	url: String,
	https_only: bool,
}

struct Running {
	revision: u64,
	tx: mpsc::Sender<Message>,
	task: JoinHandle<()>,
}
impl Drop for Running {
	fn drop(&mut self) {
		self.task.abort();
	}
}

enum Message {
	Observe(Observation),
	Shutdown(oneshot::Sender<()>),
}

/// Consumed on completion, so duplicate terminal events cannot report an attempt twice.
pub struct ActivationAttempt {
	tx: mpsc::Sender<Message>,
	started: Instant,
	scope: Scope,
	activation: ActivationKind,
}

impl std::fmt::Debug for ActivationAttempt {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("ActivationAttempt")
			.finish_non_exhaustive()
	}
}

impl ActivationAttempt {
	pub fn finish(self, outcome: Outcome) {
		let _ = self
			.tx
			.try_send(Message::Observe(Observation::ActivationFinished {
				target: self.scope,
				activation: self.activation,
				outcome,
				duration: self.started.elapsed().into(),
			}));
	}
}

impl Telemetry {
	pub fn new(interface: Interface, runtime: Handle) -> Self {
		Self {
			inner: Some(Arc::new(Inner {
				interface,
				runtime,
				source: Arc::new(|| {
					let consent = Config::load_or_default().ok()?.telemetry;
					(consent.status() == TelemetryStatus::Enabled).then_some(consent.revision)
				}),
				destination: RECIPIENT.map(|url| Destination {
					url: url.into(),
					https_only: true,
				}),
				running: Mutex::new(None),
			})),
		}
	}

	/// Start counting only after successful front-end initialization or eligible CLI dispatch.
	pub fn started(&self) {
		self.refresh();
		if let Some(inner) = &self.inner
			&& let Ok(running) = inner.running.lock()
			&& let Some(run) = running.as_ref()
		{
			let _ = run
				.tx
				.try_send(Message::Observe(Observation::AppStarted {}));
		}
	}

	/// Call after a successfully persisted consent change. Does not invent a start observation.
	pub fn refresh(&self) {
		let Some(inner) = &self.inner else {
			return;
		};
		let Ok(mut running) = inner.running.lock() else {
			return;
		};
		let Some(destination) = inner.destination.clone() else {
			running.take();
			return;
		};
		let Some(revision) = (inner.source)() else {
			running.take();
			return;
		};
		if running
			.as_ref()
			.is_some_and(|run| run.revision == revision && !run.tx.is_closed())
		{
			return;
		}
		running.take();
		let (tx, rx) = mpsc::channel(64);
		let task = inner.runtime.spawn(worker(
			inner.interface,
			inner.source.clone(),
			destination,
			revision,
			rx,
		));
		*running = Some(Running { revision, tx, task });
	}

	/// Stop before attempting to persist an opt-out, including when persistence later fails.
	pub fn disable(&self) {
		if let Some(inner) = &self.inner
			&& let Ok(mut running) = inner.running.lock()
		{
			running.take();
		}
	}

	pub fn begin_activation(
		&self,
		scope: nixbox_config::Target,
		activation: crate::Activation,
	) -> Option<ActivationAttempt> {
		let inner = self.inner.as_ref()?;
		let running = inner.running.lock().ok()?;
		let run = running.as_ref()?;
		if (inner.source)() != Some(run.revision) || run.tx.is_closed() {
			return None;
		}
		Some(ActivationAttempt {
			tx: run.tx.clone(),
			started: Instant::now(),
			scope: scope.into(),
			activation: activation.into(),
		})
	}

	/// At most 100 ms for the final report. Cancellation and errors discard it.
	pub async fn shutdown(&self) {
		let run = self
			.inner
			.as_ref()
			.and_then(|inner| inner.running.lock().ok()?.take());
		if let Some(run) = run {
			let (done, wait) = oneshot::channel();
			if run.tx.try_send(Message::Shutdown(done)).is_ok() {
				let _ = tokio::time::timeout(Duration::from_millis(100), wait).await;
			}
		}
	}
}

fn aggregate(counters: &mut Vec<Counter>, observation: Observation) {
	if let Some(counter) = counters
		.iter_mut()
		.find(|counter| counter.observation == observation)
	{
		counter.count = counter.count.saturating_add(1).min(MAX_COUNT);
	} else if counters.len() < MAX_GROUPS {
		counters.push(Counter {
			observation,
			count: 1,
		});
	}
}

async fn worker(
	interface: Interface,
	source: ConsentSource,
	destination: Destination,
	revision: u64,
	mut rx: mpsc::Receiver<Message>,
) {
	let mut report = Report::current(interface);
	let mut last_send: Option<Instant> = None;
	let mut tick = tokio::time::interval(Duration::from_secs(1));
	tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
	loop {
		if source() != Some(revision) {
			return;
		}
		let (flush, shutdown) = tokio::select! {
			message = rx.recv() => match message {
				Some(Message::Observe(observation)) => {
					if source() != Some(revision) { return; }
					aggregate(&mut report.counters, observation);
					(false, None)
				}
				Some(Message::Shutdown(done)) => (true, Some(done)),
				None => return,
			},
			_ = tick.tick() => (true, None),
		};
		// CLI reports only at completion; interactive reports respect a one-minute cadence.
		let eligible = last_send.is_none_or(|last| last.elapsed() >= Duration::from_secs(60));
		if flush
			&& eligible
			&& !report.counters.is_empty()
			&& (interface != Interface::Cli || shutdown.is_some())
		{
			if source() != Some(revision) {
				return;
			}
			last_send = Some(Instant::now());
			let payload = std::mem::replace(&mut report, Report::current(interface));
			let send = transmit(&destination, &payload);
			tokio::pin!(send);
			loop {
				tokio::select! {
					_ = &mut send => break,
					_ = tick.tick() => if source() != Some(revision) { return; },
				}
			}
		}
		if let Some(done) = shutdown {
			let _ = done.send(());
			return;
		}
	}
}

async fn transmit(destination: &Destination, report: &Report) {
	if report.validate().is_err() {
		return;
	}
	let Ok(client) = reqwest::Client::builder()
		.https_only(destination.https_only)
		.redirect(reqwest::redirect::Policy::none())
		.referer(false)
		.retry(reqwest::retry::never())
		.user_agent("nixbox-telemetry/1")
		.pool_max_idle_per_host(0)
		.connect_timeout(Duration::from_secs(1))
		.timeout(Duration::from_secs(2))
		.build()
	else {
		return;
	};
	let _ = client.post(&destination.url).json(report).send().await;
}

#[cfg(test)]
mod tests;
