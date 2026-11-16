//! The NixOS settings pages, shared by every front-end: the curated pages
//! that name common options in plain words, the views around them, the
//! namespace tree over the whole schema, search across both, and how an
//! edit is staged.
//!
//! Curated entries list candidate paths newest first. Each resolves against
//! the selected host's schema, so a path missing from a release drops out
//! instead of showing a broken row.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nixbox_config::Target;
use nixbox_nix::settings::{OptionPath, SettingValue, option_path_display};
use nixbox_nix::system_options::SystemOption;

use crate::hardware::HardwareFilter;
use crate::session::Session;
use crate::system_options::{Category, GROUP_LIMIT, filter_options};

/// One curated setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
	/// Candidate option paths, newest release first.
	pub paths: &'static [&'static str],
	pub label: &'static str,
	pub description: &'static str,
}

/// A titled run of entries on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
	pub title: &'static str,
	pub entries: &'static [Entry],
}

const fn entry(
	paths: &'static [&'static str],
	label: &'static str,
	description: &'static str,
) -> Entry {
	Entry {
		paths,
		label,
		description,
	}
}

/// Where a page sits in the navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageGroup {
	System,
	Software,
}

impl PageGroup {
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::System => "System",
			Self::Software => "Software",
		}
	}
}

/// A curated settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
	General,
	Network,
	Desktop,
	Audio,
	Graphics,
	Users,
	Boot,
	Programs,
	Services,
	Virtualization,
	Nix,
}

impl SettingsPage {
	pub const ALL: [Self; 11] = [
		Self::General,
		Self::Network,
		Self::Desktop,
		Self::Audio,
		Self::Graphics,
		Self::Users,
		Self::Boot,
		Self::Programs,
		Self::Services,
		Self::Virtualization,
		Self::Nix,
	];

	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::General => "General and region",
			Self::Network => "Network",
			Self::Desktop => "Desktop and login",
			Self::Audio => "Audio and devices",
			Self::Graphics => "Graphics and power",
			Self::Users => "Users and security",
			Self::Boot => "Boot and storage",
			Self::Programs => "Programs",
			Self::Services => "Services",
			Self::Virtualization => "Virtualization",
			Self::Nix => "Nix and maintenance",
		}
	}

	#[must_use]
	pub const fn description(self) -> &'static str {
		match self {
			Self::General => "Host name, time, language, keyboard, and fonts.",
			Self::Network => "Connections, name resolution, the firewall, and remote access.",
			Self::Desktop => "Desktops, the login screen, input, and printing.",
			Self::Audio => "Sound, Bluetooth, firmware, and attached devices.",
			Self::Graphics => "Graphics drivers, power profiles, and what the lid does.",
			Self::Users => "Accounts, administrator rights, hardening, and keys.",
			Self::Boot => "The boot loader, kernel, temporary files, and disks.",
			Self::Programs => "Shells, editors, and applications with system modules.",
			Self::Services => "Background services, file sharing, and servers.",
			Self::Virtualization => "Containers, virtual machines, and guest tools.",
			Self::Nix => "The Nix daemon, cleanup, and automatic upgrades.",
		}
	}

	#[must_use]
	pub const fn group(self) -> PageGroup {
		match self {
			Self::Programs | Self::Services | Self::Virtualization | Self::Nix => {
				PageGroup::Software
			}
			_ => PageGroup::System,
		}
	}

	#[must_use]
	pub fn index(self) -> usize {
		Self::ALL
			.iter()
			.position(|page| *page == self)
			.unwrap_or_default()
	}

	#[must_use]
	pub const fn sections(self) -> &'static [Section] {
		match self {
			Self::General => GENERAL,
			Self::Network => NETWORK,
			Self::Desktop => DESKTOP,
			Self::Audio => AUDIO,
			Self::Graphics => GRAPHICS,
			Self::Users => USERS,
			Self::Boot => BOOT,
			Self::Programs => PROGRAMS,
			Self::Services => SERVICES,
			Self::Virtualization => VIRTUALIZATION,
			Self::Nix => NIX,
		}
	}
}

const GENERAL: &[Section] = &[
	Section {
		title: "Identity and time",
		entries: &[
			entry(
				&["networking.hostName"],
				"Host name",
				"The name this machine uses on the network.",
			),
			entry(
				&["time.timeZone"],
				"Time zone",
				"An IANA zone such as Europe/London. Empty leaves it to the desktop.",
			),
			entry(
				&["services.automatic-timezoned.enable"],
				"Set the time zone automatically",
				"Follow the location reported by GeoClue.",
			),
			entry(
				&["services.timesyncd.enable"],
				"Synchronize the clock",
				"Keep the clock accurate with network time servers.",
			),
			entry(
				&["time.hardwareClockInLocalTime"],
				"Hardware clock in local time",
				"Needed when dual-booting Windows without changing its setting.",
			),
		],
	},
	Section {
		title: "Language and keyboard",
		entries: &[
			entry(
				&["i18n.defaultLocale"],
				"System language",
				"The locale for messages, dates, and number formats.",
			),
			entry(
				&["services.xserver.xkb.layout", "services.xserver.layout"],
				"Keyboard layout",
				"The layout for graphical sessions, such as us or de.",
			),
			entry(
				&[
					"services.xserver.xkb.variant",
					"services.xserver.xkbVariant",
				],
				"Keyboard variant",
				"A variant of the layout, such as dvorak or colemak.",
			),
			entry(
				&["console.keyMap"],
				"Console keyboard layout",
				"The layout on text consoles before a desktop starts.",
			),
			entry(
				&["console.useXkbConfig"],
				"Console follows the desktop layout",
				"Derive the console layout from the keyboard settings above.",
			),
		],
	},
	Section {
		title: "Fonts and documentation",
		entries: &[
			entry(
				&["fonts.enableDefaultPackages", "fonts.enableDefaultFonts"],
				"Default fonts",
				"Install a basic set of fonts for most scripts.",
			),
			entry(
				&["fonts.fontDir.enable"],
				"Shared font directory",
				"Expose installed fonts to applications that look in a fixed directory.",
			),
			entry(
				&["documentation.man.enable"],
				"Manual pages",
				"Install man pages for system packages.",
			),
			entry(
				&["documentation.nixos.enable"],
				"NixOS manual",
				"Install the NixOS manual and the nixos-help command.",
			),
		],
	},
];

