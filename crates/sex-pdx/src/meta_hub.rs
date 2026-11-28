// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Shared zero-heap protocol, geometry, hit-testing, and quick-command parser
//! for the SexOS Meta Control Center & Super-Launcher Bento Hub.
//!
//! Shell owns all live hardware/server state and drives Display over
//! [`OP_META_HUB`]. Both Shell (pointer hit-testing & continuous slider drag)
//! and Display (frosted Obsidian Glass rendering) share [`Layout`] so every
//! drawn tile, toggle pill, and slider matches pointer coordinates 1:1.

/// Shell -> Display PDX opcode for Meta Control Hub packets.
pub const OP_META_HUB: u64 = 0x302;

/// Packet discriminator stored in bits `60..64` of `arg0`.
pub const PACKET_STATE: u64 = 0;
pub const PACKET_SSID: u64 = 1;
pub const PACKET_ANCHOR: u64 = 2;

/// Anchor presentation modes.
pub const MODE_COMMAND_PALETTE: u8 = 0;
pub const MODE_SPOTLIGHT: u8 = 1;

/// Power / thermal performance presets (`power_mode`).
pub const POWER_SAVER: u8 = 0;
pub const POWER_BALANCED: u8 = 1;
pub const POWER_PERFORMANCE: u8 = 2;
pub const POWER_GMODE: u8 = 3;

/// Number of curated macOS Noir accents (`0..=3`).
pub const ACCENT_COUNT: u8 = 4;
pub const NIGHT_LEVELS: u8 = 4;

pub const ACCENT_NAMES: [&str; 4] = ["Rose", "Graphite", "Lavender", "Ice"];
pub const POWER_NAMES: [&str; 4] = ["Quiet", "Balanced", "Perf", "G-Mode"];
pub const NIGHT_NAMES: [&str; 4] = ["Off", "Warm", "Amber", "Deep"];

/// Quick-launch dock action count and labels.
pub const QUICK_LAUNCH_COUNT: usize = 6;
pub const QUICK_LAUNCH_LABELS: [&str; QUICK_LAUNCH_COUNT] = [
    "Spindle", "Settings", "SexShop", "Wi-Fi", "Lock", "Snip",
];

/// Screen rectangle `(x, y, width, height)` in device pixels.
pub type Rect = (usize, usize, usize, usize);

/// True when `(px, py)` lies inside `r`.
#[inline]
pub const fn rect_contains(r: Rect, px: usize, py: usize) -> bool {
    px >= r.0 && py >= r.1 && px < r.0 + r.2 && py < r.1 + r.3
}

/// True when rectangles `a` and `b` have zero overlapping pixels.
#[inline]
pub const fn rects_disjoint(a: Rect, b: Rect) -> bool {
    a.2 == 0
        || a.3 == 0
        || b.2 == 0
        || b.3 == 0
        || a.0 + a.2 <= b.0
        || b.0 + b.2 <= a.0
        || a.1 + a.3 <= b.1
        || b.1 + b.3 <= a.1
}

/// Slider currently being dragged with the left mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DragSlider {
    None = 0,
    Brightness = 1,
    Volume = 2,
}

impl DragSlider {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Brightness,
            2 => Self::Volume,
            _ => Self::None,
        }
    }
}

/// Interactive targets inside the Meta Control Hub Bento card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HitTarget {
    Outside = 0,
    Panel = 1,
    SpacePill = 2,
    TileWifi = 3,
    TileWifiExpand = 4,
    TileBluetooth = 5,
    TilePower = 6,
    TileNightLight = 7,
    TileAccent = 8,
    TileDnd = 9,
    BtnLock = 10,
    BtnShot = 11,
    SliderBrightness = 12,
    BtnSpeakerMute = 13,
    SliderVolume = 14,
    BtnMicMute = 15,
    MediaPrev = 16,
    MediaPlayPause = 17,
    MediaNext = 18,
    QuickLaunch0 = 19,
    QuickLaunch1 = 20,
    QuickLaunch2 = 21,
    QuickLaunch3 = 22,
    QuickLaunch4 = 23,
    QuickLaunch5 = 24,
}

impl HitTarget {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Panel,
            2 => Self::SpacePill,
            3 => Self::TileWifi,
            4 => Self::TileWifiExpand,
            5 => Self::TileBluetooth,
            6 => Self::TilePower,
            7 => Self::TileNightLight,
            8 => Self::TileAccent,
            9 => Self::TileDnd,
            10 => Self::BtnLock,
            11 => Self::BtnShot,
            12 => Self::SliderBrightness,
            13 => Self::BtnSpeakerMute,
            14 => Self::SliderVolume,
            15 => Self::BtnMicMute,
            16 => Self::MediaPrev,
            17 => Self::MediaPlayPause,
            18 => Self::MediaNext,
            19 => Self::QuickLaunch0,
            20 => Self::QuickLaunch1,
            21 => Self::QuickLaunch2,
            22 => Self::QuickLaunch3,
            23 => Self::QuickLaunch4,
            24 => Self::QuickLaunch5,
            _ => Self::Outside,
        }
    }

    pub const fn quick_launch(idx: usize) -> Self {
        match idx {
            0 => Self::QuickLaunch0,
            1 => Self::QuickLaunch1,
            2 => Self::QuickLaunch2,
            3 => Self::QuickLaunch3,
            4 => Self::QuickLaunch4,
            _ => Self::QuickLaunch5,
        }
    }

    pub const fn quick_launch_index(self) -> Option<usize> {
        match self {
            Self::QuickLaunch0 => Some(0),
            Self::QuickLaunch1 => Some(1),
            Self::QuickLaunch2 => Some(2),
            Self::QuickLaunch3 => Some(3),
            Self::QuickLaunch4 => Some(4),
            Self::QuickLaunch5 => Some(5),
            _ => None,
        }
    }

    /// Hover code passed over wire (`0` means no interactive control hovered).
    pub const fn hover_code(self) -> u8 {
        match self {
            Self::Outside | Self::Panel => 0,
            _ => self as u8,
        }
    }
}

/// Complete Control Center snapshot shared between Shell and Display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaHubState {
    pub visible: bool,
    pub mode: u8,
    pub anchor: (i16, i16, u16, u16),
    pub brightness_pct: u8,
    pub volume_pct: u8,
    pub speaker_muted: bool,
    pub mic_muted: bool,
    pub mic_gain_pct: u8,
    pub wifi_enabled: bool,
    pub wifi_connected: bool,
    pub wifi_rssi_bars: u8,
    pub ssid: [u8; 16],
    pub ssid_len: u8,
    pub bt_enabled: bool,
    pub bt_available: bool,
    pub bt_connected_count: u8,
    pub power_mode: u8,
    pub night_light_level: u8,
    pub noir_accent: u8,
    pub dnd_enabled: bool,
    pub active_space: u8,
    pub open_windows: u8,
    pub parked_apps: u8,
    pub media_playing: bool,
    pub battery_pct: u8,
    pub battery_known: bool,
    pub charging: bool,
    pub hover_target: u8,
    pub active_drag_slider: DragSlider,
}

