// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! MacOS Noir Background App Lifecycle, Top-Right SilkBar Parked-App Tray,
//! Quick-Action Popovers, Two-Section Open/Background Switcher, Close vs Quit
//! Confirmation Sheet, Force Quit Dialog, and Ctrl+Left-Click Translation.
//!
//! Shell owns policy state, parked frame/task registry, hit-testing, and input;
//! Display renders via `OP_BG_APPS` (`0x2FF`) using the exact shared `BarLayout`
//! and `OverlayLayout` rectangles. Pure `#![no_std]` with zero heap.

/// Shell -> Display PDX opcode for background apps, switcher, and Noir sheets.
pub const OP_BG_APPS: u64 = 0x2FF;

/// Maximum open windows listed in the two-section Switcher / Force Quit dialog.
pub const MAX_OPEN: usize = 8;
/// Maximum parked background apps shown in the SilkBar tray and Switcher.
pub const MAX_PARKED: usize = 6;
/// Maximum UTF-8/ASCII bytes per app title / label on the wire.
pub const NAME_MAX: usize = 16;

/// Per-app window close behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClosePolicy {
    /// Prompt with the frosted macOS Noir sheet ("Keep Running in SilkBar" vs "Quit App").
    Ask = 0,
    /// Hide/unmap the window from the active Space while keeping its task alive in SilkBar.
    KeepInSilkBar = 1,
    /// Terminate the app process cleanly when its window is closed.
    QuitApp = 2,
}

/// Known SexOS app archetypes for badges, quick actions, and default close policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AppKind {
    Generic = 0,
    Tether = 1,
    Tailor = 2,
    Qupid = 3,
    Swingers = 4,
    Spindle = 5,
    Quil = 6,
    Browser = 7,
    Settings = 8,
    Files = 9,
    SexShop = 10,
}

impl AppKind {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Tether,
            2 => Self::Tailor,
            3 => Self::Qupid,
            4 => Self::Swingers,
            5 => Self::Spindle,
            6 => Self::Quil,
            7 => Self::Browser,
            8 => Self::Settings,
            9 => Self::Files,
            10 => Self::SexShop,
            _ => Self::Generic,
        }
    }

    pub const fn default_policy(self) -> ClosePolicy {
        match self {
            Self::Tether | Self::Tailor | Self::Qupid | Self::Swingers => ClosePolicy::KeepInSilkBar,
            // The close button ends an ordinary program at once (no sheet each time);
            // keeping it in the SilkBar is a remembered choice (Settings or the sheet).
            _ => ClosePolicy::QuitApp,
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Tether => "Tether",
            Self::Tailor => "Tailor AI",
            Self::Qupid => "Qupid",
            Self::Swingers => "Swingers",
            Self::Spindle => "Spindle",
            Self::Quil => "Quil",
            Self::Browser => "Browser",
            Self::Settings => "Settings",
            Self::Files => "Files",
            Self::SexShop => "SexShop",
            Self::Generic => "Program",
        }
    }

    pub const fn monogram(self) -> &'static str {
        match self {
            Self::Tether => "Te",
            Self::Tailor => "Ta",
            Self::Qupid => "Qu",
            Self::Swingers => "Sw",
            Self::Spindle => "Sp",
            Self::Quil => "Ql",
            Self::Browser => "Br",
            Self::Settings => "St",
            Self::Files => "Fl",
            Self::SexShop => "Sh",
            Self::Generic => "Ap",
        }
    }

    /// Accent swatch RGB (`0x00RRGGBB`) for top-bar badge and popover accents.
    pub const fn badge_rgb(self) -> u32 {
        match self {
            Self::Tether => 0x00F4_3F5E,
            Self::Tailor => 0x008B_5CF6,
            Self::Qupid => 0x00FB_923C,
            Self::Swingers => 0x0010_B981,
            Self::Spindle => 0x0038_BDF8,
            Self::Quil => 0x00A7_8BFA,
            Self::Browser => 0x0060_A5FA,
            Self::Settings => 0x0094_A3B8,
            Self::Files => 0x0034_D399,
            Self::SexShop => 0x00EC_4899,
            Self::Generic => 0x00E1_1D48,
        }
    }

    /// Two contextual quick-action labels rendered in the Quick-Action Popover.
    pub const fn quick_actions(self) -> [&'static str; 2] {
        match self {
            Self::Tether => ["Reconnect Host Link", "ACP / MCP Link Status"],
            Self::Tailor => ["Agent Stream: Active", "New Agent Session"],
            Self::Qupid => ["Play / Pause Playback", "Now Playing Status"],
            Self::Swingers => ["View Torrent Transfers", "Swarm Background Status"],
            Self::Spindle => ["Focus Terminal", "Check Session Status"],
            Self::Browser => ["Bring Browser Here", "Active Tabs Status"],
            Self::Settings => ["Open System Settings", "Show Preferences Status"],
            Self::Files => ["Open Files Window", "Background Sync Status"],
            Self::SexShop => ["Open App Store", "Store Status"],
            Self::Quil => ["Bring Editor Here", "Document Status"],
            Self::Generic => ["Focus & Bring to Space", "Background Task Status"],
        }
    }
}