const NETWORK: &[Section] = &[
	Section {
		title: "Connections",
		entries: &[
			entry(
				&["networking.networkmanager.enable"],
				"NetworkManager",
				"Manage wired, Wi-Fi, and VPN connections from the desktop.",
			),
			entry(
				&["networking.wireless.enable"],
				"wpa_supplicant Wi-Fi",
				"Configure Wi-Fi declaratively. Do not combine with NetworkManager.",
			),
			entry(
				&["networking.useDHCP"],
				"DHCP on every interface",
				"Request addresses on all interfaces by default.",
			),
			entry(
				&["networking.enableIPv6"],
				"IPv6",
				"Enable IPv6 on this machine.",
			),
		],
	},
	Section {
		title: "Name resolution",
		entries: &[
			entry(
				&["networking.nameservers"],
				"Name servers",
				"Addresses of DNS servers to use, such as 1.1.1.1.",
			),
			entry(
				&["services.resolved.enable"],
				"systemd-resolved",
				"Resolve names with systemd-resolved, including DNS over TLS.",
			),
			entry(
				&["services.avahi.enable"],
				"Local network discovery",
				"Find and announce services on the local network with Avahi.",
			),
			entry(
				&["services.avahi.nssmdns4", "services.avahi.nssmdns"],
				"Resolve .local names",
				"Look up .local host names through Avahi.",
			),
		],
	},
	Section {
		title: "Firewall",
		entries: &[
			entry(
				&["networking.firewall.enable"],
				"Firewall",
				"Block incoming connections that are not explicitly allowed.",
			),
			entry(
				&["networking.firewall.allowPing"],
				"Answer pings",
				"Reply to ICMP echo requests.",
			),
			entry(
				&["networking.firewall.allowedTCPPorts"],
				"Open TCP ports",
				"Ports that accept incoming TCP connections.",
			),
			entry(
				&["networking.firewall.allowedUDPPorts"],
				"Open UDP ports",
				"Ports that accept incoming UDP traffic.",
			),
		],
	},
	Section {
		title: "Remote access",
		entries: &[
			entry(
				&["services.openssh.enable"],
				"SSH server",
				"Allow logging in to this machine over SSH.",
			),
			entry(
				&[
					"services.openssh.settings.PasswordAuthentication",
					"services.openssh.passwordAuthentication",
				],
				"SSH password login",
				"Accept passwords as well as keys.",
			),
			entry(
				&[
					"services.openssh.settings.PermitRootLogin",
					"services.openssh.permitRootLogin",
				],
				"SSH root login",
				"Whether root may log in over SSH.",
			),
			entry(
				&["services.tailscale.enable"],
				"Tailscale",
				"Join this machine to a Tailscale network.",
			),
		],
	},
];

const DESKTOP: &[Section] = &[
	Section {
		title: "Desktops",
		entries: &[
			entry(
				&["services.xserver.enable"],
				"X11 display server",
				"Required by X11 desktops and most display managers.",
			),
			entry(
				&[
					"services.desktopManager.gnome.enable",
					"services.xserver.desktopManager.gnome.enable",
				],
				"GNOME",
				"Install the GNOME desktop.",
			),
			entry(
				&["services.desktopManager.plasma6.enable"],
				"KDE Plasma 6",
				"Install the Plasma 6 desktop.",
			),
			entry(
				&[
					"services.desktopManager.cosmic.enable",
					"services.xserver.desktopManager.cosmic.enable",
				],
				"COSMIC",
				"Install the COSMIC desktop.",
			),
			entry(
				&["services.xserver.desktopManager.xfce.enable"],
				"Xfce",
				"Install the Xfce desktop.",
			),
			entry(
				&["programs.hyprland.enable"],
				"Hyprland",
				"Install the Hyprland Wayland compositor.",
			),
			entry(
				&["programs.niri.enable"],
				"niri",
				"Install the niri scrollable-tiling compositor.",
			),
			entry(
				&["programs.sway.enable"],
				"Sway",
				"Install the Sway tiling compositor.",
			),
		],
	},
	Section {
		title: "Login",
		entries: &[
			entry(
				&[
					"services.displayManager.gdm.enable",
					"services.xserver.displayManager.gdm.enable",
				],
				"GDM login screen",
				"The GNOME display manager.",
			),
			entry(
				&["services.displayManager.sddm.enable"],
				"SDDM login screen",
				"The display manager used by KDE.",
			),
			entry(
				&["services.displayManager.sddm.wayland.enable"],
				"SDDM on Wayland",
				"Run the SDDM greeter on Wayland.",
			),
			entry(
				&["services.displayManager.autoLogin.enable"],
				"Automatic login",
				"Sign a user in without a password at boot.",
			),
			entry(
				&["services.displayManager.autoLogin.user"],
				"Automatic login user",
				"The account signed in automatically.",
			),
		],
	},
	Section {
		title: "Input and printing",
		entries: &[
			entry(
				&[
					"services.libinput.enable",
					"services.xserver.libinput.enable",
				],
				"Touchpad and mouse support",
				"Handle input devices with libinput.",
			),
			entry(
				&[
					"services.libinput.touchpad.tapping",
					"services.xserver.libinput.touchpad.tapping",
				],
				"Tap to click",
				"Treat a touchpad tap as a click.",
			),
			entry(
				&[
					"services.libinput.touchpad.naturalScrolling",
					"services.xserver.libinput.touchpad.naturalScrolling",
				],
				"Natural scrolling",
				"Scroll content in the direction your fingers move.",
			),
			entry(
				&["services.printing.enable"],
				"Printing",
				"Run CUPS so this machine can print.",
			),
			entry(
				&["hardware.sane.enable"],
				"Scanners",
				"Support scanners through SANE.",
			),
			entry(
				&["xdg.portal.enable"],
				"Desktop portals",
				"Let sandboxed applications open files and share the screen.",
			),
		],
	},
];

