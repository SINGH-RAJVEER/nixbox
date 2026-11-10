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
	workspace = (builtins.fromTOML (builtins.readFile (source + "/Cargo.toml"))).workspace;
	inherit (workspace.package) version;
	crates = map builtins.baseNameOf workspace.members;
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
	# Dependency artifacts must not depend on the workspace version, which CI
	# changes on every nightly: that would miss the binary cache on every run.
	# Crane reads manifests at evaluation time, so dropping the version fields
	# and rewriting the lockfile here keeps the version out of the derivation.
	# Cargo treats a package without a version as 0.0.0.
	dependencyLockContents =
		builtins.replaceStrings
			(map (crate: "name = \"${crate}\"\nversion = \"${version}\"") crates)
			(map (crate: "name = \"${crate}\"\nversion = \"0.0.0\"") crates)
			(builtins.unsafeDiscardStringContext (builtins.readFile (source + "/Cargo.lock")));
	dependencyLock = pkgs.writeText "Cargo.lock" dependencyLockContents;
	isWorkspaceVersion =
		path:
		let
			length = builtins.length path;
		in
		length >= 2
		&& lib.last path == "version"
		&& (
			path == [
				"package"
				"version"
			]
			|| path == [
				"workspace"
				"package"
				"version"
			]
			|| builtins.elem (builtins.elemAt path (length - 2)) crates
		);
	# The terminal interfaces share one derivation, so the source must not vary
	# between them. Only the manifests and entry points matter here.
	dependencySrc = lib.cleanSourceWith {
		src =
			if gui then
				src
			else
				import ./source.nix {
					inherit lib source;
					package = "nixbox-tui";
					tests = true;
				};
		filter = path: _: builtins.baseNameOf path != "Cargo.lock";
	};
	# No check/test builds in the dependency derivation used by installations.
	cargoArtifacts = craneLib.buildDepsOnly (
		commonArgs
		// {
			pname = if gui then package else "nixbox-terminal";
			version = "0.0.0";
			dummySrc = craneLib.mkDummySrc {
				src = dependencySrc;
				cargoLock = dependencyLock;
				cleanCargoTomlFilter = path: craneLib.filters.cargoTomlDefault path && !isWorkspaceVersion path;
			};
			cargoVendorDir = craneLib.vendorCargoDeps { cargoLockContents = dependencyLockContents; };
			doCheck = false;
			cargoCheckCommand = "true";
		}
		// lib.optionalAttrs (!gui) {
			# Separate builds, because features unify across packages built
			# together: `--package nixbox-cli` alone would then miss the artifacts.
			cargoExtraArgs = "--locked";
			buildPhaseCargoCommand = ''
				cargoWithProfile build --locked --package nixbox-tui
				cargoWithProfile build --locked --package nixbox-cli
			'';
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