impl MetaHubState {
    pub const DEFAULT: Self = Self {
        visible: false,
        mode: MODE_COMMAND_PALETTE,
        anchor: (0, 0, 0, 0),
        brightness_pct: 80,
        volume_pct: 75,
        speaker_muted: false,
        mic_muted: false,
        mic_gain_pct: 100,
        wifi_enabled: false,
        wifi_connected: false,
        wifi_rssi_bars: 0,
        ssid: [0; 16],
        ssid_len: 0,
        bt_enabled: false,
        bt_available: false,
        bt_connected_count: 0,
        power_mode: POWER_BALANCED,
        night_light_level: 0,
        noir_accent: 0,
        dnd_enabled: false,
        active_space: 0,
        open_windows: 0,
        parked_apps: 0,
        media_playing: false,
        battery_pct: 0,
        battery_known: false,
        charging: false,
        hover_target: 0,
        active_drag_slider: DragSlider::None,
    };

    /// Clean printable ASCII slice of the connected Wi-Fi SSID (up to 16 bytes).
    pub fn ssid_str(&self) -> &str {
        let len = (self.ssid_len as usize).min(16);
        core::str::from_utf8(&self.ssid[..len]).unwrap_or("")
    }

    /// Sets `ssid` sanitizing non-printable bytes to spaces/omitted.
    pub fn set_ssid(&mut self, raw: &[u8]) {
        self.ssid = [0; 16];
        let mut n = 0usize;
        for &b in raw {
            if n == 16 {
                break;
            }
            if (0x20..=0x7E).contains(&b) {
                self.ssid[n] = b;
                n += 1;
            }
        }
        self.ssid_len = n as u8;
    }

    /// Packs `(PACKET_SSID, w1, w2)` for `OP_META_HUB`.
    pub fn pack_ssid_telemetry(&self) -> (u64, u64, u64) {
        let slen = (self.ssid_len.min(16)) as u64;
        let arg0 = (PACKET_SSID << 60)
            | slen
            | ((self.bt_connected_count as u64) << 8)
            | ((self.open_windows as u64) << 16)
            | ((self.parked_apps as u64) << 24);
        let mut lo = [0u8; 8];
        let mut hi = [0u8; 8];
        lo.copy_from_slice(&self.ssid[..8]);
        hi.copy_from_slice(&self.ssid[8..16]);
        (arg0, u64::from_le_bytes(lo), u64::from_le_bytes(hi))
    }

    /// Packs `(PACKET_ANCHOR, w1, w2)` for `OP_META_HUB`.
    pub fn pack_anchor(&self) -> (u64, u64, u64) {
        let arg0 = (PACKET_ANCHOR << 60) | ((self.mode & 1) as u64);
        let arg1 = (self.anchor.0 as u16 as u64)
            | ((self.anchor.1 as u16 as u64) << 16)
            | ((self.anchor.2 as u64) << 32)
            | ((self.anchor.3 as u64) << 48);
        let arg2 = (self.battery_pct.min(100) as u64)
            | ((self.charging as u64) << 8)
            | ((self.battery_known as u64) << 9);
        (arg0, arg1, arg2)
    }

    /// Packs `(PACKET_STATE, 0, 0)` for `OP_META_HUB`. When `!visible`, emits a fast single word `0`.
    pub fn pack_state(&self) -> (u64, u64, u64) {
        if !self.visible {
            return (0, 0, 0);
        }
        let w = 1u64
            | (((self.mode & 1) as u64) << 1)
            | (((self.brightness_pct.min(100) & 0x7F) as u64) << 2)
            | (((self.volume_pct.min(100) & 0x7F) as u64) << 9)
            | ((self.speaker_muted as u64) << 16)
            | ((self.mic_muted as u64) << 17)
            | (((self.mic_gain_pct.min(100) & 0x7F) as u64) << 18)
            | ((self.wifi_enabled as u64) << 25)
            | ((self.wifi_connected as u64) << 26)
            | (((self.wifi_rssi_bars.min(4) & 0x7) as u64) << 27)
            | ((self.bt_enabled as u64) << 30)
            | (((self.power_mode & 0x3) as u64) << 31)
            | (((self.night_light_level & 0x3) as u64) << 33)
            | (((self.noir_accent & 0x3) as u64) << 35)
            | ((self.dnd_enabled as u64) << 37)
            | (((self.active_space.min(4) & 0x7) as u64) << 38)
            | ((self.media_playing as u64) << 41)
            | (((self.hover_target & 0x3F) as u64) << 42)
            | (((self.active_drag_slider as u64) & 0x7) << 48)
            | ((self.bt_available as u64) << 51);
        (w, 0, 0)
    }
}

/// Stateful Display-side packet assembler for [`OP_META_HUB`].
pub struct Receiver {
    pub current: MetaHubState,
}

impl Receiver {
    pub const fn new() -> Self {
        Self { current: MetaHubState::DEFAULT }
    }