/// Program-store indices aligned with `sex_config::PROGRAMS` (0..19) plus
/// built-in window bit assignments (`28` spindle, `29` browser) in `CLOSE_KEEP`/`CLOSE_QUIT`.
const POLICY_BITS: [(&str, u8, AppKind); 22] = [
    ("jot", 0, AppKind::Generic),
    ("loom", 1, AppKind::Files),
    ("mesh", 2, AppKind::Generic),
    ("abacus", 3, AppKind::Generic),
    ("gaze", 4, AppKind::Generic),
    ("qupid", 5, AppKind::Qupid),
    ("vellum", 6, AppKind::Generic),
    ("date", 7, AppKind::Generic),
    ("quil", 8, AppKind::Quil),
    ("gossip", 9, AppKind::Generic),
    ("chastity", 10, AppKind::Generic),
    ("tether", 11, AppKind::Tether),
    ("tailor", 12, AppKind::Tailor),
    ("swingers", 13, AppKind::Swingers),
    ("tatami", 14, AppKind::Settings),
    ("sexshop", 15, AppKind::SexShop),
    ("kaleidoscope", 16, AppKind::Browser),
    ("flash", 17, AppKind::Generic),
    ("aftercare", 18, AppKind::Generic),
    ("spindle", 28, AppKind::Spindle),
    ("browser", 29, AppKind::Browser),
    ("linen", 1, AppKind::Files),
];

fn starts_with_ignore_ascii_case(hay: &[u8], prefix: &[u8]) -> bool {
    hay.len() >= prefix.len() && hay[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn contains_ignore_ascii_case(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// Maps an app store name or window title slice to an `AppKind`.
pub fn classify_name(name: &[u8]) -> AppKind {
    for &(key, _, kind) in &POLICY_BITS {
        if starts_with_ignore_ascii_case(name, key.as_bytes()) {
            return kind;
        }
    }
    if contains_ignore_ascii_case(name, b"tailor") {
        return AppKind::Tailor;
    }
    if contains_ignore_ascii_case(name, b"tether") {
        return AppKind::Tether;
    }
    if contains_ignore_ascii_case(name, b"qupid") {
        return AppKind::Qupid;
    }
    if contains_ignore_ascii_case(name, b"swinger") {
        return AppKind::Swingers;
    }
    if contains_ignore_ascii_case(name, b"setting") || contains_ignore_ascii_case(name, b"tatami") {
        return AppKind::Settings;
    }
    if contains_ignore_ascii_case(name, b"sexshop") {
        return AppKind::SexShop;
    }
    if contains_ignore_ascii_case(name, b"browser") || contains_ignore_ascii_case(name, b"kaleidoscope") {
        return AppKind::Browser;
    }
    if contains_ignore_ascii_case(name, b"files") || contains_ignore_ascii_case(name, b"loom") || contains_ignore_ascii_case(name, b"linen") {
        return AppKind::Files;
    }
    AppKind::Generic
}

/// Stable policy bit index (`0..=30`) for persisting per-app close preference.
pub fn policy_bit_for_name(name: &[u8]) -> Option<u8> {
    for &(key, bit, _) in &POLICY_BITS {
        if starts_with_ignore_ascii_case(name, key.as_bytes()) {
            return Some(bit);
        }
    }
    match classify_name(name) {
        AppKind::Tether => Some(11),
        AppKind::Tailor => Some(12),
        AppKind::Swingers => Some(13),
        AppKind::Qupid => Some(5),
        AppKind::Settings => Some(14),
        AppKind::SexShop => Some(15),
        AppKind::Files => Some(1),
        AppKind::Browser => Some(29),
        AppKind::Spindle => Some(28),
        AppKind::Quil => Some(8),
        AppKind::Generic => None,
    }
}

/// Resolves the effective `ClosePolicy` from persisted config bitmasks and app identity.
pub fn resolve_policy(name: &[u8], kind: AppKind, keep_mask: i32, quit_mask: i32) -> ClosePolicy {
    if let Some(bit) = policy_bit_for_name(name) {
        if bit < 31 {
            let m = 1i32 << bit;
            if (quit_mask & m) != 0 {
                return ClosePolicy::QuitApp;
            }
            if (keep_mask & m) != 0 {
                return ClosePolicy::KeepInSilkBar;
            }
        }
    }
    kind.default_policy()
}

/// Updates persisted `(keep_mask, quit_mask)` when `[x] Remember my choice` is confirmed.
pub fn apply_remember_policy(bit: Option<u8>, choice: ClosePolicy, keep_mask: &mut i32, quit_mask: &mut i32) {
    let Some(b) = bit else { return };
    if b >= 31 {
        return;
    }
    let m = 1i32 << b;
    match choice {
        ClosePolicy::KeepInSilkBar => {
            *keep_mask |= m;
            *quit_mask &= !m;
        }
        ClosePolicy::QuitApp => {
            *quit_mask |= m;
            *keep_mask &= !m;
        }
        ClosePolicy::Ask => {
            *keep_mask &= !m;
            *quit_mask &= !m;
        }
    }
}

/// Stateful `Ctrl + Left-Click` -> Right-Click translator with press/release symmetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CtrlClickTranslator {
    ctrl_left_active: bool,
}

impl CtrlClickTranslator {
    pub const fn new() -> Self {
        Self { ctrl_left_active: false }
    }

    /// Maps button `1` (left) + `ctrl_held` into `2` (right) on press and keeps its matching
    /// release mapped to `2` even if Ctrl is released mid-click. Leaves `Spindle` terminal
    /// content clicks as raw `1` when `in_spindle_content` is true so TUI apps keep raw Ctrl+Click.
    pub fn translate(&mut self, button: u8, pressed: bool, ctrl_held: bool, in_spindle_content: bool) -> u8 {
        if button == 1 {
            if pressed {
                if ctrl_held && !in_spindle_content {
                    self.ctrl_left_active = true;
                    return 2;
                }
                self.ctrl_left_active = false;
                return 1;
            } else if self.ctrl_left_active {
                self.ctrl_left_active = false;
                return 2;
            }
        }
        button
    }
}

/// Copies up to `NAME_MAX` bytes cleanly into a fixed `[u8; NAME_MAX]` slot.
pub fn pack_fixed_name(src: &[u8]) -> ([u8; NAME_MAX], u8) {
    let mut out = [0u8; NAME_MAX];
    let n = src.len().min(NAME_MAX);
    out[..n].copy_from_slice(&src[..n]);
    (out, n as u8)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenWindow {
    pub sid: u16,
    pub kind: AppKind,
    pub scene: u8,
    pub focused: bool,
    pub name: [u8; NAME_MAX],
    pub name_len: u8,
}

impl OpenWindow {
    pub const EMPTY: Self = Self {
        sid: 0,
        kind: AppKind::Generic,
        scene: 0,
        focused: false,
        name: [0; NAME_MAX],
        name_len: 0,
    };

    pub fn title_bytes(&self) -> &[u8] {
        let len = (self.name_len as usize).min(NAME_MAX);
        if len == 0 {
            self.kind.display_name().as_bytes()
        } else {
            &self.name[..len]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkedApp {
    pub target_pd: u8,
    pub sid: u16,
    pub kind: AppKind,
    pub status: u8,
    pub name: [u8; NAME_MAX],
    pub name_len: u8,
}

impl ParkedApp {
    pub const EMPTY: Self = Self {
        target_pd: 0,
        sid: 0,
        kind: AppKind::Generic,
        status: 0,
        name: [0; NAME_MAX],
        name_len: 0,
    };

    pub fn title_bytes(&self) -> &[u8] {
        let len = (self.name_len as usize).min(NAME_MAX);
        if len == 0 {
            self.kind.display_name().as_bytes()
        } else {
            &self.name[..len]
        }
    }

    pub fn badge_chars(&self) -> [u8; 2] {
        let len = (self.name_len as usize).min(NAME_MAX);
        if len == 0 || self.kind != AppKind::Generic {
            let m = self.kind.monogram().as_bytes();
            return [m[0], m[1]];
        }
        let a = self.name[0].to_ascii_uppercase();
        let b = if len >= 2 { self.name[1].to_ascii_lowercase() } else { b' ' };
        [a, b]
    }
}

/// Active frosted Noir overlay / modal sheet driven by `bg_apps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OverlayKind {
    None = 0,
    QuickAction = 1,
    Switcher = 2,
    CloseConfirm = 3,
    ForceQuit = 4,
    AboutSexOs = 5,
}

impl OverlayKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::QuickAction),
            2 => Some(Self::Switcher),
            3 => Some(Self::CloseConfirm),
            4 => Some(Self::ForceQuit),
            5 => Some(Self::AboutSexOs),
            _ => None,
        }
    }

    pub const fn is_modal(self) -> bool {
        matches!(self, Self::CloseConfirm | Self::ForceQuit | Self::AboutSexOs)
    }
}

/// Hover target ID encoded on wire as `u16` (`kind << 8 | idx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HoverTarget {
    #[default]
    None,
    SwitcherPill,
    ParkedBadge(u8),
    QuickRow(u8),
    SwitcherOpenFocus(u8),
    SwitcherOpenClose(u8),
    SwitcherParkedReopen(u8),
    SwitcherParkedQuit(u8),
    CloseRemember,
    CloseKeep,
    CloseQuit,
    CloseCancel,
    ForceRow(u8),
    ForceQuitBtn,
    ForceCloseBtn,
    AboutSettingsBtn,
    AboutCloseBtn,
}

