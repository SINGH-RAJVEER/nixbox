{ pkgs, packages }:

let
	packageList = builtins.attrValues (builtins.removeAttrs packages [ "default" ]);
	invalidLimit =
		package: buildJobs: !(builtins.tryEval (package.override { inherit buildJobs; }).preBuild).success;
	testPackage =
		package:
		assert builtins.all (invalidLimit package) [
			0
			(-1)
			"2"
		];
		assert !package.doCheck && !package.cargoArtifacts.doCheck;
		assert pkgs.lib.hasInfix "--test-threads" package.tests.unit.checkPhase;
		pkgs.lib.concatMapStringsSep "\n"
			(
				{ configured, buildJobs }:
				pkgs.lib.concatMapStringsSep "\n"
					(
						derivation:
						pkgs.lib.concatMapStringsSep "\n"
							(
								phase:
								pkgs.lib.concatMapStringsSep "\n"
									(
										cores:
										let
											expected = toString (if buildJobs == null then cores else pkgs.lib.min cores buildJobs);
										in
										''
																	(
																		export NIX_BUILD_CORES=${toString cores}
																		unset CARGO_BUILD_JOBS
																		CARGO_BUILD_JOBS=24
																		${derivation.${phase}}
																		test "$NIX_BUILD_CORES" = ${expected}
																		test "$CARGO_BUILD_JOBS" = ${expected}
																		# Both limits must reach compiler and test subprocesses.
																		bash -c 'test "$NIX_BUILD_CORES" = ${expected} && test "$CARGO_BUILD_JOBS" = ${expected}'
																	)
																''
									)
									[
										1
										2
										24
									]
							)
							[
								"preBuild"
								"preCheck"
							]
					)
					[
						configured
						configured.cargoArtifacts
						configured.tests.unit
					]
			)
			[
				{
					configured = package;
					buildJobs = null;
				}
				{
					configured = package.override { buildJobs = 2; };
					buildJobs = 2;
				}
				{
					configured = package.override { buildJobs = 8; };
					buildJobs = 8;
				}
			];
in
pkgs.runCommand "nixbox-build-jobs" { } ''
		${pkgs.lib.concatMapStringsSep "\n" testPackage packageList}
		touch "$out"
''
