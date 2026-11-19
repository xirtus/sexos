# Device power management, part 1: link power states and the sleep notice (2026-11-19)

Goal: devices sleep when idle and the whole machine suspends, x17 R1 first. What
already existed before this work: NVMe APST (the idle SSD enters its own low-power
states, `apps/sexdrive/src/apst.rs`), the unused NVIDIA GPU in D3cold (`kernel/src/hal/dgpu.rs`
+ sexacpi), and suspend to idle that reaches the firmware's LPS0 `_DSM`
(`LAPTOP_POWER_2026_11_13.md`). Missing: link power states, devices going down when the
machine sleeps, and a way for drivers to power their device down.

## What this part adds

| Part | Where | What it does |
| --- | --- | --- |
| Link power policy | `crates/sex-pcipm` (`aspm`) | For every PCIe link: ASPM L0s/L1, the L1.1/L1.2 substates (ASPM and PCI-PM), clock PM (CLKREQ#) and LTR. Only states both ends support and every endpoint below accepts (exit latency against Device Capabilities, +1 us per switch for L1). Adds only: what firmware enabled stays, and firmware's own L1-substate timing is kept. L1.2 timing as the spec says (T_POWER_ON and Common_Mode_Restore of both ends, LTR_L1.2_THRESHOLD = 2 + 4 + CM + T_POWER_ON us). Quirks: Intel 8257x/82598 and Intel Wi-Fi no L0s, Realtek Ethernet no L0s/L1.2, ASMedia 1080 nothing. Pre-1.1 devices and PCIe-to-PCI bridges are left alone. |
| D-states | `crates/sex-pcipm` (`pm`) | D0 <-> D3hot with the configuration a reset loses saved and written back (header, PCIe controls, MSI, MSI-X control, L1SS, LTR), 10 ms transition waits, PME status never written as one. |
| BOOT pass | `kernel/src/pci_power.rs`, `devmgr::init` | Maps each function's ECAM page (needed above 0x100), runs the policy before any lease, logs `[pci.aspm] link=... support= before= after= l12_threshold_us= skip= lnkctl=` per link and `[pci.aspm.summary]`. The FADT `IAPC_BOOT_ARCH` "no ASPM" bit turns it into a report only. Devices behind VMD are left as firmware set them. |
| Driver D-state op | syscall 86 `OP_POWER` (6) | A driver moves a device BOOT leased it to D3hot and back (`sex_pdx::device::set_power`). The kernel keeps the saved configuration and the MSI-X table entries across a reset. |
| Sleep notice | `pci_power::tell_drivers`, `sex_pdx::power::OP_DEVICE_SLEEP` (0x50F3) | When the desktop sleeps (`CTL_SLEEP`) every service holding a device lease hears it (before the firmware's LPS0 entry; on wake after its exit). sexdrive flushes the disk, so an empty battery while asleep loses nothing; USB, network, sound, input and display acknowledge it (their power-down comes in part 2). |

ABI: syscall 86 op 6 `OP_POWER` (+ `D0`, `D3HOT`, `ERR_POWER`), message `OP_DEVICE_SLEEP`
0x50F3 (kernel to lease holders, no reply). No new syscall number. Kernel growth: about 155
lines (`pci_power.rs`, config access and records only; policy is host-tested in the crate).

## Tests

- `cargo test -p sex-pcipm` (18, CI): root port + SSD get every state with Linux's L1.2
  timing; second pass changes nothing; FADT flag; endpoint latency limits; firmware
  substates kept; Realtek quirk; pre-1.1/bridges skipped; empty slot; L1.2 needs LTR;
  a switch with two downstream ports (latency adds up, LTR follows the path); threshold
  encoding against Linux's; D3hot round trip restoring a reset function; No_Soft_Reset.
- `scripts/laptop_power_gate.py` (CI): the NVMe now sits behind a PCIe root port; checks
  the link gets L0s (all QEMU offers), the summary, and that closing the lid tells the
  drivers (`[pci.power.sleep] asleep=1 drivers_told=N`, `[sexdrive.sleep] asleep=1 flush=0x0`,
  `[sexusb.sleep]`) and opening it does again.

## On the x17 R1, look for

1. `[pci.aspm.summary] links=N changed=M ... ecam_pages=K fadt_no_aspm=0`. If `fadt_no_aspm=1`
   the firmware forbids OS ASPM and only its own settings apply.
2. One `[pci.aspm]` line per link: the Samsung SSD, Wi-Fi, Killer E3000 Ethernet
   (`10ec:3000`, L1 + L1.1 only by quirk), the RTX 3070 link (`10de:...`). `after=` should
   include `l1+l1.1+l1.2+pm1.1+pm1.2` where both ends offer substates, and `lnkctl=2/2`
   (or 3/3) read back.
3. Closing the lid: `[pci.power.sleep] asleep=1 drivers_told=...` then `[sexdrive.sleep]
   asleep=1 flush=0x0`, `[sexinput.touchpad.power] asleep=1 ok=1`, `[bell.hda.power] asleep=1
   groups=N reached=N` and `[display.intel.sleep] asleep=1 edp=1 panel_power=1 backlight=1`.
   Opening it: the same lines with `asleep=0`, and the screen and touchpad come back.
4. If anything misbehaves after boot (a device vanishing, NVMe timeouts), note the link
   line for it: a quirk entry in `sex_pcipm::aspm::quirk` turns the state off for that id.

## Part 2: devices power down while asleep

On the sleep notice each driver now turns its own hardware down, and back up on wake:

| Driver | While asleep | Log |
|---|---|---|
| Touchpad (sexinput) | HID-over-I2C `SET_POWER` sleep, not polled; `SET_POWER` on at wake | `[sexinput.touchpad.power] asleep= ok=` |
| USB (sexusb, `servers/sexusb/src/sleep.rs`) | Root ports whose device only moves data on request (storage, idle devices) go to U3: cache flushed (SYNCHRONIZE CACHE), bulk endpoints stopped with the suspend flag, link to U3. Keyboards, pointers, hubs, the network adapter and Bluetooth stay up. Wake: USB 2 ports get 20 ms of resume signalling, then U0. A disk request while suspended resumes first. | `[sexusb.suspend] port= slot= id= storage= flushed= ok=`, `[sexusb.resume] port= ok=`, `[sexusb.sleep] ... ports=` |
| Sound (sexbell) | Every HDA audio function group with an output or microphone to D3 (`SET_POWER` 0x705), checked with `GET_POWER`; D0 at wake. Widget settings are kept. | `[bell.hda.power] asleep= groups= reached=` |
| Display (sexdisplay, Intel) | eDP backlight PWM off (brightness kept by `sex_gpu_intel::backlight::sleep`/`wake`), panel to DPCD D3 (0x600 = 2); back at wake | `[display.intel.sleep] asleep= edp= panel_power= backlight=` |

The laptop gate now has a USB stick on the xHCI controller and checks its suspend and
resume around the lid close. QEMU has no HDA codec, eDP or I2C touchpad in that gate, so
those three are proven on the x17 by their log lines.

## Later

- Thunderbolt: the GO2SX / GO2SX_EXIT mailbox (VS_CAP_19 commands 0x02/0x03) around sleep,
  next to the force-power code from the Thunderbolt work (PR #455).
- USB controller to D3hot (xHCI save/restore state, CSS/CRS) when nothing needs it.
- HDA codec D3 when nothing plays (not only asleep); the controller to D3hot on sleep.
- Panel self refresh / pipe off when the screen is dark (with the Intel display work).
- A slower kernel tick while asleep, and reading the PMC's S0ix residency so a sleep that
  never reaches S0ix is visible.
- S3 suspend to RAM for machines whose firmware has `_S3` (the x17 R1 is Modern Standby only).
