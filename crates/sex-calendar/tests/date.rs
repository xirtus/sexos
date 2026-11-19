// SPDX-License-Identifier: GPL-3.0-only
//! Date's model beyond the first calendar: RRULE BY parts (expected days computed with
//! python-dateutil), EXDATE/RDATE, several calendars, reminders, the world clock's
//! zones, alarms and timers, export round trips and invitations.
use sex_calendar::clock::{self, Alarm, Clock};
use sex_calendar::{ics, tz, Calendar, Date, Event, PartStat, Rule, SyncState};

fn d(y: i32, m: u8, day: u8) -> Date { Date::new(y, m, day).unwrap() }
fn unix_min(text: &str) -> i64 {
    // YYYY-MM-DDTHH:MM in UTC.
    let day = Date::parse(&text[..10]).unwrap();
    day.days() * 1440 + text[11..13].parse::<i64>().unwrap() * 60 + text[14..16].parse::<i64>().unwrap()
}

fn case(start: &str, rule: &str, expect: &[&str]) {
    let w = ics::parse_when(start).unwrap();
    let mut e = Event::all_day(w.date, "x");
    e.rule = Some(Rule::parse(rule).unwrap_or_else(|| panic!("{rule}")));
    let mut seen = Vec::new();
    e.starts_between(d(2020, 1, 1), d(2035, 1, 1), |s| seen.push(s.to_string()));
    assert_eq!(seen, expect, "{rule}");
    // Starting the walk mid-way (no COUNT) finds the same days.
    if !rule.contains("COUNT") {
        let mut late = Vec::new();
        e.starts_between(Date::parse(expect[2]).unwrap(), d(2035, 1, 1), |s| late.push(s.to_string()));
        assert_eq!(late, &expect[2..], "{rule} from the third");
    }
    // The rule reads back as it was written.
    assert_eq!(Rule::parse(&e.rule.as_ref().unwrap().to_string()), e.rule);
}

#[test]
fn rrule_by_parts_match_dateutil() {
    case("20260105", "FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=7", &["2026-01-05", "2026-01-07", "2026-01-09", "2026-01-12", "2026-01-14", "2026-01-16", "2026-01-19"]);
    case("20260130", "FREQ=MONTHLY;BYDAY=-1FR;COUNT=4", &["2026-01-30", "2026-02-27", "2026-03-27", "2026-04-24"]);
    case("20260113", "FREQ=MONTHLY;BYDAY=2TU;COUNT=3", &["2026-01-13", "2026-02-10", "2026-03-10"]);
    case("20260131", "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=4", &["2026-01-31", "2026-02-28", "2026-03-31", "2026-04-30"]);
    case("20260130", "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=3", &["2026-01-30", "2026-02-27", "2026-03-31"]);
    case("20261126", "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH;COUNT=3", &["2026-11-26", "2027-11-25", "2028-11-23"]);
    case("20260213", "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13;COUNT=3", &["2026-02-13", "2026-03-13", "2026-11-13"]);
    case("20260106", "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;UNTIL=20260215", &["2026-01-06", "2026-01-08", "2026-01-20", "2026-01-22", "2026-02-03", "2026-02-05"]);
    case("20260103", "FREQ=DAILY;BYDAY=SA,SU;COUNT=4", &["2026-01-03", "2026-01-04", "2026-01-10", "2026-01-11"]);
    case("20260518", "FREQ=YEARLY;BYDAY=20MO;COUNT=2", &["2026-05-18", "2027-05-17"]);
    case("20260330", "FREQ=MONTHLY;BYMONTHDAY=1,15,30;COUNT=5", &["2026-03-30", "2026-04-01", "2026-04-15", "2026-04-30", "2026-05-01"]);
    case("20260111", "FREQ=WEEKLY;INTERVAL=2;BYDAY=SU,MO;WKST=SU;COUNT=4", &["2026-01-11", "2026-01-12", "2026-01-25", "2026-01-26"]);
    assert_eq!(Rule::parse("FREQ=WEEKLY;BYDAY=XX"), None);
}

#[test]
fn exdate_rdate_and_overrides() {
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:w@x\r\nDTSTART:20260302T090000Z\r\nDTEND:20260302T100000Z\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\n\
EXDATE:20260309T090000Z\r\nRDATE;VALUE=DATE:20260320\r\nSUMMARY:Weekly\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:w@x\r\nRECURRENCE-ID:20260316T090000Z\r\nDTSTART:20260317T110000Z\r\nDURATION:PT30M\r\nSUMMARY:Weekly (moved)\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let events = ics::import(text).unwrap();
    assert_eq!(events.len(), 2);
    let mut seen = Vec::new();
    events[0].starts_between(d(2026, 1, 1), d(2026, 12, 31), |s| seen.push(s));
    // COUNT=4 counts the excluded 9th; the moved 16th is excluded; the RDATE adds the 20th.
    assert_eq!(seen, [d(2026, 3, 2), d(2026, 3, 20), d(2026, 3, 23)]);
    assert_eq!((events[1].start, events[1].time, events[1].recurrence_id), (d(2026, 3, 17), Some((660, 690)), Some(d(2026, 3, 16))));
    let c = Calendar { events, ..Default::default() };
    assert_eq!(c.on_day(d(2026, 3, 17))[0].title, "Weekly (moved)");
    assert!(c.on_day(d(2026, 3, 16)).is_empty() && c.on_day(d(2026, 3, 9)).is_empty());
}

