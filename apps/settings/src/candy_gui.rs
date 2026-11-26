// SPDX-License-Identifier: GPL-3.0-only
//! Candy Lavender + HyprMod-inspired GUI for SexOS Settings (`apps/settings`).
//!
//! Renders a categorized Candy Lavender sidebar, top toolbar with Undo/Reset,
//! live-preview Appearance controls (Theme pills, Accent color swatches, Frosted
//! Glass & Animations toggles), Windows & Tabs chrome/tiling controls (Hyprland
//! & KDE styles), Keybinds reference & profile switcher, and one-click Named
//! Profiles — all with instant apply and zero heap allocation.

use sex_config::{id, Config, Kind, KEYS};
use sex_font::cache::{GlyphCache, Key as GlyphKey};
use sex_font::raster::scratch_len;
use sex_font::Font;
use sex_theme::candy_ui::{
    button_text_color, draw_candy_badge, draw_candy_button, draw_candy_panel,
    draw_candy_selection_row, draw_candy_swatch, draw_candy_tab, draw_candy_toggle,
    draw_candy_toolbar, fill_rounded_rect, mix_rgb, ButtonState, ButtonStyle, CandyPalette,
};
use sex_theme::Mode;

use crate::model::{self, Model};

const CELL_W: usize = 8;
const CELL_H: usize = 16;
const GLYPH_SLOTS: usize = 128;

pub const SIDEBAR_W: i32 = 188;
pub const HEADER_H: i32 = 42;
pub const MAX_HITS: usize = 96;

/// Accent color swatches in the Appearance picker (Candy Lavender default first).
pub const CANDY_SWATCHES: [(&str, i32); 6] = [
    ("Candy Lavender", 0xA78BFA),
    ("Orchid", 0xD6336C),
    ("Mint", 0x34D399),
    ("Sky", 0x60A5FA),
    ("Peach", 0xFB923C),
    ("Slate", 0x64748B),
];

/// Window corner radius presets (in px).
pub const CORNER_RADII: [(&str, u8); 3] = [("Sharp 4px", 4), ("Soft 8px", 8), ("Candy 12px", 12)];

/// Tab bar visual style presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBarStyle {
    SilkPill,
    Classic,
}
impl TabBarStyle {
    pub const fn label(self) -> &'static str {
        match self {
            TabBarStyle::SilkPill => "SilkBar Pill",
            TabBarStyle::Classic => "KDE Classic",
        }
    }
}

/// Window tiling behavior presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TilingStyle {
    HyprlandMaster,
    KdeFloat,
    HybridCandy,
}
impl TilingStyle {
    pub const fn label(self) -> &'static str {
        match self {
            TilingStyle::HyprlandMaster => "Hyprland Master",
            TilingStyle::KdeFloat => "KDE Floating",
            TilingStyle::HybridCandy => "Hybrid Candy",
        }
    }
}

/// Hotkey profile presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeybindProfile {
    HyprlandKde,
    HyprlandPure,
    KdeClassic,
}
impl KeybindProfile {
    pub const fn label(self) -> &'static str {
        match self {
            KeybindProfile::HyprlandKde => "Hyprland + KDE",
            KeybindProfile::HyprlandPure => "Hyprland",
            KeybindProfile::KdeClassic => "KDE Classic",
        }
    }
}

/// HyprMod-inspired named desktop profile (one-click switch + instant apply).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfilePreset {
    pub name: &'static str,
    pub subtitle: &'static str,
    pub theme: i32,
    pub accent: i32,
    pub reduce_transparency: i32,
    pub animations: i32,
    pub corner_radius_idx: usize,
    pub tab_bar: TabBarStyle,
    pub tiling: TilingStyle,
    pub keybinds: KeybindProfile,
}

pub const PROFILES: [ProfilePreset; 4] = [
    ProfilePreset {
        name: "Candy Lavender",
        subtitle: "Frosted lavender glass, Orchid accent, Hybrid tiling",
        theme: 1,
        accent: 0xA78BFA,
        reduce_transparency: 0,
        animations: 1,
        corner_radius_idx: 2,
        tab_bar: TabBarStyle::SilkPill,
        tiling: TilingStyle::HybridCandy,
        keybinds: KeybindProfile::HyprlandKde,
    },
    ProfilePreset {
        name: "Midnight Orchid",
        subtitle: "Deep plum dark mode, SexOS Rose accent, SilkBar pills",
        theme: 1,
        accent: 0xD6336C,
        reduce_transparency: 0,
        animations: 1,
        corner_radius_idx: 2,
        tab_bar: TabBarStyle::SilkPill,
        tiling: TilingStyle::HyprlandMaster,
        keybinds: KeybindProfile::HyprlandKde,
    },
    ProfilePreset {
        name: "Hyprland Compact",
        subtitle: "Crisp dark tiling, Sky accent, compact 4px corners",
        theme: 1,
        accent: 0x60A5FA,
        reduce_transparency: 0,
        animations: 1,
        corner_radius_idx: 0,
        tab_bar: TabBarStyle::SilkPill,
        tiling: TilingStyle::HyprlandMaster,
        keybinds: KeybindProfile::HyprlandPure,
    },
    ProfilePreset {
        name: "KDE Classic",
        subtitle: "Airy light mode, opaque chrome, classic floating windows",
        theme: 0,
        accent: 0x64748B,
        reduce_transparency: 1,
        animations: 1,
        corner_radius_idx: 1,
        tab_bar: TabBarStyle::Classic,
        tiling: TilingStyle::KdeFloat,
        keybinds: KeybindProfile::KdeClassic,
    },
];

/// Reference hotkeys shown in the Keybinds pane.
pub const HOTKEY_ROWS: [(&str, &str, &str); 8] = [
    ("Super + Enter", "Launch Spindle terminal", "Hyprland"),
    ("Super + Q / Alt + F4", "Close focused window", "Hyprland + KDE"),
    ("Super + 1..9", "Switch workspace / scene", "Hyprland + KDE"),
    ("Alt + Tab / Super + J/K", "Cycle window focus", "KDE + Hyprland"),
    ("Super + F / F11", "Toggle fullscreen / zoom", "Hyprland + KDE"),
    ("Super + V", "Toggle floating / tiled mode", "Hyprland"),
    ("Super + L / Ctrl+Alt+L", "Lock session immediately", "KDE + Hyprland"),
    ("Super + Space", "Open SilkBar launcher", "Candy Desktop"),
];

/// Maximum partitions (plus free space runs) tracked by the Candy graphical installer.
pub const MAX_INSTALL_PARTS: usize = 16;
/// Minimum partition size (1.5 GiB in 512-byte sectors) required to replace with SexOS A/B + Store.
pub const MIN_INSTALL_SECTORS: u64 = 1536 * 2048;

/// Partition category for color-coding in the Candy GPT Partition Bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskPartKind {
    Esp,
    Msr,
    Windows,
    Linux,
    SexOs,
    FreeSpace,
    Other,
}

impl DiskPartKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Esp => "ESP",
            Self::Msr => "MSR",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::SexOs => "SexOS",
            Self::FreeSpace => "Free Space",
            Self::Other => "Other",
        }
    }

    /// Distinct Candy palette RGB color for each partition type in the GPT bar.
    pub const fn color(self) -> u32 {
        match self {
            Self::Esp => 0x00F5_9E0B,       // Amber (EFI System)
            Self::Msr => 0x0064_748B,       // Slate (MSR)
            Self::Windows => 0x0038_BDF8,   // Sky Blue (Windows)
            Self::Linux => 0x0034_D399,     // Mint/Emerald (Linux / Garuda / Void)
            Self::SexOs => 0x00A7_8BFA,     // Candy Lavender (SexOS)
            Self::FreeSpace => 0x0047_5569, // Muted Slate (Free Space)
            Self::Other => 0x00F4_72B6,     // Orchid Pink (Other)
        }
    }

    pub fn from_code(code: &[u8]) -> Self {
        match code {
            b"esp" => Self::Esp,
            b"msr" => Self::Msr,
            b"win" => Self::Windows,
            b"lin" => Self::Linux,
            b"sex" => Self::SexOs,
            b"free" => Self::FreeSpace,
            _ => Self::Other,
        }
    }
}

/// One partition or free-space entry displayed in the Candy graphical installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskPartRow {
    /// 1-based GPT partition number (`0` for Free Space).
    pub part_num: u16,
    pub kind: DiskPartKind,
    pub first_lba: u64,
    pub sectors: u64,
    pub replaceable: bool,
    pub name: [u8; 32],
    pub name_len: usize,
}

