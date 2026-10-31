//! SexTime: the SexOS civil calendar epoch and its Gregorian and Maya Long
//! Count views.
//!
//! SexTime counts civil seconds from 2012-12-21T00:00:00Z (Unix 1,356,048,000).
//! Like Unix time it does not count leap seconds, so it is a civil label, not
//! an elapsed-SI-second scale. A civil timestamp is distinct from a monotonic
//! instant: scheduling, deadlines and animation must never use this type, and
//! a wall-clock correction can never move them. Converting a time does not
//! make its source trustworthy; source provenance travels separately.
//!
//! The Long Count uses the Goodman–Martínez–Thompson correlation GMT 584283
//! ("GMT" names the correlation, not Greenwich Mean Time) on UTC days.
//! B'ak'tun is not wrapped modulo 13, and dates before the Long Count origin
//! are rejected rather than given an unspecified signed notation.
#![cfg_attr(not(test), no_std)]

/// Seconds from the Unix epoch to the SexTime epoch.
pub const UNIX_OFFSET: i64 = 1_356_048_000;
pub const SECONDS_PER_DAY: i64 = 86_400;
/// GMT correlation constant: Julian Day Number of Long Count 0.0.0.0.0.
pub const GMT_CORRELATION: i64 = 584_283;
/// Julian Day Number of the SexTime epoch day.
pub const EPOCH_JDN: i64 = 2_456_283;
/// Long Count day count of the SexTime epoch day (13.0.0.0.0).
pub const EPOCH_LONG_COUNT_DAYS: i64 = 1_872_000;
const NANOS_PER_SECOND: u32 = 1_000_000_000;

/// A civil instant: signed SexTime seconds plus normalized nanoseconds.
/// Nanosecond representation does not promise nanosecond source accuracy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SexTimestamp {
    seconds: i64,
    nanos: u32,
}

impl SexTimestamp {
    pub const EPOCH: Self = Self { seconds: 0, nanos: 0 };

    pub const fn new(seconds: i64, nanos: u32) -> Option<Self> {
        if nanos < NANOS_PER_SECOND { Some(Self { seconds, nanos }) } else { None }
    }

    pub const fn from_unix(unix_seconds: i64, nanos: u32) -> Option<Self> {
        match unix_seconds.checked_sub(UNIX_OFFSET) {
            Some(seconds) => Self::new(seconds, nanos),
            None => None,
        }
    }

    pub const fn to_unix(self) -> Option<i64> {
        self.seconds.checked_add(UNIX_OFFSET)
    }

    pub const fn seconds(self) -> i64 { self.seconds }
    pub const fn nanos(self) -> u32 { self.nanos }

    /// UTC day number relative to the epoch day; Euclidean, so one second
    /// before the epoch belongs to day -1.
    pub const fn utc_day(self) -> i64 { self.seconds.div_euclid(SECONDS_PER_DAY) }
    pub const fn second_of_day(self) -> u32 { self.seconds.rem_euclid(SECONDS_PER_DAY) as u32 }

    pub const fn julian_day_number(self) -> i64 { EPOCH_JDN + self.utc_day() }
}

/// Proleptic Gregorian UTC civil date and time. Years are astronomical
/// (year 0 is 1 BCE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gregorian {
    pub year: i64,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Days from 1970-01-01 to the SexTime epoch day.
const EPOCH_UNIX_DAYS: i64 = UNIX_OFFSET / SECONDS_PER_DAY;

pub const fn is_leap_year(year: i64) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

pub const fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap_year(year) { 29 } else { 28 },
        _ => 0,
    }
}

// Era-based civil conversions over 400-year cycles. Inputs are bounded by
// the callers so no intermediate overflows.
const fn civil_from_unix_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

const fn unix_days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Supported Gregorian year range; wide enough for any Long Count date this
/// crate represents and small enough that no day arithmetic can overflow.
pub const MIN_YEAR: i64 = -1_000_000;
pub const MAX_YEAR: i64 = 1_000_000;

impl Gregorian {
    pub fn from_timestamp(ts: SexTimestamp) -> Option<Self> {
        let unix_days = ts.utc_day().checked_add(EPOCH_UNIX_DAYS)?;
        // Bound before the era arithmetic.
        if unix_days.unsigned_abs() > (MAX_YEAR as u64) * 366 { return None; }
        let (year, month, day) = civil_from_unix_days(unix_days);
        let second = ts.second_of_day();
        Some(Self { year, month, day, hour: (second / 3600) as u8, minute: (second / 60 % 60) as u8, second: (second % 60) as u8 })
    }

