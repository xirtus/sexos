// SPDX-License-Identifier: GPL-3.0-only
//! Desktop widgets (slices S5b, S5c): the clock, weather and system cards
//! drawn on the wallpaper, under every window (`sex_pdx::desktop_widgets`).
//! The Shell turns them on or off and passes on the weather reports; the
//! clock comes from the civil time and zone this server keeps for the bar,
//! the system card from the kernel's numbers.
//!
//! The cards are drawn once into `CACHE` whenever a word on them changes (the
//! minute, the date, the weather, the processor load, free memory), and the wallpaper path
//! (`wallpaper_mark::pixel`) blends that cache over the gradient, so windows
//! cover the cards like any wallpaper.

use sex_pdx::desktop_widgets::{self as dw, Layout, Line, Report, Sample};

/// Largest cards kept: scale 2.
const CAP_W: usize = dw::CARD_W * 2;
const CAP_H: usize = (dw::CLOCK_H + dw::GAP + dw::WEATHER_H + dw::GAP + dw::SYSTEM_H) * 2;
/// Each pixel: alpha in the top byte (0: wallpaper shows), colour below.
static mut CACHE: [u32; CAP_W * CAP_H] = [0; CAP_W * CAP_H];

struct State {
    cards: u64,
    layout: Option<Layout>,
    /// What the cache shows: the screen it was laid out for and its words.
    drawn: Option<((usize, usize, usize, usize), [Line; 6])>,
    /// The latest weather report and the zone it is for.
    weather: Option<(Report, usize)>,
    sample: Option<Sample>,
    sampled_at: u64,
    cpu: Option<u8>,
    free_pages: Option<u64>,
    receipts: u32,
}
static mut STATE: State = State { cards: 0, layout: None, drawn: None, weather: None, sample: None, sampled_at: 0, cpu: None, free_pages: None, receipts: 64 };
fn st() -> &'static mut State { unsafe { &mut *core::ptr::addr_of_mut!(STATE) } }

const CARD: u32 = 0x0014_121A;
const CARD_ALPHA: u32 = 0xC8;
const RIM: u32 = 0x0046_3E4C;
const INK: u32 = 0x00F4_EEF2;
const SOFT: u32 = 0x00C2_B6C0;

/// `OP_DESKTOP_WIDGETS` from the Shell. True when the cards changed (the
/// whole frame repaints, so they appear or go).
pub fn receive(caller: u32, cards: u64) -> bool {
    if caller != 3 || cards & !dw::ALL != 0 { return false; }
    let s = st();
    if s.cards == cards { return false; }
    s.cards = cards;
    s.layout = None;
    s.drawn = None;
    sex_pdx::serial_record!("[display.widgets] cards={}", cards);
    true
}

/// `OP_WEATHER` from the Shell: the helper's report for a zone. The next
/// tick draws it.
pub fn weather(caller: u32, report: u64, zone: u64) {
    if caller != 3 { return; }
    let Some(r) = Report::unpack(report) else { return };
    st().weather = Some((r, zone as usize));
    sex_pdx::serial_record!("[display.widgets.weather] zone={} report={:?}", zone, r);
}

/// The column's rows and columns, if cards show.
pub fn span(y: usize) -> Option<(usize, usize)> {
    let (x, top, w, h) = st().layout?.column;
    (st().drawn.is_some() && y >= top && y < top + h).then_some((x, x + w))
}

/// The wallpaper pixel `back` at (x, y) with the cards over it.
#[inline]
pub fn over(x: usize, y: usize, back: u32) -> u32 {
    let s = st();
    let (Some(l), Some(_)) = (s.layout, s.drawn) else { return back };
    let (cx, cy, cw, ch) = l.column;
    if x < cx || y < cy || x >= cx + cw || y >= cy + ch { return back; }
    let p = unsafe { CACHE[(y - cy) * cw + (x - cx)] };
    let a = (p >> 24) as u8;
    if a == 0 { back } else { super::coverage_blend(p & 0x00FF_FFFF, back, a) | (back & 0xFF00_0000) }
}

