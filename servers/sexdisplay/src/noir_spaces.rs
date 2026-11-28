// SPDX-License-Identifier: GPL-3.0-only
//! Obsidian / macOS Noir Space Sliding, Snapshot Cache, Frosted Noir Snap Preview,
//! Mission Control 5-Space Strip, and SilkBar Quick-Layout Icon Painter.

use sex_pdx::space_slide::{
    eval_slide_offset_px, quick_layout_rects, MissionControlLayout, OverviewSpacesState,
    QuickLayoutBtn, SlideCommand, SnapPreviewState, SnapZone, DESKTOP_SPACE_COUNT,
};
use sex_theme::{candy_lavender as cl, Mode};
use silkbar_model::{ModuleSlot, SilkBar};

const SHELL_PD: u32 = 3;
const SNAP_W: usize = 200;
const SNAP_H: usize = 125;
const SPACE_CAP: usize = 9;

static mut SLIDE_CMD: SlideCommand = SlideCommand::IDLE;
static mut PREV_SLIDING: bool = false;
static mut PENDING_CAPTURE_SPACE: Option<u8> = None;

static mut SNAP_PREVIEW: SnapPreviewState = SnapPreviewState::HIDDEN;
static mut OVERVIEW_SPACES: OverviewSpacesState = OverviewSpacesState::EMPTY;
static mut OVERVIEW_ACTIVE: bool = false;

static mut SPACE_SNAP_VALID: [bool; SPACE_CAP] = [false; SPACE_CAP];
static mut SPACE_SNAPS: [[u32; SNAP_W * SNAP_H]; SPACE_CAP] = [[0; SNAP_W * SNAP_H]; SPACE_CAP];

#[inline]
fn alpha_mix(under: u32, over: u32, alpha: u8) -> u32 {
    crate::chrome_candy::mix_u8(under, over, alpha)
}

#[inline]
fn noir_palette() -> (u32, u32, u32, u32, u32) {
    let mode = crate::desk_theme::current().mode;
    match mode {
        Mode::Light => (0x00EA_EDF5, 0x00DC_E0EC, 0x008A_94AC, 0x001D_2230, cl::ORCHID_ACCENT & 0x00FF_FFFF),
        Mode::HighContrast => (0x0008_080C, 0x0014_141E, 0x00FF_FFFF, 0x00FF_FFFF, 0x00FF_D447),
        Mode::Dark => (0x0010_121A, 0x001A_1E2B, 0x008B_94AD, 0x00EC_EFF7, cl::ORCHID_ACCENT & 0x00FF_FFFF),
    }
}

/// Captures a downsampled thumbnail of `space` from the current committed front framebuffer.
pub fn capture_from_front(space: u8, front: &[u32], w: usize, h: usize, bar: usize) {
    let idx = space as usize;
    if idx >= SPACE_CAP || w == 0 || h <= bar || front.len() < w * h {
        return;
    }
    let ch = h - bar;
    unsafe {
        let dst = &mut *core::ptr::addr_of_mut!(SPACE_SNAPS[idx]);
        for sy in 0..SNAP_H {
            let src_y = bar + sy * ch / SNAP_H;
            let row_off = src_y * w;
            for sx in 0..SNAP_W {
                let src_x = sx * w / SNAP_W;
                dst[sy * SNAP_W + sx] = front[row_off + src_x];
            }
        }
        (*core::ptr::addr_of_mut!(SPACE_SNAP_VALID))[idx] = true;
    }
}

/// Runs any deferred snapshot capture from `FRAME_OUTPUT.front_pixels()` before the new Space paints.
pub fn flush_pending_capture(front: Option<&[u32]>, w: usize, h: usize, bar: usize) {
    let space = unsafe { core::ptr::replace(&mut *core::ptr::addr_of_mut!(PENDING_CAPTURE_SPACE), None) };
    if let (Some(sp), Some(px)) = (space, front) {
        capture_from_front(sp, px, w, h, bar);
    }
}

/// Handles `OP_SPACE_SLIDE` from Shell (`caller_pd == 3`). Returns `(handled_repaint_area, capture_space)`.
pub fn receive_slide(caller_pd: u32, a0: u64, a1: u64, a2: u64) -> (bool, Option<u8>) {
    if caller_pd != SHELL_PD {
        return (false, None);
    }
    let Some(cmd) = SlideCommand::decode(a0, a1, a2) else {
        return (false, None);
    };
    let capture = cmd.prepare_switch.then_some(cmd.neighbor_space);
    unsafe {
        if capture.is_some() {
            *core::ptr::addr_of_mut!(PENDING_CAPTURE_SPACE) = capture;
        }
        *core::ptr::addr_of_mut!(SLIDE_CMD) = cmd;
    }
    (true, capture)
}

