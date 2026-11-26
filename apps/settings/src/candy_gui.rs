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

/// Active top-level Candy sidebar tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarView {
    Appearance,
    WindowsTabs,
    Keybinds,
    Profiles,
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

    let primary_items: [(&[u8], SidebarView); 4] = [
        (b"Appearance", SidebarView::Appearance),
        (b"Windows & Tabs", SidebarView::WindowsTabs),
        (b"Keybinds", SidebarView::Keybinds),
        (b"Profiles", SidebarView::Profiles),
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
    }
}
