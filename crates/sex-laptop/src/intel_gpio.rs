// SPDX-License-Identifier: GPL-3.0-only
//! Intel PCH GPIO pads (Tiger Lake and Alder Lake): where a `GpioInt` pin lives and
//! whether firmware already wired that pad straight to an I/O APIC line.
//!
//! A touchpad's `_CRS` names its interrupt as a pin on the GPIO controller
//! (`\_SB.GPI0`, `_HID` `INT34C6` on the Alienware x17 R1's Tiger Lake-H PCH). That pin
//! number is the controller's *GPIO number*, not a register index: each chip splits
//! its pads into communities (one `Memory32Fixed` each in the controller's `_CRS`, in
//! order) and pad groups, and each group's GPIO numbers start at a 32-aligned base.
//! The tables below map one to the other. Pad group boundaries are hardware facts,
//! read from Linux's `drivers/pinctrl/intel/pinctrl-{tigerlake,alderlake}.c` as a
//! reference (docs/BRIDGE_LEDGER.md table 2); no code came from there.
//!
//! Each pad has a configuration block (`PADCFG`, at the community's `PADBAR` offset,
//! 16 bytes a pad from revision 0x94, else 8). Firmware routes touchpad pads to the
//! I/O APIC on most boards (Intel's reference code sets `GpioIntApic` with input
//! inversion and writes the line into `INTSEL`). When it did, the pad's interrupt is a
//! plain I/O APIC line and needs no GPIO driver at all: [`Pad::apic_line`]. This module
//! only *reads* pads.

/// One pad group: first and last pad (chip-wide numbering) and its first GPIO number,
/// `None` for groups firmware never names (JTAG, SPI, HVCMOS).
type Group = (u16, u16, Option<u16>);

/// A PCH GPIO controller: its pad groups, community by community.
#[derive(Clone, Copy, Debug)]
pub struct Chip {
    pub name: &'static str,
    communities: &'static [&'static [Group]],
}

const TGL_LP: &[&[Group]] = &[
    &[(0, 25, Some(0)), (26, 41, Some(32)), (42, 66, Some(64))],
    &[(67, 74, Some(96)), (75, 98, Some(128)), (99, 119, Some(160)), (120, 143, Some(192)), (144, 170, Some(224))],
    &[(171, 194, Some(256)), (195, 219, Some(288)), (220, 225, None), (226, 250, Some(320)), (251, 259, None)],
    &[(260, 267, Some(352)), (268, 276, None)],
];

const TGL_H: &[&[Group]] = &[
    &[(0, 24, Some(0)), (25, 44, Some(32)), (45, 70, Some(64)), (71, 78, Some(96))],
    &[(79, 104, Some(128)), (105, 128, Some(160)), (129, 136, Some(192)), (137, 153, Some(224)), (154, 180, Some(256))],
    &[(181, 193, Some(288)), (194, 217, Some(320))],
    &[(218, 241, Some(352)), (242, 251, Some(384)), (252, 266, Some(416))],
    &[(267, 281, Some(448)), (282, 290, None)],
];

const ADL_S: &[&[Group]] = &[
    &[(0, 24, Some(0)), (25, 47, Some(32)), (48, 59, Some(64)), (60, 86, Some(96)), (87, 94, Some(128))],
    &[(95, 118, Some(160)), (119, 126, Some(192)), (127, 150, Some(224))],
    &[(151, 159, None), (160, 175, Some(256)), (176, 199, Some(288))],
    &[(200, 207, Some(320)), (208, 230, Some(352)), (231, 245, Some(384)), (246, 269, Some(416))],
    &[(270, 294, Some(448)), (295, 303, None)],
];

const ADL_N: &[&[Group]] = &[
    &[(0, 25, Some(0)), (26, 41, Some(32)), (42, 66, Some(64))],
    &[(67, 74, Some(96)), (75, 94, Some(128)), (95, 118, Some(160)), (119, 139, Some(192)), (140, 168, Some(224))],
    &[(169, 192, Some(256)), (193, 217, Some(288)), (218, 223, None), (224, 248, Some(320))],
    &[(249, 256, Some(352))],
];

