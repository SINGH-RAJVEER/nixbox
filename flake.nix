{
	description = "NixBox, a desktop package manager for NixOS and Home Manager";

	inputs.crane.url = "github:ipetkov/crane";
	nixConfig = {
		extra-substituters = [ "https://nixbox.cachix.org" ];
		extra-trusted-public-keys = [ "nixbox.cachix.org-1:aAlStGW7Q18EIEW91Ql9Iebuu/MsyXC/WKg/TlOxn0A=" ];
	};
	inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
	inputs.stable-source = {
		url = "github:SINGH-RAJVEER/nixbox/v0.4.5";
		flake = false;
	};

	outputs =
		{
			self,
			nixpkgs,
			stable-source,
			crane,
		}:
		let
			supportedSystems = [
				"x86_64-linux"
				"aarch64-linux"
				"aarch64-darwin"
			];
			forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
			# nixosConfigurations is a flat attribute set, so the test VM is pinned
			# to one system rather than generated per system.
			vmSystem = "x86_64-linux";
			# `nixbox` is the desktop front-end, Linux only for now, and the only
			# one that needs the graphics stack. `nixbox-tui` adds the terminal UI
			# and only `nixbox-cli` includes CLI commands. The interfaces share
			# the engine and install different executables, so each is optional.
			mkNixbox = nixpkgs.lib.makeOverridable (
				{
					source ? ./.,
					...
				}@args:
				(import ./nix/package.nix { inherit crane; }) (args // { inherit source; })
			);
			isLinux = nixpkgs.lib.hasSuffix "-linux";
			stableSource = stable-source.outPath;
			channelPackages =
				pkgs: system:
				let
					interfaces = [
						"nixbox-tui"
						"nixbox-cli"
					]
					++ nixpkgs.lib.optional (isLinux system) "nixbox";
				in
				nixpkgs.lib.foldl' (
					outputs: package:
					let
						stablePackage = mkNixbox {
							inherit pkgs package;
							source = stableSource;
						};
					in
					outputs
					// {
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
				channelPackages pkgs system
				// {
					# The stable desktop GUI on Linux, stable TUI on macOS.
					default =
						self.packages.${system}.${if isLinux system then "nixbox-stable" else "nixbox-tui-stable"};
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

			checks = forAllSystems (
				system:
				let
					pkgs = nixpkgs.legacyPackages.${system};
				in
				{
					# Tests are separate from installation derivations and reuse compiled dependencies.
					nixbox-tui-stable = self.packages.${system}.nixbox-tui-stable.tests.unit;
					nixbox-tui-nightly = self.packages.${system}.nixbox-tui-nightly.tests.unit;
					nixbox-cli-stable = self.packages.${system}.nixbox-cli-stable.tests.unit;
					nixbox-cli-nightly = self.packages.${system}.nixbox-cli-nightly.tests.unit;
					build-jobs = import ./nix/build-jobs-test.nix {
						inherit pkgs;
						packages = self.packages.${system};
					};
				}
				// nixpkgs.lib.optionalAttrs (isLinux system) {
					nixbox-stable = self.packages.${system}.nixbox-stable.tests.unit;
					nixbox-nightly = self.packages.${system}.nixbox-nightly.tests.unit;
					release-channels = import ./nix/release-channels-test.nix {
						inherit pkgs;
						packages = self.packages.${system};
						apps = self.apps.${system};
						overlay = self.overlays.default pkgs { };
						nightlyVersion = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
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
