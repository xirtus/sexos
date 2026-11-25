// SPDX-License-Identifier: GPL-3.0-only
//! Date, the calendar: month, week and day views, several calendars with their own
//! colours (each can be hidden), repeating events (RRULE with BYDAY, BYMONTHDAY,
//! EXDATE...), reminders, alarms and timers that ring through Bell, a world clock,
//! invitations to accept or decline, and two-way CalDAV sync. Events live in the
//! program's own folder (`events.txt`, see sex_calendar::Calendar), the clock in
//! `clock.txt`, the CalDAV account in `dav.txt`. `run date @file.ics` imports a file;
//! `run +net date @account.dav` takes a CalDAV account and syncs.
//!
//! Keys, anywhere: `m` month, `w` week, `d` day, `c` clock, `l` calendars, `s` sync,
//! `e` export every shown event to `date-export.ics`.
//! `v` reviews local copies kept after CalDAV conflicts; press again for the next.
//! Ctrl+Z / Ctrl+Y undo and redo local event additions and removals until the
//! next server sync or other calendar change, and Clock zone, alarm and timer edits.
//! Month: arrows move a day or a week, PageUp and PageDown a month, Home (or `t`)
//! today, Enter opens the chosen day and Esc goes back, `n` adds an event (start it
//! with `HH:MM` to give it a time). An open day: Up/Down pick, `d` copies the
//! selected occurrence to tomorrow, Delete removes, `a`
//! prepares accepting an invitation, `x` prepares declining it; Enter queues a
//! local reply and Esc cancels. Week and day: Left/Right a day, Up/Down
//! scroll the hours, PageUp/PageDown a week. Clock: `n` new alarm (`07:30 Wake up`),
//! `r` new timer (`5m Tea`), `z` add a zone, `h` set the local zone, Space switches an
//! alarm on or off, Delete removes. Calendars: Up/Down pick, Space shows or hides,
//! `n` adds a local calendar; new events go to the picked one. The mouse picks days,
//! hours, tabs, calendars.
//! Press `?` for the complete in-app shortcut reference; Up/Down scroll it.
#![no_std]
#![no_main]

extern crate alloc;

mod dav;
mod draw;
mod views;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use draw::Face;
use sex_app::{files, settings, Event, Nav, Window};
use sex_calendar::clock::{self, Alarm, Clock, Timer};
use sex_calendar::{ics, tz, Calendar, Date, Event as Entry, MonthGrid, PartStat, SyncState};
use sex_calendar::history::{ClockEditKind, ClockHistory, ClockState, EventEdit, EventHistory, ReplyDraftHistory};
use sex_calendar::notice_cursor::{deliver_due, lease_live, DueNotice, NoticeCadence, NoticeCursor, NoticeRetry, ReceiptJournal};
use sex_calendar::conflicts;
use sex_calendar::storage::{finish_replace, ReplaceFailure as SaveFailure};
use sex_dav::FileAccount;
use sex_pdx::app_pointer::Kind;

