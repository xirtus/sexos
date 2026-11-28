// SPDX-License-Identifier: GPL-3.0-only
//! Shared macOS Noir [`RichTextInput`] widget for SexOS pixel-buffer applications (`#![no_std]`).
//!
//! Backs any text well (Tailor, Tether, Settings, Store, etc.) with `RichEditor` + `sex-spell` +
//! Geist Sans/Mono rendering:
//! - Click to place caret, click-and-drag selection, double-click word, triple-click line
//! - Universal `Super` (⌘) and `Ctrl` shortcuts (`A/C/X/V/Z`, `Shift+Z`/`Y`), `Alt` word jump/delete,
//!   `Super+Left/Right` line edges, `Super+Backspace` delete-to-line-start
//! - Red wavy underlines (`#F43F5E`) on misspelled prose (auto-disabled in Code/Password/Url/Terminal)
//! - Dim inline ghost suffix (`rgba(160, 170, 190, 0.45)`) accepted with `Tab` or `Right Arrow`
//! - Optional Space-bar autocorrect with indigo underline (`#6366F1`) and instant 1-`Backspace` revert
//! - Right-click (`button == 2`) & `Ctrl + Left-Click` floating macOS Noir Context Popup.

use alloc::string::String;
use alloc::vec::Vec;
use sex_config::Config;
use sex_font::gamma::blend;
use sex_font::raster::{scratch_len, FX};
use sex_font::{Font, Render, Synthetic};
use sex_pdx::app_key::Key as AppKey;
use sex_pdx::app_pointer::{Kind as PKind, Pointer, CONTROL, SHIFT};
pub use sex_spell::TextMode;
use sex_spell::WordSpan;
use sex_textedit::editor::{ContextAction, Key as EditorKey, Modifiers, RichEditor};
use sex_theme::candy_ui::{fill_rounded_rect, mix_rgb, CandyPalette};

use crate::{clipboard, text as key_text, Event, Window};

const SUPER_LEFT: u8 = 0x5B;
const SUPER_RIGHT: u8 = 0x5C;

/// Pixel bounding box matching SexOS window coordinates (`0x00RRGGBB`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFieldRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl TextFieldRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Colors for a macOS Noir `RichTextInput` and its floating context menu (`0x00RRGGBB`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextFieldPalette {
    pub bg: u32,
    pub border: u32,
    pub border_focus: u32,
    pub text: u32,
    pub placeholder: u32,
    pub selection: u32,
    pub caret: u32,
    pub spell_underline: u32,
    pub autocorrect_underline: u32,
    pub ghost_text: u32,
    pub menu_bg: u32,
    pub menu_border: u32,
    pub menu_hover: u32,
    pub menu_text: u32,
    pub menu_shortcut: u32,
    pub menu_disabled: u32,
    pub menu_divider: u32,
}

impl Default for TextFieldPalette {
    fn default() -> Self {
        Self::noir()
    }
}

impl TextFieldPalette {
    /// Sleek macOS Noir dark surface palette.
    pub const fn noir() -> Self {
        let bg = 0x0011_131B;
        Self {
            bg,
            border: 0x0026_2B3B,
            border_focus: 0x0060_A5FA,
            text: 0x00EC_EEF4,
            placeholder: 0x006E_778F,
            selection: 0x0025_4B7C,
            caret: 0x0060_A5FA,
            spell_underline: 0x00F4_3F5E,
            autocorrect_underline: 0x0063_66F1,
            ghost_text: mix_rgb(bg, 0x00A0_AABE, 115),
            menu_bg: 0x0014_161E,
            menu_border: 0x002D_3244,
            menu_hover: 0x003B_82F6,
            menu_text: 0x00EC_F0F8,
            menu_shortcut: 0x008E_96AA,
            menu_disabled: 0x0064_6C7E,
            menu_divider: 0x0025_2A3A,
        }
    }

    /// Harmonized Noir input well tuned to an active [`CandyPalette`].
    pub const fn from_candy(pal: &CandyPalette) -> Self {
        let mut p = Self::noir();
        p.bg = pal.well;
        p.border = pal.rim;
        p.border_focus = pal.accent;
        p.text = pal.text;
        p.placeholder = pal.text_dim;
        p.selection = pal.selection;
        p.caret = pal.accent;
        p.ghost_text = mix_rgb(pal.well, 0x00A0_AABE, 115);
        p
    }
}

