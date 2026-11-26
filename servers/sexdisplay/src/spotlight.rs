// SPDX-License-Identifier: GPL-3.0-only
//! Frosted Candy Lavender renderer for the OLauncher / macOS Spotlight-style
//! App Launcher & Search overlay.
//!
//! Shell drives state via `OP_SPOTLIGHT_QUERY` + `OP_SPOTLIGHT`; Display renders
//! using the shared `sex_pdx::spotlight::Layout` so pointer clicks match drawn pixels.

use sex_pdx::spotlight::{
    self, CATALOG, Category, FOOTER_HINTS, Layout, PLACEHOLDER, Rect, SpotlightState, eval_calc,
    filter_catalog,
};
use sex_theme::{candy_lavender as cl, Mode};
use crate::desk_theme::current as desk;

const SHELL_PD: u32 = 3;

/// Largest cached Spotlight app icon side (`48` logical px at max scale `3`).
const ICON_CAP: usize = 144;

struct IconCache {
    /// Cached pixel side per `CATALOG` index (`0` = not yet rasterized).
    sides: [usize; spotlight::CATALOG_LEN],
    /// True if `sex_icons::draw` succeeded for this slot at `sides[idx]`.
    valid: [bool; spotlight::CATALOG_LEN],
    /// Non-premultiplied `0xAARRGGBB` rasterized brand icons.
    pixels: [[u32; ICON_CAP * ICON_CAP]; spotlight::CATALOG_LEN],
    /// Reusable `sex_icons` rasterizer scratch buffers.
    acc: [i32; sex_font::raster::scratch_len(ICON_CAP, ICON_CAP)],
    mask: [u8; ICON_CAP * ICON_CAP],
}

static mut ICON_CACHE: IconCache = IconCache {
    sides: [0; spotlight::CATALOG_LEN],
    valid: [false; spotlight::CATALOG_LEN],
    pixels: [[0; ICON_CAP * ICON_CAP]; spotlight::CATALOG_LEN],
    acc: [0; sex_font::raster::scratch_len(ICON_CAP, ICON_CAP)],
    mask: [0; ICON_CAP * ICON_CAP],
};

/// Returns the cached `side x side` non-premultiplied `0xAARRGGBB` SexOS vector
/// brand icon for `CATALOG[cat_idx]`, rasterizing it once per `(cat_idx, side)`.
#[allow(static_mut_refs)]
fn cached_app_icon(cat_idx: usize, icon_name: &str, side: usize) -> Option<(&'static [u32], usize)> {
    let side = side.min(ICON_CAP);
    if cat_idx >= spotlight::CATALOG_LEN || side == 0 {
        return None;
    }
    let c = unsafe { &mut ICON_CACHE };
    if c.sides[cat_idx] != side {
        let ok = sex_icons::find(sex_icons::Kind::App, icon_name).is_some()
            && sex_icons::draw(
                sex_icons::Kind::App,
                icon_name,
                side,
                &sex_icons::Palette::BRAND,
                &mut c.acc,
                &mut c.mask[..side * side],
                &mut c.pixels[cat_idx][..side * side],
            );
        c.sides[cat_idx] = side;
        c.valid[cat_idx] = ok;
    }
    if c.valid[cat_idx] {
        Some((&c.pixels[cat_idx][..side * side], side))
    } else {
        None
    }
}

/// Alpha-composites a `side x side` non-premultiplied `0xAARRGGBB` icon onto `fb`.
fn blit_icon(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    bx: usize,
    by: usize,
    side: usize,
    pixels: &[u32],
) {
    for iy in 0..side {
        let y = by + iy;
        if y >= h {
            break;
        }
        let row = iy * side;
        for ix in 0..side {
            let x = bx + ix;
            if x >= w {
                break;
            }
            let px = pixels[row + ix];
            let a = (px >> 24) as u8;
            if a == 0 {
                continue;
            }
            let at = y * w + x;
            if at >= total {
                return;
            }
            let fg = px & 0x00FF_FFFF;
            unsafe {
                let p = fb.add(at);
                if a == 255 {
                    core::ptr::write_volatile(p, fg);
                } else {
                    let under = core::ptr::read_volatile(p);
                    core::ptr::write_volatile(p, super::coverage_blend(fg, under, a));
                }
            }
        }
    }
}

