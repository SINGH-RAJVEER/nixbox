//! The fixture follows ~/.config/nixos-dendritic's host/desktop split.

use nixbox_config::Target;
use nixbox_nix::{FlakeOutput, ManagedFlakeFile, PackageMode, dendritic::Layout};

use crate::{Engine, ImportState, Op, SilentReporter};

fn fixture(dir: &crate::tests::TempConfigDir) {
	let root = dir.config().config_root();
	std::fs::create_dir(root.join("modules")).unwrap();
	for (path, source) in [
		(
			"flake.nix",
			include_str!("../../nixbox-nix/tests/fixtures/dendritic/flake.nix"),
		),
		(
			"modules/host.nix",
			include_str!("../../nixbox-nix/tests/fixtures/dendritic/modules/host.nix"),
		),
		(
			"modules/desktop.nix",
			include_str!("../../nixbox-nix/tests/fixtures/dendritic/modules/desktop.nix"),
		),
	] {
		std::fs::write(root.join(path), source).unwrap();
	}
}

#[tokio::test]
async fn standalone_home_outputs_use_their_own_class_and_activation() {
	let dir = crate::tests::temp_dir("dendritic-standalone");
	fixture(&dir);
	std::fs::write(
		dir.config().config_root().join("modules/standalone.nix"),
		r#"
{ config, inputs, ... }: {
	flake.homeConfigurations."alice@work" = inputs.home-manager.lib.homeManagerConfiguration {
		modules = [ config.flake.modules.homeManager.work ];
	};
	flake.modules.homeManager.work = { home.username = "alice"; };
}
"#,
	)
	.unwrap();
	let mut config = dir.config();
	config.home_configuration = Some("alice@work".into());
	let mut engine = Engine::with_config(config).unwrap();
	assert_eq!(engine.config.home_user.as_deref(), Some("alice"));
	assert_eq!(engine.config.target, Target::HomeManager);
	engine
		.apply(
			&Op::Install {
				hit: crate::tests::hit("hello"),
				scope: Target::HomeManager,
			},
			&mut SilentReporter,
		)
		.unwrap();
	assert!(
		engine
			.config
			.managed_file_for(Target::HomeManager)
			.starts_with(engine.config.config_root().join("modules/nixbox/homes"))
	);
	assert_eq!(
		engine.import_state(Target::HomeManager),
		ImportState::Imported
	);
	let source =
		std::fs::read_to_string(engine.config.managed_file_for(Target::HomeManager)).unwrap();
	assert!(source.contains("flake.modules.homeManager.work.imports"));
	assert!(!source.contains("flake.modules.nixos"));
	assert!(engine.write_manifest(Target::NixosSystem).is_err());
	assert!(
		Engine::with_config(engine.config.clone())
			.unwrap()
			.home_manifest
			.packages
			.contains("hello")
	);
	let command = crate::rebuild::resolve_for(&engine.config, Target::HomeManager)
		.await
		.unwrap();
	assert!(command.args.iter().any(|arg| arg.contains("alice@work")));
	assert!(
		crate::profile::command_for(&engine.config, Target::HomeManager)
			.await
			.is_err()
	);
}

fn multiple_targets(dir: &crate::tests::TempConfigDir) {
	fixture(dir);
	let path = dir.config().config_root().join("modules/host.nix");
	let source = std::fs::read_to_string(&path).unwrap();
	let source = source.replace(
		"nixosConfigurations.nixos",
		"nixosConfigurations.workstation",
	);
	std::fs::write(path, source).unwrap();
	std::fs::write(
		dir.config().config_root().join("modules/server.nix"),
		r#"
{ config, inputs, ... }: {
	flake.nixosConfigurations.server = inputs.nixpkgs.lib.nixosSystem {
		modules = [ config.flake.modules.nixos.server ];
	};
	flake.modules.nixos.server = {
		home-manager.users.alice = config.flake.modules.homeManager.desktop;
		home-manager.users.bob = config.flake.modules.homeManager.desktop;
	};
}
"#,
	)
	.unwrap();
}