impl DiskPartRow {
    pub const EMPTY: Self = Self {
        part_num: 0,
        kind: DiskPartKind::Other,
        first_lba: 0,
        sectors: 0,
        replaceable: false,
        name: [0; 32],
        name_len: 0,
    };

    pub fn new(part_num: u16, kind: DiskPartKind, first_lba: u64, sectors: u64, replaceable: bool, label: &[u8]) -> Self {
        let mut row = Self {
            part_num,
            kind,
            first_lba,
            sectors,
            // Enforce protection invariant in UI too: ESP, MSR, and < 1.5 GiB are always locked.
            replaceable: replaceable
                && !matches!(kind, DiskPartKind::Esp | DiskPartKind::Msr)
                && sectors >= MIN_INSTALL_SECTORS,
            name: [0; 32],
            name_len: 0,
        };
        let n = label.len().min(row.name.len());
        row.name[..n].copy_from_slice(&label[..n]);
        row.name_len = n;
        row
    }

    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len]
    }
}

/// Zero-heap state for the Candy Lavender Graphical Installer & Partition Manager.
pub struct InstallState {
    pub total_sectors: u64,
    pub parts: [DiskPartRow; MAX_INSTALL_PARTS],
    pub part_count: usize,
    pub selected: Option<usize>,
    pub loaded: bool,
    pub plan_ready: bool,
    pub report: [u8; 512],
    pub report_len: usize,
}

impl InstallState {
    pub const fn new() -> Self {
        Self {
            total_sectors: 0,
            parts: [DiskPartRow::EMPTY; MAX_INSTALL_PARTS],
            part_count: 0,
            selected: None,
            loaded: false,
            plan_ready: false,
            report: [0; 512],
            report_len: 0,
        }
    }

    pub fn set_report(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.report.len());
        self.report[..n].copy_from_slice(&bytes[..n]);
        self.report_len = n;
    }

    pub fn report_bytes(&self) -> &[u8] {
        &self.report[..self.report_len]
    }

    /// Selects a partition or free space entry ONLY if it is replaceable (never ESP/MSR/tiny).
    pub fn select_part(&mut self, idx: usize) -> bool {
        if idx < self.part_count && self.parts[idx].replaceable {
            self.selected = Some(idx);
            self.plan_ready = false;
            true
        } else {
            false
        }
    }

    pub fn selected_row(&self) -> Option<&DiskPartRow> {
        self.selected.and_then(|i| (i < self.part_count).then_some(&self.parts[i]))
    }

    /// Parses machine-readable output from `sexfiles` (`INSTALL_LIST | INSTALL_LIST_RAW`).
    pub fn parse_raw(&mut self, data: &[u8]) {
        self.total_sectors = 0;
        self.part_count = 0;
        self.selected = None;
        self.plan_ready = false;
        self.loaded = true;
        for line in data.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"DISK ") {
                self.total_sectors = parse_u64_ascii(rest);
            } else if let Some(rest) = line.strip_prefix(b"PART ") {
                if self.part_count >= MAX_INSTALL_PARTS {
                    continue;
                }
                let (num_b, r) = split_ws(rest);
                let (kind_b, r) = split_ws(r);
                let (first_b, r) = split_ws(r);
                let (sec_b, r) = split_ws(r);
                let (rep_b, name_b) = split_ws(r);
                let num = parse_u64_ascii(num_b) as u16;
                let kind = DiskPartKind::from_code(kind_b);
                let first = parse_u64_ascii(first_b);
                let sectors = parse_u64_ascii(sec_b);
                let replaceable = rep_b == b"1";
                let label = if name_b.is_empty() { kind.label().as_bytes() } else { name_b };
                self.parts[self.part_count] = DiskPartRow::new(num, kind, first, sectors, replaceable, label);
                self.part_count += 1;
            } else if let Some(rest) = line.strip_prefix(b"FREE ") {
                if self.part_count >= MAX_INSTALL_PARTS {
                    continue;
                }
                let (first_b, sec_b) = split_ws(rest);
                let first = parse_u64_ascii(first_b);
                let sectors = parse_u64_ascii(sec_b);
                self.parts[self.part_count] =
                    DiskPartRow::new(0, DiskPartKind::FreeSpace, first, sectors, true, b"Unallocated Free Space");
                self.part_count += 1;
            } else if line.starts_with(b"ERR ") {
                self.set_report(b"No GPT partition table found. Use Spindle `install` for blank disks.");
            }
        }
        // Auto-select the first replaceable target (if any) for immediate clarity.
        for i in 0..self.part_count {
            if self.parts[i].replaceable {
                self.selected = Some(i);
                break;
            }
        }
    }

    /// Formats the safety summary line into `out`; returns `(len, is_replace_warning)`.
    pub fn safety_summary(&self, out: &mut [u8; 160]) -> (usize, bool) {
        let mut n = 0usize;
        let mut push = |s: &[u8]| {
            let take = s.len().min(out.len().saturating_sub(n));
            out[n..n + take].copy_from_slice(&s[..take]);
            n += take;
        };
        let Some(row) = self.selected_row() else {
            push(b"Select a replaceable partition (>= 1.5 GiB) or Free Space. ESP & tiny partitions are locked.");
            return (n, false);
        };
        let mut sz = [0u8; 16];
        let sz_len = format_size_short(row.sectors, &mut sz);
        if row.kind == DiskPartKind::FreeSpace {
            push(b"Will install SexOS A/B + Store into Free Space (");
            push(&sz[..sz_len]);
            push(b"). All existing partitions remain untouched.");
            (n, false)
        } else {
            let mut num_buf = [0u8; 8];
            let num_len = format_u64_ascii(row.part_num as u64, &mut num_buf);
            push(b"Will replace ONLY Partition #");
            push(&num_buf[..num_len]);
            push(b" (");
            push(row.name_bytes());
            push(b", ");
            push(&sz[..sz_len]);
            push(b") with SexOS A/B + Store. All other partitions remain untouched.");
            (n, true)
        }
    }
}

fn split_ws(mut s: &[u8]) -> (&[u8], &[u8]) {
    while let Some(&b) = s.first() {
        if b == b' ' || b == b'\t' {
            s = &s[1..];
        } else {
            break;
        }
    }
    let mut i = 0;
    while i < s.len() && s[i] != b' ' && s[i] != b'\t' {
        i += 1;
    }
    let head = &s[..i];
    let mut tail = &s[i..];
    while let Some(&b) = tail.first() {
        if b == b' ' || b == b'\t' {
            tail = &tail[1..];
        } else {
            break;
        }
    }
    (head, tail)
}

fn parse_u64_ascii(s: &[u8]) -> u64 {
    let mut v = 0u64;
    for &b in s {
        if b.is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add((b - b'0') as u64);
        } else {
            break;
        }
    }
    v
}

fn format_u64_ascii(mut v: u64, out: &mut [u8]) -> usize {
    let mut tmp = [0u8; 20];
    let mut k = 0;
    loop {
        tmp[k] = b'0' + (v % 10) as u8;
        k += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let mut n = 0;
    while k > 0 && n < out.len() {
        k -= 1;
        out[n] = tmp[k];
        n += 1;
    }
    n
}

fn format_size_short(sectors: u64, out: &mut [u8; 16]) -> usize {
    let mut n = 0usize;
    if sectors < 2048 * 1024 {
        let mib = sectors / 2048;
        n += format_u64_ascii(mib, &mut out[n..]);
        let suf = b" MiB";
        let take = suf.len().min(out.len() - n);
        out[n..n + take].copy_from_slice(&suf[..take]);
        n += take;
    } else {
        let gib = sectors / (2048 * 1024);
        let tenths = sectors % (2048 * 1024) * 10 / (2048 * 1024);
        n += format_u64_ascii(gib, &mut out[n..]);
        if n < out.len() {
            out[n] = b'.';
            n += 1;
        }
        if n < out.len() {
            out[n] = b'0' + tenths as u8;
            n += 1;
        }
        let suf = b" GiB";
        let take = suf.len().min(out.len() - n);
        out[n..n + take].copy_from_slice(&suf[..take]);
        n += take;
    }
    n
}

/// Active top-level Candy sidebar tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarView {
    Appearance,
    WindowsTabs,
    Keybinds,
    Profiles,
    /// Candy Lavender Graphical Installer & Partition Manager.
    Install,
    /// One of the existing `sex-config` schema / info pages by index.
    SystemPage(usize),
}

