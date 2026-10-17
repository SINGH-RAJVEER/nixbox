//! Minimal aggregate-only Axum receiver. No request, address, or body logging.

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use axum::extract::{DefaultBodyLimit, State, rejection::JsonRejection};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::{Connection, params};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use super::{MAX_REPORT_BYTES, Report};

const MAX_DAILY_GROUPS: i64 = 4096;

struct Cleanup(tokio::task::JoinHandle<()>);
impl Drop for Cleanup {
	fn drop(&mut self) {
		self.0.abort();
	}
}

struct Store {
	connection: Mutex<Connection>,
}

impl Store {
	fn open(path: &Path) -> Result<Self> {
		let mut options = std::fs::OpenOptions::new();
		options.read(true).write(true).create(true).truncate(false);
		#[cfg(unix)]
		{
			use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
			options.mode(0o600);
			options
				.open(path)?
				.set_permissions(std::fs::Permissions::from_mode(0o600))?;
		}
		#[cfg(not(unix))]
		options.open(path)?;
		let connection = Connection::open(path)?;
		connection.busy_timeout(Duration::from_secs(1))?;
		connection.execute_batch(
			"PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE;
			CREATE TABLE IF NOT EXISTS daily_counts (
			day INTEGER NOT NULL, app_version TEXT NOT NULL, interface TEXT NOT NULL,
			os TEXT NOT NULL, arch TEXT NOT NULL, observation TEXT NOT NULL, total INTEGER NOT NULL,
			PRIMARY KEY(day, app_version, interface, os, arch, observation)
		) WITHOUT ROWID;",
		)?;
		let store = Self {
			connection: Mutex::new(connection),
		};
		store.prune(today()?)?;
		Ok(store)
	}

	fn prune(&self, day: i64) -> Result<()> {
		self.connection
			.lock()
			.map_err(|_| anyhow::anyhow!("database lock unavailable"))?
			.execute(
				"DELETE FROM daily_counts WHERE day < ?1",
				[day.saturating_sub(89)],
			)?;
		Ok(())
	}

	fn ingest(&self, report: &Report, day: i64) -> Result<()> {
		report.validate()?;
		let mut connection = self
			.connection
			.lock()
			.map_err(|_| anyhow::anyhow!("database lock unavailable"))?;
		let transaction = connection.transaction()?;
		transaction.execute(
			"DELETE FROM daily_counts WHERE day < ?1",
			[day.saturating_sub(89)],
		)?;
		let interface = label(report.interface)?;
		let os = label(report.os)?;
		let arch = label(report.arch)?;
		for counter in &report.counters {
			let observation = serde_json::to_string(&counter.observation)?;
			let key = params![day, report.app_version, interface, os, arch, observation];
			let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM daily_counts WHERE day=?1 AND app_version=?2 AND interface=?3 AND os=?4 AND arch=?5 AND observation=?6)", key, |row| row.get(0))?;
			if !exists {
				let groups: i64 = transaction.query_row(
					"SELECT COUNT(*) FROM daily_counts WHERE day=?1",
					[day],
					|row| row.get(0),
				)?;
				ensure!(groups < MAX_DAILY_GROUPS, "daily counter capacity reached");
			}
			transaction.execute(
				"INSERT INTO daily_counts VALUES (?1,?2,?3,?4,?5,?6,?7)
				ON CONFLICT(day,app_version,interface,os,arch,observation)
				DO UPDATE SET total=min(1000000000000, total+excluded.total)",
				params![
					day,
					report.app_version,
					interface,
					os,
					arch,
					observation,
					counter.count
				],
			)?;
		}
		transaction.commit()?;
		Ok(())
	}
}

fn label(value: impl serde::Serialize) -> Result<String> {
	match serde_json::to_value(value)? {
		serde_json::Value::String(value) => Ok(value),
		_ => anyhow::bail!("invalid dimension"),
	}
}

fn today() -> Result<i64> {
	Ok(i64::try_from(
		SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / 86400,
	)?)
}

#[derive(Clone)]
struct AppState {
	store: Arc<Store>,
	permits: Arc<Semaphore>,
	database_slots: Arc<Semaphore>,
	limit: Arc<Mutex<(Instant, u16)>>,
}

fn router(store: Arc<Store>) -> Router {
	let state = AppState {
		store,
		permits: Arc::new(Semaphore::new(32)),
		database_slots: Arc::new(Semaphore::new(8)),
		limit: Arc::new(Mutex::new((Instant::now(), 0))),
	};
	Router::new()
		.route("/v1/telemetry", post(ingest))
		.route("/health", get(|| async { StatusCode::NO_CONTENT }))
		.layer(DefaultBodyLimit::max(MAX_REPORT_BYTES))
		.layer(middleware::from_fn_with_state(state.clone(), limit))
		.with_state(state)
}