const AUDIO: &[Section] = &[
	Section {
		title: "Sound",
		entries: &[
			entry(
				&["services.pipewire.enable"],
				"PipeWire",
				"The audio and video server most desktops expect.",
			),
			entry(
				&["services.pipewire.pulse.enable"],
				"PulseAudio compatibility",
				"Serve PulseAudio clients through PipeWire.",
			),
			entry(
				&["services.pipewire.alsa.enable"],
				"ALSA compatibility",
				"Route ALSA applications through PipeWire.",
			),
			entry(
				&["services.pipewire.alsa.support32Bit"],
				"32-bit ALSA applications",
				"Needed by some games and older software.",
			),
			entry(
				&["services.pipewire.jack.enable"],
				"JACK compatibility",
				"Serve low-latency JACK clients through PipeWire.",
			),
			entry(
				&["services.pulseaudio.enable", "hardware.pulseaudio.enable"],
				"PulseAudio",
				"The older sound server. Do not combine with PipeWire.",
			),
			entry(
				&["security.rtkit.enable"],
				"Realtime scheduling",
				"Let audio servers request realtime priority.",
			),
		],
	},
	Section {
		title: "Bluetooth",
		entries: &[
			entry(
				&["hardware.bluetooth.enable"],
				"Bluetooth",
				"Support Bluetooth adapters.",
			),
			entry(
				&["hardware.bluetooth.powerOnBoot"],
				"Power on at boot",
				"Turn the adapter on when the machine starts.",
			),
			entry(
				&["services.blueman.enable"],
				"Blueman",
				"A Bluetooth manager for desktops without their own.",
			),
		],
	},
	Section {
		title: "Firmware and devices",
		entries: &[
			entry(
				&["hardware.enableRedistributableFirmware"],
				"Redistributable firmware",
				"Install firmware that may be freely redistributed.",
			),
			entry(
				&["hardware.enableAllFirmware"],
				"All firmware",
				"Install every firmware package, including unfree ones.",
			),
			entry(
				&["services.fwupd.enable"],
				"Firmware updates",
				"Update device firmware with fwupd.",
			),
			entry(
				&["services.udisks2.enable"],
				"Removable drives",
				"Mount USB drives and other removable media from the desktop.",
			),
			entry(
				&["services.fprintd.enable"],
				"Fingerprint reader",
				"Support fingerprint login with fprintd.",
			),
		],
	},
];

const GRAPHICS: &[Section] = &[
	Section {
		title: "Graphics",
		entries: &[
			entry(
				&["hardware.graphics.enable", "hardware.opengl.enable"],
				"Hardware acceleration",
				"Install graphics drivers for OpenGL and Vulkan.",
			),
			entry(
				&[
					"hardware.graphics.enable32Bit",
					"hardware.opengl.driSupport32Bit",
				],
				"32-bit graphics",
				"Needed by Steam and other 32-bit applications.",
			),
			entry(
				&["services.xserver.videoDrivers"],
				"Video drivers",
				"Drivers to load, such as modesetting, amdgpu, or nvidia.",
			),
			entry(
				&["hardware.nvidia.open"],
				"Open NVIDIA kernel module",
				"Use NVIDIA's open kernel module on supported GPUs.",
			),
			entry(
				&["hardware.nvidia.modesetting.enable"],
				"NVIDIA modesetting",
				"Required for Wayland on NVIDIA.",
			),
			entry(
				&["hardware.nvidia.prime.offload.enable"],
				"NVIDIA PRIME offload",
				"Run applications on the discrete GPU on demand.",
			),
		],
	},
	Section {
		title: "Power",
		entries: &[
			entry(
				&["powerManagement.enable"],
				"Power management",
				"Support suspend and hibernation.",
			),
			entry(
				&["powerManagement.cpuFreqGovernor"],
				"CPU frequency governor",
				"Such as powersave or performance. Empty leaves the kernel default.",
			),
			entry(
				&["services.power-profiles-daemon.enable"],
				"Power profiles",
				"Switch between balanced, power saver, and performance from the desktop.",
			),
			entry(
				&["services.tlp.enable"],
				"TLP",
				"Tune laptop power use. Do not combine with power profiles.",
			),
			entry(
				&["services.thermald.enable"],
				"Thermal daemon",
				"Prevent Intel CPUs from overheating.",
			),
			entry(
				&["services.upower.enable"],
				"Battery status",
				"Report battery and power status to applications.",
			),
		],
	},
	Section {
		title: "Lid and buttons",
		entries: &[
			entry(
				&[
					"services.logind.settings.Login.HandleLidSwitch",
					"services.logind.lidSwitch",
				],
				"When the lid closes",
				"What happens on battery when the lid is closed.",
			),
			entry(
				&[
					"services.logind.settings.Login.HandleLidSwitchExternalPower",
					"services.logind.lidSwitchExternalPower",
				],
				"When the lid closes on power",
				"What happens on external power when the lid is closed.",
			),
			entry(
				&[
					"services.logind.settings.Login.HandlePowerKey",
					"services.logind.powerKey",
				],
				"Power button",
				"What happens when the power button is pressed.",
			),
		],
	},
];

