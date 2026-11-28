// SPDX-License-Identifier: GPL-3.0-only
//! macOS Noir [`NoirTextEdit`] wrapper around `egui::TextEdit`:
//! - Red wavy underlines (`#F43F5E`) under misspelled words in prose fields (auto-disabled in Code/Password/Url/Terminal).
//! - Dim inline ghost-text completions (`rgba(160, 170, 190, 0.45)`) accepted with `Tab` or `Right Arrow`.
//! - Optional Space-bar autocorrect with indigo underline (`#6366F1`) and instant 1-`Backspace` revert.
//! - Right-click (`button == 2`) & `Ctrl + Left-Click` macOS Noir Context Popup (top 1–3 fixes, Ignore/Learn, Cut/Copy/Paste/Select All).

use egui::epaint::text::{ByteIndex, CharIndex};
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditOutput;
use egui::{
    Align2, Area, AsIdSalt, Color32, Context, CornerRadius, Event, EventFilter, FontId,
    FontSelection, Frame, Id, IdSalt, Key, Margin, Order, PointerButton, Pos2, Response, Sense,
    Stroke, TextBuffer, TextEdit as EguiTextEdit, TextStyle, Ui, Vec2, Widget, WidgetText,
};
use sex_config::Config;
pub use sex_spell::TextMode;
use sex_spell::{
    active_word_prefix_at, word_span_at, AutocorrectRevert, SpellEngine, WordBuf, WordSpan,
};

const SPELL_UNDERLINE_COLOR: Color32 = Color32::from_rgb(0xF4, 0x3F, 0x5E);
const AUTOCORRECT_UNDERLINE_COLOR: Color32 = Color32::from_rgb(0x63, 0x66, 0xF1);
const MENU_HOVER: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
const MENU_TEXT: Color32 = Color32::from_rgb(236, 240, 248);
const MENU_SHORTCUT: Color32 = Color32::from_rgb(142, 150, 170);
const MENU_DISABLED: Color32 = Color32::from_rgb(100, 108, 126);

#[inline]
fn ghost_text_color() -> Color32 {
    Color32::from_rgba_unmultiplied(160, 170, 190, 115)
}
#[inline]
fn menu_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(20, 22, 30, 246)
}
#[inline]
fn menu_border() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 32)
}
#[inline]
fn menu_divider() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 22)
}

/// Shared `SpellEngine` id in `egui::Context` temporary storage so Learn/Ignore persists across fields.
fn shared_spell_id() -> Id {
    Id::NULL.with("sex_egui::shared_spell_engine")
}

#[derive(Clone, Debug)]
struct SharedSpell(SpellEngine);

