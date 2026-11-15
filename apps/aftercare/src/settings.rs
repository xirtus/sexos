// SPDX-License-Identifier: GPL-3.0-only
//! The job's settings, kept in `aftercare.conf` in the program's own folder:
//!
//! ```text
//! source=@/home/ann
//! destination=@/media/USB/aftercare
//! keep=last=10 daily=7 weekly=4 monthly=12
//! schedule=21:30
//! last-scheduled-day=20406
//! ```
//!
//! Times are UTC: SexOS has no time zone setting yet. `schedule=off` turns the daily
//! backup off.
use aftercare_core::KeepPolicy;

pub const FILE: &str = "aftercare.conf";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The folder backed up, as this run names a granted folder (`@/...`).
    pub source: Option<String>,
    /// The repository folder.
    pub destination: Option<String>,
    pub keep: KeepPolicy,
    /// Minutes after midnight (UTC) for the daily backup.
    pub schedule: Option<u32>,
    /// The day (days since 1970-01-01) the scheduled backup last ran.
    pub last_scheduled_day: Option<i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            source: None,
            destination: None,
            keep: KeepPolicy { last: 10, daily: 7, weekly: 4, monthly: 12, ..Default::default() },
            schedule: None,
            last_scheduled_day: None,
        }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "source" if !v.is_empty() => s.source = Some(v.into()),
                "destination" if !v.is_empty() => s.destination = Some(v.into()),
                "keep" => {
                    if let Ok(p) = parse_keep(v) {
                        s.keep = p
                    }
                }
                "schedule" => s.schedule = parse_time(v).ok().flatten(),
                "last-scheduled-day" => s.last_scheduled_day = v.parse().ok(),
                _ => {}
            }
        }
        s
    }

    pub fn format(&self) -> String {
        let mut out = String::new();
        if let Some(p) = &self.source {
            out += &format!("source={p}\n");
        }
        if let Some(p) = &self.destination {
            out += &format!("destination={p}\n");
        }
        out += &format!("keep={}\n", format_keep(&self.keep));
        out += &format!("schedule={}\n", format_time(self.schedule));
        if let Some(d) = self.last_scheduled_day {
            out += &format!("last-scheduled-day={d}\n");
        }
        out
    }

    /// Whether the daily backup should start now (`now` in seconds since 1970, UTC):
    /// within the hour after its time, and not yet today. The app checks this while it
    /// is open; a background scheduler that starts it is a later slice.
    pub fn due(&self, now: i64) -> bool {
        let Some(at) = self.schedule else { return false };
        let (day, minute) = (now.div_euclid(86_400), (now.rem_euclid(86_400) / 60) as u32);
        minute >= at && minute < at + 60 && self.last_scheduled_day != Some(day)
    }
}

/// `last=7 daily=7 weekly=4 monthly=12 yearly=2 hourly=0`; words may be left out.
pub fn parse_keep(text: &str) -> Result<KeepPolicy, String> {
    let mut p = KeepPolicy::default();
    for word in text.split_whitespace() {
        let (k, v) = word.split_once('=').ok_or_else(|| format!("'{word}' is not NAME=NUMBER"))?;
        let n: u32 = v.parse().map_err(|_| format!("'{v}' is not a number"))?;
        match k {
            "last" => p.last = n,
            "hourly" => p.hourly = n,
            "daily" => p.daily = n,
            "weekly" => p.weekly = n,
            "monthly" => p.monthly = n,
            "yearly" => p.yearly = n,
            _ => return Err(format!("unknown period '{k}' (last hourly daily weekly monthly yearly)")),
        }
    }
    if p == KeepPolicy::default() {
        return Err("keep at least one snapshot (for example last=1)".into());
    }
    Ok(p)
}

pub fn format_keep(p: &KeepPolicy) -> String {
    let parts = [("last", p.last), ("hourly", p.hourly), ("daily", p.daily), ("weekly", p.weekly), ("monthly", p.monthly), ("yearly", p.yearly)];
    parts.iter().filter(|(_, n)| *n > 0).map(|(k, n)| format!("{k}={n}")).collect::<Vec<_>>().join(" ")
}

/// `HH:MM` (24-hour) or `off`.
pub fn parse_time(text: &str) -> Result<Option<u32>, String> {
    let t = text.trim();
    if t.is_empty() || t == "off" {
        return Ok(None);
    }
    let bad = || format!("'{t}' is not HH:MM or off");
    let (h, m) = t.split_once(':').ok_or_else(bad)?;
    let (h, m): (u32, u32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(Some(h * 60 + m))
}

pub fn format_time(t: Option<u32>) -> String {
    match t {
        Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
        None => "off".into(),
    }
}

/// `YYYY-MM-DD HH:MM` (UTC) for seconds since 1970.
pub fn civil(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's days_from_civil, inverted.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

/// `12 B`, `3.4 KB`, `5.0 MB`, `1.2 GB`.
pub fn bytes(n: u64) -> String {
    const U: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1000.0;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            source: Some("@/data/src".into()),
            destination: Some("@/media/USB/backup".into()),
            keep: parse_keep("last=3 weekly=2").unwrap(),
            schedule: Some(21 * 60 + 5),
            last_scheduled_day: Some(20_406),
        };
        let text = s.format();
        assert_eq!(text, "source=@/data/src\ndestination=@/media/USB/backup\nkeep=last=3 weekly=2\nschedule=21:05\nlast-scheduled-day=20406\n");
        assert_eq!(Settings::parse(&text), s);
        assert_eq!(Settings::parse(""), Settings::default());
    }

    #[test]
    fn keep_and_time_errors() {
        assert!(parse_keep("").is_err());
        assert!(parse_keep("last=0").is_err());
        assert!(parse_keep("daily=x").is_err());
        assert!(parse_keep("fortnightly=2").is_err());
        assert_eq!(parse_time("off"), Ok(None));
        assert_eq!(parse_time("07:30"), Ok(Some(450)));
        assert!(parse_time("24:00").is_err());
        assert!(parse_time("7").is_err());
    }

    #[test]
    fn due_once_a_day_within_the_hour() {
        let mut s = Settings { schedule: Some(9 * 60), ..Default::default() };
        let day = 20_406i64;
        let at = |h: i64, m: i64| day * 86_400 + h * 3600 + m * 60;
        assert!(!s.due(at(8, 59)));
        assert!(s.due(at(9, 0)));
        assert!(s.due(at(9, 59)));
        assert!(!s.due(at(10, 0)));
        s.last_scheduled_day = Some(day);
        assert!(!s.due(at(9, 30)));
        assert!(s.due(at(9, 30) + 86_400));
        s.schedule = None;
        assert!(!s.due(at(9, 30) + 86_400));
    }

    #[test]
    fn dates_and_sizes() {
        assert_eq!(civil(0), "1970-01-01 00:00");
        assert_eq!(civil(1_763_115_000), "2025-11-14 10:10");
        assert_eq!(civil(951_782_400), "2000-02-29 00:00");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(3_400), "3.4 KB");
        assert_eq!(bytes(5_000_000), "5.0 MB");
    }
}
