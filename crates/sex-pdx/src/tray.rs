// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Panel tray: the network, volume and battery icons in the bar, the quick
//! settings menu their click opens (volume slider and mute, network, battery,
//! keyboard layout) and the calendar the clock opens. The Shell gathers the
//! state and handles clicks; Display draws icons and menus. Both place the
//! menus with the layouts here, so a click lands on what is drawn.
//!
//! Shell -> Display `OP_TRAY`: arg0 = `Status::wire`, arg1 = the keyboard
//! layout label (up to 8 ASCII bytes, little-endian), arg2 = `View::wire`.

pub const OP_TRAY: u64 = 0x2E4;

/// No Wi-Fi state known (no chip, or no answer yet).
pub const WIFI_UNKNOWN: u8 = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    /// Master volume 0..=100 and mute; `sound` once Bell has answered.
    pub volume: u8,
    pub muted: bool,
    pub sound: bool,
    /// Battery 0..=100, or None without one.
    pub battery: Option<u8>,
    pub charging: bool,
    pub on_ac: Option<bool>,
    /// `sex_pdx::wifi_status` phase, or `WIFI_UNKNOWN`.
    pub wifi: u8,
}

impl Status {
    pub const UNKNOWN: Self = Self { volume: 0, muted: false, sound: false, battery: None, charging: false, on_ac: None, wifi: WIFI_UNKNOWN };
    pub fn wire(&self) -> u64 {
        let (b, has) = match self.battery { Some(p) => (p.min(100) as u64, 1u64), None => (0, 0) };
        let ac = match self.on_ac { Some(v) => 1 | (v as u64) << 1, None => 0 };
        self.volume.min(100) as u64 | (self.muted as u64) << 8 | (self.sound as u64) << 9
            | b << 16 | has << 24 | (self.charging as u64) << 25 | ac << 26 | (self.wifi as u64) << 32
    }
    pub fn decode(w: u64) -> Self {
        Self {
            volume: (w as u8).min(100), muted: w & 1 << 8 != 0, sound: w & 1 << 9 != 0,
            battery: (w & 1 << 24 != 0).then_some(((w >> 16) as u8).min(100)),
            charging: w & 1 << 25 != 0,
            on_ac: (w & 1 << 26 != 0).then_some(w & 1 << 27 != 0),
            wifi: (w >> 32) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Menu { #[default] None, Quick, Calendar }

/// Which menu shows, and for the calendar how many months from today.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View { pub menu: Menu, pub month_offset: i8 }

impl View {
    pub fn wire(&self) -> u64 {
        let m = match self.menu { Menu::None => 0, Menu::Quick => 1, Menu::Calendar => 2 };
        m | (self.month_offset as u8 as u64) << 8
    }
    pub fn decode(w: u64) -> Option<Self> {
        let menu = match w & 0xFF { 0 => Menu::None, 1 => Menu::Quick, 2 => Menu::Calendar, _ => return None };
        Some(Self { menu, month_offset: (w >> 8) as u8 as i8 })
    }
}

pub fn label_wire(label: &[u8]) -> u64 {
    let mut b = [0u8; 8];
    for (d, s) in b.iter_mut().zip(label.iter().filter(|c| c.is_ascii_graphic() || **c == b' ')) { *d = *s; }
    u64::from_le_bytes(b)
}
/// The label bytes in `out`; returns their length.
pub fn label_decode(w: u64, out: &mut [u8; 8]) -> usize {
    *out = w.to_le_bytes();
    out.iter().position(|b| !(b.is_ascii_graphic() || *b == b' ')).unwrap_or(8)
}

pub type Rect = (usize, usize, usize, usize);
fn inside(r: Rect, x: usize, y: usize) -> bool { x >= r.0 && y >= r.1 && x < r.0 + r.2 && y < r.1 + r.3 }

/// The quick settings menu: four rows under the bar at the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quick { pub panel: Rect, pub rows: [Rect; 4], pub slider: Rect, pub mute: Rect }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickHit { Volume(u8), Mute, Network, Battery, Layout, Inside, Outside }

impl Quick {
    pub const ROWS: usize = 4;
    pub fn new(width: usize, bar: usize, scale: usize) -> Option<Self> {
        let s = scale.max(1);
        let (w, pad, row) = (320 * s, 12 * s, 40 * s);
        if width < w + 16 * s { return None; }
        let (x, y) = (width - w - 8 * s, bar + 6 * s);
        let h = 2 * pad + Self::ROWS * row;
        let mut rows = [(0, 0, 0, 0); 4];
        for (i, r) in rows.iter_mut().enumerate() { *r = (x + pad, y + pad + i * row, w - 2 * pad, row); }
        let (rx, ry, rw, rh) = rows[0];
        let mute = (rx + rw - 64 * s, ry + 6 * s, 64 * s, rh - 12 * s);
        let slider = (rx + 84 * s, ry + rh / 2 - 6 * s, rw - 84 * s - 64 * s - 12 * s, 12 * s);
        Some(Self { panel: (x, y, w, h), rows, slider, mute })
    }
    pub fn volume_at(&self, x: usize) -> u8 {
        let (sx, _, sw, _) = self.slider;
        (x.saturating_sub(sx).min(sw) * 100 / sw.max(1)) as u8
    }
    pub fn hit(&self, x: usize, y: usize) -> QuickHit {
        if !inside(self.panel, x, y) { return QuickHit::Outside; }
        if inside(self.mute, x, y) { return QuickHit::Mute; }
        // The slider takes the whole row height around its track.
        let (sx, _, sw, _) = self.slider;
        let (_, ry, _, rh) = self.rows[0];
        if inside((sx.saturating_sub(6), ry, sw + 12, rh), x, y) { return QuickHit::Volume(self.volume_at(x)); }
        if inside(self.rows[1], x, y) { return QuickHit::Network; }
        if inside(self.rows[2], x, y) { return QuickHit::Battery; }
        if inside(self.rows[3], x, y) { return QuickHit::Layout; }
        QuickHit::Inside
    }
}

/// The clock's month calendar: a header with previous/next, weekday names, six weeks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calendar { pub panel: Rect, pub header: Rect, pub prev: Rect, pub next: Rect, pub grid: (usize, usize), pub cell: (usize, usize) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarHit { Prev, Next, Today, Inside, Outside }

impl Calendar {
    pub fn new(width: usize, bar: usize, scale: usize) -> Option<Self> {
        let s = scale.max(1);
        let (cw, ch, pad, head, names) = (36 * s, 28 * s, 12 * s, 32 * s, 24 * s);
        let (w, h) = (7 * cw + 2 * pad, 2 * pad + head + names + 6 * ch);
        if width < w + 16 * s { return None; }
        let (x, y) = (width - w - 8 * s, bar + 6 * s);
        let header = (x + pad, y + pad, 7 * cw, head);
        let prev = (header.0, header.1, head, head);
        let next = (header.0 + header.2 - head, header.1, head, head);
        Some(Self { panel: (x, y, w, h), header, prev, next, grid: (x + pad, y + pad + head + names), cell: (cw, ch) })
    }
    /// The cell for `day` of a month starting on weekday `first` (0 = Monday).
    pub fn day_cell(&self, first: u8, day: u8) -> Rect {
        let i = first as usize + day as usize - 1;
        (self.grid.0 + i % 7 * self.cell.0, self.grid.1 + i / 7 * self.cell.1, self.cell.0, self.cell.1)
    }
    pub fn hit(&self, x: usize, y: usize) -> CalendarHit {
        if !inside(self.panel, x, y) { CalendarHit::Outside }
        else if inside(self.prev, x, y) { CalendarHit::Prev }
        else if inside(self.next, x, y) { CalendarHit::Next }
        else if inside(self.header, x, y) { CalendarHit::Today }
        else { CalendarHit::Inside }
    }
}

pub fn is_leap(year: i64) -> bool { year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) }
pub fn days_in_month(year: i64, month: u8) -> u8 {
    match month { 2 => if is_leap(year) { 29 } else { 28 }, 4 | 6 | 9 | 11 => 30, _ => 31 }
}
/// 0 = Monday .. 6 = Sunday.
pub fn weekday(year: i64, month: u8, day: u8) -> u8 {
    // Days since 1970-01-01 (a Thursday), Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (days + 3).rem_euclid(7) as u8
}
/// The month `offset` months from `year`-`month`.
pub fn shift_month(year: i64, month: u8, offset: i32) -> (i64, u8) {
    let n = year * 12 + month as i64 - 1 + offset as i64;
    (n.div_euclid(12), (n.rem_euclid(12) + 1) as u8)
}
pub const MONTH_NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
pub const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// What the Wi-Fi line of the quick menu says for a phase.
pub fn wifi_text(phase: u8) -> &'static str {
    match phase {
        0 => "Network: wired (no Wi-Fi)",
        1 => "Wi-Fi: starting",
        2 => "Wi-Fi: scanning",
        3 => "Wi-Fi: not connected",
        4 => "Wi-Fi: failed",
        5..=9 => "Wi-Fi: connecting",
        10 => "Wi-Fi: connected",
        _ => "Network: unknown",
    }
}

/// Writes a short line into `out` (no allocation); returns its length.
pub fn battery_text(s: &Status, out: &mut [u8; 32]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8], n: &mut usize| for &c in b { if *n < out.len() { out[*n] = c; *n += 1; } };
    match s.battery {
        Some(p) => {
            put(b"Battery ", &mut n);
            if p >= 100 { put(b"100", &mut n); } else if p >= 10 { put(&[b'0' + p / 10, b'0' + p % 10], &mut n); } else { put(&[b'0' + p], &mut n); }
            put(b"%", &mut n);
            if s.charging { put(b", charging", &mut n); } else if s.on_ac == Some(true) { put(b", plugged in", &mut n); }
        }
        None => put(if s.on_ac == Some(false) { b"On battery" } else { b"On AC power" }, &mut n),
    }
    n
}
