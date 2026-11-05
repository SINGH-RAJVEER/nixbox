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
use crate::options::{OptionEntry, OptionKind, Provenance, RawType};
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
	/// A non-default definition outside nixpkgs, or unknown when its
	/// provenance cannot be read. This never evaluates the option's value.
	#[serde(default)]
	pub configured: Option<bool>,
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

impl SystemOption {
	/// A concrete option with a detected configuration definition. Inherited
	/// metadata on a nested schema describes its parent, not an actual instance.
	#[must_use]
	pub fn is_configured(&self) -> bool {
		self.configured == Some(true) && !self.nested && !self.template
	}
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

/// One option of a group being edited.
#[derive(Debug, Clone)]
pub struct EditableOption {
	pub details: SystemOptionDetails,
	/// The option as the package options editor sees it: its kind and who
	/// defines it. `value` is set only when the value is plain enough to edit.
	pub entry: OptionEntry,
}

#[derive(Deserialize)]
struct RawGroup {
	root: String,
	options: Vec<RawGroupOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGroupOption {
	#[serde(flatten)]
	details: SystemOptionDetails,
	#[serde(rename = "type")]
	kind: Option<RawType>,
	priority: Option<i64>,
	#[serde(default)]
	read_only: bool,
}

impl RawGroup {
	fn into_options(self, settings_file: &str) -> Vec<EditableOption> {
		self.options
			.into_iter()
			.map(|raw| {
				let (kind, type_description) = match &raw.kind {
					Some(raw_type) => (raw_type.kind(), raw_type.description.clone()),
					None => (OptionKind::Other("unknown".into()), "unknown".into()),
				};
				let files = raw.details.definitions.clone().unwrap_or_default();
				let provenance =
					Provenance::read(&self.root, settings_file, &files, raw.priority, &kind);
				let value = match &raw.details.value {
					InspectedValue::Available(value) => plain_value(value),
					_ => None,
				};
				let entry = OptionEntry {
					path: raw.details.path.clone(),
					kind,
					type_description,
					description: None,
					default: raw.details.default.clone(),
					example: raw.details.example.clone(),
					value,
					defined_in: provenance.defined_in,
					set_by_nixbox: provenance.set_by_nixbox,
					set_by_module: provenance.set_by_module,
					read_only: raw.read_only,
				};
				EditableOption {
					details: raw.details,
					entry,
				}
			})
			.collect()
	}
}

/// A previewed value as plain JSON, when nothing was cut from it: a whole
/// list rather than its first items, and no withheld or function markers.
fn plain_value(value: &Value) -> Option<Value> {
	match value {
		Value::Object(map) => match map.get("_nixbox").and_then(Value::as_str) {
			Some("list") if map.get("truncated") != Some(&Value::Bool(true)) => map
				.get("items")?
				.as_array()?
				.iter()
				.map(plain_value)
				.collect::<Option<Vec<_>>>()
				.map(Value::Array),
			_ => None,
		},
		Value::Array(_) => None,
		value => Some(value.clone()),
	}
}

/// Evaluates every option in `paths` at once, with what deciding whether
/// nixbox may write each one needs. `settings_file` is the name of nixbox's
/// settings module, whose own definitions do not lock an option. Paths no
/// longer declared are left out.
pub async fn inspect_group(
	configuration: &NixosConfiguration,
	paths: &[OptionPath],
	settings_file: &str,
	limit: Duration,
) -> Result<Vec<EditableOption>> {
	let group: RawGroup = evaluate(
		serde_json::json!({
			"root": configuration.root,
			"host": configuration.host,
			"mode": "group",
			"paths": paths,
		}),
		limit,
	)
	.await?;
	Ok(group.into_options(settings_file))
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
	fn group_entries_decode_into_kinds_values_and_ownership() {
		let bool_type = serde_json::json!({"name": "bool", "description": "boolean", "elem": null, "values": null});
		let group: RawGroup = serde_json::from_value(serde_json::json!({
			"root": "/nix/store/abc-source",
			"options": [
				{
					"path": ["services", "openssh", "enable"],
					"value": {"status": "available", "value": true},
					"default": "false",
					"example": null,
					"definitions": ["/nix/store/abc-source/hosts/configuration.nix"],
					"type": bool_type,
					"priority": 100,
					"readOnly": false
				},
				{
					"path": ["networking", "nameservers"],
					"value": {"status": "available", "value": {"_nixbox": "list", "items": ["1.1.1.1"], "truncated": false}},
					"default": null,
					"example": null,
					"definitions": ["/nix/store/abc-source/nixbox-system-settings.nix"],
					"type": {"name": "listOf", "description": "list of string", "elem": {"name": "str", "description": "string", "elem": null, "values": null}, "values": null},
					"priority": 100
				},
				{
					"path": ["services", "openssh", "ports"],
					"value": {"status": "available", "value": {"_nixbox": "list", "items": [22], "truncated": true}},
					"default": null,
					"example": null,
					"definitions": [],
					"type": {"name": "listOf", "description": "list of 16 bit unsigned integer", "elem": null, "values": null},
					"priority": 1500
				},
				{
					"path": ["users", "users", "me", "hashedPassword"],
					"value": {"status": "sensitive"},
					"default": null,
					"example": null,
					"definitions": null,
					"type": null,
					"priority": null,
					"readOnly": true
				}
			]
		}))
		.unwrap();

		let options = group.into_options("nixbox-system-settings.nix");

		let enable = &options[0].entry;
		assert_eq!(enable.kind, OptionKind::Bool);
		assert_eq!(enable.value, Some(Value::Bool(true)));
		assert_eq!(enable.defined_in, ["hosts/configuration.nix"]);
		assert_eq!(enable.default.as_deref(), Some("false"));
		let nameservers = &options[1].entry;
		assert_eq!(nameservers.kind, OptionKind::StrList);
		assert!(nameservers.set_by_nixbox && nameservers.defined_in.is_empty());
		assert_eq!(nameservers.value, Some(serde_json::json!(["1.1.1.1"])));
		// A cut-off preview is never offered as the value to edit.
		assert_eq!(options[2].entry.value, None);
		let password = &options[3];
		assert_eq!(password.details.value, InspectedValue::Sensitive);
		assert!(password.entry.read_only);
		assert!(!password.entry.kind.is_editable());
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
		std::fs::write(
			root.0.join("configuration.nix"),
			include_str!("../tests/fixtures/system-options/configuration.nix"),
		)
		.unwrap();
		std::fs::write(
			root.0.join("hardware-configuration.nix"),
			include_str!("../tests/fixtures/system-options/hardware-configuration.nix"),
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
		assert_eq!(entry("custom.greeting").configured, Some(true));
		assert_eq!(entry("custom.enable").configured, Some(false));
		assert_eq!(entry("custom.generated").configured, Some(false));
		assert_eq!(entry("credentials.apiToken").configured, Some(false));
		assert!(entry("credentials.password").is_configured());
		assert!(entry("custom.imported").is_configured());
		assert!(entry("hardware.bluetooth.enable").is_configured());
		assert!(entry("users.users").is_configured());
		assert!(entry("services.jobs").is_configured());
		assert_eq!(entry("hardware.nvidia.open").configured, Some(true));
		assert_eq!(
			entry("hardware.nvidia.modesetting.enable").configured,
			Some(false)
		);
		assert!(entry("custom.fixed").read_only);
		assert!(entry("users.users.<name>.active").template);
		assert!(!entry("users.users.<name>.active").is_configured());
		assert!(!entry("services.jobs.*.name").is_configured());
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
			("credentials.password", InspectedValue::Sensitive),
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
		let group = inspect_group(
			&configuration,
			&[
				path("custom.enable"),
				path("custom.greeting"),
				path("custom.deleted"),
			],
			"nixbox-system-settings.nix",
			limit,
		)
		.await
		.unwrap();
		assert_eq!(group.len(), 2, "Undeclared paths are left out");
		assert_eq!(group[0].entry.kind, OptionKind::Bool);
		assert_eq!(group[0].entry.value, Some(Value::Bool(false)));
		// A default declared in the local flake has that flake as its source too.
		assert_eq!(group[0].entry.defined_in, ["flake.nix"]);
		assert_eq!(group[1].entry.kind, OptionKind::Str);
		assert_eq!(group[1].entry.defined_in, ["flake.nix"]);
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
