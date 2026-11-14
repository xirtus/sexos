// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The touchpad hand-off from the ACPI service to the Input service.
//!
//! Firmware alone knows where a laptop's I2C touchpad sits: `_CRS` names its address,
//! bus speed and the Serial IO controller, `_DSM` names its HID descriptor register,
//! and the controller's `_ADR`, `FMCN`/`SSCN` give its PCI function and clock counts.
//! The ACPI service reads those and sends one [`Found`] to Input, which holds the
//! controller's register lease (BOOT leases every Intel Serial IO I2C function to
//! Input, at [`CONTROLLER_SLOT`] and then the extra device slots).

/// Slot in the ACPI service's table of its route to Input.
pub const ROUTE_SLOT: u64 = 61;
/// Slot in Input's table of its first Serial IO controller.
pub const CONTROLLER_SLOT: u64 = 28;
/// The message: ACPI service → Input. BOOT's ACPI identity is the only sender Input takes.
pub const OP_FOUND: u64 = 0x50F2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// The Serial IO controller (PCI function, from `_ADR`; bus 0 on every PCH).
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    /// The touchpad's I2C address and whether it is a 10-bit one.
    pub address: u16,
    pub ten_bit: bool,
    /// Bus speed in kHz (100 or 400; 1000 is run as 400).
    pub khz: u32,
    /// HID descriptor register, from `_DSM` function 1.
    pub hid_register: u16,
    /// SCL high and low counts and SDA hold for that speed (`FMCN`/`SSCN`), if firmware has them.
    pub counts: Option<(u16, u16, u16)>,
    /// GPIO interrupt pin, if `_CRS` names one.
    pub gpio_pin: Option<u16>,
}

impl Found {
    pub fn encode(&self) -> [u64; 3] {
        let a0 = self.bus as u64 | (self.device as u64 & 31) << 8 | (self.function as u64 & 7) << 13 | (self.khz as u64 & 0xffff) << 16
            | (self.address as u64 & 0x3ff) << 32 | (self.ten_bit as u64) << 42;
        let a1 = self.hid_register as u64 | self.gpio_pin.map_or(0, |p| p as u64 | 1 << 16) << 16;
        let a2 = self.counts.map_or(0, |(h, l, s)| h as u64 | (l as u64) << 16 | (s as u64) << 32 | 1 << 48);
        [a0, a1, a2]
    }
    pub fn decode(w: [u64; 3]) -> Option<Self> {
        let [a0, a1, a2] = w;
        if a0 >> 43 != 0 || a1 >> 33 != 0 || a2 >> 49 != 0 { return None; }
        let khz = (a0 >> 16 & 0xffff) as u32;
        if khz == 0 { return None; }
        Some(Self {
            bus: a0 as u8, device: (a0 >> 8 & 31) as u8, function: (a0 >> 13 & 7) as u8, khz,
            address: (a0 >> 32 & 0x3ff) as u16, ten_bit: a0 >> 42 & 1 != 0,
            hid_register: a1 as u16,
            gpio_pin: (a1 >> 32 & 1 != 0).then_some((a1 >> 16) as u16),
            counts: (a2 >> 48 & 1 != 0).then_some((a2 as u16, (a2 >> 16) as u16, (a2 >> 32) as u16)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips() {
        for f in [
            Found { bus: 0, device: 0x15, function: 1, address: 0x2c, ten_bit: false, khz: 400, hid_register: 0x20, counts: Some((0x32, 0x71, 0x1e)), gpio_pin: Some(0x15c) },
            Found { bus: 0, device: 0x19, function: 0, address: 0x2a5, ten_bit: true, khz: 100, hid_register: 1, counts: None, gpio_pin: None },
        ] {
            assert_eq!(Found::decode(f.encode()), Some(f));
        }
    }
    #[test]
    fn refuses_stray_bits_and_no_speed() {
        let w = Found { khz: 400, ..Default::default() }.encode();
        assert!(Found::decode([w[0] | 1 << 50, w[1], w[2]]).is_none());
        assert!(Found::decode([w[0], w[1] | 1 << 40, w[2]]).is_none());
        assert!(Found::decode([0, 0, 0]).is_none());
    }
}
