// SPDX-License-Identifier: GPL-3.0-only
//! Power: shutting the machine down, restarting it, and reading its power sensors.
//!
//! The kernel owns the ACPI hardware. BOOT hands each role a `Power` capability at
//! `SLOT` with only the rights that role needs: Shell may ask to shut down or restart,
//! Files is told before either happens and answers once its writes are on the disk,
//! the status bar may read the sensors. Nothing else can turn the machine off.
//!
//! A shut down or restart, whether asked by Shell or by the power button, runs in two
//! steps: the kernel sends Files `OP_PREPARE`, Files commits and flushes, then calls
//! `READY`. The kernel then enters S5 (or resets). If Files does not answer within
//! `FLUSH_TICKS`, the kernel goes ahead anyway, so a hung service never keeps a machine on.

pub const SYS: u64 = 84;
/// Capability slot every holder uses.
pub const SLOT: u64 = 62;

/// Ops (rdi).
pub const INFO: u64 = 1;
pub const OFF: u64 = 2;
pub const RESTART: u64 = 3;
pub const READY: u64 = 4;
pub const SENSORS: u64 = 5;

/// Rights in a grant.
pub const REQUEST: u8 = 1;
pub const FLUSH: u8 = 2;
pub const SENSE: u8 = 4;

/// The message the kernel sends Files (as an IPC call from pd 0) before power changes.
/// arg0 is the op (`OFF` or `RESTART`).
pub const OP_PREPARE: u64 = 0x50F0;
/// How long the kernel waits for Files, in timer ticks (about a millisecond each).
pub const FLUSH_TICKS: u64 = 5000;

pub const OK: u64 = 0;
pub const DENIED: u64 = u64::MAX;
pub const UNAVAILABLE: u64 = u64::MAX - 1;
pub const BUSY: u64 = u64::MAX - 2;
pub const INVALID: u64 = u64::MAX - 3;

