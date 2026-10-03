{
  description = "NixBox, a TUI package manager for NixOS and Home Manager";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
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
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      # `nixbox` with the terminal UI and `nixbox-cli` without it share one
      # command tree. `nixbox-gui` is the desktop front-end, Linux only for
      # now, and the only one that needs the graphics stack.
      mkNixbox =
        {
          pkgs,
          package ? "nixbox",
        }:
        let
          # `nixbox-cli` deliberately installs under its own name so both can
          # sit in one profile without overwriting each other.
          binary = package;
          gui = package == "nixbox-gui";
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
        pkgs.rustPlatform.buildRustPackage {
          pname = package;
          version = cargoToml.workspace.package.version;

          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.lock
              ./Cargo.toml
              ./LICENSE
              ./README.md
              ./crates
            ];
          };

          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "--package"
            package
          ] ++ pkgs.lib.optionals gui [
            "--features"
            "native"
          ];
          useNextest = true;
          # A workspace test run unifies features and always enables the UI,
          # so the CLI package is tested on its own to cover the other half.
          # The desktop front-end is excluded so the TUI build never needs the
          # graphics stack.
          cargoTestFlags =
            if package == "nixbox" then
              [
                "--workspace"
                "--exclude"
                "nixbox-gui"
              ]
            else if gui then
              [
                "--package"
                "nixbox-gui"
                "--features"
                "native"
              ]
            else
              [
                "--package"
                "nixbox-cli"
                "--package"
                "nixbox-cmd"
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
            wrapProgram $out/bin/${binary} \
              --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  pkgs.gh
                  pkgs.git
					pkgs.jujutsu
                  pkgs.nix
                ]
              }${pkgs.lib.optionalString gui " --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath guiLibraries}"}
            ${pkgs.lib.optionalString gui ''
              install -Dm644 ${./assets/nixbox-gui.svg} "$out/share/icons/hicolor/scalable/apps/nixbox-gui.svg"
              install -Dm644 ${./assets/nixbox-gui.desktop.in} "$out/share/applications/nixbox-gui.desktop"
              substituteInPlace "$out/share/applications/nixbox-gui.desktop" \
                --replace-fail '@NIXBOX_GUI_EXEC@' "$out/bin/nixbox-gui"
              desktop-file-validate "$out/share/applications/nixbox-gui.desktop"
            ''}
          '';

          meta = {
            description =
              if package == "nixbox" then
                "TUI package manager for NixOS and Home Manager"
              else if gui then
                "Desktop package manager for NixOS and Home Manager"
              else
                "Command-line package manager for NixOS and Home Manager";
            homepage = "https://github.com/SINGH-RAJVEER/nixbox";
            license = pkgs.lib.licenses.asl20;
            mainProgram = binary;
            platforms = if gui then pkgs.lib.platforms.linux else pkgs.lib.platforms.unix;
          };
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        rec {
          nixbox = mkNixbox { inherit pkgs; };
          nixbox-cli = mkNixbox {
            inherit pkgs;
            package = "nixbox-cli";
          };
          default = nixbox;
        }
        // nixpkgs.lib.optionalAttrs (nixpkgs.lib.hasSuffix "-linux" system) {
          nixbox-gui = mkNixbox {
            inherit pkgs;
            package = "nixbox-gui";
          };
        }
      );

      apps = forAllSystems (
        system:
        {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/nixbox";
            meta.description = "Run NixBox";
          };
          nixbox-cli = {
            type = "app";
            program = "${self.packages.${system}.nixbox-cli}/bin/nixbox-cli";
            meta.description = "Run the NixBox command line";
          };
        }
        // nixpkgs.lib.optionalAttrs (nixpkgs.lib.hasSuffix "-linux" system) {
          nixbox-gui = {
            type = "app";
            program = "${self.packages.${system}.nixbox-gui}/bin/nixbox-gui";
            meta.description = "Run the NixBox desktop GUI";
          };
        }
      );

      overlays.default = final: _prev: {
        nixbox = mkNixbox { pkgs = final; };
        nixbox-cli = mkNixbox {
          pkgs = final;
          package = "nixbox-cli";
        };
        nixbox-gui = mkNixbox {
          pkgs = final;
          package = "nixbox-gui";
        };
      };

      # A throwaway VM for exercising NixBox against a real rebuild. Build and
      # boot it with `just vm`. Nothing here is ever applied to the host: the
      # only safe subcommands are `build-vm` and `build-vm-with-bootloader`.
      nixosConfigurations.nixbox-testvm = nixpkgs.lib.nixosSystem {
        system = vmSystem;
        specialArgs = {
          nixbox = self.packages.${vmSystem}.default;
          nixboxCli = self.packages.${vmSystem}.nixbox-cli;
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
          cli = import ./vm/test.nix {
            nixbox = self.packages.${system}.default;
            nixboxCli = self.packages.${system}.nixbox-cli;
            nixpkgsFlake = nixpkgs;
            inherit (pkgs) testers;
          };
        }
      );
    };
}