impl Chip {
    /// The chip a GPIO controller's `_HID` names.
    pub fn from_hid(hid: &str) -> Option<Chip> {
        let (name, communities) = match hid {
            // Tiger Lake-LP; INTC1055 is Alder Lake-P, which kept Tiger Lake-LP's pads.
            "INT34C5" | "INTC1055" => ("tgl-lp", TGL_LP),
            // Tiger Lake-H (500-series PCH): the Alienware x17 R1.
            "INT34C6" => ("tgl-h", TGL_H),
            // Alder Lake-S and Raptor Lake-S desktop PCHs.
            "INTC1056" | "INTC1085" => ("adl-s", ADL_S),
            "INTC1057" => ("adl-n", ADL_N),
            _ => return None,
        };
        Some(Chip { name, communities })
    }

    /// Community index (= which `Memory32Fixed`) and pad number within it for a
    /// `GpioInt` pin.
    pub fn locate(&self, pin: u16) -> Option<(usize, u16)> {
        for (index, groups) in self.communities.iter().enumerate() {
            let first = groups.first()?.0;
            for &(start, end, base) in groups.iter() {
                let Some(base) = base else { continue };
                if pin >= base && pin - base <= end - start {
                    return Some((index, start + (pin - base) - first));
                }
            }
        }
        None
    }

    pub fn communities(&self) -> usize { self.communities.len() }
}

/// Community register offsets.
pub const REVID: u32 = 0x000;
pub const PADBAR: u32 = 0x00C;

/// Byte offset of pad `pad`'s configuration (DW0; DW1 follows) in its community.
pub fn pad_offset(revid: u32, padbar: u32, pad: u16) -> u32 {
    let stride = if revid >> 16 >= 0x94 { 16 } else { 8 };
    padbar + pad as u32 * stride
}

const PMODE: u32 = 0xF << 10;
const RXDIS: u32 = 1 << 9;
const ROUTE_IOXAPIC: u32 = 1 << 20;
const RXINV: u32 = 1 << 23;
const RXEVCFG_SHIFT: u32 = 25;

/// An I/O APIC line as the kernel programs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub gsi: u16,
    pub edge: bool,
    pub active_low: bool,
}

/// Why a pad is not usable as an I/O APIC line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotRouted {
    /// The pad is a native function, not a GPIO, or its input buffer is off.
    NotGpioInput,
    /// Firmware left the pad to a GPIO driver (shared controller interrupt) or SCI.
    NotApicRouted,
    /// `INTSEL` names no PCH I/O APIC line above the legacy ones.
    NoLine,
}

/// One pad's configuration words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    pub dw0: u32,
    pub dw1: u32,
}

