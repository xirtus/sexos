// SPDX-License-Identifier: GPL-3.0-only
//! macOS-level SilkBar interactive animations in SexOS's candy-glass style.
//!
//! Pure integer state machine and capsule/pill pixel helpers — zero heap,
//! deterministic, host-testable, zero idle cost when settled, and strictly
//! instant-snap when animations / Reduce Motion are off.

use crate::chrome_candy::{
    mix_u8, MODE_DARK, MODE_HIGH_CONTRAST,
    SILKBAR_DARK_PILL_RIM, SILKBAR_DARK_SPECULAR, SILKBAR_LIGHT_PILL_RIM,
};

/// Matches `silkbar_model::LAYOUT_COUNT` (Launcher + 9 workspaces + 3 chips + Clock + Bell).
pub const SLOT_COUNT: usize = 15;
pub const WS_COUNT: usize = 9;

/// Tactile press pulse duration in ms.
pub const PRESS_MS: u64 = 90;

/// Step result returned once per frame/event tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StepResult {
    /// True when SilkBar top strip needs a repaint (hover/active/press/ws changed).
    pub bar_dirty: bool,
    /// True when the SilkBar tray dropdown reveal progressed.
    pub menu_dirty: bool,
    /// True while any animation is in flight; false at rest (zero idle cost).
    pub animating: bool,
}

/// Snapshot of animated SilkBar visual levels for rendering a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimSnapshot {
    pub hover_alpha: [u8; SLOT_COUNT],
    pub active_alpha: [u8; SLOT_COUNT],
    pub ws_expand: [u8; WS_COUNT],
    pub press_slot: Option<u8>,
    pub menu_progress: u8,
}

impl AnimSnapshot {
    pub const IDLE: Self = Self {
        hover_alpha: [0; SLOT_COUNT],
        active_alpha: [0; SLOT_COUNT],
        ws_expand: [0; WS_COUNT],
        press_slot: None,
        menu_progress: 255,
    };
}

/// Stateful SilkBar animator owned by `sexdisplay`.
#[derive(Clone, Copy, Debug)]
pub struct SilkBarAnim {
    hover_alpha: [u8; SLOT_COUNT],
    active_alpha: [u8; SLOT_COUNT],
    ws_expand: [u8; WS_COUNT],
    press_slot: Option<usize>,
    press_until: u64,
    menu_progress: u8,
    last_tray_open: bool,
    last_tick: u64,
    initialized: bool,
}

impl SilkBarAnim {
    pub const fn new() -> Self {
        Self {
            hover_alpha: [0; SLOT_COUNT],
            active_alpha: [0; SLOT_COUNT],
            ws_expand: [0; WS_COUNT],
            press_slot: None,
            press_until: 0,
            menu_progress: 255,
            last_tray_open: false,
            last_tick: 0,
            initialized: false,
        }
    }

    /// Current immutable visual snapshot for rasterizing a frame.
    pub const fn snapshot(&self) -> AnimSnapshot {
        AnimSnapshot {
            hover_alpha: self.hover_alpha,
            active_alpha: self.active_alpha,
            ws_expand: self.ws_expand,
            press_slot: match self.press_slot {
                Some(s) => Some(s as u8),
                None => None,
            },
            menu_progress: self.menu_progress,
        }
    }

    /// Triggers a crisp tactile press highlight on SilkBar slot `slot`.
    pub fn trigger_press(&mut self, slot: usize, now_ms: u64, animations_on: bool) {
        if slot >= SLOT_COUNT { return; }
        if !animations_on {
            self.press_slot = None;
            self.press_until = 0;
            return;
        }
        self.press_slot = Some(slot);
        self.press_until = now_ms.saturating_add(PRESS_MS);
        self.hover_alpha[slot] = 255;
    }

