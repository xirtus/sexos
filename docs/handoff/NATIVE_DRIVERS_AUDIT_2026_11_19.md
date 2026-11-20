# Native drivers audit and week plan (2026-11-19)

Question (Andreas, 2026-11-19): are SexOS drivers mostly Linux drivers adapted
through Tuxedo? Is that a slowdown native drivers would fix? Should we reverse
engineer (REA) native drivers, x17 R1 first, then the most common PC devices?

Audited master at `128f08c8`, plus the open driver PRs #432, #438, #445, #449.

## 1. Short answer

**No driver runs through Tuxedo. Every SexOS driver is already native Rust.**

- `servers/tuxedo` is a 68-line placeholder (`init()` is empty, `_start` loops) with
  no callers. `docs/BRIDGE_LEDGER.md` table 1 has no rows, and the guardrail test
  `scripts/tests/test_kernel_design_guardrails.py` fails any crate that depends on
  `tuxedo` or is `GPL-2.0-only` without a ledger row. Nothing does.
- Some native drivers read Linux or OpenBSD source as a *reference* for register
  sequences (ledger table 2), the way you would read a datasheet. No Linux code was
  copied or runs. That is already the REA approach.
- What does come from outside and cannot be replaced: **device firmware** that runs
  on the device itself (Intel Wi-Fi/BT `.ucode`/`.sfi`, NVIDIA GSP-RM). Every OS,
  Linux included, loads the same blobs. And the BIOS's own **ACPI AML**, which the
  MIT `acpi` crate interprets in the ring-3 `sexacpi` server.

So there is no Linux-bridge slowdown to remove. The slowdowns that exist are in our
**own native drivers**, which were written simple-first: polled instead of
interrupt-driven, one command at a time instead of deep queues, firmware framebuffer
instead of a real display engine. Those are the things worth a week.

## 2. Alienware x17 R1 devices

