// SPDX-License-Identifier: GPL-3.0-only
//! Frosted glass on Display's frame (slice S6b): the popups drawn over the
//! desktop (program tray menus, tooltips and cards, the quick settings and
//! calendar, the Bluetooth panel, the Alt+Tab switcher and the window menu)
//! take `glass::frost` for their background instead of a flat fill. Their
//! rims, rows and text stay as they were. The launcher panel (surface 0x92) is
//! a compositor surface whose own fill is the body under it, so it stays flat.
//!
//! The blur reads the body under the panel and around it, so that body must be
//! this frame's, never last frame's glass: on a partial frame (damage, strip)
//! `raster_batch` paints the body again under every panel that shows
//! (`shown` / `region`) before any overlay is drawn. Glass is off in the
//! high-contrast theme and with Reduce transparency, and for a panel too big
//! for the scratch lines; then the panel is opaque as before.
use crate::glass::{self, Off, Params, Rect};
use crate::desk_theme::current as desk;

/// The popups that are glass, for receipts and for their once-per-show log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel { TrayItemsMenu, TrayItemsTip, TrayItemsCard, TrayMenu, Bluetooth, Switcher, WindowMenu, SilkBarMenu, Spotlight }
const PANELS: usize = 9;
impl Panel {
    pub const fn name(self) -> &'static str {
        match self {
            Panel::TrayItemsMenu => "tray_items_menu", Panel::TrayItemsTip => "tray_items_tip",
            Panel::TrayItemsCard => "tray_items_card", Panel::TrayMenu => "tray_menu", Panel::Bluetooth => "bt_panel",
            Panel::Switcher => "switcher", Panel::WindowMenu => "window_menu", Panel::SilkBarMenu => "silkbar_menu",
            Panel::Spotlight => "spotlight",
        }
    }
    const fn index(self) -> usize { self as usize }
}

/// Every panel that shows this frame, with its whole rectangle (rim included).
pub type Shown = [Option<(Panel, Rect)>; PANELS];

/// The pixels the glass of `panel` reads, as (x0, y0, x1, y1): the panel grown
/// by the blur margin, below the bar. A partial frame paints these again.
pub fn region(panel: Rect, w: usize, h: usize, bar: usize, scale: u32) -> (usize, usize, usize, usize) {
    glass::region(panel, glass::margin(glass::radius(scale)), (0, bar, w, h))
}

static mut BAND: [u32; glass::BAND_LEN] = [0; glass::BAND_LEN];
const LINE: usize = crate::MAX_FB_W + 2 * glass::margin(glass::radius(glass::MAX_SCALE));
static mut LINE_A: [u32; LINE] = [0; LINE];
static mut LINE_B: [u32; LINE] = [0; LINE];

struct Frame { fb: *mut u32, w: usize, total: usize }
impl glass::Pixels for Frame {
    fn get(&self, x: usize, y: usize) -> u32 {
        let i = y * self.w + x;
        if i < self.total { unsafe { core::ptr::read_volatile(self.fb.add(i)) } } else { 0 }
    }
    fn set(&mut self, x: usize, y: usize, c: u32) {
        let i = y * self.w + x;
        if i < self.total { unsafe { core::ptr::write_volatile(self.fb.add(i), c) } }
    }
}

/// The glass the desktop theme gives popups now, or why there is none.
pub fn params() -> Result<Params, Off> {
    let d = desk();
    glass::params(d.menu, &d.glass_inks(), d.mode == sex_theme::Mode::HighContrast, sex_pdx::desktop_look::reduce_transparency())
}

/// What each panel last logged (its interior and whether it was glass), and
/// which panels showed in the frame being drawn.
static mut LOGGED: [Option<(Rect, Option<Off>)>; PANELS] = [None; PANELS];
static mut LOG_BUDGET: u32 = 256;