impl HoverTarget {
    pub const fn encode(self) -> u16 {
        match self {
            Self::None => 0,
            Self::SwitcherPill => 1 << 8,
            Self::ParkedBadge(i) => (2 << 8) | i as u16,
            Self::QuickRow(i) => (3 << 8) | i as u16,
            Self::SwitcherOpenFocus(i) => (4 << 8) | i as u16,
            Self::SwitcherOpenClose(i) => (5 << 8) | i as u16,
            Self::SwitcherParkedReopen(i) => (6 << 8) | i as u16,
            Self::SwitcherParkedQuit(i) => (7 << 8) | i as u16,
            Self::CloseRemember => 8 << 8,
            Self::CloseKeep => 9 << 8,
            Self::CloseQuit => 10 << 8,
            Self::CloseCancel => 11 << 8,
            Self::ForceRow(i) => (12 << 8) | i as u16,
            Self::ForceQuitBtn => 13 << 8,
            Self::ForceCloseBtn => 14 << 8,
            Self::AboutSettingsBtn => 15 << 8,
            Self::AboutCloseBtn => 16 << 8,
        }
    }

    pub const fn decode(v: u16) -> Self {
        let kind = (v >> 8) as u8;
        let idx = (v & 0xFF) as u8;
        match kind {
            1 => Self::SwitcherPill,
            2 => Self::ParkedBadge(idx),
            3 => Self::QuickRow(idx),
            4 => Self::SwitcherOpenFocus(idx),
            5 => Self::SwitcherOpenClose(idx),
            6 => Self::SwitcherParkedReopen(idx),
            7 => Self::SwitcherParkedQuit(idx),
            8 => Self::CloseRemember,
            9 => Self::CloseKeep,
            10 => Self::CloseQuit,
            11 => Self::CloseCancel,
            12 => Self::ForceRow(idx),
            13 => Self::ForceQuitBtn,
            14 => Self::ForceCloseBtn,
            15 => Self::AboutSettingsBtn,
            16 => Self::AboutCloseBtn,
            _ => Self::None,
        }
    }
}

