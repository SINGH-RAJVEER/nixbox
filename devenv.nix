{ pkgs, ... }:

{
  packages = [
    pkgs.nix
    pkgs.nixd
    pkgs.nil
    pkgs.just
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
