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
//! Month: arrows move a day or a week, PageUp and PageDown a month, Home (or `t`)
//! today, Enter opens the chosen day and Esc goes back, `n` adds an event (start it
//! with `HH:MM` to give it a time). An open day: Up/Down pick, Delete removes, `a`
//! accepts an invitation, `x` declines it. Week and day: Left/Right a day, Up/Down
//! scroll the hours, PageUp/PageDown a week. Clock: `n` new alarm (`07:30 Wake up`),
//! `r` new timer (`5m Tea`), `z` add a zone, `h` set the local zone, Space switches an
//! alarm on or off, Delete removes. Calendars: Up/Down pick, Space shows or hides,
//! `n` adds a local calendar; new events go to the picked one. The mouse picks days,
//! hours, tabs, calendars.
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
use sex_app::{files, Event, Nav, Window};
use sex_calendar::clock::{self, Alarm, Clock, Timer};
use sex_calendar::{ics, tz, Calendar, Date, Event as Entry, MonthGrid, PartStat, SyncState};
use sex_dav::FileAccount;
use sex_pdx::app_pointer::Kind;

const FILE: &[u8] = b"events.txt";
const CLOCK: &[u8] = b"clock.txt";
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
pub enum Ask { Event, Alarm, Timer, Zone, Home, NewCalendar }

