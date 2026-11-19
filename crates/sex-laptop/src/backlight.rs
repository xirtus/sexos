// SPDX-License-Identifier: GPL-3.0-only
//! Panel brightness through the ACPI video extension (ACPI 6.5 appendix B): `_BCL` lists
//! the levels the firmware supports, `_BCM` sets one, `_BQC` reads the current one.

/// The usable levels from `_BCL`: element 0 is the level on AC power and element 1 on
/// battery (both repeated later in the list), so they are skipped; the rest are sorted and
/// deduplicated. Returns how many levels were written to `out`.
pub fn levels(bcl: &[u64], out: &mut [u8; 101]) -> usize {
    let mut seen = [false; 101];
    for &v in bcl.iter().skip(2) { if v <= 100 { seen[v as usize] = true; } }
    let mut n = 0;
    for (v, s) in seen.iter().enumerate() { if *s { out[n] = v as u8; n += 1; } }
    n
}

/// The level one step up or down from `current` (which may sit between two levels).
pub fn step(levels: &[u8], current: u8, up: bool) -> Option<u8> {
    if levels.is_empty() { return None; }
    if up { levels.iter().copied().find(|&l| l > current).or(levels.last().copied()) }
    else {
        // Never all the way to the lowest listed level when it is 0 (a black screen reads as off).
        let floor = levels.iter().copied().find(|&l| l > 0).unwrap_or(levels[0]);
        levels.iter().rev().copied().find(|&l| l < current && l >= floor).or(Some(floor))
    }
}

/// The supported level closest to a requested percentage.
pub fn nearest(levels: &[u8], percent: u8) -> Option<u8> {
    levels.iter().copied().min_by_key(|&l| (l as i16 - percent as i16).abs())
}

/// Video notifications the firmware sends when the brightness keys are pressed.
pub const NOTIFY_CYCLE: u64 = 0x85;
pub const NOTIFY_UP: u64 = 0x86;
pub const NOTIFY_DOWN: u64 = 0x87;
pub const NOTIFY_ZERO: u64 = 0x88;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_skip_the_two_defaults_and_sort() {
        let mut out = [0u8; 101];
        let n = levels(&[80, 50, 0, 10, 50, 30, 30, 100, 80, 200], &mut out);
        assert_eq!(&out[..n], &[0, 10, 30, 50, 80, 100]);
    }

    #[test]
    fn steps_move_between_levels_and_stop_at_the_ends() {
        let l = [0, 10, 30, 50, 80, 100];
        assert_eq!(step(&l, 30, true), Some(50));
        assert_eq!(step(&l, 40, true), Some(50));
        assert_eq!(step(&l, 100, true), Some(100));
        assert_eq!(step(&l, 50, false), Some(30));
        assert_eq!(step(&l, 10, false), Some(10));
        assert_eq!(step(&l, 0, false), Some(10));
        assert_eq!(step(&[], 50, true), None);
    }

    #[test]
    fn nearest_level() {
        let l = [5, 20, 60, 100];
        assert_eq!(nearest(&l, 70), Some(60));
        assert_eq!(nearest(&l, 90), Some(100));
        assert_eq!(nearest(&[], 90), None);
    }
}
