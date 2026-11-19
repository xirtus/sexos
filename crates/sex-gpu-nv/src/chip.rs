// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Which NVIDIA chip this is, from `PMC_BOOT_0`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family { Turing, Ga100, Ampere, Ada, Other }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chip {
    pub boot0: u32,
    /// 0x174 for GA104.
    pub chipset: u16,
    pub revision: u8,
}

impl Chip {
    /// Decode `PMC_BOOT_0` the way nouveau does for NV10 and later.
    pub fn decode(boot0: u32) -> Option<Chip> {
        if boot0 & 0x1f00_0000 == 0 || boot0 == 0xffff_ffff { return None; }
        Some(Chip { boot0, chipset: ((boot0 & 0x1ff0_0000) >> 20) as u16, revision: (boot0 & 0xff) as u8 })
    }
    pub fn name(&self) -> &'static str { name(self.chipset) }
    pub fn family(&self) -> Family {
        match self.chipset {
            0x162..=0x168 => Family::Turing,
            0x170 => Family::Ga100,
            0x172..=0x177 => Family::Ampere,
            0x192..=0x197 => Family::Ada,
            _ => Family::Other,
        }
    }
    /// The GSP-RM boot in this crate covers GA102..GA107 (the RTX 30 series)
    /// and AD102..AD107 (RTX 40), which share the Booter/FWSEC sequence.
    pub fn gsp_boot_supported(&self) -> bool { matches!(self.family(), Family::Ampere | Family::Ada) }
    /// The ELF section in `gsp-*.bin` that holds this chip's GSP-RM signature.
    pub fn signature_section(&self) -> &'static str {
        match self.family() { Family::Ada => ".fwsignature_ad10x", Family::Turing => ".fwsignature_tu10x", _ => ".fwsignature_ga10x" }
    }
    /// linux-firmware directory name (`nvidia/<dir>/gsp/`); GA10x and AD10x
    /// all link to `ga102`'s booter and GSP-RM except where noted there.
    pub fn firmware_dir(&self) -> &'static str {
        match self.chipset {
            0x172 => "ga102", 0x173 => "ga103", 0x174 => "ga104", 0x176 => "ga106", 0x177 => "ga107",
            0x192 => "ad102", 0x193 => "ad103", 0x194 => "ad104", 0x196 => "ad106", 0x197 => "ad107",
            _ => "unknown",
        }
    }
}

pub fn name(chipset: u16) -> &'static str {
    match chipset {
        0x162 => "TU102", 0x164 => "TU104", 0x166 => "TU106", 0x167 => "TU117", 0x168 => "TU116",
        0x170 => "GA100", 0x172 => "GA102", 0x173 => "GA103", 0x174 => "GA104", 0x176 => "GA106", 0x177 => "GA107",
        0x192 => "AD102", 0x193 => "AD103", 0x194 => "AD104", 0x196 => "AD106", 0x197 => "AD107",
        _ => "unknown",
    }
}

/// RTX 30 laptop parts whose GPU is a GA104 (the x17 R1's RTX 3070/3080 Laptop).
pub const GA104_LAPTOP_IDS: &[u16] = &[0x249c, 0x249d, 0x24dc, 0x24dd, 0x24b6, 0x24b7, 0x24b8, 0x24b9, 0x24ba, 0x24bb];