/// Handles `OP_SNAP_PREVIEW` from Shell. Returns true when preview visibility/bounds changed.
pub fn receive_snap_preview(caller_pd: u32, a0: u64, a1: u64, a2: u64) -> bool {
    if caller_pd != SHELL_PD {
        return false;
    }
    let Some(next) = SnapPreviewState::decode(a0, a1, a2) else {
        return false;
    };
    unsafe {
        let cur = &mut *core::ptr::addr_of_mut!(SNAP_PREVIEW);
        if *cur == next {
            return false;
        }
        *cur = next;
    }
    true
}

/// Handles `OP_OVERVIEW_SPACES` from Shell. Returns true when Mission Control strip state changed.
pub fn receive_overview_spaces(caller_pd: u32, a0: u64, a1: u64, a2: u64) -> bool {
    if caller_pd != SHELL_PD {
        return false;
    }
    let next_active = a0 != 0;
    let Some(next) = OverviewSpacesState::decode(a0, a1, a2) else {
        return false;
    };
    unsafe {
        let cur = &mut *core::ptr::addr_of_mut!(OVERVIEW_SPACES);
        let was_active = *core::ptr::addr_of!(OVERVIEW_ACTIVE);
        if was_active == next_active && *cur == next {
            return false;
        }
        *cur = next;
        *core::ptr::addr_of_mut!(OVERVIEW_ACTIVE) = next_active;
    }
    true
}

/// True while a non-drag spring slide animation is actively ticking at `now_ms`.
/// When `cmd.dragging` is true, Display repaints synchronously on each incoming
/// `OP_SPACE_SLIDE` delta instead of spinning while fingers rest stationary.
pub fn is_sliding(w: usize, now_ms: u64) -> bool {
    let cmd = unsafe { *core::ptr::addr_of!(SLIDE_CMD) };
    if cmd.dragging {
        return false;
    }
    let (_, active) = eval_slide_offset_px(&cmd, w, now_ms);
    active
}

/// True when `is_sliding` should keep driving 60Hz pacer ticks and reports if just settled.
pub fn poll_step(w: usize, now_ms: u64) -> (bool, bool) {
    let sliding = is_sliding(w, now_ms);
    let settled = unsafe {
        let prev = core::ptr::replace(&mut *core::ptr::addr_of_mut!(PREV_SLIDING), sliding);
        prev && !sliding
    };
    (sliding, settled)
}

pub fn mission_only_open(switcher_visible: bool) -> bool {
    !switcher_visible && unsafe { *core::ptr::addr_of!(OVERVIEW_ACTIVE) }
}

/// Samples cached Space thumbnail or clean Noir Space backdrop for `(nx, y)`.
#[inline]
fn sample_neighbor_pixel(space: u8, nx: usize, y: usize, w: usize, h: usize, bar: usize) -> u32 {
    let idx = space as usize;
    let ch = h.saturating_sub(bar).max(1);
    if idx < SPACE_CAP && unsafe { (*core::ptr::addr_of!(SPACE_SNAP_VALID))[idx] } {
        let sy = ((y.saturating_sub(bar)) * SNAP_H / ch).min(SNAP_H - 1);
        let sx = (nx * SNAP_W / w.max(1)).min(SNAP_W - 1);
        return unsafe { (*core::ptr::addr_of!(SPACE_SNAPS))[idx][sy * SNAP_W + sx] };
    }
    // Crisp Obsidian Noir fallback backdrop for unvisited Spaces.
    let (bg_top, bg_bot, _, ink, accent) = noir_palette();
    let t = ((y.saturating_sub(bar)) * 255 / ch).min(255) as u8;
    let base = alpha_mix(bg_top, bg_bot, t);
    // Subtle centered "Space N" floating indicator card on empty unvisited Spaces.
    let cw = (w / 5).max(140);
    let ch_box = 44;
    let cx0 = (w.saturating_sub(cw)) / 2;
    let cy0 = bar + (ch.saturating_sub(ch_box)) / 2;
    if nx >= cx0 && nx < cx0 + cw && y >= cy0 && y < cy0 + ch_box {
        let lx = nx - cx0;
        let ly = y - cy0;
        if lx < 2 || lx + 2 >= cw || ly < 2 || ly + 2 >= ch_box {
            return accent;
        }
        let digit = b'1' + (space % 9);
        let lbl = [b'D', b'E', b'S', b'K', b'T', b'O', b'P', b' ', digit];
        if let Some(tw) = super::glyphs::ui_width(&lbl, 1) {
            let tx = (cw.saturating_sub(tw)) / 2;
            let ty = (ch_box.saturating_sub(super::glyphs::UI_H)) / 2;
            if lx >= tx && lx < tx + tw && ly >= ty && ly < ty + super::glyphs::UI_H {
                if let Some(a) = super::glyphs::ui_coverage(&lbl, (lx - tx) as i32, ly - ty, 1) {
                    if a != 0 {
                        return alpha_mix(bg_bot, ink, a);
                    }
                }
            }
        }
        return bg_bot;
    }
    base
}

