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

pub mod clock;
pub mod ics;
pub mod tz;

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

/// A repeat rule: RFC 5545's RRULE at day resolution: FREQ (DAILY to YEARLY),
/// INTERVAL, COUNT, UNTIL, BYDAY (with ordinals such as `-1FR`), BYMONTHDAY (negative
/// counts from the month's end), BYMONTH, BYSETPOS and WKST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    /// How many occurrences in all, the first included.
    pub count: Option<u32>,
    /// The last moment an occurrence may start: a day and minute of that day.
    pub until: Option<(Date, u16)>,
    /// Weekdays (0 Monday) with an ordinal: 0 every one, 2 the second, -1 the last.
    pub by_day: Vec<(i8, u8)>,
    pub by_month_day: Vec<i8>,
    pub by_month: Vec<u8>,
    pub by_set_pos: Vec<i16>,
    /// The first day of the week, for WEEKLY rules with an INTERVAL (0 Monday).
    pub wkst: u8,
}

const DAY_CODES: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

fn list<T: core::str::FromStr>(value: &str) -> Option<Vec<T>> {
    value.split(',').map(|v| v.trim().parse().ok()).collect()
}

impl Rule {
    pub fn new(freq: Freq) -> Rule {
        Rule { freq, interval: 1, count: None, until: None, by_day: Vec::new(), by_month_day: Vec::new(), by_month: Vec::new(), by_set_pos: Vec::new(), wkst: 0 }
    }
    /// The value of an RRULE (`FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=5`). Parts it
    /// does not know are ignored; a rule without FREQ, or with a part it cannot read,
    /// is none.
    pub fn parse(text: &str) -> Option<Rule> {
        let mut rule = Rule::new(Freq::Daily);
        let mut freq = None;
        for part in text.split(';').filter(|p| !p.trim().is_empty()) {
            let (key, value) = part.split_once('=')?;
            let value = value.trim();
            match key.trim().to_ascii_uppercase().as_str() {
                "FREQ" => freq = Some(match value.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily, "WEEKLY" => Freq::Weekly, "MONTHLY" => Freq::Monthly, "YEARLY" => Freq::Yearly,
                    _ => return None,
                }),
                "INTERVAL" => rule.interval = value.parse().ok().filter(|n| *n > 0)?,
                "COUNT" => rule.count = Some(value.parse().ok()?),
                "UNTIL" => rule.until = ics::parse_when(value).map(|w| (w.date, w.minute.unwrap_or(24 * 60 - 1))),
                "BYDAY" => {
                    for d in value.split(',') {
                        let d = d.trim().to_ascii_uppercase();
                        if d.len() < 2 { return None; }
                        let (n, code) = d.split_at(d.len() - 2);
                        let wd = DAY_CODES.iter().position(|c| *c == code)? as u8;
                        let n: i8 = if n.is_empty() { 0 } else { n.trim_start_matches('+').parse().ok().filter(|n: &i8| (-53..=53).contains(n) && *n != 0)? };
                        rule.by_day.push((n, wd));
                    }
                }
                "BYMONTHDAY" => rule.by_month_day = list::<i8>(value)?.into_iter().filter(|d| *d != 0 && (-31..=31).contains(d)).collect(),
                "BYMONTH" => rule.by_month = list::<u8>(value)?.into_iter().filter(|m| (1..=12).contains(m)).collect(),
                "BYSETPOS" => rule.by_set_pos = list::<i16>(value)?.into_iter().filter(|p| *p != 0).collect(),
                "WKST" => rule.wkst = DAY_CODES.iter().position(|c| value.eq_ignore_ascii_case(c))? as u8,
                _ => {}
            }
        }
        rule.freq = freq?;
        Some(rule)
    }

    /// The days of a month the BYMONTHDAY and BYDAY parts pick, in order.
    fn month_days(&self, year: i32, month: u8, start_day: u8) -> Vec<Date> {
        let last = days_in_month(year, month) as i32;
        let mut out = Vec::new();
        if !self.by_month_day.is_empty() {
            for &md in &self.by_month_day {
                let day = if md > 0 { md as i32 } else { last + 1 + md as i32 };
                if (1..=last).contains(&day) { out.push(Date { year, month, day: day as u8 }); }
            }
            // BYDAY then only limits.
            if !self.by_day.is_empty() { out.retain(|d| self.by_day.iter().any(|&(_, wd)| wd == d.weekday())); }
        } else if !self.by_day.is_empty() {
            let first = Date { year, month, day: 1 };
            for &(n, wd) in &self.by_day {
                let first_wd = 1 + (wd as i32 - first.weekday() as i32).rem_euclid(7);
                let all: Vec<i32> = (0..5).map(|k| first_wd + 7 * k).filter(|d| *d <= last).collect();
                let pick = |i: i32| Date { year, month, day: i as u8 };
                match n {
                    0 => out.extend(all.iter().map(|d| pick(*d))),
                    n if n > 0 => if let Some(d) = all.get(n as usize - 1) { out.push(pick(*d)) },
                    n => if let Some(d) = all.len().checked_sub((-n) as usize).and_then(|i| all.get(i)) { out.push(pick(*d)) },
                }
            }
        } else if let Some(d) = Date::new(year, month, start_day) {
            out.push(d);
        }
        out
    }

    /// The candidate days of period `k` (the `k`th day, week, month or year counted in
    /// INTERVALs from `start`), sorted, before COUNT and UNTIL. The first day of the
    /// period comes too, so the caller knows when it has passed the range.
    fn period(&self, start: Date, k: i64) -> (Date, Vec<Date>) {
        let step = k * self.interval as i64;
        let month_ok = |d: &Date| self.by_month.is_empty() || self.by_month.contains(&d.month);
        let (first, days) = match self.freq {
            Freq::Daily => {
                let d = start.add_days(step);
                let ok = month_ok(&d)
                    && (self.by_month_day.is_empty() || self.by_month_day.iter().any(|&md| {
                        let last = days_in_month(d.year, d.month) as i32;
                        (if md > 0 { md as i32 } else { last + 1 + md as i32 }) == d.day as i32
                    }))
                    && (self.by_day.is_empty() || self.by_day.iter().any(|&(_, wd)| wd == d.weekday()));
                return (d, if ok { alloc::vec![d] } else { Vec::new() });
            }
            Freq::Weekly => {
                let week = start.add_days(-((start.weekday() as i64 - self.wkst as i64).rem_euclid(7)) + 7 * step);
                let mut v: Vec<Date> = if self.by_day.is_empty() {
                    alloc::vec![week.add_days((start.weekday() as i64 - self.wkst as i64).rem_euclid(7))]
                } else {
                    (0..7).map(|i| week.add_days(i)).filter(|d| self.by_day.iter().any(|&(_, wd)| wd == d.weekday())).collect()
                };
                v.retain(month_ok);
                return (week, self.set_pos(v));
            }
            Freq::Monthly => {
                let m = Date { year: start.year, month: start.month, day: 1 }.add_months(step as i32);
                let mut v = self.month_days(m.year, m.month, start.day);
                v.retain(month_ok);
                (m, v)
            }
            Freq::Yearly => {
                let year = start.year + step as i32;
                let jan1 = Date { year, month: 1, day: 1 };
                let v = if !self.by_month.is_empty() {
                    let mut months = self.by_month.clone();
                    months.sort_unstable();
                    months.iter().flat_map(|&m| self.month_days(year, m, start.day)).collect()
                } else if !self.by_month_day.is_empty() {
                    (1..=12).flat_map(|m| self.month_days(year, m, start.day)).collect()
                } else if !self.by_day.is_empty() {
                    // Ordinals count within the year: 20MO is the twentieth Monday.
                    let len = if is_leap(year) { 366 } else { 365 };
                    let mut v = Vec::new();
                    for &(n, wd) in &self.by_day {
                        let first = (wd as i64 - jan1.weekday() as i64).rem_euclid(7);
                        let all: Vec<i64> = (0..54).map(|k| first + 7 * k).filter(|d| *d < len).collect();
                        match n {
                            0 => v.extend(all.iter().map(|d| jan1.add_days(*d))),
                            n if n > 0 => if let Some(d) = all.get(n as usize - 1) { v.push(jan1.add_days(*d)) },
                            n => if let Some(d) = all.len().checked_sub((-n) as usize).and_then(|i| all.get(i)) { v.push(jan1.add_days(*d)) },
                        }
                    }
                    v
                } else {
                    Date::new(year, start.month, start.day).into_iter().collect()
                };
                (jan1, v)
            }
        };
        (first, self.set_pos(days))
    }

    fn set_pos(&self, mut days: Vec<Date>) -> Vec<Date> {
        days.sort_unstable();
        days.dedup();
        if self.by_set_pos.is_empty() { return days; }
        let n = days.len() as i16;
        let mut out: Vec<Date> = self.by_set_pos.iter().filter_map(|&p| {
            let i = if p > 0 { p - 1 } else { n + p };
            (0..n).contains(&i).then(|| days[i as usize])
        }).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The period to start walking at so that nothing before `from` is missed, when no
    /// COUNT forces a walk from the start.
    fn skip_to(&self, start: Date, from: Date) -> i64 {
        let gap = from.days() - start.days();
        if gap <= 0 || self.count.is_some() { return 0; }
        let unit = match self.freq {
            Freq::Daily => gap,
            Freq::Weekly => gap / 7,
            Freq::Monthly => (from.year as i64 * 12 + from.month as i64) - (start.year as i64 * 12 + start.month as i64),
            Freq::Yearly => (from.year - start.year) as i64,
        };
        (unit / self.interval as i64 - 1).max(0)
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let freq = match self.freq { Freq::Daily => "DAILY", Freq::Weekly => "WEEKLY", Freq::Monthly => "MONTHLY", Freq::Yearly => "YEARLY" };
        write!(f, "FREQ={freq}")?;
        if self.interval != 1 { write!(f, ";INTERVAL={}", self.interval)?; }
        if let Some(n) = self.count { write!(f, ";COUNT={n}")?; }
        if let Some((d, m)) = self.until { write!(f, ";UNTIL={:04}{:02}{:02}T{:02}{:02}00", d.year, d.month, d.day, m / 60, m % 60)?; }
        let sep = |f: &mut fmt::Formatter, i: usize, name: &str| if i == 0 { write!(f, ";{name}=") } else { f.write_str(",") };
        for (i, (n, wd)) in self.by_day.iter().enumerate() {
            sep(f, i, "BYDAY")?;
            if *n != 0 { write!(f, "{n}")?; }
            f.write_str(DAY_CODES[*wd as usize % 7])?;
        }
        for (i, d) in self.by_month_day.iter().enumerate() { sep(f, i, "BYMONTHDAY")?; write!(f, "{d}")?; }
        for (i, m) in self.by_month.iter().enumerate() { sep(f, i, "BYMONTH")?; write!(f, "{m}")?; }
        for (i, p) in self.by_set_pos.iter().enumerate() { sep(f, i, "BYSETPOS")?; write!(f, "{p}")?; }
        if self.wkst != 0 { write!(f, ";WKST={}", DAY_CODES[self.wkst as usize % 7])?; }
        Ok(())
    }
}

