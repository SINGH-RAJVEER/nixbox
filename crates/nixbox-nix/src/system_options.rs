//! Host-wide NixOS option discovery without evaluating configuration values.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use crate::build::find_in_nix_profiles;
use crate::settings::OptionPath;

const EXPRESSION: &str = include_str!("system_options.nix");
const QUERY_ENV: &str = "NIXBOX_SYSTEM_OPTIONS_QUERY";

/// A configuration being inspected. This identity never changes the install
/// target or the host used by an existing queued operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixosConfiguration {
	pub root: PathBuf,
	pub host: String,
}

/// Schema metadata only. Defaults and current values are deliberately absent.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOption {
	pub path: OptionPath,
	pub description: Option<String>,
	pub type_description: String,
	pub declarations: Vec<String>,
	pub diagnostic: bool,
	pub read_only: bool,
	/// Nested schema fields cannot yet be resolved to concrete instances.
	pub nested: bool,
	pub template: bool,
	/// A failed or recursive submodule expansion remains visible in the index.
	pub children_unavailable: bool,
	pub children_truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemOptionCatalog {
	pub options: Vec<SystemOption>,
}

/// A value can be unavailable, intentionally withheld, or a schema template.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "camelCase")]
pub enum InspectedValue {
	Available(Value),
	Unavailable,
	Sensitive,
	Template,
}

