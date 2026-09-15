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
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      mkNixbox =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "nixbox";
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
          ];
          cargoTestFlags = [ "--workspace" ];

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
            description = "TUI package manager for NixOS and Home Manager";
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
          nixbox = mkNixbox pkgs;
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
        nixbox = mkNixbox final;
      };
    };
}
