// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Shared protocol, spring physics, snap zones, Mission Control Space bar,
//! and SilkBar quick-layout geometry for SexOS Desktop Spaces & Multitouch.
//!
//! Self-contained (`#![no_std]`, zero heap) so `sex-hid-touch` consumers,
//! `sexinput`, `silk-shell`, `sexdisplay`, and host tests all compile the
//! exact same math and wire encoders.

/// Input -> Shell: continuous 3-finger horizontal swipe & 3-finger vertical swipes.
pub const OP_TOUCHPAD_GESTURE: u64 = 0x2A1;
/// Shell -> Display: live Space slide offset, spring animation, and active bounds.
pub const OP_SPACE_SLIDE: u64 = 0x2A2;
/// Shell -> Display: live frosted Noir window-snap preview outline.
pub const OP_SNAP_PREVIEW: u64 = 0x2A3;
/// Shell -> Display: Mission Control 5-Space bar state and Overview DnD ghost.
pub const OP_OVERVIEW_SPACES: u64 = 0x2A4;

/// Primary macOS-style Desktop Spaces visible in Mission Control & swipe loop.
pub const DESKTOP_SPACE_COUNT: u8 = 5;
/// Total workspaces supported by Shell/SilkBar (`Super+1..9`).
pub const MAX_SPACES: u8 = 9;

/// Fixed-point 1.0 screen-width unit (`Q12`).
pub const Q12_ONE: i32 = 4096;
/// Maximum rubber-band overshoot at Space 1 / Space 5 edges (`0.25 * screen_w`).
pub const RUBBER_BAND_MAX_Q12: i32 = Q12_ONE / 4;
/// Position commit threshold (`35%` of screen width).
pub const COMMIT_POS_Q12: i32 = (Q12_ONE * 35) / 100;
/// Velocity commit threshold in Q12 screen-widths per second (`~0.58 screens/sec`).
pub const COMMIT_VEL_Q12_PER_S: i32 = 2400;
/// Minimum displacement required when committing purely via velocity flick (`~5%`).
pub const COMMIT_FLICK_MIN_POS_Q12: i32 = Q12_ONE / 20;

/// Crisp macOS-style settle duration (ms) for discrete and spring settles.
pub const SPRING_SETTLE_MS: u64 = 140;
/// Edge spring-load hover delay (ms) when carrying a dragged window across Spaces.
pub const SPRING_LOAD_EDGE_MS: u64 = 350;
/// Repeat cadence (ms) when continuing to hold a dragged window on the screen edge.
pub const EDGE_CARRY_REPEAT_MS: u64 = 450;

pub type Rect = (usize, usize, usize, usize);

#[inline]
pub const fn rect_contains(r: Rect, x: usize, y: usize) -> bool {
    x >= r.0 && y >= r.1 && x - r.0 < r.2 && y - r.1 < r.3
}

/// Pure integer ease-out cubic spring curve `0..=1024` over `duration_ms`.
/// Reaches 1024 at `elapsed >= duration_ms` with brisk initial velocity and
/// zero bounce past target.
#[inline]
pub fn spring_ease(elapsed_ms: u64, duration_ms: u64) -> u32 {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return 1024;
    }
    let t = (elapsed_ms * 1024 / duration_ms) as i64;
    let u = 1024 - t;
    (1024 - (u * u * u) / (1024 * 1024)) as u32
}

/// Applies macOS/iOS-style progressive rubber-band compression `x * M / (x + M)`
/// symmetrically around zero, clamping asymptotic overshoot below `RUBBER_BAND_MAX_Q12`.
#[inline]
pub fn rubber_band_q12(raw_q12: i32) -> i32 {
    let m = RUBBER_BAND_MAX_Q12 as i64;
    let a = raw_q12.unsigned_abs() as i64;
    if a == 0 {
        return 0;
    }
    let out = ((a * m) / (a + m)).min(m) as i32;
    if raw_q12 < 0 { -out } else { out }
}

// ── 1. Touchpad Gesture IPC Wire (`sexinput` -> `silk-shell`) ────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureMsg {
    /// Continuous 3-finger horizontal delta in Q12 screen-widths (`dt_ms` since last frame).
    SwipeMove { dx_q12: i32, dt_ms: u16 },
    /// 3-finger horizontal swipe ended with EMA velocity (Q12 screen-widths/sec).
    SwipeEnd { vel_q12_per_s: i32 },
    /// Discrete 3-finger swipe left (fallback / non-continuous input).
    SwipeLeft,
    /// Discrete 3-finger swipe right (fallback / non-continuous input).
    SwipeRight,
    /// 3-finger swipe up: open Mission Control / Overview.
    SwipeUp,
    /// 3-finger swipe down: close Overview or restore minimized window.
    SwipeDown,
}

impl GestureMsg {
    pub const fn wire(self) -> [u64; 3] {
        match self {
            Self::SwipeMove { dx_q12, dt_ms } => [
                0,
                (dx_q12 as i64) as u64,
                dt_ms as u64,
            ],
            Self::SwipeEnd { vel_q12_per_s } => [
                1,
                (vel_q12_per_s as i64) as u64,
                0,
            ],
            Self::SwipeLeft => [2, 0, 0],
            Self::SwipeRight => [3, 0, 0],
            Self::SwipeUp => [4, 0, 0],
            Self::SwipeDown => [5, 0, 0],
        }
    }

