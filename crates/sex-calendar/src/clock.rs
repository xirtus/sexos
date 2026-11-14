// SPDX-License-Identifier: GPL-3.0-only
//! Date's clock: alarms at a time of day (once, or on chosen weekdays), countdown
//! timers, the world clock's list of zones and the local zone. Kept in `clock.txt`:
//!
//! ```text
//! # sex-clock 1
//! home	Europe/Berlin
//! zone	Asia/Tokyo
//! alarm	07:30	1111100	1	Wake up
//! timer	1793836800000	300	Tea
//! ```
//!
//! An alarm's days are seven `0`/`1` from Monday (all `0`: once, at the next such
//! time); a timer is when it ends (Unix ms), its length in seconds and its label.
use crate::tz::{self, Zone};
use crate::Date;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alarm {
    pub minute: u16,
    /// Bit 0 is Monday; none set rings once.
    pub days: u8,
    pub enabled: bool,
    pub label: String,
}

impl Alarm {
    /// The first moment after `after` and up to `upto` (minutes since 1970 UTC) that the
    /// alarm rings, in a zone `offset` minutes east of UTC.
    pub fn due(&self, after: i64, upto: i64, offset: i32) -> Option<i64> {
        if !self.enabled || upto <= after { return None; }
        let first = (after + offset as i64).div_euclid(1440);
        let last = (upto + offset as i64).div_euclid(1440);
        (first..=last).map(|d| d * 1440 + self.minute as i64 - offset as i64).find(|&at| {
            let wd = Date::from_days((at + offset as i64).div_euclid(1440)).weekday();
            at > after && at <= upto && (self.days == 0 || self.days & (1 << wd) != 0)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timer { pub ends_ms: u64, pub seconds: u32, pub label: String }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// The local zone's tz name; empty for UTC.
    pub home: String,
    pub zones: Vec<String>,
    pub alarms: Vec<Alarm>,
    pub timers: Vec<Timer>,
}

pub const HEADER: &str = "# sex-clock 1";

/// `07:30` or `7:30`.
pub fn parse_time(text: &str) -> Option<u16> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// A timer's length: `90` or `90s`, `5m`, `1h30m`, `2m30s`, `01:30` (minutes and
/// seconds). None for nothing or more than a day.
pub fn parse_duration(text: &str) -> Option<u32> {
    let t = text.trim();
    if let Some((m, s)) = t.split_once(':') {
        let (m, s): (u32, u32) = (m.parse().ok()?, s.parse().ok()?);
        return (s < 60 && m * 60 + s > 0 && m * 60 + s <= 86_400).then_some(m * 60 + s);
    }
    let mut total = 0u32;
    let mut num = String::new();
    for c in t.chars() {
        if c.is_ascii_digit() { num.push(c); continue; }
        let n: u32 = num.parse().ok()?;
        num.clear();
        total += n * match c.to_ascii_lowercase() { 'h' => 3600, 'm' => 60, 's' => 1, _ => return None };
    }
    if !num.is_empty() { total += num.parse::<u32>().ok()?; }
    (total > 0 && total <= 86_400).then_some(total)
}

/// `5m` for 300, `1h30m`, `45s`, `2m05s`.
pub fn format_duration(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    let mut out = String::new();
    if h > 0 { let _ = write!(out, "{h}h"); }
    if m > 0 { let _ = write!(out, "{m}m"); }
    if s > 0 || out.is_empty() { let _ = write!(out, "{s:02}s"); }
    out
}

impl Clock {
    pub fn parse(text: &str) -> Clock {
        let mut c = Clock::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f[0] {
                "home" if f.len() > 1 => c.home = f[1].into(),
                "zone" if f.len() > 1 && tz::find(f[1]).is_some() => c.zones.push(f[1].into()),
                "alarm" if f.len() > 4 => {
                    let (Some(minute), Ok(days)) = (parse_time(f[1]), u8::from_str_radix(&f[2].chars().rev().collect::<String>(), 2)) else { continue };
                    c.alarms.push(Alarm { minute, days, enabled: f[3] != "0", label: f[4].into() });
                }
                "timer" if f.len() > 3 => {
                    let (Ok(ends_ms), Ok(seconds)) = (f[1].parse(), f[2].parse()) else { continue };
                    c.timers.push(Timer { ends_ms, seconds, label: f[3].into() });
                }
                _ => {}
            }
        }
        c
    }
    pub fn serialize(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        let mut out = String::from(HEADER);
        out.push('\n');
        if !self.home.is_empty() { let _ = writeln!(out, "home\t{}", self.home); }
        for z in &self.zones { let _ = writeln!(out, "zone\t{z}"); }
        for a in &self.alarms {
            let days: String = (0..7).map(|i| if a.days & (1 << i) != 0 { '1' } else { '0' }).collect();
            let _ = writeln!(out, "alarm\t{:02}:{:02}\t{days}\t{}\t{}", a.minute / 60, a.minute % 60, a.enabled as u8, clean(&a.label));
        }
        for t in &self.timers { let _ = writeln!(out, "timer\t{}\t{}\t{}", t.ends_ms, t.seconds, clean(&t.label)); }
        out
    }
    /// The local zone (UTC when none is set or the name is unknown).
    pub fn home_zone(&self) -> &'static Zone { tz::find(&self.home).unwrap_or(&tz::ZONES[0]) }
    /// The zones the world clock shows: the ones chosen, or a few to start with.
    pub fn world(&self) -> Vec<&'static Zone> {
        if self.zones.is_empty() {
            return ["America/Los_Angeles", "America/New_York", "Europe/London", "Europe/Berlin", "Asia/Kolkata", "Asia/Tokyo", "Australia/Sydney"].iter().filter_map(|n| tz::find(n)).collect();
        }
        self.zones.iter().filter_map(|n| tz::find(n)).collect()
    }
    /// Alarms ringing in `(after, upto]` (minutes since 1970 UTC); an alarm that rings
    /// once is switched off when it does.
    pub fn alarms_due(&mut self, after: i64, upto: i64) -> Vec<Alarm> {
        let zone = self.home_zone();
        let mut rung = Vec::new();
        for a in &mut self.alarms {
            if a.due(after, upto, zone.offset(upto)).is_some() {
                rung.push(a.clone());
                if a.days == 0 { a.enabled = false; }
            }
        }
        rung
    }
    /// Timers ended by `now_ms`, taken off the list.
    pub fn timers_done(&mut self, now_ms: u64) -> Vec<Timer> {
        let (done, left): (Vec<Timer>, Vec<Timer>) = self.timers.drain(..).partition(|t| t.ends_ms <= now_ms);
        self.timers = left;
        done
    }
}
