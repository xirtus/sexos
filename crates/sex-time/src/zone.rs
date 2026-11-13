//! Time zones: each zone's standard offset and today's daylight-saving rule, as
//! compiled Rust data (no tzdb files at run time). The table follows the rules in
//! force since 2023 (IANA tzdb 2025 for these zones); it does not describe history,
//! so a local time decades back may be off by an hour. Zones are in
//! `sex_config::ZONES` order, which is append-only: the index is what settings store.
use crate::unix_days_from_civil;

/// When a zone's daylight-saving time starts and ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dst {
    None,
    /// EU: last Sunday of March to last Sunday of October, 01:00 UTC.
    Eu,
    /// US and Canada: second Sunday of March 02:00 local to first Sunday of November 02:00 local.
    Us,
    /// South-east Australia: first Sunday of October 02:00 standard to first Sunday of April 03:00 daylight.
    AuSouth,
    /// New Zealand: last Sunday of September 02:00 standard to first Sunday of April 03:00 daylight.
    Nz,
    /// Chile: first Sunday of September 04:00 UTC to first Sunday of April 03:00 UTC.
    Chile,
    /// Israel: Friday before the last Sunday of March 02:00 to last Sunday of October 02:00 local.
    Israel,
    /// Egypt: last Friday of April 00:00 to last Thursday of October 24:00 local.
    Egypt,
}

#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub name: &'static str,
    /// Standard offset east of UTC, in minutes.
    pub standard: i16,
    pub dst: Dst,
    pub standard_abbr: &'static str,
    pub dst_abbr: &'static str,
}

const fn z(name: &'static str, standard: i16, dst: Dst, standard_abbr: &'static str, dst_abbr: &'static str) -> Zone {
    Zone { name, standard, dst, standard_abbr, dst_abbr }
}
const fn fixed(name: &'static str, standard: i16, abbr: &'static str) -> Zone { z(name, standard, Dst::None, abbr, abbr) }

