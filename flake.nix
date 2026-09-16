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
      # Two packages, because the workspace publishes two binaries from one
      # shared command tree: `nixbox` with the terminal UI, `nixbox-cli`
      # without it. Same subcommands either way.
      mkNixbox =
        {
          pkgs,
          package ? "nixbox",
        }:
        let
          # `nixbox-cli` deliberately installs under its own name so both can
          # sit in one profile without overwriting each other.
          binary = package;
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
          ];
          # A workspace test run unifies features and always enables the UI,
          # so the CLI package is tested on its own to cover the other half.
          cargoTestFlags =
            if package == "nixbox" then
              [ "--workspace" ]
            else
              [
                "--package"
                "nixbox-cli"
                "--package"
                "nixbox-cmd"
              ];

          nativeBuildInputs = [ pkgs.makeWrapper ];

          postInstall = ''
            wrapProgram $out/bin/${binary} \
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
              if package == "nixbox" then
                "TUI package manager for NixOS and Home Manager"
              else
                "Command-line package manager for NixOS and Home Manager";
            homepage = "https://github.com/SINGH-RAJVEER/nix-box";
            license = pkgs.lib.licenses.asl20;
            mainProgram = binary;
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
            package = "nixbox-cli";
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
          package = "nixbox-cli";
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