/// An attendee's answer to an invitation (RFC 5545 PARTSTAT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartStat { NeedsAction, Accepted, Declined, Tentative }

impl PartStat {
    pub fn parse(text: &str) -> PartStat {
        match text.to_ascii_uppercase().as_str() { "ACCEPTED" => PartStat::Accepted, "DECLINED" => PartStat::Declined, "TENTATIVE" => PartStat::Tentative, _ => PartStat::NeedsAction }
    }
    pub fn as_str(self) -> &'static str {
        match self { PartStat::NeedsAction => "NEEDS-ACTION", PartStat::Accepted => "ACCEPTED", PartStat::Declined => "DECLINED", PartStat::Tentative => "TENTATIVE" }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attendee {
    /// The calendar address, usually `mailto:someone@example.org`.
    pub address: String,
    pub name: String,
    pub partstat: PartStat,
    pub rsvp: bool,
}

/// Where an event stands against its server copy (CalDAV).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// As the server has it (or never on a server).
    Clean,
    /// Made here, not yet on the server.
    New,
    /// Changed here since the server's copy.
    Modified,
    /// Removed here; the server still has it.
    Deleted,
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
    /// The iCalendar UID; empty for an event made before UIDs were kept.
    pub uid: String,
    /// Which calendar of [`Calendar::calendars`] it belongs to (0 is the first).
    pub calendar: u16,
    /// Days a repeat skips (EXDATE) and days it adds (RDATE).
    pub exdates: Vec<Date>,
    pub rdates: Vec<Date>,
    /// This event replaces one occurrence of the repeating event with the same UID.
    pub recurrence_id: Option<Date>,
    /// Reminders, in minutes before the start (negative: after it).
    pub alarms: Vec<i32>,
    pub description: String,
    pub organizer: String,
    pub attendees: Vec<Attendee>,
    /// It came as an invitation (METHOD:REQUEST) and waits for an answer.
    pub invitation: bool,
    pub sequence: u32,
    /// Its server copy: the resource's address and entity tag, and how it differs.
    pub href: String,
    pub etag: String,
    pub state: SyncState,
}

