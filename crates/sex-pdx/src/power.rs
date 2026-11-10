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
//!
//! Firmware AML (batteries, lid, thermal zones, `_CST`, `_PTS`) does not run in the kernel.
//! The ACPI service (`servers/sexacpi`) holds the `FIRMWARE` right: it reads the AML tables
//! through `FW_TABLE`, reaches the hardware the firmware names through `FW_IO`, `FW_MEM` and
//! `FW_PCI` (the kernel refuses RAM, the kernel's own ports and PCI BARs), and tells the
//! kernel what it found (`FW_INVENTORY`, `FW_IDLE`) and what the sensors read
//! (`FW_SENSORS`). `SENSORS` answers from that last report.

pub const SYS: u64 = 84;
/// The ACPI service's BOOT identity (domain and layout), outside the catalog's rows.
pub const ACPI_SERVICE_PD: u32 = 50;
/// Its hardware key: the Senses organ, beside Input, USB and Bell.
pub const ACPI_SERVICE_KEY: u8 = 4;
/// Capability slot every holder uses.
pub const SLOT: u64 = 62;

/// Ops (rdi).
pub const INFO: u64 = 1;
pub const OFF: u64 = 2;
pub const RESTART: u64 = 3;
pub const READY: u64 = 4;
pub const SENSORS: u64 = 5;
/// Firmware window, `FIRMWARE` right only. rdx = arg, r10/r8 = arg2/arg3.
/// `FW_TABLE`: arg = table index (0 FADT, 1 DSDT, 2.. SSDTs), arg2 = byte offset.
/// Reply: [OK, length | signature << 32, 24 bytes of the table from offset].
pub const FW_TABLE: u64 = 16;
/// `FW_IO`: arg = port | width << 16 | write << 24, arg2 = value. Reply [OK, value].
pub const FW_IO: u64 = 17;
/// `FW_MEM`: arg = physical address, arg2 = width | write << 8, arg3 = value.
pub const FW_MEM: u64 = 18;
/// `FW_PCI`: arg = segment << 32 | bus << 24 | device << 19 | function << 16 | offset,
/// arg2 = width | write << 8, arg3 = value.
pub const FW_PCI: u64 = 19;
/// `FW_INVENTORY`: arg = batteries | lids << 8 | thermal << 16 | i2c_hid << 24 | ok << 32,
/// arg2 = `\_S5` as typ_a | typ_b << 8 (or u64::MAX), arg3 = 1 if AML failed.
pub const FW_INVENTORY: u64 = 20;
/// `FW_IDLE`: arg = C-state type | latency_us << 8 | mwait hint << 32.
pub const FW_IDLE: u64 = 21;
/// `FW_SENSORS`: arg, arg2, arg3 = words 1..=3 of `Sensors::encode`.
pub const FW_SENSORS: u64 = 22;
/// `FW_DGPU`: the discrete GPUs the kernel parked in D3hot at boot, for the ACPI half of
/// D3cold. arg = sub-op | index << 8 (index into the kernel's own list; never a free BDF).
/// `DGPU_INFO` -> [OK, gpu bdf | port_alone << 16 | chain_len << 20 | vendor << 32,
///   chain bdfs 0..4 (16 bits each, root port first), chain bdfs 4..8, device id].
/// `DGPU_PORT_D3`: the root port above it to D3hot (only when the GPU is all behind it).
/// `DGPU_DONE`: arg2 = resources turned off, arg3 = 1 if the ACPI node was found.
/// `DGPU_RESTORE`: back to D0, headers restored -> [OK, on, restored, waited_ms].
/// `DGPU_PREPARE`: headers saved again and functions to D3hot, for another power-off.
pub const FW_DGPU: u64 = 23;
pub const DGPU_INFO: u64 = 0;
pub const DGPU_PORT_D3: u64 = 1;
pub const DGPU_DONE: u64 = 2;
pub const DGPU_RESTORE: u64 = 3;
pub const DGPU_PREPARE: u64 = 4;
/// PCI bus/device/function as 16 bits.
pub const fn bdf(bus: u8, dev: u8, func: u8) -> u16 { (bus as u16) << 8 | ((dev as u16) & 31) << 3 | (func as u16 & 7) }
/// A GPU's marketing name, for logs.
pub fn gpu_product(vendor: u16, device: u16) -> &'static str {
    match (vendor, device) {
        (0x10DE, 0x2520 | 0x2560 | 0x2563) => "GeForce RTX 3060 Laptop",
        (0x10DE, 0x249D | 0x24DD) => "GeForce RTX 3070 Laptop",
        (0x10DE, 0x249C | 0x24DC) => "GeForce RTX 3080 Laptop",
        (0x10DE, 0x2420 | 0x2460) => "GeForce RTX 3080 Ti Laptop",
        (0x10DE, 0x2204) => "GeForce RTX 3090",
        (0x10DE, _) => "NVIDIA GPU",
        (0x1002, _) => "AMD GPU",
        (0x8086, _) => "Intel GPU",
        _ => "display controller",
    }
}