const USERS: &[Section] = &[
	Section {
		title: "Accounts and privileges",
		entries: &[
			entry(
				&["users.mutableUsers"],
				"Allow changing users outside the configuration",
				"When off, passwd and useradd changes are reset by every rebuild.",
			),
			entry(
				&["security.sudo.enable"],
				"sudo",
				"Allow administrators to run commands as root with sudo.",
			),
			entry(
				&["security.sudo.wheelNeedsPassword"],
				"sudo asks for a password",
				"Require members of wheel to enter their password.",
			),
			entry(
				&["security.sudo-rs.enable"],
				"sudo-rs",
				"Use the memory-safe sudo-rs instead of sudo.",
			),
			entry(
				&["security.doas.enable"],
				"doas",
				"A simpler alternative to sudo.",
			),
			entry(
				&["security.polkit.enable"],
				"polkit",
				"Let desktop applications request administrator rights.",
			),
		],
	},
	Section {
		title: "Hardening",
		entries: &[
			entry(
				&["security.apparmor.enable"],
				"AppArmor",
				"Confine programs with AppArmor profiles.",
			),
			entry(
				&["security.protectKernelImage"],
				"Protect the kernel image",
				"Prevent replacing the running kernel and disable hibernation.",
			),
			entry(
				&["security.auditd.enable"],
				"Audit daemon",
				"Record security-relevant events.",
			),
			entry(
				&["services.fail2ban.enable"],
				"fail2ban",
				"Ban addresses that repeatedly fail to log in.",
			),
		],
	},
	Section {
		title: "Keys and agents",
		entries: &[
			entry(
				&["programs.gnupg.agent.enable"],
				"GnuPG agent",
				"Cache GnuPG passphrases for the session.",
			),
			entry(
				&["programs.gnupg.agent.enableSSHSupport"],
				"GnuPG agent for SSH",
				"Serve SSH keys from the GnuPG agent.",
			),
			entry(
				&["programs.ssh.startAgent"],
				"SSH agent",
				"Start ssh-agent for each session.",
			),
			entry(
				&["services.gnome.gnome-keyring.enable"],
				"GNOME Keyring",
				"Store passwords and secrets for applications.",
			),
		],
	},
];

const BOOT: &[Section] = &[
	Section {
		title: "Boot loader",
		entries: &[
			entry(
				&["boot.loader.systemd-boot.enable"],
				"systemd-boot",
				"Boot with systemd-boot on UEFI machines.",
			),
			entry(
				&["boot.loader.systemd-boot.configurationLimit"],
				"Boot entries to keep",
				"The most generations listed in the boot menu. Empty keeps all.",
			),
			entry(&["boot.loader.grub.enable"], "GRUB", "Boot with GRUB."),
			entry(
				&["boot.loader.efi.canTouchEfiVariables"],
				"Write EFI variables",
				"Let the installer register the boot loader with the firmware.",
			),
			entry(
				&["boot.loader.timeout"],
				"Boot menu timeout",
				"Seconds to show the menu. Empty waits for a choice.",
			),
			entry(
				&["boot.plymouth.enable"],
				"Boot splash",
				"Show a graphical splash screen while booting.",
			),
		],
	},
	Section {
		title: "Kernel",
		entries: &[
			entry(
				&["boot.kernelParams"],
				"Kernel parameters",
				"Extra parameters passed to the kernel at boot.",
			),
			entry(
				&["boot.kernelModules"],
				"Kernel modules",
				"Modules loaded during the second stage of boot.",
			),
			entry(
				&["boot.initrd.systemd.enable"],
				"systemd in the initrd",
				"Use systemd in the initial ramdisk.",
			),
		],
	},
	Section {
		title: "Temporary files and disks",
		entries: &[
			entry(
				&["boot.tmp.cleanOnBoot"],
				"Clean /tmp at boot",
				"Delete everything in /tmp when the machine starts.",
			),
			entry(
				&["boot.tmp.useTmpfs"],
				"/tmp in memory",
				"Mount /tmp as a tmpfs.",
			),
			entry(
				&["zramSwap.enable"],
				"Compressed swap in memory",
				"Use zram for swap.",
			),
			entry(
				&["zramSwap.memoryPercent"],
				"zram size",
				"Percent of memory zram may use.",
			),
			entry(
				&["services.fstrim.enable"],
				"Periodic TRIM",
				"Trim SSDs weekly.",
			),
			entry(
				&["services.btrfs.autoScrub.enable"],
				"Btrfs scrubbing",
				"Check btrfs filesystems for errors regularly.",
			),
			entry(
				&["services.smartd.enable"],
				"Disk health monitoring",
				"Watch drives with smartd.",
			),
		],
	},
];

