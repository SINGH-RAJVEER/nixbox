use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use nixbox_config::Config;
use nixbox_core::telemetry;

use crate::cli::GlobalArgs;

#[derive(Subcommand, Debug)]
pub enum Action {
	/// Print saved consent, effective status, recipient, and disclosure. No network request.
	Status,
	/// Print a synthetic example report. No data is collected and nothing is sent.
	Preview,
}

pub fn run(action: &Action, global: &GlobalArgs) -> Result<ExitCode> {
	match action {
		Action::Preview => println!("{}", telemetry::preview()?),
		Action::Status => {
			let consent = Config::load_or_default()?.telemetry;
			if global.json {
				println!(
					"{}",
					serde_json::to_string_pretty(&serde_json::json!({
						"saved_enabled": consent.enabled,
						"effective_status": telemetry::status(consent),
						"recipient": telemetry::RECIPIENT,
						"disclosure": telemetry::DISCLOSURE,
					}))?
				);
			} else {
				println!(
					"Saved consent: {}\nCollection: {}\nRecipient: {}\n\n{}",
					consent.enabled,
					telemetry::status_label(consent),
					telemetry::RECIPIENT.unwrap_or("not configured"),
					telemetry::DISCLOSURE
				);
			}
		}
	}
	Ok(ExitCode::SUCCESS)
}
