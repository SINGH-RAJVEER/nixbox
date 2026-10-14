{ pkgs, packages }:

let
	packageList = builtins.attrValues (builtins.removeAttrs packages [ "default" ]);
	invalidLimit = package: buildJobs:
		!(builtins.tryEval (package.override { inherit buildJobs; }).preBuild).success;
	testPackage = package:
		assert builtins.all (invalidLimit package) [ 0 (-1) "2" ];
		pkgs.lib.concatMapStringsSep "\n" ({ configured, buildJobs }:
			pkgs.lib.concatMapStringsSep "\n" (phase:
				pkgs.lib.concatMapStringsSep "\n" (cores:
					let
						expected = toString (pkgs.lib.min cores buildJobs);
					in
					''
						(
							export NIX_BUILD_CORES=${toString cores}
							unset CARGO_BUILD_JOBS
							CARGO_BUILD_JOBS=24
							${configured.${phase}}
							test "$NIX_BUILD_CORES" = ${expected}
							test "$CARGO_BUILD_JOBS" = ${expected}
							# Both limits must reach compiler and test subprocesses.
							bash -c 'test "$NIX_BUILD_CORES" = ${expected} && test "$CARGO_BUILD_JOBS" = ${expected}'
						)
					''
				) [ 1 2 24 ]
			) [ "preBuild" "preCheck" ]
		) [
			{ configured = package; buildJobs = 2; }
			{ configured = package.override { buildJobs = 8; }; buildJobs = 8; }
		];
in
pkgs.runCommand "nixbox-build-jobs" { } ''
	${pkgs.lib.concatMapStringsSep "\n" testPackage packageList}
	touch "$out"
''
