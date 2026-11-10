# Bridge ledger: Linux-derived drivers and their native (REA) replacements

Rule: `docs/DESIGN_GUARDRAILS.md` section 3. Every driver that runs Linux code through
Tuxedo (a **bridge**) has a row in table 1 before it merges, with the native SexOS driver
that will replace it. Native drivers that used Linux or BSD sources as a reference are in
table 2, so we know which borrowed behaviour still needs confirming on real hardware.
Device firmware is in table 3. Third-party code that runs in ring 0 is in table 4.

`scripts/tests/test_kernel_design_guardrails.py` fails when a crate, server or app is
licensed `GPL-2.0-only` or depends on `tuxedo` and its directory is not named in table 1.

Last full audit: 2026-11-09 (`docs/handoff/LINUX_DRIFT_AUDIT_2026_11_09.md`), covering
master at `5594b33` and open PRs #68 to #84.

## 1. Bridges (Linux code running through Tuxedo)

| Component | Wraps (upstream, licence) | Runs in | REA replacement (tracker, plan) | "Replaced" means | Status |
|-----------|---------------------------|---------|----------------------------------|------------------|--------|
| (none)    |                           |         |                                  |                  |        |

There are no bridges today. `servers/tuxedo` is a 7-line placeholder with no callers.
The earlier manual's "Phase 15 Linux driver translation (DDE)" kernel syscalls were stubs
and were removed from the kernel on 2026-11-09 (see the audit).

When the first bridge lands, add its row here in the same PR, and give the REA
replacement a tracker (a GitHub issue titled `REA: <device>` or a project thread).

## 2. Native drivers that used other systems as reference

All of these are new Rust code under GPL-3.0 (or the dual driver licence), written for
SexOS. No Linux code was copied. "Confirm" lists what we took on trust from the
reference and must check on real hardware (or replace with datasheet/REA facts).

| Driver (where) | Device | Reference used | Confirm on hardware | PR |
|----------------|--------|----------------|---------------------|----|
| `servers/sexnet` `ethernet.rs`, `nic.rs` | Intel e1000/e1000e/igb/igc (I219, I225/I226) | Intel datasheets; Linux e1000e/igb for the TIPG copper default and the igb autoneg MDIC sequence | TIPG value, PHY autoneg on I219 (pch_spt+ workarounds not written) | #79 |
| `servers/sexnet` `realtek.rs` | Realtek RTL8111/8168/8411 (r8169), RTL8125/8126 (r8125) | Linux r8169 for PHY power-up through PHYAR/OCP | PHY power-up path per chip revision | #79 |
| `servers/sexbell` `hda.rs` | Intel HD Audio | Intel HDA specification | codec quirks on the x17 R1 (Realtek ALC) | #79 |
| `apps/sexdrive` `ahci.rs` | AHCI SATA | AHCI 1.3.1 specification | none beyond spec | #79 |
| `crates/sex-vmd`, `kernel/src/vmd.rs` | Intel VMD (RST "RAID On") | PCIe spec; register meanings from Linux `drivers/pci/controller/vmd.c` | bus numbering and BAR windows on the x17 R1 | #84 |
| `crates/sex-imsm`, `apps/sexdrive` `raid.rs` | Intel IMSM RAID 0/1 metadata | metadata layout as mdadm reads it; fixtures checked with `mdadm --examine` (no mdadm code) | real RST-created volumes | #84 |
| `crates/sex-wifi`, `servers/sexnet` `wifi.rs` | Intel AX200/AX201/AX210/AX211 | OpenBSD `iwx` (ISC) and Intel firmware interface | receive-ring formats and the 0x1c80 free-ring index, firmware load to ALIVE, PNVM handoff and init commands to `[wifi.ready]`, association on real cards | #83, W2, W3 |
| `kernel/src/hal/dgpu.rs` (D3hot, headers) + `servers/sexacpi` (`_PS3`/`_OFF`) | NVIDIA dGPU power off (D3cold) | Linux runtime PM approach (`_PS3`, `_PRx` `_OFF`, root port) | `_OFF` on the x17 R1's PEG port | #82 |
| `servers/sexusb` | xHCI, hubs, HID | xHCI 1.2 and USB HID specs | firmware handoff on real PCs | #76 |
| `crates/sex-volumes` | FAT, exFAT, ext4 (read), NTFS (read), MBR/GPT | on-disk format documentation | disks made by Windows and Linux | #75 |
| `servers/sexbell` `sound.rs` | virtio sound | `virtio-drivers` crate (MIT), linked | QEMU only | master |
| Planned: NVIDIA GSP driver (`sexgpu`) | RTX 30 (GA104) | NVIDIA open-gpu-kernel-modules (MIT/GPL dual, port under MIT with notices); nova (GPL-2.0) read only | everything | plan in `docs/handoff/NVIDIA_GPU_PLAN_2026_11_09.md` |

## 3. Device firmware (runs on the device, not the CPU)

| Firmware | Device | Source and licence | Loaded by |
|----------|--------|--------------------|-----------|
| `iwlwifi-*.ucode`, `*.pnvm` | Intel Wi-Fi | linux-firmware, Intel redistributable licence | `servers/sexnet` from the boot pack `firmware/wifi.pack` (`crates/sex-wifi`, #83, W2) |
| GSP-RM (planned) | NVIDIA GA10x | linux-firmware, NVIDIA redistributable licence | planned `sexgpu` |

## 4. Third-party code that runs in ring 0

| Crate | Licence | What it does in the kernel | Status |
|-------|---------|----------------------------|--------|
| `acpi` (table parsing) | MIT/Apache-2.0 | reads MADT and other static tables at boot | master; fine (static data, no firmware code runs) |
| `acpi` / `aml` AML interpreter | MIT/Apache-2.0 | **no longer in ring 0.** Runs in the ring-3 `sexacpi` server (Senses organ, pd 50) for `\_S5`, `_PTS`, batteries, lids, thermal, `_CST`, I2C HID, and the dGPU's `_PS3`/`_PRx` `_OFF`. The kernel links the crate for table parsing only and never builds an interpreter (guardrail test `test_kernel_never_runs_firmware_aml`). | moved by the sexacpi PR #88 (2026-11-10), dGPU half included |
