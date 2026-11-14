// SPDX-License-Identifier: GPL-3.0-only
//! Disk health: the NVMe SMART / Health Information log page (log identifier 02h, NVMe
//! Base Specification 2.0 §5.16.1.3) and ATA SMART (ACS-3: IDENTIFY DEVICE, and the
//! SMART READ DATA and READ THRESHOLD pages, whose attribute table is the de facto layout
//! every drive uses), parsed into one [`Health`] summary with a verdict.
//!
//! The pages themselves come from the disk driver. What it has to send:
//!
//! - NVMe: on the admin queue, Get Log Page (opcode [`NVME_GET_LOG_PAGE`]) with NSID
//!   FFFFFFFFh, CDW10 = [`nvme_get_log_cdw10`]`(512)` (LID 02h, NUMDL for 512 bytes, RAE
//!   0), one 4 KiB PRP; the first 512 bytes are the page.
//! - ATA: IDENTIFY DEVICE (ECh), then SMART (B0h) with FEATURES D0h (READ DATA) and D1h
//!   (READ THRESHOLDS), LBA mid 4Fh, LBA high C2h ([`ATA_SMART_LBA`]), each one 512-byte
//!   PIO-in sector.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const NVME_GET_LOG_PAGE: u8 = 0x02;
pub const NVME_LOG_SMART: u8 = 0x02;
/// CDW10 of a Get Log Page for the SMART/Health page, `bytes` long (a multiple of 4).
pub const fn nvme_get_log_cdw10(bytes: u32) -> u32 { NVME_LOG_SMART as u32 | ((bytes / 4 - 1) & 0xFFFF) << 16 }
pub const ATA_IDENTIFY: u8 = 0xEC;
pub const ATA_SMART: u8 = 0xB0;
pub const ATA_SMART_READ_DATA: u8 = 0xD0;
pub const ATA_SMART_READ_THRESHOLDS: u8 = 0xD1;
/// LBA bits 8..24 (LBA mid 4Fh, LBA high C2h) every SMART command carries.
pub const ATA_SMART_LBA: u32 = 0xC2_4F00;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthError {
    /// Fewer than 512 bytes.
    Short,
    /// The page's checksum does not add up.
    Checksum,
    /// The drive says it does not do SMART.
    NoSmart,
}

/// The NVMe SMART / Health Information page, field by field.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NvmeSmart {
    pub critical_warning: u8,
    /// Composite temperature, kelvin (0 when not reported).
    pub temperature_k: u16,
    pub available_spare: u8,
    pub spare_threshold: u8,
    pub percentage_used: u8,
    /// In thousands of 512-byte units, as the page counts.
    pub data_units_read: u128,
    pub data_units_written: u128,
    pub host_reads: u128,
    pub host_writes: u128,
    pub busy_minutes: u128,
    pub power_cycles: u128,
    pub power_on_hours: u128,
    pub unsafe_shutdowns: u128,
    pub media_errors: u128,
    pub error_log_entries: u128,
    pub warning_temp_minutes: u32,
    pub critical_temp_minutes: u32,
    /// Temperature sensors 1..8, kelvin; 0 when absent.
    pub sensors_k: [u16; 8],
}

pub mod nvme_warning {
    pub const SPARE_LOW: u8 = 1 << 0;
    pub const TEMPERATURE: u8 = 1 << 1;
    pub const RELIABILITY: u8 = 1 << 2;
    pub const READ_ONLY: u8 = 1 << 3;
    pub const BACKUP_FAILED: u8 = 1 << 4;
    pub const PMR_READ_ONLY: u8 = 1 << 5;
}

fn u128_at(p: &[u8], at: usize) -> u128 { u128::from_le_bytes(p[at..at + 16].try_into().unwrap()) }
fn u16_at(p: &[u8], at: usize) -> u16 { u16::from_le_bytes([p[at], p[at + 1]]) }
fn u32_at(p: &[u8], at: usize) -> u32 { u32::from_le_bytes(p[at..at + 4].try_into().unwrap()) }

pub fn parse_nvme(page: &[u8]) -> Result<NvmeSmart, HealthError> {
    if page.len() < 512 { return Err(HealthError::Short); }
    Ok(NvmeSmart {
        critical_warning: page[0], temperature_k: u16_at(page, 1), available_spare: page[3], spare_threshold: page[4], percentage_used: page[5],
        data_units_read: u128_at(page, 32), data_units_written: u128_at(page, 48), host_reads: u128_at(page, 64), host_writes: u128_at(page, 80),
        busy_minutes: u128_at(page, 96), power_cycles: u128_at(page, 112), power_on_hours: u128_at(page, 128), unsafe_shutdowns: u128_at(page, 144),
        media_errors: u128_at(page, 160), error_log_entries: u128_at(page, 176), warning_temp_minutes: u32_at(page, 192), critical_temp_minutes: u32_at(page, 196),
        sensors_k: core::array::from_fn(|i| u16_at(page, 200 + 2 * i)),
    })
}

