// SPDX-License-Identifier: GPL-3.0-only
//! The importer against a fixture written the way other calendar programs write
//! (CRLF lines, folding, escapes, nested alarms, a to-do to skip), and the same
//! fixture the QEMU gate gives the program.
use sex_calendar::ics::{self, parse_when, When};
use sex_calendar::{Calendar, Date, Freq, MonthGrid};

const FIXTURE: &str = include_str!("fixture.ics");
fn d(y: i32, m: u8, day: u8) -> Date { Date::new(y, m, day).unwrap() }
fn titles(c: &Calendar, day: Date) -> Vec<String> { c.on_day(day).iter().map(|e| e.title.clone()).collect() }

#[test]
fn imports_every_event() {
    let events = ics::import(FIXTURE).unwrap();
    assert_eq!(events.len(), 6);
    let sync = &events[0];
    assert_eq!((sync.start, sync.time, sync.utc, sync.span), (d(2026, 3, 2), Some((540, 570)), true, 1));
    assert_eq!(sync.location, "Room 4");
    assert_eq!(sync.rule.unwrap().count, Some(4));
    // The alarm's SUMMARY is the alarm's, not the event's.
    assert_eq!(sync.title, "Team sync");
    assert_eq!((events[1].time, events[1].span), (None, 3));
    assert_eq!(events[2].rule.unwrap().freq, Freq::Yearly);
    let lunch = &events[5];
    assert_eq!(lunch.title, "Lunch with the design team, then a long walk along the river to talk about the next release");
    assert_eq!(lunch.location, "Café, Main St; Floor 2");
    assert_eq!((lunch.time, lunch.utc), (Some((750, 840)), false));
}

#[test]
fn occurrences_land_on_their_days() {
    let c = Calendar { events: ics::import(FIXTURE).unwrap() };
    assert_eq!(titles(&c, d(2026, 3, 16)), ["Birthday", "Conference", "Team sync", "Standup"]);
    assert_eq!(titles(&c, d(2026, 3, 17)), ["Conference"]);
    assert_eq!(titles(&c, d(2026, 3, 18)), ["Standup"]);
    // UNTIL is 20 March 00:00 UTC, so the 10:00 standup that day is past it.
    assert_eq!(titles(&c, d(2026, 3, 20)), ["Lunch with the design team, then a long walk along the river to talk about the next release"]);
    // COUNT=4 weekly: 2, 9, 16 and 23 March, then no more.
    assert!(c.on_day(d(2026, 3, 23)).iter().any(|e| e.title == "Team sync"));
    assert!(c.on_day(d(2026, 3, 30)).is_empty());
    assert_eq!(titles(&c, d(2026, 3, 31)), ["Monthly review"]);
    assert!(c.on_day(d(2026, 4, 30)).is_empty());
    assert_eq!(titles(&c, d(2031, 3, 16)), ["Birthday"]);
    let grid = MonthGrid::new(2026, 3);
    let counts = c.counts(&grid);
    let at = |day| counts[grid.index_of(day).unwrap()];
    assert_eq!((at(d(2026, 3, 16)), at(d(2026, 3, 15)), at(d(2026, 3, 1)), at(d(2026, 3, 9))), (4, 1, 0, 1));
    // The grid's own count agrees with the day-by-day one everywhere.
    for i in 0..MonthGrid::CELLS { assert_eq!(counts[i] as usize, c.on_day(grid.cell(i)).len(), "{}", grid.cell(i)); }
}

#[test]
fn saves_and_merges() {
    let mut c = Calendar::default();
    assert_eq!(c.merge(ics::import(FIXTURE).unwrap()), 6);
    assert_eq!(c.merge(ics::import(FIXTURE).unwrap()), 0);
    let again = Calendar::parse(&c.serialize());
    assert_eq!(again, c);
    assert_eq!(titles(&again, d(2026, 3, 16)).len(), 4);
}

#[test]
fn values_and_refusals() {
    assert_eq!(parse_when("20260316"), Some(When { date: d(2026, 3, 16), minute: None, utc: false }));
    assert_eq!(parse_when("20260316T235900Z"), Some(When { date: d(2026, 3, 16), minute: Some(1439), utc: true }));
    assert_eq!(parse_when("20260230"), None);
    assert_eq!(parse_when("20260316T250000"), None);
    assert_eq!(parse_when("20260316T120000+01"), None);
    assert_eq!(ics::import("hello"), Err(ics::Error::NotCalendar));
    // An event left open, or without a start, is skipped.
    let broken = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:no start\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20260101\n";
    assert_eq!(ics::import(broken), Ok(vec![]));
    assert_eq!(ics::unfold("A:1\r\n 2\r\n\t3\r\nB:4"), ["A:123", "B:4"]);
    assert_eq!(ics::unescape(r"a\,b\;c\\d\ne"), "a,b;c\\d\ne");
    let (name, params, value) = ics::split_line("dtstart;TZID=\"A:B\";VALUE=DATE-TIME:20260101T000000").unwrap();
    assert_eq!((name.as_str(), params[0].1.as_str(), value), ("DTSTART", "A:B", "20260101T000000"));
}