static mut QUERY_WORDS: (u64, u64, u64) = (0, 0, 0);
static mut STATE: SpotlightState = SpotlightState::closed();

/// Handle `OP_SPOTLIGHT_QUERY` from Shell (24 ASCII query bytes).
pub fn receive_query(caller: u32, q0: u64, q1: u64, q2: u64) -> bool {
    if caller != SHELL_PD {
        return false;
    }
    unsafe {
        if QUERY_WORDS == (q0, q1, q2) {
            return false;
        }
        QUERY_WORDS = (q0, q1, q2);
        STATE.open
    }
}

/// Handle `OP_SPOTLIGHT` from Shell. Returns true when display needs redraw.
pub fn receive(caller: u32, w0: u64) -> bool {
    if caller != SHELL_PD {
        return false;
    }
    let (q0, q1, q2) = unsafe { QUERY_WORDS };
    let Some(next) = SpotlightState::decode(w0, q0, q1, q2) else {
        return false;
    };
    unsafe {
        if STATE == next {
            return false;
        }
        STATE = next;
    }
    static mut BUDGET: u32 = 32;
    unsafe {
        if BUDGET > 0 {
            BUDGET -= 1;
            if next.open {
                sex_pdx::serial_record!(
                    "[display.spotlight] open=1 cat={} sel={} qlen={}",
                    next.category as u8,
                    next.selected,
                    next.query.len()
                );
            } else {
                sex_pdx::serial_record!("[display.spotlight] open=0");
            }
        }
    }
    true
}

/// Panel rectangle for `frost::glass_panels`.
pub fn glass_rect(w: usize, h: usize, bar: usize, scale: u32) -> Option<Rect> {
    let st = unsafe { STATE };
    if !st.open {
        return None;
    }
    Layout::new(w, h, bar, scale).map(|l| l.panel)
}

#[derive(Clone, Copy)]
struct Palette {
    bg: u32,
    rim: u32,
    specular: u32,
    input_bg: u32,
    tile_bg: u32,
    tile_sel: u32,
    tile_hover: u32,
    pill_bg: u32,
    pill_active: u32,
    on_active: u32,
    divider: u32,
    ink: u32,
    subtext: u32,
    accent: u32,
}