/// Rights in a grant.
pub const REQUEST: u8 = 1;
pub const FLUSH: u8 = 2;
pub const SENSE: u8 = 4;
pub const FIRMWARE: u8 = 8;

/// The message the kernel sends Files (as an IPC call from pd 0) before power changes.
/// arg0 is the op (`OFF` or `RESTART`). The ACPI service gets it too, and runs `\_PTS`.
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

/// `call` with the two extra arguments the firmware window uses.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn call4(op: u64, arg: u64, arg2: u64, arg3: u64) -> [u64; 5] {
    let (a, b, c, d, e): (u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS => a, inlateout("rdi") SLOT => _,
            inlateout("rsi") op => b, inlateout("rdx") arg => c,
            inlateout("r10") arg2 => d, inlateout("r8") arg3 => e,
            out("rcx") _, out("r11") _, options(nostack));
    }
    [a, b, c, d, e]
}
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn call4(_op: u64, _arg: u64, _arg2: u64, _arg3: u64) -> [u64; 5] { [DENIED, 0, 0, 0, 0] }

/// Kernel-owned I/O ports the firmware window never reaches: the PICs, the PIT, PCI
/// configuration (use `FW_PCI`), the PS/2 controller (Input's) and the serial log.
pub fn port_reserved(port: u16, width: u8) -> bool {
    let end = port as u32 + width.max(1) as u32;
    const KEPT: [(u32, u32); 7] = [(0x20, 0x22), (0xA0, 0xA2), (0x40, 0x44), (0xCF8, 0xD00), (0x60, 0x61), (0x64, 0x65), (0x3F8, 0x400)];
    KEPT.iter().any(|&(a, b)| (port as u32) < b && end > a)
}
/// PCI header registers the firmware window may not write: BARs and the expansion ROM.
pub fn pci_write_reserved(offset: u16, width: u8) -> bool {
    let (a, b) = (offset as u32, offset as u32 + width.max(1) as u32);
    (a < 0x28 && b > 0x10) || (a < 0x34 && b > 0x30)
}
/// Packs a `FW_PCI` address.
pub const fn pci_address(segment: u16, bus: u8, device: u8, function: u8, offset: u16) -> u64 {
    (segment as u64) << 32 | (bus as u64) << 24 | ((device & 31) as u64) << 19 | ((function & 7) as u64) << 16 | (offset & 0xFFF) as u64
}

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
    fn window_policy() {
        assert!(port_reserved(0x20, 1) && port_reserved(0xCFC, 4) && port_reserved(0x3FD, 1) && port_reserved(0x1E, 4));
        assert!(!port_reserved(0x62, 1) && !port_reserved(0x66, 1) && !port_reserved(0xB2, 1) && !port_reserved(0x70, 1));
        assert!(pci_write_reserved(0x10, 4) && pci_write_reserved(0x24, 4) && pci_write_reserved(0x30, 4) && pci_write_reserved(0x0E, 4));
        assert!(!pci_write_reserved(0x04, 2) && !pci_write_reserved(0x84, 4) && !pci_write_reserved(0x28, 4));
        assert_eq!(pci_address(0, 1, 0x1C, 4, 0x84), 1 << 24 | 0x1C << 19 | 4 << 16 | 0x84);
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
