# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Set one release version across the Cargo workspace without resolving dependencies."""

import argparse
import datetime
import re
import tomllib
from pathlib import Path


STABLE = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
NIGHTLY = STABLE + r"-nightly\.([0-9]{8})(?:\.(0|[1-9][0-9]*))?"


def release_version(current, *, nightly, version=None, date=None, sequence=None):
	if not re.fullmatch(STABLE, current) and not re.fullmatch(NIGHTLY, current):
		raise ValueError(f"unsupported workspace version: {current}")
	if not nightly:
		if date is not None or sequence is not None:
			raise ValueError("--date and --sequence require --nightly")
		result = version or current.split("-", 1)[0]
		if not re.fullmatch(STABLE, result):
			raise ValueError("stable requires a major.minor.patch version")
		return result
	if version and re.fullmatch(NIGHTLY, version):
		if date is not None or sequence is not None:
			raise ValueError("an explicit nightly version cannot use --date or --sequence")
		stamp = version.split("-nightly.", 1)[1].split(".", 1)[0]
		datetime.datetime.strptime(stamp, "%Y%m%d")
		return version
	base = version or current.split("-", 1)[0]
	if not re.fullmatch(STABLE, base):
		raise ValueError("--version requires a stable base or a complete nightly version")
	# A nightly of the next patch sorts before its eventual stable release.
	if version is None and "-" not in current:
		major, minor, patch = base.split(".")
		base = f"{major}.{minor}.{int(patch) + 1}"
	stamp = date or datetime.datetime.now(datetime.UTC).strftime("%Y%m%d")
	if not re.fullmatch(r"[0-9]{8}", stamp):
		raise ValueError("--date must be YYYYMMDD")
	datetime.datetime.strptime(stamp, "%Y%m%d")
	if sequence is not None and not re.fullmatch(r"0|[1-9][0-9]*", sequence):
		raise ValueError("--sequence must be a nonnegative integer without leading zeros")
	return f"{base}-nightly.{stamp}" + (f".{sequence}" if sequence is not None else "")


def set_version(root, version):
	manifest = root / "Cargo.toml"
	workspace = tomllib.loads(manifest.read_text())["workspace"]
	current = workspace["package"]["version"]
	manifests = [root / member / "Cargo.toml" for member in workspace["members"]]
	names = {tomllib.loads(path.read_text())["package"]["name"] for path in manifests}
	updates = {}
	text = manifest.read_text()
	text, count = re.subn(
		r'(?m)^(\[workspace\.package\]\s*\nversion\s*=\s*")[^"]+("\s*$)',
		lambda match: match[1] + version + match[2], text,
	)
	if count != 1:
		raise ValueError("could not find workspace.package.version")
	updates[manifest] = text
	# Exact prerelease requirements prevent a nightly from selecting stable crates.
	requirement = f"={version}" if "-nightly." in version else version
	for path in manifests:
		text = path.read_text()
		for name in names:
			text = re.sub(
				rf'(?m)^({re.escape(name)}\s*=\s*\{{[^\n]*\bversion\s*=\s*")[^"]+("[^\n]*\}})',
				lambda match: match[1] + requirement + match[2], text,
			)
		updates[path] = text
	lock = root / "Cargo.lock"
	text = lock.read_text()
	for name in names:
		text, count = re.subn(
			rf'(?m)^(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = "){re.escape(current)}("\n)(?!source =)',
			lambda match: match[1] + version + match[2], text,
		)
		if count != 1:
			raise ValueError(f"expected one local lockfile entry for {name} {current}")
	updates[lock] = text
	# Validate all edits before touching the checkout.
	for text in updates.values():
		tomllib.loads(text)
	for path, text in updates.items():
		if path.read_text() != text:
			path.write_text(text)


def main():
	parser = argparse.ArgumentParser(description=__doc__)
	channels = parser.add_mutually_exclusive_group(required=True)
	channels.add_argument("--stable", action="store_true")
	channels.add_argument("--nightly", action="store_true")
	parser.add_argument("--version", help="stable base or complete version for the selected channel")
	parser.add_argument("--date", help="nightly date in UTC, YYYYMMDD")
	parser.add_argument("--sequence", help="nightly build sequence, such as the CI run number")
	args = parser.parse_args()
	root = Path(__file__).resolve().parent.parent
	current = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
	try:
		version = release_version(current, nightly=args.nightly, version=args.version, date=args.date, sequence=args.sequence)
		set_version(root, version)
	except ValueError as error:
		parser.error(str(error))
	print(version)


if __name__ == "__main__":
	main()
