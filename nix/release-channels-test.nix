{ pkgs, packages, apps, overlay, nightlyVersion }:

let
	interfaces = [ "nixbox" "nixbox-tui" "nixbox-cli" ];
	consistent = package:
		let
			stable = packages."${package}-stable";
			nightly = packages."${package}-nightly";
		in
		stable.version == "0.4.5"
		&& nightly.version == nightlyVersion
		&& stable.src != nightly.src
		&& packages.${package}.drvPath == stable.drvPath
		&& overlay.${package}.drvPath == stable.drvPath
		&& overlay."${package}-nightly".drvPath == nightly.drvPath
		&& apps."${package}-stable".program == pkgs.lib.getExe stable
		&& apps."${package}-nightly".program == pkgs.lib.getExe nightly;
in
assert builtins.all consistent interfaces;
assert pkgs.lib.hasInfix "-nightly." nightlyVersion || nightlyVersion == packages.nixbox-stable.version;
assert packages.default.drvPath == packages.nixbox-stable.drvPath;
assert packages.default.meta.mainProgram == "nixbox-gui";
assert apps.default.program == apps.nixbox-stable.program;
pkgs.runCommand "nixbox-release-channels" { } ''
	touch "$out"
''
