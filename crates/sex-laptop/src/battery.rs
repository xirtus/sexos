// SPDX-License-Identifier: GPL-3.0-only
//! Battery arithmetic from the ACPI control methods (ACPI 6.5 section 10.2.2).
//!
//! `_BST` gives the state, the present rate and the remaining capacity; `_BIF`/`_BIX` the
//! design and last-full capacities. Capacity and rate share units (mWh and mW, or mAh and
//! mA), so their ratio is hours whichever the firmware picked.

pub const UNKNOWN: u64 = 0xFFFF_FFFF;
/// `_BST` state bits.
pub const DISCHARGING: u64 = 1 << 0;
pub const CHARGING: u64 = 1 << 1;
pub const CRITICAL: u64 = 1 << 2;

/// One battery's numbers, as the firmware reported them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    /// `_BST` element 0.
    pub state: u64,
    /// `_BST` element 1 (mW or mA), `UNKNOWN` if not known.
    pub rate: u64,
    /// `_BST` element 2 (mWh or mAh).
    pub remaining: u64,
    /// `_BIF` element 1 / `_BIX` element 2.
    pub design: u64,
    /// `_BIF` element 2 / `_BIX` element 3.
    pub last_full: u64,
}

fn known(v: u64) -> Option<u64> { (v != UNKNOWN && v != 0).then_some(v) }

impl Raw {
    /// Builds one from the integer elements of `_BST` and of `_BIF` (`bix` false) or `_BIX`.
    pub fn from_packages(bst: &[Option<u64>], info: &[Option<u64>], bix: bool) -> Option<Self> {
        let shift = bix as usize;
        let at = |p: &[Option<u64>], i: usize| p.get(i).copied().flatten();
        Some(Self {
            state: at(bst, 0)?,
            rate: at(bst, 1).unwrap_or(UNKNOWN),
            remaining: at(bst, 2)?,
            design: at(info, 1 + shift).unwrap_or(UNKNOWN),
            last_full: at(info, 2 + shift).unwrap_or(UNKNOWN),
        })
    }
}

/// What the bar and Settings show, summed over every battery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub percent: u8,
    pub charging: bool,
    pub discharging: bool,
    /// The firmware flagged a battery as critically low.
    pub critical: bool,
    /// Minutes until empty (discharging) or full (charging), when the rate is known.
    pub minutes: Option<u16>,
    /// Last-full over design capacity, 0..=100: how worn the cells are.
    pub health: Option<u8>,
}

/// Longest estimate worth showing (a near-zero rate gives nonsense).
pub const MAX_MINUTES: u64 = 48 * 60;

/// Sums the batteries; None when none reports a usable capacity.
pub fn combine(batteries: &[Raw]) -> Option<Reading> {
    let (mut now, mut full, mut design, mut rate) = (0u64, 0u64, 0u64, 0u64);
    let (mut charging, mut discharging, mut critical, mut rate_known) = (false, false, false, true);
    let mut any = false;
    for b in batteries {
        let Some(cap) = known(b.last_full).or(known(b.design)) else { continue };
        if b.remaining == UNKNOWN { continue; }
        any = true;
        now += b.remaining.min(cap);
        full += cap;
        if let Some(d) = known(b.design) { design += d; }
        charging |= b.state & CHARGING != 0;
        discharging |= b.state & DISCHARGING != 0;
        critical |= b.state & CRITICAL != 0;
        if b.rate == UNKNOWN { rate_known = false } else { rate += b.rate }
    }
    if !any || full == 0 { return None; }
    let percent = ((now * 100 + full / 2) / full).min(100) as u8;
    // Charging and discharging both set is a firmware quirk: trust "discharging".
    let charging = charging && !discharging;
    let minutes = if rate_known && rate > 0 {
        let left = if discharging { now } else if charging { full - now } else { 0 };
        (left > 0).then(|| (left * 60 / rate).min(MAX_MINUTES) as u16)
    } else { None };
    let health = (design > 0 && design >= full / 2).then(|| ((full * 100 + design / 2) / design).min(100) as u8);
    Some(Reading { percent, charging, discharging, critical, minutes, health })
}

/// "2:05" style text for minutes (h:mm), as the panel prints it.
pub fn hm(minutes: u16, out: &mut [u8; 6]) -> usize {
    let h = (minutes / 60).min(99);
    let m = minutes % 60;
    let mut n = 0;
    if h >= 10 { out[n] = b'0' + (h / 10) as u8; n += 1; }
    out[n] = b'0' + (h % 10) as u8; n += 1;
    out[n] = b':'; n += 1;
    out[n] = b'0' + (m / 10) as u8; n += 1;
    out[n] = b'0' + (m % 10) as u8; n += 1;
    n
}

