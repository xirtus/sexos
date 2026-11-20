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
| `crates/sex-wifi`, `servers/sexnet` `wifi.rs` | Intel AX200/AX201/AX210/AX211 | OpenBSD `iwx` (ISC) and Intel firmware interface | receive-ring formats and the 0x1c80 free-ring index, firmware load to ALIVE, PNVM handoff and init commands to `[wifi.ready]`, scan request and RX MPDU layout, association on real cards | #83, W2, W3, W4 |
| `kernel/src/hal/dgpu.rs` (D3hot, headers) + `servers/sexacpi` (`_PS3`/`_OFF`) | NVIDIA dGPU power off (D3cold) | Linux runtime PM approach (`_PS3`, `_PRx` `_OFF`, root port) | `_OFF` on the x17 R1's PEG port | #82 |
| `servers/sexusb` | xHCI, hubs, HID | xHCI 1.2 and USB HID specs | firmware handoff on real PCs | #76 |
| `crates/sex-bt` (`intel.rs`, `host.rs`, `smp.rs`, `ssp.rs`), `servers/sexusb` `bt.rs`, `servers/cufflinks` | Intel USB Bluetooth (AX201 8087:0026, AX210/Killer AX1675x 8087:0032, AX211 8087:0033) and the HCI host stack | Bluetooth Core 5.4 spec (HCI, L2CAP, SMP, SSP; SMP sample data as test vectors); Linux `btintel`/`btusb` (GPL-2.0, read only, no code copied) for the Intel vendor flow: Read Version 0xFC05 with 0xFF and its TLV layout, Secure Send fragment types and offsets (RSA 644 / ECDSA 320 bytes, 252-byte fragments, 4-byte-aligned command buffer), Intel Reset parameters, vendor events 0xFF/0x06 and 0x02, DDC 0xFC8B records, firmware file naming, Secure Send on bulk OUT and bootloader events on bulk IN; BlueZ default event mask value | the whole Intel load on the x17 R1 (8087:0032), ncmd=0 quirk on the first completion, bulk-IN events in the bootloader, interrupt event splitting | W1; W2 adds HID over GATT, classic HID and bonds from the HID Profile 1.1, HOGP 1.0 and Core spec ATT/GATT/ah() (spec sample data as test vectors), no code copied |
| `crates/sex-volumes` | FAT, exFAT, ext4 (read), NTFS (read), MBR/GPT | on-disk format documentation | disks made by Windows and Linux | #75 |
| `crates/sex-gpu-intel` (driver logic for the planned `servers/sexgpu-intel`), `crates/sex-kms` | Intel Gen12 Tiger Lake display engine (UHD 8086:9a60 on the x17 R1): GMBUS, DP AUX, transcoder readout, planes, cursor, backlight PWM, hotplug | Intel public PRMs (Tiger Lake Vol 2c registers, Vol 12 display); MIT i915 register headers as cross-check only; VESA E-EDID, DisplayPort and CTA-861 layouts | GMBUS pin numbers, AUX timing, TGL-H port set, SDEISR/DE HPD bits, PLANE_SURFLIVE latching, PWM controller on the x17 R1 | L1/L3 |
| `crates/sex-gpu-amd` (driver logic for a planned AMD display service), `crates/sex-kms` | AMD Radeon DCN 2.0-3.5 display engine (Ryzen APUs Renoir to Strix, RDNA1-3 dGPUs): OTG timing readout, HUBP surface readout and vsync flips, DCN AUX engine (DPCD, EDID over I2C-over-AUX), DCN VM aperture | AMD's MIT-licensed register headers (`drivers/gpu/drm/amd/include/asic_reg/dcn/dcn_*_offset.h`, `_sh_mask.h`, `*_ip_offset.h`) for offsets and fields; the MIT-licensed DC code (`dcn10_optc.c`, `dcn10_hubp.c`, `dce_aux.c`) read for the timing encoding, flip order and AUX sequence, no code copied; amdgpu's PCI id table | DCN segment base on IP-discovery chips (Phoenix and newer), AUX pad mux, which AUX/DIG serves which connector, pixel clock (DCCG DTO), real-hardware receipts | A1 |
| `crates/sex-laptop` `intel_gpio.rs`, `servers/sexacpi` `touchpad.rs`, `kernel/src/device_irq.rs` (lines) | Intel PCH GPIO pads (Tiger Lake-LP/H, Alder Lake-P/S/N, Raptor Lake-S) behind a touchpad's `GpioInt`; read only | Pad group tables (first pad, last pad, GPIO base per group) and the `PADBAR`/`REVID` stride rule from Linux `drivers/pinctrl/intel/pinctrl-{tigerlake,alderlake,intel}.c` (GPL-2.0, read only, facts, no code copied); PADCFG DW0/DW1 bit meanings (`PMODE`, `GPIORXDIS`, `GPIROUTIOXAPIC`, `RXINV`, `RXEVCFG`, `INTSEL`) from Intel PCH datasheets; HID over I2C 1.0 §7.4 for the level interrupt | that the x17 R1's touchpad pad is I/O APIC routed (`[sexacpi.touchpad.gpio]` dw0 bit 20) and `FW_MEM` can read GPIO community MMIO; `INTSEL` line and polarity (`[sexinput.touchpad.mode] mode=interrupt`, no `line_stuck`/`line_silent`) | this PR |
| `servers/sexdisplay` `virtio_gpu.rs` | virtio-gpu 2D (QEMU) | virtio 1.2 specification §5.7 only; no crate, no Linux code | QEMU only | L2 |
| `servers/sexbell` `sound.rs` | virtio sound | `virtio-drivers` crate (MIT), linked | QEMU only | master |
| `crates/drag-vmm`, `apps/drag` (Drag D2, emulated devices for guests, not host drivers) | 16550A UART, 8259A PIC, 8254 PIT, virtio-mmio virtio-blk, Linux x86 boot protocol loader | Linux `Documentation/arch/x86/boot.rst` (boot protocol, read only), Intel SDM (VMX exits), 8250/16550, 8259A, 8254 data sheets, virtio 1.2 §2, §4.2, §5.2; no rust-vmm, QEMU or Linux code | a real Linux bzImage boots to its console under VMX (KVM host or x17) | D2 |
| `crates/sex-gpu-nv` (driver logic for the planned `servers/sexgpu-nv`) | NVIDIA RTX 30 (GA104 first, GA102..GA107 and AD10x share the path): VBIOS/FWSEC-FRTS, falcon heavy-secure boot, WPR2 layout, Booter, GSP-RM RPC queues, CPU sequencer | nouveau GSP-RM path (`nvkm/subdev/gsp`, `nvkm/falcon`, `nvkm/subdev/bios`, MIT, re-implemented in Rust); NVIDIA open-gpu-kernel-modules 535.113.01 struct layouts (MIT/GPL dual; offsets checked by compiling the MIT headers); nova-core VBIOS doc (GPL-2.0+ OR MIT); nova code (GPL-2.0) not used | everything past parsing: PROM readout on the x17 (laptop VBIOS may need ACPI `_ROM`), FWSEC fuse/signature choice, WPR2 bounds, Booter mailbox 0, GSP_INIT_DONE | N4a |

