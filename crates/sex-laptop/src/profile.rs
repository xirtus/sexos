// SPDX-License-Identifier: GPL-3.0-only
//! CPU power profiles. On Intel HWP the profile is the energy-performance preference in
//! `IA32_HWP_REQUEST` bits 31:24 (0 = performance, 255 = energy); AMD CPPC has the same
//! field in `CPPC_REQUEST`. The values match the four points Linux's `intel_pstate`
//! names, so measurements compare like for like.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Profile { Performance = 0, Balanced = 1, Saver = 2 }

impl Profile {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v { 0 => Some(Self::Performance), 1 => Some(Self::Balanced), 2 => Some(Self::Saver), _ => None }
    }
    /// Energy-performance preference.
    pub const fn epp(self) -> u8 {
        match self { Self::Performance => 0x00, Self::Balanced => 0x80, Self::Saver => 0xC0 }
    }
    pub const fn name(self) -> &'static str {
        match self { Self::Performance => "performance", Self::Balanced => "balanced", Self::Saver => "saver" }
    }
}

/// `IA32_HWP_REQUEST` with a new preference, the rest kept.
pub const fn with_epp(request: u64, epp: u8) -> u64 { request & !(0xFF << 24) | (epp as u64) << 24 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epp_field() {
        assert_eq!(with_epp(0x8000_2A05, Profile::Saver.epp()), 0xC000_2A05);
        assert_eq!(with_epp(0xFFFF_FFFF_FFFF_FFFF, 0), 0xFFFF_FFFF_00FF_FFFF);
        assert_eq!(Profile::from_u8(1), Some(Profile::Balanced));
        assert_eq!(Profile::from_u8(3), None);
        assert_eq!(Profile::Performance.epp(), 0);
    }
}