impl Default for Event {
    fn default() -> Event {
        Event { start: Date { year: 1970, month: 1, day: 1 }, time: None, utc: false, span: 1, title: String::new(), location: String::new(), rule: None,
            uid: String::new(), calendar: 0, exdates: Vec::new(), rdates: Vec::new(), recurrence_id: None, alarms: Vec::new(), description: String::new(),
            organizer: String::new(), attendees: Vec::new(), invitation: false, sequence: 0, href: String::new(), etag: String::new(), state: SyncState::Clean }
    }
}

/// Rules are walked at most this far, so a hostile COUNT cannot stall the program.
const MAX_STEPS: u32 = 100_000;

impl Event {
    pub fn all_day(start: Date, title: &str) -> Event {
        Event { start, title: title.into(), ..Event::default() }
    }
    /// Calls `f` with each occurrence's first day that lies in `from..=to`, in order.
    /// RFC 5545: DTSTART is the first occurrence, COUNT counts before EXDATE removes
    /// any, RDATE adds days, and a day that does not exist is skipped, not moved.
    pub fn starts_between(&self, from: Date, to: Date, mut f: impl FnMut(Date)) {
        let mut found = Vec::new();
        match &self.rule {
            None => if self.start >= from && self.start <= to { found.push(self.start) },
            Some(rule) => {
                let minute = self.time.map_or(0, |t| t.0);
                let past_until = |day: Date| rule.until.is_some_and(|u| (day, minute) > u);
                let mut made = 0u32;
                if !past_until(self.start) {
                    made = 1;
                    if self.start >= from && self.start <= to { found.push(self.start); }
                }
                let mut k = rule.skip_to(self.start, from);
                'walk: for _ in 0..MAX_STEPS {
                    let (first, days) = rule.period(self.start, k);
                    k += 1;
                    if first > to || rule.until.is_some_and(|u| first > u.0) { break; }
                    for day in days {
                        if day <= self.start { continue; }
                        if day > to || past_until(day) || rule.count.is_some_and(|c| made >= c) { break 'walk; }
                        made += 1;
                        if day >= from { found.push(day); }
                    }
                    if rule.count.is_some_and(|c| made >= c) { break; }
                }
            }
        }
        found.extend(self.rdates.iter().filter(|d| **d >= from && **d <= to));
        found.sort_unstable();
        found.dedup();
        for d in found { if !self.exdates.contains(&d) { f(d); } }
    }
    /// Whether some occurrence covers `day`.
    pub fn occurs_on(&self, day: Date) -> bool {
        let mut hit = false;
        self.starts_between(day.add_days(1 - self.span.max(1) as i64), day, |_| hit = true);
        hit
    }
    /// The attendee that is this calendar's owner: one of `me` (addresses, compared
    /// without `mailto:` and case), else the only one still asked to answer.
    pub fn my_attendee(&self, me: &[&str]) -> Option<usize> {
        let bare = |a: &str| String::from(a.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:")).to_ascii_lowercase();
        if let Some(i) = self.attendees.iter().position(|a| me.iter().any(|m| bare(m) == bare(&a.address))) { return Some(i); }
        let asked: Vec<usize> = (0..self.attendees.len()).filter(|&i| self.attendees[i].rsvp || self.attendees[i].partstat == PartStat::NeedsAction).collect();
        (asked.len() == 1).then(|| asked[0])
    }
    /// Answers an invitation: sets my PARTSTAT and marks the event changed. Returns
    /// whether there was an attendee to answer for.
    pub fn respond(&mut self, me: &[&str], answer: PartStat) -> bool {
        let Some(i) = self.my_attendee(me) else { return false };
        self.attendees[i].partstat = answer;
        self.attendees[i].rsvp = false;
        self.invitation = false;
        self.touch();
        true
    }
    /// Marks a change made here, for the next sync.
    pub fn touch(&mut self) {
        if self.state == SyncState::Clean && !self.href.is_empty() { self.state = SyncState::Modified; }
    }
    /// The start as minutes since 1970-01-01 00:00 UTC, given the local zone's offset
    /// for floating times. An all-day event starts at local midnight.
    pub fn start_minute_utc(&self, day: Date, local_offset: i32) -> i64 {
        let m = day.days() * 1440 + self.time.map_or(0, |t| t.0) as i64;
        if self.utc { m } else { m - local_offset as i64 }
    }
}

/// One calendar of several: a name, a colour, whether it is shown, and for a CalDAV
/// calendar its collection's address and the server's change markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarInfo {
    pub id: u16,
    pub name: String,
    /// 0xRRGGBB.
    pub color: u32,
    pub visible: bool,
    pub url: String,
    pub ctag: String,
    pub sync_token: String,
}

