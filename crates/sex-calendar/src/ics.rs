// SPDX-License-Identifier: GPL-3.0-only
//! iCalendar (RFC 5545) import and export.
//!
//! Import reads the VEVENTs of a `.ics` file: DTSTART and DTEND as dates or date-times
//! (UTC `Z` or floating), SUMMARY, LOCATION, DESCRIPTION, UID, RRULE, EXDATE, RDATE,
//! RECURRENCE-ID, SEQUENCE, ORGANIZER, ATTENDEE (with PARTSTAT, RSVP and CN), the
//! TRIGGER of each VALARM, and the calendar's METHOD (a REQUEST is an invitation).
//! Lines are unfolded first (a line starting with a space or tab continues the one
//! before) and text values unescaped (`\n`, `\,`, `\;`, `\\`). Anything else in the
//! file, other components included, is skipped, so a calendar from any program
//! imports what it can.
//!
//! Export writes the same fields back, folded at 75 octets, so an event survives the
//! trip to a CalDAV server and back; [`reply`] writes the METHOD:REPLY answer to an
//! invitation for the mail program to send.
use crate::{Attendee, Date, Event, PartStat, Rule};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

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

/// A DURATION (`-PT15M`, `P1D`, `-P1DT2H`, `PT0S`, `P2W`) in whole minutes, rounded
/// towards zero.
pub fn parse_duration(text: &str) -> Option<i64> {
    let t = text.trim();
    let (sign, t) = match t.as_bytes().first()? { b'-' => (-1, &t[1..]), b'+' => (1, &t[1..]), _ => (1, t) };
    let t = t.strip_prefix('P').or_else(|| t.strip_prefix('p'))?;
    let mut seconds = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    let mut any = false;
    for c in t.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => num.push(c),
            'T' => in_time = true,
            u => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                any = true;
                seconds += n * match (u, in_time) { ('W', false) => 604_800, ('D', false) => 86_400, ('H', true) => 3600, ('M', true) => 60, ('S', true) => 1, _ => return None };
            }
        }
    }
    (any && num.is_empty()).then_some(sign * seconds / 60)
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

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// The fields of a VEVENT as they are read.
#[derive(Default)]
struct Draft { start: Option<When>, end: Option<When>, duration: Option<i64>, event: Event }

