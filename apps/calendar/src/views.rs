// SPDX-License-Identifier: GPL-3.0-only
//! Drawing Date's views: the month page, the week and day hour grids, the clock
//! (world clock, alarms, timers) and the list of calendars, with the tab row, the
//! side panel of the chosen day and the bottom line.
use crate::draw::{clip, Canvas, Face};
use crate::{when, App, Ask, View};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::Window;
use sex_calendar::{clock, tz, Date, Event as Entry, MonthGrid, PartStat, MONTH_NAMES, WEEKDAY_NAMES};

const BACK: u32 = 0x0017_191E;
const PANEL: u32 = 0x000F_1014;
const CELL: u32 = 0x0022_252C;
const CELL_OUT: u32 = 0x001A_1C21;
const ACCENT: u32 = 0x0035_74E8;
const TODAY: u32 = 0x00E8_6A3A;
const TEXT: u32 = 0x00E8_EAED;
const DIM: u32 = 0x0080_868E;
const SELECT: u32 = 0x002C_3A55;
const LINE: u32 = 0x002A_2E36;

pub struct Faces { pub small: Face, pub tiny: Face, pub big: Face }

pub const TABS: [&str; 6] = ["Month", "Week", "Day", "Clock", "Calendars", "Sync"];

/// Where the parts of the window are.
pub struct Layout { pub w: i32, pub h: i32, pub grid_x: i32, pub grid_y: i32, pub cell_w: i32, pub cell_h: i32, pub side_x: i32 }

impl Layout {
    pub const WEEK_W: i32 = 34;
    const TAB_Y: i32 = 40;
    const TAB_H: i32 = 22;
    const TAB_W: i32 = 92;
    pub const HOUR_H: i32 = 40;
    pub const ALL_DAY_H: i32 = 36;
    pub const ROW_H: i32 = 44;
    pub fn new((w, h): (usize, usize)) -> Layout {
        let (w, h) = (w.max(360) as i32, h.max(260) as i32);
        let side_w = (w * 3 / 10).clamp(150, 320);
        let side_x = w - side_w;
        let grid_x = 8 + Self::WEEK_W;
        let grid_y = 96;
        let cell_w = (side_x - grid_x - 8) / 7;
        let cell_h = (h - grid_y - 30) / 6;
        Layout { w, h, grid_x, grid_y, cell_w, cell_h, side_x }
    }
    pub fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        if !(Self::TAB_Y..Self::TAB_Y + Self::TAB_H).contains(&y) || x < 8 { return None; }
        let i = ((x - 8) / Self::TAB_W) as usize;
        (i < TABS.len()).then_some(i)
    }
    pub fn cell_at(&self, x: i32, y: i32) -> Option<usize> {
        if x < self.grid_x || y < self.grid_y { return None; }
        let (c, r) = ((x - self.grid_x) / self.cell_w, (y - self.grid_y) / self.cell_h);
        (c < 7 && r < 6).then_some((r * 7 + c) as usize)
    }
    fn col_w(&self, cols: i32) -> i32 { (self.side_x - self.grid_x - 8) / cols }
    fn hours_y(&self) -> i32 { self.grid_y + Self::ALL_DAY_H }
    fn hours_shown(&self) -> i32 { ((self.h - 30 - self.hours_y()) / Self::HOUR_H).max(1) }
    /// The column and hour (None: the all-day strip) under the pointer.
    pub fn hour_at(&self, x: i32, y: i32, cols: i32, first_hour: u16) -> Option<(usize, Option<u16>)> {
        if x < self.grid_x || y < self.grid_y || x >= self.grid_x + self.col_w(cols) * cols { return None; }
        let col = ((x - self.grid_x) / self.col_w(cols)) as usize;
        if y < self.hours_y() { return Some((col, None)); }
        let hour = first_hour as i32 + (y - self.hours_y()) / Self::HOUR_H;
        (hour < 24).then_some((col, Some(hour as u16)))
    }
    pub fn row_at(&self, y: i32) -> Option<usize> { (y >= self.grid_y).then(|| ((y - self.grid_y) / Self::ROW_H) as usize) }
}

/// The Monday on or before `day`.
pub fn week_start(day: Date) -> Date { day.add_days(-(day.weekday() as i64)) }