/// Applies the live 1:1 Space horizontal slide directly over the body area `bar..h` of `fb`.
pub fn apply_slide_to_fb(
    fb: *mut u32,
    w: usize,
    h: usize,
    total_pixels: usize,
    scene: &crate::scene::SceneSnapshot,
    now_ms: u64,
) {
    if w == 0 || h == 0 || total_pixels < w * h {
        return;
    }
    let bar = scene.metrics.bar() as usize;
    if h <= bar {
        return;
    }
    let cmd = unsafe { *core::ptr::addr_of!(SLIDE_CMD) };
    let (offset_px, active) = eval_slide_offset_px(&cmd, w, now_ms);
    if offset_px == 0 {
        if !active && !cmd.dragging {
            // Also refresh snapshot thumbnail of active Space periodically when idle on transition completion.
        }
        return;
    }

    let s = scene.metrics.scale().clamp(1, 3) as usize;
    let gutter = (6 * s).min(w / 10).max(2);
    let is_rubber_band = cmd.neighbor_space == cmd.active_space;
    let (_, _, rim_col, _, _) = noir_palette();

    if offset_px < 0 {
        let d = ((-offset_px) as usize).min(w);
        let keep = w.saturating_sub(d);
        let gut_end = (keep + gutter).min(w);
        for y in bar..h {
            unsafe {
                let row = fb.add(y * w);
                if keep > 0 {
                    core::ptr::copy(row.add(d), row, keep);
                }
                for x in keep..gut_end {
                    let c = if x == keep || x + 1 == gut_end {
                        alpha_mix(0x000A_0B10, rim_col, 72)
                    } else {
                        0x0009_0A0F
                    };
                    core::ptr::write_volatile(row.add(x), c);
                }
                for x in gut_end..w {
                    let c = if is_rubber_band {
                        0x0007_080C
                    } else {
                        let nx = x - gut_end;
                        sample_neighbor_pixel(cmd.neighbor_space, nx, y, w, h, bar)
                    };
                    core::ptr::write_volatile(row.add(x), c);
                }
            }
        }
    } else {
        let d = (offset_px as usize).min(w);
        let keep = w.saturating_sub(d);
        let gut_start = d.saturating_sub(gutter);
        for y in bar..h {
            unsafe {
                let row = fb.add(y * w);
                if keep > 0 {
                    core::ptr::copy(row, row.add(d), keep);
                }
                for x in 0..gut_start {
                    let c = if is_rubber_band {
                        0x0007_080C
                    } else {
                        let nx = w - gut_start + x;
                        sample_neighbor_pixel(cmd.neighbor_space, nx, y, w, h, bar)
                    };
                    core::ptr::write_volatile(row.add(x), c);
                }
                for x in gut_start..d {
                    let c = if x == gut_start || x + 1 == d {
                        alpha_mix(0x000A_0B10, rim_col, 72)
                    } else {
                        0x0009_0A0F
                    };
                    core::ptr::write_volatile(row.add(x), c);
                }
            }
        }
    }
}