const PROGRAMS: &[Section] = &[
	Section {
		title: "Shells",
		entries: &[
			entry(
				&["programs.zsh.enable"],
				"Zsh",
				"Install zsh and set up its system configuration.",
			),
			entry(
				&["programs.fish.enable"],
				"fish",
				"Install fish and set up its system configuration.",
			),
			entry(
				&[
					"programs.bash.completion.enable",
					"programs.bash.enableCompletion",
				],
				"Bash completion",
				"Complete commands and arguments in bash.",
			),
			entry(
				&["programs.command-not-found.enable"],
				"Suggest missing commands",
				"Show which package provides a command that was not found.",
			),
		],
	},
	Section {
		title: "Tools",
		entries: &[
			entry(
				&["programs.git.enable"],
				"Git",
				"Install git with a system configuration.",
			),
			entry(&["programs.neovim.enable"], "Neovim", "Install Neovim."),
			entry(
				&["programs.neovim.defaultEditor"],
				"Neovim as the default editor",
				"Set EDITOR to nvim.",
			),
			entry(
				&["programs.direnv.enable"],
				"direnv",
				"Load project environments when entering directories.",
			),
			entry(
				&["programs.nix-ld.enable"],
				"Run unpatched binaries",
				"Provide a dynamic loader for binaries built for other distributions.",
			),
			entry(
				&["programs.tmux.enable"],
				"tmux",
				"Install tmux with a system configuration.",
			),
		],
	},
	Section {
		title: "Applications",
		entries: &[
			entry(
				&["programs.firefox.enable"],
				"Firefox",
				"Install Firefox with system policies.",
			),
			entry(
				&["programs.steam.enable"],
				"Steam",
				"Install Steam with the libraries it needs.",
			),
			entry(
				&["programs.gamemode.enable"],
				"GameMode",
				"Let games request performance optimizations.",
			),
			entry(
				&["programs.appimage.enable"],
				"AppImage support",
				"Run AppImage files.",
			),
			entry(
				&["programs.dconf.enable"],
				"dconf",
				"Store settings for GNOME and GTK applications.",
			),
		],
	},
];

const SERVICES: &[Section] = &[
	Section {
		title: "Desktop services",
		entries: &[
			entry(
				&["services.flatpak.enable"],
				"Flatpak",
				"Install applications from Flatpak repositories.",
			),
			entry(
				&["services.gvfs.enable"],
				"Virtual filesystems",
				"Browse network shares and trash in file managers.",
			),
			entry(
				&["services.locate.enable"],
				"File index",
				"Update the locate database periodically.",
			),
			entry(
				&["services.syncthing.enable"],
				"Syncthing",
				"Synchronize folders between devices.",
			),
		],
	},
	Section {
		title: "File sharing",
		entries: &[
			entry(
				&["services.samba.enable"],
				"Samba",
				"Share folders with Windows and macOS machines.",
			),
			entry(
				&["services.nfs.server.enable"],
				"NFS server",
				"Export folders over NFS.",
			),
		],
	},
	Section {
		title: "Servers",
		entries: &[
			entry(
				&["services.nginx.enable"],
				"nginx",
				"Serve websites with nginx.",
			),
			entry(
				&["services.caddy.enable"],
				"Caddy",
				"Serve websites with automatic HTTPS.",
			),
			entry(
				&["services.postgresql.enable"],
				"PostgreSQL",
				"Run a PostgreSQL database server.",
			),
			entry(
				&["services.mysql.enable"],
				"MySQL",
				"Run a MySQL or MariaDB database server.",
			),
			entry(
				&["services.jellyfin.enable"],
				"Jellyfin",
				"Stream your media library.",
			),
		],
	},
];

const VIRTUALIZATION: &[Section] = &[
	Section {
		title: "Containers",
		entries: &[
			entry(
				&["virtualisation.docker.enable"],
				"Docker",
				"Run the Docker daemon.",
			),
			entry(
				&["virtualisation.docker.rootless.enable"],
				"Rootless Docker",
				"Run Docker as an unprivileged user.",
			),
			entry(
				&["virtualisation.podman.enable"],
				"Podman",
				"Run containers without a daemon.",
			),
			entry(
				&["virtualisation.podman.dockerCompat"],
				"Podman as docker",
				"Provide a docker command backed by Podman.",
			),
			entry(
				&["virtualisation.oci-containers.backend"],
				"Container backend",
				"The engine that runs declared containers.",
			),
		],
	},
	Section {
		title: "Virtual machines",
		entries: &[
			entry(
				&["virtualisation.libvirtd.enable"],
				"libvirt",
				"Run virtual machines with libvirt and QEMU.",
			),
			entry(
				&["programs.virt-manager.enable"],
				"Virtual Machine Manager",
				"A desktop application for libvirt machines.",
			),
			entry(
				&["virtualisation.virtualbox.host.enable"],
				"VirtualBox",
				"Run VirtualBox virtual machines.",
			),
			entry(
				&["virtualisation.waydroid.enable"],
				"Waydroid",
				"Run Android applications in a container.",
			),
			entry(
				&["virtualisation.spiceUSBRedirection.enable"],
				"USB redirection",
				"Pass USB devices to SPICE virtual machines.",
			),
		],
	},
	Section {
		title: "When this is a guest",
		entries: &[
			entry(
				&["services.qemuGuest.enable"],
				"QEMU guest agent",
				"Let the host manage this machine when it runs under QEMU.",
			),
			entry(
				&["services.spice-vdagentd.enable"],
				"SPICE guest agent",
				"Share the clipboard and resize the display under SPICE.",
			),
		],
	},
];

