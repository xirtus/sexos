// SPDX-License-Identifier: GPL-3.0-only
//! The calendar's model, apart from its window so the host can test it: civil dates in
//! the proleptic Gregorian calendar, ISO weeks, the six-week month grid, events that
//! may repeat, the program's own event file and the iCalendar importer ([`ics`]).
//!
//! Times are wall-clock minutes. SexOS keeps no time-zone database yet, so a UTC time
//! from an `.ics` file is shown as UTC (and marked so) and a `TZID` time is taken as it
//! is written, like a floating time.
#![no_std]

extern crate alloc;

pub mod ics;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Write};

pub const MONTH_NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
/// Monday first, as ISO 8601 and most of the world count the week.
pub const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn is_leap(year: i32) -> bool { year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) }
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month { 2 if is_leap(year) => 29, 2 => 28, 4 | 6 | 9 | 11 => 30, _ => 31 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date { pub year: i32, pub month: u8, pub day: u8 }

impl Date {
    /// A real day, or None (2026-02-29 is not one).
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        ((1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)).then_some(Date { year, month, day })
    }
    /// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    pub fn days(self) -> i64 {
        let y = self.year as i64 - (self.month <= 2) as i64;
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (self.month as i64 + 9) % 12;
        let doy = (153 * mp + 2) / 5 + self.day as i64 - 1;
        era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
    }
    pub fn from_days(z: i64) -> Date {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        Date { year: (yoe + era * 400 + (month <= 2) as i64) as i32, month, day }
    }
    /// The UTC day of a Unix time in milliseconds, as the kernel's civil clock gives it.
    pub fn from_unix_ms(ms: u64) -> Date { Date::from_days((ms / 86_400_000) as i64) }
    /// 0 is Monday, 6 is Sunday (1970-01-01 was a Thursday).
    pub fn weekday(self) -> u8 { (self.days() + 3).rem_euclid(7) as u8 }
    pub fn add_days(self, n: i64) -> Date { Date::from_days(self.days() + n) }
    /// The same day `n` months on, or the month's last day when it is shorter.
    pub fn add_months(self, n: i32) -> Date {
        let index = self.year * 12 + self.month as i32 - 1 + n;
        let (year, month) = (index.div_euclid(12), (index.rem_euclid(12) + 1) as u8);
        Date { year, month, day: self.day.min(days_in_month(year, month)) }
    }
    /// The ISO 8601 week-numbering year and week: the week belongs to the year that
    /// holds its Thursday.
    pub fn iso_week(self) -> (i32, u8) {
        let thursday = self.add_days(3 - self.weekday() as i64);
        let jan1 = Date { year: thursday.year, month: 1, day: 1 };
        (thursday.year, ((thursday.days() - jan1.days()) / 7 + 1) as u8)
    }
    /// `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Option<Date> {
        let b = text.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' { return None; }
        Date::new(text[..4].parse().ok()?, text[5..7].parse().ok()?, text[8..].parse().ok()?)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day) }
}

/// A month as a calendar page: six rows of seven days from the Monday on or before its
/// first, so every month fits and the page never changes height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonthGrid { pub year: i32, pub month: u8, pub start: Date }

