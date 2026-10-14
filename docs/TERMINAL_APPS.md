# Separate interface installations

Install only the NixBox interfaces you want. The GUI and TUI use the shared engine directly and contain no CLI command implementation.

| Flake package | Installed commands |
| --- | --- |
| `nixbox` | `nixbox-gui` |
| `nixbox-tui` | `nixbox-tui` |
| `nixbox-cli` | `nixbox`, `nb`, `nixbox-cli` |

Installing the GUI or TUI does not create `nixbox` or `nb` commands or register shell detection. The CLI has no `--gui` or `--tui` option. Bare `nixbox` and `nb` show CLI help. Open the GUI through the desktop entry or `nixbox-gui`, and the TUI through `nixbox-tui`.

## Install from the flake

1. Use a Nix installation with the `nix-command` and `flakes` experimental features enabled.
2. Choose the interface you want. Run either installation command, or both:

```sh
nix profile add github:SINGH-RAJVEER/nixbox#nixbox-tui
nix profile add github:SINGH-RAJVEER/nixbox#nixbox-cli
```

3. Launch the TUI or show CLI help:

```sh
nixbox-tui
nixbox --help
nb --help
```

The GUI's Settings > Terminal applications shows these commands with copy buttons. Copying does not run a command or install anything. The GUI itself remains available as:

```sh
nix profile add github:SINGH-RAJVEER/nixbox#nixbox
nixbox-gui
```

`nix profile install` is an alias for `nix profile add`. See the [Nix profile add manual](https://nix.dev/manual/nix/2.34/command-ref/new-cli/nix3-profile-add.html).

## Install through your configuration flake

Add the NixBox input to your existing flake:

```nix
inputs.nixbox.url = "github:SINGH-RAJVEER/nixbox";
```

Include `nixbox` in the arguments of the enclosing `outputs` function. In the NixOS module constructed by that function, add the interfaces you want:

```nix
({ pkgs, ... }: {
	environment.systemPackages = [
		nixbox.packages.${pkgs.stdenv.hostPlatform.system}.nixbox-tui
		nixbox.packages.${pkgs.stdenv.hostPlatform.system}.nixbox-cli
	];
})
```

For Home Manager, use the same list in `home.packages`. The `nixbox` identifier above is the flake input in the enclosing scope, not an automatically provided module argument. Lock the new input and apply your configuration with your usual NixOS or Home Manager rebuild command. Keep only the entries for interfaces you want installed.