const NIX: &[Section] = &[
	Section {
		title: "Nix",
		entries: &[
			entry(
				&["nix.settings.experimental-features"],
				"Experimental features",
				"Such as nix-command and flakes.",
			),
			entry(
				&["nix.settings.auto-optimise-store"],
				"Deduplicate the store on write",
				"Hard-link identical files as they are added.",
			),
			entry(
				&["nix.settings.trusted-users"],
				"Trusted users",
				"Users who may configure the daemon, such as adding substituters.",
			),
			entry(
				&["nix.settings.substituters"],
				"Binary caches",
				"Where Nix downloads prebuilt packages from.",
			),
			entry(
				&["nix.settings.trusted-public-keys"],
				"Binary cache keys",
				"Keys that sign packages from the binary caches.",
			),
			entry(
				&["nixpkgs.config.allowUnfree"],
				"Allow unfree packages",
				"Permit packages with unfree licenses.",
			),
		],
	},
	Section {
		title: "Cleanup",
		entries: &[
			entry(
				&["nix.gc.automatic"],
				"Collect garbage automatically",
				"Delete unreferenced store paths on a schedule.",
			),
			entry(
				&["nix.gc.dates"],
				"Garbage collection schedule",
				"A systemd calendar expression, such as weekly.",
			),
			entry(
				&["nix.gc.options"],
				"Garbage collection options",
				"Such as --delete-older-than 14d.",
			),
			entry(
				&["nix.optimise.automatic"],
				"Optimize the store on a schedule",
				"Deduplicate the store periodically.",
			),
		],
	},
	Section {
		title: "Upgrades",
		entries: &[
			entry(
				&["system.autoUpgrade.enable"],
				"Automatic upgrades",
				"Rebuild from the configured source on a schedule.",
			),
			entry(
				&["system.autoUpgrade.allowReboot"],
				"Reboot after upgrades",
				"Reboot when an upgrade changes the kernel or initrd.",
			),
			entry(
				&["system.autoUpgrade.dates"],
				"Upgrade schedule",
				"A systemd calendar expression, such as 04:40.",
			),
			entry(
				&["system.autoUpgrade.flake"],
				"Upgrade flake",
				"The flake to upgrade from. Empty uses the channel.",
			),
		],
	},
];

/// What the settings area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum View {
	#[default]
	Overview,
	Page(SettingsPage),
	/// Staged and queued edits.
	Changed,
	/// The whole schema, as a namespace tree.
	AllOptions,
}

impl View {
	/// Every view in navigation order.
	#[must_use]
	pub fn all() -> Vec<Self> {
		let mut views = vec![Self::Overview];
		views.extend(SettingsPage::ALL.map(Self::Page));
		views.push(Self::Changed);
		views.push(Self::AllOptions);
		views
	}

	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Overview => "Overview",
			Self::Page(page) => page.label(),
			Self::Changed => "Changed",
			Self::AllOptions => "All options",
		}
	}
}

/// Option positions by path, for resolving curated entries quickly.
#[derive(Debug, Clone, Default)]
pub struct SchemaIndex(HashMap<OptionPath, usize>);

impl SchemaIndex {
	#[must_use]
	pub fn new(options: &[SystemOption]) -> Self {
		Self(
			options
				.iter()
				.enumerate()
				.map(|(index, option)| (option.path.clone(), index))
				.collect(),
		)
	}

	#[must_use]
	pub fn get(&self, path: &[String]) -> Option<usize> {
		self.0.get(path).copied()
	}
}

fn split(path: &str) -> OptionPath {
	path.split('.').map(str::to_string).collect()
}

/// The path `entry` stands for on this host: the first candidate that is an
/// ordinary option a page can read and edit. Hidden aliases, templates, and
/// fields nested in a submodule are skipped; they stay reachable through
/// search and the namespace tree.
#[must_use]
pub fn resolve(entry: &Entry, options: &[SystemOption], index: &SchemaIndex) -> Option<OptionPath> {
	entry.paths.iter().map(|raw| split(raw)).find(|path| {
		index.get(path).is_some_and(|at| {
			let option = &options[at];
			!option.diagnostic && !option.template && !option.nested
		})
	})
}

/// A curated entry resolved to this host's option path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
	pub path: OptionPath,
	pub label: &'static str,
	pub description: &'static str,
}

/// A section with the entries this host declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownSection {
	pub title: &'static str,
	pub entries: Vec<Shown>,
}

/// `page`'s sections with the entries this host declares. Empty sections
/// are dropped.
#[must_use]
pub fn page_sections(
	page: SettingsPage,
	options: &[SystemOption],
	index: &SchemaIndex,
) -> Vec<ShownSection> {
	page.sections()
		.iter()
		.filter_map(|section| {
			let entries: Vec<Shown> = section
				.entries
				.iter()
				.filter_map(|entry| {
					Some(Shown {
						path: resolve(entry, options, index)?,
						label: entry.label,
						description: entry.description,
					})
				})
				.collect();
			(!entries.is_empty()).then_some(ShownSection {
				title: section.title,
				entries,
			})
		})
		.collect()
}

/// Every path a set of sections shows, at most [`GROUP_LIMIT`].
#[must_use]
pub fn section_paths(sections: &[ShownSection]) -> Vec<OptionPath> {
	sections
		.iter()
		.flat_map(|section| section.entries.iter().map(|entry| entry.path.clone()))
		.take(GROUP_LIMIT)
		.collect()
}

/// The page, section, and entry that name `path`, when one does.
#[must_use]
pub fn curated(path: &[String]) -> Option<(SettingsPage, &'static Section, &'static Entry)> {
	let joined = path.join(".");
	SettingsPage::ALL.into_iter().find_map(|page| {
		page.sections().iter().find_map(|section| {
			section
				.entries
				.iter()
				.find(|entry| entry.paths.contains(&joined.as_str()))
				.map(|entry| (page, section, entry))
		})
	})
}

/// A plain name for `path`: its curated label, else the dotted path.
#[must_use]
pub fn label_for(path: &[String]) -> String {
	curated(path).map_or_else(
		|| option_path_display(path),
		|(_, _, entry)| entry.label.to_string(),
	)
}

/// The most option matches a search returns.
pub const SEARCH_LIMIT: usize = 200;

/// What a search found: curated settings first, then schema options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
	pub settings: Vec<(SettingsPage, Shown)>,
	/// Indices into the schema, at most [`SEARCH_LIMIT`].
	pub options: Vec<usize>,
	/// How many options matched before the limit.
	pub total: usize,
}