    pub fn to_timestamp(self) -> Option<SexTimestamp> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&self.year) || self.month == 0 || self.month > 12
            || self.day == 0 || self.day > days_in_month(self.year, self.month)
            || self.hour > 23 || self.minute > 59 || self.second > 59 {
            return None;
        }
        let day = unix_days_from_civil(self.year, self.month, self.day) - EPOCH_UNIX_DAYS;
        let seconds = day.checked_mul(SECONDS_PER_DAY)?
            .checked_add(self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64)?;
        SexTimestamp::new(seconds, 0)
    }
}

/// A Maya Long Count date for one UTC day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LongCount {
    pub baktun: u64,
    pub katun: u8,
    pub tun: u8,
    pub winal: u8,
    pub kin: u8,
}

pub const BAKTUN_DAYS: i64 = 144_000;
pub const KATUN_DAYS: i64 = 7_200;
pub const TUN_DAYS: i64 = 360;
pub const WINAL_DAYS: i64 = 20;

impl LongCount {
    /// Days since 0.0.0.0.0; None before the Long Count origin.
    pub fn days_of(ts: SexTimestamp) -> Option<i64> {
        let days = ts.utc_day().checked_add(EPOCH_LONG_COUNT_DAYS)?;
        if days < 0 { None } else { Some(days) }
    }

    pub fn from_days(days: i64) -> Option<Self> {
        if days < 0 { return None; }
        Some(Self {
            baktun: (days / BAKTUN_DAYS) as u64,
            katun: (days % BAKTUN_DAYS / KATUN_DAYS) as u8,
            tun: (days % KATUN_DAYS / TUN_DAYS) as u8,
            winal: (days % TUN_DAYS / WINAL_DAYS) as u8,
            kin: (days % WINAL_DAYS) as u8,
        })
    }

    pub fn from_timestamp(ts: SexTimestamp) -> Option<Self> {
        Self::from_days(Self::days_of(ts)?)
    }

    /// Days since the origin, rejecting out-of-range places.
    pub fn days(self) -> Option<i64> {
        if self.katun >= 20 || self.tun >= 20 || self.winal >= 18 || self.kin >= 20 { return None; }
        (i64::try_from(self.baktun).ok()?).checked_mul(BAKTUN_DAYS)?
            .checked_add(self.katun as i64 * KATUN_DAYS + self.tun as i64 * TUN_DAYS
                + self.winal as i64 * WINAL_DAYS + self.kin as i64)
    }

