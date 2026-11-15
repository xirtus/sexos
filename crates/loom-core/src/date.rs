// SPDX-License-Identifier: GPL-3.0-only
//! Calendar dates from seconds since 1970-01-01 UTC and back (proleptic Gregorian).

pub const DAY: u64 = 86_400;

/// A calendar date and time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime { pub year: i64, pub month: u32, pub day: u32, pub hour: u32, pub minute: u32, pub second: u32 }

/// Days since 1970-01-01 for a civil date.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

pub fn from_secs(secs: u64) -> DateTime {
    let (year, month, day) = civil_from_days((secs / DAY) as i64);
    let rest = (secs % DAY) as u32;
    DateTime { year, month, day, hour: rest / 3600, minute: rest / 60 % 60, second: rest % 60 }
}

/// Parses `YYYY-MM-DD` to the second it starts at; `None` when it isn't a real date.
pub fn parse_date(s: &[u8]) -> Option<u64> {
    if s.len() != 10 || s[4] != b'-' || s[7] != b'-' { return None; }
    let num = |r: &[u8]| -> Option<u32> {
        r.iter().try_fold(0u32, |a, b| if b.is_ascii_digit() { Some(a * 10 + (b - b'0') as u32) } else { None })
    };
    let (y, m, d) = (num(&s[..4])?, num(&s[5..7])?, num(&s[8..])?);
    if !(1..=12).contains(&m) || d == 0 || d > 31 { return None; }
    let days = days_from_civil(y as i64, m, d);
    if civil_from_days(days) != (y as i64, m, d) || days < 0 { return None; }
    Some(days as u64 * DAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(from_secs(1_700_000_000), DateTime { year: 2023, month: 11, day: 14, hour: 22, minute: 13, second: 20 });
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        for d in -800_000..800_000i64 {
            if d % 997 != 0 { continue; }
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }
    #[test]
    fn parse() {
        assert_eq!(parse_date(b"1970-01-02"), Some(DAY));
        assert_eq!(parse_date(b"2023-02-29"), None);
        assert_eq!(parse_date(b"2024-02-29").map(|s| from_secs(s).day), Some(29));
        assert_eq!(parse_date(b"2024-2-29"), None);
    }
}