/// Complete snapshot synced from Shell to Display via `OP_BG_APPS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub open: [OpenWindow; MAX_OPEN],
    pub open_count: u8,
    pub parked: [ParkedApp; MAX_PARKED],
    pub parked_count: u8,
    pub overlay: OverlayKind,
    /// `parked_idx` for `QuickAction`, `selected` row for `ForceQuit`.
    pub overlay_idx: u8,
    pub remember: bool,
    pub confirm_sid: u16,
    pub confirm_target: u8,
    pub confirm_kind: AppKind,
    pub confirm_name: [u8; NAME_MAX],
    pub confirm_name_len: u8,
    /// Right boundary in screen X for the Switcher Pill (left of Chip0 / battery label).
    pub bar_right_edge: u16,
    pub hover: HoverTarget,
}

impl State {
    pub const EMPTY: Self = Self {
        open: [OpenWindow::EMPTY; MAX_OPEN],
        open_count: 0,
        parked: [ParkedApp::EMPTY; MAX_PARKED],
        parked_count: 0,
        overlay: OverlayKind::None,
        overlay_idx: 0,
        remember: false,
        confirm_sid: 0,
        confirm_target: 0,
        confirm_kind: AppKind::Generic,
        confirm_name: [0; NAME_MAX],
        confirm_name_len: 0,
        bar_right_edge: 1018,
        hover: HoverTarget::None,
    };

    pub fn open_slice(&self) -> &[OpenWindow] {
        &self.open[..(self.open_count as usize).min(MAX_OPEN)]
    }

    pub fn parked_slice(&self) -> &[ParkedApp] {
        &self.parked[..(self.parked_count as usize).min(MAX_PARKED)]
    }

    pub fn confirm_title_bytes(&self) -> &[u8] {
        let len = (self.confirm_name_len as usize).min(NAME_MAX);
        if len == 0 {
            self.confirm_kind.display_name().as_bytes()
        } else {
            &self.confirm_name[..len]
        }
    }
}

const TAG_HEADER: u64 = 0;
const TAG_CONFIRM_NAME: u64 = 1;
const TAG_OPEN_ITEM: u64 = 2;
const TAG_PARKED_ITEM: u64 = 3;
const TAG_COMMIT: u64 = 4;

fn pack_name_words(name: &[u8; NAME_MAX]) -> (u64, u64) {
    let mut a = [0u8; 8];
    let mut b = [0u8; 8];
    a.copy_from_slice(&name[..8]);
    b.copy_from_slice(&name[8..16]);
    (u64::from_le_bytes(a), u64::from_le_bytes(b))
}

fn unpack_name_words(w1: u64, w2: u64) -> [u8; NAME_MAX] {
    let mut out = [0u8; NAME_MAX];
    out[..8].copy_from_slice(&w1.to_le_bytes());
    out[8..16].copy_from_slice(&w2.to_le_bytes());
    out
}

/// Emits a tear-free sequence of 3-word PDX calls representing `state`.
pub fn send_calls(state: &State, seq: u16, mut emit: impl FnMut([u64; 3]) -> bool) -> bool {
    let open_n = (state.open_count as usize).min(MAX_OPEN);
    let parked_n = (state.parked_count as usize).min(MAX_PARKED);
    let h0 = TAG_HEADER
        | ((open_n as u64) << 8)
        | ((parked_n as u64) << 16)
        | ((state.overlay as u64) << 24)
        | ((state.overlay_idx as u64) << 32)
        | ((state.remember as u64) << 40)
        | ((state.hover.encode() as u64) << 48);
    let h1 = (state.confirm_sid as u64)
        | ((state.confirm_target as u64) << 16)
        | ((state.confirm_kind as u64) << 24)
        | ((state.bar_right_edge as u64) << 32)
        | ((seq as u64) << 48);
    if !emit([h0, h1, 0]) {
        return false;
    }
    if state.overlay == OverlayKind::CloseConfirm {
        let (w1, w2) = pack_name_words(&state.confirm_name);
        if !emit([TAG_CONFIRM_NAME | ((state.confirm_name_len as u64) << 8), w1, w2]) {
            return false;
        }
    }
    for (idx, w) in state.open[..open_n].iter().enumerate() {
        let (w1, w2) = pack_name_words(&w.name);
        let a0 = TAG_OPEN_ITEM
            | ((idx as u64) << 8)
            | ((w.sid as u64) << 16)
            | ((w.kind as u64) << 32)
            | ((w.scene as u64) << 40)
            | ((w.focused as u64) << 48)
            | ((w.name_len as u64) << 56);
        if !emit([a0, w1, w2]) {
            return false;
        }
    }
    for (idx, p) in state.parked[..parked_n].iter().enumerate() {
        let (w1, w2) = pack_name_words(&p.name);
        let a0 = TAG_PARKED_ITEM
            | ((idx as u64) << 8)
            | ((p.sid as u64) << 16)
            | ((p.target_pd as u64) << 32)
            | ((p.kind as u64) << 40)
            | ((p.status as u64) << 48)
            | ((p.name_len as u64) << 56);
        if !emit([a0, w1, w2]) {
            return false;
        }
    }
    emit([TAG_COMMIT | ((seq as u64) << 8), 0, 0])
}

/// Tear-free receiver held by Display to assemble `OP_BG_APPS` snapshots.
pub struct Receiver {
    pub shown: State,
    staging: State,
    seq: u16,
    in_flight: bool,
}

impl Receiver {
    pub const fn new() -> Self {
        Self {
            shown: State::EMPTY,
            staging: State::EMPTY,
            seq: 0,
            in_flight: false,
        }
    }