#[tokio::test]
async fn selected_hosts_and_users_have_independent_native_files_and_rebuilds() {
	let dir = crate::tests::temp_dir("dendritic-targets");
	multiple_targets(&dir);
	let root = dir.config().config_root();
	let original = std::fs::read(root.join("flake.nix")).unwrap();
	assert!(Engine::with_config(dir.config()).is_err());
	let mut config = dir.config();
	config.nixos_host = Some("workstation".into());
	let mut workstation = Engine::with_config(config.clone()).unwrap();
	workstation
		.apply(
			&Op::Install {
				hit: crate::tests::hit("hello"),
				scope: Target::NixosSystem,
			},
			&mut SilentReporter,
		)
		.unwrap();
	let first = workstation.config.managed_file_for(Target::NixosSystem);
	let bytes = std::fs::read(&first).unwrap();
	assert!(first.starts_with(root.join("modules/nixbox/hosts")));
	config.nixos_host = Some("server".into());
	config.home_user = None;
	let mut server = Engine::with_config(config.clone()).unwrap();
	assert!(server.nixos_manifest.packages.is_empty());
	assert!(
		server
			.apply(
				&Op::Install {
					hit: crate::tests::hit("cowsay"),
					scope: Target::HomeManager
				},
				&mut SilentReporter
			)
			.is_err()
	);
	assert!(!server.config.managed_file_for(Target::HomeManager).exists());
	for user in ["alice", "bob"] {
		config.home_user = Some(user.into());
		let mut engine = Engine::with_config(config.clone()).unwrap();
		assert!(engine.home_manifest.packages.is_empty());
		engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit(user),
					scope: Target::HomeManager,
				},
				&mut SilentReporter,
			)
			.unwrap();
		assert!(engine.config.managed_file_for(Target::HomeManager).exists());
		let command = crate::rebuild::resolve_for(&engine.config, Target::HomeManager)
			.await
			.unwrap();
		assert!(command.args.iter().any(|arg| arg.contains("#server")));
		assert!(!command.args.iter().any(|arg| arg.contains("#workstation")));
	}
	assert_eq!(std::fs::read(first).unwrap(), bytes);
	assert_eq!(std::fs::read(root.join("flake.nix")).unwrap(), original);
	assert!(
		Engine::with_config(workstation.config.clone())
			.unwrap()
			.nixos_manifest
			.packages
			.contains("hello")
	);
	workstation.config.nixos_host = Some("server".into());
	assert!(workstation.write_manifest(Target::NixosSystem).is_err());
}

#[test]
fn recovery_is_bound_to_the_configuration_host_and_user() {
	let dir = crate::tests::temp_dir("dendritic-recovery");
	multiple_targets(&dir);
	let mut config = dir.config();
	config.nixos_host = Some("server".into());
	config.home_user = Some("alice".into());
	let config = Engine::resolve_config(config).unwrap();
	let saved = crate::PersistedState {
		context: Some(config.identity()),
		pending_queue: vec![Op::Install {
			hit: crate::tests::hit("hello"),
			scope: Target::HomeManager,
		}],
		..Default::default()
	};
	assert!(saved.ensure_context(&config).is_ok());
	let mut other = config.clone();
	other.home_user = Some("bob".into());
	assert!(saved.ensure_context(&other).is_err());
	saved.save().unwrap();
	let state_path = crate::state_path().unwrap();
	let original = std::fs::read(&state_path).unwrap();
	let (mut session, _events) = crate::Session::new(Engine::with_config(other).unwrap());
	assert!(matches!(
		session.enqueue(Op::Install {
			hit: crate::tests::hit("cowsay"),
			scope: Target::HomeManager
		}),
		crate::Enqueued::NotWritten
	));
	session.persist();
	assert!(matches!(session.restore(), crate::Restored::Nothing));
	assert!(session.queue.is_empty());
	assert!(!session.is_building());
	assert!(
		session
			.last_error
			.as_deref()
			.unwrap()
			.contains("another configuration")
	);
	assert_eq!(std::fs::read(state_path).unwrap(), original);
	other = config.clone();
	other.nixos_host = Some("workstation".into());
	assert!(saved.ensure_context(&other).is_err());
	let mut another_root = saved.clone();
	another_root.context.as_mut().unwrap().root = config.config_root().join("another");
	assert!(another_root.ensure_context(&config).is_err());
	let legacy = crate::PersistedState {
		context: None,
		..saved
	};
	assert!(legacy.ensure_context(&config).is_err());
}