fn palette() -> Palette {
    let d = desk();
    match d.mode {
        Mode::HighContrast => Palette {
            bg: d.menu,
            rim: d.menu_rim,
            specular: d.menu_rim,
            input_bg: d.menu,
            tile_bg: d.menu_button,
            tile_sel: d.menu_picked,
            tile_hover: d.menu_picked,
            pill_bg: d.menu_button,
            pill_active: d.focus,
            on_active: 0x00FF_FFFF,
            divider: d.menu_rim,
            ink: d.menu_ink,
            subtext: d.menu_soft,
            accent: d.focus,
        },
        Mode::Dark => Palette {
            bg: cl::DARK_MENU_BG & 0x00FF_FFFF,
            rim: cl::DARK_MENU_RIM & 0x00FF_FFFF,
            specular: cl::DARK_SPECULAR & 0x00FF_FFFF,
            input_bg: cl::DARK_SPOTLIGHT_INPUT & 0x00FF_FFFF,
            tile_bg: cl::DARK_SPOTLIGHT_TILE & 0x00FF_FFFF,
            tile_sel: cl::DARK_SPOTLIGHT_SELECTED & 0x00FF_FFFF,
            tile_hover: cl::DARK_MENU_HOVER & 0x00FF_FFFF,
            pill_bg: cl::DARK_SPOTLIGHT_PILL & 0x00FF_FFFF,
            pill_active: cl::ORCHID_PILL_ACTIVE & 0x00FF_FFFF,
            on_active: cl::ON_ORCHID_ACTIVE & 0x00FF_FFFF,
            divider: cl::DARK_MENU_DIVIDER & 0x00FF_FFFF,
            ink: cl::DARK_TEXT & 0x00FF_FFFF,
            subtext: cl::DARK_MENU_SUBTEXT & 0x00FF_FFFF,
            accent: cl::ORCHID_ACCENT & 0x00FF_FFFF,
        },
        Mode::Light => Palette {
            bg: cl::LIGHT_MENU_BG & 0x00FF_FFFF,
            rim: cl::LIGHT_MENU_RIM & 0x00FF_FFFF,
            specular: cl::LIGHT_SPECULAR & 0x00FF_FFFF,
            input_bg: cl::LIGHT_SPOTLIGHT_INPUT & 0x00FF_FFFF,
            tile_bg: cl::LIGHT_SPOTLIGHT_TILE & 0x00FF_FFFF,
            tile_sel: cl::LIGHT_SPOTLIGHT_SELECTED & 0x00FF_FFFF,
            tile_hover: cl::LIGHT_MENU_HOVER & 0x00FF_FFFF,
            pill_bg: cl::LIGHT_SPOTLIGHT_PILL & 0x00FF_FFFF,
            pill_active: cl::ORCHID_PILL_ACTIVE & 0x00FF_FFFF,
            on_active: cl::ON_ORCHID_ACTIVE & 0x00FF_FFFF,
            divider: cl::LIGHT_MENU_DIVIDER & 0x00FF_FFFF,
            ink: cl::LIGHT_TEXT & 0x00FF_FFFF,
            subtext: cl::LIGHT_MENU_SUBTEXT & 0x00FF_FFFF,
            accent: cl::ORCHID_ACCENT & 0x00FF_FFFF,
        },
    }
}

#[inline]
fn put_px(fb: *mut u32, w: usize, h: usize, total: usize, x: usize, y: usize, c: u32) {
    if x < w && y < h {
        let i = y * w + x;
        if i < total {
            unsafe { core::ptr::write_volatile(fb.add(i), c); }
        }
    }
}

fn fill(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            put_px(fb, w, h, total, x, y, c);
        }
    }
}

/// Fills a rounded rectangle with corner radius `rad` (clipped cleanly to `fb`).
fn fill_rounded(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, rad: usize, c: u32) {
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let r2 = (rad * rad) as isize;
    for dy in 0..r.3 {
        let y = r.1 + dy;
        if y >= h {
            break;
        }
        let cy = if dy < rad {
            rad - dy
        } else if dy + rad >= r.3 {
            dy + rad + 1 - r.3
        } else {
            0
        };
        for dx in 0..r.2 {
            let x = r.0 + dx;
            if x >= w {
                break;
            }
            if cy > 0 {
                let cx = if dx < rad {
                    rad - dx
                } else if dx + rad >= r.2 {
                    dx + rad + 1 - r.2
                } else {
                    0
                };
                if cx > 0 && (cx * cx + cy * cy) as isize > r2 {
                    continue;
                }
            }
            put_px(fb, w, h, total, x, y, c);
        }
    }
}

