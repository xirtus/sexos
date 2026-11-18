// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! What an NVMe drive says about itself, and whether this driver may use it.
//!
//! Real SSDs differ from QEMU's in ways that matter before the first read:
//! the largest transfer one command may carry (Identify Controller MDTS; QEMU
//! allows 512 KiB, some drives 64 KiB), the namespace's sector size (512 bytes
//! here; a drive formatted to 4 KiB sectors would make every 512-byte read DMA
//! 4 KiB into a 512-byte buffer) and per-sector metadata or protection
//! information (which the controller would DMA to a metadata pointer this
//! driver never sets). Pure parsing, so the rules run on the host
//! (tests/sexdrive_identify.rs). Facts from the NVMe Base and NVM Command Set
//! specifications.

/// The sector size this driver and Files work in.
pub const SECTOR_SHIFT: u8 = 9;
/// Memory page size the driver programs (CC.MPS = 0).
pub const PAGE: u64 = 4096;

/// Controller capabilities this driver needs: 4 KiB memory pages and the NVM
/// command set. `cap` is the 64-bit CAP register.
pub fn capabilities(cap: u64) -> Result<(), &'static str> {
    let mpsmin = (cap >> 48) & 0xF;
    let css = (cap >> 37) & 0xFF;
    if mpsmin != 0 { return Err("page_size_above_4k"); }
    if css & 1 == 0 { return Err("no_nvm_command_set"); }
    Ok(())
}

/// The parts of Identify Controller (CNS 1) worth a boot-log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controller {
    pub vendor: u16,
    pub serial: [u8; 20],
    pub model: [u8; 40],
    pub firmware: [u8; 8],
    /// Maximum data transfer size as a power of two of the minimum page; 0 = no limit.
    pub mdts: u8,
    /// Number of namespaces.
    pub namespaces: u32,
}

pub fn controller(id: &[u8]) -> Option<Controller> {
    if id.len() < 520 { return None; }
    let mut c = Controller { vendor: u16::from_le_bytes([id[0], id[1]]), serial: [0; 20], model: [0; 40], firmware: [0; 8],
        mdts: id[77], namespaces: u32::from_le_bytes([id[516], id[517], id[518], id[519]]) };
    c.serial.copy_from_slice(&id[4..24]);
    c.model.copy_from_slice(&id[24..64]);
    c.firmware.copy_from_slice(&id[64..72]);
    Some(c)
}

/// The largest transfer one command may carry, capped at `ceiling` (the driver's
/// own PRP-list limit) and never below one page.
pub fn max_transfer(mdts: u8, ceiling: u64) -> u64 {
    if mdts == 0 { return ceiling; }
    let bytes = if mdts >= 40 { u64::MAX } else { PAGE << mdts };
    bytes.min(ceiling).max(PAGE)
}

/// ASCII text of an Identify string field, trailing spaces and NULs dropped and
/// anything unprintable shown as '?'. Returns how many bytes of `out` it used.
pub fn text(field: &[u8], out: &mut [u8]) -> usize {
    let end = field.iter().rposition(|&b| b != b' ' && b != 0).map_or(0, |i| i + 1);
    let n = end.min(out.len());
    for (o, &b) in out.iter_mut().zip(&field[..n]) { *o = if (0x20..0x7F).contains(&b) && b != b'"' { b } else { b'?' }; }
    n
}

/// Why a namespace is not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reason: &'static str,
    pub lba_bytes: u64,
    pub metadata_bytes: u16,
    /// What the owner can do about it.
    pub fix: &'static str,
}

/// Identify Namespace (CNS 0): the namespace's size in 512-byte sectors, or why
/// this driver must not read or write it.
pub fn namespace(ns: &[u8]) -> Result<u64, Refusal> {
    let refuse = |reason, lba_bytes, metadata_bytes, fix| Err(Refusal { reason, lba_bytes, metadata_bytes, fix });
    if ns.len() < 192 { return refuse("short_identify", 0, 0, "none"); }
    let sectors = u64::from_le_bytes(ns[0..8].try_into().unwrap());
    let flbas = ns[26];
    // FLBAS bits 3:0 pick the format; bits 6:5 extend the index past 16 formats.
    let format = (flbas & 0xF) as usize | (((flbas >> 5) & 3) as usize) << 4;
    let at = 128 + 4 * format;
    if at + 4 > ns.len() { return refuse("lba_format_out_of_range", 0, 0, "none"); }
    let metadata = u16::from_le_bytes([ns[at], ns[at + 1]]);
    let shift = ns[at + 2];
    let lba_bytes = if shift < 64 { 1u64 << shift } else { 0 };
    if sectors == 0 { return refuse("inactive_namespace", lba_bytes, metadata, "create_namespace_1"); }
    if shift != SECTOR_SHIFT { return refuse("sector_size", lba_bytes, metadata, "nvme_format_lbaf_512"); }
    if metadata != 0 {
        // Extended LBAs (FLBAS bit 4) stretch every sector; a separate buffer needs a
        // metadata pointer in every command. Either way a 512-byte sector is not 512.
        let reason = if flbas & 0x10 != 0 { "metadata_inline" } else { "metadata_separate" };
        return refuse(reason, lba_bytes, metadata, "nvme_format_lbaf_512_no_metadata");
    }
    Ok(sectors)
}