    /// Processes an incoming `OP_META_HUB` message.
    /// Returns `Some(true)` when visible visual state changed, `Some(false)` when accepted
    /// with no visible delta, or `None` on malformed packet.
    pub fn receive(&mut self, arg0: u64, arg1: u64, arg2: u64) -> Option<bool> {
        let kind = arg0 >> 60;
        let prev = self.current;
        match kind {
            PACKET_SSID => {
                let len = (arg0 & 0xFF) as u8;
                if len > 16 || (arg0 >> 32) & 0x0FFF_FFFF != 0 {
                    return None;
                }
                let mut buf = [0u8; 16];
                buf[..8].copy_from_slice(&arg1.to_le_bytes());
                buf[8..16].copy_from_slice(&arg2.to_le_bytes());
                self.current.set_ssid(&buf[..len as usize]);
                self.current.bt_connected_count = ((arg0 >> 8) & 0xFF) as u8;
                self.current.open_windows = ((arg0 >> 16) & 0xFF) as u8;
                self.current.parked_apps = ((arg0 >> 24) & 0xFF) as u8;
                Some(self.current.visible && self.current != prev)
            }
            PACKET_ANCHOR => {
                if (arg0 & 0x0FFF_FFFF_FFFF_FFFE) != 0 || (arg2 >> 10) != 0 {
                    return None;
                }
                self.current.mode = (arg0 & 1) as u8;
                self.current.anchor = (
                    (arg1 & 0xFFFF) as u16 as i16,
                    ((arg1 >> 16) & 0xFFFF) as u16 as i16,
                    ((arg1 >> 32) & 0xFFFF) as u16,
                    ((arg1 >> 48) & 0xFFFF) as u16,
                );
                self.current.battery_pct = ((arg2 & 0xFF) as u8).min(100);
                self.current.charging = ((arg2 >> 8) & 1) != 0;
                self.current.battery_known = ((arg2 >> 9) & 1) != 0;
                Some(self.current.visible && self.current != prev)
            }
            PACKET_STATE => {
                if arg0 == 0 {
                    if arg1 != 0 || arg2 != 0 {
                        return None;
                    }
                    self.current.visible = false;
                    self.current.hover_target = 0;
                    self.current.active_drag_slider = DragSlider::None;
                    return Some(prev.visible);
                }
                if (arg0 >> 52) != 0 || (arg0 & 1) == 0 || arg1 != 0 || arg2 != 0 {
                    return None;
                }
                let drag = ((arg0 >> 48) & 0x7) as u8;
                if drag > 2 {
                    return None;
                }
                let bright = ((arg0 >> 2) & 0x7F) as u8;
                let vol = ((arg0 >> 9) & 0x7F) as u8;
                let mic = ((arg0 >> 18) & 0x7F) as u8;
                let bars = ((arg0 >> 27) & 0x7) as u8;
                let space = ((arg0 >> 38) & 0x7) as u8;
                let hover = ((arg0 >> 42) & 0x3F) as u8;
                if bright > 100 || vol > 100 || mic > 100 || bars > 4 || space > 4 || hover > 24 {
                    return None;
                }
                self.current.visible = true;
                self.current.mode = ((arg0 >> 1) & 1) as u8;
                self.current.brightness_pct = bright;
                self.current.volume_pct = vol;
                self.current.speaker_muted = ((arg0 >> 16) & 1) != 0;
                self.current.mic_muted = ((arg0 >> 17) & 1) != 0;
                self.current.mic_gain_pct = mic;
                self.current.wifi_enabled = ((arg0 >> 25) & 1) != 0;
                self.current.wifi_connected = ((arg0 >> 26) & 1) != 0;
                self.current.wifi_rssi_bars = bars;
                self.current.bt_enabled = ((arg0 >> 30) & 1) != 0;
                self.current.power_mode = ((arg0 >> 31) & 0x3) as u8;
                self.current.night_light_level = ((arg0 >> 33) & 0x3) as u8;
                self.current.noir_accent = ((arg0 >> 35) & 0x3) as u8;
                self.current.dnd_enabled = ((arg0 >> 37) & 1) != 0;
                self.current.active_space = space;
                self.current.media_playing = ((arg0 >> 41) & 1) != 0;
                self.current.hover_target = hover;
                self.current.active_drag_slider = DragSlider::from_u8(drag);
                self.current.bt_available = ((arg0 >> 51) & 1) != 0;
                Some(self.current != prev)
            }
            _ => None,
        }
    }
}

/// Shared non-overlapping Bento layout used by both Shell hit-testing and Display rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub scale: u32,
    pub compact: bool,
    pub panel: Rect,
    pub header: Rect,
    pub space_pill: Rect,
    pub tiles: [Rect; 6],
    pub tile_wifi_expand: Rect,
    pub tile_dnd_toggle: Rect,
    pub btn_lock: Rect,
    pub btn_shot: Rect,
    pub slider_brightness: Rect,
    pub volume_pill: Rect,
    pub btn_speaker_mute: Rect,
    pub slider_volume: Rect,
    pub btn_mic_mute: Rect,
    pub media_card: Rect,
    pub media_prev: Rect,
    pub media_play_pause: Rect,
    pub media_next: Rect,
    pub quick_actions: [Rect; QUICK_LAUNCH_COUNT],
}