impl MonthGrid {
    pub const CELLS: usize = 42;
    pub fn new(year: i32, month: u8) -> MonthGrid {
        let first = Date { year, month, day: 1 };
        MonthGrid { year, month, start: first.add_days(-(first.weekday() as i64)) }
    }
    pub fn of(day: Date) -> MonthGrid { MonthGrid::new(day.year, day.month) }
    pub fn cell(&self, i: usize) -> Date { self.start.add_days(i as i64) }
    pub fn in_month(&self, d: Date) -> bool { d.year == self.year && d.month == self.month }
    pub fn index_of(&self, d: Date) -> Option<usize> {
        let i = d.days() - self.start.days();
        (0..Self::CELLS as i64).contains(&i).then_some(i as usize)
    }
    /// The ISO week number shown beside row `row`.
    pub fn week_number(&self, row: usize) -> u8 { self.cell(row * 7).iso_week().1 }
    pub fn end(&self) -> Date { self.cell(Self::CELLS - 1) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq { Daily, Weekly, Monthly, Yearly }

/// A repeat rule: RFC 5545's RRULE without the BY parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    /// How many occurrences in all, the first included.
    pub count: Option<u32>,
    /// The last moment an occurrence may start: a day and minute of that day.
    pub until: Option<(Date, u16)>,
}

impl Rule {
    /// The value of an RRULE (`FREQ=WEEKLY;INTERVAL=2;COUNT=5`). Parts it does not know
    /// are ignored; a rule without FREQ is none.
    pub fn parse(text: &str) -> Option<Rule> {
        let mut rule = Rule { freq: Freq::Daily, interval: 1, count: None, until: None };
        let mut freq = None;
        for part in text.split(';') {
            let (key, value) = part.split_once('=')?;
            match key.trim().to_ascii_uppercase().as_str() {
                "FREQ" => freq = Some(match value.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily, "WEEKLY" => Freq::Weekly, "MONTHLY" => Freq::Monthly, "YEARLY" => Freq::Yearly,
                    _ => return None,
                }),
                "INTERVAL" => rule.interval = value.trim().parse().ok().filter(|n| *n > 0)?,
                "COUNT" => rule.count = Some(value.trim().parse().ok()?),
                "UNTIL" => rule.until = ics::parse_when(value.trim()).map(|w| (w.date, w.minute.unwrap_or(24 * 60 - 1))),
                _ => {}
            }
        }
        rule.freq = freq?;
        Some(rule)
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let freq = match self.freq { Freq::Daily => "DAILY", Freq::Weekly => "WEEKLY", Freq::Monthly => "MONTHLY", Freq::Yearly => "YEARLY" };
        write!(f, "FREQ={freq}")?;
        if self.interval != 1 { write!(f, ";INTERVAL={}", self.interval)?; }
        if let Some(n) = self.count { write!(f, ";COUNT={n}")?; }
        if let Some((d, m)) = self.until { write!(f, ";UNTIL={:04}{:02}{:02}T{:02}{:02}00", d.year, d.month, d.day, m / 60, m % 60)?; }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub start: Date,
    /// Start and end, in minutes of the day; None for an all-day event.
    pub time: Option<(u16, u16)>,
    /// The times are UTC (an `.ics` time ending in Z).
    pub utc: bool,
    /// Days each occurrence covers, at least one.
    pub span: u32,
    pub title: String,
    pub location: String,
    pub rule: Option<Rule>,
}

/// Rules are walked at most this far, so a hostile COUNT cannot stall the program.
const MAX_STEPS: u32 = 100_000;

impl Event {
    pub fn all_day(start: Date, title: &str) -> Event {
        Event { start, time: None, utc: false, span: 1, title: title.into(), location: String::new(), rule: None }
    }
    /// The `n`th date the rule produces, before RFC 5545's rule that a date which does
    /// not exist (the 31st of a short month, 29 February) is skipped, not moved.
    fn nth(&self, rule: &Rule, n: u32) -> Option<Date> {
        let step = n as i64 * rule.interval as i64;
        let s = self.start;
        match rule.freq {
            Freq::Daily => Some(s.add_days(step)),
            Freq::Weekly => Some(s.add_days(7 * step)),
            Freq::Monthly => { let m = s.add_months(step as i32); Date::new(m.year, m.month, s.day) }
            Freq::Yearly => Date::new(s.year + step as i32, s.month, s.day),
        }
    }
    /// Calls `f` with each occurrence's first day that lies in `from..=to`.
    pub fn starts_between(&self, from: Date, to: Date, mut f: impl FnMut(Date)) {
        let Some(rule) = self.rule else {
            if self.start >= from && self.start <= to { f(self.start); }
            return;
        };
        let minute = self.time.map_or(0, |t| t.0);
        // Without COUNT, a daily or weekly rule can jump straight to the range.
        let mut n = match (rule.freq, rule.count) {
            (Freq::Daily | Freq::Weekly, None) => {
                let unit = rule.interval as i64 * if rule.freq == Freq::Weekly { 7 } else { 1 };
                ((from.days() - self.start.days()).max(0) / unit) as u32
            }
            _ => 0,
        };
        let mut made = 0u32;
        for _ in 0..MAX_STEPS {
            let Some(day) = self.nth(&rule, n) else { n += 1; continue };
            n += 1;
            if day > to || rule.until.is_some_and(|u| (day, minute) > u) || rule.count.is_some_and(|c| made >= c) { return; }
            made += 1;
            if day >= from { f(day); }
        }
    }
    /// Whether some occurrence covers `day`.
    pub fn occurs_on(&self, day: Date) -> bool {
        let mut hit = false;
        self.starts_between(day.add_days(1 - self.span.max(1) as i64), day, |_| hit = true);
        hit
    }
}

/// The program's events. Saved as text, one event a line, tab-separated:
///
/// ```text
/// # sex-calendar 1
/// 2026-03-16	09:00-10:00	Z	1	FREQ=WEEKLY;COUNT=4	Team sync	Room 4
/// ```
///
/// Fields: first day, time or `-` for all day, `Z` or `-`, days covered, rule or `-`,
/// title, location. Tabs, newlines and backslashes in text are escaped as `\t` `\n` `\\`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calendar { pub events: Vec<Event> }

pub const HEADER: &str = "# sex-calendar 1";

fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c { '\t' => out.push_str("\\t"), '\n' => out.push_str("\\n"), '\\' => out.push_str("\\\\"), '\r' => {}, c => out.push(c) }
    }
}
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() { Some('t') => out.push('\t'), Some('n') => out.push('\n'), Some(c) => out.push(c), None => {} }
    }
    out
}
fn minutes(text: &str) -> Option<u16> {
    let (h, m) = text.split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl Calendar {
    /// Reads the event file. Lines it cannot read are skipped, so one damaged line
    /// costs that event and not the calendar.
    pub fn parse(text: &str) -> Calendar {
        let mut events = Vec::new();
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() { continue; }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 7 { continue; }
            let Some(start) = Date::parse(f[0]) else { continue };
            let time = if f[1] == "-" { None } else {
                let Some((a, b)) = f[1].split_once('-') else { continue };
                let (Some(a), Some(b)) = (minutes(a), minutes(b)) else { continue };
                Some((a, b))
            };
            let Ok(span) = f[3].parse::<u32>() else { continue };
            let rule = if f[4] == "-" { None } else { Rule::parse(f[4]) };
            events.push(Event { start, time, utc: f[2] == "Z", span: span.clamp(1, 3660), title: unescape(f[5]), location: unescape(f[6]), rule });
        }
        Calendar { events }
    }
    pub fn serialize(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        for e in &self.events {
            let _ = write!(out, "{}\t", e.start);
            match e.time {
                Some((a, b)) => { let _ = write!(out, "{:02}:{:02}-{:02}:{:02}\t", a / 60, a % 60, b / 60, b % 60); }
                None => out.push_str("-\t"),
            }
            out.push_str(if e.utc { "Z\t" } else { "-\t" });
            let _ = write!(out, "{}\t", e.span);
            match e.rule { Some(r) => { let _ = write!(out, "{r}\t"); } None => out.push_str("-\t") }
            escape(&mut out, &e.title);
            out.push('\t');
            escape(&mut out, &e.location);
            out.push('\n');
        }
        out
    }
    /// The events on `day`: all-day ones first, then by start time, then by title.
    pub fn on_day(&self, day: Date) -> Vec<&Event> {
        let mut found: Vec<&Event> = self.events.iter().filter(|e| e.occurs_on(day)).collect();
        found.sort_by(|a, b| (a.time.map(|t| t.0), &a.title).cmp(&(b.time.map(|t| t.0), &b.title)));
        found
    }
    /// How many events fall on each cell of `grid`, in one pass per event.
    pub fn counts(&self, grid: &MonthGrid) -> [u16; MonthGrid::CELLS] {
        let mut counts = [0u16; MonthGrid::CELLS];
        for e in &self.events {
            let mut covered = [false; MonthGrid::CELLS];
            e.starts_between(grid.start.add_days(1 - e.span.max(1) as i64), grid.end(), |s| {
                for k in 0..e.span.max(1) as i64 {
                    if let Some(i) = grid.index_of(s.add_days(k)) { covered[i] = true; }
                }
            });
            for (c, hit) in counts.iter_mut().zip(covered) { *c += hit as u16; }
        }
        counts
    }
    /// Adds events not already present (same day, time, title and rule), so the same
    /// file imported twice does not double the calendar. Returns how many were new.
    pub fn merge(&mut self, events: Vec<Event>) -> usize {
        let mut added = 0;
        for e in events {
            if !self.events.iter().any(|o| o.start == e.start && o.time == e.time && o.title == e.title && o.rule == e.rule) {
                self.events.push(e);
                added += 1;
            }
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    fn d(y: i32, m: u8, day: u8) -> Date { Date::new(y, m, day).unwrap() }

    #[test]
    fn civil_round_trip() {
        assert_eq!(d(1970, 1, 1).days(), 0);
        assert_eq!(d(2000, 3, 1).days(), 11_017);
        assert_eq!(Date::from_days(-1), d(1969, 12, 31));
        for z in (-800_000..800_000).step_by(997) { assert_eq!(Date::from_days(z).days(), z); }
        assert_eq!(Date::from_unix_ms(1_793_836_800_000), d(2026, 11, 5));
    }
    #[test]
    fn months_and_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(Date::new(2026, 2, 29), None);
        assert_eq!(Date::new(2026, 13, 1), None);
        assert_eq!(d(2026, 1, 31).add_months(1), d(2026, 2, 28));
        assert_eq!(d(2026, 3, 15).add_months(-3), d(2025, 12, 15));
        assert_eq!(d(2024, 2, 29).add_months(12), d(2025, 2, 28));
    }
    #[test]
    fn weekdays_and_iso_weeks() {
        assert_eq!(d(2026, 11, 13).weekday(), 4); // a Friday
        assert_eq!(d(2000, 1, 1).weekday(), 5); // a Saturday
        assert_eq!(d(2026, 1, 1).iso_week(), (2026, 1));
        assert_eq!(d(2021, 1, 3).iso_week(), (2020, 53));
        assert_eq!(d(2024, 12, 30).iso_week(), (2025, 1));
        assert_eq!(d(2026, 12, 31).iso_week(), (2026, 53));
        assert_eq!(d(2027, 1, 3).iso_week(), (2026, 53));
    }
    #[test]
    fn grid() {
        let g = MonthGrid::new(2026, 3);
        assert_eq!(g.start, d(2026, 2, 23));
        assert_eq!(g.index_of(d(2026, 3, 1)), Some(6));
        assert_eq!(g.end(), d(2026, 4, 5));
        assert_eq!(g.week_number(0), 9);
        // A month starting on Monday starts the grid on its first.
        assert_eq!(MonthGrid::new(2026, 6).start, d(2026, 6, 1));
    }
    #[test]
    fn store_round_trip() {
        let mut c = Calendar::default();
        c.events.push(Event::all_day(d(2026, 3, 1), "tab\there \\ and\nnewline"));
        c.events.push(Event { start: d(2026, 3, 2), time: Some((9 * 60, 10 * 60 + 30)), utc: true, span: 1, title: "Sync".into(), location: "Room 4".into(),
            rule: Some(Rule { freq: Freq::Weekly, interval: 2, count: Some(3), until: None }) });
        let text = c.serialize();
        assert!(text.starts_with(HEADER));
        assert_eq!(Calendar::parse(&text), c);
        assert_eq!(Calendar::parse("garbage\n2026-02-30\t-\t-\t1\t-\tx\t\n"), Calendar::default());
    }
    #[test]
    fn repeats() {
        let mut e = Event::all_day(d(2026, 1, 31), "Review");
        e.rule = Rule::parse("FREQ=MONTHLY;COUNT=4");
        let mut seen = Vec::new();
        e.starts_between(d(2026, 1, 1), d(2026, 12, 31), |s| seen.push(s));
        // February, April and June have no 31st: skipped, and not counted.
        assert_eq!(seen, [d(2026, 1, 31), d(2026, 3, 31), d(2026, 5, 31), d(2026, 7, 31)]);
        let mut daily = Event::all_day(d(2020, 1, 1), "Pill");
        daily.rule = Rule::parse("FREQ=DAILY;INTERVAL=3");
        assert!(daily.occurs_on(d(2020, 1, 4)) && !daily.occurs_on(d(2020, 1, 5)));
        assert!(daily.occurs_on(d(2020, 1, 1).add_days(3 * 10_000)));
        let mut leap = Event::all_day(d(2024, 2, 29), "Leap");
        leap.rule = Rule::parse("FREQ=YEARLY");
        assert!(leap.occurs_on(d(2028, 2, 29)) && !leap.occurs_on(d(2025, 2, 28)));
        let mut until = Event::all_day(d(2026, 3, 1), "Until");
        until.rule = Rule::parse("FREQ=WEEKLY;UNTIL=20260315");
        assert!(until.occurs_on(d(2026, 3, 15)) && !until.occurs_on(d(2026, 3, 22)));
        assert_eq!(Rule::parse("FREQ=WEEKLY;INTERVAL=2;COUNT=5").unwrap().to_string(), "FREQ=WEEKLY;INTERVAL=2;COUNT=5");
        assert_eq!(Rule::parse("INTERVAL=2"), None);
    }
}
