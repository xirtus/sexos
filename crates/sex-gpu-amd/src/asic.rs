// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Which Radeon this is, from its PCI device id: the DCN version, the register
//! layout that version uses, and how many timing generators (OTG), pipes
//! (HUBP) and AUX channels it has. Ids come from amdgpu's PCI table where it
//! lists them; chips amdgpu finds by IP discovery (Phoenix and newer) are
//! listed by their known device ids and marked `confirmed: false` until a
//! SexOS boot receipt from that chip confirms the layout.

/// Register layouts. The OTG, HUBP and AUX blocks sit at the same offsets from
/// DCN 2.0 to 3.2; DCN 3.1 moved the DCN VM aperture registers and DCN 3.5
/// moved part of the OTG block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// DCN 2.0, 2.1, 3.0, 3.0.1, 3.0.2, 3.0.3.
    Dcn20,
    /// DCN 3.1.x and 3.2.x.
    Dcn31,
    /// DCN 3.5.
    Dcn35,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asic {
    pub device: u16,
    pub name: &'static str,
    /// DCN version (major, minor, revision).
    pub dcn: (u8, u8, u8),
    pub layout: Layout,
    pub otgs: u32,
    pub pipes: u32,
    pub auxes: u32,
    /// Integrated (Ryzen APU): the framebuffer is carved out of system RAM.
    pub apu: bool,
    /// The layout was checked against a header for this exact DCN version and
    /// the device id is in amdgpu's PCI table (or a SexOS receipt confirmed it).
    pub confirmed: bool,
}

const fn a(device: u16, name: &'static str, dcn: (u8, u8, u8), otgs: u32, pipes: u32, auxes: u32, apu: bool, confirmed: bool) -> Asic {
    let layout = if dcn.0 == 3 && dcn.1 >= 5 { Layout::Dcn35 } else if dcn.0 == 3 && dcn.1 >= 1 { Layout::Dcn31 } else { Layout::Dcn20 };
    Asic { device, name, dcn, layout, otgs, pipes, auxes, apu, confirmed }
}

pub const TABLE: &[Asic] = &[
    // Renoir family APUs, DCN 2.1: Renoir (Ryzen 4000), Lucienne and Cezanne (5000), Barcelo.
    a(0x1636, "Renoir", (2, 1, 0), 4, 4, 5, true, true),
    a(0x164C, "Lucienne", (2, 1, 0), 4, 4, 5, true, true),
    a(0x1638, "Cezanne", (2, 1, 0), 4, 4, 5, true, true),
    a(0x15E7, "Barcelo", (2, 1, 0), 4, 4, 5, true, true),
    // Rembrandt (Ryzen 6000, 7035), DCN 3.1.2.
    a(0x1681, "Rembrandt", (3, 1, 2), 4, 4, 5, true, true),
    a(0x164D, "Rembrandt", (3, 1, 2), 4, 4, 5, true, true),
    // Van Gogh (Steam Deck), DCN 3.0.1.
    a(0x163F, "Van Gogh", (3, 0, 1), 4, 4, 4, true, false),
    // IP-discovery APUs: Phoenix / Hawk Point (Ryzen 7040, 8040), Raphael (Ryzen 7000
    // desktop iGPU), Mendocino (7020), Strix Point (AI 300).
    a(0x15BF, "Phoenix", (3, 1, 4), 4, 4, 5, true, false),
    a(0x15C8, "Phoenix 2", (3, 1, 4), 4, 4, 5, true, false),
    a(0x1900, "Hawk Point", (3, 1, 4), 4, 4, 5, true, false),
    a(0x1901, "Hawk Point", (3, 1, 4), 4, 4, 5, true, false),
    a(0x164E, "Raphael", (3, 1, 5), 4, 4, 5, true, false),
    a(0x1506, "Mendocino", (3, 1, 6), 4, 4, 5, true, false),
    a(0x150E, "Strix Point", (3, 5, 0), 4, 4, 5, true, false),
    // RDNA1 dGPUs, DCN 2.0: Navi 10 (RX 5700), Navi 14 (RX 5500), Navi 12.
    a(0x731F, "Navi 10", (2, 0, 0), 6, 6, 6, false, true),
    a(0x7340, "Navi 14", (2, 0, 0), 5, 5, 6, false, true),
    a(0x7360, "Navi 12", (2, 0, 0), 6, 6, 6, false, true),
    // RDNA2 dGPUs, DCN 3.0.x: Navi 21 (RX 6800/6900), 22 (6700), 23 (6600), 24 (6500).
    a(0x73BF, "Navi 21", (3, 0, 0), 6, 6, 6, false, true),
    a(0x73AF, "Navi 21", (3, 0, 0), 6, 6, 6, false, true),
    a(0x73DF, "Navi 22", (3, 0, 0), 5, 5, 6, false, true),
    a(0x73FF, "Navi 23", (3, 0, 2), 5, 5, 6, false, true),
    a(0x73EF, "Navi 23", (3, 0, 2), 5, 5, 6, false, true),
    a(0x743F, "Navi 24", (3, 0, 3), 2, 2, 2, false, true),
    // RDNA3 dGPUs, DCN 3.2.x: Navi 31 (RX 7900), 32 (7800/7700), 33 (7600).
    a(0x744C, "Navi 31", (3, 2, 0), 4, 4, 5, false, false),
    a(0x747E, "Navi 32", (3, 2, 1), 4, 4, 5, false, false),
    a(0x7480, "Navi 33", (3, 2, 1), 4, 4, 5, false, false),
];

/// The Radeon behind a PCI vendor/device id, if SexOS knows its display engine.
pub fn identify(vendor: u16, device: u16) -> Option<Asic> {
    if vendor != 0x1002 { return None; }
    TABLE.iter().copied().find(|a| a.device == device)
}