fn color_of(app: &App, e: &Entry) -> u32 { app.cal.calendars.iter().find(|c| c.id == e.calendar).map_or_else(|| app.cal.info(e.calendar).color, |c| c.color) }
fn dim(color: u32) -> u32 { (color >> 2) & 0x003F_3F3F | 0x0010_1010 }

pub fn render(window: &mut Window, app: &App, f: &mut Faces) {
    let l = Layout::new(window.size());
    let (w, h) = (l.w as usize, l.h as usize);
    let mut c = Canvas { px: window.pixels(), w, h };
    c.fill(0, 0, l.w, l.h, BACK);
    let title = match app.view {
        View::Month => format!("{} {}", MONTH_NAMES[app.selected.month as usize - 1], app.selected.year),
        View::Week => { let s = week_start(app.selected); format!("Week {} of {}", s.iso_week().1, s.iso_week().0) }
        View::Day => format!("{} {} {}", WEEKDAY_NAMES[app.selected.weekday() as usize], app.selected.day, MONTH_NAMES[app.selected.month as usize - 1]),
        View::Clock => String::from("Clock"),
        View::Calendars => String::from("Calendars"),
    };
    c.text(&mut f.big, 12, 6, &title, TEXT, l.side_x - 20);
    let current = match app.view { View::Month => 0, View::Week => 1, View::Day => 2, View::Clock => 3, View::Calendars => 4 };
    for (i, name) in TABS.iter().enumerate() {
        let x = 8 + i as i32 * Layout::TAB_W;
        if x + Layout::TAB_W > l.side_x { break; }
        c.fill(x + 2, Layout::TAB_Y, Layout::TAB_W - 4, Layout::TAB_H, if i == current { ACCENT } else { CELL });
        c.centred(&mut f.tiny, x + 2, Layout::TAB_Y, Layout::TAB_W - 4, Layout::TAB_H, name, TEXT);
    }
    match app.view {
        View::Month => month(&mut c, app, &l, f),
        View::Week => hours(&mut c, app, &l, f, week_start(app.selected), 7),
        View::Day => hours(&mut c, app, &l, f, app.selected, 1),
        View::Clock => clock_view(&mut c, app, &l, f),
        View::Calendars => calendars(&mut c, app, &l, f),
    }
    side(&mut c, app, &l, f);
    if let Some(b) = &app.banner {
        c.fill(8, 4, l.side_x - 16, 32, TODAY);
        c.text(&mut f.small, 16, 10, b, TEXT, l.side_x - 32);
    }
    let bottom = match app.ask {
        Some(ask) => {
            let what = match ask {
                Ask::Event => format!("New event on {} in {}", app.selected, app.cal.info(app.target).name),
                Ask::Alarm => String::from("New alarm (07:30 label)"),
                Ask::Timer => String::from("New timer (5m label)"),
                Ask::Zone => String::from("Add zone (city or Area/City)"),
                Ask::Home => String::from("Local zone (city or Area/City)"),
                Ask::NewCalendar => String::from("New calendar's name"),
            };
            format!("{what}: {}_", app.input)
        }
        None if !app.status.is_empty() => app.status.clone(),
        None if app.agenda => String::from("Up/Down pick, Delete removes, a accepts, x declines, Esc back"),
        None => String::from(match app.view {
            View::Month => "Enter opens a day, n adds, m w d c l views, s syncs, e exports",
            View::Week | View::Day => "Arrows move, Up/Down hours, n adds, Enter opens the day",
            View::Clock => "n alarm, r timer, z zone, h local zone, Space on/off, Delete removes",
            View::Calendars => "Space shows/hides, n adds a calendar; new events go to the picked one",
        }),
    };
    c.text(&mut f.small, 12, l.h - 26, &bottom, if app.ask.is_some() { TEXT } else { DIM }, l.side_x - 20);
    window.show_pixels(w, h);
}

