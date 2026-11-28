// SPDX-License-Identifier: GPL-3.0-only
//! Unified rich text editing controller combining `Doc` (UTF-8 buffer + undo tree),
//! `sex-spell` spellcheck/suggestions/prediction/autocorrect, universal `Super` + `Ctrl`
//! shortcuts, double/triple-click selection, and macOS Noir right-click / `Ctrl+Click`
//! context menu actions.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
pub use sex_spell::{AutocorrectRevert, SpellEngine, TextMode, WordBuf, WordSpan};

use crate::{next_char, prev_char, Doc, Motion};

/// Modifier key state (normalizes both `Ctrl` and `Super` cleanly).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_cmd: bool,
}

impl Modifiers {
    pub const NONE: Self = Self { shift: false, ctrl: false, alt: false, super_cmd: false };

    /// True when either `Super` (⌘) or `Ctrl` is pressed (without `Alt`).
    pub const fn primary_cmd(self) -> bool {
        self.ctrl || self.super_cmd
    }
}

/// Platform-independent key input for `RichEditor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Letter(u8),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Result of handling a keyboard event or context-menu action.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyOutcome {
    pub handled: bool,
    pub text_changed: bool,
    pub submitted: bool,
    pub copied: Option<Vec<u8>>,
    pub cut: Option<Vec<u8>>,
    pub request_paste: bool,
}

/// Concrete action triggered by choosing a context-menu item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextAction {
    ReplaceTarget { start: usize, end: usize, replacement: String },
    IgnoreWord(String),
    LearnWord(String),
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// Single entry inside a Noir text context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextItem {
    pub label: String,
    pub shortcut: &'static str,
    pub enabled: bool,
    pub prominent: bool,
    pub action: ContextAction,
}

/// Active right-click / `Ctrl+Left-Click` context menu state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenu {
    pub target_span: Option<WordSpan>,
    pub target_word: Option<String>,
    pub misspelled: bool,
    pub items: Vec<ContextItem>,
    pub divider_after: Option<usize>,
}

/// Shared high-level text controller backing single-line and multi-line text fields.
pub struct RichEditor {
    pub doc: Doc,
    pub mode: TextMode,
    pub read_only: bool,
    pub spellcheck_enabled: bool,
    pub autocorrect_enabled: bool,
    pub prediction_enabled: bool,
    pub spell: SpellEngine,
    pub autocorrect: AutocorrectRevert,
    pub context_menu: Option<ContextMenu>,
    misspelled: Vec<WordSpan>,
    ghost: Option<(usize, WordBuf)>,
    drag_anchor: Option<(usize, usize)>,
    drag_granularity: u8,
    suppress_autocorrect_at: Option<usize>,
}

impl RichEditor {
    pub fn new(text: Vec<u8>, mode: TextMode) -> Self {
        let mut ed = Self {
            doc: Doc::new(text),
            mode,
            read_only: false,
            spellcheck_enabled: true,
            autocorrect_enabled: true,
            prediction_enabled: true,
            spell: SpellEngine::new(),
            autocorrect: AutocorrectRevert::new(),
            context_menu: None,
            misspelled: Vec::new(),
            ghost: None,
            drag_anchor: None,
            drag_granularity: 0,
            suppress_autocorrect_at: None,
        };
        ed.refresh_analysis();
        ed
    }

    pub fn from_str(text: &str, mode: TextMode) -> Self {
        Self::new(text.as_bytes().to_vec(), mode)
    }

    pub fn text_str(&self) -> &str {
        self.doc.text_str()
    }

    pub fn set_text(&mut self, text: &str) {
        if self.doc.text_str() == text {
            return;
        }
        let cursor = self.doc.cursor.min(text.len());
        self.doc = Doc::new(text.as_bytes().to_vec());
        self.doc.move_to(Motion::To(cursor), false);
        self.autocorrect.clear();
        self.context_menu = None;
        self.suppress_autocorrect_at = None;
        self.refresh_analysis();
    }

    pub fn set_mode(&mut self, mode: TextMode) {
        if self.mode != mode {
            self.mode = mode;
            if !mode.allows_autocorrect() {
                self.autocorrect.clear();
            }
            self.refresh_analysis();
        }
    }