/// Snapshot of editable desktop preferences for one-click Undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefSnapshot {
    pub theme: i32,
    pub accent: i32,
    pub reduce_transparency: i32,
    pub animations: i32,
    pub corner_radius_idx: usize,
    pub tab_bar: TabBarStyle,
    pub tiling: TilingStyle,
    pub keybinds: KeybindProfile,
}

/// Interactive hitbox action emitted on click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    SelectView(SidebarView),
    SetTheme(i32),
    SetAccent(i32),
    ToggleFrostedGlass,
    ToggleAnimations,
    SetCornerRadius(usize),
    SetTabBar(TabBarStyle),
    SetTiling(TilingStyle),
    SetKeybinds(KeybindProfile),
    ApplyProfile(usize),
    Undo,
    ResetDefaults,
    SelectRow(usize),
    StepRow(usize, bool),
    SelectInstallPart(usize),
    InstallRefresh,
    InstallPlan,
    InstallExecute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub action: UiAction,
}

impl HitRect {
    pub const fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// Settings GUI state (kept in static or on stack; zero heap).
pub struct GuiState {
    pub view: SidebarView,
    pub corner_radius_idx: usize,
    pub tab_bar: TabBarStyle,
    pub tiling: TilingStyle,
    pub keybinds: KeybindProfile,
    pub active_profile: Option<usize>,
    pub undo: Option<PrefSnapshot>,
    pub hovered: Option<UiAction>,
    pub pressed: Option<UiAction>,
    pub hits: [Option<HitRect>; MAX_HITS],
    pub hit_count: usize,
    pub install: InstallState,
}

impl GuiState {
    pub const fn new() -> Self {
        Self {
            view: SidebarView::Appearance,
            corner_radius_idx: 2,
            tab_bar: TabBarStyle::SilkPill,
            tiling: TilingStyle::HybridCandy,
            keybinds: KeybindProfile::HyprlandKde,
            active_profile: Some(0),
            undo: None,
            hovered: None,
            pressed: None,
            hits: [None; MAX_HITS],
            hit_count: 0,
            install: InstallState::new(),
        }
    }

    pub fn snapshot(&self, values: &Config) -> PrefSnapshot {
        PrefSnapshot {
            theme: values.value(id::THEME),
            accent: values.value(id::ACCENT),
            reduce_transparency: values.value(id::REDUCE_TRANSPARENCY),
            animations: values.value(id::ANIMATIONS),
            corner_radius_idx: self.corner_radius_idx,
            tab_bar: self.tab_bar,
            tiling: self.tiling,
            keybinds: self.keybinds,
        }
    }

    pub fn save_undo(&mut self, values: &Config) {
        self.undo = Some(self.snapshot(values));
    }

    pub fn hit_at(&self, px: i32, py: i32) -> Option<UiAction> {
        for slot in self.hits[..self.hit_count].iter().rev() {
            if let Some(h) = slot {
                if h.contains(px, py) {
                    return Some(h.action);
                }
            }
        }
        None
    }

    fn clear_hits(&mut self) {
        self.hit_count = 0;
    }

    fn push_hit(&mut self, x: i32, y: i32, w: i32, h: i32, action: UiAction) {
        if self.hit_count < MAX_HITS && w > 0 && h > 0 {
            self.hits[self.hit_count] = Some(HitRect { x, y, w, h, action });
            self.hit_count += 1;
        }
    }

    fn btn_state(&self, action: UiAction) -> ButtonState {
        if self.pressed == Some(action) {
            ButtonState::Pressed
        } else if self.hovered == Some(action) {
            ButtonState::Hover
        } else {
            ButtonState::Rest
        }
    }

    /// Applies a named profile and returns the `(key_id, value)` pairs to persist.
    pub fn apply_profile(&mut self, idx: usize, values: &Config) -> [(u16, i32); 4] {
        let p = PROFILES[idx.min(PROFILES.len() - 1)];
        self.save_undo(values);
        self.corner_radius_idx = p.corner_radius_idx;
        self.tab_bar = p.tab_bar;
        self.tiling = p.tiling;
        self.keybinds = p.keybinds;
        self.active_profile = Some(idx.min(PROFILES.len() - 1));
        [
            (id::THEME, p.theme),
            (id::ACCENT, p.accent),
            (id::REDUCE_TRANSPARENCY, p.reduce_transparency),
            (id::ANIMATIONS, p.animations),
        ]
    }

    /// Reverts to the previous undo snapshot if one exists.
    pub fn take_undo(&mut self) -> Option<[(u16, i32); 4]> {
        let snap = self.undo.take()?;
        self.corner_radius_idx = snap.corner_radius_idx;
        self.tab_bar = snap.tab_bar;
        self.tiling = snap.tiling;
        self.keybinds = snap.keybinds;
        self.active_profile = None;
        Some([
            (id::THEME, snap.theme),
            (id::ACCENT, snap.accent),
            (id::REDUCE_TRANSPARENCY, snap.reduce_transparency),
            (id::ANIMATIONS, snap.animations),
        ])
    }

    /// Resets to the default `Candy Lavender` profile.
    pub fn reset_defaults(&mut self, values: &Config) -> [(u16, i32); 4] {
        self.apply_profile(0, values)
    }

    /// Builds a live CandyPalette customized by the active Theme mode and Accent color.
    pub fn palette(&self, values: &Config) -> CandyPalette {
        let mode = Mode::from_setting(values.value(id::THEME));
        let mut pal = CandyPalette::for_mode(mode);
        let accent = (values.value(id::ACCENT) as u32) & 0x00FF_FFFF;
        if mode != Mode::HighContrast && accent != 0 {
            pal.accent = accent;
            pal.accent_hover = mix_rgb(accent, 0x00FF_FFFF, 36);
            pal.accent_pressed = mix_rgb(accent, 0x0014_0D24, 48);
            pal.rim_accent = mix_rgb(accent, pal.specular, 72);
            pal.selection = mix_rgb(pal.card, accent, if mode == Mode::Light { 42 } else { 68 });
        }
        pal
    }
}

/// Compact Geist Mono glyph cache for Settings GUI.
pub struct Glyphs {
    font: Option<Font<'static>>,
    loaded: bool,
    cache: GlyphCache<GLYPH_SLOTS, { GLYPH_SLOTS * CELL_W * CELL_H }>,
    acc: [i32; scratch_len(CELL_W, CELL_H)],
}

impl Glyphs {
    pub const fn new() -> Self {
        Self {
            font: None,
            loaded: false,
            cache: GlyphCache::new(),
            acc: [0; scratch_len(CELL_W, CELL_H)],
        }
    }

    fn cell(&mut self, c: u8) -> Option<&[u8]> {
        if !self.loaded {
            self.font = Font::new(sex_font::GEIST_MONO);
            self.loaded = true;
        }
        let font = self.font.as_ref()?;
        let acc = &mut self.acc;
        let key = GlyphKey { font: 0, scalar: c as u32, size: 0, phase: 0, w: CELL_W as u8, h: CELL_H as u8 };
        self.cache.get(key, |out| sex_font::draw_cell(font, c as u32, CELL_W, CELL_H, acc, out))
    }
}

fn draw_text(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    max_w: i32,
    text: &[u8],
    color: u32,
) {
    let mut at = x;
    let limit = (x + max_w).min(w as i32);
    for &b in text {
        if at + CELL_W as i32 > limit {
            break;
        }
        let c = if (0x21..0x7F).contains(&b) {
            b
        } else if b == b' ' {
            at += CELL_W as i32;
            continue;
        } else {
            b'?'
        };
        if let Some(cov) = g.cell(c) {
            for (i, &a) in cov.iter().enumerate() {
                if a == 0 {
                    continue;
                }
                let px = at + (i % CELL_W) as i32;
                let py = y + (i / CELL_W) as i32;
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    let idx = py as usize * w + px as usize;
                    buf[idx] = mix_rgb(buf[idx], color, a);
                }
            }
        }
        at += CELL_W as i32;
    }
}