    pub const fn decode(arg0: u64, arg1: u64, arg2: u64) -> Option<Self> {
        match arg0 {
            0 => {
                let raw = arg1 as i64;
                if raw < -65536 || raw > 65536 || arg2 > 65535 {
                    return None;
                }
                Some(Self::SwipeMove {
                    dx_q12: raw as i32,
                    dt_ms: arg2 as u16,
                })
            }
            1 => {
                if arg2 != 0 {
                    return None;
                }
                let raw = arg1 as i64;
                let vel = if raw < -65536 { -65536 } else if raw > 65536 { 65536 } else { raw } as i32;
                Some(Self::SwipeEnd { vel_q12_per_s: vel })
            }
            2 if arg1 == 0 && arg2 == 0 => Some(Self::SwipeLeft),
            3 if arg1 == 0 && arg2 == 0 => Some(Self::SwipeRight),
            4 if arg1 == 0 && arg2 == 0 => Some(Self::SwipeUp),
            5 if arg1 == 0 && arg2 == 0 => Some(Self::SwipeDown),
            _ => None,
        }
    }
}

// ── 2. Space Slide Spring Physics & Wire (`silk-shell` -> `sexdisplay`) ──────

/// Display-side state wire transmitted via `OP_SPACE_SLIDE`.
///
/// Word layout:
/// - `arg0`: bits 0..7 `active_space`, bits 8..15 `neighbor_space`, bits 16..23 `space_bound`,
///   bit 24 `animations_on`, bit 25 `prepare_switch` (capture snapshot of `neighbor_space` first!),
///   bit 26 `dragging`, bits 32..63 signed `offset_q12` (`i32 as u32`).
/// - `arg1`: signed `target_q12` (`i32 as u32` in low 32 bits) | `duration_ms` (bits 32..47).
/// - `arg2`: `start_tick_ms` (`0` when settled or live-dragging).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideCommand {
    pub active_space: u8,
    pub neighbor_space: u8,
    pub space_bound: u8,
    pub animations_on: bool,
    pub prepare_switch: bool,
    pub dragging: bool,
    pub offset_q12: i32,
    pub target_q12: i32,
    pub duration_ms: u16,
    pub start_tick_ms: u64,
}

impl SlideCommand {
    pub const IDLE: Self = Self {
        active_space: 0,
        neighbor_space: 0,
        space_bound: DESKTOP_SPACE_COUNT,
        animations_on: true,
        prepare_switch: false,
        dragging: false,
        offset_q12: 0,
        target_q12: 0,
        duration_ms: 0,
        start_tick_ms: 0,
    };

    pub const fn wire(self) -> [u64; 3] {
        let a0 = (self.active_space as u64 & 0xF)
            | ((self.neighbor_space as u64 & 0xF) << 8)
            | ((self.space_bound as u64 & 0xF) << 16)
            | ((self.animations_on as u64) << 24)
            | ((self.prepare_switch as u64) << 25)
            | ((self.dragging as u64) << 26)
            | (((self.offset_q12 as i32 as u32) as u64) << 32)
            | (1u64 << 63);
        let a1 = ((self.target_q12 as i32 as u32) as u64)
            | ((self.duration_ms as u64) << 32);
        [a0, a1, self.start_tick_ms]
    }

    pub fn decode(arg0: u64, arg1: u64, arg2: u64) -> Option<Self> {
        if arg0 == 0 {
            return Some(Self::IDLE);
        }
        if (arg0 >> 63) == 0 {
            return None;
        }
        let active_space = (arg0 & 0xF) as u8;
        let neighbor_space = ((arg0 >> 8) & 0xF) as u8;
        let space_bound = ((arg0 >> 16) & 0xF) as u8;
        if space_bound == 0 || space_bound > MAX_SPACES || active_space >= space_bound || neighbor_space >= MAX_SPACES {
            return None;
        }
        let animations_on = (arg0 >> 24) & 1 != 0;
        let prepare_switch = (arg0 >> 25) & 1 != 0;
        let dragging = (arg0 >> 26) & 1 != 0;
        if ((arg0 >> 27) & 0x1F) != 0 {
            return None;
        }
        let offset_q12 = ((arg0 >> 32) as u32) as i32;
        let target_q12 = (arg1 as u32) as i32;
        let duration_ms = ((arg1 >> 32) & 0xFFFF) as u16;
        if (arg1 >> 48) != 0 {
            return None;
        }
        if offset_q12.abs() > Q12_ONE * 2 || target_q12.abs() > Q12_ONE * 2 {
            return None;
        }
        Some(Self {
            active_space,
            neighbor_space,
            space_bound,
            animations_on,
            prepare_switch,
            dragging,
            offset_q12,
            target_q12,
            duration_ms,
            start_tick_ms: arg2,
        })
    }
}

/// Pure integer Shell-side state machine coordinating continuous 3-finger Space
/// sliding, boundary rubber-banding, velocity-assisted commits, and 140ms spring settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpaceSlidePhysics {
    pub active_space: u8,
    pub dragging: bool,
    pub raw_q12: i32,
    pub visual_q12: i32,
    pub settling: bool,
    pub settle_from_q12: i32,
    pub settle_neighbor: u8,
    pub settle_start_ms: u64,
    pub settle_duration_ms: u16,
}

impl SpaceSlidePhysics {
    pub const fn new() -> Self {
        Self {
            active_space: 0,
            dragging: false,
            raw_q12: 0,
            visual_q12: 0,
            settling: false,
            settle_from_q12: 0,
            settle_neighbor: 0,
            settle_start_ms: 0,
            settle_duration_ms: 0,
        }
    }

