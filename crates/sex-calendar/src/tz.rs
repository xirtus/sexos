// SPDX-License-Identifier: GPL-3.0-only
//! A small built-in table of time zones for the world clock and the local zone:
//! each zone's standard offset and its daylight-saving rule as written in law today
//! (EU, US/Canada, southern Australia, New Zealand, or none). SexOS carries no tz
//! database yet; past rule changes are not modelled.
use crate::Date;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dst {
    None,
    /// Last Sunday of March 01:00 UTC to last Sunday of October 01:00 UTC.
    Eu,
    /// Second Sunday of March 02:00 local to first Sunday of November 02:00 local.
    Us,
    /// First Sunday of October 02:00 standard to first Sunday of April 03:00 daylight.
    AuSouth,
    /// Last Sunday of September 02:00 standard to first Sunday of April 03:00 daylight.
    Nz,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone { pub name: &'static str, pub city: &'static str, /** Minutes east of UTC in standard time. */ pub standard: i32, pub dst: Dst }

const fn z(name: &'static str, city: &'static str, standard: i32, dst: Dst) -> Zone { Zone { name, city, standard, dst } }

pub const ZONES: &[Zone] = &[
    z("UTC", "UTC", 0, Dst::None),
    z("Europe/London", "London", 0, Dst::Eu),
    z("Europe/Dublin", "Dublin", 0, Dst::Eu),
    z("Europe/Lisbon", "Lisbon", 0, Dst::Eu),
    z("Europe/Paris", "Paris", 60, Dst::Eu),
    z("Europe/Berlin", "Berlin", 60, Dst::Eu),
    z("Europe/Amsterdam", "Amsterdam", 60, Dst::Eu),
    z("Europe/Madrid", "Madrid", 60, Dst::Eu),
    z("Europe/Rome", "Rome", 60, Dst::Eu),
    z("Europe/Stockholm", "Stockholm", 60, Dst::Eu),
    z("Europe/Warsaw", "Warsaw", 60, Dst::Eu),
    z("Europe/Athens", "Athens", 120, Dst::Eu),
    z("Europe/Helsinki", "Helsinki", 120, Dst::Eu),
    z("Europe/Kyiv", "Kyiv", 120, Dst::Eu),
    z("Europe/Istanbul", "Istanbul", 180, Dst::None),
    z("Europe/Moscow", "Moscow", 180, Dst::None),
    z("Africa/Lagos", "Lagos", 60, Dst::None),
    z("Africa/Johannesburg", "Johannesburg", 120, Dst::None),
    z("Africa/Nairobi", "Nairobi", 180, Dst::None),
    z("Asia/Dubai", "Dubai", 240, Dst::None),
    z("Asia/Karachi", "Karachi", 300, Dst::None),
    z("Asia/Kolkata", "Kolkata", 330, Dst::None),
    z("Asia/Kathmandu", "Kathmandu", 345, Dst::None),
    z("Asia/Bangkok", "Bangkok", 420, Dst::None),
    z("Asia/Jakarta", "Jakarta", 420, Dst::None),
    z("Asia/Shanghai", "Shanghai", 480, Dst::None),
    z("Asia/Hong_Kong", "Hong Kong", 480, Dst::None),
    z("Asia/Singapore", "Singapore", 480, Dst::None),
    z("Asia/Tokyo", "Tokyo", 540, Dst::None),
    z("Asia/Seoul", "Seoul", 540, Dst::None),
    z("Australia/Perth", "Perth", 480, Dst::None),
    z("Australia/Brisbane", "Brisbane", 600, Dst::None),
    z("Australia/Adelaide", "Adelaide", 570, Dst::AuSouth),
    z("Australia/Sydney", "Sydney", 600, Dst::AuSouth),
    z("Australia/Melbourne", "Melbourne", 600, Dst::AuSouth),
    z("Pacific/Auckland", "Auckland", 720, Dst::Nz),
    z("Pacific/Honolulu", "Honolulu", -600, Dst::None),
    z("America/Anchorage", "Anchorage", -540, Dst::Us),
    z("America/Los_Angeles", "Los Angeles", -480, Dst::Us),
    z("America/Vancouver", "Vancouver", -480, Dst::Us),
    z("America/Denver", "Denver", -420, Dst::Us),
    z("America/Phoenix", "Phoenix", -420, Dst::None),
    z("America/Chicago", "Chicago", -360, Dst::Us),
    z("America/Mexico_City", "Mexico City", -360, Dst::None),
    z("America/New_York", "New York", -300, Dst::Us),
    z("America/Toronto", "Toronto", -300, Dst::Us),
    z("America/Halifax", "Halifax", -240, Dst::Us),
    z("America/Sao_Paulo", "Sao Paulo", -180, Dst::None),
    z("America/Argentina/Buenos_Aires", "Buenos Aires", -180, Dst::None),
];

/// A zone by its tz name or city, ignoring case.
pub fn find(name: &str) -> Option<&'static Zone> {
    let name = name.trim();
    ZONES.iter().find(|z| z.name.eq_ignore_ascii_case(name) || z.city.eq_ignore_ascii_case(name))
}

/// The `n`th (1-based; 0 the last) Sunday of a month.
fn sunday(year: i32, month: u8, n: u8) -> Date {
    if n == 0 {
        let last = Date { year, month, day: crate::days_in_month(year, month) };
        return last.add_days(-(((last.weekday() as i64) + 1) % 7));
    }
    let first = Date { year, month, day: 1 };
    first.add_days((6 - first.weekday() as i64).rem_euclid(7) + 7 * (n as i64 - 1))
}

/// Minutes since 1970 UTC of a local wall time in a zone `offset` minutes east.
fn at(d: Date, minute: i64, offset: i32) -> i64 { d.days() * 1440 + minute - offset as i64 }

impl Zone {
    /// Whether daylight-saving time is in force at `utc` (minutes since 1970 UTC).
    pub fn is_dst(&self, utc: i64) -> bool {
        let year = Date::from_days(utc.div_euclid(1440)).year;
        let s = self.standard;
        match self.dst {
            Dst::None => false,
            Dst::Eu => utc >= at(sunday(year, 3, 0), 60, 0) && utc < at(sunday(year, 10, 0), 60, 0),
            Dst::Us => utc >= at(sunday(year, 3, 2), 120, s) && utc < at(sunday(year, 11, 1), 120, s + 60),
            // Southern summers span the new year: daylight time outside April..October.
            Dst::AuSouth => !(utc >= at(sunday(year, 4, 1), 180, s + 60) && utc < at(sunday(year, 10, 1), 120, s)),
            Dst::Nz => !(utc >= at(sunday(year, 4, 1), 180, s + 60) && utc < at(sunday(year, 9, 0), 120, s)),
        }
    }
    /// Minutes east of UTC at `utc` (minutes since 1970 UTC).
    pub fn offset(&self, utc: i64) -> i32 { self.standard + if self.is_dst(utc) { 60 } else { 0 } }
    /// The local day and minute of the day at `utc`.
    pub fn local(&self, utc: i64) -> (Date, u16) {
        let m = utc + self.offset(utc) as i64;
        (Date::from_days(m.div_euclid(1440)), m.rem_euclid(1440) as u16)
    }
}

/// `+05:30`, `-03:00`, `+00:00`.
pub fn format_offset(minutes: i32) -> alloc::string::String {
    let sign = if minutes < 0 { '-' } else { '+' };
    alloc::format!("{sign}{:02}:{:02}", minutes.abs() / 60, minutes.abs() % 60)
}