    /// Advances all SilkBar animations toward their current targets.
    ///
    /// When `animations_on` is false (Reduce Motion), snaps all values
    /// immediately to their targets with `animating = false`.
    pub fn step(
        &mut self,
        now_ms: u64,
        animations_on: bool,
        hover_slot: Option<usize>,
        active_ws: usize,
        open_menu_slot: Option<usize>,
        tray_open: bool,
    ) -> StepResult {
        if !self.initialized {
            self.initialized = true;
            self.last_tick = now_ms;
            self.last_tray_open = tray_open;
            if active_ws < WS_COUNT {
                self.ws_expand[active_ws] = 255;
            }
            if let Some(s) = hover_slot.filter(|&s| s < SLOT_COUNT) {
                self.hover_alpha[s] = 255;
            }
            if let Some(s) = open_menu_slot.filter(|&s| s < SLOT_COUNT) {
                self.active_alpha[s] = 255;
            }
            self.menu_progress = if tray_open { 255 } else { 0 };
            return StepResult::default();
        }

        let dt = now_ms.saturating_sub(self.last_tick).clamp(1, 50) as u32;
        self.last_tick = now_ms;

        let mut bar_dirty = false;
        let mut menu_dirty = false;
        let mut animating = false;

        // Detect tray menu freshly opening to start smooth reveal from 0 -> 255.
        if tray_open && !self.last_tray_open {
            self.menu_progress = if animations_on { 0 } else { 255 };
            menu_dirty = true;
        } else if !tray_open && self.last_tray_open {
            self.menu_progress = 0;
            menu_dirty = true;
        }
        self.last_tray_open = tray_open;

        // Expire tactile press pulse.
        if let Some(_) = self.press_slot {
            if !animations_on || now_ms >= self.press_until {
                self.press_slot = None;
                self.press_until = 0;
                bar_dirty = true;
            } else {
                animating = true;
            }
        }

        // Approach helper: snappy ease-out (faster when far from target, smooth landing).
        let approach = |cur: u8, target: u8, dur_ms: u32| -> (u8, bool, bool) {
            if cur == target { return (cur, false, false); }
            if !animations_on { return (target, true, false); }
            let diff = cur.abs_diff(target) as u32;
            // Base linear step + proportional ease-out boost so hover feels instant yet silky.
            let linear = ((255 * dt) / dur_ms.max(1)).max(1);
            let step = (linear + (diff * dt) / (dur_ms / 2).max(1)).max(1).min(diff) as u8;
            let next = if cur < target { cur.saturating_add(step) } else { cur.saturating_sub(step) };
            (next, next != cur, next != target)
        };

        // 1. Per-slot hover capsule fade (120ms in, 140ms out).
        for i in 0..SLOT_COUNT {
            let target = if hover_slot == Some(i) { 255 } else { 0 };
            let dur = if target > 0 { 110 } else { 140 };
            let (next, ch, act) = approach(self.hover_alpha[i], target, dur);
            self.hover_alpha[i] = next;
            bar_dirty |= ch;
            animating |= act;
        }

        // 2. Per-slot active/open-menu highlight (100ms in, 130ms out).
        for i in 0..SLOT_COUNT {
            let target = if open_menu_slot == Some(i) { 255 } else { 0 };
            let dur = if target > 0 { 100 } else { 130 };
            let (next, ch, act) = approach(self.active_alpha[i], target, dur);
            self.active_alpha[i] = next;
            bar_dirty |= ch;
            animating |= act;
        }

        // 3. Smooth workspace pill expansion/contraction (160ms).
        for i in 0..WS_COUNT {
            let target = if i == active_ws { 255 } else { 0 };
            let (next, ch, act) = approach(self.ws_expand[i], target, 160);
            self.ws_expand[i] = next;
            bar_dirty |= ch;
            animating |= act;
        }

        // 4. Tray dropdown smooth reveal (140ms).
        if tray_open {
            let (next, ch, act) = approach(self.menu_progress, 255, 140);
            self.menu_progress = next;
            menu_dirty |= ch;
            animating |= act;
        }

        StepResult { bar_dirty, menu_dirty, animating }
    }

