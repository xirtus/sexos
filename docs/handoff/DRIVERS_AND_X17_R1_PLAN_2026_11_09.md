# Drivers: open source sources, the Alienware x17 R1 plan, and the sex-manual

Date: 2026-11-09. Thread: "Plan drivers for the Alienware x17 R1".
Ledger entry: "Hardware census, VMD warning, NVMe waits for real SSDs" in
`SEXOS_FEATURE_LEDGER_2026_11_02.md`.

## 1. In short

- The x17 R1 was only ever sold with a **laptop RTX 3060, 3070 or 3080 (16 GB)**. There
  was no RTX 3090 in any x17 (the 3080 Ti arrived in the x17 R2). Andreas's machine is
  most likely the **RTX 3070 Laptop (GA104, 8 GB)**; `lspci -nn` should show 10de:249d or
  10de:24dd. The 3070 and 3080 are the same GA104 chip, so the plan is the same.
- The biggest risk for next week is not the GPU. It is **storage mode**. Many x17 R1s
  shipped with two NVMe drives in Intel RAID 0 ("RAID On", Intel RST). In that mode the
  drives sit behind Intel VMD and no NVMe controller shows up on PCI, so SexOS has no
  disk. Boot now says so in plain words (`[hw.warning] ... Intel VMD/RST RAID is on`).
- On the first boot SexOS should get: picture (firmware framebuffer), USB keyboard and
  mouse, the internal keyboard if it is PS/2, NVMe (if not behind VMD), power off. In
  open branches: wired Ethernet (Killer E3100G is a Realtek RTL8125), headphone sound
  (Intel HDA), touchpad (I2C HID, needs ACPI), native panel resolution. Not yet planned
  to work: Wi-Fi, Bluetooth, the NVIDIA GPU itself, webcam, card reader, RGB.
- This PR adds a **hardware census** (one line per PCI device: name, owning service,
  support level, warnings), fixes **NVMe waits for real SSDs** (they were poll counts
  of a few milliseconds; now CAP.TO in clock ticks, up to seconds), honours the NVMe
  **doorbell stride**, logs **4 KiB-sector namespaces** clearly, and makes the
  **first NVMe** the system disk instead of whichever was enumerated last.
- The rest of this report: the machine (2), what to set up before the test (3), the
  driver priority list (4), open source sources and what REA and Omarchy are (5), how
  to grow the driver framework to many systems (6), and where we drifted from the
  sex-manual and how to come back (7).

## 2. The machine: Alienware x17 R1 (2021, P48E)

Sources: Dell "Setup and Specifications" (Rev. A04), Notebookcheck review of the RTX 3080
model (2021-09-25), a third-party copy of Dell's spec sheet. PCI ids are the usual ones
for these parts; confirm them with `lspci -nn` on the real machine.

