# Alienware x17 R1: every device, who owns it, where it stands (2026-11-19)

Thread "Polish x17-specific hardware support" owns this list. Every device on the machine
has an owner thread; when a row says "this thread", the work is tracked here. Device ids
marked "verify" come from Dell's spec sheet and Linux reports; the first real boot's
Spindle `hw` report (runbook: `X17_BARE_METAL_RUNBOOK_2026_11_18.md`) confirms them.

| Device | Id (expected) | Owner | State |
|---|---|---|---|
| CPU, Tiger Lake-H (i7-11800H / i9-11980HK) | PKU, HWP | core | Works in QEMU; HWP EPP power profiles (PR #179) |
| NVMe x2, Intel VMD / RST | 8086:9a0b | "Prepare Alienware x17 for bare metal" | PR #432 (MDTS/4K, VMD MSI remap follow-up) |
| Wi-Fi, Killer AX1675 (AX210/211) | 8086:2725 | "Bring up Intel Wi-Fi" (done) | W1-W5 merged; first real-hardware run pending |
| Bluetooth (same card, USB) | 8087:0032 (verify) | "Expand real-PC hardware support" | sex-bt host stack; real controller pending |
| Ethernet, Killer E3100G (RTL8125) | 10ec:8125 | "Expand real-PC hardware support" | |
| Intel UHD iGPU (Xe, TGL GT1) | 8086:9a60 | "Expand real-PC hardware support" | GOP framebuffer today |
| NVIDIA RTX 3070 Laptop (GA104) | 10de:249d / 24dd | "Bring up the NVIDIA RTX 3070" | Boot power-off (D3cold) merged; GSP bring-up next |
| eDP panel (FHD 165/360 Hz or UHD 120 Hz) and MUX | EDID | this thread | GOP mode only; see "Panel and MUX" |
| HDMI 2.1, mini-DP, USB-C DP (on the dGPU) | | "Bring up the NVIDIA RTX 3070" (N5) | |
| HDA controller + Realtek ALC3281 codec | 8086:43c8, 10ec:0289 family (verify) | "Expand real-PC hardware support" | Generic HDA works; Dell 1028:xxxx pin/EAPD quirks there |
| xHCI (PCH + Thunderbolt 4 TCSS) | 8086:43ed, 8086:9a13 | "Expand real-PC hardware support" | |
| Internal keyboard (i8042) | PS/2 | sexinput | Works; media keys mapped |
| Fn hotkeys | ACPI video notify, Dell WMI 9DBB5994 | this thread | Brightness via ACPI video (PR #179); Dell WMI events decoded (this PR) |
| Keyboard and chassis lighting (AlienFX) | USB HID 187c:05xx (verify) | this thread | Not started: needs the USB ids from `hw`; firmware keeps the last colours |
| Touchpad, I2C HID on Serial IO | PNP0C50 | PR #179 + "Expand real-PC hardware support" | Described and polled; GPIO interrupt pending |
| Webcam (FHD + IR, USB UVC) | USB 0bda / 0c45 (verify) | this thread | `sex-uvc` protocol + `sexuvc` server run on a synthetic camera; no isochronous USB transfers yet |
| Fans and temperatures (AWCC) | WMI A70591CE (`WMAX`) | this thread | This PR: fans, sensors, profiles read and logged |
| Thermal profiles (quiet / balanced / performance) | AWCC method 0x15 | this thread | This PR: follow the desktop's power profile |
| Battery, AC, lid | PNP0C0A, ACPI0003, PNP0C0D | PR #179 (done) | Works in QEMU |
| Battery charge limits | Dell SMBIOS tokens 0x0341-0x034A | this thread | PR 2: read and set through `CTL_CHARGE`; desktop control pending |
| Suspend to idle | LPS0 _DSM | PR #179 (done) | Devices to D3 while asleep still open |

## This PR: WMI, Alienware fans and profiles, Dell hotkeys

- `crates/sex-laptop/src/wmi.rs`: `_WDG` parsing, GUID constants, `WMxx` names and event
  notify ids. Shared with the AMD laptop thread for ASUS/Lenovo WMI later.
- `crates/sex-laptop/src/awcc.rs`: the Alienware Command Center protocol (system
  description, resource ids, fan rpm/min/max, temperatures, current profile, activate
  profile, fan boost). Linux's `alienware-wmi-wmax` lists the "Alienware x17" with fan
  monitoring and platform profiles, so the x17 R1 is expected to answer.
- `crates/sex-laptop/src/dell_wmi.rs`: Dell hotkey records from `_WED(0xD0)`.
- `servers/sexacpi/src/wmi.rs`: finds `PNP0C14` devices, calls `WMAX`, reads `_WED`.
  Boot lines: `[sexacpi.wmi]`, `[sexacpi.awcc]`, `[sexacpi.awcc.fan]`, `[sexacpi.awcc.temp]`,
  `[sexacpi.awcc.profile]`, `[sexacpi.awcc.reading]` (about once a minute),
  `[sexacpi.awcc.set]`, `[sexacpi.hotkey]`, `[sexacpi.vendor]`.
- Kernel: `CTL_PROFILE` is also queued to the ACPI service, so Power Saver puts the fans in
  the firmware's quiet profile and Performance in its performance profile.
- AWCC sensors raise the reported temperature when hotter than every ACPI zone (the CPU
  sensor usually is); the critical trip stays the firmware's `_CRT`.
- vendor/acpi: `DerefOf (PKG [i])` failed for every package lookup, and buffer fields could
  not be compared or used in arithmetic. Real DSDTs (and every WMI method that takes its
  input apart with `CreateByteField`) need both. Fixed and covered by
  `crates/sex-laptop/tests/awcc_aml.rs`, which runs real AML through the interpreter.
- Gate: `scripts/laptop_power_gate.py` boots with a pretend x17 WMI device in
  `tests/acpi/laptop_gate_ssdt.asl`.

Not yet acted on: mic mute and the Command Center key are decoded and logged, not routed
to Bell or the desktop; manual fan boost has no desktop control yet.

## PR 2: SMBIOS and battery charge modes

- Kernel: Limine's SMBIOS request; the structure table is copied at boot (through a
  read-only mapping when the HHDM leaves firmware memory out) and served as one more
  `FW_TABLE` entry signed `SMBI`. Boot line `[acpi.smbios]`.
- `sex_laptop::smbios`: entry points and structures; vendor, product, SKU, BIOS version;
  `is_alienware_x17_r1`. Boot line `[sexacpi.smbios] ... x17r1=1` on the real machine.
- `sex_laptop::dell_smbios`: Dell token table (type 0xDA), the WMI calling interface
  buffer, the descriptor block, charge modes (standard, express, adaptive, primarily AC,
  custom 50-95 / 55-100, 5 apart). Boot lines `[sexacpi.charge]`, `[sexacpi.charge.state]`.
- `CTL_CHARGE` (power control 6): value 0-4 = mode, 55-100 = custom mode stopping at that
  percent and starting 10 lower. The kernel validates and queues it for the ACPI service.
  No desktop control sends it yet; a Settings/launcher entry is the next step.
- Tested: `crates/sex-laptop/tests/awcc_aml.rs` runs a Dell WMI device in AML (read, custom
  70-80%, adaptive); the laptop power gate gives QEMU an Alienware x17 R1 type 1 and a
  Dell token table and checks the boot lines.

## Panel and MUX

The x17 R1 panel runs at its native refresh only once a display driver sets the mode; with
GOP the firmware's mode (usually the panel's preferred timing) stays. With the BIOS
"Advanced Optimus"/discrete setting, the eDP is wired to the NVIDIA GPU and the GOP
framebuffer belongs to it, which the dGPU power-off already refuses to touch. Keep
Hybrid graphics for the first boots (runbook). Next here: print the panel's EDID name,
native mode and refresh in the `hw` report so we know which of the three panels it is.

## Next in this thread, in order

1. Desktop control for charge mode (Settings or launcher, sends `CTL_CHARGE`).
2. Webcam: isochronous IN endpoints in sexusb's xHCI, then bind `sexuvc` to a real UVC
   interface.
3. AlienFX lighting once the USB ids are known.
4. Panel EDID line in `hw`; desktop controls for thermal profile and fan boost.
