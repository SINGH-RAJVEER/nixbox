//! Fixed templates over the append-only application journal. No clock, sorting,
//! grouping, reversal filtering, or option values enter the rendered message.

use std::path::PathBuf;

use nixbox_config::Target;
use nixbox_nix::manifest::FlakeOutput;
use nixbox_nix::settings::option_path_display;

use super::JournalEntry;
use crate::Op;

pub(super) fn render(entries: &[JournalEntry]) -> String {
	if entries.is_empty() {
		return "nixbox: update configuration".into();
	}
	let nixos = entries
		.iter()
		.any(|entry| entry.target == Some(Target::NixosSystem));
	let home = entries
		.iter()
		.any(|entry| entry.target == Some(Target::HomeManager));
	let configuration = match (nixos, home) {
		(true, true) => "NixOS and Home Manager configurations",
		(true, false) => "NixOS configuration",
		(false, true) => "Home Manager configuration",
		(false, false) => "configuration",
	};
	let subject = match entries {
		[entry] => entry
			.summary
			.as_ref()
			.map(|summary| format!("nixbox: {}", clean(summary)))
			.filter(|subject| subject.chars().count() <= 72)
			.unwrap_or_else(|| format!("nixbox: update {configuration}")),
		_ => format!(
			"nixbox: update {configuration} ({} operations)",
			entries.len()
		),
	};
	let operations = entries
		.iter()
		.enumerate()
		.map(|(index, entry)| format!("{}. {}", index.saturating_add(1), clean(&entry.operation)))
		.collect::<Vec<_>>()
		.join("\n");
	format!("{subject}\n\nOperations in application order:\n{operations}")
}

pub(super) fn entries(op: &Op, changed_files: &[PathBuf]) -> Vec<JournalEntry> {
	let target = op.scope();
	let scope = match target {
		Target::NixosSystem => "NixOS",
		Target::HomeManager => "Home Manager",
	};
	let mut descriptions = match op {
		Op::Install { hit, .. } => vec![(
			format!("install {} for {scope}", hit.attr),
			if hit.version.is_empty() {
				format!("Install package {} for {scope}.", hit.attr)
			} else {
				format!(
					"Install package {} version {} for {scope}.",
					hit.attr, hit.version
				)
			},
		)],
		Op::Uninstall { name, .. } => vec![(
			format!("remove {name} from {scope}"),
			format!("Remove package {name} from {scope}."),
		)],
		Op::InstallFlake { repo, module, .. } => vec![(
			format!("import {repo}#{module} for {scope}"),
			format!("Import module {module} from flake {repo} for {scope}."),
		)],
		Op::InstallFlakePackage { repo, package, .. } => vec![(
			format!("install {repo}#{package} for {scope}"),
			format!("Install package output {package} from flake {repo} for {scope}."),
		)],
		Op::UninstallFlake { repo, .. } => vec![(
			format!("remove flake {repo} from {scope}"),
			format!("Remove flake {repo} from {scope}."),
		)],
		Op::UninstallFlakeOutput { input, output, .. } => {
			let (kind, path) = match output {
				FlakeOutput::Package(path) => ("package", path),
				FlakeOutput::Module(path) => ("module", path),
			};
			vec![(
				format!("remove {input}#{path} from {scope}"),
				format!("Remove {kind} output {path} from flake input {input} for {scope}."),
			)]
		}
		Op::Migrate { names, .. } => names
			.iter()
			.map(|name| {
				(
					format!("manage declared package {name} for {scope}"),
					format!("Move declared package {name} into NixBox management for {scope}."),
				)
			})
			.collect(),
		Op::MigrateFlakePackage { input, package, .. } => vec![(
			format!("manage declared package {input}#{package} for {scope}"),
			format!(
				"Move declared package output {package} from flake input {input} into NixBox management for {scope}."
			),
		)],
		Op::SetOptions { changes, .. } => changes
			.iter()
			.map(|change| {
				let path = option_path_display(&change.path);
				let (action, summary) = if change.value.is_some() {
					("Set", "set")
				} else {
					("Unset", "unset")
				};
				(
					format!("{summary} {path} for {scope}"),
					format!("{action} option {path} for {scope}."),
				)
			})
			.collect(),
	};
	if descriptions.is_empty() {
		descriptions.push((
			format!("rewrite managed configuration for {scope}"),
			format!("Rewrite managed configuration for {scope}."),
		));
	}
	// Never serialize `Op`: it contains the option values we intentionally omit.
	descriptions
		.into_iter()
		.map(|(summary, operation)| JournalEntry {
			operation: clean(&operation),
			changed_files: changed_files.to_vec(),
			summary: Some(clean(&summary)),
			target: Some(target),
		})
		.collect()
}

fn clean(value: &str) -> String {
	value
		.chars()
		.map(|character| {
			if character.is_control() {
				character.escape_default().to_string()
			} else {
				character.to_string()
			}
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::OptionChange;
	use nixbox_nix::settings::SettingValue;

	#[test]
	fn option_batches_keep_order_and_redact_values_in_storage_and_messages() {
		let operations = entries(
			&Op::SetOptions {
				changes: vec![
					OptionChange {
						path: vec!["z".into()],
						value: Some(SettingValue::Str("private value".into())),
					},
					OptionChange {
						path: vec!["a".into()],
						value: None,
					},
					OptionChange {
						path: vec!["z".into()],
						value: Some(SettingValue::Bool(true)),
					},
				],
				scope: Target::NixosSystem,
			},
			&[],
		);
		assert_eq!(
			render(&operations),
			"nixbox: update NixOS configuration (3 operations)\n\nOperations in application order:\n1. Set option z for NixOS.\n2. Unset option a for NixOS.\n3. Set option z for NixOS."
		);
		assert!(
			!serde_json::to_string(&operations)
				.unwrap()
				.contains("private value")
		);
	}

	#[test]
	fn long_identifiers_stay_complete_in_the_body_and_controls_are_escaped() {
		let name = format!("{}\n\x1b[31m", "package".repeat(20));
		let operations = entries(
			&Op::Uninstall {
				name: name.clone(),
				scope: Target::HomeManager,
			},
			&[],
		);
		let message = render(&operations);
		assert!(message.starts_with("nixbox: update Home Manager configuration\n"));
		assert!(message.contains(&clean(&name)));
		assert!(!message.contains('\x1b'));
	}

	#[test]
	fn migration_batches_list_every_package_in_the_given_order() {
		let operations = entries(
			&Op::Migrate {
				names: vec!["z".into(), "a".into()],
				scope: Target::HomeManager,
			},
			&[],
		);
		assert_eq!(
			operations
				.iter()
				.map(|entry| entry.operation.as_str())
				.collect::<Vec<_>>(),
			[
				"Move declared package z into NixBox management for Home Manager.",
				"Move declared package a into NixBox management for Home Manager.",
			]
		);
	}
}