/// What IDENTIFY DEVICE says about a drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtaIdentity {
    pub model: String,
    pub serial: String,
    pub firmware: String,
    /// Addressable sectors (LBA48 if supported, else LBA28).
    pub sectors: u64,
    pub smart_supported: bool,
    pub smart_enabled: bool,
    /// Word 217: `Some(0)` for a solid-state drive, `Some(rpm)` for a spinning one.
    pub rotation_rpm: Option<u16>,
}

/// ATA strings swap the two bytes of every word and pad with spaces.
fn ata_string(p: &[u8], words: core::ops::Range<usize>) -> String {
    let mut s = String::new();
    for w in words { for b in [p[2 * w + 1], p[2 * w]] { s.push(if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' }); } }
    String::from(s.trim())
}

pub fn parse_identify(page: &[u8]) -> Result<AtaIdentity, HealthError> {
    if page.len() < 512 { return Err(HealthError::Short); }
    // Word 255: A5h signature, then a checksum making all 512 bytes sum to 0.
    if page[510] == 0xA5 && page[..512].iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 { return Err(HealthError::Checksum); }
    let w = |i: usize| u16_at(page, 2 * i);
    let lba48 = w(83) & (1 << 10) != 0;
    let sectors = if lba48 { (0..4).map(|i| (w(100 + i) as u64) << (16 * i)).sum() } else { w(60) as u64 | (w(61) as u64) << 16 };
    let valid = |x: u16| x != 0 && x != 0xFFFF;
    let rotation_rpm = match w(217) { 1 => Some(0), r if (0x0401..0xFFFF).contains(&r) => Some(r), _ => None };
    Ok(AtaIdentity {
        model: ata_string(page, 27..47), serial: ata_string(page, 10..20), firmware: ata_string(page, 23..27), sectors,
        smart_supported: valid(w(82)) && w(82) & 1 != 0, smart_enabled: valid(w(85)) && w(85) & 1 != 0, rotation_rpm,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtaAttribute {
    pub id: u8,
    pub flags: u16,
    /// Normalised value and the worst it has been (1..253; higher is better).
    pub value: u8,
    pub worst: u8,
    /// The vendor's 48-bit raw value.
    pub raw: u64,
    /// From the READ THRESHOLDS page, when it was read.
    pub threshold: Option<u8>,
}
impl AtaAttribute {
    /// Bit 0 of the flags: a pre-failure attribute (below threshold = failing now).
    pub fn prefail(&self) -> bool { self.flags & 1 != 0 }
    pub fn name(&self) -> &'static str { attr_name(self.id) }
}

pub fn attr_name(id: u8) -> &'static str {
    match id {
        1 => "Raw read error rate", 3 => "Spin-up time", 4 => "Start/stop count", 5 => "Reallocated sectors", 7 => "Seek error rate",
        9 => "Power-on hours", 10 => "Spin retry count", 12 => "Power cycles", 177 => "Wear levelling count", 184 => "End-to-end errors",
        187 => "Reported uncorrectable", 188 => "Command timeouts", 190 => "Airflow temperature", 194 => "Temperature",
        196 => "Reallocation events", 197 => "Pending sectors", 198 => "Offline uncorrectable", 199 => "UDMA CRC errors",
        231 => "SSD life left", 233 => "Media wearout indicator", 241 => "Total LBAs written", 242 => "Total LBAs read",
        _ => "Vendor specific",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtaSmart {
    pub revision: u16,
    pub attributes: Vec<AtaAttribute>,
    pub offline_status: u8,
    pub self_test_status: u8,
}

/// The SMART READ DATA page, with thresholds from READ THRESHOLDS when given.
pub fn parse_ata_smart(data: &[u8], thresholds: Option<&[u8]>) -> Result<AtaSmart, HealthError> {
    if data.len() < 512 || thresholds.is_some_and(|t| t.len() < 512) { return Err(HealthError::Short); }
    let sums = |p: &[u8]| p[..512].iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0;
    if !sums(data) || thresholds.is_some_and(|t| !sums(t)) { return Err(HealthError::Checksum); }
    let mut attributes = Vec::new();
    for e in data[2..2 + 30 * 12].chunks_exact(12) {
        if e[0] == 0 { continue; }
        let mut raw = [0u8; 8];
        raw[..6].copy_from_slice(&e[5..11]);
        let threshold = thresholds.and_then(|t| t[2..2 + 30 * 12].chunks_exact(12).find(|x| x[0] == e[0]).map(|x| x[1]));
        attributes.push(AtaAttribute { id: e[0], flags: u16_at(e, 1), value: e[3], worst: e[4], raw: u64::from_le_bytes(raw), threshold });
    }
    Ok(AtaSmart { revision: u16_at(data, 0), attributes, offline_status: data[362], self_test_status: data[363] })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict { Good, Warning, Failing }
impl Verdict { pub fn name(self) -> &'static str { match self { Verdict::Good => "Good", Verdict::Warning => "Warning", Verdict::Failing => "Failing" } } }

/// One summary for either kind of drive; `None` where the drive does not say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health {
    pub verdict: Verdict,
    pub temperature_c: Option<i32>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    /// Endurance used, percent (NVMe Percentage Used; ATA from a wear attribute). May pass 100.
    pub percentage_used: Option<u8>,
    pub spare_percent: Option<u8>,
    pub media_errors: Option<u64>,
    pub reallocated_sectors: Option<u64>,
    pub pending_sectors: Option<u64>,
    pub uncorrectable_sectors: Option<u64>,
    /// Why the verdict is not Good, one sentence each.
    pub reasons: Vec<String>,
}

fn sat(x: u128) -> u64 { x.min(u64::MAX as u128) as u64 }

impl Health {
    fn new() -> Health {
        Health { verdict: Verdict::Good, temperature_c: None, power_on_hours: None, power_cycles: None, percentage_used: None, spare_percent: None,
            media_errors: None, reallocated_sectors: None, pending_sectors: None, uncorrectable_sectors: None, reasons: Vec::new() }
    }
    fn flag(&mut self, v: Verdict, why: String) { self.verdict = self.verdict.max(v); self.reasons.push(why); }

    pub fn from_nvme(s: &NvmeSmart) -> Health {
        use nvme_warning as w;
        let mut h = Health::new();
        h.temperature_c = (s.temperature_k != 0).then(|| s.temperature_k as i32 - 273);
        h.power_on_hours = Some(sat(s.power_on_hours));
        h.power_cycles = Some(sat(s.power_cycles));
        h.percentage_used = Some(s.percentage_used);
        h.spare_percent = Some(s.available_spare);
        h.media_errors = Some(sat(s.media_errors));
        let cw = s.critical_warning;
        if cw & w::SPARE_LOW != 0 { h.flag(Verdict::Failing, format!("Spare capacity is {}%, below the drive's threshold of {}%.", s.available_spare, s.spare_threshold)); }
        if cw & w::RELIABILITY != 0 { h.flag(Verdict::Failing, "The drive reports its reliability degraded by media or internal errors.".into()); }
        if cw & w::READ_ONLY != 0 { h.flag(Verdict::Failing, "The drive has put itself in read-only mode.".into()); }
        if cw & w::BACKUP_FAILED != 0 { h.flag(Verdict::Failing, "The drive's volatile memory backup has failed.".into()); }
        if cw & w::PMR_READ_ONLY != 0 { h.flag(Verdict::Warning, "The drive's persistent memory region is read only.".into()); }
        if cw & w::TEMPERATURE != 0 { h.flag(Verdict::Warning, format!("Temperature {} °C is outside the drive's limits.", h.temperature_c.unwrap_or(0))); }
        if s.percentage_used >= 90 { h.flag(Verdict::Warning, format!("{}% of the rated endurance is used.", s.percentage_used)); }
        if s.media_errors > 0 { h.flag(Verdict::Warning, format!("{} unrecovered media errors.", s.media_errors)); }
        h
    }

    pub fn from_ata(id: Option<&AtaIdentity>, s: &AtaSmart) -> Health {
        let mut h = Health::new();
        if id.is_some_and(|i| i.smart_supported && !i.smart_enabled) { h.flag(Verdict::Warning, "SMART is turned off on this drive.".into()); }
        let get = |n: u8| s.attributes.iter().find(|a| a.id == n);
        let raw32 = |n: u8| get(n).map(|a| a.raw & 0xFFFF_FFFF);
        h.temperature_c = get(194).or(get(190)).map(|a| (a.raw & 0xFF) as i32);
        h.power_on_hours = raw32(9);
        h.power_cycles = raw32(12);
        h.reallocated_sectors = raw32(5);
        h.pending_sectors = raw32(197);
        h.uncorrectable_sectors = raw32(198).or(raw32(187));
        h.percentage_used = get(231).or(get(233)).or(get(177)).map(|a| 100u8.saturating_sub(a.value.min(100)));
        for a in &s.attributes {
            let Some(t) = a.threshold.filter(|t| *t != 0 && *t != 0xFE) else { continue };
            if a.value <= t {
                let v = if a.prefail() { Verdict::Failing } else { Verdict::Warning };
                h.flag(v, format!("{} ({}) is at {}, at or below its threshold {}{}.", a.name(), a.id, a.value, t, if a.prefail() { ": the drive predicts failure" } else { "" }));
            } else if a.worst <= t {
                h.flag(Verdict::Warning, format!("{} ({}) was at or below its threshold {} in the past.", a.name(), a.id, t));
            }
        }
        for (n, what) in [(h.reallocated_sectors, "sectors have been reallocated"), (h.pending_sectors, "sectors are waiting to be reallocated"), (h.uncorrectable_sectors, "sectors could not be read")] {
            if let Some(n) = n.filter(|n| *n > 0) { h.flag(Verdict::Warning, format!("{n} {what}.")); }
        }
        // Self-test execution status: high nibble 0 = passed or never run, 1..=8 failures.
        if (3..=8).contains(&(s.self_test_status >> 4)) { h.flag(Verdict::Failing, "The last self-test failed.".into()); }
        h
    }
}