pub struct App {
    pub cal: Calendar,
    pub clock: Clock,
    pub today: Date,
    pub selected: Date,
    pub view: View,
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

/// `20261114T101500Z` for DTSTAMP.
fn stamp(ms: u64) -> String {
    let m = (ms / 60_000) as i64;
    let d = Date::from_days(m.div_euclid(1440));
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", d.year, d.month, d.day, m.rem_euclid(1440) / 60, m.rem_euclid(60), ms / 1000 % 60)
}

impl App {
    pub fn local_offset(&self) -> i32 { self.clock.home_zone().offset((self.now_ms / 60_000) as i64) }
    fn save(&mut self, window: &mut Window) {
        let text = self.cal.serialize();
        match files::write(window, FILE, text.as_bytes()) {
            Ok(()) => sex_pdx::serial_record!("[calendar.saved] events={} bytes={}", self.cal.events.len(), text.len()),
            Err(e) => {
                sex_pdx::serial_record!("[calendar.save.failed] status={}", e);
                self.status = String::from(core::str::from_utf8(files::message(e)).unwrap_or("Not saved."));
            }
        }
    }
    fn save_clock(&mut self, window: &mut Window) {
        if let Err(e) = files::write(window, CLOCK, self.clock.serialize().as_bytes()) {
            self.status = String::from(core::str::from_utf8(files::message(e)).unwrap_or("Not saved."));
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
                    let _ = files::write(window, ACCOUNT, text.as_bytes());
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
                    for note in &r.conflicts {
                        for mut e in ics::import(note).unwrap_or_default() {
                            e.calendar = notes;
                            e.title = format!("Conflict: {}", e.title);
                            e.uid = format!("{}-conflict", e.uid);
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
        match result {
            Ok((pulled, pushed, conflicts)) => {
                sex_pdx::serial_record!("[date.sync.done] events={} pulled={} pushed={} conflicts={}", self.cal.events.len(), pulled, pushed, conflicts);
                self.status = format!("Synced: {pulled} from the server, {pushed} to it{}.", if conflicts > 0 { format!(", {conflicts} conflict(s) kept as notes") } else { String::new() });
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
    fn answer(&mut self, window: &mut Window, answer: PartStat) {
        let Some(target) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
        let me = self.me(window);
        let me: Vec<&str> = me.iter().map(String::as_str).collect();
        let Some(e) = self.cal.events.iter_mut().find(|e| **e == target) else { return };
        if !e.respond(&me, answer) { self.status = String::from("No attendee to answer for."); return; }
        let reply = ics::reply(e, &me, &stamp(self.now_ms)).unwrap_or_default();
        let uid = e.uid.clone();
        // Valentine sends replies; until then the reply waits in this folder.
        let name = format!("reply-{}", sex_dav::resource_name(&uid, ".ics"));
        let _ = files::write(window, name.as_bytes(), reply.as_bytes());
        sex_pdx::serial_record!("[date.invite.answered] uid={} partstat={} reply={} bytes={}", uid, answer.as_str(), name, reply.len());
        self.status = format!("{} \"{}\"; the reply is in {name}.", if answer == PartStat::Accepted { "Accepted" } else { "Declined" }, target.title);
        self.save(window);
    }
    fn ring(&mut self, kind: &str, title: &str, body: &str) {
        let posted = sex_pdx::notify::post(title.as_bytes(), body.as_bytes(), sex_pdx::notify::URGENCY_CRITICAL);
        sex_pdx::serial_record!("[date.alarm.fired] kind={} title={} notified={}", kind, title, posted as u8);
        self.banner = Some(if body.is_empty() { String::from(title) } else { format!("{title}: {body}") });
    }
    /// Looks at the clock: reminders, alarms and timers due since last time. Returns
    /// whether anything rang.
    fn tick(&mut self, window: &mut Window) -> bool {
        self.now_ms = now_ms();
        if self.now_ms == 0 { return false; }
        let mut rang = false;
        for t in self.clock.timers_done(self.now_ms) {
            self.ring("timer", if t.label.is_empty() { "Timer" } else { &t.label }, &format!("{} timer done", clock::format_duration(t.seconds as u64)));
            rang = true;
        }
        let minute = (self.now_ms / 60_000) as i64;
        if minute > self.last_minute {
            let offset = self.local_offset();
            for (i, day, at) in self.cal.reminders_due(self.last_minute, minute, offset) {
                let e = &self.cal.events[i];
                let (title, start) = (e.title.clone(), e.start_minute_utc(day, offset));
                let lead = (start - at).max(0);
                let body = if lead == 0 { format!("now, {}", when(e)) } else { format!("in {} min, {} {}", lead, day, when(e)) };
                self.ring("reminder", &title, &body);
                rang = true;
            }
            for a in self.clock.alarms_due(self.last_minute, minute) {
                self.ring("alarm", if a.label.is_empty() { "Alarm" } else { &a.label }, &format!("{:02}:{:02}", a.minute / 60, a.minute % 60));
                rang = true;
            }
            self.last_minute = minute;
            let today = self.clock.home_zone().local(minute).0;
            if today != self.today { self.today = today; rang = true; }
        }
        if rang { self.save_clock(window); }
        rang
    }
    fn typed_done(&mut self, window: &mut Window, ask: Ask) {
        let text = String::from(self.input.trim());
        self.input.clear();
        if text.is_empty() { return; }
        match ask {
            Ask::Event => {
                let mut e = quick_event(self.selected, &text);
                e.uid = self.new_uid();
                e.calendar = self.target;
                if !self.cal.info(self.target).url.is_empty() { e.state = SyncState::New; }
                sex_pdx::serial_record!("[calendar.added] date={} when={} title={}", e.start, when(&e), e.title);
                self.status = format!("Added \"{}\" on {}.", e.title, e.start);
                self.cal.events.push(e);
                self.save(window);
            }
            Ask::Alarm => {
                let (time, label) = text.split_once(' ').unwrap_or((&text, ""));
                match clock::parse_time(time) {
                    Some(minute) => {
                        self.clock.alarms.push(Alarm { minute, days: 0, enabled: true, label: label.trim().into() });
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
                        self.clock.timers.push(Timer { ends_ms: self.now_ms + seconds as u64 * 1000, seconds, label: label.trim().into() });
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
                    if ask == Ask::Home { self.clock.home = z.name.into(); } else if !self.clock.zones.iter().any(|n| n == z.name) {
                        if self.clock.zones.is_empty() { self.clock.zones = self.clock.world().iter().map(|z| String::from(z.name)).collect(); }
                        self.clock.zones.push(z.name.into());
                    }
                    sex_pdx::serial_record!("[date.zone] zone={} home={}", z.name, (ask == Ask::Home) as u8);
                    self.status = format!("{} ({})", z.city, z.name);
                    self.save_clock(window);
                }
                None => self.status = format!("No zone called {text}."),
            },
            Ask::NewCalendar => {
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
        if let Some(ask) = self.ask {
            match key.code {
                0x1C => { self.ask = None; self.typed_done(window, ask); }
                0x01 => { self.ask = None; self.input.clear(); }
                0x0E => { self.input.pop(); }
                _ => if let Some(c) = typed { if self.input.len() < 120 { self.input.push(c as char); } },
            }
            return;
        }
        if self.agenda {
            match (key.code, sex_app::nav(key), typed) {
                (0x01 | 0x1C, _, _) => self.agenda = false,
                (_, Some(Nav::Up), _) => self.picked = self.picked.saturating_sub(1),
                (_, Some(Nav::Down), _) => self.picked = (self.picked + 1).min(self.cal.on_day(self.selected).len().saturating_sub(1)),
                (_, Some(Nav::Delete), _) | (0x6F, _, _) => {
                    let Some(gone) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
                    // Removes the whole event, every repeat of it included; one on a
                    // server waits to be deleted there at the next sync.
                    for e in self.cal.events.iter_mut().filter(|e| **e == gone) { if !e.href.is_empty() { e.state = SyncState::Deleted; } }
                    self.cal.events.retain(|e| *e != gone || e.state == SyncState::Deleted);
                    sex_pdx::serial_record!("[calendar.removed] date={} title={}", self.selected, gone.title);
                    self.status = format!("Removed \"{}\".", gone.title);
                    self.picked = self.picked.saturating_sub(1);
                    self.save(window);
                }
                (_, _, Some(b'a')) => self.answer(window, PartStat::Accepted),
                (_, _, Some(b'x')) => self.answer(window, PartStat::Declined),
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
                        let a = &mut self.clock.alarms[self.picked];
                        a.enabled = !a.enabled;
                        self.save_clock(window);
                    }
                    (Some(Nav::Delete), _) => {
                        let p = self.picked;
                        if p < na { self.clock.alarms.remove(p); }
                        else if p < na + nt { self.clock.timers.remove(p - na); }
                        else {
                            if self.clock.zones.is_empty() { self.clock.zones = self.clock.world().iter().map(|z| String::from(z.name)).collect(); }
                            if p - na - nt < self.clock.zones.len() { self.clock.zones.remove(p - na - nt); }
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
    let mut window = Window::open();
    window.title(b"Date");
    let mut faces = views::Faces { small: Face::new(10, 20), tiny: Face::new(8, 16), big: Face::new(16, 30) };
    let now = now_ms();
    let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
    let cal = match files::read(&mut window, FILE, store) {
        Ok(n) => Calendar::parse(&String::from_utf8_lossy(&store[..n])),
        Err(_) => Calendar::default(),
    };
    let clock = match files::read(&mut window, CLOCK, store) {
        Ok(n) => Clock::parse(&String::from_utf8_lossy(&store[..n])),
        Err(_) => Clock::default(),
    };
    sex_pdx::serial_record!("[calendar.loaded] events={}", cal.events.len());
    // Today as the kernel's civil clock has it, in the local zone; a machine without
    // one starts where the program was written.
    let minute = (now / 60_000) as i64;
    let today = if now == 0 { Date { year: 2026, month: 1, day: 1 } } else { clock.home_zone().local(minute).0 };
    let mut app = App { cal, clock, today, selected: today, view: View::Month, agenda: false, ask: None, input: String::new(), status: String::new(), banner: None,
        picked: 0, first_hour: 7, target: 0, now_ms: now, last_minute: minute, uid_counter: 0 };
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
    loop {
        let Some(event) = window.next_within(500) else {
            let rang = app.tick(&mut window);
            let second = app.now_ms / 1000;
            if rang || (app.view == View::Clock && second != shown_second) {
                shown_second = second;
                views::render(&mut window, &app, &mut faces);
            }
            continue;
        };
        match event {
            Event::Close => { sex_pdx::serial_record!("[calendar.closed] target={} task={}", window.target, window.task); window.close() }
            Event::Key(key) => { app.status.clear(); app.banner = None; app.key(&mut window, key); }
            Event::Pointer(p) if matches!(p.kind, Kind::Down { button: 0 }) && app.ask.is_none() => { app.banner = None; app.click(&mut window, p.x as i32, p.y as i32); }
            Event::Pointer(p) => match p.kind {
                Kind::Wheel { steps } if matches!(app.view, View::Week | View::Day) => app.first_hour = (app.first_hour as i32 - steps as i32).clamp(0, 23) as u16,
                Kind::Wheel { steps } if app.view == View::Month => app.select(app.selected.add_days(if steps > 0 { -7 } else { 7 })),
                _ => continue,
            },
            Event::Resize { .. } => {}
        }
        app.tick(&mut window);
        views::render(&mut window, &app, &mut faces);
    }
}