    pub const fn spellcheck_active(&self) -> bool {
        self.spellcheck_enabled && self.mode.allows_spellcheck()
    }

    pub const fn prediction_active(&self) -> bool {
        self.prediction_enabled && self.mode.allows_prediction() && !self.read_only
    }

    pub const fn autocorrect_active(&self) -> bool {
        self.autocorrect_enabled && self.mode.allows_autocorrect() && !self.read_only
    }

    /// Cached misspelled word byte spans in prose modes.
    pub fn misspelled_spans(&self) -> &[WordSpan] {
        &self.misspelled
    }

    /// Active inline ghost-text completion suffix right after the cursor, if any.
    pub fn ghost_suffix(&self) -> Option<&str> {
        let (at, word) = self.ghost.as_ref()?;
        (*at == self.doc.cursor && self.doc.selection().is_none()).then_some(word.as_str())
    }

    /// Active blue/indigo underline `(start, end)` from Space-bar autocorrect awaiting possible revert.
    pub fn autocorrect_underline(&self) -> Option<(usize, usize)> {
        self.autocorrect.underline_span()
    }

    /// Accepts the currently visible inline ghost completion (`Tab` / `Right Arrow`).
    pub fn accept_ghost_completion(&mut self) -> bool {
        let Some((at, suf)) = self.ghost else { return false };
        if self.read_only || at != self.doc.cursor || self.doc.selection().is_some() {
            return false;
        }
        self.autocorrect.clear();
        self.doc.insert(suf.as_bytes());
        self.doc.seal();
        self.refresh_analysis();
        true
    }

    /// Handles pointer primary press (`click_count` = 1, 2 for word, 3 for line).
    /// When `mods.ctrl` is pressed, triggers the `Ctrl+Click` context menu instead.
    pub fn on_primary_click(&mut self, byte_idx: usize, click_count: u8, mods: Modifiers, has_clipboard: bool) -> bool {
        self.autocorrect.clear();
        if mods.ctrl {
            self.open_context_menu_at(byte_idx, has_clipboard);
            return true;
        }
        self.context_menu = None;
        self.suppress_autocorrect_at = None;
        match click_count {
            2 => {
                self.doc.select_word_at(byte_idx);
                self.drag_anchor = self.doc.selection();
                self.drag_granularity = 1;
            }
            3.. => {
                self.doc.select_line_at(byte_idx);
                self.drag_anchor = self.doc.selection();
                self.drag_granularity = 2;
            }
            _ => {
                self.doc.click_at(byte_idx, mods.shift);
                self.drag_anchor = None;
                self.drag_granularity = 0;
            }
        }
        self.refresh_analysis();
        false
    }

    /// Handles pointer drag after primary press, respecting char/word/line granularity.
    pub fn on_primary_drag(&mut self, byte_idx: usize) {
        self.autocorrect.clear();
        if self.drag_granularity == 0 {
            self.doc.drag_to(byte_idx);
        } else if let Some((a0, b0)) = self.drag_anchor {
            let (u0, u1) = if self.drag_granularity == 1 {
                self.doc.word_bounds_at(byte_idx)
            } else {
                self.doc.line_bounds_at(byte_idx)
            };
            if u0 < a0 {
                self.doc.select(b0, u0);
            } else {
                self.doc.select(a0, u1.max(b0));
            }
        }
        self.refresh_analysis();
    }

    /// Opens the Noir context menu at `byte_idx` (`button == 2` right-click or `Ctrl+Left-Click`).
    pub fn on_secondary_click(&mut self, byte_idx: usize, has_clipboard: bool) -> &ContextMenu {
        self.open_context_menu_at(byte_idx, has_clipboard)
    }