    #[inline]
    pub fn effective_bound(active_space: u8) -> u8 {
        if active_space < DESKTOP_SPACE_COUNT {
            DESKTOP_SPACE_COUNT
        } else {
            MAX_SPACES
        }
    }

    /// Synchronizes active space when switched externally (e.g. boot, Alt-F-key).
    pub fn sync_space(&mut self, space: u8) {
        self.active_space = space.min(MAX_SPACES - 1);
    }

    /// Applies a continuous horizontal 3-finger delta (`dx_q12`: negative = swipe left
    /// revealing next space; positive = swipe right revealing previous space).
    /// Returns updated `SlideCommand` to send to Display.
    pub fn apply_move(&mut self, active_space: u8, dx_q12: i32, animations_on: bool) -> SlideCommand {
        self.active_space = active_space.min(MAX_SPACES - 1);
        let bound = Self::effective_bound(self.active_space);
        if !self.dragging {
            // Pick up cleanly from any currently settling offset.
            self.dragging = true;
            self.raw_q12 = self.visual_q12;
            self.settling = false;
        }
        self.raw_q12 = (self.raw_q12 + dx_q12).clamp(-Q12_ONE, Q12_ONE);
        let at_left_end = self.active_space == 0 && self.raw_q12 > 0;
        let at_right_end = self.active_space + 1 >= bound && self.raw_q12 < 0;
        self.visual_q12 = if at_left_end || at_right_end {
            rubber_band_q12(self.raw_q12)
        } else {
            self.raw_q12.clamp(-Q12_ONE, Q12_ONE)
        };
        let neighbor = if self.visual_q12 < 0 && self.active_space + 1 < bound {
            self.active_space + 1
        } else if self.visual_q12 > 0 && self.active_space > 0 {
            self.active_space - 1
        } else {
            self.active_space
        };
        SlideCommand {
            active_space: self.active_space,
            neighbor_space: neighbor,
            space_bound: bound,
            animations_on,
            prepare_switch: false,
            dragging: true,
            offset_q12: self.visual_q12,
            target_q12: self.visual_q12,
            duration_ms: 0,
            start_tick_ms: 0,
        }
    }

    /// Decides whether a released 3-finger horizontal swipe commits to an adjacent
    /// Space (`Some(target_space)`) or springs back to the current Space (`None`).
    pub fn decide_release(&self, active_space: u8, vel_q12_per_s: i32) -> Option<u8> {
        let bound = Self::effective_bound(active_space);
        let pos = self.visual_q12;
        // Swipe left (`pos < 0`) moves toward `active_space + 1`.
        if pos < 0 && active_space + 1 < bound {
            let far_enough = pos <= -COMMIT_POS_Q12;
            let flick = vel_q12_per_s <= -COMMIT_VEL_Q12_PER_S && pos <= -COMMIT_FLICK_MIN_POS_Q12;
            if far_enough || flick {
                return Some(active_space + 1);
            }
        }
        // Swipe right (`pos > 0`) moves toward `active_space - 1`.
        if pos > 0 && active_space > 0 {
            let far_enough = pos >= COMMIT_POS_Q12;
            let flick = vel_q12_per_s >= COMMIT_VEL_Q12_PER_S && pos >= COMMIT_FLICK_MIN_POS_Q12;
            if far_enough || flick {
                return Some(active_space - 1);
            }
        }
        None
    }

    /// Ends the continuous drag: either commits to `Some(target_space)` with a smooth
    /// continuation offset (`visual_q12 +/- Q12_ONE -> 0` over 140ms) or springs back
    /// (`visual_q12 -> 0` over 140ms).
    /// Returns `(committed_target, command_to_display)`.
    pub fn end_drag(
        &mut self,
        active_space: u8,
        vel_q12_per_s: i32,
        now_ms: u64,
        animations_on: bool,
    ) -> (Option<u8>, SlideCommand) {
        self.active_space = active_space.min(MAX_SPACES - 1);
        self.dragging = false;
        let bound = Self::effective_bound(self.active_space);
        let next = self.decide_release(self.active_space, vel_q12_per_s);
        if let Some(to) = next {
            // Remaining travel after switching viewport anchor from `active` to `to`:
            // e.g. swiped left `-0.4 * W` toward Space+1 -> Space+1 enters from `+0.6 * W` -> 0.
            let dir = if to > self.active_space { 1 } else { -1 };
            let start_q12 = if animations_on {
                (self.visual_q12 + dir * Q12_ONE).clamp(-Q12_ONE, Q12_ONE)
            } else {
                0
            };
            let prev = self.active_space;
            self.active_space = to;
            self.raw_q12 = 0;
            self.visual_q12 = start_q12;
            self.settling = animations_on && start_q12 != 0;
            self.settle_from_q12 = start_q12;
            self.settle_neighbor = prev;
            self.settle_start_ms = now_ms;
            self.settle_duration_ms = if self.settling { SPRING_SETTLE_MS as u16 } else { 0 };
            let cmd = SlideCommand {
                active_space: to,
                neighbor_space: prev,
                space_bound: Self::effective_bound(to),
                animations_on,
                prepare_switch: true,
                dragging: false,
                offset_q12: start_q12,
                target_q12: 0,
                duration_ms: self.settle_duration_ms,
                start_tick_ms: now_ms,
            };
            (Some(to), cmd)
        } else {
            let neighbor = if self.visual_q12 < 0 && self.active_space + 1 < bound {
                self.active_space + 1
            } else if self.visual_q12 > 0 && self.active_space > 0 {
                self.active_space - 1
            } else {
                self.active_space
            };
            let from = if animations_on { self.visual_q12 } else { 0 };
            self.raw_q12 = 0;
            self.visual_q12 = from;
            self.settling = animations_on && from != 0;
            self.settle_from_q12 = from;
            self.settle_neighbor = neighbor;
            self.settle_start_ms = now_ms;
            self.settle_duration_ms = if self.settling { SPRING_SETTLE_MS as u16 } else { 0 };
            let cmd = SlideCommand {
                active_space: self.active_space,
                neighbor_space: neighbor,
                space_bound: bound,
                animations_on,
                prepare_switch: false,
                dragging: false,
                offset_q12: from,
                target_q12: 0,
                duration_ms: self.settle_duration_ms,
                start_tick_ms: now_ms,
            };
            (None, cmd)
        }
    }