/// Event status returned by keyboard and pointer handlers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFieldEvent {
    pub handled: bool,
    pub changed: bool,
    pub submitted: bool,
    pub focus_requested: bool,
}

/// Reusable Noir single-line or multi-line pixel-buffer text field.
pub struct RichTextInput {
    pub editor: RichEditor,
    pub placeholder: String,
    pub max_bytes: usize,
    pub font_px: i32,
    pub radius: i32,
    pub padding_x: i32,
    pub rect: TextFieldRect,
    pub focused: bool,
    pub super_down: u8,
    scroll_x: i32,
    last_click_ticks: u64,
    last_click_x: i32,
    last_click_y: i32,
    click_streak: u8,
    dragging: bool,
    menu_pos: (i32, i32),
    menu_rect: TextFieldRect,
    menu_item_rects: Vec<TextFieldRect>,
    menu_hover: Option<usize>,
    clipboard_known: bool,
    font_sans: Option<Font<'static>>,
    font_mono: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
}

impl RichTextInput {
    pub fn new(text: &str, mode: TextMode) -> Self {
        Self {
            editor: RichEditor::from_str(text, mode),
            placeholder: String::new(),
            max_bytes: 4096,
            font_px: 13,
            radius: 8,
            padding_x: 10,
            rect: TextFieldRect::default(),
            focused: false,
            super_down: 0,
            scroll_x: 0,
            last_click_ticks: 0,
            last_click_x: 0,
            last_click_y: 0,
            click_streak: 0,
            dragging: false,
            menu_pos: (0, 0),
            menu_rect: TextFieldRect::default(),
            menu_item_rects: Vec::new(),
            menu_hover: None,
            clipboard_known: true,
            font_sans: Font::new(sex_font::GEIST_SANS),
            font_mono: Font::new(sex_font::GEIST_MONO),
            acc: Vec::new(),
            glyph: Vec::new(),
        }
    }

    pub fn with_placeholder(mut self, placeholder: &str) -> Self {
        self.placeholder = String::from(placeholder);
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes.max(1);
        self
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        self.editor.autocorrect_enabled = cfg.flag(sex_config::id::AUTOCORRECT);
        self.editor.spellcheck_enabled = cfg.flag(sex_config::id::SPELLCHECK);
        self.editor.prediction_enabled = cfg.flag(sex_config::id::INLINE_PREDICT);
    }

