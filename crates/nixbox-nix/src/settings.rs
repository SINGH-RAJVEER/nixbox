//! The generated module that sets package options, such as
//! `programs.git.userName`.
//!
//! Each option is one `path = value;` line between the settings markers.
//! Only values nixbox can render are accepted when loading, so every line it
//! reads back is one it could have written.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::wiring::attr_name;

const SETTINGS_HEADER: &str = "# Managed by nixbox. Do not edit by hand.";
const START_MARKER: &str = "# nixbox:settings:start";
const END_MARKER: &str = "# nixbox:settings:end";

/// A module option's attribute path, such as `["programs", "git", "enable"]`.
pub type OptionPath = Vec<String>;

/// An option value nixbox can write and read back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettingValue {
	Null,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	StrList(Vec<String>),
}

impl SettingValue {
	/// The value as a Nix expression.
	#[must_use]
	pub fn to_nix(&self) -> String {
		match self {
			SettingValue::Null => "null".into(),
			SettingValue::Bool(value) => value.to_string(),
			SettingValue::Int(value) => value.to_string(),
			SettingValue::Float(value) => float_literal(*value),
			SettingValue::Str(value) => string_literal(value),
			SettingValue::StrList(items) if items.is_empty() => "[ ]".into(),
			SettingValue::StrList(items) => {
				let items: Vec<String> = items.iter().map(|item| string_literal(item)).collect();
				format!("[ {} ]", items.join(" "))
			}
		}
	}
}

/// Every option nixbox sets for one target.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsManifest {
	pub values: BTreeMap<OptionPath, SettingValue>,
}

impl SettingsManifest {
	/// Sets `path` to `value`, or drops it when `value` is `None`. Returns
	/// whether anything changed.
	pub fn apply(&mut self, path: &[String], value: Option<&SettingValue>) -> bool {
		match value {
			Some(value) => self.values.insert(path.to_vec(), value.clone()).as_ref() != Some(value),
			None => self.values.remove(path).is_some(),
		}
	}

	/// Every option set under `namespace`, such as `programs.git`.
	pub fn under<'a>(
		&'a self,
		namespace: &'a [String],
	) -> impl Iterator<Item = (&'a OptionPath, &'a SettingValue)> {
		self.values
			.iter()
			.filter(move |(path, _)| path.starts_with(namespace))
	}
}

/// Renders `path` as a Nix attribute path, quoting segments that need it.
#[must_use]
pub fn option_path_display(path: &[String]) -> String {
	path.iter()
		.map(|segment| attr_name(segment))
		.collect::<Vec<_>>()
		.join(".")
}

pub struct SettingsFile {
	path: PathBuf,
}

impl SettingsFile {
	pub fn new(path: impl Into<PathBuf>) -> Self {
		Self { path: path.into() }
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn load(&self) -> Result<SettingsManifest> {
		if !self.path.exists() {
			return Ok(SettingsManifest::default());
		}
		let raw = fs::read_to_string(&self.path)
			.with_context(|| format!("reading {}", self.path.display()))?;
		Ok(parse(&raw))
	}

	pub fn write(&self, manifest: &SettingsManifest) -> Result<()> {
		if let Some(parent) = self.path.parent() {
			fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
		}
		fs::write(&self.path, render(manifest))
			.with_context(|| format!("writing {}", self.path.display()))
	}
}

fn render(manifest: &SettingsManifest) -> String {
	let mut body = String::new();
	for (path, value) in &manifest.values {
		body.push_str(&format!(
			"\t{} = {};\n",
			option_path_display(path),
			value.to_nix()
		));
	}
	format!("{SETTINGS_HEADER}\n{{ ... }}:\n{{\n\t{START_MARKER}\n{body}\t{END_MARKER}\n}}\n")
}

fn parse(raw: &str) -> SettingsManifest {
	let mut manifest = SettingsManifest::default();
	let mut in_block = false;
	for line in raw.lines() {
		let line = line.trim();
		match line {
			START_MARKER => in_block = true,
			END_MARKER => in_block = false,
			_ if in_block => {
				if let Some((path, value)) = parse_line(line) {
					manifest.values.insert(path, value);
				}
			}
			_ => {}
		}
	}
	manifest
}

/// Parses `path = value;`, rejecting anything [`render`] would not produce.
fn parse_line(line: &str) -> Option<(OptionPath, SettingValue)> {
	let (path, rest) = parse_path(line)?;
	let rest = rest.strip_prefix(" = ")?.strip_suffix(';')?;
	Some((path, parse_value(rest)?))
}

fn parse_path(mut input: &str) -> Option<(OptionPath, &str)> {
	let mut path = Vec::new();
	loop {
		let (segment, rest) = if input.starts_with('"') {
			parse_string(input)?
		} else {
			let end = input
				.find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '\'')))
				.unwrap_or(input.len());
			if end == 0 {
				return None;
			}
			(input[..end].to_string(), &input[end..])
		};
		path.push(segment);
		match rest.strip_prefix('.') {
			Some(next) => input = next,
			None => return Some((path, rest)),
		}
	}
}

