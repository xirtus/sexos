// SPDX-License-Identifier: GPL-3.0-only
//! iCalendar (RFC 5545) import: the VEVENTs of a `.ics` file, with DTSTART and DTEND
//! as dates or date-times (UTC `Z` or floating), SUMMARY, LOCATION and RRULE. Lines are
//! unfolded first (a line starting with a space or tab continues the one before) and
//! text values unescaped (`\n`, `\,`, `\;`, `\\`). Anything else in the file, other
//! components included, is skipped, so a calendar from any program imports what it can.
use crate::{Date, Event, Rule};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The text has no BEGIN:VCALENDAR: not an iCalendar file.
    NotCalendar,
}

/// A DATE or DATE-TIME value: the day, the minute of that day if a time was given, and
/// whether that time is UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct When { pub date: Date, pub minute: Option<u16>, pub utc: bool }

/// `20260316`, `20260316T090000` or `20260316T090000Z`.
pub fn parse_when(text: &str) -> Option<When> {
    let b = text.as_bytes();
    if b.len() < 8 || !b[..8].iter().all(u8::is_ascii_digit) { return None; }
    let date = Date::new(text[..4].parse().ok()?, text[4..6].parse().ok()?, text[6..8].parse().ok()?)?;
    if b.len() == 8 { return Some(When { date, minute: None, utc: false }); }
    if b[8] != b'T' || b.len() < 15 || !b[9..15].iter().all(u8::is_ascii_digit) { return None; }
    let (h, m): (u16, u16) = (text[9..11].parse().ok()?, text[11..13].parse().ok()?);
    if h > 23 || m > 59 { return None; }
    let utc = match &b[15..] { b"" => false, b"Z" => true, _ => return None };
    Some(When { date, minute: Some(h * 60 + m), utc })
}

/// Joins folded lines: CRLF (or LF) followed by one space or tab is not a line break.
pub fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match (line.as_bytes().first(), lines.last_mut()) {
            (Some(b' ' | b'\t'), Some(last)) => last.push_str(&line[1..]),
            _ if line.is_empty() => {}
            _ => lines.push(line.into()),
        }
    }
    lines
}

/// A TEXT value as written, escapes resolved.
pub fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() { Some('n' | 'N') => out.push('\n'), Some(c) => out.push(c), None => {} }
    }
    out
}

/// One content line: its name (upper case), its parameters and its value. The value
/// starts at the first colon outside a quoted parameter value.
pub fn split_line(line: &str) -> Option<(String, Vec<(String, String)>, &str)> {
    let mut quoted = false;
    let colon = line.char_indices().find(|&(_, c)| { if c == '"' { quoted = !quoted; } c == ':' && !quoted })?.0;
    let mut parts = line[..colon].split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts.filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_uppercase(), String::from(v.trim_matches('"')))).collect();
    Some((name, params, &line[colon + 1..]))
}

/// The events of an `.ics` file, in file order. An event without a usable DTSTART is
/// skipped, as is one left open at the end of the file.
pub fn import(text: &str) -> Result<Vec<Event>, Error> {
    let lines = unfold(text);
    if !lines.iter().any(|l| l.trim().eq_ignore_ascii_case("BEGIN:VCALENDAR")) { return Err(Error::NotCalendar); }
    let mut events = Vec::new();
    // Inside a VEVENT: its fields so far, and how deep in nested components (VALARM).
    let mut current: Option<(Option<When>, Option<When>, String, String, Option<Rule>)> = None;
    let mut nested = 0;
    for line in &lines {
        let Some((name, _params, value)) = split_line(line) else { continue };
        let value_upper = value.trim().to_ascii_uppercase();
        match (name.as_str(), current.as_mut()) {
            ("BEGIN", None) if value_upper == "VEVENT" => current = Some((None, None, String::new(), String::new(), None)),
            ("BEGIN", Some(_)) => nested += 1,
            ("END", Some(_)) if nested > 0 => nested -= 1,
            ("END", Some(_)) if value_upper == "VEVENT" => {
                let (start, end, title, location, rule) = current.take().unwrap();
                if let Some(e) = start.map(|s| event(s, end, title, location, rule)) { events.push(e); }
            }
            (_, Some(_)) if nested > 0 => {}
            ("DTSTART", Some(c)) => c.0 = parse_when(value.trim()),
            ("DTEND", Some(c)) => c.1 = parse_when(value.trim()),
            ("SUMMARY", Some(c)) => c.2 = unescape(value),
            ("LOCATION", Some(c)) => c.3 = unescape(value),
            ("RRULE", Some(c)) => c.4 = Rule::parse(value),
            _ => {}
        }
    }
    Ok(events)
}

fn event(start: When, end: Option<When>, title: String, location: String, rule: Option<Rule>) -> Event {
    let first = start.date.days();
    // A DATE end is exclusive (an all-day event on the 15th ends on the 16th); a
    // DATE-TIME end covers its own day unless it is exactly midnight.
    let span = match end {
        Some(e) if e.minute.is_none() || e.minute == Some(0) => e.date.days() - first,
        Some(e) => e.date.days() - first + 1,
        None => 1,
    };
    let time = start.minute.map(|a| (a, end.and_then(|e| e.minute).unwrap_or(a)));
    let title = if title.is_empty() { String::from("(untitled)") } else { title };
    Event { start: start.date, time, utc: start.utc, span: span.clamp(1, 3660) as u32, title, location, rule }
}