/// Draws the frosted Noir window snap zone outline preview when dragging near edges/corners.
pub fn draw_snap_preview(
    fb: *mut u32,
    w: usize,
    h: usize,
    total_pixels: usize,
    scene: &crate::scene::SceneSnapshot,
) {
    let st = unsafe { *core::ptr::addr_of!(SNAP_PREVIEW) };
    if st.zone == SnapZone::None || st.w == 0 || st.h == 0 || total_pixels < w * h {
        return;
    }
    let s = scene.metrics.scale().clamp(1, 3) as usize;
    let bar = scene.metrics.bar() as usize;
    let margin = 6 * s;
    let x0 = (st.x as usize).saturating_add(margin).min(w);
    let y0 = (st.y as usize).max(bar).saturating_add(margin).min(h);
    let x1 = ((st.x as usize) + (st.w as usize)).saturating_sub(margin).min(w);
    let y1 = ((st.y as usize) + (st.h as usize)).saturating_sub(margin).min(h);
    if x1 <= x0 + 8 * s || y1 <= y0 + 8 * s {
        return;
    }
    let border = 2 * s;
    let (bg_noir, _, rim_col, _, accent) = noir_palette();
    let outline = alpha_mix(rim_col, accent, 160);
    let fill_col = alpha_mix(bg_noir, accent, 32);
    for y in y0..y1 {
        for x in x0..x1 {
            let on_edge = x < x0 + border || x + border >= x1 || y < y0 + border || y + border >= y1;
            let idx = y * w + x;
            if idx < total_pixels {
                unsafe {
                    let under = core::ptr::read_volatile(fb.add(idx));
                    let c = if on_edge {
                        outline
                    } else if y < y0 + border + s {
                        alpha_mix(under, 0x00FF_FFFF, 44)
                    } else {
                        alpha_mix(under, fill_col, 136)
                    };
                    core::ptr::write_volatile(fb.add(idx), c);
                }
            }
        }
    }
}

fn fill_solid(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let idx = y * w + x;
            if idx < total {
                unsafe { core::ptr::write_volatile(fb.add(idx), c); }
            }
        }
    }
}

fn draw_ui_text(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    t: &[u8],
    x0: usize,
    y0: usize,
    ink: u32,
    scale: u32,
    max_w: usize,
) {
    let tw = super::glyphs::ui_width(t, scale).unwrap_or(0).min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    for y in 0..lh {
        let py = y0 + y;
        if py >= h {
            break;
        }
        for x in 0..tw {
            let px = x0 + x;
            if px >= w {
                break;
            }
            let Some(alpha) = super::glyphs::ui_coverage(t, x as i32, y, scale) else {
                break;
            };
            if alpha == 0 {
                continue;
            }
            let idx = py * w + px;
            if idx < total {
                unsafe {
                    let p = fb.add(idx);
                    core::ptr::write_volatile(p, super::coverage_blend(ink, core::ptr::read_volatile(p), alpha));
                }
            }
        }
    }
}

