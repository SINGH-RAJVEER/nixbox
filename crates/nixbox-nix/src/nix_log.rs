//! Nix's structured log, `--log-format internal-json`: activities that start
//! and stop, results reported against them, and plain messages. Each comes
//! on its own stderr line, prefixed with `@nix `.

use serde_json::Value;

/// Marks a structured line on nix's stderr.
pub const PREFIX: &str = "@nix ";

/// Arguments that switch a nix command, or a wrapper that forwards them such
/// as `nixos-rebuild` and `home-manager`, to the structured log. `-v` adds the
/// files evaluation reads, which is the only sign of how far it has got.
pub const STRUCTURED_ARGS: [&str; 3] = ["--log-format", "internal-json", "-v"];

/// The level nix prints at by default; anything above it is verbose detail.
pub const INFO: u8 = 3;

/// What an activity is doing, from nix's `ActivityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
	Unknown,
	CopyPath,
	FileTransfer,
	Realise,
	CopyPaths,
	Builds,
	Build,
	OptimiseStore,
	VerifyPaths,
	Substitute,
	QueryPathInfo,
	PostBuildHook,
	BuildWaiting,
	FetchTree,
	Other(u64),
}

impl Activity {
	#[must_use]
	pub const fn from_code(code: u64) -> Self {
		match code {
			0 => Self::Unknown,
			100 => Self::CopyPath,
			101 => Self::FileTransfer,
			102 => Self::Realise,
			103 => Self::CopyPaths,
			104 => Self::Builds,
			105 => Self::Build,
			106 => Self::OptimiseStore,
			107 => Self::VerifyPaths,
			108 => Self::Substitute,
			109 => Self::QueryPathInfo,
			110 => Self::PostBuildHook,
			111 => Self::BuildWaiting,
			112 => Self::FetchTree,
			code => Self::Other(code),
		}
	}
}

/// What a result reports, from nix's `ResultType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
	FileLinked,
	BuildLogLine,
	UntrustedPath,
	CorruptedPath,
	SetPhase,
	/// `[done, expected, running, failed]`.
	Progress,
	/// `[activity type, expected]`.
	SetExpected,
	PostBuildLogLine,
	FetchStatus,
	Other(u64),
}

impl Report {
	#[must_use]
	pub const fn from_code(code: u64) -> Self {
		match code {
			100 => Self::FileLinked,
			101 => Self::BuildLogLine,
			102 => Self::UntrustedPath,
			103 => Self::CorruptedPath,
			104 => Self::SetPhase,
			105 => Self::Progress,
			106 => Self::SetExpected,
			107 => Self::PostBuildLogLine,
			108 => Self::FetchStatus,
			code => Self::Other(code),
		}
	}
}

/// One field of an activity or result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
	Int(u64),
	Str(String),
}

/// One structured line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NixLog {
	Start {
		id: u64,
		parent: u64,
		level: u8,
		activity: Activity,
		text: String,
		fields: Vec<Field>,
	},
	Stop {
		id: u64,
	},
	Result {
		id: u64,
		report: Report,
		fields: Vec<Field>,
	},
	Message {
		level: u8,
		text: String,
	},
}

impl NixLog {
	/// Reads one stderr line, or `None` when it is not a structured one.
	#[must_use]
	pub fn parse(line: &str) -> Option<Self> {
		let json: Value = serde_json::from_str(line.strip_prefix(PREFIX)?).ok()?;
		let int = |key: &str| json.get(key).and_then(Value::as_u64);
		let text = |key: &str| {
			json.get(key)
				.and_then(Value::as_str)
				.map(strip_ansi)
				.unwrap_or_default()
		};
		let level = || int("level").map_or(0, |level| u8::try_from(level).unwrap_or(u8::MAX));
		let fields = || {
			json.get("fields")
				.and_then(Value::as_array)
				.map(|fields| {
					fields
						.iter()
						.map(|field| match field {
							Value::Number(number) => Field::Int(number.as_u64().unwrap_or(0)),
							Value::String(text) => Field::Str(strip_ansi(text)),
							other => Field::Str(other.to_string()),
						})
						.collect()
				})
				.unwrap_or_default()
		};
		match json.get("action")?.as_str()? {
			"start" => Some(Self::Start {
				id: int("id")?,
				parent: int("parent").unwrap_or(0),
				level: level(),
				activity: Activity::from_code(int("type").unwrap_or(0)),
				text: text("text"),
				fields: fields(),
			}),
			"stop" => Some(Self::Stop { id: int("id")? }),
			"result" => Some(Self::Result {
				id: int("id")?,
				report: Report::from_code(int("type")?),
				fields: fields(),
			}),
			"msg" => Some(Self::Message {
				level: level(),
				text: text("msg"),
			}),
			_ => None,
		}
	}