#[test]
fn several_calendars_hide_and_show() {
    let mut c = Calendar::default();
    let work = c.add_calendar("Work", "");
    assert_eq!(work, 1);
    assert_eq!(c.all_calendars().len(), 2);
    let mut e = Event::all_day(d(2026, 5, 1), "Ship it");
    e.calendar = work;
    c.events.push(e);
    c.events.push(Event::all_day(d(2026, 5, 1), "Lunch"));
    assert_eq!(c.on_day(d(2026, 5, 1)).len(), 2);
    assert!(!c.toggle(work));
    assert_eq!(c.on_day(d(2026, 5, 1)).len(), 1);
    let again = Calendar::parse(&c.serialize());
    assert_eq!(again, c);
    assert_eq!(again.on_day(d(2026, 5, 1))[0].title, "Lunch");
    assert_ne!(c.info(0).color, c.info(work).color);
    // An event waiting to be deleted on the server is not shown.
    let mut c2 = c.clone();
    c2.toggle(work);
    c2.events[1].state = SyncState::Deleted;
    assert_eq!(c2.on_day(d(2026, 5, 1)).len(), 1);
}

#[test]
fn reminders_fall_due() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:r@x\nDTSTART:20260316T090000Z\nRRULE:FREQ=DAILY;COUNT=3\nSUMMARY:Standup\n\
BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;VALUE=DATE-TIME:20260316T080000Z\nEND:VALARM\n\
END:VEVENT\nBEGIN:VEVENT\nUID:f@x\nDTSTART:20260316T100000\nSUMMARY:Floating\nBEGIN:VALARM\nTRIGGER:-P1D\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n";
    let c = Calendar { events: ics::import(text).unwrap(), ..Default::default() };
    assert_eq!(c.events[0].alarms, [15, 60]);
    assert_eq!(c.events[1].alarms, [1440]);
    let due = c.reminders_due(unix_min("2026-03-16T08:40"), unix_min("2026-03-16T08:45"), 0);
    assert_eq!(due, [(0, d(2026, 3, 16), unix_min("2026-03-16T08:45"))]);
    // The second day's reminders, and none twice.
    let due = c.reminders_due(unix_min("2026-03-16T08:45"), unix_min("2026-03-17T08:50"), 0);
    assert_eq!(due.iter().map(|x| x.2).collect::<Vec<_>>(), [unix_min("2026-03-17T08:00"), unix_min("2026-03-17T08:45")]);
    // A floating time follows the local zone: 10:00 in UTC+1 is 09:00 UTC, a day earlier.
    let due = c.reminders_due(unix_min("2026-03-15T08:59"), unix_min("2026-03-15T09:00"), 60);
    assert_eq!(due, [(1, d(2026, 3, 16), unix_min("2026-03-15T09:00"))]);
    assert_eq!(ics::parse_duration("-P1DT2H30M"), Some(-1590));
    assert_eq!(ics::parse_duration("P2W"), Some(20160));
    assert_eq!(ics::parse_duration("PT1H5"), None);
}

#[test]
fn zones_and_daylight_time() {
    let at = |zone: &str, t: &str| tz::find(zone).unwrap().offset(unix_min(t));
    for (zone, times, want) in [
        ("Europe/Berlin", ["2026-03-29T00:59", "2026-03-29T01:00", "2026-10-25T00:59", "2026-10-25T01:00"], [60, 120, 120, 60]),
        ("America/New_York", ["2026-03-08T06:59", "2026-03-08T07:00", "2026-11-01T05:59", "2026-11-01T06:00"], [-300, -240, -240, -300]),
        ("Australia/Sydney", ["2026-04-04T15:59", "2026-04-04T16:00", "2026-10-03T15:59", "2026-10-03T16:00"], [660, 600, 600, 660]),
        ("Pacific/Auckland", ["2026-04-04T13:59", "2026-04-04T14:00", "2026-09-26T13:59", "2026-09-26T14:00"], [780, 720, 720, 780]),
        ("Australia/Adelaide", ["2026-04-04T16:29", "2026-04-04T16:30", "2026-10-03T16:29", "2026-10-03T16:30"], [630, 570, 570, 630]),
    ] {
        for (t, w) in times.iter().zip(want) { assert_eq!(at(zone, t), w, "{zone} {t}"); }
    }
    assert_eq!(at("kolkata", "2026-06-01T00:00"), 330);
    assert_eq!(tz::find("Tokyo").unwrap().local(unix_min("2026-12-31T20:00")), (d(2027, 1, 1), 5 * 60));
    assert_eq!(tz::format_offset(-570), "-09:30");
    assert!(tz::find("Mars/Olympus").is_none());
}