/// Renders the macOS Mission Control 5-Space bar (`Desktop 1` .. `Desktop 5`) + optional
/// floating window drag ghost when in Overview or empty-Space Mission Control.
pub fn draw_mission_control(
    fb: *mut u32,
    w: usize,
    h: usize,
    total_pixels: usize,
    scene: &crate::scene::SceneSnapshot,
) {
    let ov = unsafe { *core::ptr::addr_of!(OVERVIEW_SPACES) };
    let ov_active = unsafe { *core::ptr::addr_of!(OVERVIEW_ACTIVE) };
    if !ov_active || total_pixels < w * h {
        return;
    }
    let scale = scene.metrics.scale();
    let s = scale.clamp(1, 3) as usize;
    let bar = scene.metrics.bar() as usize;
    let Some(mc) = MissionControlLayout::new(w, h, bar, scale) else {
        return;
    };
    let (bg_noir, bg_card, rim_col, ink, accent) = noir_palette();

    // If Mission Control is opened on an empty Space without Overview window cards, dim below bar.
    if !unsafe { crate::WINDOW_SWITCHER.visible() } {
        for y in bar..h {
            for x in 0..w {
                let idx = y * w + x;
                if idx < total_pixels {
                    unsafe {
                        let under = core::ptr::read_volatile(fb.add(idx));
                        core::ptr::write_volatile(fb.add(idx), alpha_mix(under, bg_noir, 180));
                    }
                }
            }
        }
    }

    // Top Mission Control strip backdrop + hairline bottom divider.
    let bx1 = (mc.bar_rect.0 + mc.bar_rect.2).min(w);
    let by1 = (mc.bar_rect.1 + mc.bar_rect.3).min(h);
    for y in mc.bar_rect.1..by1 {
        for x in mc.bar_rect.0..bx1 {
            let idx = y * w + x;
            if idx < total_pixels {
                unsafe {
                    let under = core::ptr::read_volatile(fb.add(idx));
                    let c = if y + s >= by1 {
                        alpha_mix(bg_noir, rim_col, 90)
                    } else {
                        alpha_mix(under, bg_noir, 208)
                    };
                    core::ptr::write_volatile(fb.add(idx), c);
                }
            }
        }
    }

    for i in 0..DESKTOP_SPACE_COUNT as usize {
        let Some((cx, cy, cw, ch)) = mc.space_rect(i) else { continue };
        let thumb_h = ch.saturating_sub(18 * s);
        let is_active = ov.active_space as usize == i;
        let is_hover = ov.hover_space == Some(i as u8);
        let occupied = (ov.occupied_mask & (1u16 << i)) != 0;
        let rim = if is_hover || is_active { 2 * s } else { s };
        let border_col = if is_hover {
            accent
        } else if is_active {
            ink
        } else {
            alpha_mix(bg_card, rim_col, 140)
        };

        // Card fill + miniature thumbnail of that Space.
        let valid_snap = unsafe { (*core::ptr::addr_of!(SPACE_SNAP_VALID))[i] };
        let ch_body = h.saturating_sub(bar).max(1);
        for dy in 0..ch {
            let py = cy + dy;
            if py >= h {
                break;
            }
            for dx in 0..cw {
                let px = cx + dx;
                if px >= w {
                    continue;
                }
                let idx = py * w + px;
                if idx >= total_pixels {
                    continue;
                }
                let on_rim = dx < rim || dx + rim >= cw || dy < rim || dy + rim >= ch;
                let col = if on_rim {
                    border_col
                } else if dy < thumb_h {
                    if valid_snap && cw > 0 && thumb_h > 0 {
                        let sy = (dy * SNAP_H / thumb_h).min(SNAP_H - 1);
                        let sx = (dx * SNAP_W / cw).min(SNAP_W - 1);
                        let px_snap = unsafe { (*core::ptr::addr_of!(SPACE_SNAPS))[i][sy * SNAP_W + sx] };
                        if is_hover { alpha_mix(px_snap, accent, 52) } else { px_snap }
                    } else if is_hover {
                        alpha_mix(bg_card, accent, 64)
                    } else {
                        let t = (dy * 255 / thumb_h.max(1)).min(255) as u8;
                        let _ = ch_body;
                        alpha_mix(bg_noir, bg_card, t)
                    }
                } else if is_hover {
                    alpha_mix(bg_noir, accent, 48)
                } else {
                    bg_card
                };
                unsafe { core::ptr::write_volatile(fb.add(idx), col); }
            }
        }

        // Occupied Space dot + "Desktop N" label centered inside the label band.
        let mut lbl = [b'D', b'e', b's', b'k', b't', b'o', b'p', b' ', b'1' + i as u8];
        if cw < 84 * s {
            lbl[0] = b' ';
            lbl[1] = b' ';
            lbl[2] = b'D';
            lbl[3] = b'e';
            lbl[4] = b's';
            lbl[5] = b'k';
            lbl[6] = b' ';
            lbl[7] = b'1' + i as u8;
            lbl[8] = b' ';
        }
        let text_slice: &[u8] = if cw < 84 * s { &lbl[2..8] } else { &lbl };
        let line_h = super::glyphs::UI_H * s;
        let tw = super::glyphs::ui_width(text_slice, scale).unwrap_or(0);
        let dot_w = if occupied { 6 * s } else { 0 };
        let lx = cx + (cw.saturating_sub(tw + dot_w)) / 2 + dot_w;
        let ly = cy + thumb_h + (18 * s).saturating_sub(line_h) / 2;
        if occupied {
            let dot_col = if is_active || is_hover { accent } else { ink };
            let r_dot = (cx + (cw.saturating_sub(tw + dot_w)) / 2, cy + thumb_h + 7 * s, 4 * s, 4 * s);
            fill_solid(fb, w, h, total_pixels, r_dot, dot_col);
        }
        draw_ui_text(fb, w, h, total_pixels, text_slice, lx, ly, ink, scale, cw.saturating_sub(4 * s));
    }

    // Translucent floating drag ghost while dragging a window card onto Desktop 1..5.
    if ov.drag_sid != 0 {
        let gw = 96 * s;
        let gh = 58 * s;
        let gx = (ov.drag_x as i32 - gw as i32 / 2).clamp(0, (w as i32 - gw as i32).max(0)) as usize;
        let gy = (ov.drag_y as i32 - gh as i32 / 2).clamp(0, (h as i32 - gh as i32).max(0)) as usize;
        for dy in 0..gh {
            let py = gy + dy;
            if py >= h { break; }
            for dx in 0..gw {
                let px = gx + dx;
                if px >= w { break; }
                let idx = py * w + px;
                if idx >= total_pixels { continue; }
                let on_edge = dx < 2 * s || dx + 2 * s >= gw || dy < 2 * s || dy + 2 * s >= gh;
                unsafe {
                    let under = core::ptr::read_volatile(fb.add(idx));
                    let c = if on_edge { accent } else { alpha_mix(under, bg_card, 210) };
                    core::ptr::write_volatile(fb.add(idx), c);
                }
            }
        }
    }
}