/// Colours given to calendars in turn.
pub const PALETTE: [u32; 8] = [0x7FB8FF, 0xF2A65A, 0x8BD17C, 0xE57BA5, 0xC59BF2, 0x6AD6D0, 0xF2D35A, 0xB0B6BE];

/// The program's events. Saved as text, one event a line, tab-separated:
///
/// ```text
/// # sex-calendar 1
/// !cal	1	Work	F2A65A	1	http://dav.example/cal/work/	ctag	token
/// 2026-03-16	09:00-10:00	Z	1	FREQ=WEEKLY;COUNT=4	Team sync	Room 4	uid=a@b	cal=1
/// ```
///
/// Event fields: first day, time or `-` for all day, `Z` or `-`, days covered, rule or
/// `-`, title, location, then any number of `key=value` fields (uid, cal, ex, rdate,
/// rid, alarm, desc, org, att, inv, seq, href, etag, state). Tabs, newlines and
/// backslashes in text are escaped as `\t` `\n` `\\`. A `!cal` line is one calendar:
/// id, name, colour, shown, collection address, ctag, sync token.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calendar { pub events: Vec<Event>, pub calendars: Vec<CalendarInfo> }

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
fn dates(text: &str) -> Vec<Date> { text.split(',').filter_map(Date::parse).collect() }
fn put_dates(out: &mut String, key: &str, list: &[Date]) {
    if list.is_empty() { return; }
    let _ = write!(out, "\t{key}=");
    for (i, d) in list.iter().enumerate() { if i > 0 { out.push(','); } let _ = write!(out, "{d}"); }
}
fn put_text(out: &mut String, key: &str, value: &str) {
    if value.is_empty() { return; }
    let _ = write!(out, "\t{key}=");
    escape(out, value);
}

