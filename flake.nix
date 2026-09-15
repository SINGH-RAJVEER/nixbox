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
      # `tui = false` builds the same binary without the terminal UI: every
      # subcommand still works, the dependency tree drops by roughly half, and
      # `nixbox` with no subcommand prints help instead of opening a UI.
      mkNixbox =
        {
          pkgs,
          tui ? true,
        }:
        pkgs.rustPlatform.buildRustPackage {
          pname = if tui then "nixbox" else "nixbox-cli";
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
            "nixbox"
          ]
          ++ pkgs.lib.optionals (!tui) [ "--no-default-features" ];
          cargoTestFlags =
            if tui then
              [ "--workspace" ]
            else
              [
                "--package"
                "nixbox"
                "--no-default-features"
              ];

          nativeBuildInputs = [ pkgs.makeWrapper ];

          postInstall = ''
            wrapProgram $out/bin/nixbox \
              --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  pkgs.gh
                  pkgs.git
                  pkgs.nix
                ]
              }
          '';

          meta = {
            description =
              if tui then
                "TUI package manager for NixOS and Home Manager"
              else
                "Command-line package manager for NixOS and Home Manager";
            homepage = "https://github.com/SINGH-RAJVEER/nix-box";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "nixbox";
            platforms = pkgs.lib.platforms.unix;
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
            tui = false;
          };
          default = nixbox;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/nixbox";
          meta.description = "Run NixBox";
        };
      });

      overlays.default = final: _prev: {
        nixbox = mkNixbox { pkgs = final; };
        nixbox-cli = mkNixbox {
          pkgs = final;
          tui = false;
        };
      };

      # A throwaway VM for exercising NixBox against a real rebuild. Build and
      # boot it with `just vm`. Nothing here is ever applied to the host: the
      # only safe subcommands are `build-vm` and `build-vm-with-bootloader`.
      nixosConfigurations.nixbox-testvm = nixpkgs.lib.nixosSystem {
        system = vmSystem;
        specialArgs = {
          nixbox = self.packages.${vmSystem}.default;
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
            nixpkgsFlake = nixpkgs;
            inherit (pkgs) testers;
          };
        }
      );
    };
}