    /// Starts a crisp 140ms spring slide for a discrete Space switch
    /// (`Super+Left/Right`, `Super+1..5`, SilkBar Space pill click/scroll).
    pub fn start_discrete(
        &mut self,
        from: u8,
        to: u8,
        now_ms: u64,
        animations_on: bool,
    ) -> SlideCommand {
        let from = from.min(MAX_SPACES - 1);
        let to = to.min(MAX_SPACES - 1);
        self.dragging = false;
        self.raw_q12 = 0;
        if from == to {
            self.visual_q12 = 0;
            self.settling = false;
            return SlideCommand {
                active_space: to,
                neighbor_space: to,
                space_bound: Self::effective_bound(to),
                animations_on,
                prepare_switch: false,
                dragging: false,
                offset_q12: 0,
                target_q12: 0,
                duration_ms: 0,
                start_tick_ms: 0,
            };
        }
        let dir = if to > from { 1 } else { -1 };
        let start_q12 = if animations_on { dir * Q12_ONE } else { 0 };
        self.active_space = to;
        self.visual_q12 = start_q12;
        self.settling = animations_on;
        self.settle_from_q12 = start_q12;
        self.settle_neighbor = from;
        self.settle_start_ms = now_ms;
        self.settle_duration_ms = if animations_on { SPRING_SETTLE_MS as u16 } else { 0 };
        SlideCommand {
            active_space: to,
            neighbor_space: from,
            space_bound: Self::effective_bound(to),
            animations_on,
            prepare_switch: true,
            dragging: false,
            offset_q12: start_q12,
            target_q12: 0,
            duration_ms: self.settle_duration_ms,
            start_tick_ms: now_ms,
        }
    }

    /// Advances active settle tracking in Shell so subsequent continuous swipes can
    /// pick up cleanly mid-flight.
    pub fn step(&mut self, now_ms: u64) -> bool {
        if !self.settling {
            return false;
        }
        let dur = self.settle_duration_ms as u64;
        let elapsed = now_ms.saturating_sub(self.settle_start_ms);
        if dur == 0 || elapsed >= dur {
            self.visual_q12 = 0;
            self.settling = false;
            return false;
        }
        let k = spring_ease(elapsed, dur) as i64;
        self.visual_q12 = (self.settle_from_q12 as i64 * (1024 - k) / 1024) as i32;
        true
    }
}

/// Evaluates current integer pixel offset `(offset_px, active)` on Display at `now_ms`.
#[inline]
pub fn eval_slide_offset_px(cmd: &SlideCommand, screen_w: usize, now_ms: u64) -> (i32, bool) {
    if screen_w == 0 {
        return (0, false);
    }
    let w = screen_w as i64;
    if cmd.dragging || cmd.duration_ms == 0 {
        let px = ((cmd.offset_q12 as i64) * w / Q12_ONE as i64) as i32;
        return (px, cmd.dragging);
    }
    if !cmd.animations_on {
        return (0, false);
    }
    let elapsed = now_ms.saturating_sub(cmd.start_tick_ms);
    let dur = cmd.duration_ms as u64;
    if elapsed >= dur {
        return (((cmd.target_q12 as i64) * w / Q12_ONE as i64) as i32, false);
    }
    let k = spring_ease(elapsed, dur) as i64;
    let cur_q12 = cmd.offset_q12 as i64 + (cmd.target_q12 as i64 - cmd.offset_q12 as i64) * k / 1024;
    (((cur_q12 * w) / Q12_ONE as i64) as i32, true)
}

// ── 3. Frosted Noir Window Snap Zones & Preview Wire ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SnapZone {
    None = 0,
    Left = 1,
    Right = 2,
    Top = 3,
    TopLeft = 4,
    TopRight = 5,
    BottomLeft = 6,
    BottomRight = 7,
}

impl SnapZone {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Left),
            2 => Some(Self::Right),
            3 => Some(Self::Top),
            4 => Some(Self::TopLeft),
            5 => Some(Self::TopRight),
            6 => Some(Self::BottomLeft),
            7 => Some(Self::BottomRight),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::TopLeft => "top_left",
            Self::TopRight => "top_right",
            Self::BottomLeft => "bottom_left",
            Self::BottomRight => "bottom_right",
        }
    }
}

