use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const HEADER: &str = "# Managed by nixbox. Do not edit by hand.\n# nixbox:generated-inputs ";

#[derive(Serialize, Deserialize)]
struct Input {
	repo: String,
	follows: bool,
}

fn render(inputs: &BTreeMap<String, Input>) -> Result<String> {
	let mut source = format!(
		"{HEADER}{}\n{{ ... }}:\n{{\n\tflake-file.inputs = {{\n",
		serde_json::to_string(inputs)?
	);
	for (name, input) in inputs {
		let name = super::attr_name(name);
		source.push_str(&format!(
			"\t\t{name}.url = {};\n",
			serde_json::to_string(&format!("github:{}", input.repo))?
		));
		if input.follows {
			source.push_str(&format!(
				"\t\t{name}.inputs.nixpkgs.follows = \"nixpkgs\";\n"
			));
		}
	}
	source.push_str("\t};\n}\n");
	Ok(source)
}

/// Edits the module source and regenerates flake.nix through the configured
/// generator. On failure both the generated output and owned source roll back.
pub(super) fn change(
	flake: &Path,
	name: &str,
	value: Option<(&str, bool)>,
) -> Result<Option<bool>> {
	let root = flake.parent().context("flake has no directory")?;
	let Some(discovery) = crate::dendritic::Layout::inspect(root)? else {
		return Ok(None);
	};
	if !discovery.generated_inputs {
		return Ok(None);
	}
	let path = discovery.tree.join("nixbox/inputs.nix");
	let previous = fs::read_to_string(&path).ok();
	let mut inputs: BTreeMap<String, Input> = match previous.as_deref() {
		Some(source) => {
			let json = source
				.strip_prefix(HEADER)
				.and_then(|source| source.lines().next())
				.context("generated input module is not owned by NixBox")?;
			let inputs = serde_json::from_str(json)?;
			anyhow::ensure!(
				render(&inputs)? == source,
				"generated input module has custom edits; move them to another module before editing inputs"
			);
			inputs
		}
		None => BTreeMap::new(),
	};
	match value {
		Some((repo, follows)) => {
			inputs.insert(
				name.into(),
				Input {
					repo: repo.into(),
					follows,
				},
			);
		}
		None => {
			if inputs.remove(name).is_none() {
				return Ok(Some(false));
			}
		}
	}
	let old_flake = fs::read(flake)?;
	let lock = root.join("flake.lock");
	let old_lock = fs::read(&lock).ok();
	fs::create_dir_all(path.parent().context("input module has no parent")?)?;
	fs::write(&path, render(&inputs)?)?;
	let result = (|| {
		let output = std::process::Command::new("nix")
			.current_dir(root)
			.args(["run", &format!("path:{}#write-flake", root.display())])
			.output()
			.context("running flake-file's write-flake app")?;
		anyhow::ensure!(
			output.status.success(),
			"write-flake failed: {}",
			String::from_utf8_lossy(&output.stderr)
		);
		let source = fs::read_to_string(flake)?;
		let actual =
			RootInputs::parse(&source).context("write-flake produced unsupported inputs")?;
		if let Some((repo, _)) = value {
			anyhow::ensure!(
				actual.name_for_repo(repo) == Some(name),
				"write-flake did not generate the declared input {name}"
			);
		}
		Ok(())
	})();
	if let Err(error) = result {
		fs::write(flake, old_flake)?;
		if let Some(source) = previous {
			fs::write(&path, source)?;
		} else {
			fs::remove_file(&path)?;
		}
		if let Some(bytes) = old_lock {
			fs::write(lock, bytes)?;
		} else if lock.exists() {
			fs::remove_file(lock)?;
		}
		return Err(error);
	}
	Ok(Some(true))
}