/// Draws a 1-scaled-pixel rounded border around `r`.
fn stroke_rounded(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, rad: usize, thick: usize, c: u32) {
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let outer_r2 = (rad * rad) as isize;
    let inner_rad = rad.saturating_sub(thick);
    let inner_r2 = (inner_rad * inner_rad) as isize;
    for dy in 0..r.3 {
        let y = r.1 + dy;
        if y >= h {
            break;
        }
        let in_y_band = dy < thick || dy + thick >= r.3;
        let cy = if dy < rad {
            rad - dy
        } else if dy + rad >= r.3 {
            dy + rad + 1 - r.3
        } else {
            0
        };
        for dx in 0..r.2 {
            let x = r.0 + dx;
            if x >= w {
                break;
            }
            let in_x_band = dx < thick || dx + thick >= r.2;
            let cx = if dx < rad {
                rad - dx
            } else if dx + rad >= r.2 {
                dx + rad + 1 - r.2
            } else {
                0
            };
            if cx > 0 && cy > 0 {
                let d2 = (cx * cx + cy * cy) as isize;
                if d2 <= outer_r2 && d2 >= inner_r2 {
                    put_px(fb, w, h, total, x, y, c);
                }
            } else if in_x_band || in_y_band {
                put_px(fb, w, h, total, x, y, c);
            }
        }
    }
}

fn tint_glass(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, tint: u32, alpha: u8) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let i = y * w + x;
            if i < total {
                unsafe {
                    let p = fb.add(i);
                    let under = core::ptr::read_volatile(p);
                    core::ptr::write_volatile(p, crate::chrome_candy::mix_u8(under, tint, alpha));
                }
            }
        }
    }
}

fn draw_text(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize) {
    let width = super::glyphs::ui_width(t, scale).unwrap_or(0).min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let at = y * w + x;
            if at >= total {
                return;
            }
            let Some(alpha) = super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale) else {
                break;
            };
            if alpha == 0 {
                continue;
            }
            unsafe {
                let p = fb.add(at);
                core::ptr::write_volatile(p, super::coverage_blend(ink, core::ptr::read_volatile(p), alpha));
            }
        }
    }
}

/// Draws a crisp magnifying glass icon centered vertically inside the search bar.
fn draw_search_icon(fb: *mut u32, w: usize, h: usize, total: usize, cx: usize, cy: usize, s: usize, color: u32) {
    let r = (6 * s) as isize;
    let r_out = r * r;
    let r_in = (r - (2 * s) as isize).max(1);
    let r_in2 = r_in * r_in;
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = dx * dx + dy * dy;
            if d2 <= r_out && d2 >= r_in2 {
                let px = (cx as isize + dx).max(0) as usize;
                let py = (cy as isize + dy).max(0) as usize;
                put_px(fb, w, h, total, px, py, color);
            }
        }
    }
    // Diagonal handle down-right
    let start = (4 * s) as usize;
    let end = (9 * s) as usize;
    for d in start..=end {
        for t in 0..=(s) {
            put_px(fb, w, h, total, cx + d + t, cy + d, color);
            put_px(fb, w, h, total, cx + d, cy + d + t, color);
        }
    }
}