#[test]
fn existing_root_flake_parts_stay_at_their_original_paths() {
	let dir = crate::tests::temp_dir("dendritic-root-compatibility");
	fixture(&dir);
	let engine = Engine::with_config(dir.config()).unwrap();
	let root = engine.config.config_root();
	for scope in [Target::NixosSystem, Target::HomeManager] {
		engine.write_manifest(scope).unwrap();
		let native = engine.config.managed_file_for(scope);
		let source = std::fs::read_to_string(&native).unwrap();
		let source = source
			.lines()
			.filter(|line| {
				!line.starts_with("# nixbox:output ") && !line.starts_with("# nixbox:user ")
			})
			.collect::<Vec<_>>()
			.join("\n")
			+ "\n";
		let legacy = root.join(native.file_name().unwrap());
		std::fs::write(&legacy, source).unwrap();
		std::fs::remove_file(native).unwrap();
		Layout::discover(&root)
			.unwrap()
			.unwrap()
			.ensure_loaded(&legacy)
			.unwrap();
	}
	let mut reloaded = Engine::with_config(dir.config()).unwrap();
	for scope in [Target::NixosSystem, Target::HomeManager] {
		assert_eq!(
			reloaded.config.managed_file_for(scope).parent(),
			Some(root.as_path())
		);
		reloaded
			.apply(
				&Op::Install {
					hit: crate::tests::hit("hello"),
					scope,
				},
				&mut SilentReporter,
			)
			.unwrap();
		assert_eq!(reloaded.import_state(scope), ImportState::Imported);
	}
}

#[test]
fn dendritic_packages_profiles_and_flake_modules_round_trip_without_global_arguments() {
	let dir = crate::tests::temp_dir("dendritic-roundtrip");
	fixture(&dir);
	let config = dir.config();
	let root = config.config_root();
	let original_host = std::fs::read(root.join("modules/host.nix")).unwrap();
	let mut engine = Engine::with_config(config.clone()).unwrap();
	let config = engine.config.clone();
	for scope in [Target::NixosSystem, Target::HomeManager] {
		assert!(engine.has_main_file(scope));
		assert_eq!(engine.import_state(scope), ImportState::NotImported);
		engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit("hello"),
					scope,
				},
				&mut SilentReporter,
			)
			.unwrap();
		assert_eq!(engine.import_state(scope), ImportState::Imported);
		engine
			.apply(
				&Op::SetupProfile {
					scope,
					home_inside_nixos: scope == Target::HomeManager,
				},
				&mut SilentReporter,
			)
			.unwrap();
		assert_eq!(engine.manifest_for(scope).mode, PackageMode::Profile);
		engine
			.apply(
				&Op::InstallFlake {
					repo: "owner/tools".into(),
					module: if scope == Target::NixosSystem {
						"nixosModules.default"
					} else {
						"homeModules.default"
					}
					.into(),
					scope,
				},
				&mut SilentReporter,
			)
			.unwrap();
		let generated = std::fs::read_to_string(config.managed_file_for(scope)).unwrap();
		assert!(generated.contains("# nixbox:layout dendritic"));
		assert!(generated.contains("inputs.tools."));
		assert!(!generated.contains("lib, inputs, pkgs"));
		assert!(generated.contains("# nixbox:mode profile"));
		let before = std::fs::read(config.managed_file_for(scope)).unwrap();
		engine.write_manifest(scope).unwrap();
		assert_eq!(
			std::fs::read(config.managed_file_for(scope)).unwrap(),
			before
		);
	}
	assert_eq!(
		std::fs::read(root.join("modules/host.nix")).unwrap(),
		original_host
	);
	let flake = std::fs::read_to_string(config.flake_file()).unwrap();
	assert!(!flake.contains("specialArgs"));
	assert!(!flake.contains("extraSpecialArgs"));
	assert_eq!(flake.matches("./nixbox-system-packages.nix").count(), 0);
	assert_eq!(flake.matches("./nixbox-home-packages.nix").count(), 0);
	assert!(Layout::discover(&root).unwrap().is_some());
	let reloaded = Engine::with_config(config.clone()).unwrap();
	assert_eq!(
		reloaded.home_manifest.packages,
		engine.home_manifest.packages
	);
	assert_eq!(
		reloaded.nixos_manifest.packages,
		engine.nixos_manifest.packages
	);
	assert_eq!(reloaded.home_manifest.mode, engine.home_manifest.mode);
	assert!(
		ManagedFlakeFile::new(config.managed_file_for(Target::HomeManager))
			.load()
			.unwrap()
			.outputs()
			.iter()
			.any(|(_, output)| matches!(output, FlakeOutput::Module(_)))
	);
	for (scope, module) in [
		(Target::HomeManager, "homeModules.default"),
		(Target::NixosSystem, "nixosModules.default"),
	] {
		engine
			.apply(
				&Op::UninstallFlakeOutput {
					input: "tools".into(),
					output: FlakeOutput::Module(module.into()),
					scope,
				},
				&mut SilentReporter,
			)
			.unwrap();
		assert!(engine.manifest_for(scope).packages.contains("hello"));
		assert_eq!(engine.import_state(scope), ImportState::Imported);
		if scope == Target::HomeManager {
			assert!(
				std::fs::read_to_string(config.flake_file())
					.unwrap()
					.contains("tools")
			);
		}
	}
	assert!(
		!std::fs::read_to_string(config.flake_file())
			.unwrap()
			.contains("tools")
	);
}