/// Detects the active snap zone when dragging a window to screen edges or corners.
/// Keeps top-center pull-to-maximize strictly above `bar_h - 8*scale` so moving a
/// window sideways along the top title band doesn't accidentally maximize.
pub fn detect_snap_zone(
    px: i32,
    py: i32,
    screen_w: i32,
    screen_h: i32,
    bar_h: i32,
    scale: u32,
) -> SnapZone {
    if screen_w <= 0 || screen_h <= bar_h || px < 0 || py < 0 {
        return SnapZone::None;
    }
    let s = scale.clamp(1, 3) as i32;
    let edge_px = 24 * s;
    let corner_x = 48 * s;
    let corner_y = 48 * s;
    let top_trigger_y = bar_h.saturating_sub(8 * s).max(1);
    let top_corner_y = bar_h + 24 * s;

    let in_left_corner_x = px < corner_x;
    let in_right_corner_x = px >= screen_w - corner_x;
    let in_left = px < edge_px;
    let in_right = px >= screen_w - edge_px;
    let in_bottom = py >= screen_h - corner_y;

    // Corners first (intuitive quarter snapping near any of the 4 screen corners).
    if in_left && py < top_corner_y {
        return SnapZone::TopLeft;
    }
    if in_right && py < top_corner_y {
        return SnapZone::TopRight;
    }
    if in_left_corner_x && in_bottom {
        return SnapZone::BottomLeft;
    }
    if in_right_corner_x && in_bottom {
        return SnapZone::BottomRight;
    }
    // Top center edge -> Maximize.
    if py < top_trigger_y {
        return SnapZone::Top;
    }
    // Left / Right halves.
    if in_left {
        return SnapZone::Left;
    }
    if in_right {
        return SnapZone::Right;
    }
    SnapZone::None
}

/// Computes `(x, y, w, h)` for snapping a window to `zone` cleanly below `bar_h`.
pub fn snap_rect(zone: SnapZone, screen_w: i32, screen_h: i32, bar_h: i32) -> Option<(i32, i32, u32, u32)> {
    if screen_w <= 2 || screen_h <= bar_h + 2 {
        return None;
    }
    let cw = screen_w as u32;
    let ch = (screen_h - bar_h) as u32;
    let half_w = cw / 2;
    let right_w = cw - half_w;
    let half_h = ch / 2;
    let bottom_h = ch - half_h;
    Some(match zone {
        SnapZone::None => return None,
        SnapZone::Left => (0, bar_h, half_w, ch),
        SnapZone::Right => (half_w as i32, bar_h, right_w, ch),
        SnapZone::Top => (0, bar_h, cw, ch),
        SnapZone::TopLeft => (0, bar_h, half_w, half_h),
        SnapZone::TopRight => (half_w as i32, bar_h, right_w, half_h),
        SnapZone::BottomLeft => (0, bar_h + half_h as i32, half_w, bottom_h),
        SnapZone::BottomRight => (half_w as i32, bar_h + half_h as i32, right_w, bottom_h),
    })
}

/// Wire message for `OP_SNAP_PREVIEW` (`Shell -> Display`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapPreviewState {
    pub zone: SnapZone,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl SnapPreviewState {
    pub const HIDDEN: Self = Self { zone: SnapZone::None, x: 0, y: 0, w: 0, h: 0 };

    pub const fn wire(self) -> [u64; 3] {
        if matches!(self.zone, SnapZone::None) || self.w == 0 || self.h == 0 {
            return [0, 0, 0];
        }
        let w0 = (self.zone as u64)
            | ((self.x as u64) << 16)
            | ((self.y as u64) << 32)
            | (1u64 << 63);
        let w1 = (self.w as u64) | ((self.h as u64) << 16);
        [w0, w1, 0]
    }

    pub fn decode(arg0: u64, arg1: u64, arg2: u64) -> Option<Self> {
        if arg0 == 0 && arg1 == 0 && arg2 == 0 {
            return Some(Self::HIDDEN);
        }
        if (arg0 >> 63) == 0 || arg2 != 0 || (arg1 >> 32) != 0 {
            return None;
        }
        if ((arg0 >> 8) & 0xFF) != 0 || ((arg0 >> 48) & 0x7FFF) != 0 {
            return None;
        }
        let zone = SnapZone::from_u8((arg0 & 0xFF) as u8)?;
        if matches!(zone, SnapZone::None) {
            return Some(Self::HIDDEN);
        }
        let x = ((arg0 >> 16) & 0xFFFF) as u16;
        let y = ((arg0 >> 32) & 0xFFFF) as u16;
        let w = (arg1 & 0xFFFF) as u16;
        let h = ((arg1 >> 16) & 0xFFFF) as u16;
        if w == 0 || h == 0 {
            return None;
        }
        Some(Self { zone, x, y, w, h })
    }
}

/// Edge spring-load tracker when dragging a window against the extreme left/right edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeCarryTracker {
    edge_dir: i8,
    since_ms: u64,
    armed_cooldown_until: u64,
}

impl EdgeCarryTracker {
    pub const fn new() -> Self {
        Self { edge_dir: 0, since_ms: 0, armed_cooldown_until: 0 }
    }

    pub fn reset(&mut self) {
        self.edge_dir = 0;
        self.since_ms = 0;
        self.armed_cooldown_until = 0;
    }

    /// Updates pointer coordinates during drag. Returns `Some(-1)` (prev Space) or
    /// `Some(+1)` (next Space) once held at the extreme edge for `SPRING_LOAD_EDGE_MS`.
    pub fn update(&mut self, px: i32, screen_w: i32, scale: u32, now_ms: u64) -> Option<i8> {
        if screen_w <= 16 {
            self.edge_dir = 0;
            return None;
        }
        let thresh = (3 * scale.clamp(1, 3) as i32).max(2);
        let dir = if px <= thresh {
            -1i8
        } else if px >= screen_w - 1 - thresh {
            1i8
        } else {
            0i8
        };
        if dir == 0 {
            self.edge_dir = 0;
            self.since_ms = 0;
            return None;
        }
        if dir != self.edge_dir {
            self.edge_dir = dir;
            self.since_ms = now_ms;
            return None;
        }
        if now_ms < self.armed_cooldown_until {
            return None;
        }
        if now_ms.saturating_sub(self.since_ms) >= SPRING_LOAD_EDGE_MS {
            self.armed_cooldown_until = now_ms.saturating_add(EDGE_CARRY_REPEAT_MS);
            return Some(dir);
        }
        None
    }
}