fn parse_value(input: &str) -> Option<SettingValue> {
	match input {
		"null" => return Some(SettingValue::Null),
		"true" => return Some(SettingValue::Bool(true)),
		"false" => return Some(SettingValue::Bool(false)),
		"[ ]" => return Some(SettingValue::StrList(Vec::new())),
		_ => {}
	}
	if input.starts_with('"') {
		let (value, rest) = parse_string(input)?;
		return rest.is_empty().then_some(SettingValue::Str(value));
	}
	if let Some(inner) = input
		.strip_prefix("[ ")
		.and_then(|inner| inner.strip_suffix(" ]"))
	{
		let mut items = Vec::new();
		let mut rest = inner;
		loop {
			let (item, after) = parse_string(rest)?;
			items.push(item);
			if after.is_empty() {
				return Some(SettingValue::StrList(items));
			}
			rest = after.strip_prefix(' ')?;
		}
	}
	let digits = input.strip_prefix('-').unwrap_or(input);
	if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) {
		return input.parse().ok().map(SettingValue::Int);
	}
	if digits.contains('.')
		&& digits
			.chars()
			.all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | 'e' | 'E' | '-' | '+'))
	{
		return input.parse().ok().map(SettingValue::Float);
	}
	None
}

/// Reads one double-quoted string from the start of `input`, returning it
/// unescaped along with the text after the closing quote.
fn parse_string(input: &str) -> Option<(String, &str)> {
	let mut chars = input.strip_prefix('"')?.char_indices();
	let mut out = String::new();
	while let Some((index, ch)) = chars.next() {
		match ch {
			'"' => return Some((out, &input[index + 2..])),
			'\\' => match chars.next()?.1 {
				'n' => out.push('\n'),
				'r' => out.push('\r'),
				't' => out.push('\t'),
				other => out.push(other),
			},
			'$' if input[index + 2..].starts_with('{') => return None,
			other => out.push(other),
		}
	}
	None
}

fn string_literal(value: &str) -> String {
	let mut out = String::with_capacity(value.len() + 2);
	out.push('"');
	let mut chars = value.chars().peekable();
	while let Some(ch) = chars.next() {
		match ch {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			'$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
			other => out.push(other),
		}
	}
	out.push('"');
	out
}

/// Nix needs a `.` in every float literal, so `1` and `1e20` would not
/// read back as floats.
fn float_literal(value: f64) -> String {
	let debug = format!("{value:?}");
	if debug.contains('.') {
		debug
	} else if let Some((mantissa, exponent)) = debug.split_once('e') {
		format!("{mantissa}.0e{exponent}")
	} else {
		format!("{debug}.0")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn path(raw: &str) -> OptionPath {
		raw.split('.').map(str::to_string).collect()
	}

	#[test]
	fn every_value_kind_round_trips() {
		let mut manifest = SettingsManifest::default();
		let values = [
			("programs.git.enable", SettingValue::Bool(true)),
			("programs.git.lfs.enable", SettingValue::Bool(false)),
			("programs.git.signing.key", SettingValue::Null),
			("programs.bat.config.tabs", SettingValue::Int(-4)),
			("programs.foot.settings.scale", SettingValue::Float(1.0)),
			("programs.foot.settings.big", SettingValue::Float(1e20)),
			(
				"programs.git.userName",
				SettingValue::Str("Raj \"R\" \\ ${x} $y\nnext".into()),
			),
			(
				"programs.git.ignores",
				SettingValue::StrList(vec!["*.swp".into(), ".direnv".into()]),
			),
			("programs.git.attributes", SettingValue::StrList(Vec::new())),
		];
		for (key, value) in values {
			manifest.values.insert(path(key), value);
		}
		manifest.values.insert(
			vec!["programs".into(), "with".into(), "a b".into()],
			SettingValue::Int(1),
		);

		let rendered = render(&manifest);
		assert!(rendered.contains("\tprograms.\"with\".\"a b\" = 1;\n"));
		assert!(rendered.contains("\\${x} $y"));
		assert_eq!(parse(&rendered), manifest);
	}

	#[test]
	fn lines_outside_the_block_or_not_rendered_by_nixbox_are_ignored() {
		let raw = "\
# Managed by nixbox. Do not edit by hand.
{ ... }:
{
	programs.git.enable = true;
	# nixbox:settings:start
	programs.fish.enable = true;
	programs.git.extraConfig = { core.pager = \"less\"; };
	programs.git.userName = \"a${b}\";
	programs.git.package = pkgs.git;
	# nixbox:settings:end
}
";
		let manifest = parse(raw);
		assert_eq!(manifest.values.len(), 1);
		assert_eq!(
			manifest.values.get(&path("programs.fish.enable")),
			Some(&SettingValue::Bool(true))
		);
	}

	#[test]
	fn apply_sets_replaces_and_unsets() {
		let mut manifest = SettingsManifest::default();
		let key = path("programs.git.enable");
		assert!(manifest.apply(&key, Some(&SettingValue::Bool(true))));
		assert!(!manifest.apply(&key, Some(&SettingValue::Bool(true))));
		assert!(manifest.apply(&key, Some(&SettingValue::Bool(false))));
		assert!(manifest.apply(&key, None));
		assert!(!manifest.apply(&key, None));
		assert!(manifest.values.is_empty());
	}
}
