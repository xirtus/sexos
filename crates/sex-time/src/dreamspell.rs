// SPDX-License-Identifier: GPL-3.0-only
//! The Dreamspell (13 Moon) kin count, shared with the Dreamroot calendar
//! app through `dreamroot-core` (github.com/xirtus/dreamroot): 260 kin of 20
//! solar seals and 13 galactic tones, wave spells, castles and the destiny
//! oracle. Read a day's kin from a timestamp with [`crate::CivilDay`].

pub use dreamroot_core::dreamspell::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CivilDay, Gregorian};
    fn day(y: i64, m: u8, d: u8) -> Kin {
        Kin::from_timestamp(Gregorian { year: y, month: m, day: d, hour: 12, minute: 0, second: 0 }.to_timestamp().unwrap()).unwrap()
    }
    #[test]
    fn timestamps_name_the_same_kin_as_dreamroot() {
        assert_eq!(day(2012, 12, 21).number(), 207);
        assert_eq!(day(2026, 11, 28).number(), 94);
        assert_eq!(day(2024, 2, 29), day(2024, 2, 28));
        assert_eq!(Kin::from_unix(1_795_867_200).map(Kin::number), Some(94)); // 2026-11-28T12:00Z
    }
}
