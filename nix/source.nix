{
	lib,
	source,
	package,
	tests ? false,
}:

let
	commonCrates = [
		"nixbox-config"
		"nixbox-nix"
		"nixbox-core"
	];
	selectedCrates =
		commonCrates
		++ (
			if package == "nixbox" then
				[ "nixbox" ]
			else if package == "nixbox-tui" then
				[ "nixbox-tui" ]
				++ lib.optionals tests [
					"nixbox-cli"
					"nixbox-cmd"
				]
			else
				[
					"nixbox-cli"
					"nixbox-cmd"
				]
		);
	root = toString source + "/";
in
lib.cleanSourceWith {
	src = source;
	name = "nixbox-source";
	filter =
		path: type:
		let
			relative = lib.removePrefix root (toString path);
			parts = lib.splitString "/" relative;
			crate = builtins.elemAt parts 1;
			selected = builtins.elem crate selectedCrates;
		in
		if relative == "Cargo.toml" || relative == "Cargo.lock" then
			true
		else if relative == "assets" || lib.hasPrefix "assets/" relative then
			package == "nixbox"
		else if relative == "crates" then
			true
		else if lib.hasPrefix "crates/" relative then
			# Keep every manifest and target entry point so Cargo can discover the
			# whole workspace, without copying unrelated interface implementations.
			if type == "directory" then
				selected
				|| builtins.length parts == 2
				|| builtins.length parts == 3 && builtins.elemAt parts 2 == "src"
			else
				builtins.baseNameOf path == "Cargo.toml"
				|| (selected && !lib.hasSuffix ".md" relative && (tests || !lib.hasInfix "/tests/" relative))
				|| builtins.match "crates/[^/]+/src/(lib|main|nixbox|nb)\\.rs" relative != null
		else
			false;
}
