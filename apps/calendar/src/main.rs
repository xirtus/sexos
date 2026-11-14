// SPDX-License-Identifier: GPL-3.0-only
//! Calendar: a month at a time, today marked, events kept in the program's own folder
//! (`events.txt`, see sex_calendar::Calendar) and `.ics` files imported with
//! `run calendar @file.ics`. The date arithmetic, repeat rules and importer are
//! sex-calendar's, tested on the host; this file is the window around them.
//!
//! Keys: arrows move a day or a week, PageUp and PageDown a month, Home (or `t`) goes to
//! today, Enter opens the chosen day and Esc goes back, `n` adds an event on the chosen
//! day (start the title with `HH:MM` to give it a time), Delete removes the event
//! picked in an open day. The mouse picks a day; picking it again opens it.
#![no_std]
#![no_main]

extern crate alloc;

mod draw;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use draw::{clip, Canvas, Face};
use sex_app::{files, Event, Nav, Window};
use sex_calendar::{ics, Calendar, Date, Event as Entry, MonthGrid, MONTH_NAMES, WEEKDAY_NAMES};
use sex_pdx::app_pointer::Kind;

const FILE: &[u8] = b"events.txt";
/// The event file as read, and an imported `.ics` file.
static mut STORE: [u8; 256 << 10] = [0; 256 << 10];
static mut IMPORT: [u8; 1 << 20] = [0; 1 << 20];

const BACK: u32 = 0x0017_191E;
const PANEL: u32 = 0x000F_1014;
const CELL: u32 = 0x0022_252C;
const CELL_OUT: u32 = 0x001A_1C21;
const ACCENT: u32 = 0x0035_74E8;
const TODAY: u32 = 0x00E8_6A3A;
const TEXT: u32 = 0x00E8_EAED;
const DIM: u32 = 0x0080_868E;
const EVENT: u32 = 0x007F_B8FF;
const SELECT: u32 = 0x002C_3A55;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[calendar.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, PartialEq)]
enum Mode { Month, Day, Adding }

struct App {
    cal: Calendar,
    today: Date,
    selected: Date,
    mode: Mode,
    /// The title being typed for a new event.
    input: String,
    /// One line at the bottom: what just happened.
    status: String,
    /// The event picked in an open day.
    picked: usize,
}

fn when(e: &Entry) -> String {
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

impl App {
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
    fn select(&mut self, day: Date) {
        let month_changed = (day.year, day.month) != (self.selected.year, self.selected.month);
        self.selected = day;
        self.picked = 0;
        if month_changed { sex_pdx::serial_record!("[calendar.month] month={:04}-{:02}", day.year, day.month); }
    }
    fn open_day(&mut self) {
        self.mode = Mode::Day;
        self.picked = 0;
        let events = self.cal.on_day(self.selected);
        sex_pdx::serial_record!("[calendar.day] date={} events={}", self.selected, events.len());
        for e in &events { sex_pdx::serial_record!("[calendar.event] date={} when={} title={}", self.selected, when(e), e.title); }
    }
    fn import(&mut self, window: &mut Window, path: &[u8]) {
        let input = unsafe { &mut *core::ptr::addr_of_mut!(IMPORT) };
        let name = core::str::from_utf8(path).unwrap_or("?");
        let read = match files::read(window, path, input) {
            Ok(n) if n == input.len() => { self.status = String::from("That file is larger than 1 MiB."); return; }
            Ok(n) => n,
            Err(e) => {
                sex_pdx::serial_record!("[calendar.import.failed] path={} status={}", name, e);
                self.status = format!("Could not read {name}: {}", core::str::from_utf8(files::message(e)).unwrap_or("?"));
                return;
            }
        };
        let text = String::from_utf8_lossy(&input[..read]);
        match ics::import(&text) {
            Ok(events) => {
                let found = events.len();
                let added = self.cal.merge(events);
                sex_pdx::serial_record!("[calendar.imported] events={} added={}", found, added);
                self.status = format!("Imported {found} events from {name} ({added} new).");
                if added > 0 { self.save(window); }
            }
            Err(_) => {
                sex_pdx::serial_record!("[calendar.import.failed] path={} status=not-calendar", name);
                self.status = format!("{name} is not an iCalendar file.");
            }
        }
    }
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let typed = sex_app::typed(key);
        match self.mode {
            Mode::Adding => match key.code {
                0x1C => {
                    self.mode = Mode::Month;
                    if self.input.trim().is_empty() { return; }
                    let e = quick_event(self.selected, &self.input);
                    sex_pdx::serial_record!("[calendar.added] date={} when={} title={}", e.start, when(&e), e.title);
                    self.status = format!("Added \"{}\" on {}.", e.title, e.start);
                    self.cal.events.push(e);
                    self.input.clear();
                    self.save(window);
                }
                0x01 => { self.mode = Mode::Month; self.input.clear(); }
                0x0E => { self.input.pop(); }
                _ => if let Some(c) = typed { if self.input.len() < 120 { self.input.push(c as char); } },
            },
            Mode::Day => match (key.code, sex_app::nav(key)) {
                (0x01 | 0x1C, _) => self.mode = Mode::Month,
                (_, Some(Nav::Up)) => self.picked = self.picked.saturating_sub(1),
                (_, Some(Nav::Down)) => self.picked = (self.picked + 1).min(self.cal.on_day(self.selected).len().saturating_sub(1)),
                (_, Some(Nav::Delete)) | (0x6F, _) => {
                    let Some(gone) = self.cal.on_day(self.selected).get(self.picked).map(|e| (*e).clone()) else { return };
                    // Removes the whole event, every repeat of it included.
                    self.cal.events.retain(|e| *e != gone);
                    sex_pdx::serial_record!("[calendar.removed] date={} title={}", self.selected, gone.title);
                    self.status = format!("Removed \"{}\".", gone.title);
                    self.picked = self.picked.saturating_sub(1);
                    self.save(window);
                }
                _ => {}
            },
            Mode::Month => match (key.code, sex_app::nav(key), typed) {
                (0x1C, _, _) => self.open_day(),
                (0x49 | 0x68, _, _) => self.select(self.selected.add_months(-1)),
                (0x51 | 0x6D, _, _) => self.select(self.selected.add_months(1)),
                (_, Some(Nav::Left), _) => self.select(self.selected.add_days(-1)),
                (_, Some(Nav::Right), _) => self.select(self.selected.add_days(1)),
                (_, Some(Nav::Up), _) => self.select(self.selected.add_days(-7)),
                (_, Some(Nav::Down), _) => self.select(self.selected.add_days(7)),
                (_, Some(Nav::Home), _) | (_, _, Some(b't')) => self.select(self.today),
                (_, _, Some(b'n')) => { self.mode = Mode::Adding; self.input.clear(); }
                _ => {}
            },
        }
    }
}