    /// Processes one `OP_BG_APPS` word triple; returns `true` when `shown` changed cleanly.
    pub fn push(&mut self, a0: u64, a1: u64, a2: u64) -> bool {
        let tag = a0 & 0xFF;
        match tag {
            TAG_HEADER => {
                let open_n = ((a0 >> 8) & 0xFF) as usize;
                let parked_n = ((a0 >> 16) & 0xFF) as usize;
                let Some(overlay) = OverlayKind::from_u8(((a0 >> 24) & 0xFF) as u8) else {
                    self.in_flight = false;
                    return false;
                };
                if open_n > MAX_OPEN || parked_n > MAX_PARKED {
                    self.in_flight = false;
                    return false;
                }
                let mut next = State::EMPTY;
                next.open_count = open_n as u8;
                next.parked_count = parked_n as u8;
                next.overlay = overlay;
                next.overlay_idx = ((a0 >> 32) & 0xFF) as u8;
                next.remember = ((a0 >> 40) & 1) != 0;
                next.hover = HoverTarget::decode(((a0 >> 48) & 0xFFFF) as u16);
                next.confirm_sid = (a1 & 0xFFFF) as u16;
                next.confirm_target = ((a1 >> 16) & 0xFF) as u8;
                next.confirm_kind = AppKind::from_u8(((a1 >> 24) & 0xFF) as u8);
                next.bar_right_edge = ((a1 >> 32) & 0xFFFF) as u16;
                self.seq = ((a1 >> 48) & 0xFFFF) as u16;
                self.staging = next;
                self.in_flight = true;
                false
            }
            TAG_CONFIRM_NAME if self.in_flight => {
                self.staging.confirm_name_len = (((a0 >> 8) & 0xFF) as u8).min(NAME_MAX as u8);
                self.staging.confirm_name = unpack_name_words(a1, a2);
                false
            }
            TAG_OPEN_ITEM if self.in_flight => {
                let idx = ((a0 >> 8) & 0xFF) as usize;
                if idx >= self.staging.open_count as usize || idx >= MAX_OPEN {
                    return false;
                }
                self.staging.open[idx] = OpenWindow {
                    sid: ((a0 >> 16) & 0xFFFF) as u16,
                    kind: AppKind::from_u8(((a0 >> 32) & 0xFF) as u8),
                    scene: ((a0 >> 40) & 0xFF) as u8,
                    focused: ((a0 >> 48) & 1) != 0,
                    name: unpack_name_words(a1, a2),
                    name_len: (((a0 >> 56) & 0xFF) as u8).min(NAME_MAX as u8),
                };
                false
            }
            TAG_PARKED_ITEM if self.in_flight => {
                let idx = ((a0 >> 8) & 0xFF) as usize;
                if idx >= self.staging.parked_count as usize || idx >= MAX_PARKED {
                    return false;
                }
                self.staging.parked[idx] = ParkedApp {
                    sid: ((a0 >> 16) & 0xFFFF) as u16,
                    target_pd: ((a0 >> 32) & 0xFF) as u8,
                    kind: AppKind::from_u8(((a0 >> 40) & 0xFF) as u8),
                    status: ((a0 >> 48) & 0xFF) as u8,
                    name: unpack_name_words(a1, a2),
                    name_len: (((a0 >> 56) & 0xFF) as u8).min(NAME_MAX as u8),
                };
                false
            }
            TAG_COMMIT if self.in_flight => {
                let seq = ((a0 >> 8) & 0xFFFF) as u16;
                self.in_flight = false;
                if seq != self.seq || self.shown == self.staging {
                    return false;
                }
                self.shown = self.staging;
                true
            }
            _ => {
                self.in_flight = false;
                false
            }
        }
    }
}

pub type Rect = (usize, usize, usize, usize);

#[inline]
pub const fn rect_contains(r: Rect, x: usize, y: usize) -> bool {
    x >= r.0 && y >= r.1 && x - r.0 < r.2 && y - r.1 < r.3
}

/// Top-bar geometry shared identically by Shell hit-testing and Display rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarLayout {
    pub switcher_pill: Rect,
    pub parked_badges: [Rect; MAX_PARKED],
    pub parked_count: usize,
    /// Leftmost X coordinate available to custom `OP_TRAY_ITEM` icons to stay left of parked apps.
    pub tray_right_edge: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHit {
    SwitcherPill,
    ParkedBadge(usize),
    Outside,
}

impl BarLayout {
    pub fn compute(
        bar_h: usize,
        scale: u32,
        ws_last_right: usize,
        right_edge: usize,
        parked_count: usize,
    ) -> Self {
        let s = scale.clamp(1, 3) as usize;
        let left_bound = ws_last_right.saturating_add(8 * s);
        let pill_w = 54 * s;
        let pill_h = 22 * s;
        let pill_y = bar_h.saturating_sub(pill_h) / 2;
        let switcher_pill = if right_edge >= left_bound + pill_w {
            (right_edge - pill_w, pill_y, pill_w, pill_h)
        } else {
            (0, 0, 0, 0)
        };

        let mut badges = [(0, 0, 0, 0); MAX_PARKED];
        let mut cursor_x = if switcher_pill.2 > 0 {
            switcher_pill.0.saturating_sub(6 * s)
        } else {
            right_edge
        };
        let badge_sz = 22 * s;
        let gap = 4 * s;
        let mut placed = 0usize;
        let n = parked_count.min(MAX_PARKED);
        for i in 0..n {
            if cursor_x < left_bound + badge_sz {
                break;
            }
            let bx = cursor_x - badge_sz;
            badges[i] = (bx, pill_y, badge_sz, badge_sz);
            cursor_x = bx.saturating_sub(gap);
            placed += 1;
        }
        let tray_right_edge = if placed > 0 {
            badges[placed - 1].0.saturating_sub(6 * s)
        } else if switcher_pill.2 > 0 {
            switcher_pill.0.saturating_sub(6 * s)
        } else {
            right_edge
        };

        Self {
            switcher_pill,
            parked_badges: badges,
            parked_count: placed,
            tray_right_edge,
        }
    }

