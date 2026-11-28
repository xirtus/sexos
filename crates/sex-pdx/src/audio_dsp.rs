// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The Intel audio DSP (cAVS 2.5, Tiger Lake) beside an HD Audio lease: what BOOT
//! hands Bell so it can boot Sound Open Firmware and reach the built-in digital
//! microphones (docs/handoff/SOF_DMIC_PLAN_2026_11_28.md, stage 3).
//!
//! Only for the controllers [`firmware`] knows (Tiger Lake-H `8086:43c8`, Tiger
//! Lake-LP `8086:a0c8`); every other HD Audio controller keeps exactly the lease it
//! had (BAR0 view at `virtio::SLOT_SOUND`, command bits, the
//! `pci_command::HDA_DMA_REGION_ID` pool). For a cAVS controller Bell also holds:
//!
//! - [`SLOT_REGISTERS`]: a read/write view of BAR4, the DSP: core power/reset/stall
//!   (`ADSPCS`), IPC doorbells and the SRAM windows (mailboxes, ROM/firmware status)
//!   from `0x80000`. Linux `sound/soc/sof/intel/hda.h`: `HDA_DSP_BAR` 4,
//!   `SRAM_WINDOW_OFFSET(x)` = `0x80000 + x * 0x20000`, four windows, so 1 MiB.
//! - [`SLOT_DMA`]: a DMA pool for the code-loader copy of the firmware (`sof-tgl.ri`
//!   is 513 KiB), its BDL and the DMIC capture ring. Bell's codec pool stays as it was.
//! - [`SLOT_FIRMWARE`]: the boot module [`FIRMWARE_MODULE`] read only, when the image
//!   carries it. The kernel never looks inside.
//!
//! Bell reads each one like any region: `memory_region_info(slot)` checks the
//! `region_id` and length, then `memory_region_full_view(slot, QUERY_VIEW | READ |
//! WRITE)` (the firmware pack: `QUERY_VIEW | READ`). A slot that holds nothing means
//! there is no DSP here; BOOT logs `[boot.hda.dsp]` or `[boot.hda.dsp.absent]`.

/// A cAVS audio controller whose DSP BOOT leases: one [`firmware`] names a file for.
pub const fn cavs(vendor: u16, device: u16) -> bool { firmware(vendor, device).is_some() }

/// The BAR holding the DSP registers.
pub const BAR: u8 = 4;
/// BAR0 (HD Audio) must reach the extended capabilities too: processing pipe (PP,
/// about 0x800) and multi-link (ML, about 0xc00). Tiger Lake's BAR0 is 16 KiB and the
/// class lease grants all of it (`Kind::Hda`: whole BAR up to 64 KiB).
pub const HDA_BYTES: u64 = 16 * 1024;

pub const SLOT_REGISTERS: u64 = 1015;
pub const REGISTERS_REGION_ID: u32 = 0x4301;
/// SRAM windows 0..3 end at `0x80000 + 4 * 0x20000`: all of Tiger Lake's 1 MiB BAR4.
pub const REGISTER_BYTES: u64 = 0x80000 + 4 * 0x20000;

pub const SLOT_DMA: u64 = 1016;
pub const DMA_REGION_ID: u32 = 0x4302;
pub const DMA_BYTES: u64 = 1 << 20;

pub const SLOT_FIRMWARE: u64 = 1017;
pub const FIRMWARE_REGION_ID: u32 = 0x4303;
/// The boot module holding the SOF firmware pack, built by
/// `scripts/make_sof_firmware_pack.py` from `scripts/fetch_sof_firmware.sh` (sof-bin,
/// pinned, SHA-256 checked). Layout: `sex_wifi::pack` `SEXWIFW2` with every file
/// stored (`sex_wifi::pack::Pack::parse(bytes)?.file(name)` reads it), holding
/// [`FIRMWARE_TGL_H`], [`FIRMWARE_TGL`] and the licence `LICENCE.Intel`. A build that
/// could not fetch the firmware carries an empty pack.
pub const FIRMWARE_MODULE: &str = "sof.pack";
/// Intel-signed IPC3 images (sof-bin `sof/intel-signed/`).
pub const FIRMWARE_TGL_H: &str = "sof-tgl-h.ri";
pub const FIRMWARE_TGL: &str = "sof-tgl.ri";

/// The firmware file a controller boots: Tiger Lake-H (Alienware x17 R1) and Tiger Lake-LP.
pub const fn firmware(vendor: u16, device: u16) -> Option<&'static str> {
    match (vendor, device) {
        (0x8086, 0x43c8) => Some(FIRMWARE_TGL_H),
        (0x8086, 0xa0c8) => Some(FIRMWARE_TGL),
        _ => None,
    }
}