// ── 4. Mission Control / Overview 5-Space Bar Layout & Wire ──────────────────

/// Shared layout for the macOS Mission Control 5-Space top strip rendered inside Overview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissionControlLayout {
    pub bar_rect: Rect,
    pub card_w: usize,
    pub card_h: usize,
    pub gap: usize,
    pub cards_x0: usize,
    pub cards_y: usize,
    /// Bottom Y coordinate of the Mission Control strip; Overview window grid starts below this.
    pub content_top: usize,
}

impl MissionControlLayout {
    pub fn new(screen_w: usize, screen_h: usize, silkbar_h: usize, scale: u32) -> Option<Self> {
        let s = scale.clamp(1, 3) as usize;
        if screen_w < 320 * s || screen_h <= silkbar_h + 120 * s {
            return None;
        }
        let gap = 12 * s;
        let pad_top = 10 * s;
        let pad_bot = 8 * s;
        let max_cards_w = screen_w.saturating_sub(32 * s);
        let count = DESKTOP_SPACE_COUNT as usize;
        let card_w = ((max_cards_w.saturating_sub(gap * (count - 1))) / count).min(144 * s);
        if card_w < 44 * s {
            return None;
        }
        let card_h = (card_w * 9 / 16).clamp(36 * s, 64 * s);
        let label_band = 18 * s;
        let total_card_h = card_h + label_band;
        let strip_h = pad_top + total_card_h + pad_bot;
        let row_w = count * card_w + (count - 1) * gap;
        let cards_x0 = (screen_w - row_w) / 2;
        let cards_y = silkbar_h + pad_top;
        let bar_rect = (0, silkbar_h, screen_w, strip_h);
        Some(Self {
            bar_rect,
            card_w,
            card_h: total_card_h,
            gap,
            cards_x0,
            cards_y,
            content_top: silkbar_h + strip_h + 6 * s,
        })
    }

    pub fn space_rect(&self, index: usize) -> Option<Rect> {
        if index >= DESKTOP_SPACE_COUNT as usize {
            return None;
        }
        Some((
            self.cards_x0 + index * (self.card_w + self.gap),
            self.cards_y,
            self.card_w,
            self.card_h,
        ))
    }

    pub fn space_at(&self, x: usize, y: usize) -> Option<u8> {
        (0..DESKTOP_SPACE_COUNT as usize)
            .find(|&i| self.space_rect(i).is_some_and(|r| rect_contains(r, x, y)))
            .map(|i| i as u8)
    }
}

/// Wire message for `OP_OVERVIEW_SPACES` (`Shell -> Display`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverviewSpacesState {
    pub active_space: u8,
    pub occupied_mask: u16,
    pub hover_space: Option<u8>,
    pub drag_sid: u16,
    pub drag_x: i16,
    pub drag_y: i16,
}

impl OverviewSpacesState {
    pub const EMPTY: Self = Self {
        active_space: 0,
        occupied_mask: 0,
        hover_space: None,
        drag_sid: 0,
        drag_x: 0,
        drag_y: 0,
    };

    pub const fn wire(self) -> [u64; 3] {
        let hover = match self.hover_space {
            Some(h) if h < DESKTOP_SPACE_COUNT => h as u64,
            _ => 15u64,
        };
        let a0 = (self.active_space as u64 & 0xF)
            | (hover << 4)
            | (((self.occupied_mask & 0x1FF) as u64) << 8)
            | ((self.drag_sid as u64) << 32)
            | (1u64 << 63);
        let a1 = ((self.drag_x as u16) as u64) | (((self.drag_y as u16) as u64) << 16);
        [a0, a1, 0]
    }

    pub fn decode(arg0: u64, arg1: u64, arg2: u64) -> Option<Self> {
        if arg0 == 0 {
            return Some(Self::EMPTY);
        }
        if (arg0 >> 63) == 0 || arg2 != 0 || (arg1 >> 32) != 0 {
            return None;
        }
        let active_space = (arg0 & 0xF) as u8;
        let hover_raw = ((arg0 >> 4) & 0xF) as u8;
        if active_space >= MAX_SPACES || (hover_raw != 15 && hover_raw >= DESKTOP_SPACE_COUNT) {
            return None;
        }
        let occupied_mask = ((arg0 >> 8) & 0x1FF) as u16;
        let drag_sid = ((arg0 >> 32) & 0xFFFF) as u16;
        let drag_x = (arg1 & 0xFFFF) as u16 as i16;
        let drag_y = ((arg1 >> 16) & 0xFFFF) as u16 as i16;
        Some(Self {
            active_space,
            occupied_mask,
            hover_space: (hover_raw != 15).then_some(hover_raw),
            drag_sid,
            drag_x,
            drag_y,
        })
    }
}

// ── 5. Focused-App SilkBar Quick-Layout Icon Buttons ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum QuickLayoutBtn {
    TileLeft = 0,
    TileRight = 1,
    MaxFloat = 2,
    NextSpace = 3,
}