fn month(c: &mut Canvas, app: &App, l: &Layout, f: &mut Faces) {
    let grid = MonthGrid::of(app.selected);
    for (i, name) in WEEKDAY_NAMES.iter().enumerate() {
        c.centred(&mut f.small, l.grid_x + i as i32 * l.cell_w, l.grid_y - 24, l.cell_w, 20, name, if i >= 5 { DIM } else { TEXT });
    }
    let counts = app.cal.counts(&grid);
    let th = f.tiny.h as i32;
    let titles_fit = ((l.cell_h - 22) / th).max(0) as usize;
    let cell_chars = ((l.cell_w - 10) / f.tiny.w as i32).max(0) as usize;
    for row in 0..6 {
        let y = l.grid_y + row as i32 * l.cell_h;
        c.centred(&mut f.tiny, 8, y + 4, Layout::WEEK_W - 4, th, &format!("W{}", grid.week_number(row)), DIM);
        for col in 0..7 {
            let i = row * 7 + col;
            let day = grid.cell(i);
            let x = l.grid_x + col as i32 * l.cell_w;
            let inside = grid.in_month(day);
            c.fill(x + 1, y + 1, l.cell_w - 2, l.cell_h - 2, if inside { CELL } else { CELL_OUT });
            if day == app.today { c.fill(x + 3, y + 3, f.small.w as i32 * 2 + 6, f.small.h as i32 + 2, TODAY); }
            if day == app.selected { c.outline(x + 1, y + 1, l.cell_w - 2, l.cell_h - 2, 2, ACCENT); }
            c.text(&mut f.small, x + 6, y + 4, &format!("{:2}", day.day), if inside || day == app.today { TEXT } else { DIM }, l.cell_w);
            let n = counts[i] as usize;
            if n == 0 { continue; }
            // As many titles as the cell holds, each in its calendar's colour, then how many more.
            let events = if titles_fit > 0 { app.cal.on_day(day) } else { Vec::new() };
            let shown = if n > titles_fit { titles_fit.saturating_sub(1) } else { n };
            for (k, e) in events.iter().take(shown).enumerate() {
                let ey = y + 22 + k as i32 * th;
                let color = color_of(app, e);
                c.fill(x + 4, ey + 2, 3, th - 4, color);
                c.text(&mut f.tiny, x + 9, ey, clip(&e.title, cell_chars), color, l.cell_w - 10);
            }
            if shown < n {
                let more = format!("+{}", n - shown);
                let at = if titles_fit == 0 { x + l.cell_w - 6 - (more.len() * f.tiny.w) as i32 } else { x + 4 };
                let my = if titles_fit == 0 { y + 6 } else { y + 22 + shown as i32 * f.tiny.h as i32 };
                c.text(&mut f.tiny, at, my, &more, DIM, l.cell_w);
            }
        }
    }
}

/// The week (7 columns from `first`) or a day (1 column) as an hour grid with an
/// all-day strip on top.
fn hours(c: &mut Canvas, app: &App, l: &Layout, f: &mut Faces, first: Date, cols: i32) {
    let col_w = l.col_w(cols);
    let top = l.hours_y();
    let shown = l.hours_shown();
    let first_hour = (app.first_hour as i32).min(24 - shown).max(0);
    for k in 0..cols {
        let day = first.add_days(k as i64);
        let x = l.grid_x + k * col_w;
        let label = if cols == 1 { format!("{} {}", WEEKDAY_NAMES[day.weekday() as usize], day) } else { format!("{} {}", WEEKDAY_NAMES[day.weekday() as usize], day.day) };
        c.fill(x + 1, l.grid_y - 26, col_w - 2, 22, if day == app.today { TODAY } else if day == app.selected { SELECT } else { BACK });
        c.centred(&mut f.small, x, l.grid_y - 25, col_w, 20, &label, TEXT);
        c.fill(x + 1, l.grid_y, col_w - 2, Layout::ALL_DAY_H - 2, CELL_OUT);
        c.fill(x + 1, top, col_w - 2, shown * Layout::HOUR_H, if day == app.selected { SELECT } else { CELL });
    }
    for r in 0..=shown {
        let y = top + r * Layout::HOUR_H;
        c.fill(l.grid_x, y, col_w * cols, 1, LINE);
        if r < shown && first_hour + r < 24 { c.text(&mut f.tiny, 4, y + 2, &format!("{:02}:00", first_hour + r), DIM, Layout::WEEK_W + 4); }
    }
    let (tw, th) = (f.tiny.w as i32, f.tiny.h as i32);
    let chars = |px: i32| ((px - 8) / tw).max(0) as usize;
    for k in 0..cols {
        let day = first.add_days(k as i64);
        let x = l.grid_x + k * col_w;
        let events = app.cal.on_day(day);
        let mut all_day = 0;
        for e in &events {
            let color = color_of(app, e);
            match e.time {
                None => {
                    if all_day < 2 {
                        let y = l.grid_y + 1 + all_day * (th + 1);
                        c.fill(x + 3, y, col_w - 6, th, dim(color));
                        let t = clip(&e.title, chars(col_w - 4));
                        c.text(&mut f.tiny, x + 6, y, t, color, col_w - 8);
                    }
                    all_day += 1;
                }
                Some((a, b)) => {
                    // An event that runs past midnight fills the rest of its first day.
                    let end = if b > a && e.span <= 1 { b } else { 24 * 60 };
                    let (a, end) = (a as i32 - first_hour * 60, end as i32 - first_hour * 60);
                    if end <= 0 || a >= shown * 60 { continue; }
                    let y0 = top + a.max(0) * Layout::HOUR_H / 60;
                    let y1 = (top + end.min(shown * 60) * Layout::HOUR_H / 60).max(y0 + th);
                    c.fill(x + 3, y0 + 1, col_w - 6, y1 - y0 - 2, dim(color));
                    c.fill(x + 3, y0 + 1, 3, y1 - y0 - 2, color);
                    let line = if cols == 1 { format!("{} {}{}", when(e), e.title, if e.location.is_empty() { String::new() } else { format!(" - {}", e.location) }) } else { e.title.clone() };
                    c.text(&mut f.tiny, x + 9, y0 + 2, clip(&line, chars(col_w - 8)), TEXT, col_w - 12);
                    if cols > 1 && y1 - y0 > 2 * th { c.text(&mut f.tiny, x + 9, y0 + 2 + th, &when(e), DIM, col_w - 12); }
                }
            }
        }
        if all_day > 2 { c.text(&mut f.tiny, x + col_w - 26, l.grid_y + 1, &format!("+{}", all_day - 2), DIM, 24); }
        // The time now, as a line across today.
        if day == app.today && app.now_ms > 0 {
            let m = ((app.now_ms / 60_000) as i64 + app.local_offset() as i64).rem_euclid(1440) as i32 - first_hour * 60;
            if (0..shown * 60).contains(&m) { c.fill(x + 1, top + m * Layout::HOUR_H / 60, col_w - 2, 2, TODAY); }
        }
    }
}