	/// The lines nix's plain log format prints for this at the default
	/// verbosity: messages, and what an activity is starting to do.
	#[must_use]
	pub fn plain_lines(&self) -> Vec<String> {
		match self {
			Self::Message { level, text } if *level <= INFO => {
				text.lines().map(str::to_string).collect()
			}
			Self::Start { level, text, .. } if *level <= INFO && !text.is_empty() => {
				vec![format!("{text}...")]
			}
			_ => Vec::new(),
		}
	}
}

impl Field {
	#[must_use]
	pub const fn int(&self) -> Option<u64> {
		match self {
			Self::Int(value) => Some(*value),
			Self::Str(_) => None,
		}
	}

	#[must_use]
	pub fn str(&self) -> Option<&str> {
		match self {
			Self::Str(text) => Some(text),
			Self::Int(_) => None,
		}
	}
}

/// The package a store path or derivation names: `hello-2.12` for
/// `/nix/store/<hash>-hello-2.12.drv`.
#[must_use]
pub fn store_name(path: &str) -> &str {
	let base = path.rsplit('/').next().unwrap_or(path);
	let base = base.strip_suffix(".drv").unwrap_or(base);
	match base.split_once('-') {
		Some((hash, name)) if hash.len() == 32 => name,
		_ => base,
	}
}

/// Removes terminal escape sequences, which nix keeps in messages even
/// when its output is not a terminal.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut chars = text.chars().peekable();
	while let Some(ch) = chars.next() {
		if ch != '\u{1b}' {
			out.push(ch);
			continue;
		}
		match chars.next() {
			// CSI: parameters, then one final byte in `@`..=`~`.
			Some('[') => {
				for ch in chars.by_ref() {
					if ('@'..='~').contains(&ch) {
						break;
					}
				}
			}
			// OSC: ends at BEL or ESC `\`.
			Some(']') => {
				while let Some(ch) = chars.next() {
					if ch == '\u{7}' {
						break;
					}
					if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
						chars.next();
						break;
					}
				}
			}
			_ => {}
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_every_action() {
		assert_eq!(
			NixLog::parse(
				r#"@nix {"action":"start","fields":["/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-probe.drv","",1,1],"id":7,"level":3,"parent":0,"text":"building '/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-probe.drv'","type":105}"#
			),
			Some(NixLog::Start {
				id: 7,
				parent: 0,
				level: 3,
				activity: Activity::Build,
				text: "building '/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-probe.drv'".into(),
				fields: vec![
					Field::Str("/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-probe.drv".into()),
					Field::Str(String::new()),
					Field::Int(1),
					Field::Int(1),
				],
			})
		);
		assert_eq!(
			NixLog::parse(r#"@nix {"action":"result","fields":[1,2,0,0],"id":8,"type":105}"#),
			Some(NixLog::Result {
				id: 8,
				report: Report::Progress,
				fields: vec![Field::Int(1), Field::Int(2), Field::Int(0), Field::Int(0)],
			})
		);
		assert_eq!(
			NixLog::parse(r#"@nix {"action":"stop","id":8}"#),
			Some(NixLog::Stop { id: 8 })
		);
		assert_eq!(
			NixLog::parse(
				"@nix {\"action\":\"msg\",\"level\":0,\"msg\":\"\\u001b[31;1merror:\\u001b[0m oops\"}"
			),
			Some(NixLog::Message {
				level: 0,
				text: "error: oops".into(),
			})
		);
	}

	#[test]
	fn plain_lines_and_garbage_are_not_structured() {
		assert_eq!(NixLog::parse("activating the configuration..."), None);
		assert_eq!(NixLog::parse("@nix not json"), None);
		assert_eq!(NixLog::parse(r#"@nix {"action":"dance"}"#), None);
	}

	#[test]
	fn plain_lines_follow_the_default_verbosity() {
		let start = |level, text: &str| NixLog::Start {
			id: 1,
			parent: 0,
			level,
			activity: Activity::CopyPath,
			text: text.into(),
			fields: Vec::new(),
		};
		assert_eq!(
			start(3, "copying path 'x'").plain_lines(),
			["copying path 'x'..."]
		);
		assert!(start(4, "downloading 'x'").plain_lines().is_empty());
		assert!(start(0, "").plain_lines().is_empty());
		let message = NixLog::Message {
			level: 0,
			text: "error: builder failed\n  last line".into(),
		};
		assert_eq!(
			message.plain_lines(),
			["error: builder failed", "  last line"]
		);
	}

	#[test]
	fn store_names_drop_the_hash_and_extension() {
		assert_eq!(
			store_name("/nix/store/z91qddwj7wi5cy096nqchdfy7a83gpjs-hello-2.12.drv"),
			"hello-2.12"
		);
		assert_eq!(store_name("not-a-store-path"), "not-a-store-path");
	}

	#[test]
	fn escapes_are_stripped() {
		assert_eq!(strip_ansi("\u{1b}[1;35mbold\u{1b}[0m text"), "bold text");
		assert_eq!(
			strip_ansi("\u{1b}]8;;https://x\u{1b}\\link\u{1b}]8;;\u{7}"),
			"link"
		);
	}
}