    /// Vertical slide-in offset (px) for SilkBar tray dropdown reveal.
    /// Returns 0 once fully settled (`menu_progress == 255`).
    pub const fn menu_slide_offset(&self, scale: usize) -> usize {
        let rem = 255u32.saturating_sub(self.menu_progress as u32);
        let s = if scale == 0 { 1 } else { scale } as u32;
        ((rem * 8 * s) / 255) as usize
    }
}

/// Renders a smooth anti-aliased candy-lavender translucent hover / active / press
/// capsule behind a SilkBar item box `(bx, by, bw, bh)`.
pub fn slot_capsule_pixel(
    x: usize,
    y: usize,
    bx: usize,
    by: usize,
    bw: usize,
    bh: usize,
    scale: usize,
    mode_id: u8,
    accent: u32,
    hover_alpha: u8,
    active_alpha: u8,
    pressed: bool,
    bg: u32,
) -> u32 {
    if hover_alpha == 0 && active_alpha == 0 && !pressed { return bg; }
    let s = scale.max(1);
    let pad_y = 2 * s;
    let ph = bh.saturating_sub(2 * pad_y);
    let pw = bw;
    if pw < 4 * s || ph < 4 * s { return bg; }
    let left = bx;
    let top = by + pad_y;
    if x < left || x >= left + pw || y < top || y >= top + ph { return bg; }

    // Smooth rounded pill SDF (corner radius = 6 * s, clamped to ph/2).
    let r = (6 * s).min(ph / 2) as i32;
    let r2 = r * 2;
    let px2 = (x * 2 + 1) as i32;
    let py2 = (y * 2 + 1) as i32;
    let x_l = (left * 2) as i32 + r2;
    let x_r = ((left + pw) * 2) as i32 - r2;
    let y_t = (top * 2) as i32 + r2;
    let y_b = ((top + ph) * 2) as i32 - r2;
    let cx = px2.clamp(x_l, x_r);
    let cy = py2.clamp(y_t, y_b);
    let dx = px2 - cx;
    let dy = py2 - cy;
    let d2 = dx * dx + dy * dy;
    let r2_sq = r2 * r2;
    if d2 >= r2_sq { return bg; }

    // Crisp straight edges (d = r2 - 1) with smooth anti-aliasing on curved corners.
    let aa_inner = (r2 - 1).max(1);
    let aa_sq = aa_inner * aa_inner;
    let cov: u8 = if d2 <= aa_sq {
        255
    } else {
        let num = (r2_sq - d2).max(0) as u32;
        let den = (r2_sq - aa_sq).max(1) as u32;
        ((num * 255) / den).min(255) as u8
    };

    let on_rim = d2 >= (r2 - 2 * s as i32).max(1).pow(2)
        || (x >= left && x < left + s)
        || (x + s >= left + pw)
        || (y >= top && y < top + s)
        || (y + s >= top + ph);

    if mode_id == MODE_HIGH_CONTRAST {
        let px = if on_rim { 0x00FF_E600 } else { bg };
        return mix_u8(bg, px, cov);
    }

    let strength = hover_alpha.max(active_alpha).max(if pressed { 255 } else { 0 });
    let is_active = active_alpha > 16 || pressed;
    let accent_rgb = accent & 0x00FF_FFFF;
    let glow_rgb = mix_u8(accent_rgb, 0x00FF_FFFF, 72);
    let (fill_col, max_fill_a, rim_col, max_rim_a): (u32, u32, u32, u32) = if mode_id == MODE_DARK {
        if is_active {
            (mix_u8(accent_rgb, glow_rgb, 96), 76, glow_rgb, 148)
        } else {
            (SILKBAR_DARK_SPECULAR, 38, SILKBAR_DARK_PILL_RIM, 128)
        }
    } else if is_active {
        (accent_rgb, 58, accent_rgb, 148)
    } else {
        (accent_rgb, 32, SILKBAR_LIGHT_PILL_RIM, 96)
    };

    let fill_a = ((max_fill_a * strength as u32) / 255) as u8;
    let mut out = mix_u8(bg, fill_col, fill_a);
    if on_rim {
        let rim_a = ((max_rim_a * strength as u32) / 255) as u8;
        out = mix_u8(out, rim_col, rim_a);
    }
    mix_u8(bg, out, cov)
}

