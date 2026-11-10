import shutil
import tempfile
import tomllib
import unittest
from pathlib import Path

from version import release_version, set_version


class ReleaseVersionTests(unittest.TestCase):
	def test_channel_transitions(self):
		self.assertEqual(release_version("0.4.5", nightly=True, date="20261025"), "0.4.6-nightly.20261025")
		self.assertEqual(release_version("0.4.6", nightly=True, date="20261025", sequence="123"), "0.4.7-nightly.20261025.123")
		self.assertEqual(release_version("0.4.7-nightly.20261024", nightly=True, date="20261025"), "0.4.7-nightly.20261025")
		self.assertEqual(release_version("0.4.7-nightly.20261025", nightly=False), "0.4.7")

	def test_explicit_versions(self):
		for nightly, version in [(False, "1.0.0"), (True, "1.0.0-nightly.20261025.123")]:
			self.assertEqual(release_version("0.4.6", nightly=nightly, version=version), version)

	def test_invalid_channel_versions(self):
		for options in [
			{"nightly": False, "version": "0.4.7-nightly.20261025"},
			{"nightly": False, "date": "20261025"},
			{"nightly": True, "date": "20260230"},
			{"nightly": True, "date": "2026-10-25"},
			{"nightly": True, "sequence": "01"},
			{"nightly": True, "version": "0.4.7-nightly.20260230"},
			{"nightly": True, "version": "01.4.7"},
		]:
			with self.subTest(options=options), self.assertRaises(ValueError):
				release_version("0.4.6", **options)

	def test_workspace_round_trip(self):
		source = Path(__file__).resolve().parent.parent
		workspace = tomllib.loads((source / "Cargo.toml").read_text())["workspace"]
		with tempfile.TemporaryDirectory() as directory:
			root = Path(directory)
			for file in ["Cargo.toml", "Cargo.lock"] + [f"{member}/Cargo.toml" for member in workspace["members"]]:
				target = root / file
				target.parent.mkdir(parents=True, exist_ok=True)
				shutil.copyfile(source / file, target)
			names = {tomllib.loads((root / member / "Cargo.toml").read_text())["package"]["name"] for member in workspace["members"]}
			original_lock = tomllib.loads((root / "Cargo.lock").read_text())
			for version in ["0.4.7-nightly.20261025.123", "0.4.7"]:
				set_version(root, version)
				self.assertEqual(tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"], version)
				for member in workspace["members"]:
					manifest = tomllib.loads((root / member / "Cargo.toml").read_text())
					for section in ["dependencies", "dev-dependencies", "build-dependencies"]:
						for name, dependency in manifest.get(section, {}).items():
							if name in names:
								self.assertEqual(dependency["version"], f"={version}" if "nightly" in version else version)
				lock = tomllib.loads((root / "Cargo.lock").read_text())
				self.assertEqual([package for package in lock["package"] if package["name"] not in names], [package for package in original_lock["package"] if package["name"] not in names])
				self.assertEqual({package["version"] for package in lock["package"] if package["name"] in names}, {version})
				before = (root / "Cargo.lock").read_bytes()
				set_version(root, version)
				self.assertEqual((root / "Cargo.lock").read_bytes(), before)


if __name__ == "__main__":
	unittest.main()