/// When the machine should shut itself down to save the work in memory: the firmware
/// says critical, or 2% or less is left, while running on the battery.
pub fn must_shut_down(r: &Reading, on_ac: Option<bool>) -> bool {
    on_ac != Some(true) && !r.charging && r.discharging && (r.critical || r.percent <= 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discharging_gives_time_to_empty() {
        // 43 Wh of 86 Wh last-full, 90 Wh design, drawing 21.5 W: two hours left.
        let b = Raw { state: DISCHARGING, rate: 21_500, remaining: 43_000, design: 90_000, last_full: 86_000 };
        let r = combine(&[b]).unwrap();
        assert_eq!(r.percent, 50);
        assert_eq!(r.minutes, Some(120));
        assert_eq!(r.health, Some(96));
        assert!(r.discharging && !r.charging && !r.critical);
        let mut s = [0u8; 6];
        let n = hm(125, &mut s);
        assert_eq!(&s[..n], b"2:05");
    }

    #[test]
    fn charging_gives_time_to_full() {
        let b = Raw { state: CHARGING, rate: 3_000, remaining: 3_000, design: 6_000, last_full: 6_000 };
        let r = combine(&[b]).unwrap();
        assert_eq!((r.percent, r.minutes, r.charging), (50, Some(60), true));
    }

    #[test]
    fn unknown_rate_and_full_battery_have_no_estimate() {
        let b = Raw { state: 0, rate: UNKNOWN, remaining: 50_000, design: 50_000, last_full: 50_000 };
        assert_eq!(combine(&[b]).unwrap().minutes, None);
        let b = Raw { state: CHARGING, rate: 0, remaining: 100, design: 100, last_full: 100 };
        assert_eq!(combine(&[b]).unwrap().minutes, None);
    }

    #[test]
    fn two_batteries_are_summed() {
        let a = Raw { state: DISCHARGING, rate: 10_000, remaining: 10_000, design: 20_000, last_full: 20_000 };
        let b = Raw { state: 0, rate: 0, remaining: 20_000, design: 20_000, last_full: 20_000 };
        let r = combine(&[a, b]).unwrap();
        assert_eq!(r.percent, 75);
        assert_eq!(r.minutes, Some(180));
    }

    #[test]
    fn absent_or_unknown_capacity_reads_none() {
        assert_eq!(combine(&[]), None);
        let b = Raw { state: 0, rate: 0, remaining: UNKNOWN, design: 100, last_full: 100 };
        assert_eq!(combine(&[b]), None);
        // last-full unknown falls back to design.
        let b = Raw { state: 0, rate: 0, remaining: 30, design: 60, last_full: UNKNOWN };
        assert_eq!(combine(&[b]).unwrap().percent, 50);
        // Remaining above last-full (recalibration) clamps at 100.
        let b = Raw { state: 0, rate: 0, remaining: 70, design: 60, last_full: 60 };
        assert_eq!(combine(&[b]).unwrap().percent, 100);
    }

    #[test]
    fn packages_decode_bif_and_bix() {
        let bst = [Some(1), Some(5000), Some(25_000), Some(11_400)];
        let bif = [Some(0), Some(60_000), Some(50_000), Some(1)];
        let bix = [Some(1), Some(0), Some(60_000), Some(50_000)];
        let a = Raw::from_packages(&bst, &bif, false).unwrap();
        let b = Raw::from_packages(&bst, &bix, true).unwrap();
        assert_eq!(a, b);
        assert_eq!((a.design, a.last_full, a.remaining, a.rate), (60_000, 50_000, 25_000, 5000));
    }

    #[test]
    fn critical_battery_shuts_down_only_on_battery_power() {
        let r = Reading { percent: 2, discharging: true, ..Default::default() };
        assert!(must_shut_down(&r, Some(false)));
        assert!(must_shut_down(&r, None));
        assert!(!must_shut_down(&r, Some(true)));
        let r = Reading { percent: 40, discharging: true, critical: true, ..Default::default() };
        assert!(must_shut_down(&r, Some(false)));
        let r = Reading { percent: 1, charging: true, ..Default::default() };
        assert!(!must_shut_down(&r, Some(false)));
    }
}
