use anyhow::{Context, Result};
use nixbox_core::telemetry::receiver;

#[tokio::main]
async fn main() -> Result<()> {
	let address =
		std::env::var("NIXBOX_TELEMETRY_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
	let database = std::env::var_os("NIXBOX_TELEMETRY_DATABASE")
		.unwrap_or_else(|| "nixbox-telemetry.sqlite3".into());
	let listener = tokio::net::TcpListener::bind(&address)
		.await
		.context("binding telemetry receiver")?;
	println!("Telemetry receiver listening on {}", listener.local_addr()?);
	receiver::serve(listener, std::path::PathBuf::from(database), async {
		#[cfg(unix)]
		{
			if let Ok(mut termination) =
				tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
			{
				tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = termination.recv() => {} }
				return;
			}
		}
		let _ = tokio::signal::ctrl_c().await;
	})
	.await
}