#[test]
fn alarms_and_timers() {
    let mut c = Clock::parse("# sex-clock 1\nhome\tEurope/Berlin\nzone\tAsia/Tokyo\nzone\tNowhere/At_All\nalarm\t07:30\t1111100\t1\tWake up\nalarm\t12:00\t0000000\t1\tOnce\ntimer\t5000\t300\tTea\n");
    assert_eq!(c.zones, ["Asia/Tokyo"]);
    assert_eq!(c.alarms[0], Alarm { minute: 450, days: 0b0011111, enabled: true, label: "Wake up".into() });
    assert_eq!(Clock::parse(&c.serialize()), c);
    // Friday 13 November 2026, 06:30 UTC is 07:30 in Berlin; Saturday's is skipped.
    let rung = c.alarms_due(unix_min("2026-11-13T06:29"), unix_min("2026-11-13T06:30"));
    assert_eq!(rung.len(), 1);
    assert!(c.alarms_due(unix_min("2026-11-14T06:00"), unix_min("2026-11-14T07:00")).is_empty());
    // A one-time alarm rings once and switches itself off.
    assert_eq!(c.alarms_due(unix_min("2026-11-14T10:00"), unix_min("2026-11-14T11:00"))[0].label, "Once");
    assert!(!c.alarms[1].enabled);
    assert!(c.alarms_due(unix_min("2026-11-15T10:00"), unix_min("2026-11-15T11:00")).is_empty());
    assert!(c.timers_done(4999).is_empty());
    assert_eq!(c.timers_done(5000)[0].label, "Tea");
    assert!(c.timers.is_empty());
    assert_eq!(clock::parse_duration("1h30m"), Some(5400));
    assert_eq!(clock::parse_duration("90"), Some(90));
    assert_eq!(clock::parse_duration("5s"), Some(5));
    assert_eq!(clock::parse_duration("02:30"), Some(150));
    assert_eq!(clock::parse_duration("x"), None);
    assert_eq!(clock::format_duration(5405), "1h30m05s");
    assert_eq!(Clock::default().home_zone().name, "UTC");
    assert_eq!(Clock::default().world().len(), 7);
}

const INVITE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:plan@example.org\r\nSEQUENCE:2\r\n\
DTSTART:20261120T140000Z\r\nDTEND:20261120T150000Z\r\nSUMMARY:Planning\r\nORGANIZER;CN=Boss:mailto:boss@example.org\r\n\
ATTENDEE;CN=Boss;PARTSTAT=ACCEPTED:mailto:boss@example.org\r\nATTENDEE;CN=\"Me, Myself\";PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.org\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn invitations_accept_and_decline() {
    let mut e = ics::import(INVITE).unwrap().remove(0);
    assert!(e.invitation);
    assert_eq!((e.organizer.as_str(), e.attendees.len(), e.sequence), ("mailto:boss@example.org", 2, 2));
    assert_eq!(e.attendees[1].name, "Me, Myself");
    // Without knowing my address, the one attendee asked to answer is me.
    assert_eq!(e.my_attendee(&[]), Some(1));
    e.href = "/cal/plan.ics".into();
    assert!(e.respond(&["ME@example.org"], PartStat::Accepted));
    assert_eq!((e.attendees[1].partstat, e.invitation, e.state), (PartStat::Accepted, false, SyncState::Modified));
    let reply = ics::reply(&e, &["me@example.org"], "20261114T120000Z").unwrap();
    assert!(reply.contains("METHOD:REPLY\r\n") && reply.contains("PARTSTAT=ACCEPTED:mailto:me@example.org") && reply.matches("ATTENDEE").count() == 1);
    let back = ics::import(&reply).unwrap().remove(0);
    assert_eq!((back.uid.as_str(), back.attendees.len(), back.attendees[0].partstat), ("plan@example.org", 1, PartStat::Accepted));
    assert!(e.respond(&["me@example.org"], PartStat::Declined));
    assert!(ics::export([&e], None, "20261114T120000Z").contains("PARTSTAT=DECLINED"));
}

#[test]
fn export_round_trips() {
    let fixture = include_str!("fixture.ics");
    let mut events = ics::import(fixture).unwrap();
    events.extend(ics::import(INVITE).unwrap());
    let mut extra = Event::all_day(d(2026, 7, 1), "Holiday; long, with \\ backslash\nand a second line that is long enough to fold across the 75 octet limit, ünïcödé");
    extra.span = 14;
    extra.exdates = vec![d(2027, 7, 1)];
    extra.rule = Rule::parse("FREQ=YEARLY;BYMONTH=7;BYMONTHDAY=1");
    extra.uid = "h@x".into();
    events.push(extra);
    let text = ics::export(&events, None, "20261114T120000Z");
    assert!(text.lines().all(|l| l.len() <= 76));
    let mut back = ics::import(&text).unwrap();
    // A request read back without METHOD is no longer an invitation.
    for (a, b) in events.iter().zip(back.iter_mut()) { b.invitation = a.invitation; }
    assert_eq!(back, events);
}