pub const ZONES: &[Zone] = &[
    fixed("UTC", 0, "UTC"),
    z("Europe/London", 0, Dst::Eu, "GMT", "BST"),
    z("Europe/Dublin", 0, Dst::Eu, "GMT", "IST"),
    z("Europe/Lisbon", 0, Dst::Eu, "WET", "WEST"),
    z("Europe/Paris", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Berlin", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Madrid", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Rome", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Amsterdam", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Stockholm", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Oslo", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Copenhagen", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Helsinki", 120, Dst::Eu, "EET", "EEST"),
    z("Europe/Warsaw", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Prague", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Vienna", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Zurich", 60, Dst::Eu, "CET", "CEST"),
    z("Europe/Athens", 120, Dst::Eu, "EET", "EEST"),
    fixed("Europe/Istanbul", 180, "+03"),
    z("Europe/Kyiv", 120, Dst::Eu, "EET", "EEST"),
    fixed("Europe/Moscow", 180, "MSK"),
    z("America/New_York", -300, Dst::Us, "EST", "EDT"),
    z("America/Chicago", -360, Dst::Us, "CST", "CDT"),
    z("America/Denver", -420, Dst::Us, "MST", "MDT"),
    fixed("America/Phoenix", -420, "MST"),
    z("America/Los_Angeles", -480, Dst::Us, "PST", "PDT"),
    z("America/Anchorage", -540, Dst::Us, "AKST", "AKDT"),
    fixed("Pacific/Honolulu", -600, "HST"),
    z("America/Toronto", -300, Dst::Us, "EST", "EDT"),
    z("America/Vancouver", -480, Dst::Us, "PST", "PDT"),
    fixed("America/Mexico_City", -360, "CST"),
    fixed("America/Sao_Paulo", -180, "-03"),
    fixed("America/Buenos_Aires", -180, "-03"),
    fixed("America/Bogota", -300, "-05"),
    z("America/Santiago", -240, Dst::Chile, "-04", "-03"),
    fixed("Asia/Tokyo", 540, "JST"),
    fixed("Asia/Seoul", 540, "KST"),
    fixed("Asia/Shanghai", 480, "CST"),
    fixed("Asia/Hong_Kong", 480, "HKT"),
    fixed("Asia/Taipei", 480, "CST"),
    fixed("Asia/Singapore", 480, "+08"),
    fixed("Asia/Kolkata", 330, "IST"),
    fixed("Asia/Dubai", 240, "+04"),
    fixed("Asia/Bangkok", 420, "+07"),
    fixed("Asia/Jakarta", 420, "WIB"),
    fixed("Asia/Manila", 480, "PST"),
    fixed("Asia/Karachi", 300, "PKT"),
    fixed("Asia/Tehran", 210, "+0330"),
    z("Asia/Jerusalem", 120, Dst::Israel, "IST", "IDT"),
    z("Africa/Cairo", 120, Dst::Egypt, "EET", "EEST"),
    fixed("Africa/Johannesburg", 120, "SAST"),
    fixed("Africa/Lagos", 60, "WAT"),
    fixed("Africa/Nairobi", 180, "EAT"),
    z("Australia/Sydney", 600, Dst::AuSouth, "AEST", "AEDT"),
    z("Australia/Melbourne", 600, Dst::AuSouth, "AEST", "AEDT"),
    fixed("Australia/Brisbane", 600, "AEST"),
    fixed("Australia/Perth", 480, "AWST"),
    z("Australia/Adelaide", 570, Dst::AuSouth, "ACST", "ACDT"),
    z("Pacific/Auckland", 720, Dst::Nz, "NZST", "NZDT"),
];

/// The zone stored as index `i`, UTC for an index this system does not know.
pub fn zone(i: usize) -> &'static Zone { ZONES.get(i).unwrap_or(&ZONES[0]) }

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;

fn year_of(unix: i64) -> i64 { crate::civil_from_unix_days(unix.div_euclid(DAY)).0 }
/// Unix day of the `n`th (1-based) `weekday` (0 = Sunday) of a month.
fn nth(year: i64, month: u8, weekday: i64, n: i64) -> i64 {
    let first = unix_days_from_civil(year, month, 1);
    // 1970-01-01 was a Thursday (4).
    let first_wd = (first + 4).rem_euclid(7);
    first + (weekday - first_wd).rem_euclid(7) + 7 * (n - 1)
}
/// Unix day of the last `weekday` of a month.
fn last(year: i64, month: u8, weekday: i64) -> i64 {
    let end = unix_days_from_civil(year, month, crate::days_in_month(year, month));
    end - (((end + 4).rem_euclid(7)) - weekday).rem_euclid(7)
}

impl Zone {
    /// The daylight period of `year` as Unix seconds [start, end). For a southern
    /// zone the period crosses the new year: start is in `year`, end in `year + 1`.
    fn dst_span(&self, year: i64) -> Option<(i64, i64)> {
        let std = self.standard as i64 * 60;
        let local = |day: i64, at: i64, offset: i64| day * DAY + at - offset;
        Some(match self.dst {
            Dst::None => return None,
            Dst::Eu => (last(year, 3, 0) * DAY + HOUR, last(year, 10, 0) * DAY + HOUR),
            Dst::Us => (local(nth(year, 3, 0, 2), 2 * HOUR, std), local(nth(year, 11, 0, 1), 2 * HOUR, std + HOUR)),
            Dst::AuSouth => (local(nth(year, 10, 0, 1), 2 * HOUR, std), local(nth(year + 1, 4, 0, 1), 3 * HOUR, std + HOUR)),
            Dst::Nz => (local(last(year, 9, 0), 2 * HOUR, std), local(nth(year + 1, 4, 0, 1), 3 * HOUR, std + HOUR)),
            Dst::Chile => (nth(year, 9, 0, 1) * DAY + 4 * HOUR, nth(year + 1, 4, 0, 1) * DAY + 3 * HOUR),
            Dst::Israel => (local(last(year, 3, 0) - 2, 2 * HOUR, std), local(last(year, 10, 0), 2 * HOUR, std + HOUR)),
            Dst::Egypt => (local(last(year, 4, 5), 0, std), local(last(year, 10, 4), 24 * HOUR, std + HOUR)),
        })
    }
    /// True when daylight-saving time is in force at Unix second `unix`.
    pub fn in_dst(&self, unix: i64) -> bool {
        let year = year_of(unix);
        // The span that starts this year, or the southern one that started last year.
        [year, year - 1].iter().any(|y| self.dst_span(*y).is_some_and(|(s, e)| s <= unix && unix < e))
    }
    /// Seconds east of UTC at `unix`, and the abbreviation in force.
    pub fn offset_at(&self, unix: i64) -> (i32, &'static str) {
        let std = self.standard as i32 * 60;
        if self.in_dst(unix) { (std + 3600, self.dst_abbr) } else { (std, self.standard_abbr) }
    }
    /// Local wall-clock seconds (Unix-style count) for UTC `unix`.
    pub fn local(&self, unix: i64) -> i64 { unix + self.offset_at(unix).0 as i64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Unix seconds of a UTC civil time.
    fn utc(y: i64, mo: u8, d: u8, h: i64, mi: i64) -> i64 { unix_days_from_civil(y, mo, d) * DAY + h * HOUR + mi * 60 }
    fn by(name: &str) -> &'static Zone { ZONES.iter().find(|z| z.name == name).unwrap() }

    #[test]
    fn table_matches_the_settings_list() {
        let names: Vec<&str> = ZONES.iter().map(|z| z.name).collect();
        assert_eq!(names, sex_config::ZONES, "same zones in the same order");
        for z in ZONES { assert!(z.standard_abbr.len() <= 5 && z.dst_abbr.len() <= 5, "{} fits the bar", z.name); }
    }

    #[test]
    fn berlin_2026() {
        // DST: 2026-03-29 01:00 UTC to 2026-10-25 01:00 UTC.
        let b = by("Europe/Berlin");
        assert_eq!(b.offset_at(utc(2026, 3, 29, 0, 59)), (3600, "CET"));
        assert_eq!(b.offset_at(utc(2026, 3, 29, 1, 0)), (7200, "CEST"));
        assert_eq!(b.offset_at(utc(2026, 10, 25, 0, 59)), (7200, "CEST"));
        assert_eq!(b.offset_at(utc(2026, 10, 25, 1, 0)), (3600, "CET"));
        assert_eq!(b.local(utc(2026, 7, 1, 12, 0)), utc(2026, 7, 1, 14, 0));
        assert_eq!(by("Europe/London").offset_at(utc(2026, 6, 1, 0, 0)), (3600, "BST"));
    }

    #[test]
    fn new_york_2026() {
        // DST: 2026-03-08 07:00 UTC (02:00 EST) to 2026-11-01 06:00 UTC (02:00 EDT).
        let n = by("America/New_York");
        assert_eq!(n.offset_at(utc(2026, 3, 8, 6, 59)).1, "EST");
        assert_eq!(n.offset_at(utc(2026, 3, 8, 7, 0)), (-4 * 3600, "EDT"));
        assert_eq!(n.offset_at(utc(2026, 11, 1, 5, 59)).1, "EDT");
        assert_eq!(n.offset_at(utc(2026, 11, 1, 6, 0)), (-5 * 3600, "EST"));
        assert_eq!(by("America/Phoenix").offset_at(utc(2026, 7, 1, 0, 0)), (-7 * 3600, "MST"));
    }

    #[test]
    fn southern_hemisphere() {
        // Sydney: DST ends 2026-04-05 03:00 AEDT (16:00 UTC on 4th), starts 2026-10-04 02:00 AEST (16:00 UTC on 3rd).
        let s = by("Australia/Sydney");
        assert_eq!(s.offset_at(utc(2026, 1, 15, 0, 0)), (11 * 3600, "AEDT"));
        assert_eq!(s.offset_at(utc(2026, 4, 4, 15, 59)).1, "AEDT");
        assert_eq!(s.offset_at(utc(2026, 4, 4, 16, 0)).1, "AEST");
        assert_eq!(s.offset_at(utc(2026, 10, 3, 15, 59)).1, "AEST");
        assert_eq!(s.offset_at(utc(2026, 10, 3, 16, 0)).1, "AEDT");
        // Auckland: ends 2026-04-05 03:00 NZDT (14:00 UTC 4th), starts 2026-09-27 02:00 NZST (14:00 UTC 26th).
        let a = by("Pacific/Auckland");
        assert_eq!(a.offset_at(utc(2026, 4, 4, 13, 59)).1, "NZDT");
        assert_eq!(a.offset_at(utc(2026, 4, 4, 14, 0)).1, "NZST");
        assert_eq!(a.offset_at(utc(2026, 9, 26, 14, 0)).1, "NZDT");
        assert_eq!(by("Asia/Kolkata").offset_at(utc(2026, 1, 1, 0, 0)), (19800, "IST"));
        assert_eq!(by("Asia/Tehran").offset_at(utc(2026, 7, 1, 0, 0)).0, 12600);
    }

    #[test]
    fn israel_and_egypt_2026() {
        // Israel: Fri 2026-03-27 02:00 IST (00:00 UTC) to Sun 2026-10-25 02:00 IDT (23:00 UTC 24th).
        let j = by("Asia/Jerusalem");
        assert_eq!(j.offset_at(utc(2026, 3, 26, 23, 59)).1, "IST");
        assert_eq!(j.offset_at(utc(2026, 3, 27, 0, 0)).1, "IDT");
        assert_eq!(j.offset_at(utc(2026, 10, 24, 22, 59)).1, "IDT");
        assert_eq!(j.offset_at(utc(2026, 10, 24, 23, 0)).1, "IST");
        // Egypt: Fri 2026-04-24 00:00 EET (22:00 UTC 23rd) to Thu 2026-10-29 24:00 EEST (21:00 UTC 29th).
        let c = by("Africa/Cairo");
        assert_eq!(c.offset_at(utc(2026, 4, 23, 21, 59)).1, "EET");
        assert_eq!(c.offset_at(utc(2026, 4, 23, 22, 0)).1, "EEST");
        assert_eq!(c.offset_at(utc(2026, 10, 29, 20, 59)).1, "EEST");
        assert_eq!(c.offset_at(utc(2026, 10, 29, 21, 0)).1, "EET");
    }

    #[test]
    fn unknown_index_is_utc() {
        assert_eq!(zone(9999).name, "UTC");
        assert_eq!(zone(0).offset_at(0), (0, "UTC"));
    }
}