    pub fn hit(&self, x: usize, y: usize) -> BarHit {
        if self.switcher_pill.2 > 0 && rect_contains(self.switcher_pill, x, y) {
            return BarHit::SwitcherPill;
        }
        for i in 0..self.parked_count {
            let r = self.parked_badges[i];
            if r.2 > 0 && rect_contains(r, x, y) {
                return BarHit::ParkedBadge(i);
            }
        }
        BarHit::Outside
    }
}

/// Frosted Noir App Quick-Action Popover layout under a clicked parked badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickActionLayout {
    pub panel: Rect,
    pub scale: u32,
    pub header: Rect,
    pub quick_rows: [Rect; 2],
    pub reopen_btn: Rect,
    pub quit_btn: Rect,
}

impl QuickActionLayout {
    pub fn compute(anchor: Rect, screen_w: usize, screen_h: usize, bar_h: usize, scale: u32) -> Option<Self> {
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = 256 * s;
            let pad = 8 * s;
            let hdr_h = 36 * s;
            let row_h = 26 * s;
            let btn_h = 28 * s;
            let ph = pad * 2 + hdr_h + 4 * s + row_h * 2 + 8 * s + btn_h * 2 + 6 * s;
            if screen_w < pw + 12 * s || screen_h < bar_h + ph + 12 * s {
                return None;
            }
            let margin = 8 * s;
            let raw_x = (anchor.0 + anchor.2 / 2).saturating_sub(pw / 2);
            let x = raw_x.clamp(margin, screen_w.saturating_sub(pw + margin));
            let y = (bar_h + 6 * s).min(screen_h.saturating_sub(ph + margin));
            let iw = pw - 2 * pad;
            let ix = x + pad;
            let mut cur_y = y + pad;
            let header = (ix, cur_y, iw, hdr_h);
            cur_y += hdr_h + 4 * s;
            let q0 = (ix, cur_y, iw, row_h);
            cur_y += row_h;
            let q1 = (ix, cur_y, iw, row_h);
            cur_y += row_h + 8 * s;
            let reopen_btn = (ix, cur_y, iw, btn_h);
            cur_y += btn_h + 6 * s;
            let quit_btn = (ix, cur_y, iw, btn_h);
            Some(Self {
                panel: (x, y, pw, ph),
                scale: sc,
                header,
                quick_rows: [q0, q1],
                reopen_btn,
                quit_btn,
            })
        })
    }
}

/// Android-style "Open Windows & Background Apps" Two-Section Switcher layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitcherLayout {
    pub panel: Rect,
    pub scale: u32,
    pub open_rows: [(Rect, Rect); MAX_OPEN], // (row_focus, close_x_btn)
    pub open_visible: usize,
    pub open_hdr_y: usize,
    pub bg_hdr_y: usize,
    pub parked_rows: [(Rect, Rect, Rect); MAX_PARKED], // (row, reopen_btn, quit_btn)
    pub parked_visible: usize,
}

impl SwitcherLayout {
    pub fn compute(
        anchor_right: usize,
        open_count: usize,
        parked_count: usize,
        screen_w: usize,
        screen_h: usize,
        bar_h: usize,
        scale: u32,
    ) -> Option<Self> {
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = 336 * s;
            let pad = 10 * s;
            let title_h = 24 * s;
            let sec_hdr_h = 20 * s;
            let open_row_h = 28 * s;
            let bg_row_h = 30 * s;
            let open_v = open_count.min(MAX_OPEN);
            let parked_v = parked_count.min(MAX_PARKED);
            let open_block_h = if open_v == 0 { 24 * s } else { open_v * open_row_h };
            let bg_block_h = if parked_v == 0 { 24 * s } else { parked_v * bg_row_h };
            let ph = pad * 2 + title_h + 4 * s + sec_hdr_h + open_block_h + 10 * s + sec_hdr_h + bg_block_h;
            if screen_w < pw + 12 * s || screen_h < bar_h + ph + 12 * s {
                return None;
            }
            let margin = 8 * s;
            let x = anchor_right.saturating_sub(pw).clamp(margin, screen_w.saturating_sub(pw + margin));
            let y = (bar_h + 6 * s).min(screen_h.saturating_sub(ph + margin));
            let ix = x + pad;
            let iw = pw - 2 * pad;

            let mut cur_y = y + pad + title_h + 4 * s;
            let open_hdr_y = cur_y;
            cur_y += sec_hdr_h;

            let mut open_rows = [((0, 0, 0, 0), (0, 0, 0, 0)); MAX_OPEN];
            for i in 0..open_v {
                let close_sz = 20 * s;
                let close_r = (ix + iw - close_sz - 2 * s, cur_y + (open_row_h - close_sz) / 2, close_sz, close_sz);
                let focus_r = (ix, cur_y, iw.saturating_sub(close_sz + 6 * s), open_row_h);
                open_rows[i] = (focus_r, close_r);
                cur_y += open_row_h;
            }
            if open_v == 0 {
                cur_y += 24 * s;
            }
            cur_y += 10 * s;
            let bg_hdr_y = cur_y;
            cur_y += sec_hdr_h;

            let mut parked_rows = [((0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)); MAX_PARKED];
            let quit_w = 50 * s;
            let reopen_w = 62 * s;
            let btn_h = 22 * s;
            let gap = 4 * s;
            for i in 0..parked_v {
                let by = cur_y + (bg_row_h - btn_h) / 2;
                let quit_r = (ix + iw - quit_w, by, quit_w, btn_h);
                let reopen_r = (quit_r.0.saturating_sub(reopen_w + gap), by, reopen_w, btn_h);
                let row_r = (ix, cur_y, iw, bg_row_h);
                parked_rows[i] = (row_r, reopen_r, quit_r);
                cur_y += bg_row_h;
            }

            Some(Self {
                panel: (x, y, pw, ph),
                scale: sc,
                open_rows,
                open_visible: open_v,
                open_hdr_y,
                bg_hdr_y,
                parked_rows,
                parked_visible: parked_v,
            })
        })
    }
}

