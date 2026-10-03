{ pkgs, ... }:

let
	guiLibraries = [
		pkgs.vulkan-loader
		pkgs.wayland
		pkgs.libxkbcommon
		pkgs.libx11
		pkgs.libxcb
	];
in
{
	packages = [
		# Build, lint, and test commands.
		pkgs.bash
		pkgs.just
		pkgs.cargo-nextest
		pkgs.nixd
		pkgs.nil

		# Package discovery, rebuilds, and configuration version control.
		pkgs.nix
		pkgs.nixos-rebuild
		pkgs.home-manager
		pkgs.git
		pkgs.jujutsu
		pkgs.gh
		pkgs.openssh

		# Native GUI compilation and desktop-entry validation.
		pkgs.pkg-config
		pkgs.fontconfig
		pkgs.freetype
		pkgs.desktop-file-utils
	] ++ guiLibraries;

	# GPUI loads these libraries at runtime rather than linking them.
	env.LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiLibraries;

	languages.rust = {
		enable = true;
		channel = "nightly";
		components = [
			"rustc"
			"cargo"
			"clippy"
			"rustfmt"
			"rust-analyzer"
			"rust-src"
		];
	};

	enterTest = ''
		just ci
	'';
}