    pub fn open_context_menu_at(&mut self, byte_idx: usize, has_clipboard: bool) -> &ContextMenu {
        self.autocorrect.clear();
        self.ghost = None;
        let text = self.doc.text_str();
        let clamped = byte_idx.min(text.len());
        let span = if self.mode != TextMode::Password {
            sex_spell::word_span_at(text, clamped)
        } else {
            None
        };

        // Select/target the word under cursor cleanly unless user already made a multi-char selection covering it.
        if let Some(s) = span {
            let keep_existing = self
                .doc
                .selection()
                .is_some_and(|(a, b)| a <= s.start && s.end <= b && (b - a) > (s.end - s.start));
            if !keep_existing {
                self.doc.select(s.start, s.end);
            }
        } else if self.doc.selection().is_none() {
            self.doc.click_at(clamped, false);
        }

        let target_word = span.and_then(|s| self.doc.text_str().get(s.start..s.end).map(ToString::to_string));
        let mut items = Vec::new();
        let mut misspelled = false;
        let mut divider_after = None;

        if self.spellcheck_active() {
            if let (Some(s), Some(ref word)) = (span, &target_word) {
                if !self.spell.check_word(word) {
                    misspelled = true;
                    let mut sug_buf = [WordBuf::new(); 3];
                    let count = self.spell.suggest(word, &mut sug_buf);
                    for sug in sug_buf[..count].iter() {
                        items.push(ContextItem {
                            label: sug.as_str().to_string(),
                            shortcut: "",
                            enabled: !self.read_only,
                            prominent: true,
                            action: ContextAction::ReplaceTarget {
                                start: s.start,
                                end: s.end,
                                replacement: sug.as_str().to_string(),
                            },
                        });
                    }
                    items.push(ContextItem {
                        label: "Ignore Word".to_string(),
                        shortcut: "",
                        enabled: true,
                        prominent: false,
                        action: ContextAction::IgnoreWord(word.clone()),
                    });
                    items.push(ContextItem {
                        label: "Learn Spelling".to_string(),
                        shortcut: "",
                        enabled: true,
                        prominent: false,
                        action: ContextAction::LearnWord(word.clone()),
                    });
                    divider_after = Some(items.len() - 1);
                }
            }
        }

        let has_selection = self.doc.selection().is_some();
        let allow_copy = self.mode != TextMode::Password;

        items.push(ContextItem {
            label: "Cut".to_string(),
            shortcut: "⌘X",
            enabled: !self.read_only && allow_copy && has_selection,
            prominent: false,
            action: ContextAction::Cut,
        });
        items.push(ContextItem {
            label: "Copy".to_string(),
            shortcut: "⌘C",
            enabled: allow_copy && has_selection,
            prominent: false,
            action: ContextAction::Copy,
        });
        items.push(ContextItem {
            label: "Paste".to_string(),
            shortcut: "⌘V",
            enabled: !self.read_only && has_clipboard,
            prominent: false,
            action: ContextAction::Paste,
        });
        items.push(ContextItem {
            label: "Select All".to_string(),
            shortcut: "⌘A",
            enabled: !self.doc.is_empty(),
            prominent: false,
            action: ContextAction::SelectAll,
        });

        self.context_menu = Some(ContextMenu {
            target_span: span,
            target_word,
            misspelled,
            items,
            divider_after,
        });
        self.context_menu.as_ref().expect("just inserted")
    }

    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
    }

    /// Executes an action chosen from `context_menu` and closes the popup.
    pub fn execute_context_action(&mut self, action: ContextAction, clipboard: Option<&[u8]>) -> KeyOutcome {
        self.context_menu = None;
        self.autocorrect.clear();
        let mut out = KeyOutcome { handled: true, ..Default::default() };
        match action {
            ContextAction::ReplaceTarget { start, end, replacement } => {
                if !self.read_only && end <= self.doc.len() && start <= end {
                    self.doc.replace(start, end, replacement.as_bytes());
                    self.doc.seal();
                    out.text_changed = true;
                    self.refresh_analysis();
                }
            }
            ContextAction::IgnoreWord(w) => {
                self.spell.ignore_word(&w);
                self.refresh_analysis();
            }
            ContextAction::LearnWord(w) => {
                self.spell.learn_word(&w);
                self.refresh_analysis();
            }
            ContextAction::Cut => {
                if !self.read_only && self.mode != TextMode::Password {
                    let bytes = self.doc.cut();
                    if !bytes.is_empty() {
                        out.cut = Some(bytes);
                        out.text_changed = true;
                        self.refresh_analysis();
                    }
                }
            }
            ContextAction::Copy => {
                if self.mode != TextMode::Password && !self.doc.selected().is_empty() {
                    out.copied = Some(self.doc.selected().to_vec());
                }
            }
            ContextAction::Paste => {
                if !self.read_only {
                    if let Some(bytes) = clipboard {
                        out.text_changed = self.paste_bytes(bytes);
                    } else {
                        out.request_paste = true;
                    }
                }
            }
            ContextAction::SelectAll => {
                self.doc.select_all();
                self.refresh_analysis();
            }
        }
        out
    }

    pub fn paste_bytes(&mut self, bytes: &[u8]) -> bool {
        if self.read_only || bytes.is_empty() {
            return false;
        }
        self.autocorrect.clear();
        let filtered: Vec<u8> = if self.mode.is_multiline() {
            bytes.to_vec()
        } else {
            bytes.iter().copied().filter(|&b| b != b'\r' && b != b'\n').collect()
        };
        if filtered.is_empty() {
            return false;
        }
        self.doc.insert(&filtered);
        self.doc.seal();
        self.refresh_analysis();
        true
    }

    /// Processes a keyboard event with full macOS + PC shortcuts, prediction acceptance,
    /// and Space-bar autocorrect / 1-Backspace revert.
    pub fn handle_key(&mut self, key: Key, mods: Modifiers, clipboard: Option<&[u8]>) -> KeyOutcome {
        let mut out = KeyOutcome::default();

        if key == Key::Escape && self.context_menu.is_some() {
            self.context_menu = None;
            out.handled = true;
            return out;
        }

        // Primary shortcuts: accept BOTH Super (Cmd) AND Ctrl identically!
        if let Some(ch) = shortcut_letter(key) {
            if mods.primary_cmd() && !mods.alt {
                self.context_menu = None;
                return self.handle_primary_shortcut(ch, mods.shift, clipboard);
            }
        }

        // Accept inline ghost prediction with Tab or Right Arrow (no selection, unmodified, at word end).
        if !mods.shift && !mods.ctrl && !mods.alt && !mods.super_cmd && matches!(key, Key::Tab | Key::Right) {
            if self.accept_ghost_completion() {
                out.handled = true;
                out.text_changed = true;
                return out;
            }
        }

        match key {
            Key::Tab => {
                if !self.read_only && self.mode.is_multiline() && !mods.primary_cmd() && !mods.alt {
                    self.context_menu = None;
                    self.autocorrect.clear();
                    self.doc.insert(b"    ");
                    self.doc.seal();
                    out.handled = true;
                    out.text_changed = true;
                    self.refresh_analysis();
                }
            }
            Key::Enter => {
                self.context_menu = None;
                self.autocorrect.clear();
                self.suppress_autocorrect_at = None;
                if self.mode.is_multiline() && !mods.primary_cmd() && !self.read_only {
                    self.doc.newline();
                    out.handled = true;
                    out.text_changed = true;
                    self.refresh_analysis();
                } else if !self.mode.is_multiline() {
                    out.handled = true;
                    out.submitted = true;
                }
            }
            Key::Backspace => {
                self.context_menu = None;
                out.handled = true;
                if !self.read_only {
                    out.text_changed = self.handle_backspace(mods);
                }
            }
            Key::Delete => {
                self.context_menu = None;
                out.handled = true;
                if !self.read_only {
                    let before = self.doc.edits();
                    self.autocorrect.clear();
                    if mods.super_cmd {
                        self.doc.delete_to_line_end();
                    } else if mods.alt || mods.ctrl {
                        self.doc.delete_word_forward();
                    } else {
                        self.doc.delete();
                    }
                    out.text_changed = self.doc.edits() != before;
                    if out.text_changed {
                        self.refresh_analysis();
                    }
                }
            }
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown => {
                self.context_menu = None;
                self.autocorrect.clear();
                self.suppress_autocorrect_at = None;
                self.handle_navigation(key, mods);
                out.handled = true;
                self.refresh_analysis();
            }
            Key::Char(c) if !mods.primary_cmd() && !mods.alt && !c.is_control() => {
                self.context_menu = None;
                out.handled = true;
                if !self.read_only {
                    self.insert_typed_char(c);
                    out.text_changed = true;
                }
            }
            _ => {}
        }
        out
    }

    fn handle_primary_shortcut(&mut self, letter: u8, shift: bool, clipboard: Option<&[u8]>) -> KeyOutcome {
        let mut out = KeyOutcome::default();
        self.autocorrect.clear();
        self.suppress_autocorrect_at = None;
        match letter {
            b'a' => {
                self.doc.select_all();
                out.handled = true;
                self.refresh_analysis();
            }
            b'c' => {
                out.handled = true;
                if self.mode != TextMode::Password && !self.doc.selected().is_empty() {
                    out.copied = Some(self.doc.selected().to_vec());
                }
            }
            b'x' => {
                out.handled = true;
                if !self.read_only && self.mode != TextMode::Password {
                    let cut = self.doc.cut();
                    if !cut.is_empty() {
                        out.cut = Some(cut);
                        out.text_changed = true;
                        self.refresh_analysis();
                    }
                }
            }
            b'v' => {
                out.handled = true;
                if !self.read_only {
                    if let Some(bytes) = clipboard {
                        out.text_changed = self.paste_bytes(bytes);
                    } else {
                        out.request_paste = true;
                    }
                }
            }
            b'z' if !shift => {
                out.handled = true;
                if !self.read_only && self.doc.undo() {
                    out.text_changed = true;
                    self.refresh_analysis();
                }
            }
            b'z' | b'y' => {
                out.handled = true;
                if !self.read_only && self.doc.redo() {
                    out.text_changed = true;
                    self.refresh_analysis();
                }
            }
            _ => {}
        }
        out
    }

    fn handle_backspace(&mut self, mods: Modifiers) -> bool {
        // 1-Backspace revert for the most recent Space-bar autocorrect.
        if !mods.shift && !mods.alt && !mods.ctrl && !mods.super_cmd && self.doc.selection().is_none() {
            if self.autocorrect.should_revert_at(self.doc.cursor) {
                let word_start = self.autocorrect.word_start;
                let corrected_end = self.autocorrect.corrected_end;
                let orig = self.autocorrect.original;
                self.autocorrect.clear();
                if corrected_end <= self.doc.len() && word_start <= corrected_end {
                    self.doc.replace(word_start, corrected_end, orig.as_bytes());
                    // Leave cursor right after the preserved trailing Space.
                    let after_space = next_char(self.doc.text(), word_start + orig.len());
                    self.doc.click_at(after_space, false);
                    self.doc.seal();
                    self.suppress_autocorrect_at = Some(word_start);
                    self.refresh_analysis();
                    return true;
                }
            }
        }

        let before = self.doc.edits();
        self.autocorrect.clear();
        self.suppress_autocorrect_at = None;
        if mods.super_cmd {
            self.doc.delete_to_line_start();
        } else if mods.alt || mods.ctrl {
            self.doc.delete_word_back();
        } else {
            self.doc.backspace();
        }
        let changed = self.doc.edits() != before;
        if changed {
            self.refresh_analysis();
        }
        changed
    }

    fn handle_navigation(&mut self, key: Key, mods: Modifiers) {
        let extend = mods.shift;
        if mods.super_cmd {
            match key {
                Key::Left => self.doc.move_to(Motion::LineStart, extend),
                Key::Right => self.doc.move_to(Motion::LineEnd, extend),
                Key::Up | Key::Home => self.doc.move_to(Motion::Start, extend),
                Key::Down | Key::End => self.doc.move_to(Motion::End_, extend),
                _ => {}
            }
            return;
        }
        if mods.alt || mods.ctrl {
            match key {
                Key::Left => self.doc.move_to(Motion::WordLeft, extend),
                Key::Right => self.doc.move_to(Motion::WordRight, extend),
                Key::Home => self.doc.move_to(Motion::Start, extend),
                Key::End => self.doc.move_to(Motion::End_, extend),
                Key::Up => self.doc.move_to(Motion::Up, extend),
                Key::Down => self.doc.move_to(Motion::Down, extend),
                _ => {}
            }
            return;
        }
        match key {
            Key::Left => self.doc.move_to(Motion::Left, extend),
            Key::Right => self.doc.move_to(Motion::Right, extend),
            Key::Up => self.doc.move_to(Motion::Up, extend),
            Key::Down => self.doc.move_to(Motion::Down, extend),
            Key::Home => self.doc.move_to(Motion::Home, extend),
            Key::End => self.doc.move_to(Motion::End, extend),
            Key::PageUp => self.doc.move_to(Motion::PageUp(10), extend),
            Key::PageDown => self.doc.move_to(Motion::PageDown(10), extend),
            _ => {}
        }
    }

    fn insert_typed_char(&mut self, c: char) {
        if c == ' ' && self.autocorrect_active() && self.doc.selection().is_none() {
            if let Some((word_start, orig_word)) = word_immediately_before_cursor(self.doc.text_str(), self.doc.cursor) {
                let suppressed = self.suppress_autocorrect_at == Some(word_start);
                if !suppressed {
                    if let Some(corrected) = self.spell.autocorrect_on_space(orig_word) {
                        let orig_copy = WordBuf::from_str(orig_word);
                        let mut repl = Vec::with_capacity(corrected.len() + 1);
                        repl.extend_from_slice(corrected.as_bytes());
                        repl.push(b' ');
                        let cursor = self.doc.cursor;
                        self.doc.replace(word_start, cursor, &repl);
                        self.doc.seal();
                        if let Some(orig_buf) = orig_copy {
                            self.autocorrect.record(word_start, orig_buf.as_str(), corrected.as_str(), 1);
                        }
                        self.suppress_autocorrect_at = None;
                        self.refresh_analysis();
                        return;
                    }
                }
            }
        }

        self.autocorrect.clear();
        if c != ' ' {
            self.suppress_autocorrect_at = None;
        }
        let mut utf8 = [0u8; 4];
        self.doc.insert(c.encode_utf8(&mut utf8).as_bytes());
        self.refresh_analysis();
    }

    /// Recomputes cached misspelled spans and inline ghost completion after buffer/cursor changes.
    pub fn refresh_analysis(&mut self) {
        self.ghost = None;
        if self.prediction_active() && self.doc.selection().is_none() {
            if let Some((_, prefix)) = sex_spell::active_word_prefix_at(self.doc.text_str(), self.doc.cursor) {
                if let Some(suf) = self.spell.predict_completion(prefix) {
                    self.ghost = Some((self.doc.cursor, suf));
                }
            }
        }

        self.misspelled.clear();
        if !self.spellcheck_active() {
            return;
        }
        let text = self.doc.text_str();
        let active_ghost_cursor = self.ghost.as_ref().map(|(at, _)| *at);
        let spans = &mut self.misspelled;
        self.spell.scan_misspelled(text, self.mode, |span| {
            // Avoid flashing a red underline under an active word currently displaying ghost text.
            if active_ghost_cursor == Some(span.end) {
                return;
            }
            spans.push(span);
        });
    }

    /// Moves caret strictly within valid character boundaries (utility for custom renderers).
    pub fn clamp_cursor(&mut self) {
        let len = self.doc.len();
        if self.doc.cursor > len {
            let clamped = prev_char(self.doc.text(), len);
            self.doc.click_at(clamped, false);
        }
    }
}

fn shortcut_letter(key: Key) -> Option<u8> {
    match key {
        Key::Letter(b) if b.is_ascii_alphabetic() => Some(b.to_ascii_lowercase()),
        Key::Char(c) if c.is_ascii_alphabetic() => Some((c as u8).to_ascii_lowercase()),
        _ => None,
    }
}

/// Extracts an ASCII alphabetic word (`len >= 2`) immediately ending at `cursor` if not part of code/URL.
pub fn word_immediately_before_cursor(text: &str, cursor: usize) -> Option<(usize, &str)> {
    if cursor > text.len() || cursor < 2 || !text.is_char_boundary(cursor) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut start = cursor;
    while start > 0 && (bytes[start - 1].is_ascii_alphabetic() || bytes[start - 1] == b'\'') {
        start -= 1;
    }
    if cursor - start < 2 {
        return None;
    }
    if start > 0 && (bytes[start - 1].is_ascii_digit()
        || matches!(bytes[start - 1], b'_' | b'/' | b'\\' | b'.' | b'@' | b':'))
    {
        return None;
    }
    Some((start, &text[start..cursor]))
}