#[test]
fn dendritic_ambiguous_hosts_and_failed_installs_preserve_inputs_and_manifests() {
	let dir = crate::tests::temp_dir("dendritic-failure");
	fixture(&dir);
	let config = dir.config();
	let mut engine = Engine::with_config(config.clone()).unwrap();
	let config = engine.config.clone();
	let original = std::fs::read(config.flake_file()).unwrap();
	std::fs::create_dir_all(
		config
			.managed_file_for(Target::HomeManager)
			.parent()
			.unwrap(),
	)
	.unwrap();
	// A conventional managed file cannot be overwritten as a different module class.
	std::fs::write(
		config.managed_file_for(Target::HomeManager),
		"{ ... }: {}\n",
	)
	.unwrap();
	let result = engine.apply(
		&Op::InstallFlakePackage {
			repo: "owner/tools".into(),
			package: "default".into(),
			scope: Target::HomeManager,
		},
		&mut SilentReporter,
	);
	assert!(result.is_err());
	assert_eq!(std::fs::read(config.flake_file()).unwrap(), original);
	assert!(engine.home_manifest.packages.is_empty());
	assert_eq!(
		std::fs::read_to_string(config.managed_file_for(Target::HomeManager)).unwrap(),
		"{ ... }: {}\n"
	);
	std::fs::write(
		config.config_root().join("modules/second.nix"),
		"{ inputs, ... }: { flake.nixosConfigurations.other = inputs.nixpkgs.lib.nixosSystem {}; }",
	)
	.unwrap();
	assert!(
		engine
			.apply(
				&Op::Install {
					hit: crate::tests::hit("hello"),
					scope: Target::NixosSystem
				},
				&mut SilentReporter
			)
			.is_err()
	);
	assert!(!config.managed_file_for(Target::NixosSystem).exists());
	assert_eq!(std::fs::read(config.flake_file()).unwrap(), original);
}

#[test]
fn dendritic_discovery_ignores_excluded_files_and_import_comments() {
	let dir = crate::tests::temp_dir("dendritic-exclusions");
	fixture(&dir);
	let config = dir.config();
	std::fs::write(
		config.config_root().join("modules/_disabled.nix"),
		"this is deliberately invalid Nix",
	)
	.unwrap();
	let mut source = std::fs::read_to_string(config.flake_file()).unwrap();
	source.insert_str(0, "# ./nixbox-home-packages.nix is only a comment\n");
	std::fs::write(config.flake_file(), source).unwrap();
	let engine = Engine::with_config(config.clone()).unwrap();
	assert_eq!(
		engine.import_state(Target::HomeManager),
		ImportState::NotImported
	);
	engine.write_manifest(Target::HomeManager).unwrap();
	assert_eq!(
		engine.import_state(Target::HomeManager),
		ImportState::Imported
	);
}

#[test]
fn dendritic_scanning_follows_selected_classes_and_keeps_external_entries_read_only() {
	let dir = crate::tests::temp_dir("dendritic-scan");
	fixture(&dir);
	let config = dir.config();
	std::fs::write(config.config_root().join("modules/unused.nix"),
		"{ ... }: { flake.modules.homeManager.unused = { pkgs, ... }: { home.packages = [ pkgs.cowsay ]; }; }").unwrap();
	let engine = Engine::with_config(config.clone()).unwrap();
	assert!(
		engine
			.external_packages
			.iter()
			.any(|entry| entry.name == "git"
				&& entry.scope == nixbox_nix::scan::ScanTarget::HomeManager
				&& !entry.migratable)
	);
	assert!(
		!engine
			.external_packages
			.iter()
			.any(|entry| entry.name == "cowsay"
				|| entry.scope == nixbox_nix::scan::ScanTarget::Nixos)
	);
	assert_eq!(
		engine.integration_file_for(Target::HomeManager),
		config.flake_file()
	);
}