    pub fn text(&self) -> &str {
        self.editor.text_str()
    }

    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
        self.scroll_x = 0;
    }

    pub fn clear(&mut self) {
        self.editor.set_text("");
        self.scroll_x = 0;
    }

    pub fn set_mode(&mut self, mode: TextMode) {
        self.editor.set_mode(mode);
    }

    pub fn has_context_menu(&self) -> bool {
        self.editor.context_menu.is_some()
    }

    pub fn dismiss_context_menu(&mut self) {
        self.editor.close_context_menu();
        self.menu_hover = None;
    }

    /// Updates internal `Super` modifier bits whenever `Key` / `KeyUp` events arrive.
    pub fn observe_event(&mut self, ev: &Event) {
        match ev {
            Event::Key(k) => match k.code {
                SUPER_LEFT => self.super_down |= 1,
                SUPER_RIGHT => self.super_down |= 2,
                _ => {}
            },
            Event::KeyUp(k) => match k.code {
                SUPER_LEFT => self.super_down &= !1,
                SUPER_RIGHT => self.super_down &= !2,
                _ => {}
            },
            _ => {}
        }
    }

    fn active_font(&self) -> Option<&Font<'static>> {
        if self.editor.mode == TextMode::Code || self.editor.mode == TextMode::Terminal {
            self.font_mono.as_ref().or(self.font_sans.as_ref())
        } else {
            self.font_sans.as_ref()
        }
    }

    fn display_text(&self) -> String {
        let raw = self.editor.text_str();
        if self.editor.mode == TextMode::Password {
            raw.chars().map(|_| '•').collect()
        } else {
            String::from(raw)
        }
    }

    /// Translates `window.next()` pointer events (click, drag, double/triple-click, right-click, `Ctrl+Click`).
    pub fn handle_pointer(&mut self, window: &mut Window, p: Pointer) -> TextFieldEvent {
        let px = p.x as i32;
        let py = p.y as i32;
        let mut ev = TextFieldEvent::default();

        // Active context menu intercepts all pointer interactions first.
        if self.editor.context_menu.is_some() {
            match p.kind {
                PKind::Move => {
                    self.menu_hover = self.hit_menu_item(px, py);
                    ev.handled = true;
                }
                PKind::Down { button: 0 } | PKind::Down { button: 2 } => {
                    if let Some(idx) = self.hit_menu_item(px, py) {
                        let item_opt = self
                            .editor
                            .context_menu
                            .as_ref()
                            .and_then(|m| m.items.get(idx))
                            .cloned();
                        if let Some(item) = item_opt {
                            if item.enabled {
                                let clip = if matches!(item.action, ContextAction::Paste) {
                                    clipboard::paste(window).ok().map(|s| s.as_bytes().to_vec())
                                } else {
                                    None
                                };
                                let outcome = self.editor.execute_context_action(item.action, clip.as_deref());
                                self.sync_clipboard_out(window, &outcome.copied, &outcome.cut);
                                ev.changed = outcome.text_changed;
                            } else {
                                self.dismiss_context_menu();
                            }
                        }
                    } else {
                        self.dismiss_context_menu();
                    }
                    self.menu_hover = None;
                    ev.handled = true;
                }
                PKind::Up { .. } | PKind::Leave => {
                    ev.handled = true;
                }
                _ => {}
            }
            return ev;
        }

        match p.kind {
            PKind::Down { button: 0 } => {
                if !self.rect.contains(px, py) {
                    self.dragging = false;
                    return ev;
                }
                self.focused = true;
                ev.focus_requested = true;
                ev.handled = true;

                let now = sex_pdx::get_ticks();
                let dx = (px - self.last_click_x).abs();
                let dy = (py - self.last_click_y).abs();
                if now.saturating_sub(self.last_click_ticks) <= 420 && dx <= 6 && dy <= 6 {
                    self.click_streak = self.click_streak.saturating_add(1).min(3);
                } else {
                    self.click_streak = 1;
                }
                self.last_click_ticks = now;
                self.last_click_x = px;
                self.last_click_y = py;

                let byte_idx = self.byte_at_point(px, py);
                let mods = Modifiers {
                    shift: p.mods & SHIFT != 0,
                    ctrl: p.mods & CONTROL != 0,
                    alt: p.mods & sex_pdx::app_pointer::ALT != 0,
                    super_cmd: self.super_down != 0,
                };
                if mods.ctrl {
                    self.menu_pos = (px, py);
                    self.editor.open_context_menu_at(byte_idx, self.clipboard_known);
                    self.menu_hover = None;
                } else {
                    self.editor
                        .on_primary_click(byte_idx, self.click_streak, mods, self.clipboard_known);
                    self.dragging = true;
                }
            }
            PKind::Down { button: 2 } => {
                if !self.rect.contains(px, py) {
                    return ev;
                }
                self.focused = true;
                ev.focus_requested = true;
                ev.handled = true;
                self.dragging = false;
                let byte_idx = self.byte_at_point(px, py);
                self.menu_pos = (px, py);
                self.editor.on_secondary_click(byte_idx, self.clipboard_known);
                self.menu_hover = None;
            }
            PKind::Move => {
                if self.dragging {
                    let byte_idx = self.byte_at_point(px, py);
                    self.editor.on_primary_drag(byte_idx);
                    ev.handled = true;
                }
            }
            PKind::Up { button: 0 } => {
                if self.dragging {
                    self.dragging = false;
                    ev.handled = true;
                }
            }
            _ => {}
        }
        ev
    }

    /// Processes a hardware `AppKey` event using `Window` clipboard grants for Cut/Copy/Paste.
    pub fn handle_key(&mut self, window: &mut Window, key: AppKey) -> TextFieldEvent {
        match key.code {
            SUPER_LEFT => {
                self.super_down |= 1;
                return TextFieldEvent { handled: true, ..Default::default() };
            }
            SUPER_RIGHT => {
                self.super_down |= 2;
                return TextFieldEvent { handled: true, ..Default::default() };
            }
            _ => {}
        }

        let mods = Modifiers {
            shift: key.shift,
            ctrl: key.control,
            alt: key.alt,
            super_cmd: self.super_down != 0,
        };
        let Some(ed_key) = translate_key(key) else {
            return TextFieldEvent::default();
        };

        // Read Shell clipboard only when a Paste chord arrives so normal typing touches no IPC.
        let wants_paste = mods.primary_cmd() && !mods.alt && matches!(ed_key, EditorKey::Letter(b'v') | EditorKey::Char('v') | EditorKey::Char('V'));
        let clip_owned = if wants_paste {
            clipboard::paste(window).ok().map(|s| s.as_bytes().to_vec())
        } else {
            None
        };

        if matches!(ed_key, EditorKey::Char(_)) && self.editor.doc.len() >= self.max_bytes && self.editor.doc.selection().is_none() {
            return TextFieldEvent { handled: true, ..Default::default() };
        }

        let outcome = self.editor.handle_key(ed_key, mods, clip_owned.as_deref());
        self.sync_clipboard_out(window, &outcome.copied, &outcome.cut);
        if self.editor.doc.len() > self.max_bytes {
            self.editor.doc.undo();
            self.editor.refresh_analysis();
        }
        TextFieldEvent {
            handled: outcome.handled,
            changed: outcome.text_changed,
            submitted: outcome.submitted,
            focus_requested: false,
        }
    }

    fn sync_clipboard_out(&mut self, window: &mut Window, copied: &Option<Vec<u8>>, cut: &Option<Vec<u8>>) {
        let bytes = copied.as_deref().or(cut.as_deref());
        if let Some(data) = bytes {
            if let Ok(s) = core::str::from_utf8(data) {
                if !s.is_empty() {
                    let _ = clipboard::copy(window, s);
                    self.clipboard_known = true;
                }
            }
        }
    }

    /// Renders the input well, selection highlight, red/indigo underlines, ghost completion, and caret.
    pub fn render(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        rect: TextFieldRect,
        focused: bool,
        pal: &TextFieldPalette,
    ) {
        self.rect = rect;
        self.focused = focused;

        fill_rounded_rect(buf, bw, bh, rect.x, rect.y, rect.w, rect.h, self.radius, pal.bg);
        let rim = if focused { pal.border_focus } else { pal.border };
        draw_rounded_outline(buf, bw, bh, rect, self.radius, rim);

        let text_x = rect.x + self.padding_x;
        let max_inner_w = (rect.w - self.padding_x * 2).max(8);
        let clip_x0 = text_x;
        let clip_x1 = text_x + max_inner_w;

        let disp = self.display_text();
        let line_h = self.line_height_px();
        let text_y = rect.y + ((rect.h - line_h) / 2).max(2);

        if disp.is_empty() {
            self.scroll_x = 0;
            if !self.placeholder.is_empty() {
                let ph = self.placeholder.clone();
                self.draw_text_span(buf, bw, bh, text_x, text_y, clip_x0, clip_x1, &ph, pal.placeholder);
            }
            if focused {
                self.draw_caret(buf, bw, bh, text_x, text_y, line_h, pal.caret);
            }
            return;
        }

        // Single-line horizontal scroll keeping caret visible.
        let cursor_byte = if self.editor.mode == TextMode::Password {
            disp.char_indices()
                .nth(self.editor.text_str()[..self.editor.doc.cursor.min(self.editor.doc.len())].chars().count())
                .map_or(disp.len(), |(i, _)| i)
        } else {
            self.editor.doc.cursor.min(disp.len())
        };
        let caret_rel = self.measure_prefix_px(&disp, cursor_byte);
        if !self.editor.mode.is_multiline() {
            if caret_rel - self.scroll_x > max_inner_w - 6 {
                self.scroll_x = caret_rel - (max_inner_w - 6);
            }
            if caret_rel - self.scroll_x < 0 {
                self.scroll_x = caret_rel;
            }
            self.scroll_x = self.scroll_x.max(0);
        } else {
            self.scroll_x = 0;
        }

        let draw_x = text_x - self.scroll_x;

        // 1. Selection highlight
        if let Some((a, b)) = self.editor.doc.selection() {
            let (da, db) = if self.editor.mode == TextMode::Password {
                let ca = self.editor.text_str()[..a].chars().count();
                let cb = self.editor.text_str()[..b].chars().count();
                let ba = disp.char_indices().nth(ca).map_or(disp.len(), |(i, _)| i);
                let bb = disp.char_indices().nth(cb).map_or(disp.len(), |(i, _)| i);
                (ba, bb)
            } else {
                (a.min(disp.len()), b.min(disp.len()))
            };
            let x0 = (draw_x + self.measure_prefix_px(&disp, da)).max(clip_x0);
            let x1 = (draw_x + self.measure_prefix_px(&disp, db)).min(clip_x1);
            if x1 > x0 {
                fill_rect_clipped(buf, bw, bh, x0, text_y - 1, x1 - x0, line_h + 2, pal.selection);
            }
        }

        // 2. Crisp text glyphs
        self.draw_text_span(buf, bw, bh, draw_x, text_y, clip_x0, clip_x1, &disp, pal.text);

        // 3. Inline dim ghost-text suffix right after caret
        if focused {
            if let Some(ghost) = self.editor.ghost_suffix().map(String::from) {
                let gx = draw_x + caret_rel;
                if gx < clip_x1 - 4 {
                    self.draw_text_span(buf, bw, bh, gx, text_y, clip_x0, clip_x1, &ghost, pal.ghost_text);
                }
            }
        }

        // 4. Red wavy underlines on misspelled words (prose modes only)
        if self.editor.spellcheck_active() {
            let spans: Vec<WordSpan> = self.editor.misspelled_spans().to_vec();
            let uy = text_y + line_h - 1;
            for sp in spans {
                if sp.end <= disp.len() && sp.start < sp.end {
                    let x0 = (draw_x + self.measure_prefix_px(&disp, sp.start)).max(clip_x0);
                    let x1 = (draw_x + self.measure_prefix_px(&disp, sp.end)).min(clip_x1);
                    if x1 > x0 + 2 {
                        draw_wavy_underline(buf, bw, bh, x0, x1, uy, pal.spell_underline);
                    }
                }
            }
        }

        // 5. Blue/indigo underline beneath recent Space-bar autocorrect awaiting possible revert
        if let Some((a, b)) = self.editor.autocorrect_underline() {
            if b <= disp.len() && a < b {
                let x0 = (draw_x + self.measure_prefix_px(&disp, a)).max(clip_x0);
                let x1 = (draw_x + self.measure_prefix_px(&disp, b)).min(clip_x1);
                if x1 > x0 + 2 {
                    let uy = text_y + line_h - 1;
                    fill_rect_clipped(buf, bw, bh, x0, uy, x1 - x0, 2, pal.autocorrect_underline);
                }
            }
        }

        // 6. Focused vertical caret
        if focused {
            let cx = draw_x + caret_rel;
            if cx >= clip_x0 - 1 && cx <= clip_x1 {
                self.draw_caret(buf, bw, bh, cx, text_y, line_h, pal.caret);
            }
        }
    }

    /// Draws the floating macOS Noir Context Popup above window pixels (`button == 2` / `Ctrl+Click`).
    /// Call this LAST in window rendering so the menu sits cleanly above all panels & controls.
    pub fn render_context_menu(&mut self, buf: &mut [u32], bw: usize, bh: usize, pal: &TextFieldPalette) {
        let Some(menu) = self.editor.context_menu.clone() else {
            self.menu_item_rects.clear();
            self.menu_rect = TextFieldRect::default();
            return;
        };

        let menu_w = 216i32;
        let row_h = 25i32;
        let div_h = 7i32;
        let pad = 6i32;
        let has_div = menu.divider_after.is_some();
        let menu_h = pad * 2 + (menu.items.len() as i32) * row_h + if has_div { div_h } else { 0 };

        let mx = self.menu_pos.0.min((bw as i32 - menu_w - 6).max(4)).max(4);
        let my = self.menu_pos.1.min((bh as i32 - menu_h - 6).max(4)).max(4);
        let mrect = TextFieldRect::new(mx, my, menu_w, menu_h);
        self.menu_rect = mrect;

        // Subtle dark drop shadow + translucent Noir glass body + crisp 1px rim
        fill_rounded_rect_alpha(buf, bw, bh, mx + 2, my + 4, menu_w, menu_h, 10, 0x0004_0508, 120);
        fill_rounded_rect_alpha(buf, bw, bh, mx, my, menu_w, menu_h, 10, pal.menu_bg, 245);
        draw_rounded_outline(buf, bw, bh, mrect, 10, pal.menu_border);

        self.menu_item_rects.clear();
        let mut cy = my + pad;
        for (idx, item) in menu.items.iter().enumerate() {
            let rrect = TextFieldRect::new(mx + 4, cy, menu_w - 8, row_h);
            self.menu_item_rects.push(rrect);

            let hovered = item.enabled && self.menu_hover == Some(idx);
            if hovered {
                fill_rounded_rect(buf, bw, bh, rrect.x, rrect.y, rrect.w, rrect.h, 6, pal.menu_hover);
            }
            let fg = if !item.enabled {
                pal.menu_disabled
            } else if hovered {
                0x00FF_FFFF
            } else {
                pal.menu_text
            };
            self.draw_text_span(
                buf,
                bw,
                bh,
                rrect.x + 10,
                rrect.y + 4,
                rrect.x + 6,
                rrect.x + rrect.w - 42,
                &item.label,
                fg,
            );
            if !item.shortcut.is_empty() {
                let sc_w = self.measure_prefix_px(item.shortcut, item.shortcut.len());
                let sc_col = if hovered { 0x00E6_EEFF } else { pal.menu_shortcut };
                self.draw_text_span(
                    buf,
                    bw,
                    bh,
                    rrect.x + rrect.w - 10 - sc_w,
                    rrect.y + 4,
                    rrect.x + 10,
                    rrect.x + rrect.w - 6,
                    item.shortcut,
                    sc_col,
                );
            }
            cy += row_h;

            if menu.divider_after == Some(idx) {
                fill_rect_clipped(buf, bw, bh, mx + 10, cy + 3, menu_w - 20, 1, pal.menu_divider);
                cy += div_h;
            }
        }
    }

    fn hit_menu_item(&self, px: i32, py: i32) -> Option<usize> {
        self.menu_item_rects.iter().position(|r| r.contains(px, py))
    }

    fn byte_at_point(&self, px: i32, _py: i32) -> usize {
        let raw = self.editor.text_str();
        if raw.is_empty() {
            return 0;
        }
        let target_x = px - (self.rect.x + self.padding_x) + self.scroll_x;
        if target_x <= 0 {
            return 0;
        }
        let disp = self.display_text();
        let Some(font) = self.active_font() else {
            return raw.len();
        };
        let size = (self.font_px.max(10) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        let mut disp_positions: Vec<(usize, i32)> = Vec::new();
        for (bi, ch) in disp.char_indices() {
            let cx = pen / FX;
            disp_positions.push((bi, cx));
            if let Some(id) = font.glyph(ch as u32) {
                if let Some(p) = prev {
                    pen += font.kerning(p, id, size);
                }
                pen += font.advance(id, size);
                prev = Some(id);
            } else {
                pen += (self.font_px / 2) * FX;
                prev = None;
            }
        }
        let end_x = pen / FX;
        disp_positions.push((disp.len(), end_x));

        let mut best_char_idx = disp_positions.len() - 1;
        for w in 0..disp_positions.len() - 1 {
            let mid = (disp_positions[w].1 + disp_positions[w + 1].1) / 2;
            if target_x <= mid {
                best_char_idx = w;
                break;
            }
        }
        if self.editor.mode == TextMode::Password {
            raw.char_indices().nth(best_char_idx).map_or(raw.len(), |(i, _)| i)
        } else {
            disp_positions[best_char_idx].0.min(raw.len())
        }
    }

    fn line_height_px(&self) -> i32 {
        let px_size = self.font_px.max(10);
        let Some(font) = self.active_font() else {
            return px_size + 4;
        };
        let size = (px_size as u32) * 64;
        ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size)
    }

    fn measure_prefix_px(&self, text: &str, byte_end: usize) -> i32 {
        let Some(font) = self.active_font() else { return 0 };
        let size = (self.font_px.max(10) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        for ch in text[..byte_end.min(text.len())].chars() {
            let Some(id) = font.glyph(ch as u32) else {
                pen += (self.font_px / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            pen += font.advance(id, size);
            prev = Some(id);
        }
        pen / FX
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_span(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        clip_x0: i32,
        clip_x1: i32,
        text: &str,
        color: u32,
    ) {
        let font_data = if self.editor.mode == TextMode::Code || self.editor.mode == TextMode::Terminal {
            sex_font::GEIST_MONO
        } else {
            sex_font::GEIST_SANS
        };
        let Some(font) = Font::new(font_data) else { return };
        let px_size = self.font_px.max(10) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let line_h = ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let cr = ((color >> 16) & 0xFF) as u32;
        let cg = ((color >> 8) & 0xFF) as u32;
        let cb = (color & 0xFF) as u32;
        let fg = (cr << 16) | (cg << 8) | cb;
        let render = Render::ui_for(fg);
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;

        for ch in text.chars() {
            if pen / FX >= clip_x1 {
                break;
            }
            let Some(id) = font.glyph(ch as u32) else {
                pen += (self.font_px / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            let adv = font.advance(id, size);
            let gx0 = (pen / FX) - pad as i32;
            let gw = ((adv / FX).max(0) as usize + 2 * pad + 2).min(64);
            if gx0 + gw as i32 >= clip_x0 && gw > 0 && line_h > 0 && line_h <= 64 {
                let slen = scratch_len(gw, line_h);
                if self.acc.len() < slen {
                    self.acc.resize(slen, 0);
                }
                if self.glyph.len() < gw * line_h {
                    self.glyph.resize(gw * line_h, 0);
                }
                self.glyph[..gw * line_h].fill(0);
                let sub = pen - (pen / FX) * FX + pad as i32 * FX;
                font.draw_rendered(
                    id,
                    size,
                    sub,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut self.glyph[..gw * line_h],
                    Synthetic::NONE,
                    render,
                );
                for row in 0..line_h {
                    let py = y + row as i32;
                    if py < 0 || py >= bh as i32 {
                        continue;
                    }
                    let row_off = py as usize * bw;
                    for col in 0..gw {
                        let px = gx0 + col as i32;
                        if px < clip_x0 || px >= clip_x1 || px < 0 || px >= bw as i32 {
                            continue;
                        }
                        let a = self.glyph[row * gw + col];
                        if a == 0 {
                            continue;
                        }
                        let idx = row_off + px as usize;
                        if idx < buf.len() {
                            buf[idx] = blend(fg, buf[idx], a);
                        }
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
    }

    fn draw_caret(&self, buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, line_h: i32, color: u32) {
        fill_rect_clipped(buf, bw, bh, x, y, 2, line_h.max(12), color);
    }
}

fn translate_key(key: AppKey) -> Option<EditorKey> {
    if let Some(l) = letter_scancode(key.code) {
        if key.control || key.alt {
            return Some(EditorKey::Letter(l));
        }
    }
    match key.code {
        0x01 => Some(EditorKey::Escape),
        0x0E => Some(EditorKey::Backspace),
        0x0F => Some(EditorKey::Tab),
        0x1C => Some(EditorKey::Enter),
        0x4B | 0x69 => Some(EditorKey::Left),
        0x4D | 0x6A => Some(EditorKey::Right),
        0x48 | 0x67 => Some(EditorKey::Up),
        0x50 | 0x6C => Some(EditorKey::Down),
        0x47 | 0x66 => Some(EditorKey::Home),
        0x4F | 0x6B => Some(EditorKey::End),
        0x49 | 0x68 => Some(EditorKey::PageUp),
        0x51 | 0x6D => Some(EditorKey::PageDown),
        0x53 | 0x6F => Some(EditorKey::Delete),
        _ => {
            if let Some(ch) = key_text(key) {
                Some(EditorKey::Char(ch))
            } else {
                letter_scancode(key.code).map(EditorKey::Letter)
            }
        }
    }
}

fn letter_scancode(code: u8) -> Option<u8> {
    Some(match code {
        0x1E => b'a', 0x30 => b'b', 0x2E => b'c', 0x20 => b'd', 0x12 => b'e', 0x21 => b'f', 0x22 => b'g', 0x23 => b'h',
        0x17 => b'i', 0x24 => b'j', 0x25 => b'k', 0x26 => b'l', 0x32 => b'm', 0x31 => b'n', 0x18 => b'o', 0x19 => b'p',
        0x10 => b'q', 0x13 => b'r', 0x1F => b's', 0x14 => b't', 0x16 => b'u', 0x2F => b'v', 0x11 => b'w', 0x2D => b'x',
        0x15 => b'y', 0x2C => b'z',
        _ => return None,
    })
}

fn fill_rect_clipped(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(bw as i32);
    let y1 = (y + h).min(bh as i32);
    if x1 <= x0 || y1 <= y0 || buf.len() < bw.saturating_mul(bh) {
        return;
    }
    let col = color & 0x00FF_FFFF;
    for py in y0..y1 {
        let row = py as usize * bw;
        for px in x0..x1 {
            buf[row + px as usize] = col;
        }
    }
}

fn fill_rounded_rect_alpha(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32, alpha: u8) {
    if alpha >= 250 {
        fill_rounded_rect(buf, bw, bh, x, y, w, h, radius, color);
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(bw as i32);
    let y1 = (y + h).min(bh as i32);
    if x1 <= x0 || y1 <= y0 || buf.len() < bw.saturating_mul(bh) {
        return;
    }
    let r = radius.clamp(0, w.min(h) / 2);
    let col = color & 0x00FF_FFFF;
    for py in y0..y1 {
        let row = py as usize * bw;
        for px in x0..x1 {
            let inside = point_in_rounded_rect(px - x, py - y, w, h, r);
            if inside {
                let idx = row + px as usize;
                buf[idx] = mix_rgb(buf[idx], col, alpha);
            }
        }
    }
}

fn draw_rounded_outline(buf: &mut [u32], bw: usize, bh: usize, rect: TextFieldRect, radius: i32, color: u32) {
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.w).min(bw as i32);
    let y1 = (rect.y + rect.h).min(bh as i32);
    if x1 <= x0 || y1 <= y0 || buf.len() < bw.saturating_mul(bh) {
        return;
    }
    let r = radius.clamp(0, rect.w.min(rect.h) / 2);
    let col = color & 0x00FF_FFFF;
    for py in y0..y1 {
        let ry = py - rect.y;
        let row = py as usize * bw;
        for px in x0..x1 {
            let rx = px - rect.x;
            if point_in_rounded_rect(rx, ry, rect.w, rect.h, r)
                && !point_in_rounded_rect(rx - 1, ry - 1, rect.w - 2, rect.h - 2, (r - 1).max(0))
            {
                buf[row + px as usize] = col;
            }
        }
    }
}

#[inline]
fn point_in_rounded_rect(px: i32, py: i32, w: i32, h: i32, r: i32) -> bool {
    if px < 0 || py < 0 || px >= w || py >= h {
        return false;
    }
    if r <= 0 {
        return true;
    }
    let cx = if px < r {
        r - px - 1
    } else if px >= w - r {
        px - (w - r)
    } else {
        return true;
    };
    let cy = if py < r {
        r - py - 1
    } else if py >= h - r {
        py - (h - r)
    } else {
        return true;
    };
    cx * cx + cy * cy <= r * r
}

fn draw_wavy_underline(buf: &mut [u32], bw: usize, bh: usize, x0: i32, x1: i32, y_base: i32, color: u32) {
    let col = color & 0x00FF_FFFF;
    for px in x0.max(0)..x1.min(bw as i32) {
        let phase = (px - x0) % 4;
        let dy = match phase {
            0 => 0,
            1 => -1,
            2 => 0,
            _ => 1,
        };
        let py = y_base + dy;
        if py >= 0 && py < bh as i32 {
            let idx = py as usize * bw + px as usize;
            if idx < buf.len() {
                buf[idx] = col;
            }
        }
        // Soft anti-aliased thicken vertically for readable red wave
        let py2 = py + 1;
        if py2 >= 0 && py2 < bh as i32 {
            let idx = py2 as usize * bw + px as usize;
            if idx < buf.len() {
                buf[idx] = mix_rgb(buf[idx], col, 140);
            }
        }
    }
}