## 3. Device firmware (runs on the device, not the CPU)

| Firmware | Device | Source and licence | Loaded by |
|----------|--------|--------------------|-----------|
| `iwlwifi-*.ucode`, `*.pnvm` | Intel Wi-Fi | linux-firmware, Intel redistributable licence | `servers/sexnet` from the boot pack `firmware/wifi.pack` (`crates/sex-wifi`, #83, W2) |
| `ibt-*.sfi`, `ibt-*.ddc` | Intel Bluetooth (AX201, AX210/AX1675x, AX211) | linux-firmware (pinned commit, SHA-256 in `scripts/fetch_bt_firmware.sh`), Intel redistributable licence (LICENCE.ibt_firmware) | `servers/cufflinks` from a firmware pack region (`sex_pdx::bt_hci::FIRMWARE_SLOT`): BOOT grants `firmware/wifi.pack`, which does not carry the `ibt-*` files yet (the installed boot slot has about 1 MB free), W1/W2 |
| `gsp-535.113.01.bin` (GSP-RM, 38 MB), `booter_load/unload-535.113.01.bin`, `bootloader-535.113.01.bin` | NVIDIA GA102..GA107 (x17 R1 GA104; linux-firmware links `ga104/gsp` to `ga102/gsp`) | linux-firmware (pinned commit, SHA-256 in `scripts/fetch_nvidia_gsp_firmware.sh`), NVIDIA redistributable licence (LICENCE.nvidia) | planned `servers/sexgpu-nv` from the system volume (too big for the boot slot), parsed by `crates/sex-gpu-nv`; FWSEC comes from the GPU's own VBIOS, N4a |

## 4. Third-party code that runs in ring 0

| Crate | Licence | What it does in the kernel | Status |
|-------|---------|----------------------------|--------|
| `acpi` (table parsing) | MIT/Apache-2.0 | reads MADT and other static tables at boot | master; fine (static data, no firmware code runs) |
| `acpi` / `aml` AML interpreter | MIT/Apache-2.0 | **no longer in ring 0.** Runs in the ring-3 `sexacpi` server (Senses organ, pd 50) for `\_S5`, `_PTS`, batteries, lids, thermal, `_CST`, I2C HID, and the dGPU's `_PS3`/`_PRx` `_OFF`. The kernel links the crate for table parsing only and never builds an interpreter (guardrail test `test_kernel_never_runs_firmware_aml`). | moved by the sexacpi PR #88 (2026-11-10), dGPU half included |