/// Called with the panels that show before a frame's overlays: a panel that no
/// longer shows logs again the next time it does.
pub fn frame(shown: &Shown) {
    let logged = unsafe { &mut *core::ptr::addr_of_mut!(LOGGED) };
    for (i, slot) in logged.iter_mut().enumerate() {
        if !shown.iter().flatten().any(|(p, _)| p.index() == i) { *slot = None; }
    }
}

fn receipt(panel: Panel, inner: Rect, r: usize, result: Result<&Params, Off>) {
    let logged = unsafe { &mut *core::ptr::addr_of_mut!(LOGGED) };
    let key = (inner, result.err());
    if logged[panel.index()] == Some(key) { return; }
    logged[panel.index()] = Some(key);
    let budget = unsafe { &mut *core::ptr::addr_of_mut!(LOG_BUDGET) };
    if *budget == 0 { return; }
    *budget -= 1;
    let (w, h, x, y) = (inner.2, inner.3, inner.0, inner.1);
    match result {
        Ok(p) => { sex_pdx::serial_record!("[display.glass] panel={} glass=1 radius={} alpha={} rect={}x{}+{}+{} tint={:06x} floor={} ceil={}",
            panel.name(), r, p.alpha, w, h, x, y, p.tint, p.floor, p.ceil); }
        Err(off) => { sex_pdx::serial_record!("[display.glass] panel={} glass=0 reason={} rect={}x{}+{}+{}", panel.name(), off.reason(), w, h, x, y); }
    }
}

/// Frosts the interior `inner` of `panel`; false (nothing drawn) when glass is
/// off, so the caller fills it as before. Draw the rim after this: the blur
/// reads the body under it.
pub fn glass(fb: *mut u32, w: usize, h: usize, total: usize, panel: Panel, inner: Rect, scale: u32, bar: usize) -> bool {
    let r = glass::radius(scale);
    let drawn = params().and_then(|p| {
        let mut frame = Frame { fb, w, total };
        let mut s = unsafe { glass::Scratch {
            band: &mut *core::ptr::addr_of_mut!(BAND),
            a: &mut *core::ptr::addr_of_mut!(LINE_A),
            b: &mut *core::ptr::addr_of_mut!(LINE_B),
        } };
        glass::frost(&mut frame, inner, r, (0, bar, w, h.min(total / w.max(1))), &p, &mut s).map(|()| p)
    });
    receipt(panel, inner, r, drawn.as_ref().map_err(|e| *e));
    drawn.is_ok()
}

/// The interior of `panel` as glass, or filled with `fill` when glass is off.
pub fn glass_or(fb: *mut u32, w: usize, h: usize, total: usize, panel: Panel, inner: Rect, scale: u32, bar: usize, fill: u32) {
    if glass(fb, w, h, total, panel, inner, scale, bar) { return; }
    for y in inner.1..(inner.1 + inner.3).min(h) {
        for x in inner.0..(inner.0 + inner.2).min(w) {
            let i = y * w + x;
            if i < total { unsafe { core::ptr::write_volatile(fb.add(i), fill); } }
        }
    }
}

/// A rim `t` pixels thick around `r` (the edges only, so the glass inside stays).
pub fn rim(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, t: usize, c: u32) {
    let (x, y, rw, rh) = r;
    let t = t.min(rw / 2).min(rh / 2);
    let fill = |r: Rect| {
        for yy in r.1..(r.1 + r.3).min(h) {
            for xx in r.0..(r.0 + r.2).min(w) {
                let i = yy * w + xx;
                if i < total { unsafe { core::ptr::write_volatile(fb.add(i), c); } }
            }
        }
    };
    fill((x, y, rw, t));
    fill((x, y + rh - t, rw, t));
    fill((x, y + t, t, rh - 2 * t));
    fill((x + rw - t, y + t, t, rh - 2 * t));
}

/// The interior of a panel with a rim `t` thick.
pub const fn inside(r: Rect, t: usize) -> Rect {
    (r.0 + t, r.1 + t, r.2.saturating_sub(2 * t), r.3.saturating_sub(2 * t))
}