/// Renders the centered Frosted Candy Lavender Spotlight Launcher overlay.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32, bar: usize) {
    let st = unsafe { STATE };
    if !st.open {
        return;
    }
    let Some(l) = Layout::new(w, h, bar, scale) else {
        return;
    };
    let sc = l.scale;
    let s = sc as usize;
    let p = palette();
    let inner = crate::frost::inside(l.panel, s);

    // 1. Frosted Candy Lavender glass backdrop + specular top sheen + orchid rim.
    crate::frost::glass_or(fb, w, h, total, crate::frost::Panel::Spotlight, inner, sc, bar, p.bg);
    if desk().mode != Mode::HighContrast {
        tint_glass(fb, w, h, total, inner, p.bg, 84);
        tint_glass(fb, w, h, total, (inner.0, inner.1, inner.2, 2 * s), p.specular, 80);
    }
    crate::frost::rim(fb, w, h, total, l.panel, s, p.rim);

    let line_h = super::glyphs::UI_H * s;

    // 2. Top Search Input bar with magnifying glass icon, live text, caret, and inline calculator pill.
    let sb = l.search_box;
    fill_rounded(fb, w, h, total, sb, 10 * s, p.input_bg);
    stroke_rounded(fb, w, h, total, sb, 10 * s, s, p.accent);

    draw_search_icon(fb, w, h, total, sb.0 + 20 * s, sb.1 + sb.3 / 2 - s, s, p.accent);

    let calc = eval_calc(st.query.as_str());
    let mut calc_pill_w = 0usize;
    if let Some(res) = calc {
        let rb = res.as_str().as_bytes();
        let tw = super::glyphs::ui_width(rb, sc).unwrap_or(0);
        let pw = tw + 20 * s;
        let ph = sb.3.saturating_sub(14 * s);
        let px = sb.0 + sb.2.saturating_sub(pw + 8 * s);
        let py = sb.1 + (sb.3.saturating_sub(ph)) / 2;
        fill_rounded(fb, w, h, total, (px, py, pw, ph), 8 * s, p.pill_active);
        let ty = py + ph.saturating_sub(line_h) / 2;
        draw_text(fb, w, h, total, rb, px + 10 * s, ty, p.on_active, sc, tw);
        calc_pill_w = pw + 16 * s;
    }

    let text_x = sb.0 + 40 * s;
    let text_y = sb.1 + sb.3.saturating_sub(line_h) / 2;
    let text_max_w = sb.2.saturating_sub(50 * s + calc_pill_w);
    if st.query.is_empty() {
        draw_text(fb, w, h, total, PLACEHOLDER.as_bytes(), text_x, text_y, p.subtext, sc, text_max_w);
    } else {
        let qb = st.query.as_bytes();
        draw_text(fb, w, h, total, qb, text_x, text_y, p.ink, sc, text_max_w);
        let qw = super::glyphs::ui_width(qb, sc).unwrap_or(0).min(text_max_w);
        // Crisp orchid text caret
        fill(fb, w, h, total, (text_x + qw + 2 * s, text_y, 2 * s, line_h), p.accent);
    }

    // 3. Category Filter Pills: All | Apps | Media | AI & Dev | System
    for (i, &cat) in Category::ALL.iter().enumerate() {
        let r = l.category_pills[i];
        if r.2 == 0 {
            continue;
        }
        let active = st.category == cat;
        let hovered = st.hover_cat == Some(i as u8);
        let bg = if active {
            p.pill_active
        } else if hovered {
            p.tile_hover
        } else {
            p.pill_bg
        };
        fill_rounded(fb, w, h, total, r, 10 * s, bg);
        if !active {
            stroke_rounded(fb, w, h, total, r, 10 * s, s, p.divider);
        }
        let lbl = cat.label().as_bytes();
        let tw = super::glyphs::ui_width(lbl, sc).unwrap_or(0);
        let tx = r.0 + r.2.saturating_sub(tw) / 2;
        let ty = r.1 + r.3.saturating_sub(line_h) / 2;
        let ink = if active { p.on_active } else { p.ink };
        draw_text(fb, w, h, total, lbl, tx, ty, ink, sc, r.2.saturating_sub(4 * s));
    }

    // 4. App Icon Grid (5 columns x 2 rows)
    let (order, count) = filter_catalog(st.category, st.query.as_str());
    let shown = count.min(spotlight::GRID_COLS * spotlight::GRID_ROWS);
    if shown == 0 {
        let msg: &[u8] = if calc.is_some() {
            b"Calculator result ready above"
        } else {
            b"No matching apps found"
        };
        let tw = super::glyphs::ui_width(msg, sc).unwrap_or(0);
        let tx = l.grid_area.0 + l.grid_area.2.saturating_sub(tw) / 2;
        let ty = l.grid_area.1 + l.grid_area.3.saturating_sub(line_h) / 2;
        draw_text(fb, w, h, total, msg, tx, ty, p.subtext, sc, l.grid_area.2);
    } else {
        let sel_idx = (st.selected as usize).min(shown - 1);
        for i in 0..shown {
            let Some(tr) = l.tile_rect(i) else { break };
            let cat_idx = order[i];
            let app = &CATALOG[cat_idx];
            let is_sel = i == sel_idx;
            let is_hov = st.hover_tile == Some(i as u8);

            // Tile card background + soft lavender highlight on selected/hovered tile
            let tile_bg = if is_sel {
                p.tile_sel
            } else if is_hov {
                p.tile_hover
            } else {
                p.tile_bg
            };
            fill_rounded(fb, w, h, total, tr, 12 * s, tile_bg);
            if is_sel {
                stroke_rounded(fb, w, h, total, tr, 12 * s, 2 * s, p.accent);
            } else if is_hov {
                stroke_rounded(fb, w, h, total, tr, 12 * s, s, p.rim);
            } else {
                stroke_rounded(fb, w, h, total, tr, 12 * s, s, p.divider);
            }

            // Real SexOS vector brand icon (48x48 logical px, cached per catalog index & size)
            let icon_gap = 8 * s;
            let badge_sz = (48 * s)
                .min(tr.2.saturating_sub(16 * s))
                .min(tr.3.saturating_sub(line_h + icon_gap + 12 * s));
            let content_h = badge_sz + icon_gap + line_h;
            let bx = tr.0 + tr.2.saturating_sub(badge_sz) / 2;
            let by = tr.1 + tr.3.saturating_sub(content_h) / 2;

            if let Some((pixels, side)) = cached_app_icon(cat_idx, app.icon, badge_sz) {
                let ix = bx + badge_sz.saturating_sub(side) / 2;
                let iy = by + badge_sz.saturating_sub(side) / 2;
                blit_icon(fb, w, h, total, ix, iy, side, pixels);
            } else {
                // Fallback rounded Candy badge + white monogram if sex_icons::find returned None.
                let badge_rect = (bx, by, badge_sz, badge_sz);
                fill_rounded(fb, w, h, total, badge_rect, 12 * s, app.badge_rgb);
                if desk().mode != Mode::HighContrast && badge_sz > 10 * s {
                    tint_glass(fb, w, h, total, (bx + 4 * s, by + 2 * s, badge_sz.saturating_sub(8 * s), 3 * s), 0x00FF_FFFF, 70);
                }
                let mono = app.monogram.as_bytes();
                let mono_sc = (sc * 2).min(3);
                let mw = super::glyphs::ui_width(mono, mono_sc).unwrap_or(0);
                let mh = super::glyphs::UI_H * mono_sc as usize;
                let mx = bx + badge_sz.saturating_sub(mw) / 2;
                let my = by + badge_sz.saturating_sub(mh) / 2;
                draw_text(fb, w, h, total, mono, mx, my, 0x00FF_FFFF, mono_sc, badge_sz);
            }

            // Crisp app label centered beneath the icon
            let name = app.name.as_bytes();
            let nw = super::glyphs::ui_width(name, sc).unwrap_or(0);
            let max_nw = tr.2.saturating_sub(8 * s);
            let nx = tr.0 + tr.2.saturating_sub(nw.min(max_nw)) / 2;
            let ny = by + badge_sz + icon_gap;
            draw_text(fb, w, h, total, name, nx, ny, p.ink, sc, max_nw);
        }
    }

    // 5. Footer Shortcut Bar: "Arrows Select   Enter Open   Tab Filter   Esc Close"
    let ft = l.footer;
    fill(fb, w, h, total, (ft.0, ft.1, ft.2, s), p.divider);
    let hints = FOOTER_HINTS.as_bytes();
    let hw = super::glyphs::ui_width(hints, sc).unwrap_or(0);
    let hx = ft.0 + ft.2.saturating_sub(hw.min(ft.2)) / 2;
    let hy = ft.1 + ft.3.saturating_sub(line_h) / 2;
    draw_text(fb, w, h, total, hints, hx, hy, p.subtext, sc, ft.2);
}