fn hhmm(m: u16) -> String { format!("{:02}:{:02}", m / 60, m % 60) }

fn clock_view(c: &mut Canvas, app: &App, l: &Layout, f: &mut Faces) {
    let minute = (app.now_ms / 60_000) as i64;
    let seconds = app.now_ms / 1000 % 60;
    let home = app.clock.home_zone();
    let (day, m) = home.local(minute);
    c.text(&mut f.small, l.grid_x, l.grid_y - 26, &format!("{} {}:{:02}  {} ({} {})", day, hhmm(m), seconds, home.city, tz::format_offset(home.offset(minute)), home.name), TEXT, l.side_x - l.grid_x);
    let mut row = 0usize;
    let width = l.side_x - l.grid_x - 8;
    let line = |c: &mut Canvas, f: &mut Faces, text: &str, sub: &str, color: u32, row: usize| {
        let y = l.grid_y + row as i32 * Layout::ROW_H;
        if y + Layout::ROW_H > l.h - 30 { return; }
        c.fill(l.grid_x, y + 1, width, Layout::ROW_H - 2, if row == app.picked { SELECT } else { CELL });
        c.text(&mut f.small, l.grid_x + 8, y + 3, text, color, width - 16);
        c.text(&mut f.tiny, l.grid_x + 8, y + 3 + f.small.h as i32, sub, DIM, width - 16);
    };
    for a in &app.clock.alarms {
        let days: String = if a.days == 0 { String::from("once") } else { (0..7).filter(|i| a.days & (1 << i) != 0).map(|i| WEEKDAY_NAMES[i]).collect::<Vec<_>>().join(" ") };
        line(c, f, &format!("Alarm {}  {}", hhmm(a.minute), a.label), &format!("{} - {}", days, if a.enabled { "on" } else { "off" }), if a.enabled { TEXT } else { DIM }, row);
        row += 1;
    }
    for t in &app.clock.timers {
        let left = t.ends_ms.saturating_sub(app.now_ms) / 1000;
        line(c, f, &format!("Timer {}  {}", clock::format_duration(left), t.label), &format!("of {}", clock::format_duration(t.seconds as u64)), TODAY, row);
        row += 1;
    }
    for z in app.clock.world() {
        let (zd, zm) = z.local(minute);
        let diff = zd.days() - day.days();
        let rel = match diff { 0 => String::from("today"), 1 => String::from("tomorrow"), -1 => String::from("yesterday"), d => format!("{d:+} days") };
        line(c, f, &format!("{}  {}", hhmm(zm), z.city), &format!("{rel}, UTC{} {}", tz::format_offset(z.offset(minute)), z.name), TEXT, row);
        row += 1;
    }
}