    /// The first second (00:00:00 UTC) of this Long Count day.
    pub fn to_timestamp(self) -> Option<SexTimestamp> {
        let day = self.days()?.checked_sub(EPOCH_LONG_COUNT_DAYS)?;
        SexTimestamp::new(day.checked_mul(SECONDS_PER_DAY)?, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lc(b: u64, k: u8, t: u8, w: u8, i: u8) -> LongCount {
        LongCount { baktun: b, katun: k, tun: t, winal: w, kin: i }
    }
    fn g(year: i64, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Gregorian {
        Gregorian { year, month, day, hour, minute, second }
    }

    #[test]
    fn epoch_is_2012_12_21_utc_and_13_0_0_0_0() {
        let e = SexTimestamp::EPOCH;
        assert_eq!(e.to_unix(), Some(1_356_048_000));
        assert_eq!(Gregorian::from_timestamp(e), Some(g(2012, 12, 21, 0, 0, 0)));
        assert_eq!(LongCount::from_timestamp(e), Some(lc(13, 0, 0, 0, 0)));
        assert_eq!(e.julian_day_number(), 2_456_283);
        assert_eq!(e.julian_day_number() - GMT_CORRELATION, EPOCH_LONG_COUNT_DAYS);
    }

    #[test]
    fn one_day_later_and_one_second_earlier() {
        let next = SexTimestamp::new(SECONDS_PER_DAY, 0).unwrap();
        assert_eq!(LongCount::from_timestamp(next), Some(lc(13, 0, 0, 0, 1)));
        let before = SexTimestamp::new(-1, 999_999_999).unwrap();
        assert_eq!(before.utc_day(), -1);
        assert_eq!(LongCount::from_timestamp(before), Some(lc(12, 19, 19, 17, 19)));
        assert_eq!(Gregorian::from_timestamp(before), Some(g(2012, 12, 20, 23, 59, 59)));
    }

    #[test]
    fn unix_epoch_and_offset_round_trip() {
        let unix = SexTimestamp::from_unix(0, 0).unwrap();
        assert_eq!(unix.seconds(), -1_356_048_000);
        assert_eq!(Gregorian::from_timestamp(unix), Some(g(1970, 1, 1, 0, 0, 0)));
        for value in [-1, 0, 1, 1_356_047_999, 1_356_048_000, 4_102_444_800] {
            assert_eq!(SexTimestamp::from_unix(value, 7).unwrap().to_unix(), Some(value));
        }
    }

    #[test]
    fn nanoseconds_are_normalized_by_rejection() {
        assert!(SexTimestamp::new(0, 999_999_999).is_some());
        assert!(SexTimestamp::new(0, 1_000_000_000).is_none());
        assert!(SexTimestamp::from_unix(0, u32::MAX).is_none());
    }

    #[test]
    fn checked_arithmetic_rejects_overflow() {
        assert!(SexTimestamp::from_unix(i64::MIN, 0).is_none());
        assert!(SexTimestamp::new(i64::MAX, 0).unwrap().to_unix().is_none());
        assert!(Gregorian::from_timestamp(SexTimestamp::new(i64::MAX, 0).unwrap()).is_none());
        assert!(Gregorian::from_timestamp(SexTimestamp::new(i64::MIN, 0).unwrap()).is_none());
        assert!(LongCount::from_timestamp(SexTimestamp::new(i64::MIN, 0).unwrap()).is_none());
        assert!(lc(u64::MAX, 0, 0, 0, 0).days().is_none());
    }

    #[test]
    fn gregorian_leap_rules() {
        for (year, leap) in [(2000, true), (1900, false), (2024, true), (2100, false), (2012, true), (0, true), (-4, true), (-100, false)] {
            assert_eq!(is_leap_year(year), leap, "{year}");
        }
        assert!(g(2000, 2, 29, 0, 0, 0).to_timestamp().is_some());
        assert!(g(1900, 2, 29, 0, 0, 0).to_timestamp().is_none());
        assert!(g(2024, 2, 30, 0, 0, 0).to_timestamp().is_none());
        for bad in [g(2024, 0, 1, 0, 0, 0), g(2024, 13, 1, 0, 0, 0), g(2024, 4, 31, 0, 0, 0), g(2024, 1, 1, 24, 0, 0),
                    g(2024, 1, 1, 0, 60, 0), g(2024, 1, 1, 0, 0, 60), g(MAX_YEAR + 1, 1, 1, 0, 0, 0)] {
            assert!(bad.to_timestamp().is_none(), "{bad:?}");
        }
    }

    #[test]
    fn gregorian_round_trips_every_day_across_four_centuries_and_day_boundaries() {
        let start = g(1900, 1, 1, 0, 0, 0).to_timestamp().unwrap().utc_day();
        for day in start..start + 146_097 {
            for second in [0, 1, 43_200, 86_399] {
                let ts = SexTimestamp::new(day * SECONDS_PER_DAY + second, 0).unwrap();
                let civil = Gregorian::from_timestamp(ts).unwrap();
                assert_eq!(civil.to_timestamp(), Some(ts));
            }
        }
    }

    #[test]
    fn long_count_carries_and_origin() {
        assert_eq!(LongCount::from_days(19), Some(lc(0, 0, 0, 0, 19)));
        assert_eq!(LongCount::from_days(20), Some(lc(0, 0, 0, 1, 0)));
        assert_eq!(LongCount::from_days(359), Some(lc(0, 0, 0, 17, 19)));
        assert_eq!(LongCount::from_days(360), Some(lc(0, 0, 1, 0, 0)));
        assert_eq!(LongCount::from_days(7_200), Some(lc(0, 1, 0, 0, 0)));
        assert_eq!(LongCount::from_days(144_000), Some(lc(1, 0, 0, 0, 0)));
        // B'ak'tun is not wrapped modulo 13.
        assert_eq!(LongCount::from_days(14 * 144_000), Some(lc(14, 0, 0, 0, 0)));
        // GMT 584283: 0.0.0.0.0 is 11 August 3114 BCE (astronomical -3113).
        let origin = lc(0, 0, 0, 0, 0).to_timestamp().unwrap();
        assert_eq!(Gregorian::from_timestamp(origin), Some(g(-3113, 8, 11, 0, 0, 0)));
        let before = SexTimestamp::new(origin.seconds() - 1, 0).unwrap();
        assert!(LongCount::from_timestamp(before).is_none());
        for bad in [lc(13, 20, 0, 0, 0), lc(13, 0, 20, 0, 0), lc(13, 0, 0, 18, 0), lc(13, 0, 0, 0, 20)] {
            assert!(bad.days().is_none());
        }
    }

    #[test]
    fn long_count_round_trips_by_day() {
        for days in (0..3_000_000).step_by(997) {
            let date = LongCount::from_days(days).unwrap();
            assert_eq!(date.days(), Some(days));
            assert_eq!(LongCount::from_timestamp(date.to_timestamp().unwrap()), Some(date));
        }
    }
}