impl SearchResults {
	#[must_use]
	pub fn len(&self) -> usize {
		self.settings.len() + self.options.len()
	}

	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// The option path at `position`, counting settings first.
	#[must_use]
	pub fn path_at<'a>(
		&'a self,
		options: &'a [SystemOption],
		position: usize,
	) -> Option<&'a [String]> {
		match self.settings.get(position) {
			Some((_, shown)) => Some(&shown.path),
			None => options
				.get(*self.options.get(position - self.settings.len())?)
				.map(|option| option.path.as_slice()),
		}
	}
}

/// Searches curated labels and descriptions, then option paths and
/// descriptions, case-insensitively. An empty query finds nothing.
#[must_use]
pub fn search(
	options: &[SystemOption],
	index: &SchemaIndex,
	text: &str,
	show_diagnostic: bool,
	hardware: &HardwareFilter,
) -> SearchResults {
	let query = text.trim().to_lowercase();
	if query.is_empty() {
		return SearchResults::default();
	}
	let mut settings = Vec::new();
	for page in SettingsPage::ALL {
		for section in page.sections() {
			for entry in section.entries {
				let matches = entry.label.to_lowercase().contains(&query)
					|| entry.description.to_lowercase().contains(&query)
					|| entry
						.paths
						.iter()
						.any(|path| path.to_lowercase().contains(&query));
				if !matches {
					continue;
				}
				if let Some(path) = resolve(entry, options, index) {
					settings.push((
						page,
						Shown {
							path,
							label: entry.label,
							description: entry.description,
						},
					));
				}
			}
		}
	}
	let filtered = filter_options(options, &query, Category::All, show_diagnostic, hardware);
	let total = filtered.visible.len();
	SearchResults {
		settings,
		options: filtered.visible.into_iter().take(SEARCH_LIMIT).collect(),
		total,
	}
}

/// One row of the namespace tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
	/// A namespace holding `count` options, such as `services.pipewire`.
	Namespace {
		prefix: OptionPath,
		count: usize,
		expanded: bool,
	},
	/// An option, by schema index, at `depth` levels below the root. Its
	/// label is the path from `depth` on, which is longer than one segment
	/// when a namespace holding only this option collapsed into it.
	Option { index: usize, depth: usize },
}

impl TreeRow {
	/// How far the row is indented.
	#[must_use]
	pub fn depth(&self) -> usize {
		match self {
			Self::Namespace { prefix, .. } => prefix.len() - 1,
			Self::Option { depth, .. } => *depth,
		}
	}
}

/// The rows of the namespace tree over `visible`, opening the namespaces in
/// `expanded`. A namespace holding a single option collapses into that
/// option's row.
#[must_use]
pub fn namespace_rows(
	options: &[SystemOption],
	visible: &[usize],
	expanded: &BTreeSet<OptionPath>,
) -> Vec<TreeRow> {
	let mut sorted = visible.to_vec();
	sorted.sort_by(|a, b| options[*a].path.cmp(&options[*b].path));
	let mut rows = Vec::new();
	push_level(options, &sorted, 0, expanded, &mut rows);
	rows
}

fn push_level(
	options: &[SystemOption],
	run: &[usize],
	depth: usize,
	expanded: &BTreeSet<OptionPath>,
	rows: &mut Vec<TreeRow>,
) {
	let mut start = 0;
	while start < run.len() {
		let path = &options[run[start]].path;
		let Some(segment) = path.get(depth) else {
			rows.push(TreeRow::Option {
				index: run[start],
				depth,
			});
			start += 1;
			continue;
		};
		let end = start
			+ run[start..]
				.iter()
				.take_while(|index| options[**index].path.get(depth) == Some(segment))
				.count();
		let group = &run[start..end];
		if let [only] = group {
			rows.push(TreeRow::Option {
				index: *only,
				depth,
			});
		} else {
			let prefix = path[..=depth].to_vec();
			let open = expanded.contains(&prefix);
			rows.push(TreeRow::Namespace {
				prefix,
				count: group.len(),
				expanded: open,
			});
			if open {
				push_level(options, group, depth + 1, expanded, rows);
			}
		}
		start = end;
	}
}

/// A tree row's name: the namespace's last segment, or an option's path
/// from its depth on.
#[must_use]
pub fn tree_label(options: &[SystemOption], row: &TreeRow) -> String {
	match row {
		TreeRow::Namespace { prefix, .. } => prefix.last().cloned().unwrap_or_default(),
		TreeRow::Option { index, depth } => options
			.get(*index)
			.map(|option| option_path_display(&option.path[(*depth).min(option.path.len())..]))
			.unwrap_or_default(),
	}
}

/// Every namespace above `path`, so expanding them reveals it.
#[must_use]
pub fn ancestors(path: &[String]) -> Vec<OptionPath> {
	(1..path.len()).map(|len| path[..len].to_vec()).collect()
}

/// Paths with edits waiting: staged here or queued for the NixOS rebuild.
#[must_use]
pub fn changed_paths(
	staged: &BTreeMap<OptionPath, Option<SettingValue>>,
	session: &Session,
) -> Vec<OptionPath> {
	let mut paths: BTreeSet<OptionPath> = staged.keys().cloned().collect();
	for op in &session.queue {
		if let crate::Op::SetOptions { changes, scope } = op
			&& *scope == Target::NixosSystem
		{
			paths.extend(changes.iter().map(|change| change.path.clone()));
		}
	}
	paths.into_iter().collect()
}

