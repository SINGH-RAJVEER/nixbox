use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

async fn server(
	status: u16,
	location: Option<String>,
	hang: bool,
) -> (String, mpsc::Receiver<String>, JoinHandle<()>) {
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
	let url = format!(
		"http://{}/v1/telemetry",
		listener.local_addr().expect("address")
	);
	let (tx, rx) = mpsc::channel(8);
	let task = tokio::spawn(async move {
		while let Ok((mut stream, _)) = listener.accept().await {
			let mut bytes = Vec::new();
			let mut buffer = [0; 1024];
			loop {
				let size = stream.read(&mut buffer).await.expect("read");
				if size == 0 {
					break;
				}
				bytes.extend_from_slice(&buffer[..size]);
				if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
					let header = String::from_utf8_lossy(&bytes[..end]);
					let length: usize = header
						.lines()
						.find_map(|line| {
							line.to_lowercase()
								.strip_prefix("content-length:")
								.and_then(|value| value.trim().parse().ok())
						})
						.unwrap_or(0);
					if bytes.len() >= end + 4 + length {
						let _ = tx
							.send(String::from_utf8_lossy(&bytes[end + 4..]).into())
							.await;
						break;
					}
				}
			}
			if hang {
				std::future::pending::<()>().await;
			}
			let location = location
				.as_ref()
				.map(|url| format!("Location: {url}\r\n"))
				.unwrap_or_default();
			let _ = stream
				.write_all(
					format!(
						"HTTP/1.1 {status} result\r\nContent-Length: 0\r\nConnection: close\r\n{location}\r\n"
					)
					.as_bytes(),
				)
				.await;
		}
	});
	(url, rx, task)
}

fn collector(interface: Interface, url: String, revision: Arc<AtomicU64>) -> Telemetry {
	Telemetry {
		inner: Some(Arc::new(Inner {
			interface,
			runtime: Handle::current(),
			source: Arc::new(move || match revision.load(Ordering::SeqCst) {
				0 => None,
				revision => Some(revision),
			}),
			destination: Some(Destination {
				url,
				https_only: false,
			}),
			running: Mutex::new(None),
		})),
	}
}

#[tokio::test]
async fn disabled_consent_creates_no_worker_or_request() {
	let (url, mut received, server) = server(204, None, false).await;
	let telemetry = collector(Interface::Cli, url, Arc::new(AtomicU64::new(0)));
	telemetry.started();
	assert!(
		telemetry
			.inner
			.as_ref()
			.expect("inner")
			.running
			.lock()
			.expect("state")
			.is_none()
	);
	assert!(
		telemetry
			.begin_activation(
				nixbox_config::Target::NixosSystem,
				crate::Activation::Configuration
			)
			.is_none()
	);
	telemetry.shutdown().await;
	assert!(received.try_recv().is_err());
	server.abort();
}

#[tokio::test]
async fn cli_delivers_one_aggregate_report_only_at_completion() {
	let (url, mut received, server) = server(204, None, false).await;
	let telemetry = collector(Interface::Cli, url, Arc::new(AtomicU64::new(9)));
	telemetry.started();
	telemetry
		.begin_activation(
			nixbox_config::Target::HomeManager,
			crate::Activation::Packages,
		)
		.expect("attempt")
		.finish(Outcome::Succeeded);
	assert!(received.try_recv().is_err());
	telemetry.shutdown().await;
	let body = received.try_recv().expect("report");
	let report: Report = serde_json::from_str(&body).expect("report JSON");
	report.validate().expect("valid report");
	assert_eq!(report.interface, Interface::Cli);
	assert_eq!(report.counters.len(), 2);
	assert!(!body.contains("revision"));
	assert!(received.try_recv().is_err());
	server.abort();
}

#[tokio::test]
async fn opt_out_and_reenable_discard_old_observations_and_attempts() {
	let (url, mut received, server) = server(204, None, false).await;
	let revision = Arc::new(AtomicU64::new(1));
	let telemetry = collector(Interface::Cli, url, revision.clone());
	telemetry.started();
	let old = telemetry
		.begin_activation(
			nixbox_config::Target::NixosSystem,
			crate::Activation::Configuration,
		)
		.expect("attempt");
	telemetry.disable();
	revision.store(2, Ordering::SeqCst);
	telemetry.refresh();
	old.finish(Outcome::Failed);
	telemetry.shutdown().await;
	assert!(received.try_recv().is_err());
	server.abort();
}

#[tokio::test]
async fn external_opt_out_drops_a_pending_report() {
	let (url, mut received, server) = server(204, None, false).await;
	let revision = Arc::new(AtomicU64::new(1));
	let telemetry = collector(Interface::Cli, url, revision.clone());
	telemetry.started();
	revision.store(0, Ordering::SeqCst);
	telemetry.shutdown().await;
	assert!(received.try_recv().is_err());
	server.abort();
}

