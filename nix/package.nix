{ crane }:

{
	pkgs,
	package ? "nixbox",
	source,
	buildJobs ? null,
}:

let
	inherit (pkgs) lib;
	craneLib = crane.mkLib pkgs;
	gui = package == "nixbox";
	binary = if gui then "nixbox-gui" else package;
	version =
		(builtins.fromTOML (builtins.readFile (source + "/Cargo.toml"))).workspace.package.version;
	limitBuildJobs =
		lib.optionalString (buildJobs != null) ''
					if (( NIX_BUILD_CORES > ${toString buildJobs} )); then
						export NIX_BUILD_CORES=${toString buildJobs}
					fi
				''
		+ ''
					# Nextest's -j controls execution, not test compilation.
					export CARGO_BUILD_JOBS="$NIX_BUILD_CORES"
				'';
	guiLibraries = with pkgs; [
		vulkan-loader
		wayland
		libxkbcommon
		libx11
		libxcb
	];
	src = import ./source.nix { inherit lib source package; };
	testSrc = import ./source.nix {
		inherit lib source package;
		tests = true;
	};
	cargoExtraArgs = "--locked --package ${package}" + lib.optionalString gui " --features native";
	commonArgs = {
		inherit version cargoExtraArgs;
		pname = package;
		strictDeps = true;
		preBuild = limitBuildJobs;
		preCheck = limitBuildJobs;
		nativeBuildInputs = lib.optionals gui [ pkgs.pkg-config ];
		buildInputs = lib.optionals gui (
			guiLibraries
			++ [
				pkgs.fontconfig
				pkgs.freetype
			]
		);
	};
	# No check/test builds in the dependency derivation used by installations.
	cargoArtifacts = craneLib.buildDepsOnly (
		commonArgs
		// {
			inherit src;
			doCheck = false;
			cargoCheckCommand = "true";
		}
	);
	unitTests = craneLib.cargoNextest (
		commonArgs
		// {
			src = testSrc;
			inherit cargoArtifacts;
			doInstallCargoArtifacts = false;
			cargoNextestExtraArgs = "--test-threads \"$NIX_BUILD_CORES\"";
			cargoExtraArgs =
				if package == "nixbox-tui" then
					"--locked --workspace --exclude nixbox"
				else if gui then
					cargoExtraArgs
				else
					"--locked --package nixbox-cli --package nixbox-cmd";
			nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
				pkgs.git
				pkgs.jujutsu
			];
		}
	);
in
assert lib.assertMsg (
	buildJobs == null || (builtins.isInt buildJobs && buildJobs > 0)
) "NixBox buildJobs must be null or a positive integer";
craneLib.buildPackage (
	commonArgs
	// {
		inherit src cargoArtifacts;
		doCheck = false;
		nativeBuildInputs =
			commonArgs.nativeBuildInputs ++ [ pkgs.makeWrapper ] ++ lib.optional gui pkgs.desktop-file-utils;
		postInstall = ''
					for executable in ${binary}${lib.optionalString (package == "nixbox-cli") " nixbox nb"}; do
						wrapProgram "$out/bin/$executable" \
							--prefix PATH : ${
						lib.makeBinPath [
							pkgs.gh
							pkgs.git
							pkgs.jujutsu
							pkgs.nix
						]
					}${lib.optionalString gui " --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath guiLibraries}"}
					done
				''
		+ lib.optionalString gui ''
					install -Dm644 assets/nixbox.svg "$out/share/icons/hicolor/scalable/apps/nixbox.svg"
					install -Dm644 assets/nixbox.desktop.in "$out/share/applications/nixbox.desktop"
					substituteInPlace "$out/share/applications/nixbox.desktop" \
						--replace-fail '@NIXBOX_EXEC@' "$out/bin/nixbox-gui"
					desktop-file-validate "$out/share/applications/nixbox.desktop"
				'';
		passthru = {
			inherit cargoArtifacts;
			tests.unit = unitTests;
		};
		meta = {
			description =
				if gui then
					"Desktop package manager for NixOS and Home Manager"
				else if package == "nixbox-tui" then
					"Terminal UI package manager for NixOS and Home Manager"
				else
					"Command-line package manager for NixOS and Home Manager";
			homepage = "https://github.com/SINGH-RAJVEER/nixbox";
			license = lib.licenses.asl20;
			mainProgram = binary;
			platforms = if gui then lib.platforms.linux else lib.platforms.unix;
		};
	}
)