impl Layout {
    /// Computes an adaptive non-overlapping panel beside `anchor` inside `[0..screen_w, bar_h..screen_h]`.
    /// Returns `None` if no safe non-overlapping region >= `216x320` logical px exists.
    pub fn compute(
        screen_w: usize,
        screen_h: usize,
        bar_h: usize,
        scale: u32,
        mode: u8,
        anchor: (i16, i16, u16, u16),
    ) -> Option<Self> {
        let sc = scale.clamp(1, 3);
        let s = sc as usize;
        let edge = 8 * s;
        let gap = if mode == MODE_SPOTLIGHT { 10 * s } else { 12 * s };
        let top_min = bar_h + edge;
        let bot_max = screen_h.saturating_sub(edge);
        let right_max = screen_w.saturating_sub(edge);
        if right_max <= edge || bot_max <= top_min {
            return None;
        }
        let avail_h = bot_max - top_min;

        let ax = (anchor.0 as isize).max(0) as usize;
        let ay = (anchor.1 as isize).max(0) as usize;
        let aw = anchor.2 as usize;
        let ah = anchor.3 as usize;
        let anchor_rect: Rect = (ax, ay, aw, ah);

        let min_w = 216 * s;
        let pref_w = 348 * s;
        let min_h = 332 * s;
        let pref_h = 434 * s;

        let pick_y = |ph: usize| -> usize {
            let cy = if ah > 0 { ay + ah / 2 } else { top_min + avail_h / 2 };
            cy.saturating_sub(ph / 2)
                .max(top_min)
                .min(bot_max.saturating_sub(ph))
        };

        let mut panel: Option<Rect> = None;

        // 1. Preferred side-by-side: cleanly to the RIGHT of Command Palette / Spotlight.
        let right_x = ax.saturating_add(aw).saturating_add(gap);
        if right_max > right_x && right_max - right_x >= min_w && avail_h >= min_h {
            let pw = (right_max - right_x).min(pref_w);
            let ph = avail_h.min(pref_h);
            let py = pick_y(ph);
            let cand = (right_x, py, pw, ph);
            if rects_disjoint(cand, anchor_rect) {
                panel = Some(cand);
            }
        }

        // 2. Fallback: cleanly to the LEFT of anchor if right margin is constrained.
        if panel.is_none() {
            let left_end = ax.saturating_sub(gap);
            if left_end > edge && left_end - edge >= min_w && avail_h >= min_h {
                let pw = (left_end - edge).min(pref_w);
                let ph = avail_h.min(pref_h);
                let px = left_end - pw;
                let py = pick_y(ph);
                let cand = (px, py, pw, ph);
                if rects_disjoint(cand, anchor_rect) {
                    panel = Some(cand);
                }
            }
        }

        // 3. Fallback: cleanly BELOW anchor on narrow portrait/small screens.
        if panel.is_none() {
            let below_y = ay.saturating_add(ah).saturating_add(gap);
            let full_w = right_max.saturating_sub(edge);
            if bot_max > below_y && bot_max - below_y >= min_h && full_w >= min_w {
                let pw = full_w.min(440 * s);
                let ph = (bot_max - below_y).min(pref_h);
                let px = edge + (full_w - pw) / 2;
                let cand = (px, below_y, pw, ph);
                if rects_disjoint(cand, anchor_rect) {
                    panel = Some(cand);
                }
            }
        }

        let panel = panel?;
        let (px, py, pw, ph) = panel;
        let compact = pw < 280 * s || ph < 396 * s;

        let pad_x = if pw < 270 * s { 10 * s } else { 14 * s };
        let pad_y = if ph < 380 * s { 8 * s } else { 12 * s };
        let inner_x = px + pad_x;
        let inner_w = pw.saturating_sub(2 * pad_x);

        // Compute row heights scaled dynamically to guarantee zero vertical overflow.
        let hdr_h = if ph < 380 * s { 22 * s } else { 26 * s };
        let gap_y = if ph < 380 * s { 6 * s } else { 9 * s };
        let tile_gap = if pw < 270 * s { 6 * s } else { 8 * s };
        let slider_h = if ph < 380 * s { 28 * s } else { 34 * s };
        let media_h = if ph < 380 * s { 34 * s } else { 42 * s };
        let dock_h = if ph < 380 * s { 40 * s } else { 48 * s };
        let fixed_sum = pad_y * 2 + hdr_h + gap_y * 6 + tile_gap * 2 + slider_h * 2 + media_h + dock_h;
        let rem_for_tiles = ph.saturating_sub(fixed_sum);
        let tile_h = (rem_for_tiles / 3).clamp(36 * s, 54 * s);

        let mut cy = py + pad_y;
        let header = (inner_x, cy, inner_w, hdr_h);
        let sp_w = if pw < 270 * s { 54 * s } else { 64 * s };
        let super_w = if pw < 270 * s { 42 * s } else { 48 * s };
        let space_pill = (
            inner_x + inner_w.saturating_sub(sp_w + super_w + 4 * s),
            cy + 2 * s,
            sp_w,
            hdr_h.saturating_sub(4 * s),
        );
        cy += hdr_h + gap_y;

        // 2x3 Quick-Toggle Bento Pills.
        let tile_w = inner_w.saturating_sub(tile_gap) / 2;
        let mut tiles = [(0, 0, 0, 0); 6];
        for idx in 0..6 {
            let col = idx % 2;
            let row = idx / 2;
            let tx = inner_x + col * (tile_w + tile_gap);
            let ty = cy + row * (tile_h + tile_gap);
            tiles[idx] = (tx, ty, tile_w, tile_h);
        }

        // Chevron hotspot on top-right of Wi-Fi tile to open full Wi-Fi scanner.
        let wt = tiles[0];
        let exp_w = (22 * s).min(wt.2 / 3);
        let exp_h = (22 * s).min(wt.3 / 2);
        let tile_wifi_expand = (
            wt.0 + wt.2.saturating_sub(exp_w + 4 * s),
            wt.1 + 4 * s,
            exp_w,
            exp_h,
        );

        // Split Focus / DND card: left area toggles DND, right side holds Lock + Shot mini buttons.
        let ft = tiles[5];
        let (tile_dnd_toggle, btn_lock, btn_shot) = if ft.2 >= 134 * s {
            let bsz = (24 * s).min(ft.3.saturating_sub(8 * s));
            let b_gap = 4 * s;
            let shot_x = ft.0 + ft.2.saturating_sub(bsz + 5 * s);
            let lock_x = shot_x.saturating_sub(bsz + b_gap);
            let by = ft.1 + ft.3.saturating_sub(bsz) / 2;
            let dnd_w = lock_x.saturating_sub(ft.0 + 3 * s);
            ((ft.0, ft.1, dnd_w, ft.3), (lock_x, by, bsz, bsz), (shot_x, by, bsz, bsz))
        } else {
            // Narrow layout: stack 2 mini buttons vertically on far right of Focus tile.
            let bsz = (ft.3.saturating_sub(8 * s) / 2).clamp(16 * s, 21 * s);
            let bx = ft.0 + ft.2.saturating_sub(bsz + 4 * s);
            let top_y = ft.1 + ft.3.saturating_sub(bsz * 2 + 2 * s) / 2;
            let dnd_w = bx.saturating_sub(ft.0 + 2 * s);
            (
                (ft.0, ft.1, dnd_w, ft.3),
                (bx, top_y, bsz, bsz),
                (bx, top_y + bsz + 2 * s, bsz, bsz),
            )
        };

        cy += tile_h * 3 + tile_gap * 2 + gap_y;

        // Display Brightness Pill Slider.
        let slider_brightness = (inner_x, cy, inner_w, slider_h);
        cy += slider_h + gap_y;

        // Sound Volume Pill Slider + Speaker Mute Cap + Dedicated Mic Mute Pill.
        let mic_gap = 6 * s;
        let mic_w = if pw < 270 * s { 44 * s } else { 54 * s };
        let vol_w = inner_w.saturating_sub(mic_w + mic_gap);
        let volume_pill = (inner_x, cy, vol_w, slider_h);
        let cap_w = (32 * s).min(vol_w / 3);
        let btn_speaker_mute = (inner_x, cy, cap_w, slider_h);
        let slider_volume = (inner_x + cap_w, cy, vol_w.saturating_sub(cap_w), slider_h);
        let btn_mic_mute = (inner_x + vol_w + mic_gap, cy, mic_w, slider_h);
        cy += slider_h + gap_y;

        // Media Transport Card.
        let media_card = (inner_x, cy, inner_w, media_h);
        let mbtn_sz = (26 * s).min(media_h.saturating_sub(8 * s));
        let mbtn_gap = 5 * s;
        let m_y = cy + media_h.saturating_sub(mbtn_sz) / 2;
        let next_x = inner_x + inner_w.saturating_sub(mbtn_sz + 8 * s);
        let play_x = next_x.saturating_sub(mbtn_sz + mbtn_gap);
        let prev_x = play_x.saturating_sub(mbtn_sz + mbtn_gap);
        let media_prev = (prev_x, m_y, mbtn_sz, mbtn_sz);
        let media_play_pause = (play_x, m_y, mbtn_sz, mbtn_sz);
        let media_next = (next_x, m_y, mbtn_sz, mbtn_sz);
        cy += media_h + gap_y;

        // Quick-Launcher 6-Tile Strip.
        let q_gap = if pw < 270 * s { 4 * s } else { 5 * s };
        let q_w = inner_w.saturating_sub(q_gap * (QUICK_LAUNCH_COUNT - 1)) / QUICK_LAUNCH_COUNT;
        let q_h = dock_h.min((py + ph).saturating_sub(cy + pad_y).max(dock_h));
        let mut quick_actions = [(0, 0, 0, 0); QUICK_LAUNCH_COUNT];
        for i in 0..QUICK_LAUNCH_COUNT {
            quick_actions[i] = (inner_x + i * (q_w + q_gap), cy, q_w, q_h);
        }

        Some(Self {
            scale: sc,
            compact,
            panel,
            header,
            space_pill,
            tiles,
            tile_wifi_expand,
            tile_dnd_toggle,
            btn_lock,
            btn_shot,
            slider_brightness,
            volume_pill,
            btn_speaker_mute,
            slider_volume,
            btn_mic_mute,
            media_card,
            media_prev,
            media_play_pause,
            media_next,
            quick_actions,
        })
    }

