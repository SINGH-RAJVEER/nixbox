{
  description = "NixBox, a desktop package manager for NixOS and Home Manager";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
	inputs.stable-source = {
		url = "github:SINGH-RAJVEER/nixbox/v0.4.5";
		flake = false;
	};

  outputs =
    { self, nixpkgs, stable-source }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      linuxSystems = builtins.filter (nixpkgs.lib.hasSuffix "-linux") supportedSystems;
      forAllLinuxSystems = nixpkgs.lib.genAttrs linuxSystems;
      # nixosConfigurations is a flat attribute set, so the test VM is pinned
      # to one system rather than generated per system.
      vmSystem = "x86_64-linux";
      # `nixbox` is the desktop front-end, Linux only for now, and the only
      # one that needs the graphics stack. `nixbox-tui` adds the terminal UI
      # and only `nixbox-cli` includes CLI commands. The interfaces share
      # the engine and install different executables, so each is optional.
		mkNixbox = nixpkgs.lib.makeOverridable (
        {
          pkgs,
          package ? "nixbox",
			source ? ./.,
			buildJobs ? 2,
        }:
        let
			cargoToml = builtins.fromTOML (builtins.readFile (source + "/Cargo.toml"));
          binary = if gui then "nixbox-gui" else package;
          gui = package == "nixbox";
			limitBuildJobs = ''
				if (( NIX_BUILD_CORES > ${toString buildJobs} )); then
					export NIX_BUILD_CORES=${toString buildJobs}
				fi
				# Nextest's -j limits test execution, not test compilation.
				export CARGO_BUILD_JOBS="$NIX_BUILD_CORES"
			'';
          # Loaded at runtime rather than linked, so they go on the library
          # path through the wrapper.
          guiLibraries = [
            pkgs.vulkan-loader
            pkgs.wayland
            pkgs.libxkbcommon
            pkgs.libx11
            pkgs.libxcb
          ];
        in
		assert pkgs.lib.assertMsg (builtins.isInt buildJobs && buildJobs > 0)
			"NixBox buildJobs must be a positive integer";
        pkgs.rustPlatform.buildRustPackage {
          pname = package;
          version = cargoToml.workspace.package.version;
			preBuild = limitBuildJobs;
			preCheck = limitBuildJobs;

			src = if !builtins.isPath source then source else pkgs.lib.fileset.toSource {
				root = source;
            fileset = pkgs.lib.fileset.unions [
					(source + "/Cargo.lock")
					(source + "/Cargo.toml")
					(source + "/LICENSE")
					(source + "/README.md")
					(source + "/crates")
            ];
          };

			cargoLock.lockFile = source + "/Cargo.lock";
          cargoBuildFlags = [
            "--package"
            package
          ] ++ pkgs.lib.optionals gui [
            "--features"
            "native"
          ];
          useNextest = true;
          # The terminal UI's run covers every crate but the desktop one, so
          # the TUI build never needs the graphics stack.
          cargoTestFlags =
            if gui then
              [
                "--package"
                "nixbox"
                "--features"
                "native"
              ]
            else if package == "nixbox-tui" then
              [
                "--workspace"
                "--exclude"
                "nixbox"
              ]
            else
              [
                "--package"
                "nixbox-cli"
                "--package"
                "nixbox-cmd"
              ];

			nativeCheckInputs = [
				pkgs.git
				pkgs.jujutsu
			];

          nativeBuildInputs = [ pkgs.makeWrapper ] ++ pkgs.lib.optionals gui [
            pkgs.pkg-config
            pkgs.desktop-file-utils
          ];
          buildInputs = pkgs.lib.optionals gui (
            guiLibraries
            ++ [
              pkgs.fontconfig
              pkgs.freetype
            ]
          );

          postInstall = ''
				for executable in ${binary}${pkgs.lib.optionalString (package == "nixbox-cli") " nixbox nb"}; do
					wrapProgram $out/bin/$executable \
						--prefix PATH : ${
							pkgs.lib.makeBinPath [
								pkgs.gh
								pkgs.git
								pkgs.jujutsu
								pkgs.nix
							]
						}${pkgs.lib.optionalString gui " --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath guiLibraries}"}
				done
            ${pkgs.lib.optionalString gui ''
					install -Dm644 ${source + "/assets/nixbox.svg"} "$out/share/icons/hicolor/scalable/apps/nixbox.svg"
					install -Dm644 ${source + "/assets/nixbox.desktop.in"} "$out/share/applications/nixbox.desktop"
              substituteInPlace "$out/share/applications/nixbox.desktop" \
                --replace-fail '@NIXBOX_EXEC@' "$out/bin/nixbox-gui"
              desktop-file-validate "$out/share/applications/nixbox.desktop"
            ''}
          '';

          meta = {
            description =
              if gui then
                "Desktop package manager for NixOS and Home Manager"
              else if package == "nixbox-tui" then
                "Terminal UI package manager for NixOS and Home Manager"
              else
                "Command-line package manager for NixOS and Home Manager";
            homepage = "https://github.com/SINGH-RAJVEER/nixbox";
            license = pkgs.lib.licenses.asl20;
            mainProgram = binary;
            platforms = if gui then pkgs.lib.platforms.linux else pkgs.lib.platforms.unix;
          };
        }
		);
      isLinux = nixpkgs.lib.hasSuffix "-linux";
		stableSource = stable-source.outPath;
		channelPackages = pkgs: system:
			let
				interfaces = [ "nixbox-tui" "nixbox-cli" ] ++ nixpkgs.lib.optional (isLinux system) "nixbox";
			in
			nixpkgs.lib.foldl' (outputs: package:
				let stablePackage = mkNixbox { inherit pkgs package; source = stableSource; };
				in
				outputs // {
					"${package}-stable" = stablePackage;
					"${package}-nightly" = mkNixbox { inherit pkgs package; };
					"${package}" = stablePackage;
				}
			) { } interfaces;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
			channelPackages pkgs system // {
				# The stable desktop GUI on Linux, stable TUI on macOS.
				default = self.packages.${system}.${if isLinux system then "nixbox-stable" else "nixbox-tui-stable"};
			}
      );

      apps = forAllSystems (
        system:
        let
          app = package: description: {
            type = "app";
            program = nixpkgs.lib.getExe self.packages.${system}.${package};
            meta = { inherit description; };
          };
        in
        {
          default = app "default" "Run NixBox";
				nixbox-tui-stable = app "nixbox-tui-stable" "Run the stable NixBox terminal UI";
				nixbox-tui-nightly = app "nixbox-tui-nightly" "Run the nightly NixBox terminal UI";
				nixbox-cli-stable = app "nixbox-cli-stable" "Run the stable NixBox command line";
				nixbox-cli-nightly = app "nixbox-cli-nightly" "Run the nightly NixBox command line";
          nixbox-tui = app "nixbox-tui" "Run the NixBox terminal UI";
          nixbox-cli = app "nixbox-cli" "Run the NixBox command line";
			nb = {
				type = "app";
				program = nixpkgs.lib.getExe' self.packages.${system}.nixbox-cli "nb";
				meta.description = "Run the separately installed NixBox CLI";
			};
        }
        // nixpkgs.lib.optionalAttrs (isLinux system) {
          nixbox = app "nixbox" "Run the NixBox desktop GUI";
				nixbox-stable = app "nixbox-stable" "Run the stable NixBox desktop GUI";
				nixbox-nightly = app "nixbox-nightly" "Run the nightly NixBox desktop GUI";
        }
      );

		overlays.default = final: _prev: channelPackages final final.stdenv.hostPlatform.system;

      # A throwaway VM for exercising NixBox against a real rebuild. Build and
      # boot it with `just vm`. Nothing here is ever applied to the host: the
      # only safe subcommands are `build-vm` and `build-vm-with-bootloader`.
      # It is headless, so it gets the terminal UI rather than the default.
      nixosConfigurations.nixbox-testvm = nixpkgs.lib.nixosSystem {
        system = vmSystem;
        specialArgs = {
				nixboxTui = self.packages.${vmSystem}.nixbox-tui-nightly;
				nixboxCli = self.packages.${vmSystem}.nixbox-cli-nightly;
          nixpkgsFlake = nixpkgs;
        };
        modules = [ ./vm/host.nix ];
      };

      checks = forAllLinuxSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
				release-channels = import ./nix/release-channels-test.nix {
					inherit pkgs;
					packages = self.packages.${system};
					apps = self.apps.${system};
					overlay = self.overlays.default pkgs { };
					nightlyVersion = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
				};
			build-jobs = import ./nix/build-jobs-test.nix {
				inherit pkgs;
				packages = self.packages.${system};
			};
          cli = import ./vm/test.nix {
					nixboxTui = self.packages.${system}.nixbox-tui-nightly;
					nixboxCli = self.packages.${system}.nixbox-cli-nightly;
            nixpkgsFlake = nixpkgs;
            inherit (pkgs) testers;
          };
        }
      );
    };
}