/// The events of an `.ics` file, in file order. An event without a usable DTSTART is
/// skipped, as is one left open at the end of the file.
pub fn import(text: &str) -> Result<Vec<Event>, Error> {
    let lines = unfold(text);
    if !lines.iter().any(|l| l.trim().eq_ignore_ascii_case("BEGIN:VCALENDAR")) { return Err(Error::NotCalendar); }
    let mut events = Vec::new();
    let mut current: Option<Draft> = None;
    // How deep in nested components (VALARM), and whether the innermost is a VALARM.
    let mut nested = 0;
    let mut in_alarm = false;
    let mut request = false;
    for line in &lines {
        let Some((name, params, value)) = split_line(line) else { continue };
        let value_upper = value.trim().to_ascii_uppercase();
        match (name.as_str(), current.as_mut()) {
            ("METHOD", None) => request = value_upper == "REQUEST",
            ("BEGIN", None) if value_upper == "VEVENT" => current = Some(Draft::default()),
            ("BEGIN", Some(_)) => { nested += 1; in_alarm = nested == 1 && value_upper == "VALARM"; }
            ("END", Some(_)) if nested > 0 => { nested -= 1; in_alarm = false; }
            ("END", Some(_)) if value_upper == "VEVENT" => {
                let d = current.take().unwrap();
                if let Some(e) = finish(d) { events.push(e); }
            }
            ("TRIGGER", Some(c)) if in_alarm => {
                // Relative to the start, or (VALUE=DATE-TIME) a moment of its own.
                let minutes = match param(&params, "VALUE") {
                    Some(v) if v.eq_ignore_ascii_case("DATE-TIME") => parse_when(value.trim()).zip(c.start).map(|(at, s)| {
                        (s.date.days() * 1440 + s.minute.unwrap_or(0) as i64) - (at.date.days() * 1440 + at.minute.unwrap_or(0) as i64)
                    }),
                    _ => parse_duration(value).map(|m| -m),
                };
                if let Some(m) = minutes { if param(&params, "RELATED").map_or(true, |r| !r.eq_ignore_ascii_case("END")) { c.event.alarms.push(m.clamp(-100_000, 100_000) as i32); } }
            }
            (_, Some(_)) if nested > 0 => {}
            ("DTSTART", Some(c)) => c.start = parse_when(value.trim()),
            ("DTEND", Some(c)) => c.end = parse_when(value.trim()),
            ("DURATION", Some(c)) => c.duration = parse_duration(value),
            ("SUMMARY", Some(c)) => c.event.title = unescape(value),
            ("LOCATION", Some(c)) => c.event.location = unescape(value),
            ("DESCRIPTION", Some(c)) => c.event.description = unescape(value),
            ("UID", Some(c)) => c.event.uid = String::from(value.trim()),
            ("RRULE", Some(c)) => c.event.rule = Rule::parse(value),
            ("EXDATE", Some(c)) => c.event.exdates.extend(value.split(',').filter_map(|v| parse_when(v.trim())).map(|w| w.date)),
            ("RDATE", Some(c)) => c.event.rdates.extend(value.split(',').filter_map(|v| parse_when(v.trim())).map(|w| w.date)),
            ("RECURRENCE-ID", Some(c)) => c.event.recurrence_id = parse_when(value.trim()).map(|w| w.date),
            ("SEQUENCE", Some(c)) => c.event.sequence = value.trim().parse().unwrap_or(0),
            ("ORGANIZER", Some(c)) => c.event.organizer = String::from(value.trim()),
            ("ATTENDEE", Some(c)) => c.event.attendees.push(Attendee {
                address: String::from(value.trim()),
                name: String::from(param(&params, "CN").unwrap_or("")),
                partstat: PartStat::parse(param(&params, "PARTSTAT").unwrap_or("NEEDS-ACTION")),
                rsvp: param(&params, "RSVP").is_some_and(|r| r.eq_ignore_ascii_case("TRUE")),
            }),
            _ => {}
        }
    }
    if request {
        for e in &mut events { e.invitation = e.attendees.iter().any(|a| a.partstat == PartStat::NeedsAction); }
    }
    // An occurrence moved or changed elsewhere in the file is not also shown in place.
    let moved: Vec<(String, Date)> = events.iter().filter_map(|e| e.recurrence_id.map(|r| (e.uid.clone(), r))).collect();
    for e in events.iter_mut().filter(|e| e.recurrence_id.is_none()) {
        for (uid, day) in &moved { if *uid == e.uid && !e.exdates.contains(day) { e.exdates.push(*day); } }
    }
    Ok(events)
}

fn finish(d: Draft) -> Option<Event> {
    let start = d.start?;
    let mut e = d.event;
    let first = start.date.days();
    // A DATE end is exclusive (an all-day event on the 15th ends on the 16th); a
    // DATE-TIME end covers its own day unless it is exactly midnight.
    let end = d.end.or_else(|| d.duration.map(|m| {
        let at = first * 1440 + start.minute.unwrap_or(0) as i64 + m.max(0);
        When { date: Date::from_days(at.div_euclid(1440)), minute: start.minute.map(|_| at.rem_euclid(1440) as u16), utc: start.utc }
    }));
    let span = match end {
        Some(e) if e.minute.is_none() || e.minute == Some(0) => e.date.days() - first,
        Some(e) => e.date.days() - first + 1,
        None => 1,
    };
    e.start = start.date;
    e.time = start.minute.map(|a| (a, end.and_then(|e| e.minute).unwrap_or(a)));
    e.utc = start.utc;
    e.span = span.clamp(1, 3660) as u32;
    if e.title.is_empty() { e.title = String::from("(untitled)"); }
    Some(e)
}

/// Writes one content line, folded at 75 octets on a character boundary.
fn line(out: &mut String, text: &str) {
    let mut width = 0;
    for c in text.chars() {
        if width + c.len_utf8() > 75 { out.push_str("\r\n "); width = 1; }
        out.push(c);
        width += c.len_utf8();
    }
    out.push_str("\r\n");
}

fn text_value(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c { '\\' => out.push_str("\\\\"), ';' => out.push_str("\\;"), ',' => out.push_str("\\,"), '\n' => out.push_str("\\n"), '\r' => {}, c => out.push(c) }
    }
    out
}

fn stamp(d: Date, minute: u16, utc: bool) -> String {
    alloc::format!("{:04}{:02}{:02}T{:02}{:02}00{}", d.year, d.month, d.day, minute / 60, minute % 60, if utc { "Z" } else { "" })
}
fn date_value(d: Date) -> String { alloc::format!("{:04}{:02}{:02}", d.year, d.month, d.day) }

