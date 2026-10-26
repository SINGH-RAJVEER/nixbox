# Hardware-aware NixOS settings

The NixOS settings browser can hide hardware-specific settings that do not apply to the selected machine. General settings, custom namespaces without a rule, configured settings, and staged edits remain accessible. Both the desktop and terminal use the same rules, including category counts and search results. The desktop editor applies those rules to sibling groups too.

## Link hardware to a host

Select a host, then enable **Use this machine's hardware** in the desktop page, or press `Alt-H` in the terminal tab. This explicitly associates the selected configuration root and output key with the machine running NixBox. Do this only when that output describes this machine. A hostname match alone cannot establish that association.

Hardware filtering is enabled once linked. **Show all hardware options**, or `Alt-A` in the terminal, restores options excluded by hardware rules. The diagnostic filter remains independent. Disable **Use this machine's hardware**, or press `Alt-H` again, to unlink it.

The association lasts for the session. Reloading the same root and output refreshes its hardware inventory and keeps the association. Selecting another host, changing configuration directories, or restarting NixBox clears it. Until a host is linked, hardware applicability is unknown and no options are excluded by hardware rules. Remote-host reports and persistent associations are not implemented.

## Detection and visibility

Detection reads `/proc/cpuinfo`, PCI and USB device metadata, the Bluetooth and power-supply classes, and the DMI chassis type. It does not load drivers, enable services, require sudo, or change system configuration. PCI and USB probes inspect device identities independently of driver binding, so disabling Bluetooth does not by itself hide its settings.

Each capability is present, absent, or unknown. Missing files, unreadable metadata, incomplete PCI enumeration, vendor-specific USB interfaces, and unsupported CPU identification leave the affected capability unknown. A missing battery driver on a laptop does not prove there is no battery. Battery absence is inferred only for known desktop/server chassis types with a readable power-supply scan and no reported battery. See the kernel's [PCI sysfs documentation](https://docs.kernel.org/PCI/sysfs-pci.html) for the device identifiers used by the PCI probe.

| Rule | Requirement |
| --- | --- |
| `hardware.cpu.intel.*` | An Intel CPU. |
| `hardware.cpu.amd.*` | An AMD CPU. |
| `hardware.nvidia.*`, `hardware.nvidia-container-toolkit.*`, `services.nvidia-optimus.*` | An NVIDIA GPU. |
| `hardware.nvidia.prime.*` | An NVIDIA GPU and a detected or still-unknown Intel/AMD GPU. This establishes possible relevance, not compatibility of every PRIME mode. |
| `hardware.amdgpu.*` | An AMD GPU. |
| `hardware.intel-gpu.*` | An Intel GPU. |
| `hardware.bluetooth.*`, `services.blueman.*` | A Bluetooth adapter. |
| `services.tuned.ppdSettings.battery`, `services.tuned.ppdSettings.main.battery_detection`, `services.undervolt.tempBat` | A battery. |

Rules match path segments, not words in descriptions. CPU vendor never determines GPU vendor. General graphics, audio, TLP, UPower, and power-profile settings remain available because they can apply without the specific devices above. The selected flake's schema remains authoritative; a rule does not create an option missing from that schema. Generic attribute settings are not inspected or rewritten to remove individual keys.

Discovery records whether an option has a surviving non-default definition outside the target nixpkgs tree, using definition locations and priorities without evaluating its value. These configured settings remain visible even if the related hardware is absent, so obsolete configuration can be reviewed. A configured parent retains its nested schema fields for hardware filtering. The **Configured options** category lists only the concrete parent, since those nested templates do not identify configured instances. If provenance cannot be evaluated, the option remains visible. Staged edits also remain visible. **Show all hardware options** provides access to unconfigured settings for disconnected or future hardware.

The detector and rule set are deliberately conservative. Unknown applicability stays visible; this is not a claim that every listed option has been verified against every device model. Reload after connecting or removing devices. Hardware detection does not validate driver compatibility or replace NixOS evaluation and rebuild checks.
