{ pkgs, ... }:

{
  packages = [
    pkgs.nix
    pkgs.nixd
    pkgs.nil
    pkgs.just
    pkgs.cargo-nextest
	pkgs.git
	pkgs.jujutsu
	pkgs.gh
    # Build inputs for nixbox-gui.
    pkgs.pkg-config
    pkgs.fontconfig
    pkgs.freetype
    pkgs.wayland
    pkgs.libxkbcommon
    pkgs.libx11
    pkgs.libxcb
    pkgs.vulkan-loader
  ];

  # gpui loads the Vulkan loader and the windowing libraries at runtime.
  env.LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
    pkgs.vulkan-loader
    pkgs.wayland
    pkgs.libxkbcommon
    pkgs.libx11
    pkgs.libxcb
  ];

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