/// One VEVENT. `dtstamp` is the moment it is written, as `YYYYMMDDTHHMMSSZ`.
pub fn write_event(out: &mut String, e: &Event, dtstamp: &str) {
    line(out, "BEGIN:VEVENT");
    if !e.uid.is_empty() { line(out, &alloc::format!("UID:{}", e.uid)); }
    line(out, &alloc::format!("DTSTAMP:{dtstamp}"));
    let dates = |out: &mut String, name: &str, list: &[Date]| {
        if list.is_empty() { return; }
        let mut v = String::new();
        for (i, d) in list.iter().enumerate() {
            if i > 0 { v.push(','); }
            match e.time { Some((a, _)) => v.push_str(&stamp(*d, a, e.utc)), None => v.push_str(&date_value(*d)) }
        }
        line(out, &alloc::format!("{name}{}:{v}", if e.time.is_none() { ";VALUE=DATE" } else { "" }));
    };
    match e.time {
        None => {
            line(out, &alloc::format!("DTSTART;VALUE=DATE:{}", date_value(e.start)));
            line(out, &alloc::format!("DTEND;VALUE=DATE:{}", date_value(e.start.add_days(e.span.max(1) as i64))));
        }
        Some((a, b)) => {
            line(out, &alloc::format!("DTSTART:{}", stamp(e.start, a, e.utc)));
            // The end lies on the event's last day, or the day after when it is midnight.
            let last = e.start.add_days(e.span.max(1) as i64 - 1);
            let end = if b == 0 && e.span > 1 { (last.add_days(1), 0) } else { (last, b) };
            if end != (e.start, a) { line(out, &alloc::format!("DTEND:{}", stamp(end.0, end.1, e.utc))); }
        }
    }
    if let Some(r) = e.recurrence_id { dates(out, "RECURRENCE-ID", &[r]); }
    if let Some(r) = &e.rule { line(out, &alloc::format!("RRULE:{r}")); }
    dates(out, "EXDATE", &e.exdates);
    dates(out, "RDATE", &e.rdates);
    line(out, &alloc::format!("SUMMARY:{}", text_value(&e.title)));
    if !e.location.is_empty() { line(out, &alloc::format!("LOCATION:{}", text_value(&e.location))); }
    if !e.description.is_empty() { line(out, &alloc::format!("DESCRIPTION:{}", text_value(&e.description))); }
    if e.sequence != 0 { line(out, &alloc::format!("SEQUENCE:{}", e.sequence)); }
    if !e.organizer.is_empty() { line(out, &alloc::format!("ORGANIZER:{}", e.organizer)); }
    for a in &e.attendees {
        let mut l = String::from("ATTENDEE");
        if !a.name.is_empty() { let _ = write!(l, ";CN=\"{}\"", a.name.replace('"', "")); }
        let _ = write!(l, ";PARTSTAT={}", a.partstat.as_str());
        if a.rsvp { l.push_str(";RSVP=TRUE"); }
        let _ = write!(l, ":{}", a.address);
        line(out, &l);
    }
    for m in &e.alarms {
        line(out, "BEGIN:VALARM");
        line(out, "ACTION:DISPLAY");
        line(out, &alloc::format!("DESCRIPTION:{}", text_value(&e.title)));
        let (sign, m) = if *m >= 0 { ("-", *m) } else { ("", -*m) };
        line(out, &alloc::format!("TRIGGER:{sign}PT{m}M"));
        line(out, "END:VALARM");
    }
    line(out, "END:VEVENT");
}

/// A whole iCalendar file holding `events`, with METHOD when given.
pub fn export<'a>(events: impl IntoIterator<Item = &'a Event>, method: Option<&str>, dtstamp: &str) -> String {
    let mut out = String::new();
    line(&mut out, "BEGIN:VCALENDAR");
    line(&mut out, "VERSION:2.0");
    line(&mut out, "PRODID:-//SexOS//Date//EN");
    if let Some(m) = method { line(&mut out, &alloc::format!("METHOD:{m}")); }
    for e in events { write_event(&mut out, e, dtstamp); }
    line(&mut out, "END:VCALENDAR");
    out
}

/// The answer to an invitation (METHOD:REPLY): the event's identity and my attendee
/// line only, for the mail program to send to the organizer.
pub fn reply(e: &Event, me: &[&str], dtstamp: &str) -> Option<String> {
    let i = e.my_attendee(me)?;
    let mut answer = Event { attendees: alloc::vec![e.attendees[i].clone()], alarms: Vec::new(), description: String::new(), ..e.clone() };
    answer.attendees[0].rsvp = false;
    Some(export([&answer], Some("REPLY"), dtstamp))
}