/// Animated workspace pill: smoothly interpolates width (`5px` dot -> `15×6` capsule)
/// and color (`muted` -> glowing orchid/lavender `active`) using `expand` (`0..=255`),
/// with a soft glowing orchid halo on the active workspace and smooth anti-aliasing.
pub fn animated_workspace_pill_pixel(
    x: usize,
    y: usize,
    bx: usize,
    by: usize,
    bw: usize,
    bh: usize,
    scale: usize,
    expand: u8,
    hover_alpha: u8,
    urgent: bool,
    active_col: u32,
    muted_col: u32,
    urgent_col: u32,
    bg: u32,
) -> Option<u32> {
    let s = scale.max(1);
    let e = if urgent { 255u32 } else { expand as u32 };
    let max_w = (15 * s).min(bw.saturating_sub(2 * s));
    let min_w = 5 * s;
    let pw = min_w + ((max_w.saturating_sub(min_w) as u32 * e) / 255) as usize;
    let ph = 5 * s + ((s as u32 * e) / 255) as usize;
    if pw == 0 || ph == 0 || bw < pw || bh < ph { return None; }
    let left = bx + (bw - pw) / 2;
    let top = by + (bh - ph) / 2;
    let glow_pad = if e > 64 { 2 * s } else { 0 };
    if x < left.saturating_sub(glow_pad)
        || x >= left + pw + glow_pad
        || y < top.saturating_sub(glow_pad)
        || y >= top + ph + glow_pad
    {
        return None;
    }

    let r = ph as i32;
    let cy2 = (top * 2 + ph) as i32;
    let x_l = (left * 2 + ph) as i32;
    let x_r = ((left + pw) * 2 - ph) as i32;
    let px2 = (x * 2 + 1) as i32;
    let py2 = (y * 2 + 1) as i32;
    let cx2 = px2.clamp(x_l, x_r);
    let dx = px2 - cx2;
    let dy = py2 - cy2;
    let d2 = dx * dx + dy * dy;

    // Soft halo around active/urgent workspace pill matching the live accent.
    if d2 >= r * r {
        if glow_pad == 0 { return None; }
        let glow_r = r + (glow_pad as i32) * 2;
        if d2 >= glow_r * glow_r { return None; }
        let num = (glow_r * glow_r - d2).max(0) as u32;
        let den = (glow_r * glow_r - r * r).max(1) as u32;
        let glow_col = if urgent {
            urgent_col & 0x00FF_FFFF
        } else {
            mix_u8(active_col & 0x00FF_FFFF, 0x00FF_FFFF, 64)
        };
        let glow_a = (((num * 84) / den) * e / 255).min(88) as u8;
        if glow_a == 0 { return None; }
        return Some(mix_u8(bg, glow_col, glow_a));
    }

    let glowing_active = active_col & 0x00FF_FFFF;
    let base_col = if urgent {
        urgent_col & 0x00FF_FFFF
    } else {
        mix_u8(muted_col & 0x00FF_FFFF, glowing_active, expand)
    };
    // Subtle specular top-half sheen inside the active capsule.
    let sheen_col = if e > 128 && y < top + ph / 2 {
        mix_u8(base_col, 0x00FF_FFFF, 56)
    } else {
        base_col
    };
    let final_col = if hover_alpha > 0 {
        mix_u8(sheen_col, 0x00FF_FFFF, (hover_alpha as u32 * 56 / 255) as u8)
    } else {
        sheen_col
    };
    let aa = (r - 2).max(1);
    let cov: u8 = if d2 <= aa * aa {
        255
    } else {
        let num = (r * r - d2).max(0) as u32;
        let den = (r * r - aa * aa).max(1) as u32;
        ((num * 255) / den).min(255) as u8
    };
    Some(mix_u8(bg, final_col, cov))
}