fn fill_solid(buf: &mut [u32], w: usize, h: usize, x: i32, y: i32, rw: i32, rh: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = x.saturating_add(rw).clamp(0, w as i32) as usize;
    let y1 = y.saturating_add(rh).clamp(0, h as i32) as usize;
    if x0 >= x1 {
        return;
    }
    for row in y0..y1 {
        buf[row * w + x0..row * w + x1].fill(color);
    }
}

/// Renders the entire Candy Lavender Settings GUI into `buf` (`w x h`, `0x00RRGGBB`).
pub fn render_gui(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, gui: &mut GuiState, m: &Model) {
    if w < 120 || h < 120 || buf.len() < w * h {
        return;
    }
    gui.clear_hits();
    let pal = gui.palette(&m.values);
    let (wi, hi) = (w as i32, h as i32);

    // Main canvas & sidebar surface.
    fill_solid(buf, w, h, 0, 0, wi, hi, pal.bg);
    fill_solid(buf, w, h, 0, HEADER_H, SIDEBAR_W, hi - HEADER_H, pal.panel);
    fill_solid(buf, w, h, SIDEBAR_W, HEADER_H, 1, hi - HEADER_H, pal.hairline);

    // Top frosted Candy Lavender toolbar with title, Undo, and Reset buttons.
    draw_candy_toolbar(buf, w, h, 0, 0, wi, HEADER_H, &pal);
    draw_text(buf, w, h, g, 14, 13, SIDEBAR_W - 20, b"Tatami Settings", pal.text);

    let title: &[u8] = match gui.view {
        SidebarView::Appearance => b"Appearance & Theme",
        SidebarView::WindowsTabs => b"Windows, Chrome & Tiling",
        SidebarView::Keybinds => b"Keybinds (Hyprland + KDE)",
        SidebarView::Profiles => b"Desktop Profiles (HyprMod)",
        SidebarView::Install => b"Install SexOS & Partition Manager",
        SidebarView::SystemPage(idx) => {
            let (pages, n) = model::pages();
            if idx < n {
                model::page_title(pages[idx]).as_bytes()
            } else {
                b"System"
            }
        }
    };
    draw_text(buf, w, h, g, SIDEBAR_W + 18, 13, wi - SIDEBAR_W - 190, title, pal.text);

    // Undo & Reset Candy pill buttons in header right.
    let reset_w = 72;
    let undo_w = 64;
    let reset_x = wi - reset_w - 12;
    let undo_x = reset_x - undo_w - 8;
    let btn_y = 8;
    let btn_h = 26;

    let undo_style = if gui.undo.is_some() { ButtonStyle::Normal } else { ButtonStyle::Disabled };
    let undo_state = if gui.undo.is_some() { gui.btn_state(UiAction::Undo) } else { ButtonState::Rest };
    draw_candy_button(buf, w, h, undo_x, btn_y, undo_w, btn_h, 13, undo_style, undo_state, &pal);
    draw_text(buf, w, h, g, undo_x + 16, btn_y + 5, undo_w - 16, b"Undo", button_text_color(undo_style, &pal));
    if gui.undo.is_some() {
        gui.push_hit(undo_x, btn_y, undo_w, btn_h, UiAction::Undo);
    }

    let reset_state = gui.btn_state(UiAction::ResetDefaults);
    draw_candy_button(buf, w, h, reset_x, btn_y, reset_w, btn_h, 13, ButtonStyle::Normal, reset_state, &pal);
    draw_text(buf, w, h, g, reset_x + 16, btn_y + 5, reset_w - 16, b"Reset", button_text_color(ButtonStyle::Normal, &pal));
    gui.push_hit(reset_x, btn_y, reset_w, btn_h, UiAction::ResetDefaults);

    // Sidebar navigation pills.
    let mut sy = HEADER_H + 10;
    draw_text(buf, w, h, g, 14, sy, SIDEBAR_W - 24, b"PERSONALIZE", pal.text_dim);
    sy += 20;

    let primary_items: [(&[u8], SidebarView); 5] = [
        (b"Appearance", SidebarView::Appearance),
        (b"Windows & Tabs", SidebarView::WindowsTabs),
        (b"Keybinds", SidebarView::Keybinds),
        (b"Profiles", SidebarView::Profiles),
        (b"Install SexOS", SidebarView::Install),
    ];
    for (label, view) in primary_items {
        let active = gui.view == view;
        let act = UiAction::SelectView(view);
        let st = gui.btn_state(act);
        let style = if active { ButtonStyle::Active } else { ButtonStyle::Subtle };
        draw_candy_button(buf, w, h, 10, sy, SIDEBAR_W - 20, 28, 10, style, st, &pal);
        if active {
            fill_rounded_rect(buf, w, h, 14, sy + 6, 3, 16, 2, pal.accent);
        }
        draw_text(buf, w, h, g, 24, sy + 6, SIDEBAR_W - 38, label, if active { pal.text } else { pal.text_dim });
        gui.push_hit(10, sy, SIDEBAR_W - 20, 28, act);
        sy += 32;
    }

    sy += 6;
    fill_solid(buf, w, h, 14, sy, SIDEBAR_W - 28, 1, pal.hairline);
    sy += 10;
    draw_text(buf, w, h, g, 14, sy, SIDEBAR_W - 24, b"SYSTEM", pal.text_dim);
    sy += 20;

    let (pages, page_count) = model::pages();
    for (idx, &sec) in pages[..page_count].iter().enumerate() {
        if sec == "appearance" {
            continue;
        }
        if sy + 26 > hi - 10 {
            break;
        }
        let view = SidebarView::SystemPage(idx);
        let active = gui.view == view;
        let act = UiAction::SelectView(view);
        let st = gui.btn_state(act);
        if active {
            draw_candy_selection_row(buf, w, h, 10, sy, SIDEBAR_W - 20, 24, 8, &pal);
        } else if st == ButtonState::Hover {
            draw_candy_button(buf, w, h, 10, sy, SIDEBAR_W - 20, 24, 8, ButtonStyle::Subtle, st, &pal);
        }
        let label = model::page_title(sec).as_bytes();
        draw_text(buf, w, h, g, 22, sy + 4, SIDEBAR_W - 34, label, if active { pal.text } else { pal.text_dim });
        gui.push_hit(10, sy, SIDEBAR_W - 20, 24, act);
        sy += 26;
    }

    // Right content pane.
    let cx = SIDEBAR_W + 18;
    let cw = (wi - SIDEBAR_W - 36).max(200);
    let cy = HEADER_H + 14;
    let ch = (hi - HEADER_H - 42).max(120);

    match gui.view {
        SidebarView::Appearance => render_appearance(buf, w, h, g, gui, m, &pal, cx, cy, cw, ch),
        SidebarView::WindowsTabs => render_windows_tabs(buf, w, h, g, gui, m, &pal, cx, cy, cw, ch),
        SidebarView::Keybinds => render_keybinds(buf, w, h, g, gui, &pal, cx, cy, cw, ch),
        SidebarView::Profiles => render_profiles(buf, w, h, g, gui, m, &pal, cx, cy, cw, ch),
        SidebarView::Install => render_install(buf, w, h, g, gui, &pal, cx, cy, cw, ch),
        SidebarView::SystemPage(_) => render_system_page(buf, w, h, g, gui, m, &pal, cx, cy, cw, ch),
    }

    // Bottom status bar.
    let status_y = hi - 26;
    fill_solid(buf, w, h, SIDEBAR_W + 1, status_y, wi - SIDEBAR_W - 1, 26, pal.panel);
    fill_solid(buf, w, h, SIDEBAR_W + 1, status_y, wi - SIDEBAR_W - 1, 1, pal.hairline);
    let status = if m.status_len > 0 {
        &m.status[..m.status_len]
    } else {
        b"Live preview & instant apply active. Click any control or use arrow keys."
    };
    draw_text(buf, w, h, g, SIDEBAR_W + 18, status_y + 5, wi - SIDEBAR_W - 32, status, pal.text_dim);
}