/// Stages `value` for `path`, or an unset when `value` is `None`. Staging
/// what `baseline` already holds drops the edit instead, and unsetting a
/// value nixbox does not own only drops a staged edit. Returns the status
/// line describing what happened.
pub fn stage_edit(
	staged: &mut BTreeMap<OptionPath, Option<SettingValue>>,
	path: &[String],
	value: Option<SettingValue>,
	baseline: Option<&SettingValue>,
	ours: bool,
) -> String {
	let name = option_path_display(path);
	match value {
		Some(value) if Some(&value) == baseline => {
			staged.remove(path);
			format!("{name} is back to {}.", value.to_nix())
		}
		Some(value) => {
			let text = format!("Staged {name} = {}.", value.to_nix());
			staged.insert(path.to_vec(), Some(value));
			text
		}
		None if ours => {
			staged.insert(path.to_vec(), None);
			format!("Staged unset {name}.")
		}
		None => {
			staged.remove(path);
			format!("Dropped the staged change to {name}.")
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn option(path: &str) -> SystemOption {
		SystemOption {
			path: split(path),
			configured: Some(false),
			description: Some(format!("{path} description")),
			type_description: "boolean".into(),
			declarations: Vec::new(),
			diagnostic: false,
			read_only: false,
			nested: false,
			template: false,
			children_unavailable: false,
			children_truncated: false,
		}
	}

	#[test]
	fn curated_pages_stay_within_one_evaluation_and_name_each_path_once() {
		let mut seen = BTreeSet::new();
		for page in SettingsPage::ALL {
			let count: usize = page.sections().iter().map(|s| s.entries.len()).sum();
			assert!(count <= GROUP_LIMIT, "{} has {count} entries", page.label());
			for section in page.sections() {
				for entry in section.entries {
					for path in entry.paths {
						assert!(seen.insert(*path), "{path} is curated twice");
					}
				}
			}
		}
	}

	#[test]
	fn entries_resolve_to_the_newest_declared_ordinary_path() {
		let mut alias = option("services.xserver.layout");
		alias.diagnostic = true;
		let options = vec![alias, option("services.xserver.xkb.layout")];
		let index = SchemaIndex::new(&options);
		let sections = page_sections(SettingsPage::General, &options, &index);
		assert_eq!(
			sections.len(),
			1,
			"Sections without declared options drop out"
		);
		assert_eq!(
			sections[0].entries[0].path,
			split("services.xserver.xkb.layout")
		);
		let options = vec![option("services.xserver.layout")];
		let index = SchemaIndex::new(&options);
		let sections = page_sections(SettingsPage::General, &options, &index);
		assert_eq!(
			sections[0].entries[0].path,
			split("services.xserver.layout"),
			"Older releases fall back to the older path"
		);
		let mut nested = option("nix.settings.substituters");
		nested.nested = true;
		let options = vec![nested];
		let index = SchemaIndex::new(&options);
		assert!(
			page_sections(SettingsPage::Nix, &options, &index).is_empty(),
			"Submodule fields cannot be read yet, so pages leave them to search"
		);
	}

	#[test]
	fn search_lists_curated_settings_before_schema_options() {
		let options = vec![
			option("services.printing.enable"),
			option("services.printing.drivers"),
		];
		let index = SchemaIndex::new(&options);
		let results = search(&options, &index, "print", false, &HardwareFilter::default());
		assert_eq!(results.settings.len(), 1);
		assert_eq!(results.settings[0].1.label, "Printing");
		assert_eq!(results.options.len(), 2);
		assert_eq!(
			results.path_at(&options, 0),
			Some(options[0].path.as_slice())
		);
		assert_eq!(
			results.path_at(&options, 2),
			Some(options[1].path.as_slice())
		);
		assert!(search(&options, &index, "  ", false, &HardwareFilter::default()).is_empty());
	}

	#[test]
	fn namespace_tree_folds_namespaces_and_collapses_single_options() {
		let options = vec![
			option("services.pipewire.enable"),
			option("networking.hostName"),
			option("services.pipewire.pulse.enable"),
			option("services.openssh.enable"),
			option("zramSwap"),
		];
		let visible: Vec<usize> = (0..options.len()).collect();
		let rows = namespace_rows(&options, &visible, &BTreeSet::new());
		assert_eq!(
			rows,
			[
				TreeRow::Option { index: 1, depth: 0 },
				TreeRow::Namespace {
					prefix: split("services"),
					count: 3,
					expanded: false
				},
				TreeRow::Option { index: 4, depth: 0 },
			],
			"networking holds one option, so it shows as that option"
		);
		let expanded: BTreeSet<OptionPath> = ancestors(&split("services.pipewire.pulse.enable"))
			.into_iter()
			.collect();
		let rows = namespace_rows(&options, &visible, &expanded);
		assert_eq!(rows.len(), 7);
		assert_eq!(rows[2], TreeRow::Option { index: 3, depth: 1 });
		assert!(matches!(
			&rows[3],
			TreeRow::Namespace {
				count: 2,
				expanded: true,
				..
			}
		));
		assert_eq!(rows[4], TreeRow::Option { index: 0, depth: 2 });
		assert_eq!(rows[5], TreeRow::Option { index: 2, depth: 2 });
		assert_eq!(tree_label(&options, &rows[5]), "pulse.enable");
		assert_eq!(tree_label(&options, &rows[3]), "pipewire");
	}

	#[test]
	fn staging_drops_edits_that_match_the_baseline() {
		let mut staged = BTreeMap::new();
		let path = split("custom.enable");
		stage_edit(
			&mut staged,
			&path,
			Some(SettingValue::Bool(true)),
			None,
			false,
		);
		assert_eq!(staged.get(&path), Some(&Some(SettingValue::Bool(true))));
		stage_edit(
			&mut staged,
			&path,
			Some(SettingValue::Bool(false)),
			Some(&SettingValue::Bool(false)),
			false,
		);
		assert!(staged.is_empty());
		stage_edit(&mut staged, &path, None, None, false);
		assert!(
			staged.is_empty(),
			"Unsetting a value nixbox does not own is a no-op"
		);
		stage_edit(&mut staged, &path, None, None, true);
		assert_eq!(staged.get(&path), Some(&None));
	}
}