/// Frosted macOS Noir Close Confirmation Sheet layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseSheetLayout {
    pub panel: Rect,
    pub scale: u32,
    pub remember_row: Rect,
    pub checkbox: Rect,
    pub cancel_btn: Rect,
    pub quit_btn: Rect,
    pub keep_btn: Rect,
}

impl CloseSheetLayout {
    pub fn compute(screen_w: usize, screen_h: usize, bar_h: usize, scale: u32) -> Option<Self> {
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = 420 * s;
            let ph = 158 * s;
            let margin = 12 * s;
            if screen_w < pw + margin * 2 || screen_h < bar_h + ph + margin * 2 {
                return None;
            }
            let x = (screen_w - pw) / 2;
            let work_h = screen_h.saturating_sub(bar_h);
            let y = bar_h + work_h.saturating_sub(ph) / 3;
            let pad = 16 * s;
            let remember_y = y + 72 * s;
            let box_sz = 16 * s;
            let remember_row = (x + pad, remember_y, pw - 2 * pad, 24 * s);
            let checkbox = (x + pad, remember_y + 4 * s, box_sz, box_sz);

            let btn_h = 30 * s;
            let btn_y = y + ph - pad - btn_h;
            let cancel_w = 72 * s;
            let quit_w = 90 * s;
            let keep_w = 176 * s;
            let cancel_btn = (x + pad, btn_y, cancel_w, btn_h);
            let keep_btn = (x + pw - pad - keep_w, btn_y, keep_w, btn_h);
            let quit_btn = (keep_btn.0.saturating_sub(8 * s + quit_w), btn_y, quit_w, btn_h);
            Some(Self {
                panel: (x, y, pw, ph),
                scale: sc,
                remember_row,
                checkbox,
                cancel_btn,
                quit_btn,
                keep_btn,
            })
        })
    }
}

/// Frosted Noir Force Quit Applications dialog layout (`Super+Alt+Esc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForceQuitLayout {
    pub panel: Rect,
    pub scale: u32,
    pub rows: [Rect; MAX_OPEN],
    pub visible: usize,
    pub close_btn: Rect,
    pub force_quit_btn: Rect,
}

impl ForceQuitLayout {
    pub fn compute(item_count: usize, screen_w: usize, screen_h: usize, bar_h: usize, scale: u32) -> Option<Self> {
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = 376 * s;
            let pad = 14 * s;
            let hdr_h = 46 * s;
            let row_h = 26 * s;
            let visible = item_count.min(MAX_OPEN);
            let list_h = if visible == 0 { 32 * s } else { visible * row_h };
            let btn_h = 28 * s;
            let ph = pad * 2 + hdr_h + list_h + 14 * s + btn_h;
            if screen_w < pw + 24 * s || screen_h < bar_h + ph + 24 * s {
                return None;
            }
            let x = (screen_w - pw) / 2;
            let y = bar_h + screen_h.saturating_sub(bar_h + ph) / 3;
            let mut rows = [(0, 0, 0, 0); MAX_OPEN];
            let mut ry = y + pad + hdr_h;
            for i in 0..visible {
                rows[i] = (x + pad, ry, pw - 2 * pad, row_h);
                ry += row_h;
            }
            let btn_y = y + ph - pad - btn_h;
            let fq_w = 110 * s;
            let close_w = 78 * s;
            let force_quit_btn = (x + pw - pad - fq_w, btn_y, fq_w, btn_h);
            let close_btn = (x + pad, btn_y, close_w, btn_h);
            Some(Self {
                panel: (x, y, pw, ph),
                scale: sc,
                rows,
                visible,
                close_btn,
                force_quit_btn,
            })
        })
    }
}

/// Frosted Noir "About This SexOS" card layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AboutLayout {
    pub panel: Rect,
    pub scale: u32,
    pub settings_btn: Rect,
    pub close_btn: Rect,
}

