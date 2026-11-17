// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Desktop widgets (workstream S5): cards drawn on the wallpaper, under every
//! window, the way Plasma's desktop widgets and GNOME's Desktop Clock
//! extension sit. Display draws them as part of the wallpaper, so a window
//! covers them and nothing about them can take focus or input.
//!
//! The Shell turns them on or off (setting `desktop.widgets`, Super+Shift+W)
//! with `OP_DESKTOP_WIDGETS`. Display fills them itself: the clock from the
//! civil time and zone it already keeps for the bar, the system card from the
//! kernel's numbers (syscall 87, which needs no right). The weather card's
//! numbers come from the Shell later (`WEATHER`, reserved).
//!
//! The words and the layout are here, host-tested in `tests/desktop_widgets.rs`.

/// Shell -> Display. arg0 = which cards show ([`CLOCK`] | [`SYSTEM`]); 0 hides all.
pub const OP_DESKTOP_WIDGETS: u64 = 0x2F9;
pub const CLOCK: u64 = 1;
pub const SYSTEM: u64 = 2;
/// Reserved for the weather card (Open-Meteo numbers carried by the Shell).
pub const WEATHER: u64 = 4;
pub const ALL: u64 = CLOCK | SYSTEM;

/// A line of card text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Line { pub bytes: [u8; 40], pub len: usize }

impl Line {
    pub const EMPTY: Line = Line { bytes: [0; 40], len: 0 };
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
    fn push(&mut self, s: &[u8]) { for &b in s { if self.len < self.bytes.len() { self.bytes[self.len] = b; self.len += 1; } } }
    fn num(&mut self, mut v: u64, width: usize) {
        let mut d = [0u8; 20];
        let mut n = 0;
        loop { d[n] = b'0' + (v % 10) as u8; n += 1; v /= 10; if v == 0 { break; } }
        while n < width { d[n] = b'0'; n += 1; }
        for i in (0..n).rev() { self.push(&[d[i]]); }
    }
}

impl core::fmt::Debug for Line {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", core::str::from_utf8(self.as_bytes()).unwrap_or("?"))
    }
}

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Year, month (1..=12) and day of a day count since 1970-01-01 (civil, proleptic Gregorian).
pub fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// The clock card for `local` seconds since 1970 in the person's zone:
/// ("14:05", "Tuesday 17 November"), or "2:05 PM" without the 24-hour clock.
pub fn clock_lines(local: i64, hour24: bool) -> (Line, Line) {
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400) as u64;
    let (h, m) = (secs / 3600, secs / 60 % 60);
    let mut time = Line::EMPTY;
    if hour24 { time.num(h, 2); } else { time.num(if h % 12 == 0 { 12 } else { h % 12 }, 1); }
    time.push(b":");
    time.num(m, 2);
    if !hour24 { time.push(if h < 12 { b" AM" } else { b" PM" }); }
    let (_, month, day) = civil_date(days);
    let mut date = Line::EMPTY;
    // 1970-01-01 was a Thursday.
    date.push(WEEKDAYS[(days + 3).rem_euclid(7) as usize].as_bytes());
    date.push(b" ");
    date.num(day as u64, 1);
    date.push(b" ");
    date.push(MONTHS[month as usize - 1].as_bytes());
    (time, date)
}

/// One look at the kernel's overview: TSC, cores, idle cycles of all cores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample { pub tsc: u64, pub cores: u64, pub idle: u64 }

/// How busy the processors were between two samples, in percent.
pub fn cpu_percent(before: Sample, now: Sample) -> Option<u8> {
    let span = now.tsc.checked_sub(before.tsc)?.checked_mul(now.cores.max(1))?;
    if span == 0 { return None; }
    let idle = now.idle.checked_sub(before.idle)?.min(span);
    Some((((span - idle) as u128 * 100 + span as u128 / 2) / span as u128).min(100) as u8)
}

/// Bytes as people read them: "512 MB", "1.2 GB".
pub fn size(bytes: u64, out: &mut Line) {
    const MB: u64 = 1 << 20;
    const GB: u64 = 1 << 30;
    if bytes >= GB {
        let tenths = (bytes * 10 + GB / 2) / GB;
        out.num(tenths / 10, 1);
        out.push(b".");
        out.num(tenths % 10, 1);
        out.push(b" GB");
    } else {
        out.num((bytes + MB / 2) / MB, 1);
        out.push(b" MB");
    }
}

/// The system card: ("Processor 12%", "Memory free 1.2 GB"); a dash while unknown.
pub fn system_lines(cpu: Option<u8>, free_pages: Option<u64>) -> (Line, Line) {
    let mut a = Line::EMPTY;
    a.push(b"Processor ");
    match cpu { Some(p) => { a.num(p as u64, 1); a.push(b"%"); } None => a.push(b"-") }
    let mut b = Line::EMPTY;
    b.push(b"Memory free ");
    match free_pages { Some(p) => size(p.saturating_mul(4096), &mut b), None => b.push(b"-") }
    (a, b)
}

/// Where the cards go on a `w`x`h` screen under a `bar`-high bar, at UI
/// `scale`: a column at the top right. (x, y, w, h) of the column and of
/// each shown card, in [`CLOCK`], [`SYSTEM`] order; None when nothing shows
/// or the screen is too small.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub column: (usize, usize, usize, usize),
    pub clock: Option<(usize, usize, usize, usize)>,
    pub system: Option<(usize, usize, usize, usize)>,
    pub scale: usize,
}

pub const CARD_W: usize = 240;
pub const CLOCK_H: usize = 104;
pub const SYSTEM_H: usize = 64;
pub const GAP: usize = 12;
pub const MARGIN: usize = 24;

impl Layout {
    pub fn new(cards: u64, w: usize, h: usize, bar: usize, scale: usize) -> Option<Layout> {
        let s = scale.clamp(1, 2);
        let shown = [(cards & CLOCK != 0, CLOCK_H), (cards & SYSTEM != 0, SYSTEM_H)];
        let n = shown.iter().filter(|c| c.0).count();
        if n == 0 { return None; }
        let height = shown.iter().filter(|c| c.0).map(|c| c.1 * s).sum::<usize>() + GAP * s * (n - 1);
        let (cw, m) = (CARD_W * s, MARGIN * s);
        if w < cw + 2 * m || h < bar + height + 2 * m { return None; }
        let (x, mut y) = (w - m - cw, bar + m);
        let mut place = |on: bool, ch: usize| on.then(|| { let r = (x, y, cw, ch * s); y += ch * s + GAP * s; r });
        let clock = place(shown[0].0, CLOCK_H);
        let system = place(shown[1].0, SYSTEM_H);
        Some(Layout { column: (x, bar + m, cw, height), clock, system, scale: s })
    }
}