/// Renders SilkBar's 4 compact focused-window layout buttons (`[Tile Left]`, `[Tile Right]`,
/// `[Maximize/Float]`, `[Send to Next Space →]`) strictly inside `314*s..ws0.x - 8*s`.
pub fn quick_layout_bar_pixel(
    x: usize,
    y: usize,
    bar: &SilkBar,
    scene: &crate::scene::SceneSnapshot,
    bg: u32,
) -> Option<u32> {
    if scene.focus == 0 {
        return None;
    }
    let scale = scene.metrics.scale();
    let s = scale.clamp(1, 3) as usize;
    let bar_h = scene.metrics.bar() as usize;
    if y < 3 * s || y + 3 * s >= bar_h || x < 314 * s {
        return None;
    }
    let ws0 = &bar.layout[ModuleSlot::Workspace0 as usize];
    let ws_full = bar.layout[ModuleSlot::Workspace1 as usize].w > 0;
    let btns = quick_layout_rects(bar_h, scale, ws0.x, ws_full, true);

    for (btn, r) in btns {
        if r.2 == 0 || !sex_pdx::space_slide::rect_contains(r, x, y) {
            continue;
        }
        let (bg_noir, bg_card, rim_col, ink, accent) = noir_palette();
        let lx = x - r.0;
        let ly = y - r.1;
        let on_rim = lx < s || lx + s >= r.2 || ly < s || ly + s >= r.3;
        let pill_bg = if on_rim {
            alpha_mix(bg_card, rim_col, 140)
        } else {
            alpha_mix(bg, bg_noir, 130)
        };

        // Normalized 10x10 icon box centered inside the pill.
        let iw = 10 * s;
        let ih = 8 * s;
        let ix0 = (r.2.saturating_sub(iw)) / 2;
        let iy0 = (r.3.saturating_sub(ih)) / 2;
        if lx < ix0 || lx >= ix0 + iw || ly < iy0 || ly >= iy0 + ih {
            return Some(pill_bg);
        }
        let gx = (lx - ix0) / s;
        let gy = (ly - iy0) / s;
        let icon_on = match btn {
            QuickLayoutBtn::TileLeft => {
                gx == 0 || gx == 9 || gy == 0 || gy == 7 || (gx >= 1 && gx <= 4 && gy >= 1 && gy <= 6)
            }
            QuickLayoutBtn::TileRight => {
                gx == 0 || gx == 9 || gy == 0 || gy == 7 || (gx >= 5 && gx <= 8 && gy >= 1 && gy <= 6)
            }
            QuickLayoutBtn::MaxFloat => {
                gx == 0 || gx == 9 || gy == 0 || gy == 7 || (gx >= 2 && gx <= 7 && gy >= 2 && gy <= 5)
            }
            QuickLayoutBtn::NextSpace => {
                (gy == 3 || gy == 4) && gx <= 7
                    || (gx == 6 && (gy == 2 || gy == 5))
                    || (gx == 5 && (gy == 1 || gy == 6))
                    || (gx == 9 && (gy >= 1 && gy <= 6))
            }
        };
        if icon_on {
            return Some(if btn == QuickLayoutBtn::NextSpace { accent } else { ink });
        }
        return Some(pill_bg);
    }
    None
}