impl AboutLayout {
    pub fn compute(screen_w: usize, screen_h: usize, bar_h: usize, scale: u32) -> Option<Self> {
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = 340 * s;
            let ph = 212 * s;
            if screen_w < pw + 24 * s || screen_h < bar_h + ph + 24 * s {
                return None;
            }
            let x = (screen_w - pw) / 2;
            let y = bar_h + screen_h.saturating_sub(bar_h + ph) / 3;
            let pad = 16 * s;
            let btn_h = 28 * s;
            let by = y + ph - pad - btn_h;
            let settings_w = 140 * s;
            let close_w = 80 * s;
            let close_btn = (x + pad, by, close_w, btn_h);
            let settings_btn = (x + pw - pad - settings_w, by, settings_w, btn_h);
            Some(Self {
                panel: (x, y, pw, ph),
                scale: sc,
                settings_btn,
                close_btn,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayLayout {
    QuickAction(QuickActionLayout),
    Switcher(SwitcherLayout),
    CloseConfirm(CloseSheetLayout),
    ForceQuit(ForceQuitLayout),
    AboutSexOs(AboutLayout),
}

impl OverlayLayout {
    pub fn compute(
        state: &State,
        bar: &BarLayout,
        screen_w: usize,
        screen_h: usize,
        bar_h: usize,
        scale: u32,
    ) -> Option<Self> {
        match state.overlay {
            OverlayKind::None => None,
            OverlayKind::QuickAction => {
                let idx = (state.overlay_idx as usize).min(bar.parked_count.saturating_sub(1));
                let anchor = if idx < bar.parked_count {
                    bar.parked_badges[idx]
                } else {
                    bar.switcher_pill
                };
                QuickActionLayout::compute(anchor, screen_w, screen_h, bar_h, scale).map(Self::QuickAction)
            }
            OverlayKind::Switcher => {
                let anchor_r = if bar.switcher_pill.2 > 0 {
                    bar.switcher_pill.0 + bar.switcher_pill.2
                } else {
                    state.bar_right_edge as usize
                };
                SwitcherLayout::compute(
                    anchor_r,
                    state.open_count as usize,
                    state.parked_count as usize,
                    screen_w,
                    screen_h,
                    bar_h,
                    scale,
                )
                .map(Self::Switcher)
            }
            OverlayKind::CloseConfirm => {
                CloseSheetLayout::compute(screen_w, screen_h, bar_h, scale).map(Self::CloseConfirm)
            }
            OverlayKind::ForceQuit => {
                let count = (state.open_count as usize + state.parked_count as usize).min(MAX_OPEN);
                ForceQuitLayout::compute(count, screen_w, screen_h, bar_h, scale).map(Self::ForceQuit)
            }
            OverlayKind::AboutSexOs => AboutLayout::compute(screen_w, screen_h, bar_h, scale).map(Self::AboutSexOs),
        }
    }

    pub const fn panel(&self) -> Rect {
        match self {
            Self::QuickAction(l) => l.panel,
            Self::Switcher(l) => l.panel,
            Self::CloseConfirm(l) => l.panel,
            Self::ForceQuit(l) => l.panel,
            Self::AboutSexOs(l) => l.panel,
        }
    }

    pub fn hit(&self, x: usize, y: usize) -> OverlayHit {
        if !rect_contains(self.panel(), x, y) {
            return OverlayHit::Outside;
        }
        match self {
            Self::QuickAction(l) => {
                if rect_contains(l.quick_rows[0], x, y) {
                    return OverlayHit::QuickAction(0);
                }
                if rect_contains(l.quick_rows[1], x, y) {
                    return OverlayHit::QuickAction(1);
                }
                if rect_contains(l.reopen_btn, x, y) {
                    return OverlayHit::QuickReopen;
                }
                if rect_contains(l.quit_btn, x, y) {
                    return OverlayHit::QuickQuit;
                }
                OverlayHit::Inside
            }
            Self::Switcher(l) => {
                for i in 0..l.open_visible {
                    let (focus_r, close_r) = l.open_rows[i];
                    if rect_contains(close_r, x, y) {
                        return OverlayHit::SwitcherOpenClose(i);
                    }
                    if rect_contains(focus_r, x, y) {
                        return OverlayHit::SwitcherOpenFocus(i);
                    }
                }
                for i in 0..l.parked_visible {
                    let (_, reopen_r, quit_r) = l.parked_rows[i];
                    if rect_contains(reopen_r, x, y) {
                        return OverlayHit::SwitcherParkedReopen(i);
                    }
                    if rect_contains(quit_r, x, y) {
                        return OverlayHit::SwitcherParkedQuit(i);
                    }
                }
                OverlayHit::Inside
            }
            Self::CloseConfirm(l) => {
                if rect_contains(l.remember_row, x, y) || rect_contains(l.checkbox, x, y) {
                    return OverlayHit::CloseRememberToggle;
                }
                if rect_contains(l.keep_btn, x, y) {
                    return OverlayHit::CloseKeepInSilkBar;
                }
                if rect_contains(l.quit_btn, x, y) {
                    return OverlayHit::CloseQuitApp;
                }
                if rect_contains(l.cancel_btn, x, y) {
                    return OverlayHit::CloseCancel;
                }
                OverlayHit::Inside
            }
            Self::ForceQuit(l) => {
                for i in 0..l.visible {
                    if rect_contains(l.rows[i], x, y) {
                        return OverlayHit::ForceSelect(i);
                    }
                }
                if rect_contains(l.force_quit_btn, x, y) {
                    return OverlayHit::ForceQuitSelected;
                }
                if rect_contains(l.close_btn, x, y) {
                    return OverlayHit::ForceClose;
                }
                OverlayHit::Inside
            }
            Self::AboutSexOs(l) => {
                if rect_contains(l.settings_btn, x, y) {
                    return OverlayHit::AboutOpenSettings;
                }
                if rect_contains(l.close_btn, x, y) {
                    return OverlayHit::AboutClose;
                }
                OverlayHit::Inside
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayHit {
    Outside,
    Inside,
    QuickAction(u8),
    QuickReopen,
    QuickQuit,
    SwitcherOpenFocus(usize),
    SwitcherOpenClose(usize),
    SwitcherParkedReopen(usize),
    SwitcherParkedQuit(usize),
    CloseRememberToggle,
    CloseKeepInSilkBar,
    CloseQuitApp,
    CloseCancel,
    ForceSelect(usize),
    ForceQuitSelected,
    ForceClose,
    AboutOpenSettings,
    AboutClose,
}