    /// Hit-tests pointer `(px, py)` against the entire Meta Control Hub.
    pub fn hit(&self, px: i32, py: i32) -> HitTarget {
        if px < 0 || py < 0 {
            return HitTarget::Outside;
        }
        let (ux, uy) = (px as usize, py as usize);
        if !rect_contains(self.panel, ux, uy) {
            return HitTarget::Outside;
        }
        if rect_contains(self.space_pill, ux, uy) {
            return HitTarget::SpacePill;
        }
        if rect_contains(self.tile_wifi_expand, ux, uy) {
            return HitTarget::TileWifiExpand;
        }
        if rect_contains(self.tiles[0], ux, uy) {
            return HitTarget::TileWifi;
        }
        if rect_contains(self.tiles[1], ux, uy) {
            return HitTarget::TileBluetooth;
        }
        if rect_contains(self.tiles[2], ux, uy) {
            return HitTarget::TilePower;
        }
        if rect_contains(self.tiles[3], ux, uy) {
            return HitTarget::TileNightLight;
        }
        if rect_contains(self.tiles[4], ux, uy) {
            return HitTarget::TileAccent;
        }
        if rect_contains(self.btn_lock, ux, uy) {
            return HitTarget::BtnLock;
        }
        if rect_contains(self.btn_shot, ux, uy) {
            return HitTarget::BtnShot;
        }
        if rect_contains(self.tile_dnd_toggle, ux, uy) || rect_contains(self.tiles[5], ux, uy) {
            return HitTarget::TileDnd;
        }
        if rect_contains(self.slider_brightness, ux, uy) {
            return HitTarget::SliderBrightness;
        }
        if rect_contains(self.btn_speaker_mute, ux, uy) {
            return HitTarget::BtnSpeakerMute;
        }
        if rect_contains(self.slider_volume, ux, uy) {
            return HitTarget::SliderVolume;
        }
        if rect_contains(self.btn_mic_mute, ux, uy) {
            return HitTarget::BtnMicMute;
        }
        if rect_contains(self.media_prev, ux, uy) {
            return HitTarget::MediaPrev;
        }
        if rect_contains(self.media_play_pause, ux, uy) {
            return HitTarget::MediaPlayPause;
        }
        if rect_contains(self.media_next, ux, uy) {
            return HitTarget::MediaNext;
        }
        for (i, &r) in self.quick_actions.iter().enumerate() {
            if rect_contains(r, ux, uy) {
                return HitTarget::quick_launch(i);
            }
        }
        HitTarget::Panel
    }

    /// Maps an X pointer coordinate inside `slider_rect` linearly into `0..=100`.
    pub fn percent_at(slider: Rect, px: i32) -> u8 {
        if slider.2 <= 1 {
            return 0;
        }
        let pad = (slider.3 / 4).min(slider.2 / 4);
        let usable_start = slider.0 + pad;
        let usable_w = slider.2.saturating_sub(pad * 2).max(1);
        let ux = px.max(0) as usize;
        if ux <= usable_start {
            return 0;
        }
        let rel = ux - usable_start;
        if rel >= usable_w {
            return 100;
        }
        ((rel * 100 + usable_w / 2) / usable_w).min(100) as u8
    }
}

/// Super-user text control commands executable by typing directly in Meta Launcher or Spotlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickCommand {
    VolumeSet(u8),
    VolumeStep(i8),
    MuteSet(bool),
    MicMuteToggle,
    MicMuteSet(bool),
    BrightnessSet(u8),
    BrightnessStep(i8),
    WifiSet(bool),
    WifiScan,
    BtSet(bool),
    BtPanel,
    PowerMode(u8),
    NightLightCycle,
    NightLightSet(u8),
    NoirAccentCycle,
    NoirAccentSet(u8),
    DndToggle,
    DndSet(bool),
    LockScreen,
    Screenshot,
    SwitchSpace(u8),
}

fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

fn strip_prefix_ci<'a>(s: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    if s.len() >= prefix.len() && eq_ci(&s[..prefix.len()], prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn trim_ws(mut s: &[u8]) -> &[u8] {
    while let Some(&b) = s.first() {
        if b == b' ' { s = &s[1..]; } else { break; }
    }
    while let Some(&b) = s.last() {
        if b == b' ' { s = &s[..s.len() - 1]; } else { break; }
    }
    s
}

fn parse_u8_pct(raw: &[u8]) -> Option<u8> {
    let mut b = trim_ws(raw);
    if b.ends_with(b"%") {
        b = trim_ws(&b[..b.len() - 1]);
    }
    if b.is_empty() || b.len() > 3 {
        return None;
    }
    let mut v: u32 = 0;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    (v <= 100).then_some(v as u8)
}