/// Where the parts of the window are.
struct Layout { w: i32, h: i32, grid_x: i32, grid_y: i32, cell_w: i32, cell_h: i32, side_x: i32 }

impl Layout {
    const WEEK_W: i32 = 34;
    fn new((w, h): (usize, usize)) -> Layout {
        let (w, h) = (w.max(360) as i32, h.max(260) as i32);
        let side_w = (w * 3 / 10).clamp(150, 320);
        let side_x = w - side_w;
        let grid_x = 8 + Self::WEEK_W;
        let grid_y = 72;
        let cell_w = (side_x - grid_x - 8) / 7;
        let cell_h = (h - grid_y - 30) / 6;
        Layout { w, h, grid_x, grid_y, cell_w, cell_h, side_x }
    }
    fn cell_at(&self, x: i32, y: i32) -> Option<usize> {
        if x < self.grid_x || y < self.grid_y { return None; }
        let (c, r) = ((x - self.grid_x) / self.cell_w, (y - self.grid_y) / self.cell_h);
        (c < 7 && r < 6).then_some((r * 7 + c) as usize)
    }
}

fn render(window: &mut Window, app: &App, small: &mut Face, tiny: &mut Face, big: &mut Face) {
    let l = Layout::new(window.size());
    let (w, h) = (l.w as usize, l.h as usize);
    let mut c = Canvas { px: window.pixels(), w, h };
    c.fill(0, 0, l.w, l.h, BACK);
    let grid = MonthGrid::of(app.selected);
    let title = format!("{} {}", MONTH_NAMES[grid.month as usize - 1], grid.year);
    c.text(big, 12, 10, &title, TEXT, l.side_x - 160);
    c.text(small, l.side_x - 150, 18, "PgUp PgDn Home", DIM, 150);
    for (i, name) in WEEKDAY_NAMES.iter().enumerate() {
        c.centred(small, l.grid_x + i as i32 * l.cell_w, l.grid_y - 24, l.cell_w, 20, name, if i >= 5 { DIM } else { TEXT });
    }
    let counts = app.cal.counts(&grid);
    let titles_fit = ((l.cell_h - 22) / tiny.h as i32).max(0) as usize;
    let cell_chars = ((l.cell_w - 6) / tiny.w as i32).max(0) as usize;
    for row in 0..6 {
        let y = l.grid_y + row as i32 * l.cell_h;
        c.centred(tiny, 8, y + 4, Layout::WEEK_W - 4, tiny.h as i32, &format!("W{}", grid.week_number(row)), DIM);
        for col in 0..7 {
            let i = row * 7 + col;
            let day = grid.cell(i);
            let x = l.grid_x + col as i32 * l.cell_w;
            let inside = grid.in_month(day);
            c.fill(x + 1, y + 1, l.cell_w - 2, l.cell_h - 2, if inside { CELL } else { CELL_OUT });
            if day == app.today {
                c.fill(x + 3, y + 3, small.w as i32 * 2 + 6, small.h as i32 + 2, TODAY);
            }
            if day == app.selected { c.outline(x + 1, y + 1, l.cell_w - 2, l.cell_h - 2, 2, ACCENT); }
            c.text(small, x + 6, y + 4, &format!("{:2}", day.day), if inside || day == app.today { TEXT } else { DIM }, l.cell_w);
            let n = counts[i] as usize;
            if n == 0 { continue; }
            // As many titles as the cell holds, then how many more.
            let events = if titles_fit > 0 { app.cal.on_day(day) } else { Vec::new() };
            let shown = if n > titles_fit { titles_fit.saturating_sub(1) } else { n };
            for (k, e) in events.iter().take(shown).enumerate() {
                c.text(tiny, x + 4, y + 22 + k as i32 * tiny.h as i32, clip(&e.title, cell_chars), EVENT, l.cell_w - 6);
            }
            if shown < n {
                let more = format!("+{}", n - shown);
                let at = if titles_fit == 0 { x + l.cell_w - 6 - (more.len() * tiny.w) as i32 } else { x + 4 };
                let my = if titles_fit == 0 { y + 6 } else { y + 22 + shown as i32 * tiny.h as i32 };
                c.text(tiny, at, my, &more, EVENT, l.cell_w);
            }
        }
    }
    // The chosen day's events beside the month.
    c.fill(l.side_x, 0, l.w - l.side_x, l.h, PANEL);
    let side_w = l.w - l.side_x - 16;
    let cols = (side_w / small.w as i32) as usize;
    let weekday = WEEKDAY_NAMES[app.selected.weekday() as usize];
    c.text(small, l.side_x + 8, 12, &format!("{weekday} {}", app.selected), TEXT, side_w);
    let (year, week) = app.selected.iso_week();
    c.text(tiny, l.side_x + 8, 34, &format!("ISO week {week} of {year}"), DIM, side_w);
    let events = app.cal.on_day(app.selected);
    let mut y = 60;
    if events.is_empty() { c.text(small, l.side_x + 8, y, "No events", DIM, side_w); }
    for (k, e) in events.iter().enumerate() {
        if y + small.h as i32 > l.h - 30 { break; }
        if app.mode == Mode::Day && k == app.picked { c.fill(l.side_x + 4, y - 2, l.w - l.side_x - 8, if app.mode == Mode::Day { 60 } else { 40 }, SELECT); }
        c.text(tiny, l.side_x + 8, y, &when(e), DIM, side_w);
        y += tiny.h as i32;
        c.text(small, l.side_x + 8, y, clip(&e.title, cols), EVENT, side_w);
        y += small.h as i32;
        if app.mode == Mode::Day {
            if !e.location.is_empty() { c.text(tiny, l.side_x + 8, y, clip(&e.location, cols + 2), TEXT, side_w); }
            y += tiny.h as i32;
        }
        y += 6;
    }
    // The bottom line: what is being typed, or what just happened, or how to use it.
    let bottom = match app.mode {
        Mode::Adding => format!("New event on {}: {}_", app.selected, app.input),
        _ if !app.status.is_empty() => app.status.clone(),
        Mode::Day => String::from("Up/Down pick, Delete removes, Esc back"),
        Mode::Month => String::from("Enter opens a day, n adds an event, t today"),
    };
    c.text(small, 12, l.h - 26, &bottom, if app.mode == Mode::Adding { TEXT } else { DIM }, l.side_x - 20);
    window.show_pixels(w, h);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Calendar");
    let mut small = Face::new(10, 20);
    let mut tiny = Face::new(8, 16);
    let mut big = Face::new(16, 30);
    // Today as the kernel's civil clock has it (UTC); a machine without one starts
    // where the program was written.
    let today = sex_pdx::civil_time_ms().map_or(Date { year: 2026, month: 1, day: 1 }, Date::from_unix_ms);
    let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
    let cal = match files::read(&mut window, FILE, store) {
        Ok(n) => Calendar::parse(&String::from_utf8_lossy(&store[..n])),
        Err(_) => Calendar::default(),
    };
    sex_pdx::serial_record!("[calendar.loaded] events={}", cal.events.len());
    let mut app = App { cal, today, selected: today, mode: Mode::Month, input: String::new(), status: String::new(), picked: 0 };
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    if let Some(path) = given.split(|b| *b == b' ').find(|w| w.first() == Some(&b'@')) {
        let mut path_copy = [0u8; sex_pdx::fs::ARGS_MAX];
        path_copy[..path.len()].copy_from_slice(path);
        app.import(&mut window, &path_copy[..path.len()]);
    }
    render(&mut window, &app, &mut small, &mut tiny, &mut big);
    sex_pdx::serial_record!("[calendar.ready] target={} task={} today={} month={:04}-{:02}", window.target, window.task, today, today.year, today.month);
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[calendar.closed] target={} task={}", window.target, window.task); window.close() }
            Event::Key(key) => { app.status.clear(); app.key(&mut window, key); }
            Event::Pointer(p) if matches!(p.kind, Kind::Down { button: 0 }) && app.mode != Mode::Adding => {
                let l = Layout::new(window.size());
                let Some(i) = l.cell_at(p.x as i32, p.y as i32) else { continue };
                let day = MonthGrid::of(app.selected).cell(i);
                if day == app.selected { app.open_day(); } else { app.mode = Mode::Month; app.select(day); }
            }
            Event::Pointer(_) => continue,
            Event::Resize { .. } => {}
        }
        render(&mut window, &app, &mut small, &mut tiny, &mut big);
    }
}
