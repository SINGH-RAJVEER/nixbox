"""Check Nix cache invalidation against real package derivations without compiling."""

import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
	root = Path(__file__).resolve().parent.parent
	with tempfile.TemporaryDirectory(prefix="nixbox-cache-check-") as directory:
		source = Path(directory)
		for name in ("Cargo.toml", "Cargo.lock", "crates", "assets"):
			original = root / name
			if original.is_dir():
				shutil.copytree(original, source / name)
			else:
				shutil.copy2(original, source / name)

		def snapshot():
			expression = """p: let package = p.override { source = %s; }; in {
				build = package.drvPath;
				dependencies = package.cargoArtifacts.drvPath;
				tests = package.tests.unit.drvPath;
			}""" % json.dumps(str(source))
			return json.loads(subprocess.check_output([
				"nix", "eval", "--accept-flake-config", "--impure", "--json",
				".#packages.x86_64-linux.nixbox-cli-nightly", "--apply", expression,
			], cwd=root, text=True))

		def append(relative, text):
			with (source / relative).open("a") as file:
				file.write(text)

		baseline = snapshot()
		(source / "README.md").write_text("Documentation edit\n")
		assert snapshot() == baseline, "Documentation invalidated compiled outputs"
		append("crates/nixbox/src/ui/build.rs", "\n// GUI-only edit\n")
		assert snapshot() == baseline, "GUI implementation invalidated the CLI"
		append("crates/nixbox-core/tests/fixtures/nix-log.jsonl", "\n")
		fixture = snapshot()
		assert fixture["build"] == baseline["build"], "Test fixture invalidated the installation"
		assert fixture["tests"] != baseline["tests"], "Test fixture did not invalidate its check"
		append("crates/nixbox-core/src/engine.rs", "\n// Application edit\n")
		application = snapshot()
		assert application["build"] != fixture["build"], "Application edit did not invalidate the binary"
		assert application["dependencies"] == fixture["dependencies"], "Application edit invalidated compiled dependencies"
		manifest = source / "Cargo.toml"
		manifest.write_text(manifest.read_text().replace(
			'features = ["derive"]', 'features = ["derive", "rc"]', 1,
		))
		assert snapshot()["dependencies"] != application["dependencies"], "Dependency feature change did not invalidate dependencies"
	print("Nix cache invalidation checks passed")


if __name__ == "__main__":
	main()