async fn limit(
	State(state): State<AppState>,
	request: axum::extract::Request,
	next: Next,
) -> Response {
	let Ok(_permit) = state.permits.try_acquire() else {
		return StatusCode::SERVICE_UNAVAILABLE.into_response();
	};
	{
		let Ok(mut limit) = state.limit.lock() else {
			return StatusCode::SERVICE_UNAVAILABLE.into_response();
		};
		if limit.0.elapsed() >= Duration::from_secs(60) {
			*limit = (Instant::now(), 0);
		}
		if limit.1 >= 600 {
			return StatusCode::TOO_MANY_REQUESTS.into_response();
		}
		limit.1 = limit.1.saturating_add(1);
	}
	tokio::time::timeout(Duration::from_secs(5), next.run(request))
		.await
		.unwrap_or_else(|_| StatusCode::REQUEST_TIMEOUT.into_response())
}

async fn ingest(
	State(state): State<AppState>,
	payload: std::result::Result<Json<Report>, JsonRejection>,
) -> StatusCode {
	let report = match payload {
		Ok(Json(report)) if report.validate().is_ok() => report,
		Err(error) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
			return StatusCode::PAYLOAD_TOO_LARGE;
		}
		_ => return StatusCode::BAD_REQUEST,
	};
	let Ok(permit) = state.database_slots.clone().try_acquire_owned() else {
		return StatusCode::SERVICE_UNAVAILABLE;
	};
	match tokio::task::spawn_blocking(move || {
		// Keep the work bound even if the HTTP request times out before SQLite finishes.
		let _permit = permit;
		state.store.ingest(&report, today()?)
	})
	.await
	{
		Ok(Ok(())) => StatusCode::NO_CONTENT,
		_ => StatusCode::SERVICE_UNAVAILABLE,
	}
}

/// Serve behind an HTTPS proxy. Expiry also runs while no reports arrive.
pub async fn serve(
	listener: TcpListener,
	database: impl AsRef<Path>,
	shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
	let store = Arc::new(Store::open(database.as_ref()).context("opening aggregate database")?);
	let cleanup_store = store.clone();
	let _cleanup = Cleanup(tokio::spawn(async move {
		loop {
			tokio::time::sleep(Duration::from_secs(60)).await;
			let store = cleanup_store.clone();
			let _ = tokio::task::spawn_blocking(move || store.prune(today()?)).await;
		}
	}));
	let result = axum::serve(listener, router(store))
		.with_graceful_shutdown(shutdown)
		.await;
	result.context("serving telemetry receiver")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn store_aggregates_without_raw_reports_and_expires_old_days() {
		let directory = tempfile::tempdir().expect("directory");
		let store = Store::open(&directory.path().join("counts.sqlite3")).expect("store");
		let report: Report =
			serde_json::from_str(&super::super::preview().expect("preview")).expect("report");
		store.ingest(&report, 100).expect("first report");
		store.ingest(&report, 100).expect("aggregate report");
		let total: i64 = store
			.connection
			.lock()
			.expect("connection")
			.query_row("SELECT SUM(total) FROM daily_counts", [], |row| row.get(0))
			.expect("total");
		assert_eq!(total, 4);
		store.prune(189).expect("retain 90 calendar days");
		let rows: i64 = store
			.connection
			.lock()
			.expect("connection")
			.query_row("SELECT COUNT(*) FROM daily_counts", [], |row| row.get(0))
			.expect("rows");
		assert_eq!(rows, 2);
		store.prune(190).expect("expire old day");
		let rows: i64 = store
			.connection
			.lock()
			.expect("connection")
			.query_row("SELECT COUNT(*) FROM daily_counts", [], |row| row.get(0))
			.expect("rows");
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn actual_http_receiver_rejects_bad_reports_and_bounds_bodies() {
		let directory = tempfile::tempdir().expect("directory");
		let store = Arc::new(Store::open(&directory.path().join("counts.sqlite3")).expect("store"));
		let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
		let url = format!(
			"http://{}/v1/telemetry",
			listener.local_addr().expect("address")
		);
		let server = tokio::spawn(axum::serve(listener, router(store.clone())).into_future());
		let client = reqwest::Client::new();
		let example = super::super::preview().expect("preview");
		for (body, expected) in [
			(example, StatusCode::NO_CONTENT),
			("{broken".into(), StatusCode::BAD_REQUEST),
			(
				" ".repeat(MAX_REPORT_BYTES + 1),
				StatusCode::PAYLOAD_TOO_LARGE,
			),
		] {
			assert_eq!(
				client
					.post(&url)
					.header("content-type", "application/json")
					.body(body)
					.send()
					.await
					.expect("response")
					.status(),
				expected
			);
		}
		server.abort();
		let total: i64 = store
			.connection
			.lock()
			.expect("connection")
			.query_row("SELECT SUM(total) FROM daily_counts", [], |row| row.get(0))
			.expect("total");
		assert_eq!(total, 2);
	}
}
