{
	outputs = { self }:
		let
			lib = import (builtins.toPath (builtins.getEnv "NIXBOX_TEST_NIXPKGS") + "/lib");
			recursiveType = lib.types.submodule {
				options.again = lib.mkOption { type = recursiveType; default = { }; };
			};
			configuration = lib.evalModules {
				modules = [
					({ lib, ... }: {
						_file = "${self.outPath}/flake.nix";
						options = {
							hardware.cpu.intel.updateMicrocode = lib.mkEnableOption "Intel microcode";
							hardware.cpu.amd.updateMicrocode = lib.mkEnableOption "AMD microcode";
							hardware.nvidia.open = lib.mkEnableOption "NVIDIA open driver";
							hardware.nvidia.modesetting.enable = lib.mkEnableOption "NVIDIA modesetting";
							hardware.bluetooth.enable = lib.mkEnableOption "Bluetooth";
							services.undervolt.tempBat = lib.mkOption {
								type = lib.types.nullOr lib.types.int;
								default = null;
							};
							custom.enable = lib.mkEnableOption "the fixture service";
							custom.greeting = lib.mkOption {
								type = lib.types.str;
								default = "hello";
								description = "Welcome message for the local service.";
							};
							custom.broken = lib.mkOption {
								type = lib.types.str;
								default = throw "unavailable fixture default";
								description = "This default must not be forced during discovery.";
							};
							custom.missingDescription = lib.mkOption {
								type = lib.types.bool;
								default = false;
								description = throw "unavailable fixture description";
							};
							custom.debug = lib.mkOption {
								type = lib.types.bool;
								default = true;
								visible = false;
							};
							custom.fixed = lib.mkOption {
								type = lib.types.str;
								default = "fixed";
								readOnly = true;
							};
							custom.nullable = lib.mkOption {
								type = lib.types.nullOr lib.types.str;
								default = null;
							};
							custom."a.b" = lib.mkOption {
								type = lib.types.str;
								default = "a dotted segment";
							};
							custom.large = lib.mkOption {
								type = lib.types.listOf lib.types.int;
								default = lib.range 1 100;
							};
							custom.settings = lib.mkOption {
								type = lib.types.attrs;
								default = { enabled = true; apiToken = builtins.abort "nested credential was evaluated"; };
							};
							custom.recursive = lib.mkOption { type = recursiveType; default = { }; };
							users.users = lib.mkOption {
								type = lib.types.attrsOf (lib.types.submodule {
									options.active = lib.mkEnableOption "this user";
								});
								default = { };
							};
							services.jobs = lib.mkOption {
								type = lib.types.listOf (lib.types.submodule {
									options.name = lib.mkOption { type = lib.types.str; };
								});
								default = [ ];
							};
							credentials.apiToken = lib.mkOption {
								type = lib.types.str;
								default = builtins.abort "credential was evaluated";
							};
						};
						config.custom.greeting = "from configuration";
						config.hardware.nvidia.open = false;
					})
				];
			};
			host = { inherit (configuration) options config; pkgs = { inherit lib; }; };
		in {
			nixosConfigurations = {
				workstation = host;
				"host with \"quotes\"" = host;
				broken = throw "host evaluation failed";
			};
		};
}