impl Default for SharedSpell {
    fn default() -> Self {
        Self(SpellEngine::new())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MenuAction {
    Replace { byte_start: usize, byte_end: usize, replacement: String },
    Ignore(String),
    Learn(String),
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[derive(Clone, Debug)]
struct MenuItem {
    label: String,
    shortcut: &'static str,
    enabled: bool,
    prominent: bool,
    action: MenuAction,
}

#[derive(Clone, Debug)]
struct NoirContextMenu {
    pos: Pos2,
    target_chars: Option<(usize, usize)>,
    items: Vec<MenuItem>,
    divider_after: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct NoirFieldState {
    /// Active inline ghost completion stored as `(cursor_char_idx, suffix)`.
    ghost: Option<(usize, WordBuf)>,
    autocorrect: AutocorrectRevert,
    suppress_autocorrect_at: Option<usize>,
    context_menu: Option<NoirContextMenu>,
    pending_changed: bool,
}

/// Drop-in Noir replacement / enhancement for `egui::TextEdit`.
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct NoirTextEdit<'t> {
    text: &'t mut dyn TextBuffer,
    id: Option<Id>,
    id_salt: Option<IdSalt>,
    hint_text: Option<WidgetText>,
    font_selection: FontSelection,
    text_color: Option<Color32>,
    background_color: Option<Color32>,
    password: bool,
    multiline: bool,
    interactive: bool,
    lock_focus: bool,
    desired_width: Option<f32>,
    desired_rows: usize,
    clip_text: bool,
    char_limit: usize,
    frame: Option<Frame>,
    margin: Option<Margin>,
    mode: TextMode,
    spellcheck: bool,
    autocorrect: bool,
    prediction: bool,
}

/// Convenient alias matching `egui::TextEdit`.
pub type TextEdit<'t> = NoirTextEdit<'t>;

/// Ergonomic helpers on `egui::Ui` for creating SexOS Noir text inputs.
pub trait SexUiExt {
    fn noir_text_singleline(&mut self, text: &mut dyn TextBuffer) -> Response;
    fn noir_text_multiline(&mut self, text: &mut dyn TextBuffer) -> Response;
}

impl SexUiExt for Ui {
    fn noir_text_singleline(&mut self, text: &mut dyn TextBuffer) -> Response {
        self.add(NoirTextEdit::singleline(text))
    }

    fn noir_text_multiline(&mut self, text: &mut dyn TextBuffer) -> Response {
        self.add(NoirTextEdit::multiline(text))
    }
}

impl<'t> NoirTextEdit<'t> {
    pub fn singleline(text: &'t mut dyn TextBuffer) -> Self {
        Self {
            text,
            id: None,
            id_salt: None,
            hint_text: None,
            font_selection: Default::default(),
            text_color: None,
            background_color: None,
            password: false,
            multiline: false,
            interactive: true,
            lock_focus: false,
            desired_width: None,
            desired_rows: 1,
            clip_text: true,
            char_limit: usize::MAX,
            frame: None,
            margin: None,
            mode: TextMode::SingleLineProse,
            spellcheck: true,
            autocorrect: true,
            prediction: true,
        }
    }

    pub fn multiline(text: &'t mut dyn TextBuffer) -> Self {
        Self {
            multiline: true,
            desired_rows: 4,
            clip_text: false,
            mode: TextMode::Prose,
            ..Self::singleline(text)
        }
    }

    pub fn code_editor(mut self) -> Self {
        self.font_selection = TextStyle::Monospace.into();
        self.lock_focus = true;
        self.mode = TextMode::Code;
        self
    }

    pub fn password(mut self, password: bool) -> Self {
        self.password = password;
        if password {
            self.mode = TextMode::Password;
        }
        self
    }

    pub fn mode(mut self, mode: TextMode) -> Self {
        self.mode = mode;
        if mode == TextMode::Password {
            self.password = true;
        } else if mode == TextMode::Code {
            self.font_selection = TextStyle::Monospace.into();
            self.lock_focus = true;
        }
        self
    }

    pub fn with_config(mut self, cfg: &Config) -> Self {
        self.autocorrect = cfg.flag(sex_config::id::AUTOCORRECT);
        self.spellcheck = cfg.flag(sex_config::id::SPELLCHECK);
        self.prediction = cfg.flag(sex_config::id::INLINE_PREDICT);
        self
    }

    pub fn spellcheck(mut self, enabled: bool) -> Self {
        self.spellcheck = enabled;
        self
    }

    pub fn autocorrect(mut self, enabled: bool) -> Self {
        self.autocorrect = enabled;
        self
    }

    pub fn prediction(mut self, enabled: bool) -> Self {
        self.prediction = enabled;
        self
    }

    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn id_salt(mut self, id_salt: impl AsIdSalt) -> Self {
        self.id_salt = Some(IdSalt::new(id_salt));
        self
    }

    pub fn id_source(self, id_salt: impl AsIdSalt) -> Self {
        self.id_salt(id_salt)
    }

    pub fn hint_text(mut self, hint_text: impl Into<WidgetText>) -> Self {
        self.hint_text = Some(hint_text.into());
        self
    }

    pub fn font(mut self, font: impl Into<FontSelection>) -> Self {
        self.font_selection = font.into();
        self
    }

    pub fn text_color(mut self, color: Color32) -> Self {
        self.text_color = Some(color);
        self
    }

    pub fn background_color(mut self, color: Color32) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    pub fn lock_focus(mut self, lock: bool) -> Self {
        self.lock_focus = lock;
        self
    }

    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    pub fn desired_rows(mut self, rows: usize) -> Self {
        self.desired_rows = rows;
        self
    }

    pub fn clip_text(mut self, clip: bool) -> Self {
        self.clip_text = clip;
        self
    }

    pub fn char_limit(mut self, limit: usize) -> Self {
        self.char_limit = limit;
        self
    }

    pub fn frame(mut self, frame: bool) -> Self {
        self.frame = Some(if frame { Frame::default() } else { Frame::NONE });
        self
    }

    pub fn margin(mut self, margin: Margin) -> Self {
        self.margin = Some(margin);
        self
    }

    fn effective_spellcheck(&self) -> bool {
        self.spellcheck && self.mode.allows_spellcheck() && !self.password
    }

    fn effective_prediction(&self) -> bool {
        self.prediction && self.mode.allows_prediction() && !self.password && self.text.is_mutable()
    }

    fn effective_autocorrect(&self) -> bool {
        self.autocorrect && self.mode.allows_autocorrect() && !self.password && self.text.is_mutable()
    }

    /// Shows the widget and returns inner `TextEditOutput` (plus overlays & context popup).
    pub fn show(mut self, ui: &mut Ui) -> TextEditOutput {
        let id = self.id.unwrap_or_else(|| {
            if let Some(salt) = &self.id_salt {
                ui.make_persistent_id(salt)
            } else {
                ui.next_auto_id()
            }
        });

        let mut field = ui.data_mut(|d| d.get_temp::<NoirFieldState>(id)).unwrap_or_default();
        let has_focus = self.interactive && ui.memory(|m| m.has_focus(id));

        if has_focus && self.text.is_mutable() {
            self.handle_pre_events(ui, id, &mut field);
        } else if !has_focus {
            field.ghost = None;
            field.autocorrect.clear();
        }

        // Handle context menu actions BEFORE painting inner TextEdit if an item was clicked on prev frame?
        // Wait: Area below runs after TextEdit, OR runs via direct mutation inside this pass and calls request_repaint().

        let lock_tab = self.lock_focus || (has_focus && field.ghost.is_some());
        let font_sel = self.font_selection.clone();
        let mut inner = if self.multiline {
            EguiTextEdit::multiline(self.text)
        } else {
            EguiTextEdit::singleline(self.text)
        }
        .id(id)
        .password(self.password)
        .font(font_sel)
        .interactive(self.interactive)
        .lock_focus(lock_tab)
        .desired_rows(self.desired_rows)
        .clip_text(self.clip_text)
        .char_limit(self.char_limit);

        if let Some(hint) = self.hint_text.take() {
            inner = inner.hint_text(hint);
        }
        if let Some(c) = self.text_color {
            inner = inner.text_color(c);
        }
        if let Some(c) = self.background_color {
            inner = inner.background_color(c);
        }
        if let Some(w) = self.desired_width {
            inner = inner.desired_width(w);
        }
        if let Some(m) = self.margin {
            inner = inner.margin(m);
        }

        let mut output = inner.show(ui);
        if field.pending_changed {
            output.response.mark_changed();
            field.pending_changed = false;
        }

        let mut spell = ui
            .data_mut(|d| d.get_temp::<SharedSpell>(shared_spell_id()))
            .unwrap_or_default();

        // Catch Space-bar autocorrect after single/multi-keystroke frame batches if not handled pre-inner.
        if has_focus && self.effective_autocorrect() && output.response.changed() {
            if let Some(cr) = output.cursor_range {
                if cr.is_empty() {
                    let cursor_byte = self.text.byte_index_from_char_index(cr.primary.index).0;
                    if self.text.as_str().as_bytes().get(cursor_byte.saturating_sub(1)) == Some(&b' ')
                        && self.did_type_space_this_frame(ui)
                        && !field.autocorrect.should_revert_at(cursor_byte)
                    {
                        if self.try_autocorrect_before_space(
                            ui.ctx(),
                            id,
                            &mut output.state,
                            cursor_byte,
                            &spell.0,
                            &mut field,
                        ) {
                            output.cursor_range = output.state.cursor.range(&output.galley);
                            output.response.mark_changed();
                            ui.ctx().request_repaint();
                        }
                    }
                }
            }
        }

        // Update inline ghost prediction at word end.
        let now_focused = self.interactive && ui.memory(|m| m.has_focus(id));
        field.ghost = None;
        if now_focused && self.effective_prediction() {
            if let Some(cr) = output.cursor_range {
                if cr.is_empty() {
                    let cursor_byte = self.text.byte_index_from_char_index(cr.primary.index).0;
                    if trailing_line_is_blank(self.text.as_str(), cursor_byte) {
                        if let Some((_, prefix)) = active_word_prefix_at(self.text.as_str(), cursor_byte) {
                            if let Some(suffix) = spell.0.predict_completion(prefix) {
                                field.ghost = Some((cr.primary.index.0, suffix));
                            }
                        }
                    }
                }
            }
        }

        // Keep Tab locked on this widget whenever ghost completion is visible.
        if now_focused && field.ghost.is_some() {
            ui.memory_mut(|mem| {
                mem.set_focus_lock_filter(
                    id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: self.multiline,
                        escape: false,
                    },
                );
            });
        }

        // Paint red misspelled wavy underlines, indigo autocorrect underline, and dim ghost suffix.
        self.paint_overlays(ui, &output, &spell.0, &field);

        // Open Noir Context Menu on Right-Click (`button == 2`) OR `Ctrl + Left-Click`.
        if self.interactive {
            self.check_context_menu_trigger(ui, id, &mut output, &spell.0, &mut field);
        }

        // Render and handle Noir Context Menu popup if active.
        if field.context_menu.is_some() {
            let changed = self.show_context_menu(ui, id, &mut output, &mut spell.0, &mut field);
            if changed {
                output.response.mark_changed();
                ui.ctx().request_repaint();
            }
        }

        ui.data_mut(|d| {
            d.insert_temp(shared_spell_id(), spell);
            d.insert_temp(id, field);
        });

        output
    }

    fn handle_pre_events(&mut self, ui: &mut Ui, id: Id, field: &mut NoirFieldState) {
        let Some(mut state) = EguiTextEdit::load_state(ui.ctx(), id) else { return };
        let Some(cr) = state.cursor.char_range() else { return };

        // 1. Accept ghost completion on Tab or Right Arrow (no selection, unmodified).
        if cr.is_empty() && self.effective_prediction() {
            if let Some((ghost_char, suffix)) = field.ghost {
                if ghost_char == cr.primary.index.0 {
                    let accept_triggered = ui.input(|i| {
                        i.events.iter().any(|ev| {
                            matches!(
                                ev,
                                Event::Key {
                                    key: Key::Tab | Key::ArrowRight,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } if !modifiers.shift && !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd
                            )
                        })
                    });
                    if accept_triggered {
                        ui.input_mut(|i| {
                            i.events.retain(|ev| {
                                !matches!(
                                    ev,
                                    Event::Key {
                                        key: Key::Tab | Key::ArrowRight,
                                        modifiers,
                                        ..
                                    } if !modifiers.shift && !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd
                                )
                            });
                        });
                        let inserted = self.text.insert_text(suffix.as_str(), CharIndex(ghost_char));
                        let new_cc = CCursor::new(ghost_char + inserted);
                        state.cursor.set_char_range(Some(CCursorRange::one(new_cc)));
                        EguiTextEdit::store_state(ui.ctx(), id, state);
                        field.ghost = None;
                        field.autocorrect.clear();
                        field.pending_changed = true;
                        return;
                    }
                }
            }
        }

        // 2. Instant 1-Backspace revert of most recent Space-bar autocorrect.
        if cr.is_empty() && self.effective_autocorrect() && field.autocorrect.active {
            let cursor_byte = self.text.byte_index_from_char_index(cr.primary.index).0;
            let backspace_pressed = ui.input(|i| {
                i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        Event::Key {
                            key: Key::Backspace,
                            pressed: true,
                            modifiers,
                            ..
                        } if !modifiers.shift && !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd
                    )
                })
            });
            if backspace_pressed && field.autocorrect.should_revert_at(cursor_byte) {
                ui.input_mut(|i| {
                    i.events.retain(|ev| {
                        !matches!(ev, Event::Key { key: Key::Backspace, .. })
                    });
                });
                let word_start = field.autocorrect.word_start;
                let corrected_end = field.autocorrect.corrected_end;
                let orig = field.autocorrect.original;
                field.autocorrect.clear();
                if corrected_end <= self.text.as_str().len() && word_start <= corrected_end {
                    let c_start = self.text.char_index_from_byte_index(ByteIndex(word_start));
                    let c_end = self.text.char_index_from_byte_index(ByteIndex(corrected_end));
                    self.text.delete_char_range(c_start..c_end);
                    self.text.insert_text(orig.as_str(), c_start);
                    // Restore cursor right after the preserved trailing Space.
                    let after_space = self
                        .text
                        .char_index_from_byte_index(ByteIndex((word_start + orig.len() + 1).min(self.text.as_str().len())));
                    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(after_space.0))));
                    EguiTextEdit::store_state(ui.ctx(), id, state);
                    field.suppress_autocorrect_at = Some(word_start);
                    field.pending_changed = true;
                    return;
                }
            }
        }

        // Clear autocorrect revert / suppression if user navigates or enters non-space input.
        let non_space_edit_or_nav = ui.input(|i| {
            i.events.iter().any(|ev| match ev {
                Event::Text(s) if s != " " => true,
                Event::Paste(_) | Event::Cut => true,
                Event::Key {
                    key: Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End | Key::Backspace | Key::Delete | Key::Enter,
                    pressed: true,
                    ..
                } => true,
                Event::PointerButton { pressed: true, .. } => true,
                _ => false,
            })
        });
        if non_space_edit_or_nav {
            field.autocorrect.clear();
            field.suppress_autocorrect_at = None;
        }
    }

    fn did_type_space_this_frame(&self, ui: &Ui) -> bool {
        ui.input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|ev| match ev {
                    Event::Text(s) => Some(s == " "),
                    _ => None,
                })
                .unwrap_or(false)
        })
    }

    fn try_autocorrect_before_space(
        &mut self,
        ctx: &Context,
        id: Id,
        state: &mut egui::text_edit::TextEditState,
        cursor_byte_after_space: usize,
        spell: &SpellEngine,
        field: &mut NoirFieldState,
    ) -> bool {
        let word_end = cursor_byte_after_space.saturating_sub(1);
        let text = self.text.as_str();
        if word_end < 2 || word_end > text.len() {
            return false;
        }
        let bytes = text.as_bytes();
        let mut word_start = word_end;
        while word_start > 0 && (bytes[word_start - 1].is_ascii_alphabetic() || bytes[word_start - 1] == b'\'') {
            word_start -= 1;
        }
        if word_end - word_start < 2 {
            return false;
        }
        if word_start > 0
            && (bytes[word_start - 1].is_ascii_digit()
                || matches!(bytes[word_start - 1], b'_' | b'/' | b'\\' | b'.' | b'@' | b':'))
        {
            return false;
        }
        if field.suppress_autocorrect_at == Some(word_start) {
            return false;
        }
        let orig = &text[word_start..word_end];
        let Some(corrected) = spell.autocorrect_on_space(orig) else {
            return false;
        };
        let Some(orig_buf) = WordBuf::from_str(orig) else {
            return false;
        };

        let c_start = self.text.char_index_from_byte_index(ByteIndex(word_start));
        let c_end = self.text.char_index_from_byte_index(ByteIndex(word_end));
        self.text.delete_char_range(c_start..c_end);
        let ins_chars = self.text.insert_text(corrected.as_str(), c_start);
        // Cursor sits right after the trailing Space (+1 char).
        let new_cc = CCursor::new(c_start.0 + ins_chars + 1);
        state.cursor.set_char_range(Some(CCursorRange::one(new_cc)));
        EguiTextEdit::store_state(ctx, id, state.clone());

        field
            .autocorrect
            .record(word_start, orig_buf.as_str(), corrected.as_str(), 1);
        field.suppress_autocorrect_at = None;
        true
    }

    fn paint_overlays(&self, ui: &Ui, output: &TextEditOutput, spell: &SpellEngine, field: &NoirFieldState) {
        let painter = ui.painter_at(output.text_clip_rect.expand(2.0));
        let galley_origin = output.galley_pos - Vec2::new(output.galley.rect.left(), 0.0);
        let text = self.text.as_str();

        if self.effective_spellcheck() && !text.is_empty() {
            let active_ghost_byte = field
                .ghost
                .as_ref()
                .map(|(ch_idx, _)| self.text.byte_index_from_char_index(CharIndex(*ch_idx)).0);

            spell.scan_misspelled(text, self.mode, |span| {
                if active_ghost_byte == Some(span.end) {
                    return;
                }
                self.paint_wavy_underline(&painter, &output.galley, galley_origin, span, SPELL_UNDERLINE_COLOR);
            });
        }

        if let Some((start, end)) = field.autocorrect.underline_span() {
            if end <= text.len() {
                self.paint_straight_underline(
                    &painter,
                    &output.galley,
                    galley_origin,
                    WordSpan { start, end },
                    AUTOCORRECT_UNDERLINE_COLOR,
                );
            }
        }

        if let (Some((ghost_ch, suffix)), Some(cr)) = (&field.ghost, output.cursor_range) {
            if cr.is_empty() && *ghost_ch == cr.primary.index.0 {
                let cursor_rect = output.galley.pos_from_cursor(cr.primary);
                let pos = galley_origin + cursor_rect.min.to_vec2();
                let font_id = self.resolve_font_id(ui);
                painter.text(pos, Align2::LEFT_TOP, suffix.as_str(), font_id, ghost_text_color());
            }
        }
    }

    fn resolve_font_id(&self, ui: &Ui) -> FontId {
        match &self.font_selection {
            FontSelection::Default => ui.style().override_font_id.clone().unwrap_or_else(|| {
                TextStyle::Body.resolve(ui.style())
            }),
            FontSelection::FontId(id) => id.clone(),
            FontSelection::Style(style) => style.resolve(ui.style()),
        }
    }

    fn span_segment(&self, galley: &egui::Galley, origin: Pos2, span: WordSpan) -> Option<(Pos2, f32)> {
        let text = self.text.as_str();
        if span.end > text.len() || span.start >= span.end {
            return None;
        }
        let c0 = CharIndex(text[..span.start].chars().count());
        let c1 = CharIndex(text[..span.end].chars().count());
        let r0 = galley.pos_from_cursor(CCursor::new(c0.0));
        let r1 = galley.pos_from_cursor(CCursor::new(c1.0));
        if (r0.min.y - r1.min.y).abs() > 2.0 || r1.min.x <= r0.min.x {
            return None;
        }
        let y = origin.y + r0.max.y - 0.5;
        Some((Pos2::new(origin.x + r0.min.x, y), r1.min.x - r0.min.x))
    }

    fn paint_wavy_underline(&self, painter: &egui::Painter, galley: &egui::Galley, origin: Pos2, span: WordSpan, color: Color32) {
        let Some((start, width)) = self.span_segment(galley, origin, span) else { return };
        let step = 2.5f32;
        let amp = 1.0f32;
        let mut pts = Vec::new();
        let mut x = 0.0f32;
        let mut up = true;
        pts.push(Pos2::new(start.x, start.y));
        while x + step < width {
            x += step;
            let dy = if up { -amp } else { amp };
            up = !up;
            pts.push(Pos2::new(start.x + x, start.y + dy));
        }
        pts.push(Pos2::new(start.x + width, start.y));
        painter.line(pts, Stroke::new(1.25, color));
    }

    fn paint_straight_underline(&self, painter: &egui::Painter, galley: &egui::Galley, origin: Pos2, span: WordSpan, color: Color32) {
        let Some((start, width)) = self.span_segment(galley, origin, span) else { return };
        painter.line_segment(
            [start, Pos2::new(start.x + width, start.y)],
            Stroke::new(1.4, color),
        );
    }

    fn check_context_menu_trigger(
        &mut self,
        ui: &Ui,
        id: Id,
        output: &mut TextEditOutput,
        spell: &SpellEngine,
        field: &mut NoirFieldState,
    ) {
        let rect = output.response.rect;
        let secondary_clicked = output.response.secondary_clicked();
        let primary_clicked = output.response.clicked();
        let trigger_pos = ui.input(|i| {
            let secondary = i.pointer.button_pressed(PointerButton::Secondary) || secondary_clicked;
            let ctrl_primary = i.modifiers.ctrl && (i.pointer.button_pressed(PointerButton::Primary) || primary_clicked);
            if !secondary && !ctrl_primary {
                return None;
            }
            i.pointer
                .hover_pos()
                .or_else(|| i.pointer.interact_pos())
                .filter(|p| rect.contains(*p))
        });

        let Some(pos) = trigger_pos else { return };

        let galley_origin = output.galley_pos - Vec2::new(output.galley.rect.left(), 0.0);
        let clicked_cc = output.galley.cursor_from_pos(pos - galley_origin);
        let byte_idx = self.text.byte_index_from_char_index(clicked_cc.index).0;
        let text = self.text.as_str();

        let target_span = if !self.password {
            word_span_at(text, byte_idx)
        } else {
            None
        };

        // Select/target clicked word unless user already selected a larger range around it.
        let mut target_chars = None;
        if let Some(span) = target_span {
            let existing_bytes = output.cursor_range.map(|cr| {
                let [c0, c1] = cr.sorted_cursors();
                (
                    self.text.byte_index_from_char_index(c0.index).0,
                    self.text.byte_index_from_char_index(c1.index).0,
                )
            });
            let c0 = self.text.char_index_from_byte_index(ByteIndex(span.start));
            let c1 = self.text.char_index_from_byte_index(ByteIndex(span.end));
            target_chars = Some((c0.0, c1.0));
            let keep_existing = existing_bytes.is_some_and(|(a, b)| a <= span.start && span.end <= b && (b - a) > (span.end - span.start));
            if !keep_existing {
                let new_range = CCursorRange::two(CCursor::new(c0.0), CCursor::new(c1.0));
                output.state.cursor.set_char_range(Some(new_range));
                output.cursor_range = Some(new_range);
                EguiTextEdit::store_state(ui.ctx(), id, output.state.clone());
            }
        }
        ui.memory_mut(|m| m.request_focus(id));

        let mut items = Vec::new();
        let mut divider_after = None;

        if self.effective_spellcheck() {
            if let Some(span) = target_span {
                let target_word = &text[span.start..span.end];
                if !spell.check_word(target_word) {
                    let mut sug_buf = [WordBuf::new(); 3];
                    let count = spell.suggest(target_word, &mut sug_buf);
                    for s in &sug_buf[..count] {
                        items.push(MenuItem {
                            label: s.as_str().to_string(),
                            shortcut: "",
                            enabled: self.text.is_mutable(),
                            prominent: true,
                            action: MenuAction::Replace {
                                byte_start: span.start,
                                byte_end: span.end,
                                replacement: s.as_str().to_string(),
                            },
                        });
                    }
                    items.push(MenuItem {
                        label: "Ignore Word".to_string(),
                        shortcut: "",
                        enabled: true,
                        prominent: false,
                        action: MenuAction::Ignore(target_word.to_string()),
                    });
                    items.push(MenuItem {
                        label: "Learn Spelling".to_string(),
                        shortcut: "",
                        enabled: true,
                        prominent: false,
                        action: MenuAction::Learn(target_word.to_string()),
                    });
                    divider_after = Some(items.len() - 1);
                }
            }
        }

        let has_sel = output.cursor_range.is_some_and(|cr| !cr.is_empty());
        let has_clip = !crate::clipboard_text(ui.ctx()).is_empty();
        let allow_copy = !self.password;

        items.push(MenuItem {
            label: "Cut".to_string(),
            shortcut: "⌘X",
            enabled: self.text.is_mutable() && allow_copy && has_sel,
            prominent: false,
            action: MenuAction::Cut,
        });
        items.push(MenuItem {
            label: "Copy".to_string(),
            shortcut: "⌘C",
            enabled: allow_copy && has_sel,
            prominent: false,
            action: MenuAction::Copy,
        });
        items.push(MenuItem {
            label: "Paste".to_string(),
            shortcut: "⌘V",
            enabled: self.text.is_mutable() && has_clip,
            prominent: false,
            action: MenuAction::Paste,
        });
        items.push(MenuItem {
            label: "Select All".to_string(),
            shortcut: "⌘A",
            enabled: !text.is_empty(),
            prominent: false,
            action: MenuAction::SelectAll,
        });

        field.context_menu = Some(NoirContextMenu {
            pos,
            target_chars,
            items,
            divider_after,
        });
    }

    fn show_context_menu(
        &mut self,
        ui: &Ui,
        id: Id,
        output: &mut TextEditOutput,
        spell: &mut SpellEngine,
        field: &mut NoirFieldState,
    ) -> bool {
        let Some(menu) = field.context_menu.clone() else { return false };

        if ui.input(|i| i.key_pressed(Key::Escape)) {
            field.context_menu = None;
            return false;
        }

        let area_id = id.with("noir_context_menu");
        let screen = ui.ctx().content_rect();
        let est_w = 212.0f32;
        let est_h = menu.items.len() as f32 * 26.0 + if menu.divider_after.is_some() { 20.0 } else { 12.0 };
        let popup_pos = Pos2::new(
            menu.pos.x.min((screen.right() - est_w - 6.0).max(screen.left() + 4.0)),
            menu.pos.y.min((screen.bottom() - est_h - 6.0).max(screen.top() + 4.0)),
        );

        let mut chosen: Option<MenuAction> = None;
        let area_resp = Area::new(area_id)
            .order(Order::Foreground)
            .fixed_pos(popup_pos)
            .show(ui.ctx(), |ui| {
                Frame::NONE
                    .fill(menu_bg())
                    .stroke(Stroke::new(1.0, menu_border()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(6, 6))
                    .show(ui, |ui| {
                        ui.set_min_width(196.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (idx, item) in menu.items.iter().enumerate() {
                            if paint_menu_row(ui, item) {
                                chosen = Some(item.action.clone());
                            }
                            if menu.divider_after == Some(idx) {
                                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 7.0), Sense::hover());
                                let y = r.center().y;
                                ui.painter().line_segment(
                                    [Pos2::new(r.left() + 4.0, y), Pos2::new(r.right() - 4.0, y)],
                                    Stroke::new(1.0, menu_divider()),
                                );
                            }
                        }
                    });
            });

        if let Some(action) = chosen {
            field.context_menu = None;
            return self.apply_menu_action(ui, id, output, spell, field, menu.target_chars, action);
        }

        // Dismiss if user clicked anywhere outside the menu popup (wait for next pointer press outside).
        let clicked_outside = ui.input(|i| {
            let pressed = i.pointer.button_pressed(PointerButton::Primary)
                || i.pointer.button_pressed(PointerButton::Secondary);
            if !pressed {
                return false;
            }
            let pos = i.pointer.hover_pos().or_else(|| i.pointer.interact_pos());
            pos.is_some_and(|p| !area_resp.response.rect.contains(p) && !output.response.rect.contains(p))
        });
        if clicked_outside {
            field.context_menu = None;
        }
        false
    }

    fn apply_menu_action(
        &mut self,
        ui: &Ui,
        id: Id,
        output: &mut TextEditOutput,
        spell: &mut SpellEngine,
        field: &mut NoirFieldState,
        target_chars: Option<(usize, usize)>,
        action: MenuAction,
    ) -> bool {
        let mut changed = false;
        match action {
            MenuAction::Replace { byte_start, byte_end, replacement } => {
                if self.text.is_mutable() && byte_end <= self.text.as_str().len() && byte_start <= byte_end {
                    let c0 = self.text.char_index_from_byte_index(ByteIndex(byte_start));
                    let c1 = self.text.char_index_from_byte_index(ByteIndex(byte_end));
                    self.text.delete_char_range(c0..c1);
                    let ins = self.text.insert_text(&replacement, c0);
                    let next_cc = CCursor::new(c0.0 + ins);
                    output.state.cursor.set_char_range(Some(CCursorRange::one(next_cc)));
                    EguiTextEdit::store_state(ui.ctx(), id, output.state.clone());
                    changed = true;
                }
            }
            MenuAction::Ignore(word) => {
                spell.ignore_word(&word);
                let _ = target_chars;
            }
            MenuAction::Learn(word) => {
                spell.learn_word(&word);
            }
            MenuAction::Copy => {
                if !self.password {
                    if let Some(cr) = output.cursor_range.filter(|r| !r.is_empty()) {
                        let sel = cr.slice_str(self.text.as_str()).to_owned();
                        ui.copy_text(sel.clone());
                        crate::set_clipboard(ui.ctx(), &sel);
                    }
                }
            }
            MenuAction::Cut => {
                if self.text.is_mutable() && !self.password {
                    if let Some(cr) = output.cursor_range.filter(|r| !r.is_empty()) {
                        let sel = cr.slice_str(self.text.as_str()).to_owned();
                        ui.copy_text(sel.clone());
                        crate::set_clipboard(ui.ctx(), &sel);
                        let new_cc = self.text.delete_selected(&cr);
                        output.state.cursor.set_char_range(Some(CCursorRange::one(new_cc)));
                        EguiTextEdit::store_state(ui.ctx(), id, output.state.clone());
                        changed = true;
                    }
                }
            }
            MenuAction::Paste => {
                if self.text.is_mutable() {
                    let clip = crate::clipboard_text(ui.ctx());
                    if !clip.is_empty() {
                        let insert_str = if self.multiline {
                            clip
                        } else {
                            clip.replace(['\r', '\n'], " ")
                        };
                        let cr = output
                            .cursor_range
                            .unwrap_or_else(|| CCursorRange::one(CCursor::end_of_str(self.text.as_str())));
                        let mut cc = self.text.delete_selected(&cr);
                        self.text.insert_text_at(&mut cc, &insert_str, self.char_limit);
                        output.state.cursor.set_char_range(Some(CCursorRange::one(cc)));
                        EguiTextEdit::store_state(ui.ctx(), id, output.state.clone());
                        changed = true;
                    }
                }
            }
            MenuAction::SelectAll => {
                let all = CCursorRange::select_all_str(self.text.as_str());
                output.state.cursor.set_char_range(Some(all));
                EguiTextEdit::store_state(ui.ctx(), id, output.state.clone());
            }
        }
        field.autocorrect.clear();
        changed
    }
}