/// Parses quick-control expressions (zero alloc, case-insensitive).
/// Intentionally avoids intercepting normal search queries (`2+2`, `bell`, `wallpaper`,
/// `wifi`, `bluetooth`, `web`, `term`, `qui`).
pub fn parse_quick_command(query: &str) -> Option<QuickCommand> {
    let q = trim_ws(query.as_bytes());
    if q.is_empty() {
        return None;
    }

    if eq_ci(q, b"mute") || eq_ci(q, b"audio mute") {
        return Some(QuickCommand::MuteSet(true));
    }
    if eq_ci(q, b"unmute") || eq_ci(q, b"audio unmute") {
        return Some(QuickCommand::MuteSet(false));
    }
    if eq_ci(q, b"mic mute") || eq_ci(q, b"mute mic") || eq_ci(q, b"mic off") {
        return Some(QuickCommand::MicMuteSet(true));
    }
    if eq_ci(q, b"mic unmute") || eq_ci(q, b"unmute mic") || eq_ci(q, b"mic on") {
        return Some(QuickCommand::MicMuteSet(false));
    }
    if eq_ci(q, b"mic") {
        return Some(QuickCommand::MicMuteToggle);
    }

    for prefix in [b"volume".as_slice(), b"vol".as_slice()] {
        if let Some(rest) = strip_prefix_ci(q, prefix) {
            let arg = trim_ws(rest);
            if arg.is_empty() {
                break;
            }
            if rest.first() == Some(&b' ') || rest.first() == Some(&b'+') || rest.first() == Some(&b'-') {
                if eq_ci(arg, b"up") || arg == b"+" {
                    return Some(QuickCommand::VolumeStep(10));
                }
                if eq_ci(arg, b"down") || arg == b"-" {
                    return Some(QuickCommand::VolumeStep(-10));
                }
                if eq_ci(arg, b"mute") {
                    return Some(QuickCommand::MuteSet(true));
                }
                if eq_ci(arg, b"unmute") {
                    return Some(QuickCommand::MuteSet(false));
                }
                if let Some(pct) = parse_u8_pct(arg) {
                    return Some(QuickCommand::VolumeSet(pct));
                }
            }
        }
    }

    for prefix in [b"brightness".as_slice(), b"bright".as_slice()] {
        if let Some(rest) = strip_prefix_ci(q, prefix) {
            let arg = trim_ws(rest);
            if arg.is_empty() {
                break;
            }
            if rest.first() == Some(&b' ') || rest.first() == Some(&b'+') || rest.first() == Some(&b'-') {
                if eq_ci(arg, b"up") || arg == b"+" {
                    return Some(QuickCommand::BrightnessStep(10));
                }
                if eq_ci(arg, b"down") || arg == b"-" {
                    return Some(QuickCommand::BrightnessStep(-10));
                }
                if let Some(pct) = parse_u8_pct(arg) {
                    return Some(QuickCommand::BrightnessSet(pct));
                }
            }
        }
    }

    if eq_ci(q, b"wifi on") {
        return Some(QuickCommand::WifiSet(true));
    }
    if eq_ci(q, b"wifi off") {
        return Some(QuickCommand::WifiSet(false));
    }
    if eq_ci(q, b"wifi scan") || eq_ci(q, b"wifi list") {
        return Some(QuickCommand::WifiScan);
    }

    if eq_ci(q, b"bt on") || eq_ci(q, b"bluetooth on") {
        return Some(QuickCommand::BtSet(true));
    }
    if eq_ci(q, b"bt off") || eq_ci(q, b"bluetooth off") {
        return Some(QuickCommand::BtSet(false));
    }
    if eq_ci(q, b"bt") {
        return Some(QuickCommand::BtPanel);
    }

    if eq_ci(q, b"gmode") || eq_ci(q, b"g-mode") || eq_ci(q, b"turbo") {
        return Some(QuickCommand::PowerMode(POWER_GMODE));
    }
    if eq_ci(q, b"perf") || eq_ci(q, b"performance") {
        return Some(QuickCommand::PowerMode(POWER_PERFORMANCE));
    }
    if eq_ci(q, b"balanced") {
        return Some(QuickCommand::PowerMode(POWER_BALANCED));
    }
    if eq_ci(q, b"saver") || eq_ci(q, b"quiet") {
        return Some(QuickCommand::PowerMode(POWER_SAVER));
    }

    if eq_ci(q, b"night") || eq_ci(q, b"night light") {
        return Some(QuickCommand::NightLightCycle);
    }
    if eq_ci(q, b"night off") || eq_ci(q, b"night 0") {
        return Some(QuickCommand::NightLightSet(0));
    }
    if eq_ci(q, b"night 1") || eq_ci(q, b"night warm") {
        return Some(QuickCommand::NightLightSet(1));
    }
    if eq_ci(q, b"night 2") || eq_ci(q, b"night amber") {
        return Some(QuickCommand::NightLightSet(2));
    }
    if eq_ci(q, b"night 3") || eq_ci(q, b"night deep") {
        return Some(QuickCommand::NightLightSet(3));
    }

    if eq_ci(q, b"accent") {
        return Some(QuickCommand::NoirAccentCycle);
    }
    if eq_ci(q, b"accent rose") || eq_ci(q, b"accent crimson") || eq_ci(q, b"accent pink") {
        return Some(QuickCommand::NoirAccentSet(0));
    }
    if eq_ci(q, b"accent graphite") || eq_ci(q, b"accent silver") || eq_ci(q, b"accent gray") {
        return Some(QuickCommand::NoirAccentSet(1));
    }
    if eq_ci(q, b"accent lavender") || eq_ci(q, b"accent purple") {
        return Some(QuickCommand::NoirAccentSet(2));
    }
    if eq_ci(q, b"accent ice") || eq_ci(q, b"accent cyan") || eq_ci(q, b"accent blue") {
        return Some(QuickCommand::NoirAccentSet(3));
    }

    if eq_ci(q, b"dnd") || eq_ci(q, b"focus") {
        return Some(QuickCommand::DndToggle);
    }
    if eq_ci(q, b"dnd on") || eq_ci(q, b"focus on") {
        return Some(QuickCommand::DndSet(true));
    }
    if eq_ci(q, b"dnd off") || eq_ci(q, b"focus off") {
        return Some(QuickCommand::DndSet(false));
    }

    if eq_ci(q, b"lock") || eq_ci(q, b"lock screen") {
        return Some(QuickCommand::LockScreen);
    }
    if eq_ci(q, b"shot") || eq_ci(q, b"snip") || eq_ci(q, b"screenshot") {
        return Some(QuickCommand::Screenshot);
    }

    if let Some(rest) = strip_prefix_ci(q, b"space ") {
        let arg = trim_ws(rest);
        if arg.len() == 1 && (b'1'..=b'5').contains(&arg[0]) {
            return Some(QuickCommand::SwitchSpace(arg[0] - b'1'));
        }
    }

    None
}

fn push_u8(buf: &mut [u8], pos: &mut usize, src: &[u8]) {
    let take = src.len().min(buf.len().saturating_sub(*pos));
    buf[*pos..*pos + take].copy_from_slice(&src[..take]);
    *pos += take;
}

fn push_pct(buf: &mut [u8], pos: &mut usize, v: u8) {
    let v = v.min(100);
    if v >= 100 {
        push_u8(buf, pos, b"100%");
    } else if v >= 10 {
        push_u8(buf, pos, &[b'0' + (v / 10), b'0' + (v % 10), b'%']);
    } else {
        push_u8(buf, pos, &[b'0' + v, b'%']);
    }
}

/// Formats a compact human title into `out` and returns written byte length.
pub fn format_quick_command(cmd: QuickCommand, out: &mut [u8; 48]) -> usize {
    let mut n = 0;
    match cmd {
        QuickCommand::VolumeSet(v) => {
            push_u8(out, &mut n, b"Set Volume ");
            push_pct(out, &mut n, v);
        }
        QuickCommand::VolumeStep(d) => {
            push_u8(out, &mut n, if d > 0 { b"Volume Up +10%" } else { b"Volume Down -10%" });
        }
        QuickCommand::MuteSet(true) => push_u8(out, &mut n, b"Mute Speakers"),
        QuickCommand::MuteSet(false) => push_u8(out, &mut n, b"Unmute Speakers"),
        QuickCommand::MicMuteToggle => push_u8(out, &mut n, b"Toggle Mic Mute"),
        QuickCommand::MicMuteSet(true) => push_u8(out, &mut n, b"Mute Microphone"),
        QuickCommand::MicMuteSet(false) => push_u8(out, &mut n, b"Unmute Microphone"),
        QuickCommand::BrightnessSet(v) => {
            push_u8(out, &mut n, b"Brightness ");
            push_pct(out, &mut n, v);
        }
        QuickCommand::BrightnessStep(d) => {
            push_u8(out, &mut n, if d > 0 { b"Brightness Up" } else { b"Brightness Down" });
        }
        QuickCommand::WifiSet(true) => push_u8(out, &mut n, b"Wi-Fi Radio On"),
        QuickCommand::WifiSet(false) => push_u8(out, &mut n, b"Wi-Fi Radio Off"),
        QuickCommand::WifiScan => push_u8(out, &mut n, b"Open Wi-Fi Scanner"),
        QuickCommand::BtSet(true) => push_u8(out, &mut n, b"Bluetooth On"),
        QuickCommand::BtSet(false) => push_u8(out, &mut n, b"Bluetooth Off"),
        QuickCommand::BtPanel => push_u8(out, &mut n, b"Open Bluetooth Panel"),
        QuickCommand::PowerMode(m) => {
            push_u8(out, &mut n, b"Power: ");
            push_u8(out, &mut n, POWER_NAMES[(m as usize).min(3)].as_bytes());
        }
        QuickCommand::NightLightCycle => push_u8(out, &mut n, b"Cycle Night Light"),
        QuickCommand::NightLightSet(lvl) => {
            push_u8(out, &mut n, b"Night Light: ");
            push_u8(out, &mut n, NIGHT_NAMES[(lvl as usize).min(3)].as_bytes());
        }
        QuickCommand::NoirAccentCycle => push_u8(out, &mut n, b"Cycle Noir Accent"),
        QuickCommand::NoirAccentSet(idx) => {
            push_u8(out, &mut n, b"Accent: ");
            push_u8(out, &mut n, ACCENT_NAMES[(idx as usize).min(3)].as_bytes());
        }
        QuickCommand::DndToggle => push_u8(out, &mut n, b"Toggle Do Not Disturb"),
        QuickCommand::DndSet(true) => push_u8(out, &mut n, b"Focus / DND On"),
        QuickCommand::DndSet(false) => push_u8(out, &mut n, b"Focus / DND Off"),
        QuickCommand::LockScreen => push_u8(out, &mut n, b"Lock Screen"),
        QuickCommand::Screenshot => push_u8(out, &mut n, b"Capture Screenshot"),
        QuickCommand::SwitchSpace(idx) => {
            push_u8(out, &mut n, b"Switch Space ");
            push_u8(out, &mut n, &[b'1' + idx.min(4)]);
        }
    }
    n
}