/// INFO flags (reply word 1).
pub const CAN_OFF: u64 = 1 << 0;
pub const CAN_RESTART: u64 = 1 << 1;
pub const POWER_BUTTON: u64 = 1 << 2;
pub const HW_REDUCED: u64 = 1 << 3;
pub const AML: u64 = 1 << 4;
/// AML hit a firmware bug and was set aside; table-only features still work.
pub const AML_QUARANTINED: u64 = 1 << 5;
pub const HWP: u64 = 1 << 6;
pub const CPPC: u64 = 1 << 7;
pub const MWAIT_IDLE: u64 = 1 << 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant { pub holder_pd: u32, pub rights: u8 }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub flags: u64,
    /// Deepest idle state in use: 1 = HLT (C1), otherwise the ACPI C-state type.
    pub idle_state: u8,
    pub idle_latency_us: u16,
    pub batteries: u8,
    pub lids: u8,
    pub thermal_zones: u8,
    /// I2C HID devices (touchpads, touchscreens) found for a later driver.
    pub i2c_hid: u8,
}
impl Info {
    pub fn decode(raw: [u64; 5]) -> Result<Self, u64> {
        if raw[0] != OK { return Err(raw[0]); }
        Ok(Self {
            flags: raw[1],
            idle_state: raw[2] as u8,
            idle_latency_us: (raw[2] >> 16) as u16,
            batteries: raw[3] as u8,
            lids: (raw[3] >> 8) as u8,
            thermal_zones: (raw[3] >> 16) as u8,
            i2c_hid: (raw[3] >> 24) as u8,
        })
    }
    pub fn encode(&self) -> [u64; 5] {
        [OK, self.flags, self.idle_state as u64 | (self.idle_latency_us as u64) << 16,
         self.batteries as u64 | (self.lids as u64) << 8 | (self.thermal_zones as u64) << 16 | (self.i2c_hid as u64) << 24, 0]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sensors {
    /// Battery charge 0..=100, or None without a battery.
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub on_ac: Option<bool>,
    /// Lid open, or None without a lid.
    pub lid_open: Option<bool>,
    /// Hottest thermal zone, in tenths of a kelvin.
    pub hottest_decikelvin: Option<u32>,
}
impl Sensors {
    pub fn encode(&self) -> [u64; 5] {
        let battery = match self.battery_percent { Some(p) => p as u64 | 1 << 8 | (self.charging as u64) << 9, None => 0 };
        let ac = match self.on_ac { Some(v) => 1 | (v as u64) << 1, None => 0 };
        let lid = match self.lid_open { Some(v) => 1 | (v as u64) << 1, None => 0 };
        let hot = self.hottest_decikelvin.map_or(0, |v| v as u64 | 1 << 32);
        [OK, battery, ac | lid << 2, hot, 0]
    }
    pub fn decode(raw: [u64; 5]) -> Result<Self, u64> {
        if raw[0] != OK { return Err(raw[0]); }
        let b = raw[1];
        Ok(Self {
            battery_percent: (b & 1 << 8 != 0).then_some((b as u8).min(100)),
            charging: b & 1 << 9 != 0,
            on_ac: (raw[2] & 1 != 0).then_some(raw[2] & 2 != 0),
            lid_open: (raw[2] & 4 != 0).then_some(raw[2] & 8 != 0),
            hottest_decikelvin: (raw[3] & 1 << 32 != 0).then_some(raw[3] as u32),
        })
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn call(op: u64, arg: u64) -> [u64; 5] {
    let (a, b, c, d, e): (u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS => a, inlateout("rdi") SLOT => _,
            inlateout("rsi") op => b, inlateout("rdx") arg => c,
            inlateout("r10") 0u64 => d, inlateout("r8") 0u64 => e,
            out("rcx") _, out("r11") _, options(nostack));
    }
    [a, b, c, d, e]
}
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn call(_op: u64, _arg: u64) -> [u64; 5] { [DENIED, 0, 0, 0, 0] }

pub fn info() -> Result<Info, u64> { Info::decode(call(INFO, 0)) }
pub fn sensors() -> Result<Sensors, u64> { Sensors::decode(call(SENSORS, 0)) }
/// Asks for a shut down (`OFF`) or `RESTART`. Returns once the kernel has started it;
/// the machine goes down after Files has flushed.
pub fn request(op: u64) -> Result<(), u64> {
    match call(op, 0)[0] { OK => Ok(()), e => Err(e) }
}
/// Files: everything is on the disk (`flushed`), or it could not be (the kernel goes ahead).
pub fn ready(flushed: bool) -> u64 { call(READY, flushed as u64)[0] }

/// Decodes a compressed EISA id (an integer `_HID`) as its seven-letter name, e.g. PNP0C0A.
pub fn eisa_id(id: u32) -> [u8; 7] {
    let v = id.swap_bytes();
    let hex = |n: u32| b"0123456789ABCDEF"[(n & 0xF) as usize];
    [
        b'@' + ((v >> 26) & 0x1F) as u8,
        b'@' + ((v >> 21) & 0x1F) as u8,
        b'@' + ((v >> 16) & 0x1F) as u8,
        hex(v >> 12), hex(v >> 8), hex(v >> 4), hex(v),
    ]
}

/// Reads the two sleep-type values of a `\_S5_` package straight from DSDT bytes, for when
/// AML cannot run. Bounds-checked; returns None for anything it does not understand.
pub fn s5_from_dsdt(aml: &[u8]) -> Option<(u8, u8)> {
    let mut i = 0;
    while i + 4 <= aml.len() {
        if &aml[i..i + 4] == b"_S5_" {
            // Expect NameOp (0x08) before the name, optionally with a root prefix.
            let named = (i >= 1 && aml[i - 1] == 0x08) || (i >= 2 && aml[i - 1] == b'\\' && aml[i - 2] == 0x08);
            let mut p = i + 4;
            if named && aml.get(p) == Some(&0x12) {
                p += 1;
                let lead = *aml.get(p)?;
                p += 1 + (lead >> 6) as usize; // PkgLength bytes
                p += 1; // NumElements
                let mut values = [0u8; 2];
                for v in values.iter_mut() {
                    match *aml.get(p)? {
                        0x0A => { *v = *aml.get(p + 1)?; p += 2; }
                        0x00 => { *v = 0; p += 1; }
                        0x01 => { *v = 1; p += 1; }
                        0x0B => { *v = *aml.get(p + 1)?; p += 3; }
                        0x0C => { *v = *aml.get(p + 1)?; p += 5; }
                        _ => return None,
                    }
                }
                return Some((values[0] & 7, values[1] & 7));
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eisa() {
        // PNP0C0A as an EISA id integer (as AML stores it).
        assert_eq!(&eisa_id(0x0A0CD041), b"PNP0C0A");
        assert_eq!(&eisa_id(0x0D0CD041), b"PNP0C0D");
    }
    #[test]
    fn s5_patterns() {
        // Name(_S5, Package(4){0x05, 0x05, 0, 0}) as iasl writes it.
        let a = [0x08, b'_', b'S', b'5', b'_', 0x12, 0x0A, 0x04, 0x0A, 0x05, 0x0A, 0x05, 0x00, 0x00];
        assert_eq!(s5_from_dsdt(&a), Some((5, 5)));
        // Zero/One shorthand and a root prefix.
        let b = [0x08, b'\\', b'_', b'S', b'5', b'_', 0x12, 0x06, 0x04, 0x00, 0x01, 0x00, 0x00];
        assert_eq!(s5_from_dsdt(&b), Some((0, 1)));
        assert_eq!(s5_from_dsdt(b"no sleep here"), None);
        // A truncated package is refused, not read past.
        assert_eq!(s5_from_dsdt(&[0x08, b'_', b'S', b'5', b'_', 0x12, 0x0A]), None);
    }
    #[test]
    fn round_trip() {
        let s = Sensors { battery_percent: Some(87), charging: true, on_ac: Some(true), lid_open: Some(false), hottest_decikelvin: Some(3132) };
        assert_eq!(Sensors::decode(s.encode()), Ok(s));
        assert_eq!(Sensors::decode(Sensors::default().encode()), Ok(Sensors::default()));
        let i = Info { flags: CAN_OFF | CAN_RESTART, idle_state: 3, idle_latency_us: 80, batteries: 1, lids: 1, thermal_zones: 2, i2c_hid: 1 };
        assert_eq!(Info::decode(i.encode()), Ok(i));
    }
}