fn calendars(c: &mut Canvas, app: &App, l: &Layout, f: &mut Faces) {
    let width = l.side_x - l.grid_x - 8;
    for (row, info) in app.cal.all_calendars().iter().enumerate() {
        let y = l.grid_y + row as i32 * Layout::ROW_H;
        if y + Layout::ROW_H > l.h - 30 { break; }
        c.fill(l.grid_x, y + 1, width, Layout::ROW_H - 2, if row == app.picked { SELECT } else { CELL });
        // The box: the calendar's colour when shown, an outline when hidden.
        if info.visible { c.fill(l.grid_x + 8, y + 10, 22, 22, info.color); } else { c.outline(l.grid_x + 8, y + 10, 22, 22, 2, info.color); }
        let count = app.cal.events.iter().filter(|e| e.calendar == info.id).count();
        let mark = if info.id == app.target { "  (new events go here)" } else { "" };
        c.text(&mut f.small, l.grid_x + 40, y + 3, &format!("{}{mark}", info.name), if info.visible { TEXT } else { DIM }, width - 48);
        let sub = if info.url.is_empty() { format!("{count} events, on this machine") } else { format!("{count} events, CalDAV {}", info.url) };
        c.text(&mut f.tiny, l.grid_x + 40, y + 3 + f.small.h as i32, &sub, DIM, width - 48);
    }
}

/// The chosen day's events beside the views.
fn side(c: &mut Canvas, app: &App, l: &Layout, f: &mut Faces) {
    c.fill(l.side_x, 0, l.w - l.side_x, l.h, PANEL);
    let side_w = l.w - l.side_x - 16;
    let cols = (side_w / f.small.w as i32) as usize;
    let weekday = WEEKDAY_NAMES[app.selected.weekday() as usize];
    c.text(&mut f.small, l.side_x + 8, 12, &format!("{weekday} {}", app.selected), TEXT, side_w);
    let (year, week) = app.selected.iso_week();
    c.text(&mut f.tiny, l.side_x + 8, 34, &format!("ISO week {week} of {year}"), DIM, side_w);
    let events = app.cal.on_day(app.selected);
    let mut y = 60;
    if events.is_empty() { c.text(&mut f.small, l.side_x + 8, y, "No events", DIM, side_w); }
    for (k, e) in events.iter().enumerate() {
        if y + f.small.h as i32 > l.h - 30 { break; }
        if app.agenda && k == app.picked { c.fill(l.side_x + 4, y - 2, l.w - l.side_x - 8, 76, SELECT); }
        let color = color_of(app, e);
        c.fill(l.side_x + 4, y, 3, f.tiny.h as i32 + f.small.h as i32, color);
        let mut head = when(e);
        if e.rule.is_some() { head.push_str(" (repeats)"); }
        if !e.alarms.is_empty() { head.push_str(" (reminder)"); }
        c.text(&mut f.tiny, l.side_x + 10, y, &head, DIM, side_w);
        y += f.tiny.h as i32;
        c.text(&mut f.small, l.side_x + 10, y, clip(&e.title, cols), color, side_w);
        y += f.small.h as i32;
        if app.agenda {
            let mut detail = e.location.clone();
            if let Some(me) = e.my_attendee(&[]).map(|i| e.attendees[i].partstat) {
                let state = match me { PartStat::NeedsAction => "invitation: a accepts, x declines", PartStat::Accepted => "accepted", PartStat::Declined => "declined", PartStat::Tentative => "maybe" };
                if !detail.is_empty() { detail.push_str(" - "); }
                detail.push_str(state);
            }
            c.text(&mut f.tiny, l.side_x + 10, y, clip(&detail, cols + 2), TEXT, side_w);
            y += f.tiny.h as i32;
            c.text(&mut f.tiny, l.side_x + 10, y, clip(&app.cal.info(e.calendar).name, cols + 2), DIM, side_w);
            y += f.tiny.h as i32;
        } else if e.invitation {
            c.text(&mut f.tiny, l.side_x + 10, y, "invitation", TODAY, side_w);
            y += f.tiny.h as i32;
        }
        y += 6;
    }
}