/// Static search keywords so `voyeur::rank::merge` matches quick commands cleanly.
pub const fn quick_command_keywords(cmd: QuickCommand) -> &'static str {
    match cmd {
        QuickCommand::VolumeSet(_) | QuickCommand::VolumeStep(_) => "vol volume sound audio",
        QuickCommand::MuteSet(_) => "mute unmute sound audio volume",
        QuickCommand::MicMuteToggle | QuickCommand::MicMuteSet(_) => "mic mute microphone",
        QuickCommand::BrightnessSet(_) | QuickCommand::BrightnessStep(_) => "bright brightness display screen",
        QuickCommand::WifiSet(_) | QuickCommand::WifiScan => "wifi wireless scan network",
        QuickCommand::BtSet(_) | QuickCommand::BtPanel => "bt bluetooth wireless",
        QuickCommand::PowerMode(_) => "gmode g-mode turbo perf performance balanced saver quiet power",
        QuickCommand::NightLightCycle | QuickCommand::NightLightSet(_) => "night light warmth amber deep off",
        QuickCommand::NoirAccentCycle | QuickCommand::NoirAccentSet(_) => "accent rose crimson pink graphite silver lavender ice cyan blue",
        QuickCommand::DndToggle | QuickCommand::DndSet(_) => "dnd focus notifications quiet",
        QuickCommand::LockScreen => "lock screen",
        QuickCommand::Screenshot => "shot snip screenshot",
        QuickCommand::SwitchSpace(_) => "space workspace desktop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centered_cp(w: usize, h: usize, sc: u32) -> (usize, usize, usize, usize) {
        let s = sc as usize;
        let bar = 28 * s;
        let gap = 8 * s;
        let aw = w - gap * 2;
        let ah = (h - bar) - gap * 2;
        let cw = 480 * s;
        let ch = 280 * s;
        (gap + (aw - cw) / 2, bar + gap + (ah - ch) / 2, cw, ch)
    }

    #[test]
    fn wire_packets_round_trip_without_loss() {
        let mut s = MetaHubState::DEFAULT;
        s.visible = true;
        s.mode = MODE_SPOTLIGHT;
        s.anchor = (280, 128, 720, 540);
        s.brightness_pct = 62;
        s.volume_pct = 48;
        s.speaker_muted = true;
        s.mic_muted = true;
        s.mic_gain_pct = 85;
        s.wifi_enabled = true;
        s.wifi_connected = true;
        s.wifi_rssi_bars = 4;
        s.set_ssid(b"SexOS-Noir-5GHz!!"); // truncates cleanly to 16 bytes
        s.bt_enabled = true;
        s.bt_available = true;
        s.bt_connected_count = 2;
        s.power_mode = POWER_GMODE;
        s.night_light_level = 3;
        s.noir_accent = 2;
        s.dnd_enabled = true;
        s.active_space = 4;
        s.open_windows = 5;
        s.parked_apps = 2;
        s.media_playing = true;
        s.battery_pct = 91;
        s.battery_known = true;
        s.charging = true;
        s.hover_target = HitTarget::SliderVolume.hover_code();
        s.active_drag_slider = DragSlider::Volume;

        let mut rx = Receiver::new();
        let (a0, a1, a2) = s.pack_anchor();
        assert_eq!(rx.receive(a0, a1, a2), Some(false));
        let (s0, s1, s2) = s.pack_ssid_telemetry();
        assert_eq!(rx.receive(s0, s1, s2), Some(false));
        let (p0, p1, p2) = s.pack_state();
        assert_eq!(rx.receive(p0, p1, p2), Some(true));
        assert_eq!(rx.current, s);
        assert_eq!(rx.current.ssid_str(), "SexOS-Noir-5GHz!");

        // Hide with fast zero state packet
        assert_eq!(rx.receive(0, 0, 0), Some(true));
        assert!(!rx.current.visible);
        assert_eq!(rx.current.hover_target, 0);
        assert_eq!(rx.current.active_drag_slider, DragSlider::None);

        // Reject malformed high bits / out-of-range values
        assert_eq!(rx.receive(p0 | (1 << 55), 0, 0), None);
        assert_eq!(rx.receive(0, 1, 0), None);
        assert_eq!(rx.receive(PACKET_SSID << 60 | 17, 0, 0), None);
    }

    #[test]
    fn command_palette_and_spotlight_layouts_never_overlap_anchors() {
        let screens = [
            (1280usize, 800usize, 1u32),
            (1600, 900, 1),
            (1920, 1080, 1),
            (1920, 1080, 2),
            (2560, 1600, 2),
            (3840, 2160, 3),
        ];
        for &(w, h, sc) in &screens {
            let bar = 28 * sc as usize;
            let cp = centered_cp(w, h, sc);
            let cp_anchor = (cp.0 as i16, cp.1 as i16, cp.2 as u16, cp.3 as u16);
            let l_cp = Layout::compute(w, h, bar, sc, MODE_COMMAND_PALETTE, cp_anchor)
                .expect("meta hub fits next to command palette");
            assert!(rects_disjoint(l_cp.panel, cp));
            assert!(l_cp.panel.0 + l_cp.panel.2 <= w);
            assert!(l_cp.panel.1 >= bar);
            assert!(l_cp.panel.1 + l_cp.panel.3 <= h);

            let sp_anchor = (
                ((w.saturating_sub(720 * sc as usize)) / 2) as i16,
                (bar + 8 * sc as usize + 64) as i16,
                (720 * sc as usize) as u16,
                (540 * sc as usize) as u16,
            );
            if let Some(l_sp) = Layout::compute(w, h, bar, sc, MODE_SPOTLIGHT, sp_anchor) {
                assert!(rects_disjoint(
                    l_sp.panel,
                    (sp_anchor.0 as usize, sp_anchor.1 as usize, sp_anchor.2 as usize, sp_anchor.3 as usize)
                ));
                assert!(l_sp.panel.0 + l_sp.panel.2 <= w);
                assert!(l_sp.panel.1 >= bar);
                assert!(l_sp.panel.1 + l_sp.panel.3 <= h);
            }
        }
    }

    #[test]
    fn tiny_screens_gracefully_skip_without_overlap() {
        // Too small to hold both 480x280 launcher and >=216w control hub without overlap.
        assert_eq!(Layout::compute(640, 480, 28, 1, MODE_COMMAND_PALETTE, (80, 100, 480, 280)), None);
    }

    #[test]
    fn hit_test_and_slider_percentage_match_geometry() {
        let l = Layout::compute(1280, 800, 28, 1, MODE_COMMAND_PALETTE, (400, 274, 480, 280)).unwrap();
        let center = |r: Rect| ((r.0 + r.2 / 2) as i32, (r.1 + r.3 / 2) as i32);

        let (wx, wy) = center(l.tiles[0]);
        // Only outside chevron hotspot should hit TileWifi
        assert_eq!(l.hit(l.tiles[0].0 as i32 + 10, wy), HitTarget::TileWifi);
        let (ex, ey) = center(l.tile_wifi_expand);
        assert_eq!(l.hit(ex, ey), HitTarget::TileWifiExpand);
        let _ = wx;

        assert_eq!(l.hit(center(l.tiles[1]).0, center(l.tiles[1]).1), HitTarget::TileBluetooth);
        assert_eq!(l.hit(center(l.tiles[2]).0, center(l.tiles[2]).1), HitTarget::TilePower);
        assert_eq!(l.hit(center(l.tiles[3]).0, center(l.tiles[3]).1), HitTarget::TileNightLight);
        assert_eq!(l.hit(center(l.tiles[4]).0, center(l.tiles[4]).1), HitTarget::TileAccent);
        assert_eq!(l.hit(center(l.btn_lock).0, center(l.btn_lock).1), HitTarget::BtnLock);
        assert_eq!(l.hit(center(l.btn_shot).0, center(l.btn_shot).1), HitTarget::BtnShot);
        assert_eq!(l.hit((l.tile_dnd_toggle.0 + 12) as i32, center(l.tile_dnd_toggle).1), HitTarget::TileDnd);

        assert_eq!(l.hit(center(l.slider_brightness).0, center(l.slider_brightness).1), HitTarget::SliderBrightness);
        assert_eq!(l.hit(center(l.btn_speaker_mute).0, center(l.btn_speaker_mute).1), HitTarget::BtnSpeakerMute);
        assert_eq!(l.hit(center(l.slider_volume).0, center(l.slider_volume).1), HitTarget::SliderVolume);
        assert_eq!(l.hit(center(l.btn_mic_mute).0, center(l.btn_mic_mute).1), HitTarget::BtnMicMute);
        assert_eq!(l.hit(center(l.media_prev).0, center(l.media_prev).1), HitTarget::MediaPrev);
        assert_eq!(l.hit(center(l.media_play_pause).0, center(l.media_play_pause).1), HitTarget::MediaPlayPause);
        assert_eq!(l.hit(center(l.media_next).0, center(l.media_next).1), HitTarget::MediaNext);
        for i in 0..QUICK_LAUNCH_COUNT {
            let (qx, qy) = center(l.quick_actions[i]);
            assert_eq!(l.hit(qx, qy), HitTarget::quick_launch(i));
        }

        // Slider percentage mapping
        assert_eq!(Layout::percent_at(l.slider_brightness, l.slider_brightness.0 as i32), 0);
        assert_eq!(Layout::percent_at(l.slider_brightness, (l.slider_brightness.0 + l.slider_brightness.2) as i32), 100);
        assert!((45..=55).contains(&Layout::percent_at(l.slider_brightness, center(l.slider_brightness).0)));
    }

    #[test]
    fn quick_command_parser_handles_controls_and_ignores_legacy_queries() {
        assert_eq!(parse_quick_command("vol 65"), Some(QuickCommand::VolumeSet(65)));
        assert_eq!(parse_quick_command("volume 80%"), Some(QuickCommand::VolumeSet(80)));
        assert_eq!(parse_quick_command("mute"), Some(QuickCommand::MuteSet(true)));
        assert_eq!(parse_quick_command("unmute"), Some(QuickCommand::MuteSet(false)));
        assert_eq!(parse_quick_command("mic mute"), Some(QuickCommand::MicMuteSet(true)));
        assert_eq!(parse_quick_command("bright 40"), Some(QuickCommand::BrightnessSet(40)));
        assert_eq!(parse_quick_command("brightness 100"), Some(QuickCommand::BrightnessSet(100)));
        assert_eq!(parse_quick_command("wifi on"), Some(QuickCommand::WifiSet(true)));
        assert_eq!(parse_quick_command("wifi off"), Some(QuickCommand::WifiSet(false)));
        assert_eq!(parse_quick_command("wifi scan"), Some(QuickCommand::WifiScan));
        assert_eq!(parse_quick_command("bt on"), Some(QuickCommand::BtSet(true)));
        assert_eq!(parse_quick_command("bt off"), Some(QuickCommand::BtSet(false)));
        assert_eq!(parse_quick_command("gmode"), Some(QuickCommand::PowerMode(POWER_GMODE)));
        assert_eq!(parse_quick_command("perf"), Some(QuickCommand::PowerMode(POWER_PERFORMANCE)));
        assert_eq!(parse_quick_command("balanced"), Some(QuickCommand::PowerMode(POWER_BALANCED)));
        assert_eq!(parse_quick_command("saver"), Some(QuickCommand::PowerMode(POWER_SAVER)));
        assert_eq!(parse_quick_command("night"), Some(QuickCommand::NightLightCycle));
        assert_eq!(parse_quick_command("night 2"), Some(QuickCommand::NightLightSet(2)));
        assert_eq!(parse_quick_command("accent rose"), Some(QuickCommand::NoirAccentSet(0)));
        assert_eq!(parse_quick_command("accent graphite"), Some(QuickCommand::NoirAccentSet(1)));
        assert_eq!(parse_quick_command("accent lavender"), Some(QuickCommand::NoirAccentSet(2)));
        assert_eq!(parse_quick_command("accent ice"), Some(QuickCommand::NoirAccentSet(3)));
        assert_eq!(parse_quick_command("dnd"), Some(QuickCommand::DndToggle));
        assert_eq!(parse_quick_command("space 3"), Some(QuickCommand::SwitchSpace(2)));

        // Legacy launcher / voyeur / spotlight queries MUST return None
        for legacy in ["2+2", "bell", "wallpaper", "web", "term", "termqq", "qui", "wifi", "bluetooth", "store", "mytool"] {
            assert_eq!(parse_quick_command(legacy), None, "must not hijack {}", legacy);
        }
    }
}