#[tokio::test]
async fn shutdown_cancels_a_hung_request_within_its_budget() {
	let (url, mut received, server) = server(204, None, true).await;
	let telemetry = collector(Interface::Gui, url, Arc::new(AtomicU64::new(1)));
	telemetry.started();
	tokio::time::timeout(Duration::from_secs(2), received.recv())
		.await
		.expect("request starts")
		.expect("report");
	let started = Instant::now();
	telemetry.shutdown().await;
	assert!(started.elapsed() < Duration::from_millis(300));
	server.abort();
}

#[tokio::test]
async fn redirects_and_failures_are_never_retried() {
	let (other, mut redirected, target) = server(204, None, false).await;
	for status in [307, 429, 500] {
		let (url, mut received, source) = server(status, Some(other.clone()), false).await;
		let telemetry = collector(Interface::Cli, url, Arc::new(AtomicU64::new(1)));
		telemetry.started();
		telemetry.shutdown().await;
		assert!(received.try_recv().is_ok());
		assert!(received.try_recv().is_err());
		assert!(redirected.try_recv().is_err());
		source.abort();
	}
	target.abort();
}

#[test]
fn aggregation_saturates_counts_and_bounds_distinct_groups() {
	let mut counters = Vec::new();
	for _ in 0..2000 {
		aggregate(&mut counters, Observation::AppStarted {});
	}
	assert_eq!(counters.first().expect("counter").count, MAX_COUNT);
	for target in [Scope::Nixos, Scope::HomeManager] {
		for activation in [ActivationKind::Configuration, ActivationKind::Packages] {
			for outcome in [Outcome::Succeeded, Outcome::Failed, Outcome::Cancelled] {
				for duration in [
					DurationBucket::Under10Seconds,
					DurationBucket::Under1Minute,
					DurationBucket::Under5Minutes,
					DurationBucket::Under15Minutes,
					DurationBucket::AtLeast15Minutes,
				] {
					aggregate(
						&mut counters,
						Observation::ActivationFinished {
							target,
							activation,
							outcome,
							duration,
						},
					);
				}
			}
		}
	}
	assert_eq!(counters.len(), MAX_GROUPS);
}

#[tokio::test]
async fn session_reports_an_active_attempt_once_without_labels_or_errors() {
	let (url, mut received, server) = server(204, None, false).await;
	let telemetry = collector(Interface::Cli, url, Arc::new(AtomicU64::new(1)));
	telemetry.started();
	let engine = crate::Engine::from_parts(
		Config::default(),
		nixbox_nix::manifest::Manifest::default(),
		nixbox_nix::manifest::Manifest::default(),
		Vec::new(),
	);
	let (session, _) = crate::Session::new(engine);
	let mut session = session
		.without_persistence()
		.with_telemetry(telemetry.clone());
	let _cancel = session.fake_build(
		nixbox_config::Target::NixosSystem,
		"/home/private/package-name",
	);
	for _ in 0..2 {
		session.on_build_event(nixbox_nix::build::BuildEvent::Finished(Err(
			"secret failure output".into(),
		)));
	}
	telemetry.shutdown().await;
	let body = received.try_recv().expect("report");
	assert!(!body.contains("private"));
	assert!(!body.contains("secret"));
	let report: Report = serde_json::from_str(&body).expect("report");
	let completed: u16 = report
		.counters
		.iter()
		.filter(|counter| matches!(counter.observation, Observation::ActivationFinished { .. }))
		.map(|counter| counter.count)
		.sum();
	assert_eq!(completed, 1);
	server.abort();
}

#[cfg(feature = "telemetry-receiver")]
#[tokio::test]
async fn collector_delivers_through_axum_into_sqlite() {
	let directory = tempfile::tempdir().expect("directory");
	let database = directory.path().join("counts.sqlite3");
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
	let base = format!("http://{}", listener.local_addr().expect("address"));
	let (stop, stopped) = oneshot::channel();
	let receiver_database = database.clone();
	let receiver = tokio::spawn(receiver::serve(listener, receiver_database, async {
		let _ = stopped.await;
	}));
	assert!(
		reqwest::get(format!("{base}/health"))
			.await
			.expect("health")
			.status()
			.is_success()
	);
	let telemetry = collector(
		Interface::Cli,
		format!("{base}/v1/telemetry"),
		Arc::new(AtomicU64::new(1)),
	);
	telemetry.started();
	telemetry
		.begin_activation(
			nixbox_config::Target::HomeManager,
			crate::Activation::Packages,
		)
		.expect("attempt")
		.finish(Outcome::Succeeded);
	telemetry.shutdown().await;
	let _ = stop.send(());
	receiver.await.expect("receiver task").expect("receiver");
	let database = rusqlite::Connection::open(database).expect("database");
	let total: i64 = database
		.query_row("SELECT SUM(total) FROM daily_counts", [], |row| row.get(0))
		.expect("totals");
	assert_eq!(total, 2);
}