/// Formats bounded previews without exposing the wire format's marker fields.
#[must_use]
pub fn format_preview(value: &Value) -> String {
	let marker = value.get("_nixbox").and_then(Value::as_str);
	match marker {
		Some("function") => "<function>".into(),
		Some("sensitive") => "<withheld>".into(),
		Some("truncated") => "...".into(),
		Some("derivation") => format!(
			"<package {}>",
			value
				.get("name")
				.and_then(Value::as_str)
				.unwrap_or("unknown")
		),
		Some("string") => format!(
			"{:?} ...",
			value
				.get("preview")
				.and_then(Value::as_str)
				.unwrap_or_default()
		),
		Some("list") => {
			let items = value
				.get("items")
				.and_then(Value::as_array)
				.map(|items| items.iter().map(format_preview).collect::<Vec<_>>())
				.unwrap_or_default();
			format!(
				"[ {}{} ]",
				items.join(" "),
				if value.get("truncated") == Some(&Value::Bool(true)) {
					" ..."
				} else {
					""
				}
			)
		}
		Some("attrs") => {
			let items = value
				.get("items")
				.and_then(Value::as_object)
				.map(|items| {
					items
						.iter()
						.map(|(name, value)| {
							format!(
								"{} = {};",
								crate::settings::option_path_display(std::slice::from_ref(name)),
								format_preview(value)
							)
						})
						.collect::<Vec<_>>()
				})
				.unwrap_or_default();
			format!(
				"{{ {}{} }}",
				items.join(" "),
				if value.get("truncated") == Some(&Value::Bool(true)) {
					" ..."
				} else {
					""
				}
			)
		}
		_ => value.to_string(),
	}
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemOptionDetails {
	pub path: OptionPath,
	pub value: InspectedValue,
	pub default: Option<String>,
	pub example: Option<String>,
	pub definitions: Option<Vec<String>>,
}

/// Lists output names without forcing their configurations to evaluate.
pub async fn discover_hosts(root: &Path, limit: Duration) -> Result<Vec<String>> {
	evaluate(serde_json::json!({ "root": root, "mode": "hosts" }), limit).await
}

/// Reads declarations, including disabled services and submodule templates.
pub async fn discover_options(
	configuration: &NixosConfiguration,
	limit: Duration,
) -> Result<SystemOptionCatalog> {
	let mut catalog: SystemOptionCatalog = evaluate(
		serde_json::json!({
			"root": configuration.root,
			"host": configuration.host,
			"mode": "catalog",
		}),
		limit,
	)
	.await?;
	catalog.options.sort_by(|a, b| a.path.cmp(&b.path));
	catalog.options.dedup_by(|a, b| a.path == b.path);
	Ok(catalog)
}

/// Evaluates just the selected option's value and provenance.
pub async fn inspect_option(
	configuration: &NixosConfiguration,
	path: &[String],
	limit: Duration,
) -> Result<SystemOptionDetails> {
	evaluate(
		serde_json::json!({
			"root": configuration.root,
			"host": configuration.host,
			"mode": "inspect",
			"path": path,
		}),
		limit,
	)
	.await
}

async fn evaluate<T: serde::de::DeserializeOwned>(query: Value, limit: Duration) -> Result<T> {
	let nix = find_in_nix_profiles("nix").unwrap_or_else(|| PathBuf::from("nix"));
	let output = timeout(
		limit,
		Command::new(nix)
			.args(["eval", "--impure", "--json", "--expr", EXPRESSION])
			.env(QUERY_ENV, query.to_string())
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.kill_on_drop(true)
			.output(),
	)
	.await
	.context("timed out reading NixOS options")??;
	if !output.status.success() {
		bail!(
			"reading NixOS options failed: {}",
			String::from_utf8_lossy(&output.stderr).trim()
		);
	}
	serde_json::from_slice(&output.stdout).context("decoding NixOS options")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn unavailable_values_are_distinct_from_null_and_templates() {
		let decode = |text| serde_json::from_str::<InspectedValue>(text).unwrap();
		assert_eq!(
			decode(r#"{"status":"available","value":null}"#),
			InspectedValue::Available(Value::Null)
		);
		assert_eq!(
			decode(r#"{"status":"unavailable"}"#),
			InspectedValue::Unavailable
		);
		assert_eq!(decode(r#"{"status":"template"}"#), InspectedValue::Template);
		assert_eq!(
			decode(r#"{"status":"sensitive"}"#),
			InspectedValue::Sensitive
		);
	}

	#[test]
	fn previews_show_values_and_truncation_without_protocol_markers() {
		assert_eq!(
			format_preview(
				&serde_json::json!({"_nixbox":"list", "items":[1, 2], "truncated":true})
			),
			"[ 1 2 ... ]"
		);
		assert_eq!(
			format_preview(
				&serde_json::json!({"_nixbox":"attrs", "items":{"apiToken":{"_nixbox":"sensitive"}}, "truncated":false})
			),
			"{ apiToken = <withheld>; }"
		);
	}

	#[tokio::test]
	#[ignore = "requires Nix and NIXBOX_TEST_NIXPKGS pointing to a nixpkgs store source"]
	async fn live_schema_and_inspection_preserve_lazy_values_and_nested_templates() {
		struct Fixture(PathBuf);
		impl Drop for Fixture {
			fn drop(&mut self) {
				let _ = std::fs::remove_dir_all(&self.0);
			}
		}
		assert!(
			std::env::var_os("NIXBOX_TEST_NIXPKGS").is_some(),
			"set NIXBOX_TEST_NIXPKGS"
		);
		let root = Fixture(
			std::env::temp_dir().join(format!("nixbox-system-options-{}", std::process::id())),
		);
		std::fs::create_dir_all(&root.0).unwrap();
		std::fs::write(
			root.0.join("flake.nix"),
			include_str!("../tests/fixtures/system-options/flake.nix"),
		)
		.unwrap();
		let limit = Duration::from_secs(30);
		let hosts = discover_hosts(&root.0, limit).await.unwrap();
		assert_eq!(hosts, ["broken", "host with \"quotes\"", "workstation"]);
		let configuration = NixosConfiguration {
			root: root.0.clone(),
			host: "workstation".into(),
		};
		let catalog = discover_options(&configuration, limit).await.unwrap();
		let path = |text: &str| text.split('.').map(str::to_string).collect::<Vec<_>>();
		let entry = |text: &str| {
			catalog
				.options
				.iter()
				.find(|option| option.path == path(text))
				.unwrap()
		};
		assert_eq!(
			entry("custom.broken").description.as_deref(),
			Some("This default must not be forced during discovery.")
		);
		assert!(entry("custom.missingDescription").description.is_none());
		assert!(entry("custom.debug").diagnostic);
		assert!(entry("custom.fixed").read_only);
		assert!(entry("users.users.<name>.active").template);
		assert!(entry("services.jobs.*.name").nested);
		assert!(
			catalog
				.options
				.iter()
				.any(|option| option.children_truncated)
		);
		assert!(
			catalog.options.len() < 100,
			"Recursive schema must be bounded"
		);
		for (name, expected) in [
			(
				"custom.enable",
				InspectedValue::Available(Value::Bool(false)),
			),
			(
				"custom.greeting",
				InspectedValue::Available(Value::String("from configuration".into())),
			),
			("custom.nullable", InspectedValue::Available(Value::Null)),
			("custom.broken", InspectedValue::Unavailable),
			("credentials.apiToken", InspectedValue::Sensitive),
			("users.users.<name>.active", InspectedValue::Template),
		] {
			let inspected = inspect_option(&configuration, &path(name), limit)
				.await
				.unwrap();
			assert_eq!(inspected.value, expected, "{name}");
		}
		let inspected = inspect_option(&configuration, &path("custom.greeting"), limit)
			.await
			.unwrap();
		assert_eq!(inspected.default.as_deref(), Some("\"hello\""));
		assert!(!inspected.definitions.unwrap().is_empty());
		let inspected = inspect_option(&configuration, &path("custom.large"), limit)
			.await
			.unwrap();
		let InspectedValue::Available(value) = inspected.value else {
			panic!("list value");
		};
		assert_eq!(value["items"].as_array().unwrap().len(), 20);
		assert_eq!(value["truncated"], true);
		let inspected = inspect_option(&configuration, &path("custom.settings"), limit)
			.await
			.unwrap();
		let InspectedValue::Available(value) = inspected.value else {
			panic!("settings value");
		};
		assert_eq!(value["items"]["apiToken"]["_nixbox"], "sensitive");
		assert_eq!(value["items"]["enabled"], true);
		let configuration = NixosConfiguration {
			host: "host with \"quotes\"".into(),
			..configuration
		};
		let inspected = inspect_option(&configuration, &["custom".into(), "a.b".into()], limit)
			.await
			.unwrap();
		assert_eq!(
			inspected.value,
			InspectedValue::Available(Value::String("a dotted segment".into()))
		);
		assert!(
			inspect_option(&configuration, &path("custom.deleted"), limit)
				.await
				.is_err()
		);
		let broken = NixosConfiguration {
			host: "broken".into(),
			..configuration
		};
		assert!(discover_options(&broken, limit).await.is_err());
	}
}