#[allow(clippy::too_many_arguments)]
fn render_live_preview(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    pw: i32,
) {
    let ph = 86;
    let r = CORNER_RADII[gui.corner_radius_idx.min(CORNER_RADII.len() - 1)].1 as i32;
    let frosted = m.values.value(id::REDUCE_TRANSPARENCY) == 0;
    draw_candy_panel(buf, w, h, x, y, pw, ph, r, pal);

    // Mini window titlebar inside preview.
    draw_candy_toolbar(buf, w, h, x + 2, y + 2, pw - 4, 24, pal);
    // Traffic-light candy dots.
    fill_rounded_rect(buf, w, h, x + 12, y + 9, 10, 10, 5, 0x00FF_5F57);
    fill_rounded_rect(buf, w, h, x + 26, y + 9, 10, 10, 5, 0x00FE_BC2E);
    fill_rounded_rect(buf, w, h, x + 40, y + 9, 10, 10, 5, 0x0028_C840);
    draw_text(buf, w, h, g, x + 60, y + 6, pw - 160, b"Live Candy Preview", pal.text);

    // Frosted badge on right of preview titlebar.
    let badge_label: &[u8] = if frosted { b"Frosted" } else { b"Opaque" };
    draw_candy_badge(buf, w, h, x + pw - 86, y + 5, 74, 18, pal.accent, pal);
    draw_text(buf, w, h, g, x + pw - 78, y + 6, 62, badge_label, pal.text_on_accent);

    // Sample controls inside preview body.
    draw_candy_button(buf, w, h, x + 14, y + 38, 118, 28, r.min(12), ButtonStyle::Primary, ButtonState::Rest, pal);
    draw_text(buf, w, h, g, x + 24, y + 44, 100, b"Apply Candy", pal.text_on_accent);

    draw_candy_button(buf, w, h, x + 142, y + 38, 116, 28, r.min(12), ButtonStyle::Normal, ButtonState::Rest, pal);
    draw_text(buf, w, h, g, x + 154, y + 44, 96, gui.tiling.label().as_bytes(), pal.text);

    draw_candy_toggle(buf, w, h, x + pw - 62, y + 41, 44, 22, frosted, ButtonState::Rest, pal);
}

#[allow(clippy::too_many_arguments)]
fn render_appearance(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _ch: i32,
) {
    render_live_preview(buf, w, h, g, gui, m, pal, x, y, cw);
    y += 98;

    // Theme mode card.
    draw_candy_panel(buf, w, h, x, y, cw, 64, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 28, b"Theme Mode", pal.text);
    let cur_theme = m.values.value(id::THEME);
    let modes: [(&[u8], i32); 3] = [(b"Light", 0), (b"Dark", 1), (b"High Contrast", 2)];
    let mut tx = x + 14;
    for (label, val) in modes {
        let bw = (label.len() as i32) * CELL_W as i32 + 28;
        let act = UiAction::SetTheme(val);
        draw_candy_tab(buf, w, h, tx, y + 30, bw, 26, cur_theme == val, gui.btn_state(act), pal);
        draw_text(buf, w, h, g, tx + 14, y + 35, bw - 16, label, if cur_theme == val { pal.text } else { pal.text_dim });
        gui.push_hit(tx, y + 30, bw, 26, act);
        tx += bw + 10;
    }
    y += 74;

    // Accent color picker swatches card.
    draw_candy_panel(buf, w, h, x, y, cw, 82, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 28, b"Accent Swatch (Candy Lavender Default)", pal.text);
    let cur_accent = m.values.value(id::ACCENT) & 0x00FF_FFFF;
    let sw_step = ((cw - 28) / CANDY_SWATCHES.len() as i32).max(58);
    for (i, (name, rgb)) in CANDY_SWATCHES.iter().enumerate() {
        let sx = x + 18 + i as i32 * sw_step;
        let sy = y + 32;
        let act = UiAction::SetAccent(*rgb);
        let selected = cur_accent == *rgb;
        let hovered = gui.hovered == Some(act);
        draw_candy_swatch(buf, w, h, sx, sy, 26, *rgb as u32, selected, hovered, pal);
        let short = if name.len() > 8 { &name.as_bytes()[..8] } else { name.as_bytes() };
        draw_text(buf, w, h, g, sx - 2, sy + 30, sw_step - 4, short, if selected { pal.text } else { pal.text_dim });
        gui.push_hit(sx - 4, sy - 4, 34, 44, act);
    }
    y += 92;

    // Frosted Glass & Animations toggles card.
    draw_candy_panel(buf, w, h, x, y, cw, 82, 10, pal);
    let frosted_on = m.values.value(id::REDUCE_TRANSPARENCY) == 0;
    draw_text(buf, w, h, g, x + 14, y + 14, cw - 90, b"Frosted Lavender Glass (off = Reduce Transparency)", pal.text);
    let fg_act = UiAction::ToggleFrostedGlass;
    draw_candy_toggle(buf, w, h, x + cw - 62, y + 11, 46, 22, frosted_on, gui.btn_state(fg_act), pal);
    gui.push_hit(x + cw - 66, y + 8, 54, 28, fg_act);

    fill_solid(buf, w, h, x + 14, y + 41, cw - 28, 1, pal.hairline);

    let anim_on = m.values.value(id::ANIMATIONS) != 0;
    draw_text(buf, w, h, g, x + 14, y + 52, cw - 90, b"Smooth Candy Animations & Transitions", pal.text);
    let an_act = UiAction::ToggleAnimations;
    draw_candy_toggle(buf, w, h, x + cw - 62, y + 49, 46, 22, anim_on, gui.btn_state(an_act), pal);
    gui.push_hit(x + cw - 66, y + 46, 54, 28, an_act);
}