| Device | Driver (all native) | State on master | Real performance cost today |
|--------|--------------------|-----------------|-----------------------------|
| Samsung NVMe (PM9A1 class) | `apps/sexdrive` | Driven, MSI-X completions | One I/O queue of 16 entries, at most 2 commands in flight, 128 KiB each (#432). A fast SSD wants 32+ in flight per queue and a queue per core. Small random reads are capped far below what the drive can do (estimate, not measured on hardware). |
| Intel VMD / RST RAID | `crates/sex-vmd`, `sex-imsm`, `kernel/src/vmd.rs` | Planned in census; VMD interrupts in #432 | n/a until it runs |
| Killer AX1675 Wi-Fi (AX210/AX211) | `crates/sex-wifi`, `servers/sexnet/src/wifi.rs` (OpenBSD `iwx` as reference) | In progress: firmware, scan, join, WPA2 written; not proven on a real card | Polled with the rest of sexnet (below) |
| Intel Bluetooth (8087:0032) | `crates/sex-bt`, `servers/cufflinks`, `sexusb` | Written; Intel firmware load untested on hardware | Rides xHCI interrupts |
| RTX 3070 Laptop (GA104) | `crates/sex-gpu-nv`, `servers/sexgpu-nv` (#449) | Firmware framebuffer today; GSP boot in progress (thread "Bring up the NVIDIA RTX 3070") | No acceleration yet; the panel is drawn by the CPU |
| Intel UHD (Tiger Lake-H) | `servers/sexdisplay` `intel.rs`, `crates/sex-gpu-intel` | Firmware framebuffer (GOP); EDID and readout native; no native modeset | CPU blits into a fixed GOP mode; no page flips, vsync, planes or native resolution change |
| Realtek HDA codec (ALC3281/1309) | `servers/sexbell` `hda.rs` | Driven; mic in #438 | **No interrupts** (manifest `vectors: 0`): Bell wakes every tick while playing |
| USB 3.2 / TB4 xHCI | `servers/sexusb` | Driven, MSI-X | Fine |
| TB4 NHI | none | Planned | n/a |
| I2C-HID touchpad | `crates/sex-i2c-dw`, `sex-hid-touch`, `servers/sexinput` | In progress | Polled (manifest `vectors: 0`) |
| Killer E3100G 2.5 GbE (RTL8125) | `servers/sexnet` `realtek.rs` | Driven | **Polled**: see section 3 |
| Webcam (UVC) | `servers/sexusb` `uvc_scan.rs`, `servers/sexuvc` | Detection in #445, isochronous transfers in progress | n/a yet |
| EC / fans / power profiles | `crates/sex-laptop` `awcc`, `servers/sexacpi` `wmi.rs` | In #445 (Dell WMI/AWCC through AML) | Fine (rare calls) |
| AlienFX lighting | none | Needs the USB ids from the first boot | n/a |
| Realtek card reader RTS5260-class | none | Planned | n/a |

## 3. Where native drivers lose real performance (evidence)

1. **Networking is polled.** `crates/sex-hwdb/src/manifest.rs` gives `Service::Net`
   `vectors: 0`, so BOOT programs no MSI for any NIC and `realtek.rs` /`nic.rs` mask
   every interrupt. With a link up, `servers/sexnet` loops: at most `NET_BURST` = 32
   frames per turn, then, with no IPC message waiting, `sys_wait_self_ticks(2)`
   (`[sexnet.receive.wait] ... kind=timer_assisted_poll irq=0`). The tick is 1 ms
   (LAPIC calibrated to ticks/ms, `kernel/src/apic.rs`). So:
   - receive latency gains up to 2 ms per packet;
   - a busy download stops for 2 ms after every 32 frames even while frames are
     still waiting: about 16,000 frames/s, roughly 190 Mbit/s with full-size frames,
     well under 1 GbE and under a tenth of the x17's 2.5 GbE port (reasoned from the
     loop, not measured on hardware);
   - an idle machine with a cable in still wakes sexnet 500 times a second, which
     costs laptop battery.
2. **NVMe queue depth** (section 2). The limits plan (`LIMITS_AND_PERFORMANCE_PLAN_2026_11_08.md`
   item 15 and "Storage throughput") already targets QD 32 by interrupt, then a queue
   per core, within 2x of Linux raw NVMe.
3. **Sound and touchpad are polled** (`vectors: 0` for Bell and Input).
4. **Display is a firmware framebuffer** on both GPUs. Native Intel modeset (planes,
   flips) is the biggest visible speed-up on the panel; NVIDIA needs GSP first.

## 4. Week plan (x17 first, then the most common PCs)

Owner column names the thread that already holds that area; this thread does the
items marked "this thread" and hands the rest over instead of duplicating.

| # | Item | Why | Owner |
|---|------|-----|-------|
| 1 | NIC interrupts (MSI/MSI-X) for Realtek r8169/r8125 and Intel e1000e/igb/igc, and no 2 ms stall while frames are waiting | Section 3.1: latency, 2.5 GbE throughput, idle battery | this thread: done, `docs/handoff/ledger/2026-11-19-nic-interrupts.md` |
| 2 | Bell HDA interrupts (stream buffer-complete + RIRB) | Idle wakeups while playing, lower audio latency | its own thread (2026-11-19) |
| 3 | NVMe: queue depth 32 by interrupt, then one queue per core | Section 3.2 | this thread, built on #432 |
| 4 | Touchpad I2C-HID: interrupt by GPIO/MSI instead of polling | Input latency, battery | its own thread (2026-11-19) |
| 5 | Intel Tiger Lake display: native modeset and page flips (`servers/sexgpu-intel`) | Biggest visible panel speed-up; also needed for external monitors | its own thread (2026-11-19) |
| 6 | Wi-Fi: prove firmware load, scan, join on the AX210 on hardware | Only remaining x17 network gap | Wi-Fi workstream / "Polish x17-specific hardware support" |
| 7 | NVIDIA GSP boot | dGPU | "Bring up the NVIDIA RTX 3070" (#449) |
| 8 | Common PCs: Intel I219 PHY quirks (pch_spt+), MediaTek MT7921/MT7922 Wi-Fi, Realtek RTL8852 Wi-Fi, Intel Xe (Alder/Raptor Lake) display, Synaptics/ELAN touchpads, USB audio class, AMD (separate thread) | Most common laptop parts we do not drive yet | split once 1-4 land; AMD parts belong to "Support AMD laptops and GPUs" |

Every item is new native Rust, with Linux/BSD/vendor source used only as a reference
and listed in ledger table 2. No Tuxedo bridge is planned.