impl Calendar {
    /// Reads the event file. Lines it cannot read are skipped, so one damaged line
    /// costs that event and not the calendar.
    pub fn parse(text: &str) -> Calendar {
        let mut events = Vec::new();
        let mut calendars = Vec::new();
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() { continue; }
            let f: Vec<&str> = line.split('\t').collect();
            if f[0] == "!cal" {
                if f.len() < 8 { continue; }
                let (Ok(id), Ok(color)) = (f[1].parse(), u32::from_str_radix(f[3], 16)) else { continue };
                calendars.push(CalendarInfo { id, name: unescape(f[2]), color, visible: f[4] != "0", url: unescape(f[5]), ctag: unescape(f[6]), sync_token: unescape(f[7]) });
                continue;
            }
            if f.len() < 7 { continue; }
            let Some(start) = Date::parse(f[0]) else { continue };
            let time = if f[1] == "-" { None } else {
                let Some((a, b)) = f[1].split_once('-') else { continue };
                let (Some(a), Some(b)) = (minutes(a), minutes(b)) else { continue };
                Some((a, b))
            };
            let Ok(span) = f[3].parse::<u32>() else { continue };
            let rule = if f[4] == "-" { None } else { Rule::parse(f[4]) };
            let mut e = Event { start, time, utc: f[2] == "Z", span: span.clamp(1, 3660), title: unescape(f[5]), location: unescape(f[6]), rule, ..Event::default() };
            for field in &f[7..] {
                let Some((key, value)) = field.split_once('=') else { continue };
                match key {
                    "uid" => e.uid = unescape(value),
                    "cal" => e.calendar = value.parse().unwrap_or(0),
                    "ex" => e.exdates = dates(value),
                    "rdate" => e.rdates = dates(value),
                    "rid" => e.recurrence_id = Date::parse(value),
                    "alarm" => e.alarms = value.split(',').filter_map(|a| a.parse().ok()).collect(),
                    "desc" => e.description = unescape(value),
                    "org" => e.organizer = unescape(value),
                    "att" => {
                        let mut p = value.splitn(4, ',');
                        let (Some(ps), Some(rsvp), Some(addr)) = (p.next(), p.next(), p.next()) else { continue };
                        e.attendees.push(Attendee { partstat: PartStat::parse(ps), rsvp: rsvp == "1", address: unescape(addr), name: unescape(p.next().unwrap_or("")) });
                    }
                    "inv" => e.invitation = value == "1",
                    "seq" => e.sequence = value.parse().unwrap_or(0),
                    "href" => e.href = unescape(value),
                    "etag" => e.etag = unescape(value),
                    "state" => e.state = match value { "new" => SyncState::New, "mod" => SyncState::Modified, "del" => SyncState::Deleted, _ => SyncState::Clean },
                    _ => {}
                }
            }
            events.push(e);
        }
        Calendar { events, calendars }
    }
    pub fn serialize(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        for c in &self.calendars {
            let _ = write!(out, "!cal\t{}\t", c.id);
            escape(&mut out, &c.name);
            let _ = write!(out, "\t{:06X}\t{}\t", c.color, c.visible as u8);
            escape(&mut out, &c.url);
            out.push('\t');
            escape(&mut out, &c.ctag);
            out.push('\t');
            escape(&mut out, &c.sync_token);
            out.push('\n');
        }
        for e in &self.events {
            let _ = write!(out, "{}\t", e.start);
            match e.time {
                Some((a, b)) => { let _ = write!(out, "{:02}:{:02}-{:02}:{:02}\t", a / 60, a % 60, b / 60, b % 60); }
                None => out.push_str("-\t"),
            }
            out.push_str(if e.utc { "Z\t" } else { "-\t" });
            let _ = write!(out, "{}\t", e.span);
            match &e.rule { Some(r) => { let _ = write!(out, "{r}\t"); } None => out.push_str("-\t") }
            escape(&mut out, &e.title);
            out.push('\t');
            escape(&mut out, &e.location);
            put_text(&mut out, "uid", &e.uid);
            if e.calendar != 0 { let _ = write!(out, "\tcal={}", e.calendar); }
            put_dates(&mut out, "ex", &e.exdates);
            put_dates(&mut out, "rdate", &e.rdates);
            if let Some(r) = e.recurrence_id { let _ = write!(out, "\trid={r}"); }
            if !e.alarms.is_empty() {
                out.push_str("\talarm=");
                for (i, a) in e.alarms.iter().enumerate() { if i > 0 { out.push(','); } let _ = write!(out, "{a}"); }
            }
            put_text(&mut out, "desc", &e.description);
            put_text(&mut out, "org", &e.organizer);
            for a in &e.attendees {
                let _ = write!(out, "\tatt={},{},", a.partstat.as_str(), a.rsvp as u8);
                escape(&mut out, &a.address.replace(',', ""));
                out.push(',');
                escape(&mut out, &a.name);
            }
            if e.invitation { out.push_str("\tinv=1"); }
            if e.sequence != 0 { let _ = write!(out, "\tseq={}", e.sequence); }
            put_text(&mut out, "href", &e.href);
            put_text(&mut out, "etag", &e.etag);
            match e.state { SyncState::Clean => {}, SyncState::New => out.push_str("\tstate=new"), SyncState::Modified => out.push_str("\tstate=mod"), SyncState::Deleted => out.push_str("\tstate=del") }
            out.push('\n');
        }
        out
    }
    /// The calendar `id`, or a stand-in for the first one when none are written yet.
    pub fn info(&self, id: u16) -> CalendarInfo {
        self.calendars.iter().find(|c| c.id == id).cloned().unwrap_or_else(|| CalendarInfo {
            id, name: if id == 0 { "Personal".into() } else { alloc::format!("Calendar {id}") }, color: PALETTE[id as usize % PALETTE.len()],
            visible: true, url: String::new(), ctag: String::new(), sync_token: String::new(),
        })
    }
    /// Every calendar to list: the written ones, and the first even when not written.
    pub fn all_calendars(&self) -> Vec<CalendarInfo> {
        let mut all = self.calendars.clone();
        if !all.iter().any(|c| c.id == 0) { all.insert(0, self.info(0)); }
        all
    }
    /// Adds a calendar (or returns the one with that collection address) and its id.
    pub fn add_calendar(&mut self, name: &str, url: &str) -> u16 {
        if !url.is_empty() { if let Some(c) = self.calendars.iter().find(|c| c.url == url) { return c.id; } }
        if !self.calendars.iter().any(|c| c.id == 0) { let first = self.info(0); self.calendars.push(first); }
        let id = self.calendars.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        self.calendars.push(CalendarInfo { id, name: name.into(), color: PALETTE[id as usize % PALETTE.len()], visible: true, url: url.into(), ctag: String::new(), sync_token: String::new() });
        id
    }
    pub fn info_mut(&mut self, id: u16) -> &mut CalendarInfo {
        if !self.calendars.iter().any(|c| c.id == id) { let c = self.info(id); self.calendars.push(c); }
        self.calendars.iter_mut().find(|c| c.id == id).unwrap()
    }
    /// Shows or hides a calendar; returns whether it is now shown.
    pub fn toggle(&mut self, id: u16) -> bool {
        let c = self.info_mut(id);
        c.visible = !c.visible;
        c.visible
    }
    /// Whether an event is shown: its calendar is, and it is not waiting to be deleted.
    pub fn shown(&self, e: &Event) -> bool {
        e.state != SyncState::Deleted && self.calendars.iter().find(|c| c.id == e.calendar).map_or(true, |c| c.visible)
    }
    /// The events on `day`: all-day ones first, then by start time, then by title.
    pub fn on_day(&self, day: Date) -> Vec<&Event> {
        let mut found: Vec<&Event> = self.events.iter().filter(|e| self.shown(e) && e.occurs_on(day)).collect();
        found.sort_by(|a, b| (a.time.map(|t| t.0), &a.title).cmp(&(b.time.map(|t| t.0), &b.title)));
        found
    }
    /// How many events fall on each cell of `grid`, in one pass per event.
    pub fn counts(&self, grid: &MonthGrid) -> [u16; MonthGrid::CELLS] {
        let mut counts = [0u16; MonthGrid::CELLS];
        for e in self.events.iter().filter(|e| self.shown(e)) {
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
    /// Adds events not already present (the same UID and occurrence, or without UIDs
    /// the same day, time, title and rule), so the same file imported twice does not
    /// double the calendar. Returns how many were new.
    pub fn merge(&mut self, events: Vec<Event>) -> usize {
        let mut added = 0;
        for e in events {
            let same = |o: &Event| if !e.uid.is_empty() && !o.uid.is_empty() { o.uid == e.uid && o.recurrence_id == e.recurrence_id }
                else { o.start == e.start && o.time == e.time && o.title == e.title && o.rule == e.rule };
            if !self.events.iter().any(same) {
                self.events.push(e);
                added += 1;
            }
        }
        added
    }
    /// Reminders that fall due in `(after, upto]`, both minutes since 1970 UTC: the
    /// event's index, the occurrence's day and the minute it is due. Floating times are
    /// read in the zone `local_offset` minutes east of UTC.
    pub fn reminders_due(&self, after: i64, upto: i64, local_offset: i32) -> Vec<(usize, Date, i64)> {
        let mut due = Vec::new();
        for (i, e) in self.events.iter().enumerate() {
            if e.alarms.is_empty() || !self.shown(e) { continue; }
            let lead = e.alarms.iter().map(|a| *a as i64).max().unwrap_or(0).max(0);
            let lag = e.alarms.iter().map(|a| -(*a as i64)).max().unwrap_or(0).max(0);
            let from = Date::from_days((after - lag - 1440).div_euclid(1440));
            let to = Date::from_days((upto + lead + 1440).div_euclid(1440));
            e.starts_between(from, to, |day| {
                let start = e.start_minute_utc(day, local_offset);
                for a in &e.alarms {
                    let at = start - *a as i64;
                    if at > after && at <= upto { due.push((i, day, at)); }
                }
            });
        }
        due.sort_by_key(|d| d.2);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;
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
            rule: Rule::parse("FREQ=WEEKLY;INTERVAL=2;COUNT=3"), uid: "u1@x".into(), calendar: 2, exdates: vec![d(2026, 3, 16)], alarms: vec![15, -5],
            attendees: vec![Attendee { address: "mailto:a@b".into(), name: "A, B".into(), partstat: PartStat::Tentative, rsvp: true }],
            organizer: "mailto:o@b".into(), invitation: true, href: "/c/u1.ics".into(), etag: "\"7\"".into(), state: SyncState::Modified, ..Event::default() });
        c.add_calendar("Work", "http://h/cal/work/");
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