impl Pad {
    /// The I/O APIC line this pad drives, given the pin's electrical polarity from
    /// `GpioInt`. The I/O APIC sees the pad after `RXINV`, so an active-low pin that
    /// firmware inverted reaches it active-high. `RXEVCFG` 0 passes the level through;
    /// anything else becomes a pulse, which the I/O APIC must take as an edge.
    pub fn apic_line(&self, pin_active_low: bool) -> Result<Line, NotRouted> {
        if self.dw0 & PMODE != 0 || self.dw0 & RXDIS != 0 { return Err(NotRouted::NotGpioInput); }
        if self.dw0 & ROUTE_IOXAPIC == 0 { return Err(NotRouted::NotApicRouted); }
        let gsi = (self.dw1 & 0xFF) as u16;
        if gsi < 16 { return Err(NotRouted::NoLine); }
        let inverted = self.dw0 & RXINV != 0;
        Ok(Line { gsi, edge: (self.dw0 >> RXEVCFG_SHIFT) & 3 != 0, active_low: pin_active_low != inverted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_chips() {
        for (hid, name) in [("INT34C5", "tgl-lp"), ("INTC1055", "tgl-lp"), ("INT34C6", "tgl-h"), ("INTC1056", "adl-s"), ("INTC1085", "adl-s"), ("INTC1057", "adl-n")] {
            assert_eq!(Chip::from_hid(hid).map(|c| c.name), Some(name));
        }
        assert!(Chip::from_hid("INT344B").is_none());
        assert_eq!(Chip::from_hid("INT34C6").unwrap().communities(), 5);
    }

    #[test]
    fn tiger_lake_h_pins() {
        let c = Chip::from_hid("INT34C6").unwrap();
        // GPP_A0 and GPP_B0 sit in community 0.
        assert_eq!(c.locate(0), Some((0, 0)));
        assert_eq!(c.locate(64), Some((0, 45)));
        // GPP_C0..: community 1 starts at pad 79; GPP_C is GPIO 160.. = pads 105..
        assert_eq!(c.locate(160), Some((1, 105 - 79)));
        assert_eq!(c.locate(160 + 23), Some((1, 128 - 79)));
        // GPP_E7 (a common touchpad pin): community 2 (registers of PCH community 3).
        assert_eq!(c.locate(288 + 7), Some((2, 7)));
        // GPP_H/J/K in community 3, GPP_I in 4.
        assert_eq!(c.locate(416 + 14), Some((3, 266 - 218)));
        assert_eq!(c.locate(448), Some((4, 0)));
        // Gaps between a group's last pad and the next base, and past the end, are no pads.
        assert_eq!(c.locate(25), None);
        assert_eq!(c.locate(160 + 24), None);
        assert_eq!(c.locate(448 + 15), None);
        assert_eq!(c.locate(u16::MAX), None);
    }

    #[test]
    fn tiger_lake_lp_skips_unnamed_groups() {
        let c = Chip::from_hid("INT34C5").unwrap();
        // GPP_E0 = GPIO 320 follows the unnamed HVCMOS pads 220..225 in community 2 (pad base 171).
        assert_eq!(c.locate(320), Some((2, 226 - 171)));
        assert_eq!(c.locate(352 + 7), Some((3, 7)));
    }

    #[test]
    fn pad_offsets_follow_revision() {
        assert_eq!(pad_offset(0x0094_0000, 0x700, 7), 0x700 + 7 * 16);
        assert_eq!(pad_offset(0x0093_0000, 0x400, 7), 0x400 + 7 * 8);
    }

    #[test]
    fn apic_routed_pad() {
        // Intel reference touchpad pad: GPIO mode, TX off, RX inverted, level, I/O APIC route, INTSEL 0x5D.
        let pad = Pad { dw0: RXINV | ROUTE_IOXAPIC | 1 << 8, dw1: 0x5D };
        assert_eq!(pad.apic_line(true), Ok(Line { gsi: 0x5D, edge: false, active_low: false }));
        assert_eq!(pad.apic_line(false), Ok(Line { gsi: 0x5D, edge: false, active_low: true }));
        let edge = Pad { dw0: ROUTE_IOXAPIC | 1 << RXEVCFG_SHIFT, dw1: 30 };
        assert_eq!(edge.apic_line(true), Ok(Line { gsi: 30, edge: true, active_low: true }));
    }

    #[test]
    fn pads_that_are_not_lines() {
        assert_eq!(Pad { dw0: 1 << 10 | ROUTE_IOXAPIC, dw1: 40 }.apic_line(true), Err(NotRouted::NotGpioInput));
        assert_eq!(Pad { dw0: RXDIS | ROUTE_IOXAPIC, dw1: 40 }.apic_line(true), Err(NotRouted::NotGpioInput));
        // GPIO-driver mode: the shared controller interrupt, which needs a GPIO driver.
        assert_eq!(Pad { dw0: RXINV, dw1: 40 }.apic_line(true), Err(NotRouted::NotApicRouted));
        assert_eq!(Pad { dw0: ROUTE_IOXAPIC, dw1: 14 }.apic_line(true), Err(NotRouted::NoLine));
    }
}