#[allow(clippy::too_many_arguments)]
fn render_windows_tabs(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _ch: i32,
) {
    render_live_preview(buf, w, h, g, gui, m, pal, x, y, cw);
    y += 98;

    // Corner radius selector.
    draw_candy_panel(buf, w, h, x, y, cw, 64, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 28, b"Window & Button Corner Radius", pal.text);
    let mut rx = x + 14;
    for (idx, (label, _r)) in CORNER_RADII.iter().enumerate() {
        let bw = (label.len() as i32) * CELL_W as i32 + 24;
        let act = UiAction::SetCornerRadius(idx);
        draw_candy_tab(buf, w, h, rx, y + 30, bw, 26, gui.corner_radius_idx == idx, gui.btn_state(act), pal);
        draw_text(buf, w, h, g, rx + 12, y + 35, bw - 12, label.as_bytes(), if gui.corner_radius_idx == idx { pal.text } else { pal.text_dim });
        gui.push_hit(rx, y + 30, bw, 26, act);
        rx += bw + 10;
    }
    y += 74;

    // Tab bar chrome style.
    draw_candy_panel(buf, w, h, x, y, cw, 64, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 28, b"Tab Bar & Window Chrome Style", pal.text);
    let mut tx = x + 14;
    for style in [TabBarStyle::SilkPill, TabBarStyle::Classic] {
        let label = style.label().as_bytes();
        let bw = (label.len() as i32) * CELL_W as i32 + 28;
        let act = UiAction::SetTabBar(style);
        draw_candy_tab(buf, w, h, tx, y + 30, bw, 26, gui.tab_bar == style, gui.btn_state(act), pal);
        draw_text(buf, w, h, g, tx + 14, y + 35, bw - 14, label, if gui.tab_bar == style { pal.text } else { pal.text_dim });
        gui.push_hit(tx, y + 30, bw, 26, act);
        tx += bw + 10;
    }
    y += 74;

    // Tiling defaults (Hyprland / KDE / Hybrid).
    draw_candy_panel(buf, w, h, x, y, cw, 64, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 28, b"Window Tiling Default (Hyprland / KDE)", pal.text);
    let mut wx = x + 14;
    for tiling in [TilingStyle::HybridCandy, TilingStyle::HyprlandMaster, TilingStyle::KdeFloat] {
        let label = tiling.label().as_bytes();
        let bw = (label.len() as i32) * CELL_W as i32 + 22;
        let act = UiAction::SetTiling(tiling);
        draw_candy_tab(buf, w, h, wx, y + 30, bw, 26, gui.tiling == tiling, gui.btn_state(act), pal);
        draw_text(buf, w, h, g, wx + 11, y + 35, bw - 12, label, if gui.tiling == tiling { pal.text } else { pal.text_dim });
        gui.push_hit(wx, y + 30, bw, 26, act);
        wx += bw + 10;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_keybinds(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    ch: i32,
) {
    // Profile toggle bar.
    draw_candy_panel(buf, w, h, x, y, cw, 64, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 28, b"Hotkey Binding Profile", pal.text);
    let mut bx = x + 14;
    for prof in [KeybindProfile::HyprlandKde, KeybindProfile::HyprlandPure, KeybindProfile::KdeClassic] {
        let label = prof.label().as_bytes();
        let bw = (label.len() as i32) * CELL_W as i32 + 24;
        let act = UiAction::SetKeybinds(prof);
        draw_candy_tab(buf, w, h, bx, y + 30, bw, 26, gui.keybinds == prof, gui.btn_state(act), pal);
        draw_text(buf, w, h, g, bx + 12, y + 35, bw - 12, label, if gui.keybinds == prof { pal.text } else { pal.text_dim });
        gui.push_hit(bx, y + 30, bw, 26, act);
        bx += bw + 10;
    }
    y += 74;

    // Hotkey reference card.
    let card_h = (ch - 80).max(160);
    draw_candy_panel(buf, w, h, x, y, cw, card_h, 10, pal);
    draw_text(buf, w, h, g, x + 14, y + 10, cw - 28, b"Active Desktop Bindings Reference", pal.text);
    let mut ry = y + 34;
    for (keys, desc, origin) in HOTKEY_ROWS {
        if ry + 24 > y + card_h - 6 {
            break;
        }
        draw_candy_badge(buf, w, h, x + 14, ry, 180, 20, pal.control, pal);
        draw_text(buf, w, h, g, x + 22, ry + 2, 166, keys.as_bytes(), pal.accent);
        draw_text(buf, w, h, g, x + 206, ry + 2, (cw - 320).max(80), desc.as_bytes(), pal.text);
        draw_text(buf, w, h, g, x + cw - 116, ry + 2, 104, origin.as_bytes(), pal.text_dim);
        ry += 25;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_profiles(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _ch: i32,
) {
    render_live_preview(buf, w, h, g, gui, m, pal, x, y, cw);
    y += 98;

    draw_text(buf, w, h, g, x + 2, y, cw, b"One-click HyprMod Profiles (Instant Apply + Undoable)", pal.text_dim);
    y += 22;

    for (idx, p) in PROFILES.iter().enumerate() {
        let act = UiAction::ApplyProfile(idx);
        let active = gui.active_profile == Some(idx);
        let st = gui.btn_state(act);
        let style = if active { ButtonStyle::Active } else { ButtonStyle::Normal };
        draw_candy_button(buf, w, h, x, y, cw, 50, 10, style, st, pal);
        draw_candy_swatch(buf, w, h, x + 14, y + 12, 24, p.accent as u32, active, false, pal);
        draw_text(buf, w, h, g, x + 50, y + 8, cw - 150, p.name.as_bytes(), pal.text);
        draw_text(buf, w, h, g, x + 50, y + 26, cw - 150, p.subtitle.as_bytes(), pal.text_dim);
        let pill_txt: &[u8] = if active { b"Active" } else { b"Apply" };
        let pill_col = if active { pal.accent } else { pal.control };
        let pill_ink = if active { pal.text_on_accent } else { pal.text };
        draw_candy_badge(buf, w, h, x + cw - 82, y + 14, 68, 22, pill_col, pal);
        draw_text(buf, w, h, g, x + cw - 68, y + 17, 52, pill_txt, pill_ink);
        gui.push_hit(x, y, cw, 50, act);
        y += 58;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_system_page(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    ch: i32,
) {
    let (rows, count) = m.rows();
    let max_y = y + ch;
    for (idx, row) in rows[..count].iter().enumerate() {
        if y + 34 > max_y {
            break;
        }
        let k = &KEYS[row.key];
        let selected = m.row == idx;
        if selected {
            draw_candy_selection_row(buf, w, h, x, y, cw, 30, 8, pal);
        } else {
            draw_candy_panel(buf, w, h, x, y, cw, 30, 8, pal);
        }
        let label = match (k.kind, row.item) {
            (Kind::Set(names), Some(item)) => names.get(item as usize).copied().unwrap_or(k.label),
            _ => k.label,
        };
        draw_text(buf, w, h, g, x + 12, y + 7, cw - 170, label.as_bytes(), pal.text);
        gui.push_hit(x, y, cw - 120, 30, UiAction::SelectRow(idx));

        let val = m.values.value(k.id);
        match k.kind {
            Kind::Bool => {
                let act = UiAction::StepRow(idx, true);
                draw_candy_toggle(buf, w, h, x + cw - 58, y + 4, 44, 22, val != 0, gui.btn_state(act), pal);
                gui.push_hit(x + cw - 64, y + 2, 54, 26, act);
            }
            Kind::Color => {
                let act = UiAction::StepRow(idx, true);
                draw_candy_swatch(buf, w, h, x + cw - 40, y + 4, 22, (val as u32) & 0x00FF_FFFF, selected, false, pal);
                gui.push_hit(x + cw - 46, y + 2, 34, 26, act);
            }
            _ => {
                let dec = UiAction::StepRow(idx, false);
                let inc = UiAction::StepRow(idx, true);
                draw_candy_button(buf, w, h, x + cw - 116, y + 4, 26, 22, 6, ButtonStyle::Normal, gui.btn_state(dec), pal);
                draw_text(buf, w, h, g, x + cw - 107, y + 7, 16, b"<", pal.text);
                gui.push_hit(x + cw - 116, y + 4, 26, 22, dec);

                draw_candy_button(buf, w, h, x + cw - 38, y + 4, 26, 22, 6, ButtonStyle::Normal, gui.btn_state(inc), pal);
                draw_text(buf, w, h, g, x + cw - 29, y + 7, 16, b">", pal.text);
                gui.push_hit(x + cw - 38, y + 4, 26, 22, inc);

                let mut vbuf = [0u8; 16];
                let vlen = format_short_value(k.kind, val, row.item, &mut vbuf);
                draw_text(buf, w, h, g, x + cw - 86, y + 7, 46, &vbuf[..vlen], pal.accent);
            }
        }
        y += 34;
    }

    // Show extra info lines (Privacy grants, About, Updates, Storage) in a Candy card below.
    let info_slice: &[u8] = if m.info_len > 0 {
        &m.info[..m.info_len]
    } else if m.extra_len > 0 {
        &m.extra[..m.extra_len]
    } else {
        b""
    };
    if !info_slice.is_empty() && y + 40 < max_y {
        let card_h = (max_y - y).max(40);
        draw_candy_panel(buf, w, h, x, y, cw, card_h, 10, pal);
        let mut ly = y + 8;
        for line in info_slice.split(|&b| b == b'\n') {
            if ly + 18 > y + card_h - 4 {
                break;
            }
            if !line.is_empty() {
                draw_text(buf, w, h, g, x + 12, ly, cw - 24, line, pal.text_dim);
            }
            ly += 18;
        }
    }
}

fn format_short_value(kind: Kind, value: i32, item: Option<u8>, out: &mut [u8; 16]) -> usize {
    match kind {
        Kind::Choice(names) => {
            let s = names.get(value as usize).copied().unwrap_or("?").as_bytes();
            let n = s.len().min(out.len());
            out[..n].copy_from_slice(&s[..n]);
            n
        }
        Kind::Set(_) => {
            let on = item.is_some_and(|i| (value & (1 << i)) != 0);
            let s: &[u8] = if on { b"Yes" } else { b"No" };
            out[..s.len()].copy_from_slice(s);
            s.len()
        }
        _ => {
            let mut n = 0usize;
            let mut v = value;
            if v < 0 {
                out[n] = b'-';
                n += 1;
                v = v.saturating_neg();
            }
            let mut tmp = [0u8; 12];
            let mut k = 0;
            loop {
                tmp[k] = b'0' + (v % 10) as u8;
                k += 1;
                v /= 10;
                if v == 0 {
                    break;
                }
            }
            while k > 0 && n < out.len() {
                k -= 1;
                out[n] = tmp[k];
                n += 1;
            }
            n
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_install(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    ch: i32,
) {
    let max_y = y + ch;

    // 1. Visual horizontal color-coded GPT Partition Bar card.
    let bar_card_h = 84;
    draw_candy_panel(buf, w, h, x, y, cw, bar_card_h, 10, pal);
    let mut hdr = [0u8; 64];
    let mut hlen = 0usize;
    let prefix = b"GPT Disk Partition Map";
    hdr[..prefix.len()].copy_from_slice(prefix);
    hlen += prefix.len();
    if gui.install.total_sectors > 0 {
        let mut sz = [0u8; 16];
        let sl = format_size_short(gui.install.total_sectors, &mut sz);
        let sep = b" (";
        hdr[hlen..hlen + sep.len()].copy_from_slice(sep);
        hlen += sep.len();
        let take = sl.min(hdr.len() - hlen - 1);
        hdr[hlen..hlen + take].copy_from_slice(&sz[..take]);
        hlen += take;
        hdr[hlen] = b')';
        hlen += 1;
    }
    draw_text(buf, w, h, g, x + 14, y + 8, cw - 120, &hdr[..hlen], pal.text);

    // Refresh Disk button on top-right of GPT bar card.
    let ref_w = 78;
    let ref_act = UiAction::InstallRefresh;
    draw_candy_button(buf, w, h, x + cw - ref_w - 10, y + 5, ref_w, 22, 8, ButtonStyle::Subtle, gui.btn_state(ref_act), pal);
    draw_text(buf, w, h, g, x + cw - ref_w + 2, y + 8, ref_w - 12, b"Refresh", pal.text_dim);
    gui.push_hit(x + cw - ref_w - 10, y + 5, ref_w, 22, ref_act);

    // Proportional horizontal GPT Partition Bar.
    let bar_x = x + 14;
    let bar_y = y + 32;
    let bar_w = (cw - 28).max(80);
    let bar_h = 30;
    fill_rounded_rect(buf, w, h, bar_x, bar_y, bar_w, bar_h, 6, pal.control);

    let count = gui.install.part_count;
    if count > 0 {
        let sum_sectors: u64 = gui.install.parts[..count]
            .iter()
            .map(|p| p.sectors.max(1))
            .sum::<u64>()
            .max(1);
        let min_seg = 24.min(bar_w / (count as i32).max(1)).max(8);
        let mut sx = bar_x;
        let bar_right = bar_x + bar_w;
        for i in 0..count {
            let p = gui.install.parts[i];
            let rem_parts = (count - 1 - i) as i32;
            let raw_w = ((p.sectors as u128 * bar_w as u128) / sum_sectors as u128) as i32;
            let max_here = (bar_right - sx - rem_parts * min_seg).max(min_seg);
            let seg_w = if i + 1 == count {
                (bar_right - sx).max(1)
            } else {
                raw_w.clamp(min_seg, max_here)
            };
            let selected = gui.install.selected == Some(i);
            let col = p.kind.color();
            if selected {
                fill_rounded_rect(buf, w, h, sx, bar_y - 2, seg_w.saturating_sub(2).max(4), bar_h + 4, 5, pal.text);
                fill_rounded_rect(buf, w, h, sx + 2, bar_y, seg_w.saturating_sub(6).max(2), bar_h, 4, col);
            } else {
                fill_rounded_rect(buf, w, h, sx, bar_y, seg_w.saturating_sub(2).max(4), bar_h, 4, col);
            }
            if seg_w >= 44 {
                let lbl = if p.kind == DiskPartKind::FreeSpace { b"Free" as &[u8] } else { p.kind.label().as_bytes() };
                draw_text(buf, w, h, g, sx + 6, bar_y + 7, seg_w - 10, lbl, 0x0010_0C1C);
            }
            if p.replaceable {
                gui.push_hit(sx, bar_y, seg_w, bar_h, UiAction::SelectInstallPart(i));
            }
            sx += seg_w;
        }
    } else {
        let msg: &[u8] = if gui.install.loaded {
            b"No GPT partitions detected (click Refresh or use Spindle `install`)."
        } else {
            b"Scanning disk layout..."
        };
        draw_text(buf, w, h, g, bar_x + 10, bar_y + 7, bar_w - 20, msg, pal.text_dim);
    }

    // Color legend strip under bar.
    let legend: [(&[u8], u32); 5] = [
        (b"ESP", DiskPartKind::Esp.color()),
        (b"Windows", DiskPartKind::Windows.color()),
        (b"Linux", DiskPartKind::Linux.color()),
        (b"SexOS", DiskPartKind::SexOs.color()),
        (b"Free", DiskPartKind::FreeSpace.color()),
    ];
    let mut lx = bar_x;
    for (lbl, col) in legend {
        if lx + 72 > bar_x + bar_w {
            break;
        }
        fill_rounded_rect(buf, w, h, lx, y + 67, 10, 10, 3, col);
        draw_text(buf, w, h, g, lx + 14, y + 64, 60, lbl, pal.text_dim);
        lx += (lbl.len() as i32) * CELL_W as i32 + 28;
    }
    y += bar_card_h + 10;

    // 2. Clickable Partition Cards / Rows (reserve room for safety panel + action buttons below).
    let footer_h = 136;
    let list_limit = (max_y - footer_h).max(y + 34);
    for i in 0..count {
        if y + 32 > list_limit {
            break;
        }
        let p = gui.install.parts[i];
        let selected = gui.install.selected == Some(i);
        let act = UiAction::SelectInstallPart(i);
        let st = gui.btn_state(act);

        if selected {
            draw_candy_selection_row(buf, w, h, x, y, cw, 28, 8, pal);
        } else if p.replaceable && st == ButtonState::Hover {
            draw_candy_button(buf, w, h, x, y, cw, 28, 8, ButtonStyle::Subtle, st, pal);
        } else {
            draw_candy_panel(buf, w, h, x, y, cw, 28, 8, pal);
        }

        // Color dot.
        fill_rounded_rect(buf, w, h, x + 10, y + 8, 12, 12, 4, p.kind.color());

        // Partition number (#N or FREE).
        let mut num_lbl = [0u8; 8];
        let num_len = if p.kind == DiskPartKind::FreeSpace {
            num_lbl[..4].copy_from_slice(b"FREE");
            4
        } else {
            num_lbl[0] = b'#';
            1 + format_u64_ascii(p.part_num as u64, &mut num_lbl[1..])
        };
        draw_text(buf, w, h, g, x + 28, y + 6, 44, &num_lbl[..num_len], pal.text);

        // Name & type.
        let name_col = if p.replaceable { pal.text } else { pal.text_dim };
        draw_text(buf, w, h, g, x + 76, y + 6, (cw - 270).max(80), p.name_bytes(), name_col);
        draw_text(buf, w, h, g, x + cw - 190, y + 6, 64, p.kind.label().as_bytes(), pal.text_dim);

        // Size.
        let mut sz = [0u8; 16];
        let sz_len = format_size_short(p.sectors, &mut sz);
        draw_text(buf, w, h, g, x + cw - 126, y + 6, 68, &sz[..sz_len], name_col);

        // Status pill (LOCKED for ESP / MSR / < 1.5 GiB, Selected / Select for replaceable).
        if !p.replaceable {
            draw_candy_badge(buf, w, h, x + cw - 68, y + 4, 60, 20, 0x0047_3520, pal);
            draw_text(buf, w, h, g, x + cw - 62, y + 6, 52, b"LOCKED", 0x00FB_BF24);
        } else {
            let pill_col = if selected { pal.accent } else { pal.control };
            let pill_ink = if selected { pal.text_on_accent } else { pal.text };
            let pill_txt: &[u8] = if selected { b"Target" } else { b"Select" };
            draw_candy_badge(buf, w, h, x + cw - 68, y + 4, 60, 20, pill_col, pal);
            draw_text(buf, w, h, g, x + cw - 62, y + 6, 52, pill_txt, pill_ink);
            gui.push_hit(x, y, cw, 28, act);
        }
        y += 32;
    }

    // 3. Green / Amber Safety Summary Panel.
    let summary_y = y.max(list_limit - 2).min(max_y - 128);
    let summary_h = 84;
    let mut sum_buf = [0u8; 160];
    let (sum_len, is_amber) = gui.install.safety_summary(&mut sum_buf);
    let has_target = gui.install.selected_row().is_some();
    let badge_bg = if !has_target {
        pal.control
    } else if is_amber {
        0x00F5_9E0B // Amber caution (replacing single partition)
    } else {
        0x0034_D399 // Green safe (free space)
    };
    let badge_lbl: &[u8] = if !has_target {
        b"SAFETY"
    } else if is_amber {
        b"SINGLE-PART"
    } else {
        b"SAFE FREE"
    };

    draw_candy_panel(buf, w, h, x, summary_y, cw, summary_h, 10, pal);
    fill_rounded_rect(buf, w, h, x + 2, summary_y + 6, 4, summary_h - 12, 2, badge_bg);
    draw_candy_badge(buf, w, h, x + 14, summary_y + 8, 98, 20, badge_bg, pal);
    draw_text(buf, w, h, g, x + 20, summary_y + 10, 88, badge_lbl, 0x0010_0C1C);

    // Wrap safety summary across 2 lines if needed.
    let chars_per_line = ((cw - 130) / CELL_W as i32).max(20) as usize;
    let line1_end = sum_len.min(chars_per_line);
    draw_text(buf, w, h, g, x + 120, summary_y + 10, cw - 130, &sum_buf[..line1_end], pal.text);
    if line1_end < sum_len {
        draw_text(buf, w, h, g, x + 14, summary_y + 32, cw - 28, &sum_buf[line1_end..sum_len], pal.text);
    }

    // Show latest plan/install report from sexfiles (first 2 non-empty lines).
    let rep = gui.install.report_bytes();
    if !rep.is_empty() {
        let mut ry = summary_y + 52;
        for line in rep.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if ry + 16 > summary_y + summary_h {
                break;
            }
            draw_text(buf, w, h, g, x + 14, ry, cw - 28, line, pal.text_dim);
            ry += 16;
        }
    } else {
        draw_text(
            buf,
            w,
            h,
            g,
            x + 14,
            summary_y + 54,
            cw - 28,
            b"Click 'Plan Install' for a dry-run preview before writing anything.",
            pal.text_dim,
        );
    }

    // 4. Clean "Plan Install" and "Install Now" Candy Action Buttons.
    let btn_y = summary_y + summary_h + 10;
    let plan_w = 136;
    let go_w = 136;
    let plan_style = if has_target { ButtonStyle::Normal } else { ButtonStyle::Disabled };
    let plan_act = UiAction::InstallPlan;
    let plan_state = if has_target { gui.btn_state(plan_act) } else { ButtonState::Rest };
    draw_candy_button(buf, w, h, x, btn_y, plan_w, 30, 10, plan_style, plan_state, pal);
    draw_text(buf, w, h, g, x + 20, btn_y + 7, plan_w - 24, b"Plan Install", button_text_color(plan_style, pal));
    if has_target {
        gui.push_hit(x, btn_y, plan_w, 30, plan_act);
    }

    let go_x = x + plan_w + 12;
    let go_style = if has_target { ButtonStyle::Primary } else { ButtonStyle::Disabled };
    let go_act = UiAction::InstallExecute;
    let go_state = if has_target { gui.btn_state(go_act) } else { ButtonState::Rest };
    draw_candy_button(buf, w, h, go_x, btn_y, go_w, 30, 10, go_style, go_state, pal);
    draw_text(buf, w, h, g, go_x + 24, btn_y + 7, go_w - 24, b"Install Now", button_text_color(go_style, pal));
    if has_target {
        gui.push_hit(go_x, btn_y, go_w, 30, go_act);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_profiles_apply_undo_and_reset_cleanly() {
        let mut cfg = Config::new();
        let mut gui = GuiState::new();
        assert_eq!(gui.undo, None);

        // Apply Hyprland Compact profile -> saves undo snapshot.
        let pairs = gui.apply_profile(2, &cfg);
        assert!(gui.undo.is_some());
        assert_eq!(gui.active_profile, Some(2));
        assert_eq!(gui.tiling, TilingStyle::HyprlandMaster);
        assert_eq!(gui.keybinds, KeybindProfile::HyprlandPure);
        for (k, v) in pairs {
            let _ = cfg.set(k, v);
        }
        assert_eq!(cfg.value(id::ACCENT), 0x60A5FA);

        // Undo restores original settings.
        let restored = gui.take_undo().expect("undo snapshot present");
        assert!(gui.undo.is_none());
        for (k, v) in restored {
            let _ = cfg.set(k, v);
        }
        assert_eq!(gui.tiling, TilingStyle::HybridCandy);

        // Reset defaults applies Candy Lavender profile.
        let reset = gui.reset_defaults(&cfg);
        for (k, v) in reset {
            let _ = cfg.set(k, v);
        }
        assert_eq!(cfg.value(id::ACCENT), 0xA78BFA);
        assert_eq!(cfg.value(id::REDUCE_TRANSPARENCY), 0);
    }

    #[test]
    fn appearance_gui_renders_and_registers_interactive_hitboxes() {
        let cfg = Config::new();
        let m = Model::new(cfg);
        let mut gui = GuiState::new();
        let mut g = Glyphs::new();
        let mut buf = vec![0u32; 800 * 520];

        render_gui(&mut buf, 800, 520, &mut g, &mut gui, &m);
        assert!(gui.hit_count >= 12, "expected sidebar + theme + swatches + toggles hitboxes");

        // Verify accent swatch hitboxes exist for all 6 Candy swatches.
        for &(_name, rgb) in &CANDY_SWATCHES {
            let found = gui.hits[..gui.hit_count]
                .iter()
                .flatten()
                .any(|h| h.action == UiAction::SetAccent(rgb));
            assert!(found, "missing hitbox for accent {rgb:#x}");
        }

        // Verify Frosted Glass toggle hitbox exists.
        let has_glass = gui.hits[..gui.hit_count]
            .iter()
            .flatten()
            .any(|h| h.action == UiAction::ToggleFrostedGlass);
        assert!(has_glass);

        // Verify Install SexOS sidebar tab hitbox exists.
        let has_install_tab = gui.hits[..gui.hit_count]
            .iter()
            .flatten()
            .any(|h| h.action == UiAction::SelectView(SidebarView::Install));
        assert!(has_install_tab, "missing Install SexOS sidebar tab");
    }

    #[test]
    fn install_gui_locks_esp_msr_tiny_and_renders_partition_bar_and_buttons() {
        let cfg = Config::new();
        let m = Model::new(cfg);
        let mut gui = GuiState::new();
        gui.view = SidebarView::Install;

        // Simulated raw GPT output from sexfiles: ESP + MSR + Windows + Void + Garuda + Tiny + Free.
        let raw = concat!(
            "DISK 1000215216\n",
            "PART 1 esp 2048 532480 0 EFI system partition\n",
            "PART 2 msr 534528 32768 0 Microsoft reserved\n",
            "PART 3 win 567296 209715200 1 Windows\n",
            "PART 4 lin 210282496 104857600 1 Void Linux\n",
            "PART 5 lin 315140096 104857600 1 Garuda Linux\n",
            "PART 6 lin 419997696 131072 0 Tiny Boot\n",
            "FREE 420128768 50000000\n",
        );
        gui.install.parse_raw(raw.as_bytes());
        assert_eq!(gui.install.part_count, 7);

        // ESP (0), MSR (1), and Tiny (5) MUST be locked and unselectable.
        assert!(!gui.install.parts[0].replaceable);
        assert!(!gui.install.parts[1].replaceable);
        assert!(!gui.install.parts[5].replaceable);
        assert!(!gui.install.select_part(0), "ESP must never be selectable");
        assert!(!gui.install.select_part(1), "MSR must never be selectable");
        assert!(!gui.install.select_part(5), "Tiny (<1.5 GiB) must never be selectable");

        // Select Garuda Linux (index 4, Partition #5).
        assert!(gui.install.select_part(4));
        let mut summary = [0u8; 160];
        let (slen, is_amber) = gui.install.safety_summary(&mut summary);
        let summary_str = core::str::from_utf8(&summary[..slen]).unwrap();
        assert!(is_amber);
        assert!(
            summary_str.contains("Will replace ONLY Partition #5 (Garuda Linux, 50.0 GiB) with SexOS A/B + Store. All other partitions remain untouched."),
            "unexpected summary: {summary_str}"
        );

        // Render Install page and verify hitboxes.
        let mut g = Glyphs::new();
        let mut buf = vec![0u32; 900 * 600];
        render_gui(&mut buf, 900, 600, &mut g, &mut gui, &m);

        let has = |act: UiAction| gui.hits[..gui.hit_count].iter().flatten().any(|h| h.action == act);
        assert!(!has(UiAction::SelectInstallPart(0)), "ESP must not have selectable hitbox");
        assert!(!has(UiAction::SelectInstallPart(1)), "MSR must not have selectable hitbox");
        assert!(!has(UiAction::SelectInstallPart(5)), "Tiny must not have selectable hitbox");
        assert!(has(UiAction::SelectInstallPart(4)), "Garuda must be clickable");
        assert!(has(UiAction::InstallRefresh), "missing Refresh button hitbox");
        assert!(has(UiAction::InstallPlan), "missing Plan Install button hitbox");
        assert!(has(UiAction::InstallExecute), "missing Install Now button hitbox");
    }
}