impl Widget for NoirTextEdit<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui).response.response
    }
}

fn paint_menu_row(ui: &mut Ui, item: &MenuItem) -> bool {
    let desired = Vec2::new(ui.available_width().max(180.0), 24.0);
    let (rect, resp) = ui.allocate_exact_size(desired, if item.enabled { Sense::click() } else { Sense::hover() });
    let hovered = item.enabled && resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(6), MENU_HOVER);
    }
    let text_color = if !item.enabled {
        MENU_DISABLED
    } else if hovered {
        Color32::WHITE
    } else {
        MENU_TEXT
    };
    let font = FontId::proportional(if item.prominent { 13.5 } else { 13.0 });
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        &item.label,
        font,
        text_color,
    );
    if !item.shortcut.is_empty() {
        let sc_color = if hovered { Color32::from_rgb(230, 238, 255) } else { MENU_SHORTCUT };
        ui.painter().text(
            Pos2::new(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            item.shortcut,
            FontId::proportional(12.0),
            sc_color,
        );
    }
    item.enabled && resp.clicked()
}

fn trailing_line_is_blank(text: &str, cursor_byte: usize) -> bool {
    let tail = &text[cursor_byte.min(text.len())..];
    for ch in tail.chars() {
        if ch == '\n' || ch == '\r' {
            return true;
        }
        if !ch.is_whitespace() {
            return false;
        }
    }
    true
}