| Part | What it is | Likely PCI id | SexOS today | Owner |
|---|---|---|---|---|
| CPU | Core i7-11800H or i9-11980HK, Tiger Lake-H, 8 cores, **has PKU** | n/a | runs (PKU required) | kernel |
| Chipset | Intel HM570 (Tiger Lake-H PCH), PCIe Gen4 from the CPU | 8086:43xx | platform | kernel |
| iGPU | Intel UHD Graphics (Xe, 32 EU) | 8086:9a60 | firmware framebuffer | Native resolution and HiDPI scale (i915-class modesetting) |
| dGPU | RTX 3060 6 GB (GA106) / **3070 8 GB (GA104), Andreas's machine** / 3080 16 GB (GA104), 130–165 W | 10de:2520/2560, 249d/24dd, 249c/24dc | firmware framebuffer only | new thread (section 4, P3) |
| Graphics switching | Optimus. Dell lists Advanced Optimus only for the 360 Hz panel; the review notes a dGPU-only option in BIOS | n/a | either works with the firmware framebuffer | |
| Panel | 1080p 165 Hz, 1080p 360 Hz, or 4K 120 Hz | eDP | GOP mode; native via EDID in PR #77 | display thread |
| Storage | **Two M.2 2230/2280 slots**, PCIe Gen3/Gen4 x4; RAID 0 options (review unit: 2x WD SN730 1 TB in RAID 0) | NVMe 01:08, or VMD 8086:9a0b | NVMe driven (one controller); **VMD not driven** | this thread (sexdrive) |
| Wi-Fi/BT | Killer Wi-Fi 6E AX1675 (an Intel AX210/AX211 module), BT 5.2, soldered | 8086:2725 (AX210) | none | new thread (P2) |
| Ethernet | Killer E3100G 2.5 GbE (Realtek RTL8125 family) | 10ec:3000 or 10ec:8125 | in open branch | Drive real PC network, sound and SATA |
| Audio | Intel Tiger Lake-H cAVS/HDA controller, Realtek ALC3281 codec, ALC1309 amp for woofers | 8086:43c8 | in open branch (HDA) | Drive real PC network, sound and SATA |
| Touchpad | Precision clickpad (I2C HID on Intel Serial IO, by Dell's usual design) | 8086 class 0c:80 | none | Power off and restart through ACPI (I2C/GPIO) |
| Keyboard | Per-key RGB (AlienFX), optional Cherry MX low profile | i8042 or USB, to confirm | PS/2 and USB keyboards work | Input (existing) |
| USB | 2x USB-A 3.2 Gen 1, 1x USB-C 3.2 Gen 2 (DP, PD), 1x Thunderbolt 4 | xHCI 8086:43ed, TCSS 8086:9a13 | xHCI driven | Reach Linux and desktop parity (USB) |
| Thunderbolt 4 | USB4/TB4 rear port | NHI 8086:9a1b/9a1d or Maple Ridge 8086:1137 | USB through xHCI only | later (needs IOMMU first) |
| Video out | HDMI 2.1, mini DisplayPort 1.4 (likely wired to the NVIDIA GPU) | n/a | none | dGPU work |
| Camera | 720p RGB + IR (Windows Hello), USB | USB UVC | none | P3 |
| Card reader | microSD | Realtek RTS5xxx PCIe or USB, to confirm | none | P3 (sexdrive) |
| Battery | 87 Wh, ACPI | ACPI EC | sensors in PR #70 | ACPI thread |
| Fans/thermals | Dell embedded controller | ACPI/EC | none | P3 |

## 3. Before the bare-metal test

1. **BIOS Storage mode.** Check the BIOS Storage page. If it says "RAID On", either:
   - switch it to "AHCI/NVMe". This **breaks an existing RAID 0 volume**: anything on it
     (Windows, Linux) will not be readable until it is switched back. Back up first; or
   - keep RAID and wait for the VMD driver (P1 below); SexOS will boot from USB but see
     no internal disk. A RAID 0 volume also needs Intel RST metadata (IMSM) support to
     be read, which is a further step.
2. **Secure Boot off** (Limine is not signed).
3. **Graphics: Hybrid** (Optimus). The Intel GPU drives the panel, which is the path the
   display thread is building. dGPU-only also boots (the NVIDIA GOP gives a framebuffer).
4. Bring a **USB keyboard and mouse** (both driven) and a **USB-C or USB-A Ethernet
   cable** to the Killer port once the NIC branch merges.
5. There is **no serial port**. Logs come from the boot log and stop screen on the
   framebuffer (PR #78) and from crash records. The new `[hw.device]` and `[hw.warning]`
   lines are in that log; a photo of it is the hardware report.
6. If you can boot any Linux live USB on it first: `lspci -nnk`, `cat /proc/mdstat`,
   `cat /sys/class/dmi/id/product_name` and `sudo dmesg | grep -i -E 'nvme|vmd|i8042|hid'`
   pasted into the project settle every "to confirm" above.

## 4. Driver priorities

P0 is what the first boot needs, P1 makes it usable every day, P2 is expected of a laptop,
P3 is the rest. "Owner" is an existing thread unless it says new.

| Pri | Driver | Why | State | Owner |
|---|---|---|---|---|
| P0 | NVMe on real SSDs (Samsung, WD, SK hynix, Micron, Kioxia, Phison) | system disk | driven; real-SSD waits, stride, census in this PR | this thread |
| P0 | Intel VMD detection | RAID mode hides the disks | warning in this PR | this thread |
| P0 | xHCI + HID | keyboard, mouse | driven (PR #76) | USB thread |
| P0 | PS/2 (i8042) keyboard | internal keyboard on many laptops | driven | existing |
| P0 | Firmware framebuffer | picture | works | display thread |
| P0 | ACPI power off/restart | clean shutdown | PR #70 | ACPI thread |
| P1 | Realtek RTL8125 (Killer E3100G) | network | open branch | NIC/sound/SATA thread |
| P1 | Intel HDA + Realtek codec | sound (headphones first; speakers need the ALC1309 amp path) | open branch | NIC/sound/SATA thread |
| P1 | I2C HID touchpad (DesignWare I2C, Tiger Lake GPIO) | touchpad | planned | ACPI thread |
| P1 | Intel VMD driver (bus driver that exposes the NVMe behind it) | keep RAID mode, or other Dell/HP/Lenovo laptops shipped in RST mode | planned | **new thread** |
| P1 | NVMe: MSI-X, one queue per core, second controller, APST power states, shutdown notification (CC.SHN) on power off | speed, battery, both M.2 slots, clean unsafe-shutdown counters | planned | this thread's follow-up |
| P1 | Intel i915-class modesetting (Tiger Lake Xe display) | native panel, brightness, external monitors on iGPU ports | in progress | display thread |
| P2 | Intel Wi-Fi AX200/AX201/AX210/AX211 (iwlwifi/iwx class) + WPA2/WPA3 | laptops are on Wi-Fi | planned | **new thread** |
| P2 | Bluetooth over USB (Intel btusb firmware) | headsets, mice | planned | with Wi-Fi |
| P2 | MSI/MSI-X and IOMMU (VT-d) for all drivers | interrupts instead of polling; DMA isolation | planned | **new thread** (framework, section 6) |
| P2 | USB mass storage, UVC webcam, USB audio | sticks, camera | planned | USB thread |
| P3 | NVIDIA Ampere (GA104/GA106) via GSP firmware | dGPU power off first, then HDMI/mDP output, then 3D | planned | **new thread** |
| P3 | Thunderbolt 4 NHI / USB4 | docks | needs IOMMU first | later |
| P3 | Realtek card reader, AlienFX RGB (USB HID), Dell fan control, IR camera | nice to have | planned | later |

Why NVIDIA is P3 and not P0: the RTX 30 laptop GPUs only run open drivers through
NVIDIA's GSP firmware (a ~60 MB blob that runs the GPU's own management CPU, driven over
an RPC queue). Nouveau and the new Rust driver nova both need it. That is months of work
for 3D. The useful first step is small: **power the dGPU off through ACPI** (`_PR3`/`_OFF`)
so the laptop runs cool and long on battery while the Intel GPU drives the panel. Then
GSP boot and modesetting for the HDMI/mDP ports, then Vulkan through NVK's userspace (MIT).

## 5. Where drivers come from

SexOS is GPL-3.0-only, except the crates driver services link (sex-pdx, sex-rt, libsys,
sex-hwdb and their config crates), which are `GPL-2.0-or-later OR MIT` since 2026-11-09
(`docs/LICENSING.md`). A driver service that links only those may contain Linux
GPL-2.0-only driver code and is then shipped as a GPL-2.0 program.

| Source | Licence | What we take | Notes |
|---|---|---|---|
| Linux core drivers (nvme, r8169, iwlwifi, snd-hda, i2c-hid, vmd) | GPL-2.0-only | **usable in driver services** that link only the dual-licensed crates (`docs/LICENSING.md`), and in tuxedo guests | GPL-2.0-only code cannot go into GPL-3.0-only services, so keep it to driver services and guests |
| Linux DRM: **i915, amdgpu, nouveau** | **MIT** | can be ported directly, with notices | the big GPU drivers are permissive; a real advantage for the display thread |
| nova-core / nova-drm (Rust, NVIDIA GSP) | GPL-2.0 | read only; design reference for a Rust GSP driver | supports Turing (RTX 20) and later, so Ampere is in scope |
| Mesa NVK (Vulkan for NVIDIA) | MIT | userspace Vulkan once a GSP kernel side exists | conformant Vulkan 1.4 Kepler to Blackwell |
| OpenBSD / FreeBSD / NetBSD drivers (`iwx` Wi-Fi AX200/AX210, `rge` RTL8125, `nvme`, `ahci`, `azalia` HDA, `dwiic` I2C, `ihidev`/`ims` I2C HID, `vmd`) | ISC / BSD | **can be ported directly** | the best source for Wi-Fi and the touchpad: small, readable, permissive |
| Redox drivers (nvmed, ahcid, e1000d, rtl8168d, ihdad, xhcid, ps2d, acpid) | MIT | can be ported directly | Rust, userspace, microkernel: closest in shape to ours. Redox moved its graphics API to Linux DRM in 2026 |
| Asahi (Apple GPU Rust driver, m1n1) | MIT/GPL mix | design reference: a Rust GPU driver talking to firmware over shared queues; m1n1's hypervisor tracing | the tracing idea fits us: trace the real driver in a VM, then write ours from the trace |
| rust-osdev crates (`acpi` with AML, `x86_64`, `uefi`, `xhci`, `pci_types`) | Apache-2.0/MIT | use as crates | ACPI thread already uses an AML interpreter |
| `virtio-drivers` (rcore-os) | MIT | already used for virtio sound | block, net, GPU, input also there |
| linux-firmware blobs (iwlwifi, rtl_nic, i915 DMC/GuC, NVIDIA GSP) | redistributable, per blob | ship as data files on the disk | loaded by the driver at start, never linked |

**What "REA" is.** REA ("Reverse Engineer Anything", by morluto, October 2026, MIT) is an
MCP server that lets coding agents drive Ghidra, IDA or Hopper to decompile and map
binaries; a Rust port (`rea-rust`) exists. It does not contain drivers. For us it is a
tool for the parts with no open source at all (AlienFX lighting, Dell fan tables, Killer
firmware quirks), used **clean-room**: one pass writes a spec from the binary, a separate
pass writes our driver only from the spec. The Surface "Hydra" Rust driver port (2026)
and the "Vino" Rust DisplayLink driver (June 2026) were built that way with Claude Code.

**What Omarchy changed.** Omarchy (DHH's Arch + Hyprland setup) made a set of laptops
first-class Linux machines in 2026: Dell shipped XPS 14/16 (Panther Lake) with day-one
Omarchy support, Intel and Dell upstreamed the missing audio, camera, Wi-Fi, display and
power fixes into Linux 7.x, and Omarchy 4.x added hybrid NVIDIA laptop setups and, in
4.5 (November 2026), Apple Silicon. It is not a driver source either. What it gives us is
better **quirk data**: the HDA pin fixups, ACPI quirks and Wi-Fi firmware fixes those
vendors now upstream are exactly the facts our drivers need for those laptops, and the
hardware list Omarchy users run is a good list of machines to support next.

## 6. Growing the driver framework to many systems

What we have now (all working, and all deliberately simple):

- The kernel's `devmgr` walks PCI once and hands each device to a service by a
  hard-coded `match` on class (NVMe to sexdrive slot 16, xHCI to sexusb, NIC to sexnet,
  virtio sound to sexbell). Device id lists live in kernel files (`nic_boot.rs`,
  `virtio_boot.rs`).
- One device per kind. A second NVMe used to replace the first; now it is logged and left.
- Drivers **poll**. Interrupts are routed by fixed legacy vectors (0x22, 0x23); there is
  no MSI or MSI-X.
- DMA goes through **fixed per-service DMA pool regions** and bounce copies, because
  there is no IOMMU: a driver can program its device to write anywhere in RAM.
- PCI config through ports 0xCF8/0xCFC only (first 256 bytes, segment 0).

The plan, each stage small enough to merge on its own:

| Stage | What | Result |
|---|---|---|
| D1 | `sex-hwdb` census table (this PR) | every boot says what each device is and who drives it |
| D2 | **Driver manifests.** Each driver service declares its match table (class, vendor, device) in its catalog entry, built from `sex-hwdb`. `devmgr` becomes generic: lease each match to the declared service, many devices per service. Device id lists leave the kernel | new hardware support without kernel edits; two NVMe, two NICs |
| D3 | **MSI/MSI-X interrupt capabilities.** The kernel programs the vector, the driver receives an interrupt message on its ring and can mask/ack through the capability | no polling; lower latency and power |
| D4 | **IOMMU (Intel VT-d, AMD-Vi).** Each driver's devices get their own DMA domain mapping only the memory lent to it | real driver isolation; zero-copy lending instead of bounce buffers; safe Thunderbolt |
| D5 | **ECAM (MCFG) config, PCIe segments, bus drivers.** Intel VMD as a bus driver in sexdrive that exposes the NVMe behind it; PCIe hot-plug | VMD laptops, servers, docks |
| D6 | **sexdrive as a storage organ.** One controller worker per device (NVMe, AHCI, VMD-NVMe, USB storage, SD, virtio-blk), a shared block layer with GPT (`sex-disk`), per-core NVMe queues, 4 KiB sectors, namespaces | Linux-class storage on any machine |
| D7 | **Small buses in userland.** I2C (DesignWare), GPIO (Intel pinctrl), SPI, SMBus as services that lease to HID, sensors, amplifiers | touchpads, sensors, speaker amps |
| D8 | **Firmware files.** A firmware store on the disk and a call for a driver to read its blob by name | Wi-Fi, GPU microcontrollers, Bluetooth |

D2 and D3 unlock most of the rest and are framework work, not one driver, so they are
the first ask for a separate thread.

## 7. Where we drifted from the sex-manual, and how to come back

The sex-manual (`docs/handoff_html/legacymd/archive/manual_sex.md`, `manual_servers.md`,
and the PreTeXt pages in `docs/handoff_html/pages/`) describes a small kernel, drivers as
isolated user-space servers, capability-routed interrupts and zero-copy DMA by lending
memory. Most of what was built since follows that; some of it took shortcuts. In plain
terms:

| The manual says | What we built | Why | How to bring it back |
|---|---|---|---|
| All drivers live in user-space server ELFs; no kernel modules | **Still true.** NVMe, USB, NIC, sound run as services | | keep |
| The kernel only routes hardware; drivers are separate | the kernel's `devmgr` knows which service gets which device class, and the NIC/virtio id lists are kernel files | quickest safe way to give the first drivers their devices with fixed, auditable slots | D2 driver manifests: the kernel leases by a table the services declare |
| Interrupt capabilities deliver MSI-X to driver rings (Phase 9) | drivers poll; two fixed legacy vectors | polling was deterministic in QEMU and needed no APIC/MSI work to get the first drivers going | D3 MSI-X capabilities; keep polling as the fallback |
| Zero-copy DMA via lent memory | fixed DMA pool regions per service, with bounce copies in sexdrive | without an IOMMU, lending a buffer lets a driver DMA anywhere; fixed pools keep custody provable | D4 IOMMU, then lend real buffers |
| One PKU key per protection domain (1–15) | **organs**: each key groups many tasks (Core, Storage, Network, Desk, ...) | there are only 16 keys and we need hundreds of tasks | this is a faithful extension, documented in `PROGRAM_SLOTS_DESIGN_2026_11_07.md` |
| Tiny kernel (5–8 kLOC) | about 20 kLOC of kernel Rust, including the ACPI interpreter | proofs, gates and BOOT policy were added in-kernel during bring-up | move ACPI AML evaluation, PCI policy and boot-time device policy into services as D2/D7 land |
| Phase 5 "NVIDIA 3070 GPU PD (nouveau-lifted skeleton)", Phase 15 "Linux driver translation (DDE)", sexnode/tuxedo "load Linux drivers" — marked complete | these were placeholders. `servers/sexdrive/src/driver.rs` is a stub that is not even in the build (the real driver is `apps/sexdrive`); `servers/tuxedo` and `servers/sexnode` are about 160 lines of stubs; there is no NVIDIA code | early rounds wrote aspirational pages and skeletons | rewrite those manual pages to match the code. **Keep tuxedo, but as an arm's-length guest**: a Linux driver (GPL-2.0-only) cannot be linked into GPL-3.0-only SexOS code, so tuxedo's Linux-API glue is a separate program, licensed MIT or GPL-2.0-or-later, built together with the Linux driver into one GPL-2.0-only binary that runs as its own protection domain and talks to SexOS only over PDX (the way Genode runs Linux drivers through dde_linux). Use it for heavy stacks where a native port is months of work (Intel Wi-Fi with mac80211, perhaps GPU display); port small drivers (NVMe, NIC, HDA, I2C HID) natively from BSD/Redox/MIT sources |
| sexdrive at `servers/sexdrive`, NVMe + AHCI | sexdrive at `apps/sexdrive`, NVMe now, AHCI in an open branch | the service was rebuilt from scratch as a proof-driven driver | update the manual path; AHCI lands with the NIC/sound/SATA branch |
| sexinput owns PS/2 and USB HID | sexusb owns xHCI and HID and feeds Input | USB is a bus with its own driver; Input stays the router | matches the multiserver idea; update the manual |
| POSIX syscall server (sexc) | POSIX as a guest library inside programs (PR #68) | keeps POSIX out of the trusted path | this is more microkernel-like than the manual; update it |

Overall, the parts that matter for a multiserver SASOS (one address space, PKU domains,
drivers as services, capabilities for every device lease, restartable services that
quiesce their devices) are in place and tested. The drift is in three mechanisms (MSI-X,
IOMMU lending, generic device leasing) and in documentation that claimed placeholders as
done. Stages D2–D4 close the mechanism gap; a manual rewrite closes the rest.

## 8. What this PR changes

- **New crate `crates/sex-hwdb`** (no_std, data only): vendor and class names, named ids
  for the x17 R1, Tiger Lake/Alder Lake platforms, Intel Wi-Fi, Realtek and Intel
  Ethernet, NVIDIA Ampere laptop GPUs, common NVMe SSDs, QEMU and virtio; owning service
  and support level per device; warnings for VMD/RAID, NVIDIA, Wi-Fi.
- **Kernel `devmgr`**: prints `[hw.device] bb:dd.f vvvv:dddd class=... <vendor> <model>
  service=<svc> support=<level>` for every device, `[hw.warning]` lines, a `[hw.census]`
  summary, and `[kernel.pci.nvme.absent] behind_vmd=1` when RAID mode hides the disk.
  The first NVMe controller in bus order is leased; others are logged
  (`[kernel.pci.nvme.extra]`) instead of silently replacing it.
- **sexdrive**: controller disable/enable waits use CAP.TO in ticks (500 ms units, real
  SSDs need up to seconds) and stop on a fatal status while enabling (`controller_ready.rs`);
  the admin completion doorbell honours CAP.DSTRD; a namespace formatted with 4 KiB
  sectors is logged as unsupported instead of quietly shrinking the disk.
- **Gates**: `scripts/hwdb_host_gate.sh` (in CI), `tests/sexdrive_controller_ready.rs` in
  `scripts/sexdrive_dma_host_gate.sh`, master runtime gate in QEMU.
- **ABI**: none. No syscall, slot, message or shared struct changed, so
  `abi_version_hash` is unchanged.