/// Once per main loop turn: new words redraw the cache. Returns the column
/// to repaint when it changed.
pub fn tick(clock: &silkbar_model::civil_clock::CivilClock, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let s = st();
    if s.cards == 0 { return None; }
    let Some(layout) = Layout::new(s.cards, w, h, super::bar_height(), sex_pdx::desktop_style::ui_scale() as usize) else {
        // No longer fits (a smaller screen or a larger scale): take the old cards away.
        let old = s.layout.take().map(|l| l.column);
        s.drawn = None;
        return old;
    };
    let now = sex_pdx::get_ticks();
    if s.cards & dw::SYSTEM != 0 && (s.sample.is_none() || now.saturating_sub(s.sampled_at) >= 2_000) {
        if let Some(o) = sex_pdx::system_stats::overview() {
            let next = Sample { tsc: o.tsc, cores: o.cores, idle: o.idle_cycles };
            if let Some(before) = s.sample { s.cpu = dw::cpu_percent(before, next).or(s.cpu); }
            s.sample = Some(next);
            s.free_pages = Some(o.free_pages);
        }
        s.sampled_at = now;
    }
    let (time, date) = if clock.state() == silkbar_model::civil_clock::State::Snapshot {
        let (offset, _) = sex_time::zone::zone(sex_pdx::clock_zone::zone()).offset_at(clock.seconds() as i64);
        dw::clock_lines(clock.seconds() as i64 + offset as i64, true)
    } else {
        let mut t = Line::EMPTY; t.bytes[..5].copy_from_slice(b"--:--"); t.len = 5;
        let mut d = Line::EMPTY; d.bytes[..12].copy_from_slice(b"Date unknown"); d.len = 12;
        (t, d)
    };
    let zone = sex_pdx::clock_zone::zone();
    // A report for another zone is old news: the helper is asking about the new one.
    let report = s.weather.filter(|w| w.1 == zone).map(|w| w.0);
    let (sky, temps) = dw::weather_lines(report, sex_time::zone::zone(zone).name);
    let (cpu, free) = dw::system_lines(s.cpu, s.free_pages);
    let words = [time, date, cpu, free, sky, temps];
    let key = (w, h, layout.scale, s.cards as usize);
    if s.layout == Some(layout) && s.drawn.as_ref().is_some_and(|d| d.0 == key && d.1 == words) { return None; }
    let moved = s.layout.map(|l| l.column).filter(|c| *c != layout.column);
    s.layout = Some(layout);
    draw(&layout, &words);
    s.drawn = Some((key, words));
    if s.receipts > 0 {
        s.receipts -= 1;
        let (x, y, cw, ch) = layout.column;
        sex_pdx::serial_record!("[display.widgets.draw] x={} y={} w={} h={} time={:?} date={:?} cpu={:?} free={:?} sky={:?} temps={:?}", x, y, cw, ch, time, date, cpu, free, sky, temps);
    }
    // A column that moved (new scale) repaints where it was as well.
    Some(match moved { Some(old) => union(old, layout.column), None => layout.column })
}

fn union(a: (usize, usize, usize, usize), b: (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
    let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
    let (x1, y1) = ((a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
    (x0, y0, x1 - x0, y1 - y0)
}

fn draw(l: &Layout, words: &[Line; 6]) {
    let (cx, cy, cw, ch) = l.column;
    let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
    cache[..cw * ch].fill(0);
    let s = l.scale;
    let ui = s as u32;
    if let Some(r) = l.clock {
        let r = (r.0 - cx, r.1 - cy, r.2, r.3);
        card(cache, cw, r, s);
        text(cache, cw, words[0].as_bytes(), r.0 + 16 * s, r.1 + 12 * s, INK, 3, r.2 - 32 * s);
        text(cache, cw, words[1].as_bytes(), r.0 + 16 * s, r.1 + 12 * s + super::glyphs::UI_H * 3 + 8 * s, SOFT, ui, r.2 - 32 * s);
    }
    let line = super::glyphs::UI_H * s + 4 * s;
    if let Some(r) = l.weather {
        let r = (r.0 - cx, r.1 - cy, r.2, r.3);
        card(cache, cw, r, s);
        text(cache, cw, words[4].as_bytes(), r.0 + 16 * s, r.1 + 12 * s, INK, ui, r.2 - 32 * s);
        text(cache, cw, words[5].as_bytes(), r.0 + 16 * s, r.1 + 12 * s + line, SOFT, ui, r.2 - 32 * s);
    }
    if let Some(r) = l.system {
        let r = (r.0 - cx, r.1 - cy, r.2, r.3);
        card(cache, cw, r, s);
        text(cache, cw, words[2].as_bytes(), r.0 + 16 * s, r.1 + 12 * s, INK, ui, r.2 - 32 * s);
        text(cache, cw, words[3].as_bytes(), r.0 + 16 * s, r.1 + 12 * s + line, INK, ui, r.2 - 32 * s);
    }
}

/// A rounded, see-through card with a one-pixel (scaled) rim.
fn card(cache: &mut [u32], stride: usize, (x, y, w, h): (usize, usize, usize, usize), s: usize) {
    let radius = (10 * s) as i64;
    for row in 0..h {
        for col in 0..w {
            // Distance into the nearest corner's square, if in one.
            let dx = if (col as i64) < radius { radius - col as i64 } else if col as i64 >= w as i64 - radius { col as i64 - (w as i64 - radius - 1) } else { 0 };
            let dy = if (row as i64) < radius { radius - row as i64 } else if row as i64 >= h as i64 - radius { row as i64 - (h as i64 - radius - 1) } else { 0 };
            let d2 = dx * dx + dy * dy;
            if d2 > radius * radius { continue; }
            let edge = row < s || col < s || row + s >= h || col + s >= w || d2 > (radius - s as i64).pow(2);
            cache[(y + row) * stride + x + col] = (CARD_ALPHA << 24) | if edge { RIM } else { CARD };
        }
    }
}

/// Text at (x0, y0) in the cache over the card, cut at `max_w`.
fn text(cache: &mut [u32], stride: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize) {
    let width = super::glyphs::ui_width(t, scale).unwrap_or(0).min(max_w).min(stride.saturating_sub(x0));
    let rows = cache.len() / stride;
    for y in y0..(y0 + super::glyphs::UI_H * scale as usize).min(rows) {
        for x in x0..x0 + width {
            let Some(cov) = super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale) else { return };
            if cov == 0 { continue; }
            let p = &mut cache[y * stride + x];
            let (a, under) = (*p >> 24, *p & 0x00FF_FFFF);
            let alpha = a + (255 - a) * cov as u32 / 255;
            *p = (alpha << 24) | super::coverage_blend(ink, under, cov) & 0x00FF_FFFF;
        }
    }
}