impl QuickLayoutBtn {
    pub const ALL: [Self; 4] = [
        Self::TileLeft,
        Self::TileRight,
        Self::MaxFloat,
        Self::NextSpace,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::TileLeft => "tile_left",
            Self::TileRight => "tile_right",
            Self::MaxFloat => "max_float",
            Self::NextSpace => "next_space",
        }
    }
}

/// Computes the 4 quick-layout button rects (`[Tile Left]`, `[Tile Right]`,
/// `[Maximize/Float]`, `[Send to Next Space]`) strictly to the right of `312 * scale`
/// (preserving pixel `(300*s, 14*s)` for `virtio_gpu_gate.py`) and strictly to the
/// left of `ws0_x - 8 * scale`. Returns empty `(0,0,0,0)` rects when `has_focus`
/// is false, the bar is in compact mode (`!ws_full`), or width is too narrow.
pub fn quick_layout_rects(
    bar_h: usize,
    scale: u32,
    ws0_x: usize,
    ws_full: bool,
    has_focus: bool,
) -> [(QuickLayoutBtn, Rect); 4] {
    let mut out = [
        (QuickLayoutBtn::TileLeft, (0, 0, 0, 0)),
        (QuickLayoutBtn::TileRight, (0, 0, 0, 0)),
        (QuickLayoutBtn::MaxFloat, (0, 0, 0, 0)),
        (QuickLayoutBtn::NextSpace, (0, 0, 0, 0)),
    ];
    if !has_focus || !ws_full {
        return out;
    }
    let s = scale.clamp(1, 3) as usize;
    let y = 3 * s;
    let h = bar_h.saturating_sub(6 * s);
    let max_right = ws0_x.saturating_sub(8 * s);
    let min_x = 314 * s;
    if h < 10 * s || max_right <= min_x {
        return out;
    }
    let avail = max_right - min_x;
    let (bw, gap) = if avail >= 4 * (22 * s) + 3 * (3 * s) {
        (22 * s, 3 * s)
    } else if avail >= 4 * (18 * s) + 3 * (2 * s) {
        (18 * s, 2 * s)
    } else {
        return out;
    };
    let mut x = min_x;
    for (i, slot) in out.iter_mut().enumerate() {
        slot.1 = (x, y, bw, h);
        let _ = i;
        x += bw + gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_and_slide_wires_round_trip() {
        for msg in [
            GestureMsg::SwipeMove { dx_q12: -312, dt_ms: 8 },
            GestureMsg::SwipeMove { dx_q12: 512, dt_ms: 16 },
            GestureMsg::SwipeEnd { vel_q12_per_s: -3200 },
            GestureMsg::SwipeLeft,
            GestureMsg::SwipeRight,
            GestureMsg::SwipeUp,
            GestureMsg::SwipeDown,
        ] {
            let [a, b, c] = msg.wire();
            assert_eq!(GestureMsg::decode(a, b, c), Some(msg));
        }
        assert_eq!(GestureMsg::decode(99, 0, 0), None);
        assert_eq!(GestureMsg::decode(4, 1, 0), None);

        let cmd = SlideCommand {
            active_space: 2,
            neighbor_space: 1,
            space_bound: 5,
            animations_on: true,
            prepare_switch: true,
            dragging: false,
            offset_q12: -1024,
            target_q12: 0,
            duration_ms: 140,
            start_tick_ms: 123456,
        };
        let [w0, w1, w2] = cmd.wire();
        assert_eq!(SlideCommand::decode(w0, w1, w2), Some(cmd));
        assert_eq!(SlideCommand::decode(0, 0, 0), Some(SlideCommand::IDLE));
    }

    #[test]
    fn space_slide_rubber_bands_at_ends_and_commits_by_pos_or_velocity() {
        let mut p = SpaceSlidePhysics::new();
        // At Space 1 (index 0), swiping right rubber-bands strictly below RUBBER_BAND_MAX_Q12.
        let c0 = p.apply_move(0, 2000, true);
        assert!(c0.offset_q12 > 0 && c0.offset_q12 < RUBBER_BAND_MAX_Q12);
        let c1 = p.apply_move(0, 2000, true);
        assert!(c1.offset_q12 > c0.offset_q12 && c1.offset_q12 < RUBBER_BAND_MAX_Q12);
        // Release rubber-band springs back without switching.
        let (committed, settle) = p.end_drag(0, 0, 100, true);
        assert_eq!(committed, None);
        assert_eq!(settle.duration_ms, SPRING_SETTLE_MS as u16);

        // At Space 1, swiping left >35% commits to Space 2.
        p.raw_q12 = 0;
        p.visual_q12 = 0;
        p.apply_move(0, -1600, true);
        let (committed, cmd) = p.end_drag(0, 0, 200, true);
        assert_eq!(committed, Some(1));
        assert!(cmd.prepare_switch);
        // Continuation starts at ~ +2496 Q12 and settles toward 0 over 140ms.
        assert!(cmd.offset_q12 > 2000 && cmd.target_q12 == 0);
        let (px_start, act0) = eval_slide_offset_px(&cmd, 1280, 200);
        let (px_mid, act1) = eval_slide_offset_px(&cmd, 1280, 270);
        let (px_end, act2) = eval_slide_offset_px(&cmd, 1280, 340);
        assert!(act0 && act1 && !act2);
        assert!(px_start > px_mid && px_mid > px_end && px_end == 0);

        // Velocity flick commits with modest displacement.
        let mut pf = SpaceSlidePhysics::new();
        pf.apply_move(1, -300, true);
        let (committed, _) = pf.end_drag(1, -2800, 300, true);
        assert_eq!(committed, Some(2));
    }

    #[test]
    fn space_slide_rubber_bands_at_space_5() {
        let mut p = SpaceSlidePhysics::new();
        let c = p.apply_move(4, -3000, true);
        assert!(c.offset_q12 < 0 && c.offset_q12 > -RUBBER_BAND_MAX_Q12);
        let (committed, _) = p.end_drag(4, -4000, 500, true);
        assert_eq!(committed, None);
    }

    #[test]
    fn snap_zones_cover_halves_top_and_quarters() {
        assert_eq!(detect_snap_zone(10, 400, 1280, 800, 28, 1), SnapZone::Left);
        assert_eq!(detect_snap_zone(1270, 400, 1280, 800, 28, 1), SnapZone::Right);
        assert_eq!(detect_snap_zone(640, 10, 1280, 800, 28, 1), SnapZone::Top);
        assert_eq!(detect_snap_zone(640, 35, 1280, 800, 28, 1), SnapZone::None);
        assert_eq!(detect_snap_zone(5, 30, 1280, 800, 28, 1), SnapZone::TopLeft);
        assert_eq!(detect_snap_zone(1275, 30, 1280, 800, 28, 1), SnapZone::TopRight);
        assert_eq!(detect_snap_zone(20, 780, 1280, 800, 28, 1), SnapZone::BottomLeft);
        assert_eq!(detect_snap_zone(1260, 780, 1280, 800, 28, 1), SnapZone::BottomRight);

        assert_eq!(snap_rect(SnapZone::Left, 1280, 800, 28), Some((0, 28, 640, 772)));
        assert_eq!(snap_rect(SnapZone::BottomRight, 1280, 800, 28), Some((640, 414, 640, 386)));
    }

    #[test]
    fn mission_control_and_quick_layout_rects_are_well_bounded() {
        for scale in 1..=3u32 {
            let s = scale as usize;
            let mc = MissionControlLayout::new(1280 * s, 800 * s, 28 * s, scale).unwrap();
            for i in 0..5 {
                let r = mc.space_rect(i).unwrap();
                assert_eq!(mc.space_at(r.0 + r.2 / 2, r.1 + r.3 / 2), Some(i as u8));
            }
            let btns = quick_layout_rects(28 * s, scale, 532 * s, true, true);
            for &(_, r) in &btns {
                assert!(r.2 > 0 && r.3 > 0);
                // Strictly preserve (300*s, 14*s) untouched SilkBar glass for virtio_gpu_gate!
                assert!(r.0 >= 314 * s);
                assert!(!rect_contains(r, 300 * s, 14 * s));
                assert!(r.0 + r.2 <= 524 * s);
            }
        }
    }

    #[test]
    fn discrete_spring_settle_and_midflight_pickup() {
        let mut p = SpaceSlidePhysics::new();
        let cmd = p.start_discrete(0, 1, 1000, true);
        assert_eq!(cmd.duration_ms, SPRING_SETTLE_MS as u16);
        assert_eq!(cmd.offset_q12, Q12_ONE);
        assert!(p.step(1070));
        assert!(p.visual_q12 > 0 && p.visual_q12 < Q12_ONE);
        // Interrupted mid-flight by 3-finger swipe picks up cleanly from `visual_q12`.
        let cont = p.apply_move(1, -200, true);
        assert!(cont.dragging);
        assert!(!p.settling);
        assert!(cont.offset_q12 < Q12_ONE && cont.offset_q12 > 0);
        // With animations off, discrete switch settles instantaneously (0ms, no slide offset).
        let no_anim = p.start_discrete(1, 2, 2000, false);
        assert_eq!(no_anim.duration_ms, 0);
        assert_eq!(no_anim.offset_q12, 0);
    }

    #[test]
    fn edge_carry_spring_loads_at_350ms_and_resets_cleanly() {
        let mut tr = EdgeCarryTracker::new();
        assert_eq!(tr.update(1, 1280, 1, 100), None);
        assert_eq!(tr.update(1, 1280, 1, 400), None);
        // Fires after SPRING_LOAD_EDGE_MS (350ms).
        assert_eq!(tr.update(1, 1280, 1, 450), Some(-1));
        // Cooldown prevents instant double-trigger.
        assert_eq!(tr.update(1, 1280, 1, 500), None);
        // Leaving edge resets timer cleanly.
        assert_eq!(tr.update(640, 1280, 1, 600), None);
        assert_eq!(tr.update(1278, 1280, 1, 950), None);
        assert_eq!(tr.update(1278, 1280, 1, 1300), Some(1));
    }

    #[test]
    fn snap_preview_and_overview_spaces_wires_round_trip() {
        let snap = SnapPreviewState { zone: SnapZone::TopRight, x: 640, y: 28, w: 640, h: 386 };
        let [s0, s1, s2] = snap.wire();
        assert_eq!(SnapPreviewState::decode(s0, s1, s2), Some(snap));
        let [h0, h1, h2] = SnapPreviewState::HIDDEN.wire();
        assert_eq!(SnapPreviewState::decode(h0, h1, h2), Some(SnapPreviewState::HIDDEN));

        let ov = OverviewSpacesState {
            active_space: 2,
            occupied_mask: 0b00101,
            hover_space: Some(4),
            drag_sid: 201,
            drag_x: 720,
            drag_y: 85,
        };
        let [o0, o1, o2] = ov.wire();
        assert_eq!(OverviewSpacesState::decode(o0, o1, o2), Some(ov));
        assert_eq!(OverviewSpacesState::decode(0, 0, 0), Some(OverviewSpacesState::EMPTY));
    }
}