const FILE: &[u8] = b"events.txt";
const CLOCK: &[u8] = b"clock.txt";
const NOTICE_CURSOR: &[u8] = b"notice.cursor";
const NOTICE_RECEIPTS: &[u8] = b"notice.receipts";
const NOTICE_UI: &[u8] = b"notice.ui-open";
const NOTICE_UI_PREFIX: &[u8] = b"notice.ui-";
const NOTICE_WORKER_ACTIVE: &[u8] = b"notice.worker-active";
const NOTICE_WORKER_ARG: &[u8] = b"--notice-worker";
const ACCOUNT: &[u8] = b"dav.txt";
/// The event file as read, and an imported `.ics` file.
static mut STORE: [u8; 1 << 20] = [0; 1 << 20];
static mut IMPORT: [u8; 1 << 20] = [0; 1 << 20];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[calendar.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View { Month, Week, Day, Clock, Calendars }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask { Event, CopyDate, MoveDate, Title, Location, Time, Alarm, Timer, Zone, Home, NewCalendar }

pub struct App {
    pub cal: Calendar,
    event_history: EventHistory,
    pub reply_history: ReplyDraftHistory,
    clock_history: ClockHistory,
    pub clock: Clock,
    pub today: Date,
    pub selected: Date,
    pub view: View,
    pub palette: views::Palette,
    theme_generation: u64,
    pub help: bool,
    pub help_offset: usize,
    /// The retained local conflict note being compared with the server copy.
    pub review: Option<(usize, usize, usize)>,
    /// A conflict choice waits for Enter before a staged calendar save.
    pub review_choice: Option<conflicts::Choice>,
    /// The chosen day's events listed with details (Enter in the month).
    pub agenda: bool,
    /// What is being typed, and for what.
    pub ask: Option<Ask>,
    pub input: String,
    /// One line at the bottom: what just happened.
    pub status: String,
    /// Something that rang, shown at the top until a key or click.
    pub banner: Option<String>,
    /// The event picked in an open day, the row picked in the clock or calendars view.
    pub picked: usize,
    /// The first hour shown in the week and day views.
    pub first_hour: u16,
    /// The calendar new events go to.
    pub target: u16,
    /// The civil clock now (Unix ms; 0 when the machine has none).
    pub now_ms: u64,
    /// Reminders and alarms are looked for after this minute (since 1970 UTC).
    last_minute: i64,
    cursor_ready: bool,
    notice_retry: NoticeRetry,
    receipts: Option<ReceiptJournal>,
    uid_counter: u32,
}

pub fn when(e: &Entry) -> String {
    match e.time {
        None => String::from("all day"),
        Some((a, b)) if a == b => format!("{:02}:{:02}{}", a / 60, a % 60, if e.utc { " UTC" } else { "" }),
        Some((a, b)) => format!("{:02}:{:02}-{:02}:{:02}{}", a / 60, a % 60, b / 60, b % 60, if e.utc { " UTC" } else { "" }),
    }
}

/// `09:30 Dentist` is an event at half past nine; anything else is all day.
fn quick_event(day: Date, typed: &str) -> Entry {
    let mut e = Entry::all_day(day, typed.trim());
    let b = typed.as_bytes();
    if b.len() > 6 && b[2] == b':' && b[5] == b' ' {
        if let (Ok(h), Ok(m)) = (typed[..2].parse::<u16>(), typed[3..5].parse::<u16>()) {
            if h < 24 && m < 60 { e.time = Some((h * 60 + m, (h * 60 + m + 60).min(24 * 60 - 1))); e.title = String::from(typed[6..].trim()); }
        }
    }
    e
}

fn now_ms() -> u64 { sex_pdx::civil_time_ms().unwrap_or(0) }

/// User mute settles a due timer without showing it. Queue/rate-limit failures
/// retry; a lost acceptance receipt may still replay a notice.
fn post_timer(t: &Timer) -> bool {
    let title = if t.label.is_empty() { "Timer" } else { &t.label };
    let body = format!("{} timer done", clock::format_duration(t.seconds as u64));
    let disposition = sex_app::notify::post_confirmed(title.as_bytes(), body.as_bytes(), sex_pdx::notify::URGENCY_CRITICAL);
    sex_pdx::serial_record!("[date.alarm.fired] kind=timer title={} notified={} settled={}", title,
        (disposition == sex_pdx::notify::Disposition::Accepted) as u8, disposition.settles_due() as u8);
    disposition.settles_due()
}

fn due_text(cal: &Calendar, notice: &DueNotice, offset: i32) -> (&'static str, String, String) {
    match notice {
        DueNotice::Reminder { event, day, at } => {
            let e = &cal.events[*event];
            let start = e.start_minute_utc(*day, offset);
            let lead = (start - at).max(0);
            let body = if lead == 0 { format!("now, {}", when(e)) } else { format!("in {} min, {} {}", lead, day, when(e)) };
            ("reminder", e.title.clone(), body)
        }
        DueNotice::Alarm { alarm, .. } => {
            let title = if alarm.label.is_empty() { String::from("Alarm") } else { alarm.label.clone() };
            ("alarm", title, format!("{:02}:{:02}", alarm.minute / 60, alarm.minute % 60))
        }
    }
}

fn receipt_digest(key: &str) -> u128 {
    let (low, high) = sex_pdx::notify::semantic_digest(key.as_bytes());
    (high as u128) << 64 | low as u128
}

/// Each visible run keeps its own lease. A dead run's file stops participating
/// after ten seconds; the smallest live task owns Bell delivery.
fn notice_ui_owner(my_task: u32, now: u64) -> bool {
    let entries = match files::direct::list(b"") { Ok(entries) => entries, Err(_) => return false };
    let mut first: Option<u32> = None;
    for entry in entries {
        let Some(suffix) = entry.name.strip_prefix(NOTICE_UI_PREFIX) else { continue };
        let Ok(name) = core::str::from_utf8(suffix) else { continue };
        let Ok(task) = name.parse::<u32>() else { continue };
        let mut stamp = [0u8; 8];
        if files::direct::read_at(&entry.name, 0, &mut stamp) != Ok(8) { continue; }
        let seen = u64::from_le_bytes(stamp);
        if lease_live(seen, now) { first = Some(first.map_or(task, |current| current.min(task))); }
    }
    first == Some(my_task)
}

fn notice_ui_name(task: u32) -> String { format!("notice.ui-{task}") }

fn notice_fresh(path: &[u8], now: u64) -> bool {
    let mut stamp = [0u8; 8];
    if files::direct::read_at(path, 0, &mut stamp) != Ok(8) { return false; }
    let seen = u64::from_le_bytes(stamp);
    lease_live(seen, now)
}

fn notice_ui_live(now: u64) -> bool {
    let Ok(entries) = files::direct::list(b"") else { return true };
    for entry in entries {
        if !entry.name.starts_with(NOTICE_UI_PREFIX) { continue; }
        let mut stamp = [0u8; 8];
        if files::direct::read_at(&entry.name, 0, &mut stamp) == Ok(8) {
            let seen = u64::from_le_bytes(stamp);
            if lease_live(seen, now) { return true; }
        }
    }
    false
}

/// The one visible Bell owner rereads shared state before checking due notices.
/// Call this on a cadence from the event loop, even while input is arriving.
fn poll_visible_notices(window: &mut Window, app: &mut App, store: &mut [u8],
    seen_events: &mut String, seen_clock: &mut String, notice_owner: &mut Option<bool>) -> bool {
    let now = now_ms();
    let owns = notice_ui_owner(window.task, now);
    let became_owner = owns && *notice_owner != Some(true);
    if *notice_owner != Some(owns) {
        sex_pdx::serial_record!("[date.notice.ui.owner] task={} owns={}", window.task, owns as u8);
        *notice_owner = Some(owns);
    }
    if !owns { app.now_ms = now; return false; }
    if became_owner || app.receipts.is_none() {
        app.receipts = match files::read(window, NOTICE_RECEIPTS, store) {
            Ok(n) => ReceiptJournal::open(Some(&String::from_utf8_lossy(&store[..n]))),
            Err(e) if e == files::NOT_FOUND => ReceiptJournal::open(None),
            Err(_) => None,
        };
    }
    if let Ok(n) = files::read(window, FILE, store) {
        let saved = String::from_utf8_lossy(&store[..n]);
        if saved.as_ref() != seen_events.as_str() { app.cal = Calendar::parse(&saved); *seen_events = saved.into_owned(); }
    }
    if let Ok(n) = files::read(window, CLOCK, store) {
        let saved = String::from_utf8_lossy(&store[..n]);
        // Keep a timer removed in memory when its save failed; the unchanged
        // disk copy must not ring again every half second.
        if saved.as_ref() != seen_clock.as_str() { app.clock = Clock::parse(&saved); *seen_clock = saved.into_owned(); }
    }
    match files::read(window, NOTICE_CURSOR, store) {
        Ok(n) => {
            if let Some(cursor) = NoticeCursor::decode(&String::from_utf8_lossy(&store[..n]), now) {
                app.last_minute = app.last_minute.max(cursor.minute);
                app.cursor_ready = true;
            } else {
                let (cursor, ready) = NoticeCursor::bootstrap(None, now, true,
                    |initial| save_file(window, NOTICE_CURSOR, initial.serialize().as_bytes()).is_ok());
                app.last_minute = app.last_minute.min(cursor.minute);
                app.cursor_ready = ready;
            }
        }
        Err(e) if e == files::NOT_FOUND => {
            let (cursor, ready) = NoticeCursor::bootstrap(None, now, true,
                |initial| save_file(window, NOTICE_CURSOR, initial.serialize().as_bytes()).is_ok());
            app.last_minute = app.last_minute.min(cursor.minute);
            app.cursor_ready = ready;
        }
        Err(_) => app.cursor_ready = false,
    }
    if !app.cursor_ready { app.now_ms = now; return false; }
    app.tick(window)
}

fn ensure_notice_worker() {
    use files::direct as fs;
    if fs::stat(NOTICE_WORKER_ACTIVE).is_ok() {
        if notice_fresh(NOTICE_WORKER_ACTIVE, now_ms()) { return; }
        sex_pdx::serial_record!("[date.notice.worker.stale]");
        let _ = fs::remove(NOTICE_WORKER_ACTIVE);
    }
    if let Err(e) = sex_app::launch::run_hidden(b"date", NOTICE_WORKER_ARG, 0) {
        sex_pdx::serial_record!("[date.notice.worker.launch.failed] {:?}", e);
    }
}

/// The hidden Date run parks while a visible lease or opening handoff is live.
/// It keeps its own claim fresh and rereads saved state on every resume.
fn notice_worker() -> ! {
    use files::direct as fs;
    if fs::create(NOTICE_WORKER_ACTIVE).is_err() { sex_app::launch::exit(0); }
    let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
    sex_pdx::serial_record!("[date.notice.worker.started]");
    loop {
        let now = loop {
            let now = now_ms();
            let _ = fs::write_at(NOTICE_WORKER_ACTIVE, 0, &now.to_le_bytes());
            if now != 0 && !notice_fresh(NOTICE_UI, now) && !notice_ui_live(now) { break now; }
            let _ = sex_pdx::sys_wait_self_ticks(500);
        };
        // A visible run may have changed any of these while the worker waited.
        let (cal, saved_events) = match fs::read_at(FILE, 0, &mut store[..]) {
            Ok(n) => (Calendar::parse(&String::from_utf8_lossy(&store[..n])), true),
            Err(_) => (Calendar::default(), false),
        };
        let (mut clock, saved_clock) = match fs::read_at(CLOCK, 0, &mut store[..]) {
            Ok(n) => (Clock::parse(&String::from_utf8_lossy(&store[..n])), true),
            Err(_) => (Clock::default(), false),
        };
        let (mut cursor, mut cursor_ready) = match fs::read_at(NOTICE_CURSOR, 0, &mut store[..]) {
            Ok(n) => NoticeCursor::bootstrap(Some(&String::from_utf8_lossy(&store[..n])), now,
                saved_events || saved_clock, |initial| save_notice_direct(NOTICE_CURSOR, initial.serialize().as_bytes())),
            Err(e) if e == files::NOT_FOUND => NoticeCursor::bootstrap(None, now,
                saved_events || saved_clock, |initial| save_notice_direct(NOTICE_CURSOR, initial.serialize().as_bytes())),
            Err(_) => NoticeCursor::bootstrap(None, now, saved_events || saved_clock, |_| false),
        };
        let mut receipts = match fs::read_at(NOTICE_RECEIPTS, 0, &mut store[..]) {
            Ok(n) => ReceiptJournal::open(Some(&String::from_utf8_lossy(&store[..n]))),
            Err(e) if e == files::NOT_FOUND => ReceiptJournal::open(None),
            Err(_) => None,
        };
        sex_pdx::serial_record!("[date.notice.receipts.loaded] owner=worker count={}", receipts.as_ref().map_or(0, ReceiptJournal::len));
        if !cursor_ready { sex_pdx::serial_record!("[date.notice.worker.save.failed] file=cursor-initial"); }
        let mut retry = NoticeRetry::default();
        sex_pdx::serial_record!("[date.notice.worker.resumed]");
        loop {
            let now = now_ms();
            let _ = fs::write_at(NOTICE_WORKER_ACTIVE, 0, &now.to_le_bytes());
            if notice_fresh(NOTICE_UI, now) || notice_ui_live(now) {
                sex_pdx::serial_record!("[date.notice.worker.handoff]");
                break;
            }
            if now == 0 { let _ = sex_pdx::sys_wait_self_ticks(500); continue; }
            if receipts.is_none() {
                receipts = match fs::read_at(NOTICE_RECEIPTS, 0, &mut store[..]) {
                    Ok(n) => ReceiptJournal::open(Some(&String::from_utf8_lossy(&store[..n]))),
                    Err(e) if e == files::NOT_FOUND => ReceiptJournal::open(None),
                    Err(_) => None,
                };
            }
            if !cursor_ready {
                match fs::read_at(NOTICE_CURSOR, 0, &mut store[..]) {
                    Ok(n) => {
                        if let Some(saved) = NoticeCursor::decode(&String::from_utf8_lossy(&store[..n]), now) {
                            cursor = saved;
                            cursor_ready = true;
                        } else { cursor_ready = save_notice_direct(NOTICE_CURSOR, cursor.serialize().as_bytes()); }
                    }
                    Err(e) if e == files::NOT_FOUND => {
                        cursor_ready = save_notice_direct(NOTICE_CURSOR, cursor.serialize().as_bytes());
                    }
                    Err(_) => {}
                }
                if !cursor_ready { let _ = sex_pdx::sys_wait_self_ticks(500); continue; }
                sex_pdx::serial_record!("[date.notice.cursor.initialized] owner=worker minute={}", cursor.minute);
            }
            let minute = NoticeCursor::current(now).minute;
            let mut rang = false;
            if !clock.deliver_timers(now, post_timer).is_empty() { rang = true; }
            let mut next_cursor = cursor.minute;
            if minute > cursor.minute && retry.ready(sex_pdx::get_ticks()) {
                let offset = clock.home_zone().offset(minute);
                let delivery = deliver_due(&cal, &mut clock, cursor.minute, minute, offset, |notice| {
                    let (kind, title, body) = due_text(&cal, notice, offset);
                    let key = notice.semantic_key(&cal);
                    let digest = key.as_deref().map(receipt_digest);
                    if digest.is_some() && receipts.is_none() { return false; }
                    if digest.is_some_and(|digest| receipts.as_ref().is_some_and(|journal| journal.contains(notice.at(), digest))) {
                        sex_pdx::serial_record!("[date.notice.receipt.reused] kind={} minute={}", kind, notice.at());
                        return true;
                    }
                    let disposition = sex_app::notify::post_confirmed_keyed(title.as_bytes(), body.as_bytes(),
                        sex_pdx::notify::URGENCY_CRITICAL, key.as_deref().map(str::as_bytes));
                    let settled = disposition.settles_due() && digest.map_or(true, |digest|
                        receipts.as_mut().is_some_and(|journal| journal.record_after_ack(notice.at(), digest,
                            |text| save_notice_direct(NOTICE_RECEIPTS, text.as_bytes()))));
                    sex_pdx::serial_record!("[date.alarm.fired] kind={} title={} notified={} settled={}", kind, title,
                        (disposition == sex_pdx::notify::Disposition::Accepted) as u8, settled as u8);
                    settled
                });
                if delivery.failed { retry.failed(sex_pdx::get_ticks()); } else { retry.succeeded(); }
                rang |= !delivery.delivered.is_empty();
                next_cursor = delivery.cursor;
            }
            if rang && !save_notice_direct(CLOCK, clock.serialize().as_bytes()) {
                sex_pdx::serial_record!("[date.notice.worker.save.failed] file=clock");
            } else if next_cursor > cursor.minute && !save_notice_direct(NOTICE_CURSOR, NoticeCursor { minute: next_cursor }.serialize().as_bytes()) {
                sex_pdx::serial_record!("[date.notice.worker.save.failed] file=cursor");
            }
            // The failed minute stays pending; a new run can recover from the
            // last durable cursor even if this save fails.
            cursor.minute = next_cursor;
            let _ = sex_pdx::sys_wait_self_ticks(500);
        }
    }
}

fn save_notice_direct(path: &[u8], bytes: &[u8]) -> bool {
    use files::direct as fs;
    const STAGE: &[u8] = b"notice.worker-stage";
    let _ = fs::remove(STAGE);
    if fs::create(STAGE).is_err() || fs::write_at(STAGE, 0, bytes) != Ok(bytes.len()) || fs::sync().is_err() {
        let _ = fs::remove(STAGE);
        return false;
    }
    fs::rename(STAGE, path).is_ok() && fs::sync().is_ok()
}

/// `20261114T101500Z` for DTSTAMP.
fn stamp(ms: u64) -> String {
    let m = (ms / 60_000) as i64;
    let d = Date::from_days(m.div_euclid(1440));
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", d.year, d.month, d.day, m.rem_euclid(1440) / 60, m.rem_euclid(60), ms / 1000 % 60)
}

/// Commit a complete replacement in the program's own folder. The old file
/// remains readable if the write or its first sync fails.
fn save_file_phase(window: &mut Window, live: &[u8], data: &[u8]) -> Result<(), SaveFailure<files::Error>> {
    let name = core::str::from_utf8(live).unwrap_or("date");
    let stage = format!("{name}.stage-{}", window.task);
    let _ = files::remove(window, stage.as_bytes());
    if let Err(e) = files::write(window, stage.as_bytes(), data) {
        let _ = files::remove(window, stage.as_bytes());
        return Err(SaveFailure::BeforeRename(e));
    }
    if let Err(e) = files::sync(window) {
        let _ = files::remove(window, stage.as_bytes());
        return Err(SaveFailure::BeforeRename(e));
    }
    finish_replace(window, |w| files::rename(w, stage.as_bytes(), live), files::sync)?;
    sex_pdx::serial_record!("[date.save.committed] file={} bytes={}", name, data.len());
    Ok(())
}

fn save_file(window: &mut Window, live: &[u8], data: &[u8]) -> Result<(), files::Error> {
    save_file_phase(window, live, data).map_err(|failure| match failure {
        SaveFailure::BeforeRename(e) | SaveFailure::AfterRename(e) => e,
    })
}

impl App {
    pub fn local_offset(&self) -> i32 { self.clock.home_zone().offset((self.now_ms / 60_000) as i64) }
    fn save(&mut self, window: &mut Window) {
        let text = self.cal.serialize();
        match save_file(window, FILE, text.as_bytes()) {
            Ok(()) => sex_pdx::serial_record!("[calendar.saved] events={} bytes={}", self.cal.events.len(), text.len()),
            Err(e) => {
                sex_pdx::serial_record!("[calendar.save.failed] status={}", e);
                self.status = String::from(core::str::from_utf8(files::message(e)).unwrap_or("Not saved."));
            }
        }
    }
    fn save_clock(&mut self, window: &mut Window) -> bool {
        match save_file(window, CLOCK, self.clock.serialize().as_bytes()) {
            Ok(()) => true,
            Err(e) => {
                self.status = String::from(core::str::from_utf8(files::message(e)).unwrap_or("Not saved."));
                false
            }
        }
    }
    fn new_uid(&mut self) -> String {
        self.uid_counter += 1;
        format!("{}-{}-{}@date.sexos", stamp(self.now_ms), self.uid_counter, sex_pdx::get_ticks() % 100_000)
    }
    fn select(&mut self, day: Date) {
        let month_changed = (day.year, day.month) != (self.selected.year, self.selected.month);
        self.selected = day;
        self.picked = 0;
        if month_changed { sex_pdx::serial_record!("[calendar.month] month={:04}-{:02}", day.year, day.month); }
    }
    fn copy_tomorrow(&mut self, window: &mut Window) {
        self.copy_to(window, self.selected.add_days(1));
    }
    fn copy_to(&mut self, window: &mut Window, day: Date) {
        let Some(source) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let from = self.selected;
        let linked = !self.cal.info(source.calendar).url.is_empty();
        let copy = source.copy_occurrence_to(day, self.new_uid(), linked);
        let index = self.cal.events.len();
        self.cal.events.push(copy.clone());
        self.event_history.push(EventEdit { index, before: None, after: Some(copy.clone()) });
        self.select(day);
        self.picked = self.cal.on_day(day).iter().position(|e| e.uid == copy.uid).unwrap_or(0);
        self.status = format!("Copied \"{}\" to {}.", copy.title, day);
        sex_pdx::serial_record!("[date.event.copied] from={} to={} title={} linked={}", from, day, copy.title, linked as u8);
        self.save(window);
    }
    fn move_to(&mut self, window: &mut Window, day: Date) {
        let Some(source) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let Some(index) = self.cal.events.iter().position(|e| *e == source) else { return };
        if day == source.start { self.status = String::from("This event is already on that date."); return; }
        let linked = !self.cal.info(source.calendar).url.is_empty();
        let moved = match source.moved_personal_to(day, linked) {
            Ok(event) => event,
            Err(why) => { self.status = String::from(why); return; }
        };
        self.cal.events[index] = moved.clone();
        self.event_history.push(EventEdit { index, before: Some(source.clone()), after: Some(moved.clone()) });
        let from = self.selected;
        self.select(day);
        self.picked = self.cal.on_day(day).iter().position(|e| **e == moved).unwrap_or(0);
        self.status = format!("Moved \"{}\" to {}.", moved.title, day);
        sex_pdx::serial_record!("[date.event.moved] from={} to={} title={} linked={}", from, day, moved.title, linked as u8);
        self.save(window);
    }
    fn retitle(&mut self, window: &mut Window, title: &str) {
        let Some(source) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let Some(index) = self.cal.events.iter().position(|e| *e == source) else { return };
        if title.trim() == source.title { self.status = String::from("The title is unchanged."); return; }
        let linked = !self.cal.info(source.calendar).url.is_empty();
        let edited = match source.retitled_personal(title, linked) {
            Ok(event) => event,
            Err(why) => { self.status = String::from(why); return; }
        };
        self.cal.events[index] = edited.clone();
        self.event_history.push(EventEdit { index, before: Some(source.clone()), after: Some(edited.clone()) });
        self.picked = self.cal.on_day(self.selected).iter().position(|e| **e == edited).unwrap_or(0);
        self.status = format!("Renamed event to \"{}\".", edited.title);
        sex_pdx::serial_record!("[date.event.retitled] date={} from={} to={} linked={}", self.selected, source.title, edited.title, linked as u8);
        self.save(window);
    }
    fn relocate(&mut self, window: &mut Window, location: &str) {
        let Some(source) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let Some(index) = self.cal.events.iter().position(|e| *e == source) else { return };
        if location.trim() == source.location { self.status = String::from("The location is unchanged."); return; }
        let linked = !self.cal.info(source.calendar).url.is_empty();
        let edited = match source.located_personal(location, linked) {
            Ok(event) => event,
            Err(why) => { self.status = String::from(why); return; }
        };
        self.cal.events[index] = edited.clone();
        self.event_history.push(EventEdit { index, before: Some(source.clone()), after: Some(edited.clone()) });
        self.picked = self.cal.on_day(self.selected).iter().position(|e| **e == edited).unwrap_or(0);
        self.status = if edited.location.is_empty() { String::from("Cleared event location.") }
            else { format!("Set event location to \"{}\".", edited.location) };
        sex_pdx::serial_record!("[date.event.location] date={} title={} from={} to={} linked={}", self.selected, source.title, source.location, edited.location, linked as u8);
        self.save(window);
    }
    fn retime(&mut self, window: &mut Window, text: &str) {
        let time = if text.eq_ignore_ascii_case("all day") { None } else {
            let Some((start, end)) = text.split_once('-') else { self.status = String::from("Enter HH:MM-HH:MM or all day."); return };
            let (Some(start), Some(end)) = (clock::parse_time(start), clock::parse_time(end)) else {
                self.status = String::from("Enter HH:MM-HH:MM or all day."); return;
            };
            Some((start, end))
        };
        let Some(source) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let Some(index) = self.cal.events.iter().position(|e| *e == source) else { return };
        if time == source.time { self.status = String::from("The time is unchanged."); return; }
        let linked = !self.cal.info(source.calendar).url.is_empty();
        let edited = match source.timed_personal(time, linked) {
            Ok(event) => event,
            Err(why) => { self.status = String::from(why); return; }
        };
        self.cal.events[index] = edited.clone();
        self.event_history.push(EventEdit { index, before: Some(source.clone()), after: Some(edited.clone()) });
        self.picked = self.cal.on_day(self.selected).iter().position(|e| **e == edited).unwrap_or(0);
        self.status = format!("Set \"{}\" to {}.", edited.title, when(&edited));
        sex_pdx::serial_record!("[date.event.timed] date={} title={} from={} to={} linked={}", self.selected, edited.title, when(&source), when(&edited), linked as u8);
        self.save(window);
    }
    fn set_view(&mut self, view: View) {
        self.view = view;
        self.agenda = false;
        self.picked = 0;
        sex_pdx::serial_record!("[date.view] view={:?} date={}", view, self.selected);
    }
    fn open_day(&mut self) {
        self.agenda = true;
        self.picked = 0;
        let events = self.cal.on_day(self.selected);
        sex_pdx::serial_record!("[calendar.day] date={} events={}", self.selected, events.len());
        for e in &events {
            sex_pdx::serial_record!("[calendar.event] date={} when={} title={}", self.selected, when(e), e.title);
        }
    }
    fn review_next(&mut self) {
        self.review_choice = None;
        self.review = conflicts::next(&self.cal, self.review.map(|(index, _, _)| index));
        match self.review {
            Some((index, position, total)) => {
                let server = conflicts::server_copy(&self.cal, index);
                sex_pdx::serial_record!("[date.conflict.review] item={} total={} local=1 server={}", position, total, server.is_some() as u8);
            }
            None => self.status = String::from("No CalDAV conflicts kept for review."),
        }
    }
    fn resolve_review(&mut self, window: &mut Window) {
        let (Some((index, _, _)), Some(choice)) = (self.review, self.review_choice) else { return };
        let next = match conflicts::resolve(&self.cal, index, choice) {
            Ok(next) => next,
            Err(message) => { self.status = String::from(message); self.review_choice = None; return; }
        };
        let text = next.serialize();
        match save_file_phase(window, FILE, text.as_bytes()) {
            result @ (Ok(()) | Err(SaveFailure::AfterRename(_))) => {
                let uncertain = matches!(result, Err(SaveFailure::AfterRename(_)));
                self.cal = next;
                self.event_history.clear();
                self.review = None;
                self.review_choice = None;
                let name = if choice == conflicts::Choice::Local { "local" } else { "server" };
                if uncertain {
                    sex_pdx::serial_record!("[date.conflict.save.uncertain] choice={} events={}", name, self.cal.events.len());
                    self.status = String::from("Resolution replaced the file, but final sync failed. Reopen Date to confirm it persisted.");
                } else {
                    sex_pdx::serial_record!("[date.conflict.resolved] choice={} events={}", name, self.cal.events.len());
                    self.status = if choice == conflicts::Choice::Local {
                        String::from("Kept the local copy. Sync to send it to the server.")
                    } else { String::from("Kept server state and removed the local note.") };
                }
            }
            Err(SaveFailure::BeforeRename(e)) => {
                sex_pdx::serial_record!("[date.conflict.save.failed] status={}", e);
                self.status = format!("Resolution not confirmed: {}", core::str::from_utf8(files::message(e)).unwrap_or("save failed"));
            }
        }
    }
    fn read_file(&mut self, window: &mut Window, path: &[u8]) -> Option<String> {
        let input = unsafe { &mut *core::ptr::addr_of_mut!(IMPORT) };
        let name = core::str::from_utf8(path).unwrap_or("?");
        match files::read(window, path, input) {
            Ok(n) if n == input.len() => { self.status = String::from("That file is larger than 1 MiB."); None }
            Ok(n) => Some(String::from_utf8_lossy(&input[..n]).into_owned()),
            Err(e) => {
                sex_pdx::serial_record!("[calendar.import.failed] path={} status={}", name, e);
                self.status = format!("Could not read {name}: {}", core::str::from_utf8(files::message(e)).unwrap_or("?"));
                None
            }
        }
    }
    fn import(&mut self, window: &mut Window, path: &[u8]) {
        let name = String::from(core::str::from_utf8(path).unwrap_or("?"));
        let Some(text) = self.read_file(window, path) else { return };
        // An account file: kept in the program's folder, then used to sync.
        if name.ends_with(".dav") {
            match FileAccount::parse(&text) {
                Some(a) => {
                    if let Err(e) = save_file(window, ACCOUNT, text.as_bytes()) {
                        self.status = format!("Could not save account: {}", core::str::from_utf8(files::message(e)).unwrap_or("?"));
                        return;
                    }
                    sex_pdx::serial_record!("[date.account] url={} user={}", a.url, a.user);
                    self.status = format!("CalDAV account {} kept; s syncs.", a.url);
                }
                None => self.status = format!("{name} is not an account (url=, user=, password=)."),
            }
            return;
        }
        match ics::import(&text) {
            Ok(mut events) => {
                let found = events.len();
                let invitations = events.iter().filter(|e| e.invitation).count();
                for e in &mut events { e.calendar = self.target; if e.uid.is_empty() { e.uid = self.new_uid(); } }
                let added = self.cal.merge(events);
                if added > 0 { self.event_history.clear(); }
                sex_pdx::serial_record!("[calendar.imported] events={} added={}", found, added);
                if invitations > 0 { sex_pdx::serial_record!("[date.invitations] count={}", invitations); }
                self.status = if invitations > 0 { format!("{invitations} invitation(s) from {name}: open the day, a accepts, x declines.") }
                    else { format!("Imported {found} events from {name} ({added} new).") };
                if added > 0 { self.save(window); }
            }
            Err(_) => {
                sex_pdx::serial_record!("[calendar.import.failed] path={} status=not-calendar", name);
                self.status = format!("{name} is not an iCalendar file.");
            }
        }
    }
    fn account(&mut self, window: &mut Window) -> Option<FileAccount> {
        let store = unsafe { &mut *core::ptr::addr_of_mut!(IMPORT) };
        let n = files::read(window, ACCOUNT, store).ok()?;
        FileAccount::parse(&String::from_utf8_lossy(&store[..n]))
    }
    fn me(&mut self, window: &mut Window) -> Vec<String> {
        self.account(window).map(|a| a.email).filter(|e| !e.is_empty()).into_iter().collect()
    }
    fn sync(&mut self, window: &mut Window) {
        let Some(account) = self.account(window) else {
            self.status = String::from("No CalDAV account: run +net date @account.dav (url=, user=, password=).");
            sex_pdx::serial_record!("[date.sync.failed] reason=no-account");
            return;
        };
        let stamp = stamp(self.now_ms);
        let mut cal = core::mem::take(&mut self.cal);
        let result = (|| -> Result<(usize, usize, usize), sex_dav::Error> {
            let (mut t, base) = dav::transport(window, &account).ok_or(sex_dav::Error::Address)?;
            if !cal.calendars.iter().any(|c| !c.url.is_empty()) {
                let found = sex_dav::sync::discover(&mut t, &base, sex_dav::dav::Kind::Calendar)?;
                if found.is_empty() { cal.add_calendar("Server", &base); }
                for f in found {
                    let id = cal.add_calendar(&f.name, &f.path);
                    if let Some(c) = f.color { cal.info_mut(id).color = c; }
                    sex_pdx::serial_record!("[date.sync.found] id={} name={} path={}", id, f.name, f.path);
                }
            }
            let ids: Vec<u16> = cal.calendars.iter().filter(|c| !c.url.is_empty()).map(|c| c.id).collect();
            let (mut pulled, mut pushed, mut conflicts) = (0, 0, 0);
            for id in ids {
                let out = dav::sync_calendar(&mut t, &mut cal, id, &stamp)?;
                let r = &out.report;
                // The server won; our copies are kept in a local calendar of notes.
                if !r.conflicts.is_empty() {
                    let notes = cal.calendars.iter().find(|c| c.name == "Conflicts" && c.url.is_empty()).map(|c| c.id);
                    let notes = notes.unwrap_or_else(|| cal.add_calendar("Conflicts", ""));
                    for (position, note) in r.conflicts.iter().enumerate() {
                        let href = r.conflict_hrefs.get(position).cloned().unwrap_or_default();
                        for mut e in ics::import(note).unwrap_or_default() {
                            e.calendar = notes;
                            e.title = format!("Conflict: {}", e.title);
                            e.uid = format!("{}-conflict", e.uid);
                            e.conflict_source_calendar = Some(id);
                            e.conflict_source_href = href.clone();
                            sex_pdx::serial_record!("[date.conflict.source] calendar={} href={}", id, href);
                            cal.events.push(e);
                        }
                    }
                }
                sex_pdx::serial_record!("[date.sync] calendar={} unchanged={} incremental={} pulled={} removed={} pushed={} deleted={} conflicts={} failed={}",
                    cal.info(id).name, r.unchanged as u8, r.incremental as u8, r.pulled, r.removed, r.pushed, r.deleted, r.conflicts.len(), r.failed);
                pulled += r.pulled + r.removed;
                pushed += r.pushed + r.deleted;
                conflicts += r.conflicts.len();
            }
            Ok((pulled, pushed, conflicts))
        })();
        self.cal = cal;
        self.event_history.clear();
        match result {
            Ok((pulled, pushed, conflicts)) => {
                sex_pdx::serial_record!("[date.sync.done] events={} pulled={} pushed={} conflicts={}", self.cal.events.len(), pulled, pushed, conflicts);
                self.status = format!("Synced: {pulled} from the server, {pushed} to it{}.", if conflicts > 0 { format!(", {conflicts} conflict(s) kept; V reviews") } else { String::new() });
            }
            Err(e) => {
                sex_pdx::serial_record!("[date.sync.failed] error={:?}", e);
                self.status = format!("Sync failed: {e:?}");
            }
        }
        self.save(window);
    }
    fn export(&mut self, window: &mut Window) {
        let shown: Vec<&Entry> = self.cal.events.iter().filter(|e| self.cal.shown(e)).collect();
        let text = ics::export(shown.iter().copied(), None, &stamp(self.now_ms));
        let n = shown.len();
        match files::write(window, b"date-export.ics", text.as_bytes()) {
            Ok(()) => { sex_pdx::serial_record!("[date.exported] events={} bytes={}", n, text.len()); self.status = format!("Wrote {n} events to date-export.ics."); }
            Err(e) => self.status = format!("Not written: {}", core::str::from_utf8(files::message(e)).unwrap_or("?")),
        }
    }
    fn prepare_answer(&mut self, window: &mut Window, answer: PartStat) {
        let Some(target) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let me = self.me(window);
        let me: Vec<&str> = me.iter().map(String::as_str).collect();
        let mut preview = target.clone();
        if !preview.respond(&me, answer) { self.status = String::from("No attendee to answer for."); return; }
        self.reply_history.choose(target.clone(), answer);
        self.status = format!("{} \"{}\"? Enter queues a local reply; Esc or Ctrl+Z cancels.",
            if answer == PartStat::Accepted { "Accept" } else { "Decline" }, target.title);
        sex_pdx::serial_record!("[date.invite.preview] uid={} partstat={}", target.uid, answer.as_str());
    }
    fn commit_answer(&mut self, window: &mut Window) {
        let Some(draft) = self.reply_history.take() else { return };
        let Some(index) = self.cal.events.iter().position(|e| *e == draft.event) else {
            self.status = String::from("Invitation changed; choose an answer again."); return;
        };
        let me = self.me(window);
        let me: Vec<&str> = me.iter().map(String::as_str).collect();
        let mut answered = draft.event.clone();
        if !answered.respond(&me, draft.answer) { self.status = String::from("No attendee to answer for."); return; }
        let Some(reply) = ics::reply(&answered, &me, &stamp(self.now_ms)) else {
            self.status = String::from("Could not prepare the invitation reply."); return;
        };
        let uid = answered.uid.clone();
        // Valentine will send replies later; Date only queues a local .ics file.
        let name = format!("reply-{}", sex_dav::resource_name(&uid, ".ics"));
        if let Err(e) = files::write_new(window, name.as_bytes(), reply.as_bytes()) {
            self.status = format!("Reply not queued: {}. Check {name} before trying again.",
                core::str::from_utf8(files::message(e)).unwrap_or("file error"));
            sex_pdx::serial_record!("[date.invite.queue.failed] uid={} status={}", uid, e);
            return;
        }
        self.cal.events[index] = answered;
        self.event_history.clear();
        sex_pdx::serial_record!("[date.invite.answered] uid={} partstat={} reply={} bytes={}", uid, draft.answer.as_str(), name, reply.len());
        self.status = format!("{} \"{}\"; the local reply is in {name}.", if draft.answer == PartStat::Accepted { "Accepted" } else { "Declined" }, draft.event.title);
        self.save(window);
    }
    fn snooze_notice(&mut self, window: &mut Window) {
        let Some(notice) = self.banner.take() else { return };
        if self.clock.snooze_notice(now_ms(), &notice) {
            let saved = self.save_clock(window);
            if saved { self.status = String::from("Snoozed for 5 minutes."); }
            sex_pdx::serial_record!("[date.notice.snoozed] seconds=300 saved={}", saved as u8);
        } else {
            self.status = String::from("No clock for snooze.");
        }
    }
    /// Looks at the clock: reminders, alarms and timers due since last time. Returns
    /// whether anything rang.
    fn tick(&mut self, window: &mut Window) -> bool {
        self.now_ms = now_ms();
        if self.now_ms == 0 { return false; }
        let mut rang = false;
        for t in self.clock.deliver_timers(self.now_ms, post_timer) {
            let title = if t.label.is_empty() { "Timer" } else { &t.label };
            self.banner = Some(format!("{title}: {} timer done", clock::format_duration(t.seconds as u64)));
            rang = true;
        }
        let minute = (self.now_ms / 60_000) as i64;
        if self.last_minute == 0 { self.last_minute = minute; }
        let mut advanced = false;
        if minute > self.last_minute && self.notice_retry.ready(sex_pdx::get_ticks()) {
            let offset = self.local_offset();
            let mut banner = None;
            let delivery = deliver_due(&self.cal, &mut self.clock, self.last_minute, minute, offset, |notice| {
                let (kind, title, body) = due_text(&self.cal, notice, offset);
                let key = notice.semantic_key(&self.cal);
                let digest = key.as_deref().map(receipt_digest);
                if digest.is_some() && self.receipts.is_none() { return false; }
                if digest.is_some_and(|digest| self.receipts.as_ref().is_some_and(|journal| journal.contains(notice.at(), digest))) {
                    sex_pdx::serial_record!("[date.notice.receipt.reused] kind={} minute={}", kind, notice.at());
                    return true;
                }
                let disposition = sex_app::notify::post_confirmed_keyed(title.as_bytes(), body.as_bytes(),
                    sex_pdx::notify::URGENCY_CRITICAL, key.as_deref().map(str::as_bytes));
                let settled = disposition.settles_due() && digest.map_or(true, |digest|
                    self.receipts.as_mut().is_some_and(|journal| journal.record_after_ack(notice.at(), digest,
                        |text| save_file(window, NOTICE_RECEIPTS, text.as_bytes()).is_ok())));
                sex_pdx::serial_record!("[date.alarm.fired] kind={} title={} notified={} settled={}", kind, title,
                    (disposition == sex_pdx::notify::Disposition::Accepted) as u8, settled as u8);
                if settled && disposition == sex_pdx::notify::Disposition::Accepted { banner = Some(format!("{title}: {body}")); }
                settled
            });
            if delivery.failed { self.notice_retry.failed(sex_pdx::get_ticks()); } else { self.notice_retry.succeeded(); }
            rang |= !delivery.delivered.is_empty();
            if banner.is_some() { self.banner = banner; }
            advanced = delivery.cursor > self.last_minute;
            self.last_minute = delivery.cursor;
        }
        let today = self.clock.home_zone().local(minute).0;
        if today != self.today { self.today = today; rang = true; }
        let clock_saved = if rang { self.save_clock(window) } else { true };
        if advanced && clock_saved {
            let cursor = NoticeCursor { minute: self.last_minute }.serialize();
            match save_file(window, NOTICE_CURSOR, cursor.as_bytes()) {
                Ok(()) => sex_pdx::serial_record!("[date.notice.cursor.saved] minute={}", self.last_minute),
                Err(e) => sex_pdx::serial_record!("[date.notice.cursor.failed] status={}", e),
            }
        }
        rang
    }
    fn typed_done(&mut self, window: &mut Window, ask: Ask) {
        let text = String::from(self.input.trim());
        self.input.clear();
        if text.is_empty() && ask != Ask::Location { return; }
        match ask {
            Ask::Event => {
                let mut e = quick_event(self.selected, &text);
                e.uid = self.new_uid();
                e.calendar = self.target;
                if !self.cal.info(self.target).url.is_empty() { e.state = SyncState::New; }
                sex_pdx::serial_record!("[calendar.added] date={} when={} title={}", e.start, when(&e), e.title);
                self.status = format!("Added \"{}\" on {}.", e.title, e.start);
                let index = self.cal.events.len();
                self.cal.events.push(e.clone());
                self.event_history.push(EventEdit { index, before: None, after: Some(e) });
                self.save(window);
            }
            Ask::CopyDate | Ask::MoveDate => match Date::parse(&text) {
                Some(day) if ask == Ask::CopyDate => self.copy_to(window, day),
                Some(day) => self.move_to(window, day),
                None => self.status = String::from("Enter a date as YYYY-MM-DD."),
            },
            Ask::Title => self.retitle(window, &text),
            Ask::Location => self.relocate(window, &text),
            Ask::Time => self.retime(window, &text),
            Ask::Alarm => {
                let (time, label) = text.split_once(' ').unwrap_or((&text, ""));
                match clock::parse_time(time) {
                    Some(minute) => {
                        let before = ClockState::from_clock(&self.clock);
                        self.clock.alarms.push(Alarm { minute, days: 0, enabled: true, label: label.trim().into() });
                        self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Alarm);
                        sex_pdx::serial_record!("[date.alarm.added] time={:02}:{:02} label={}", minute / 60, minute % 60, label.trim());
                        self.status = format!("Alarm at {:02}:{:02}.", minute / 60, minute % 60);
                        self.save_clock(window);
                    }
                    None => self.status = String::from("An alarm starts with its time, as 07:30."),
                }
            }
            Ask::Timer => {
                let (length, label) = text.split_once(' ').unwrap_or((&text, ""));
                match clock::parse_duration(length) {
                    Some(seconds) if self.now_ms > 0 => {
                        let before = ClockState::from_clock(&self.clock);
                        self.clock.timers.push(Timer { ends_ms: self.now_ms + seconds as u64 * 1000, seconds, label: label.trim().into() });
                        self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Timer);
                        sex_pdx::serial_record!("[date.timer.started] seconds={} label={}", seconds, label.trim());
                        self.status = format!("Timer: {}.", clock::format_duration(seconds as u64));
                        self.save_clock(window);
                    }
                    Some(_) => self.status = String::from("This machine has no clock for a timer."),
                    None => self.status = String::from("A timer starts with its length, as 5m or 90s."),
                }
            }
            Ask::Zone | Ask::Home => match tz::find(&text) {
                Some(z) => {
                    let before = ClockState::from_clock(&self.clock);
                    if ask == Ask::Home { self.clock.home = z.name.into(); } else if !self.clock.zones.iter().any(|n| n == z.name) {
                        if self.clock.zones.is_empty() { self.clock.zones = self.clock.world().iter().map(|z| String::from(z.name)).collect(); }
                        self.clock.zones.push(z.name.into());
                    }
                    self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Zone);
                    sex_pdx::serial_record!("[date.zone] zone={} home={}", z.name, (ask == Ask::Home) as u8);
                    self.status = format!("{} ({})", z.city, z.name);
                    self.save_clock(window);
                }
                None => self.status = format!("No zone called {text}."),
            },
            Ask::NewCalendar => {
                self.event_history.clear();
                let id = self.cal.add_calendar(&text, "");
                self.target = id;
                sex_pdx::serial_record!("[date.calendar.added] id={} name={}", id, text);
                self.save(window);
            }
        }
    }
    fn clock_rows(&self) -> usize { self.clock.alarms.len() + self.clock.timers.len() + self.clock.world().len() }
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let typed = sex_app::typed(key);
        if self.reply_history.pending().is_some() {
            match (key.control, key.code) {
                (false, 0x1C) => self.commit_answer(window),
                (false, 0x01) | (true, 0x2C) => {
                    self.reply_history.cancel();
                    self.status = String::from("Cancelled local invitation reply. Ctrl+Y restores the choice.");
                    sex_pdx::serial_record!("[date.invite.preview.undo]");
                }
                _ => {},
            }
            return;
        }
        if self.reply_history.has_redo() {
            if key.control && key.code == 0x15 {
                self.reply_history.redo();
                self.status = String::from("Restored local invitation reply choice. Enter queues it.");
                sex_pdx::serial_record!("[date.invite.preview.redo]");
                return;
            }
            self.reply_history.clear();
        }
        if self.review.is_some() {
            match (key.code, typed) {
                (0x01, _) if self.review_choice.is_some() => { self.review_choice = None; self.status = String::from("Choice cancelled; both copies remain."); },
                (0x01, _) => self.review = None,
                (_, Some(b'v')) => self.review_next(),
                (_, Some(b'l')) => {
                    if self.review.and_then(|(index, _, _)| conflicts::server_copy(&self.cal, index)).is_some() {
                        self.review_choice = Some(conflicts::Choice::Local);
                        self.status.clear();
                        sex_pdx::serial_record!("[date.conflict.choice] local");
                    } else { self.status = String::from("No unique server event matches; choose S to discard the local note."); }
                }
                (_, Some(b's')) => { self.review_choice = Some(conflicts::Choice::Server); self.status.clear(); sex_pdx::serial_record!("[date.conflict.choice] server"); },
                (0x1C, _) => self.resolve_review(window),
                _ => {},
            }
            return;
        }
        if self.help {
            match (key.code, typed) {
                (0x01, _) | (_, Some(b'?')) => { self.help = false; self.help_offset = 0; },
                (0x48 | 0x67, _) => self.help_offset = self.help_offset.saturating_sub(1),
                (0x50 | 0x6c, _) => self.help_offset = (self.help_offset + 1).min(views::HELP_ROWS.len().saturating_sub(1)),
                (0x49 | 0x68, _) => self.help_offset = self.help_offset.saturating_sub(8),
                (0x51 | 0x6d, _) => self.help_offset = (self.help_offset + 8).min(views::HELP_ROWS.len().saturating_sub(1)),
                _ => {},
            }
            sex_pdx::serial_record!("[date.help] open={} offset={}", self.help as u8, self.help_offset);
            return;
        }
        if self.ask.is_none() && typed == Some(b'?') {
            self.help = true; self.help_offset = 0;
            sex_pdx::serial_record!("[date.help] open=1 offset=0");
            return;
        }
        if let Some(ask) = self.ask {
            match key.code {
                0x1C => { self.ask = None; self.typed_done(window, ask); }
                0x01 => { self.ask = None; self.input.clear(); }
                0x0E => { self.input.pop(); }
                _ => if let Some(c) = typed { if self.input.len() < 120 { self.input.push(c as char); } },
            }
            return;
        }
        if key.control && (key.code == 0x2C || key.code == 0x15) {
            let redo = key.code == 0x15 || key.shift;
            if self.view == View::Clock {
                let changed = if redo { self.clock_history.redo_at(&mut self.clock, now_ms()) }
                    else { self.clock_history.undo_at(&mut self.clock, now_ms()) };
                if let Some(kind) = changed {
                    self.picked = self.picked.min(self.clock_rows().saturating_sub(1));
                    self.status = format!("{} {} change.", if redo { "Redid" } else { "Undid" }, match kind {
                        ClockEditKind::Zone => "zone", ClockEditKind::Alarm => "alarm", ClockEditKind::Timer => "timer",
                    });
                    self.save_clock(window);
                    match kind {
                        ClockEditKind::Zone => sex_pdx::serial_record!("[date.zone.{}] home={} zones={}", if redo { "redo" } else { "undo" }, self.clock.home, self.clock.zones.len()),
                        ClockEditKind::Alarm => sex_pdx::serial_record!("[date.alarm.{}] alarms={}", if redo { "redo" } else { "undo" }, self.clock.alarms.len()),
                        ClockEditKind::Timer => sex_pdx::serial_record!("[date.timer.{}] timers={}", if redo { "redo" } else { "undo" }, self.clock.timers.len()),
                    }
                } else { self.status = String::from(if redo { "Nothing to redo in Clock." } else { "Nothing to undo in Clock." }); }
                return;
            }
            let changed = if redo { self.event_history.redo(&mut self.cal.events) }
                else { self.event_history.undo(&mut self.cal.events) };
            if changed {
                self.picked = self.picked.min(self.cal.on_day(self.selected).len().saturating_sub(1));
                self.status = String::from(if redo { "Redid event change." } else { "Undid event change." });
                self.save(window);
                sex_pdx::serial_record!("[date.{}] events={}", if redo { "redo" } else { "undo" }, self.cal.events.len());
            } else { self.status = String::from(if redo { "Nothing to redo." } else { "Nothing to undo." }); }
            return;
        }
        if typed == Some(b'v') { self.review_next(); return; }
        if self.agenda {
            match (key.code, sex_app::nav(key), typed) {
                (0x01 | 0x1C, _, _) => self.agenda = false,
                (_, Some(Nav::Up), _) => self.picked = self.picked.saturating_sub(1),
                (_, Some(Nav::Down), _) => self.picked = (self.picked + 1).min(self.cal.on_day(self.selected).len().saturating_sub(1)),
                (_, _, Some(b'd')) => self.copy_tomorrow(window),
                (_, _, Some(b'D')) => { self.ask = Some(Ask::CopyDate); self.input.clear(); },
                (_, _, Some(b'm')) => { self.ask = Some(Ask::MoveDate); self.input.clear(); },
                (_, _, Some(b'e')) => { self.ask = Some(Ask::Title); self.input.clear(); },
                (_, _, Some(b'l')) => { self.ask = Some(Ask::Location); self.input.clear(); },
                (_, _, Some(b't')) => { self.ask = Some(Ask::Time); self.input.clear(); },
                (_, Some(Nav::Delete), _) | (0x6F, _, _) => {
                    let Some(gone) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
                    let Some(index) = self.cal.events.iter().position(|e| *e == gone) else { return };
                    // Removes the whole event, every repeat of it included; one on a
                    // server waits to be deleted there at the next sync.
                    let after = if gone.href.is_empty() {
                        self.cal.events.remove(index);
                        None
                    } else {
                        self.cal.events[index].state = SyncState::Deleted;
                        Some(self.cal.events[index].clone())
                    };
                    self.event_history.push(EventEdit { index, before: Some(gone.clone()), after });
                    sex_pdx::serial_record!("[calendar.removed] date={} title={}", self.selected, gone.title);
                    self.status = format!("Removed \"{}\".", gone.title);
                    self.picked = self.picked.saturating_sub(1);
                    self.save(window);
                }
                (_, _, Some(b'a')) => self.prepare_answer(window, PartStat::Accepted),
                (_, _, Some(b'x')) => self.prepare_answer(window, PartStat::Declined),
                _ => {}
            }
            return;
        }
        // Keys every view shares.
        match typed {
            Some(b'm') => return self.set_view(View::Month),
            Some(b'w') => return self.set_view(View::Week),
            Some(b'd') => return self.set_view(View::Day),
            Some(b'c') => return self.set_view(View::Clock),
            Some(b'l') => return self.set_view(View::Calendars),
            Some(b's') => return self.sync(window),
            Some(b'e') => return self.export(window),
            _ => {}
        }
        match self.view {
            View::Month => match (key.code, sex_app::nav(key), typed) {
                (0x1C, _, _) => self.open_day(),
                (0x49 | 0x68, _, _) => self.select(self.selected.add_months(-1)),
                (0x51 | 0x6D, _, _) => self.select(self.selected.add_months(1)),
                (_, Some(Nav::Left), _) => self.select(self.selected.add_days(-1)),
                (_, Some(Nav::Right), _) => self.select(self.selected.add_days(1)),
                (_, Some(Nav::Up), _) => self.select(self.selected.add_days(-7)),
                (_, Some(Nav::Down), _) => self.select(self.selected.add_days(7)),
                (_, Some(Nav::Home), _) | (_, _, Some(b't')) => self.select(self.today),
                (_, _, Some(b'n')) => { self.ask = Some(Ask::Event); self.input.clear(); }
                _ => {}
            },
            View::Week | View::Day => match (key.code, sex_app::nav(key), typed) {
                (0x1C, _, _) => self.open_day(),
                (0x49 | 0x68, _, _) => self.select(self.selected.add_days(-7)),
                (0x51 | 0x6D, _, _) => self.select(self.selected.add_days(7)),
                (_, Some(Nav::Left), _) => self.select(self.selected.add_days(-1)),
                (_, Some(Nav::Right), _) => self.select(self.selected.add_days(1)),
                (_, Some(Nav::Up), _) => self.first_hour = self.first_hour.saturating_sub(1),
                (_, Some(Nav::Down), _) => self.first_hour = (self.first_hour + 1).min(23),
                (_, Some(Nav::Home), _) | (_, _, Some(b't')) => self.select(self.today),
                (_, _, Some(b'n')) => { self.ask = Some(Ask::Event); self.input.clear(); }
                _ => {}
            },
            View::Clock => {
                let (na, nt) = (self.clock.alarms.len(), self.clock.timers.len());
                match (sex_app::nav(key), typed) {
                    (Some(Nav::Up), _) => self.picked = self.picked.saturating_sub(1),
                    (Some(Nav::Down), _) => self.picked = (self.picked + 1).min(self.clock_rows().saturating_sub(1)),
                    (_, Some(b'n')) => { self.ask = Some(Ask::Alarm); self.input.clear(); }
                    (_, Some(b'r')) => { self.ask = Some(Ask::Timer); self.input.clear(); }
                    (_, Some(b'z')) => { self.ask = Some(Ask::Zone); self.input.clear(); }
                    (_, Some(b'h')) => { self.ask = Some(Ask::Home); self.input.clear(); }
                    (_, Some(b' ')) if self.picked < na => {
                        let before = ClockState::from_clock(&self.clock);
                        let a = &mut self.clock.alarms[self.picked];
                        a.enabled = !a.enabled;
                        self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Alarm);
                        self.save_clock(window);
                    }
                    (Some(Nav::Delete), _) => {
                        let p = self.picked;
                        if p < na {
                            let before = ClockState::from_clock(&self.clock);
                            self.clock.alarms.remove(p);
                            self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Alarm);
                        }
                        else if p < na + nt {
                            let before = ClockState::from_clock(&self.clock);
                            self.clock.timers.remove(p - na);
                            self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Timer);
                        }
                        else {
                            let before = ClockState::from_clock(&self.clock);
                            if self.clock.zones.is_empty() { self.clock.zones = self.clock.world().iter().map(|z| String::from(z.name)).collect(); }
                            if p - na - nt < self.clock.zones.len() { self.clock.zones.remove(p - na - nt); }
                            self.clock_history.push(before, ClockState::from_clock(&self.clock), ClockEditKind::Zone);
                        }
                        self.picked = self.picked.min(self.clock_rows().saturating_sub(1));
                        self.save_clock(window);
                    }
                    _ => {}
                }
            }
            View::Calendars => {
                let all = self.cal.all_calendars();
                match (key.code, sex_app::nav(key), typed) {
                    (_, Some(Nav::Up), _) => self.picked = self.picked.saturating_sub(1),
                    (_, Some(Nav::Down), _) => self.picked = (self.picked + 1).min(all.len().saturating_sub(1)),
                    (0x1C, _, _) | (_, _, Some(b' ')) => if let Some(c) = all.get(self.picked) { self.toggle_calendar(window, c.id) },
                    (_, _, Some(b'n')) => { self.ask = Some(Ask::NewCalendar); self.input.clear(); }
                    _ => {}
                }
                if let Some(c) = self.cal.all_calendars().get(self.picked) {
                    if self.target != c.id { sex_pdx::serial_record!("[date.calendar.target] id={} name={}", c.id, c.name); }
                    self.target = c.id;
                }
            }
        }
    }
    fn toggle_calendar(&mut self, window: &mut Window, id: u16) {
        self.event_history.clear();
        let shown = self.cal.toggle(id);
        sex_pdx::serial_record!("[date.calendar.toggled] id={} visible={}", id, shown as u8);
        self.save(window);
    }
    fn click(&mut self, window: &mut Window, x: i32, y: i32) {
        let l = views::Layout::new(window.size());
        if let Some(tab) = l.tab_at(x, y) {
            match tab { 0 => self.set_view(View::Month), 1 => self.set_view(View::Week), 2 => self.set_view(View::Day), 3 => self.set_view(View::Clock), 4 => self.set_view(View::Calendars), _ => self.sync(window) }
            return;
        }
        match self.view {
            View::Month => {
                let Some(i) = l.cell_at(x, y) else { return };
                let day = MonthGrid::of(self.selected).cell(i);
                if day == self.selected { self.open_day(); } else { self.agenda = false; self.select(day); }
            }
            View::Week | View::Day => {
                let Some((col, hour)) = l.hour_at(x, y, if self.view == View::Week { 7 } else { 1 }, self.first_hour) else { return };
                let day = if self.view == View::Week { views::week_start(self.selected).add_days(col as i64) } else { self.selected };
                if day == self.selected && hour.is_none() { self.open_day(); } else { self.agenda = false; self.select(day); }
            }
            View::Calendars => {
                let Some(row) = l.row_at(y) else { return };
                if let Some(c) = self.cal.all_calendars().get(row) {
                    self.picked = row;
                    self.target = c.id;
                    if x < l.grid_x + 40 { let id = c.id; self.toggle_calendar(window, id); }
                }
            }
            View::Clock => if let Some(row) = l.row_at(y) { self.picked = row.min(self.clock_rows().saturating_sub(1)) },
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut worker_args = [0u8; sex_pdx::fs::ARGS_MAX];
    let worker_len = files::direct::args(&mut worker_args);
    if &worker_args[..worker_len] == NOTICE_WORKER_ARG { notice_worker(); }
    let _ = files::direct::create(NOTICE_UI);
    let _ = files::direct::write_at(NOTICE_UI, 0, &now_ms().to_le_bytes());
    ensure_notice_worker();
    let mut window = Window::open();
    window.title(b"Date");
    let ui_name = notice_ui_name(window.task);
    let _ = files::direct::create(ui_name.as_bytes());
    let _ = files::direct::write_at(ui_name.as_bytes(), 0, &now_ms().to_le_bytes());
    let _ = files::direct::remove(NOTICE_UI);
    ensure_notice_worker();
    let mut faces = views::Faces { small: Face::new(10, 20), tiny: Face::new(8, 16), big: Face::new(16, 30) };
    let now = now_ms();
    let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
    let (cal, saved_events) = match files::read(&mut window, FILE, store) {
        Ok(n) => (Calendar::parse(&String::from_utf8_lossy(&store[..n])), true),
        Err(_) => (Calendar::default(), false),
    };
    let (clock, saved_clock) = match files::read(&mut window, CLOCK, store) {
        Ok(n) => (Clock::parse(&String::from_utf8_lossy(&store[..n])), true),
        Err(_) => (Clock::default(), false),
    };
    let (cursor, cursor_ready) = match files::read(&mut window, NOTICE_CURSOR, store) {
        Ok(n) => {
            let raw = String::from_utf8_lossy(&store[..n]);
            let had_valid = NoticeCursor::decode(&raw, now).is_some();
            let result = NoticeCursor::bootstrap(Some(&raw), now, saved_events || saved_clock,
                |initial| save_file(&mut window, NOTICE_CURSOR, initial.serialize().as_bytes()).is_ok());
            if !had_valid && result.1 {
                sex_pdx::serial_record!("[date.notice.cursor.initialized] owner=visible minute={}", result.0.minute);
            }
            result
        }
        Err(e) if e == files::NOT_FOUND => {
            let result = NoticeCursor::bootstrap(None, now, saved_events || saved_clock,
                |initial| save_file(&mut window, NOTICE_CURSOR, initial.serialize().as_bytes()).is_ok());
            if result.1 {
                sex_pdx::serial_record!("[date.notice.cursor.initialized] owner=visible minute={}", result.0.minute);
            }
            result
        }
        Err(_) => NoticeCursor::bootstrap(None, now, saved_events || saved_clock, |_| false),
    };
    if !cursor_ready { sex_pdx::serial_record!("[date.notice.cursor.failed] status=initial"); }
    let current_minute = NoticeCursor::current(now).minute;
    sex_pdx::serial_record!("[date.notice.cursor.loaded] minute={} now={} catchup={}",
        cursor.minute, current_minute, (cursor.minute < current_minute) as u8);
    sex_pdx::serial_record!("[calendar.loaded] events={}", cal.events.len());
    // Today as the kernel's civil clock has it, in the local zone; a machine without
    // one starts where the program was written.
    let minute = current_minute;
    let today = if now == 0 { Date { year: 2026, month: 1, day: 1 } } else { clock.home_zone().local(minute).0 };
    let theme = settings::theme();
    let palette = views::Palette::from_tokens(theme);
    let theme_generation = settings::generation().unwrap_or(0);
    sex_pdx::serial_record!("[date.theme] mode={:?} accent={:06x}", theme.mode, palette.accent);
    let receipts = match files::read(&mut window, NOTICE_RECEIPTS, store) {
        Ok(n) => ReceiptJournal::open(Some(&String::from_utf8_lossy(&store[..n]))),
        Err(e) if e == files::NOT_FOUND => ReceiptJournal::open(None),
        Err(_) => None,
    };
    sex_pdx::serial_record!("[date.notice.receipts.loaded] owner=visible count={}", receipts.as_ref().map_or(0, ReceiptJournal::len));
    let mut app = App { cal, event_history: EventHistory::default(), reply_history: ReplyDraftHistory::default(), clock_history: ClockHistory::default(), clock, today, selected: today, view: View::Month, palette, theme_generation,
        help: false, help_offset: 0, review: None, review_choice: None, agenda: false, ask: None, input: String::new(), status: String::new(), banner: None,
        picked: 0, first_hour: 7, target: 0, now_ms: now, last_minute: cursor.minute, cursor_ready, notice_retry: NoticeRetry::default(), receipts, uid_counter: 0 };
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let mut path_copy = [0u8; sex_pdx::fs::ARGS_MAX];
    let mut paths: Vec<(usize, usize)> = Vec::new();
    let mut at = 0;
    for word in given.split(|b| *b == b' ') {
        if word.first() == Some(&b'@') { path_copy[at..at + word.len()].copy_from_slice(word); paths.push((at, word.len())); at += word.len(); }
    }
    for (start, len) in paths { app.import(&mut window, &path_copy[start..start + len]); }
    views::render(&mut window, &app, &mut faces);
    sex_pdx::serial_record!("[calendar.ready] target={} task={} today={} month={:04}-{:02}", window.target, window.task, today, today.year, today.month);
    let mut shown_second = now / 1000;
    let mut theme_next = sex_pdx::get_ticks().saturating_add(1000);
    let mut lease_next = sex_pdx::get_ticks().saturating_add(500);
    let mut notice_cadence = NoticeCadence::new(sex_pdx::get_ticks());
    let mut notice_owner = None;
    let mut seen_events = String::new();
    let mut seen_clock = String::new();
    loop {
        let ticks = sex_pdx::get_ticks();
        if ticks >= lease_next {
            lease_next = ticks.saturating_add(500);
            let _ = files::direct::write_at(ui_name.as_bytes(), 0, &now_ms().to_le_bytes());
        }
        if ticks >= theme_next {
            theme_next = ticks.saturating_add(1000);
            if let Ok(generation) = settings::generation() {
                if generation != app.theme_generation {
                    app.theme_generation = generation;
                    let theme = settings::theme();
                    app.palette = views::Palette::from_tokens(theme);
                    sex_pdx::serial_record!("[date.theme] mode={:?} accent={:06x}", theme.mode, app.palette.accent);
                    views::render(&mut window, &app, &mut faces);
                }
            }
        }
        if notice_cadence.due(ticks) {
            let rang = poll_visible_notices(&mut window, &mut app, store,
                &mut seen_events, &mut seen_clock, &mut notice_owner);
            let second = app.now_ms / 1000;
            if rang || (app.view == View::Clock && second != shown_second) {
                shown_second = second;
                views::render(&mut window, &app, &mut faces);
            }
        }
        let Some(event) = window.next_within(500) else { continue; };
        match event {
            Event::Close => {
                sex_pdx::serial_record!("[calendar.closed] target={} task={}", window.target, window.task);
                let _ = files::direct::remove(ui_name.as_bytes());
                ensure_notice_worker();
                window.close()
            }
            Event::Key(key) => {
                app.status.clear();
                if app.banner.is_some() && key.shift && !key.control && key.code == 0x1F {
                    app.snooze_notice(&mut window);
                } else {
                    app.banner = None;
                    app.key(&mut window, key);
                }
            }
            Event::Pointer(_) if app.help => continue,
            Event::Pointer(_) if app.reply_history.pending().is_some() => continue,
            Event::Pointer(p) if matches!(p.kind, Kind::Down { button: 0 }) && app.ask.is_none() => { app.banner = None; app.reply_history.clear(); app.click(&mut window, p.x as i32, p.y as i32); }
            Event::Pointer(p) => match p.kind {
                Kind::Wheel { steps } if matches!(app.view, View::Week | View::Day) => { app.reply_history.clear(); app.first_hour = (app.first_hour as i32 - steps as i32).clamp(0, 23) as u16; },
                Kind::Wheel { steps } if app.view == View::Month => { app.reply_history.clear(); app.select(app.selected.add_days(if steps > 0 { -7 } else { 7 })); },
                _ => continue,
            },
            Event::Drop { .. } | Event::DragEnded { .. } | Event::KeyUp(_) => continue,
            Event::Resize { .. } => {}
        }
        views::render(&mut window, &app, &mut faces);
    }
}
