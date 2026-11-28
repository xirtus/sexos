// SPDX-License-Identifier: GPL-3.0-only
//! Jot: macOS Notes Noir Rich Text Pad. Several files in tabs, an undo tree, find and
//! replace, line numbers, syntax colouring for Rust, C, JavaScript, Markdown and TOML,
//! Geist Sans/Mono rendering, spellcheck underlines & right-click suggestion popup.
#![no_std]
#![no_main]

extern crate alloc;

mod noir;
mod spell;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use noir::{
    draw_card, fill_rect, FaceKind, NoirFont, BG_BAR, BG_CANVAS, BG_CURRENT, BG_DOC, BG_GUTTER,
    BG_HIT, BG_PILL, BG_PILL_HOVER, BG_SELECTED, BORDER, DIM, FG, MISSPELT, MUTED, ROSE, ROSE_DIM,
};
use sex_app::{files, Event, Nav, Window};
use sex_pdx::app_pointer::{Kind as PKind, CONTROL, SHIFT};
use sex_textedit::encoding::{self, Eol, Format};
use sex_textedit::find::{self, Options};
use sex_textedit::highlight::{Highlighter, Kind, Lang};
use sex_textedit::markdown::{self, Style};
use sex_textedit::session::{self, Saved};
use sex_textedit::{wrap, Doc, Motion};
use sex_theme::candy_ui::fill_rounded_rect;

const FILE: &[u8] = b"notes.txt";
/// Bigger files open read-only, cut here: they belong in Quil.
const MAX_FILE: usize = 64 << 20;
const SESSION: &[u8] = b".jot-session";
/// How long after the last change the session is written.
const SESSION_MS: u64 = 800;

const TOOLBAR_H: usize = 38;
const TABBAR_H: usize = 30;
const EDITOR_PX: usize = 15;
const UI_PX: usize = 12;
const STATUS_H: usize = 26;
const BAR_H: usize = 30;
const DOC_MARGIN_X: usize = 8;

fn color(kind: Kind) -> u32 {
    match kind {
        Kind::Plain => FG,
        Kind::Keyword => 0xC084FC,
        Kind::Type => 0xFBBF24,
        Kind::String => 0x34D399,
        Kind::Number => 0xFB923C,
        Kind::Comment => 0x64748B,
        Kind::Punct => 0x94A3B8,
        Kind::Macro => 0x38BDF8,
        Kind::Heading => ROSE,
        Kind::Emphasis => 0xF472B6,
        Kind::Code => 0x2DD4BF,
        Kind::Link => 0x38BDF8,
        Kind::Key => ROSE,
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[jot.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}
fn base_name(path: &[u8]) -> &[u8] { path.rsplit(|b| *b == b'/').next().unwrap_or(path) }

fn count_words(text: &[u8]) -> usize {
    let mut words = 0usize;
    let mut in_word = false;
    for &b in text {
        if b.is_ascii_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
        }
    }
    words
}

struct Tab {
    doc: Doc,
    path: Vec<u8>,
    hl: Highlighter,
    /// First line and first column shown.
    top: usize,
    left: usize,
    /// With wrap on: rows of the top line scrolled past.
    skip: usize,
    /// How the file is stored on disk; the text in `doc` is always UTF-8 with `\n`.
    format: Format,
    /// The format was changed and not yet saved.
    format_changed: bool,
    /// Too big to hold whole: shown cut, never saved over the file.
    readonly: bool,
    /// Markdown preview instead of the text, and its first row.
    preview: bool,
    preview_top: usize,
}
impl Tab {
    fn new(path: &[u8], text: Vec<u8>) -> Tab {
        let mut doc = Doc::new(text);
        doc.move_to(Motion::End_, false);
        Tab {
            doc,
            hl: Highlighter::new(Lang::from_name(path)),
            path: path.to_vec(),
            top: 0,
            left: 0,
            skip: 0,
            format: Format::default(),
            format_changed: false,
            readonly: false,
            preview: false,
            preview_top: 0,
        }
    }
    fn name(&self) -> String { String::from_utf8_lossy(base_name(&self.path)).into_owned() }
    fn unsaved(&self) -> bool { self.doc.modified() || self.format_changed }
}

/// One screen row of text: its line number and byte range in the document.
#[derive(Clone, Copy)]
struct Row {
    line: usize,
    start: usize,
    end: usize,
    /// Last row of its line.
    last: bool,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Bar {
    None,
    Find,
    Replace,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitId {
    ToolNew,
    ToolOpen,
    ToolSave,
    ToolFind,
    ToolSpell,
    ToolPreview,
    Tab(usize),
    TabAdd,
    PopupItem(usize),
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: HitId,
}

#[derive(Clone)]
enum PopupAction {
    SpellFix { start: usize, end: usize, from: String, to: String, others: usize },
    Cut,
    Copy,
    Paste,
    SelectAll,
}

struct PopupItem {
    label: String,
    action: PopupAction,
    accent: bool,
}

struct PopupMenu {
    x: i32,
    y: i32,
    w: i32,
    items: Vec<PopupItem>,
}

struct Jot {
    tabs: Vec<Tab>,
    active: usize,
    font: NoirFont,
    cell_w: usize,
    cell_h: usize,
    width: usize,
    height: usize,
    numbers: bool,
    bar: Bar,
    /// Which field of the bar has the keys: 0 find, 1 replace with.
    field: usize,
    needle: Vec<u8>,
    with: Vec<u8>,
    status: String,
    clipboard: Vec<u8>,
    spell: spell::Checker,
    spelling: bool,
    /// The wheel moved the view away from the cursor; the next key brings it back.
    scrolled: bool,
    /// The bar just opened: the first character typed replaces the field, as if selected.
    fresh: bool,
    /// Long lines wrap at the window's edge.
    wrap: bool,
    /// When unsaved work last changed (ticks), until the session is written.
    dirty_at: Option<u64>,
    seen_edits: u64,
    hits: Vec<HitBox>,
    hover: Option<HitId>,
    popup: Option<PopupMenu>,
}

/// What a key or click asked for that needs the window.
enum Ask {
    Nothing,
    Save,
    SaveAs,
    Open,
    Close,
    Escape,
    Copy,
    Paste,
}

impl Jot {
    fn tab(&mut self) -> &mut Tab { &mut self.tabs[self.active] }
    fn doc_top(&self) -> usize { TOOLBAR_H + TABBAR_H + 6 }
    fn bottom_h(&self) -> usize { STATUS_H + if self.bar != Bar::None { BAR_H } else { 0 } + 6 }
    fn rows(&self) -> usize {
        self.height.saturating_sub(self.doc_top() + self.bottom_h() + 8) / self.cell_h.max(1)
    }
    fn rows_clamped(&self) -> usize { self.rows().max(1) }
    fn gutter(&self) -> usize {
        if self.numbers {
            self.tabs[self.active].doc.line_count().to_string_len().max(3) + 2
        } else {
            1
        }
    }
    fn doc_x0(&self) -> usize { DOC_MARGIN_X + 2 + self.gutter() * self.cell_w }
    fn cols(&self) -> usize {
        self.width.saturating_sub(self.doc_x0() + DOC_MARGIN_X + 8) / self.cell_w.max(1)
    }
    fn cols_clamped(&self) -> usize { self.cols().max(4) }

    /// The screen rows from the tab's first shown line: whole lines, or wrapped pieces.
    fn layout(&self, t: &Tab, rows: usize) -> Vec<Row> {
        let text = t.doc.text();
        let cols = self.cols_clamped();
        let mut out = Vec::new();
        let mut start = t.doc.offset_of_line(t.top);
        let mut line = t.top;
        while out.len() < rows && line < t.doc.line_count() {
            let end = t.doc.line_end(start);
            if self.wrap {
                let parts = wrap::rows(&text[start..end], cols);
                let n = parts.len();
                let skip = if line == t.top { t.skip.min(n - 1) } else { 0 };
                for (i, (a, b)) in parts.into_iter().enumerate().skip(skip) {
                    out.push(Row { line, start: start + a, end: start + b, last: i + 1 == n });
                }
            } else {
                out.push(Row { line, start, end, last: true });
            }
            start = (end + 1).min(text.len());
            line += 1;
        }
        out.truncate(rows);
        out
    }

    fn rows_in(&self, t: &Tab, line: usize) -> usize {
        let start = t.doc.offset_of_line(line);
        wrap::rows(&t.doc.text()[start..t.doc.line_end(start)], self.cols_clamped()).len()
    }

    fn scroll_rows(&mut self, by: isize) {
        let i = self.active;
        if !self.wrap {
            let t = &mut self.tabs[i];
            t.top = (t.top as isize + by).clamp(0, t.doc.line_count().saturating_sub(1) as isize) as usize;
            t.skip = 0;
            return;
        }
        for _ in 0..by.unsigned_abs() {
            let t = &self.tabs[i];
            if by > 0 {
                if t.skip + 1 < self.rows_in(t, t.top) {
                    self.tabs[i].skip += 1;
                } else if t.top + 1 < t.doc.line_count() {
                    let t = &mut self.tabs[i];
                    t.top += 1;
                    t.skip = 0;
                } else {
                    break;
                }
            } else if t.skip > 0 {
                self.tabs[i].skip -= 1;
            } else if t.top > 0 {
                let n = self.rows_in(t, t.top - 1);
                let t = &mut self.tabs[i];
                t.top -= 1;
                t.skip = n - 1;
            } else {
                break;
            }
        }
    }

    fn follow(&mut self) {
        let (rows, cols) = (self.rows_clamped(), self.cols_clamped());
        let wrapped = self.wrap;
        let t = &mut self.tabs[self.active];
        let line = t.doc.line_of(t.doc.cursor);
        if line < t.top {
            t.top = line;
            t.skip = 0;
        }
        if line >= t.top + rows {
            t.top = line + 1 - rows;
            t.skip = 0;
        }
        if wrapped {
            t.left = 0;
            if line == t.top {
                let start = t.doc.line_start(t.doc.cursor);
                let parts = wrap::rows(&t.doc.text()[start..t.doc.line_end(start)], cols);
                let row = wrap::row_of(&parts, t.doc.cursor - start);
                if row < t.skip {
                    t.skip = row;
                }
            }
            loop {
                let t = &self.tabs[self.active];
                let lay = self.layout(t, rows);
                let c = t.doc.cursor;
                let shown = lay.iter().any(|r| c >= r.start && (c < r.end || (c == r.end && r.last)));
                if shown || lay.is_empty() {
                    break;
                }
                let was = (t.top, t.skip);
                self.scroll_rows(1);
                if was == (self.tabs[self.active].top, self.tabs[self.active].skip) {
                    break;
                }
            }
            return;
        }
        t.skip = 0;
        let col = display_col(t.doc.text(), t.doc.line_start(t.doc.cursor), t.doc.cursor);
        if col < t.left {
            t.left = col.saturating_sub(8);
        }
        if col >= t.left + cols {
            t.left = col + 8 - cols;
        }
    }

    fn draw_pill(&mut self, buf: &mut [u32], x: i32, y: i32, h: i32, label: &str, active: bool, id: HitId) -> i32 {
        let tw = self.font.measure(FaceKind::SansBold, UI_PX, label);
        let pw = (tw + 20).max(46);
        let fill = if active {
            ROSE
        } else if self.hover == Some(id) {
            BG_PILL_HOVER
        } else {
            BG_PILL
        };
        let border = if active { ROSE } else { BORDER };
        draw_card(buf, self.width, self.height, x, y, pw, h, 7, fill, border);
        self.font.draw_text(buf, self.width, self.height, FaceKind::SansBold, x + 10, y + 5, pw - 14, UI_PX, label, FG);
        self.hits.push(HitBox { x, y, w: pw, h, id });
        x + pw + 6
    }

    fn draw(&mut self, window: &mut Window) {
        if !self.scrolled {
            self.follow();
        }
        self.hits.clear();
        let (w, h) = (self.width, self.height);
        let wi = w as i32;
        let hi = h as i32;
        let pixels = window.frame(w, h);
        pixels.fill(BG_CANVAS);

        // 1. Frosted Top Toolbar (New, Open, Save, Find, Spell, Preview + word/line count)
        fill_rect(pixels, w, h, 0, 0, wi, TOOLBAR_H as i32, BG_BAR);
        fill_rect(pixels, w, h, 0, TOOLBAR_H as i32 - 1, wi, 1, BORDER);

        fill_rounded_rect(pixels, w, h, 10, 7, 46, 24, 7, ROSE);
        self.font.draw_text(pixels, w, h, FaceKind::SansBold, 20, 11, 36, UI_PX, "JOT", FG);

        let mut tx = 66i32;
        tx = self.draw_pill(pixels, tx, 7, 24, "New", false, HitId::ToolNew);
        tx = self.draw_pill(pixels, tx, 7, 24, "Open", false, HitId::ToolOpen);
        tx = self.draw_pill(pixels, tx, 7, 24, "Save", self.tabs[self.active].unsaved(), HitId::ToolSave);
        tx = self.draw_pill(pixels, tx, 7, 24, "Find", self.bar != Bar::None, HitId::ToolFind);
        let spell_lbl = if self.spelling { "Spell ON" } else { "Spell OFF" };
        tx = self.draw_pill(pixels, tx, 7, 24, spell_lbl, self.spelling, HitId::ToolSpell);
        let _ = self.draw_pill(pixels, tx, 7, 24, "Preview", self.tabs[self.active].preview, HitId::ToolPreview);

        let t_ref = &self.tabs[self.active];
        let words = count_words(t_ref.doc.text());
        let lines = t_ref.doc.line_count();
        let counts = format!("{words}w · {lines}L");
        let cw_badge = self.font.measure(FaceKind::Sans, 11, &counts) + 18;
        if wi - cw_badge - 10 > tx + 4 {
            let bx = wi - cw_badge - 10;
            draw_card(pixels, w, h, bx, 8, cw_badge, 22, 7, BG_DOC, BORDER);
            self.font.draw_text(pixels, w, h, FaceKind::Sans, bx + 9, 12, cw_badge - 12, 11, &counts, DIM);
        }

        // 2. Tab Strip
        let tab_y = TOOLBAR_H as i32;
        fill_rect(pixels, w, h, 0, tab_y, wi, TABBAR_H as i32, BG_CANVAS);
        fill_rect(pixels, w, h, 0, tab_y + TABBAR_H as i32 - 1, wi, 1, BORDER);
        let mut tab_x = 10i32;
        for i in 0..self.tabs.len() {
            let t = &self.tabs[i];
            let label = format!(
                "{}{}{}",
                t.name(),
                if t.unsaved() { " •" } else { "" },
                if t.readonly { " (RO)" } else { "" }
            );
            let tw = self.font.measure(FaceKind::SansBold, 11, &label) + 22;
            if tab_x + tw + 36 > wi {
                break;
            }
            let active = i == self.active;
            let fill = if active {
                BG_DOC
            } else if self.hover == Some(HitId::Tab(i)) {
                BG_PILL_HOVER
            } else {
                BG_BAR
            };
            draw_card(pixels, w, h, tab_x, tab_y + 4, tw, 23, 6, fill, if active { ROSE } else { BORDER });
            if active {
                fill_rect(pixels, w, h, tab_x + 6, tab_y + 25, tw - 12, 2, ROSE);
            }
            self.font.draw_text(pixels, w, h, FaceKind::SansBold, tab_x + 11, tab_y + 8, tw - 16, 11, &label, if active { FG } else { DIM });
            self.hits.push(HitBox { x: tab_x, y: tab_y + 4, w: tw, h: 23, id: HitId::Tab(i) });
            tab_x += tw + 6;
        }
        draw_card(pixels, w, h, tab_x, tab_y + 4, 24, 23, 6, if self.hover == Some(HitId::TabAdd) { BG_PILL_HOVER } else { BG_PILL }, BORDER);
        self.font.draw_text(pixels, w, h, FaceKind::SansBold, tab_x + 8, tab_y + 7, 14, 12, "+", FG);
        self.hits.push(HitBox { x: tab_x, y: tab_y + 4, w: 24, h: 23, id: HitId::TabAdd });

        // 3. Document Canvas Card
        let doc_top = self.doc_top() as i32;
        let doc_h = (hi - doc_top - self.bottom_h() as i32).max(24);
        let doc_w = (wi - DOC_MARGIN_X as i32 * 2).max(40);
        draw_card(pixels, w, h, DOC_MARGIN_X as i32, doc_top, doc_w, doc_h, 10, BG_DOC, BORDER);

        let (cw, ch) = (self.cell_w, self.cell_h);
        let (rows, cols, gutter) = (self.rows_clamped(), self.cols_clamped(), self.gutter());
        let text_y0 = doc_top as usize + 4;
        let gutter_x = DOC_MARGIN_X + 2;
        let gutter_px = gutter * cw;
        let text_x0 = gutter_x + gutter_px;

        if self.tabs[self.active].preview {
            self.draw_preview(pixels, rows, cols, gutter, text_y0, text_x0);
        } else {
            if self.numbers && gutter_px > 2 {
                fill_rect(pixels, w, h, gutter_x as i32, doc_top + 2, gutter_px as i32 - 4, doc_h - 4, BG_GUTTER);
                fill_rect(pixels, w, h, (gutter_x + gutter_px - 4) as i32, doc_top + 2, 1, doc_h - 4, BORDER);
            }
            let layout = { let t = &self.tabs[self.active]; self.layout(t, rows) };
            let t = &mut self.tabs[self.active];
            let text = t.doc.text();
            let spans = t.hl.visible(text, t.top, rows);
            let cursor_line = t.doc.line_of(t.doc.cursor);
            let sel = t.doc.selection();
            let hits: Vec<(usize, usize)> = if self.bar != Bar::None && !self.needle.is_empty() {
                find::all(text, &self.needle, Options { ignore_case: true, whole_word: false })
            } else {
                Vec::new()
            };
            let prose = matches!(t.hl.lang, Lang::Plain | Lang::Markdown);
            let mut misspelt: Vec<(usize, usize)> = Vec::new();
            let mut misspelt_line = usize::MAX;

            for (r, row) in layout.iter().enumerate() {
                let n = row.line;
                let line_start = t.doc.line_start(row.start);
                let y0 = text_y0 + r * ch;
                if y0 + ch > (doc_top + doc_h - 2) as usize {
                    break;
                }
                let line_bg = if n == cursor_line { BG_CURRENT } else { BG_DOC };
                if line_bg != BG_DOC {
                    let rw_fill = (w - text_x0 - DOC_MARGIN_X - 2) as i32;
                    fill_rect(pixels, w, h, text_x0 as i32, y0 as i32, rw_fill, ch as i32, line_bg);
                }
                if self.numbers && row.start == line_start {
                    let num = format!("{:>width$}", n + 1, width = gutter - 2);
                    self.font.draw_text(
                        pixels, w, h, FaceKind::Mono,
                        (gutter_x + 4) as i32, y0 as i32, gutter_px as i32 - 8,
                        13, &num, if n == cursor_line { ROSE } else { MUTED },
                    );
                }
                if misspelt_line != n {
                    misspelt_line = n;
                    misspelt = if self.spelling && prose {
                        let e = t.doc.line_end(line_start);
                        self.spell.misspelt(&text[line_start..e])
                    } else {
                        Vec::new()
                    };
                }
                let line_spans = spans.get(n - t.top).cloned().unwrap_or_default();
                let mut col = if self.wrap { 0 } else { display_col(text, line_start, row.start) };
                let mut at = row.start;
                while at < row.end {
                    let c_end = sex_textedit::next_char(text, at);
                    let c = core::str::from_utf8(&text[at..c_end]).ok().and_then(|s| s.chars().next()).unwrap_or('?');
                    let char_w = if c == '\t' { 4 - col % 4 } else { 1 };
                    let rel = at - line_start;
                    let fg = line_spans.iter().find(|s| s.start <= rel && rel < s.end).map_or(FG, |s| color(s.kind));
                    let mut bg = line_bg;
                    if hits.iter().any(|h| h.0 <= at && at < h.1) { bg = BG_HIT; }
                    if sel.is_some_and(|(a, b)| a <= at && at < b) { bg = BG_SELECTED; }
                    for k in 0..char_w {
                        let vis = col + k;
                        if vis >= t.left && vis < t.left + cols {
                            let x0 = text_x0 + (vis - t.left) * cw;
                            self.font.draw_glyph_mono(
                                pixels, w, h, x0 as i32, y0 as i32, EDITOR_PX,
                                if c == '\t' || k > 0 { ' ' } else { c },
                                fg, bg,
                            );
                            if misspelt.iter().any(|m| m.0 <= rel && rel < m.1) {
                                // Crisp wavy/dotted SexOS Rose spellcheck underline
                                for dx in 0..cw {
                                    let dy = if (dx / 2) % 2 == 0 { ch - 2 } else { ch - 1 };
                                    if y0 + dy < h && x0 + dx < w {
                                        pixels[(y0 + dy) * w + x0 + dx] = MISSPELT;
                                    }
                                }
                            }
                        }
                    }
                    col += char_w;
                    at = c_end;
                }
                // Visible Rose caret
                let cur = t.doc.cursor;
                if cur >= row.start && (cur < row.end || (cur == row.end && row.last)) {
                    let vis = if self.wrap { display_col(text, row.start, cur) } else { display_col(text, line_start, cur) };
                    if vis >= t.left && vis <= t.left + cols {
                        let x0 = text_x0 + (vis - t.left) * cw;
                        for y in 2..ch.saturating_sub(2) {
                            if x0 + 1 < w && y0 + y < h {
                                pixels[(y0 + y) * w + x0] = ROSE;
                                pixels[(y0 + y) * w + x0 + 1] = ROSE;
                            }
                        }
                    }
                }
            }
        }

        // 4. Find / Replace Bar
        let mut by = hi - STATUS_H as i32;
        if self.bar != Bar::None {
            by -= BAR_H as i32;
            fill_rect(pixels, w, h, 0, by, wi, BAR_H as i32, BG_BAR);
            fill_rect(pixels, w, h, 0, by, wi, 1, BORDER);
            let find = format!("Find: {}{}", String::from_utf8_lossy(&self.needle), if self.field == 0 { "|" } else { "" });
            let mut fx = self.font.draw_text(pixels, w, h, FaceKind::Mono, 14, by + 7, wi - 30, 13, &find, FG);
            if self.bar == Bar::Replace {
                let with = format!("   Replace: {}{}", String::from_utf8_lossy(&self.with), if self.field == 1 { "|" } else { "" });
                fx = self.font.draw_text(pixels, w, h, FaceKind::Mono, fx, by + 7, wi - fx - 12, 13, &with, FG);
            }
            let help = if self.bar == Bar::Replace {
                "   Up/Down field · Enter replace · Ctrl+A all · Esc"
            } else {
                "   Enter next · Shift+Enter back · Esc"
            };
            self.font.draw_text(pixels, w, h, FaceKind::Sans, fx + 4, by + 8, wi - fx - 12, 11, help, MUTED);
        }

        // 5. Status Bar
        let sy = hi - STATUS_H as i32;
        fill_rect(pixels, w, h, 0, sy, wi, STATUS_H as i32, BG_BAR);
        fill_rect(pixels, w, h, 0, sy, wi, 1, BORDER);
        let t = &self.tabs[self.active];
        let line = t.doc.line_of(t.doc.cursor) + 1;
        let col = t.doc.column(t.doc.cursor) + 1;
        let right = format!(
            "Ln {}, Col {}  ·  {}  ·  {}  ·  {}  ·  {}{} ",
            line, col, t.hl.lang.name(), t.format.encoding.name(), t.format.eol.name(),
            if self.wrap { "Wrap  ·  " } else { "" },
            String::from_utf8_lossy(&t.path)
        );
        let rm = self.font.measure(FaceKind::Sans, 11, &right);
        let rx = (wi - rm - 12).max(160);
        self.font.draw_text(pixels, w, h, FaceKind::Sans, 12, sy + 6, (rx - 20).max(40), 11, &self.status, FG);
        self.font.draw_text(pixels, w, h, FaceKind::Sans, rx, sy + 6, wi - rx - 6, 11, &right, DIM);

        // 6. Right-click / Ctrl+click Context Popup Menu
        if let Some(menu) = &self.popup {
            let row_h = 26i32;
            let mh = menu.items.len() as i32 * row_h + 10;
            let mx = menu.x.min((wi - menu.w - 8).max(4)).max(4);
            let my = menu.y.min((hi - mh - 8).max(4)).max(4);
            draw_card(pixels, w, h, mx, my, menu.w, mh, 9, BG_BAR, ROSE);
            for (idx, item) in menu.items.iter().enumerate() {
                let iy = my + 5 + idx as i32 * row_h;
                let hid = HitId::PopupItem(idx);
                if self.hover == Some(hid) {
                    fill_rounded_rect(pixels, w, h, mx + 5, iy, menu.w - 10, row_h - 2, 6, ROSE_DIM);
                }
                let col = if item.accent { ROSE } else { FG };
                self.font.draw_text(pixels, w, h, FaceKind::SansBold, mx + 12, iy + 5, menu.w - 24, 12, &item.label, col);
                self.hits.push(HitBox { x: mx + 4, y: iy, w: menu.w - 8, h: row_h, id: hid });
            }
        }

        window.show_frame(w, h);
    }

    fn key(&mut self, key: sex_pdx::app_key::Key) -> Ask {
        let (ctrl, shift, code) = (key.control, key.shift, key.code);
        let page = self.rows_clamped().saturating_sub(1).max(1);
        if self.popup.is_some() && code == 0x01 {
            self.popup = None;
            return Ask::Nothing;
        }
        self.popup = None;
        if self.bar != Bar::None && self.bar_key(key) {
            return Ask::Nothing;
        }
        if self.tabs[self.active].preview {
            let t = &mut self.tabs[self.active];
            match (ctrl, code, sex_app::nav(key)) {
                (true, 0x32, _) | (_, 0x01, _) => { self.toggle_preview(); }
                (_, _, Some(Nav::Up)) => t.preview_top = t.preview_top.saturating_sub(1),
                (_, _, Some(Nav::Down)) => t.preview_top += 1,
                (false, 0x49 | 0x68, _) => t.preview_top = t.preview_top.saturating_sub(page),
                (false, 0x51 | 0x6D, _) => t.preview_top += page,
                (_, _, Some(Nav::Home)) => t.preview_top = 0,
                (true, 0x1F, _) => return if shift { Ask::SaveAs } else { Ask::Save },
                (true, 0x11, _) => return Ask::Close,
                (true, 0x49 | 0x68, _) => { self.active = (self.active + self.tabs.len() - 1) % self.tabs.len(); }
                (true, 0x51 | 0x6D, _) => { self.active = (self.active + 1) % self.tabs.len(); }
                _ => {}
            }
            return Ask::Nothing;
        }
        self.status.clear();
        let nav = sex_app::nav(key);
        let tab = &mut self.tabs[self.active];
        let before = tab.doc.line_of(tab.doc.cursor);
        let len_before = tab.doc.len();
        let mut then = Ask::Nothing;
        match (ctrl, code, nav) {
            (true, 0x1F, _) => return if shift { Ask::SaveAs } else { Ask::Save },
            (true, 0x18, _) => return Ask::Open,
            (true, 0x11, _) => return Ask::Close,
            (true, 0x12, _) => return Ask::Escape,
            (true, 0x31, _) => {
                let name = untitled(&self.tabs);
                self.tabs.push(Tab::new(&name, Vec::new()));
                self.active = self.tabs.len() - 1;
                self.tab_record();
                return Ask::Nothing;
            }
            (true, 0x49 | 0x68, _) => {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                self.tab_record();
                return Ask::Nothing;
            }
            (true, 0x51 | 0x6D, _) => {
                self.active = (self.active + 1) % self.tabs.len();
                self.tab_record();
                return Ask::Nothing;
            }
            (true, 0x2C, _) if shift => { if tab.doc.redo() { self.status = "Redone.".into(); } }
            (true, 0x2C, _) => {
                if tab.doc.undo() {
                    sex_pdx::serial_record!("[jot.undo] depth={} bytes={}", tab.doc.undo_tree().depth(), tab.doc.len());
                } else {
                    self.status = "Nothing to undo.".into();
                }
            }
            (true, 0x15, _) => {
                if tab.doc.redo() {
                    sex_pdx::serial_record!("[jot.redo] depth={} bytes={}", tab.doc.undo_tree().depth(), tab.doc.len());
                } else {
                    self.status = "Nothing to redo.".into();
                }
            }
            (true, 0x30, _) => {
                self.status = if tab.doc.switch_branch() {
                    "Redo now takes the other branch.".into()
                } else {
                    "No other branch here.".into()
                };
            }
            (true, 0x1E, _) => tab.doc.select_all(),
            (true, 0x2E, _) => {
                self.clipboard = tab.doc.selected().to_vec();
                self.status = format!("Copied {} bytes.", self.clipboard.len());
                then = Ask::Copy;
            }
            (true, 0x2D, _) => { self.clipboard = tab.doc.cut(); then = Ask::Copy; }
            (true, 0x2F, _) => return Ask::Paste,
            (true, 0x26, _) => self.numbers = !self.numbers,
            (true, 0x13, _) => {
                self.wrap = !self.wrap;
                if self.wrap { self.tabs[self.active].left = 0; }
                let row_count = { let t = &self.tabs[self.active]; self.layout(t, self.rows_clamped()).len() };
                sex_pdx::serial_record!("[jot.wrap] on={} rows={}", self.wrap as u8, row_count);
                self.status = if self.wrap { "Long lines wrap.".into() } else { "Long lines scroll.".into() };
                return Ask::Nothing;
            }
            (true, 0x32, _) => { self.toggle_preview(); return Ask::Nothing; }
            (true, 0x14, _) => {
                let t = &mut self.tabs[self.active];
                if shift { t.format.encoding = t.format.encoding.next(); } else { t.format.eol = if t.format.eol == Eol::Lf { Eol::CrLf } else { Eol::Lf }; }
                t.format_changed = true;
                sex_pdx::serial_record!("[jot.format] encoding={} eol={}", t.format.encoding.name(), t.format.eol.name());
                self.status = format!("Saves as {}, {} line ends.", t.format.encoding.name(), t.format.eol.name());
                self.dirty_at = Some(sex_pdx::get_ticks());
                return Ask::Nothing;
            }
            (true, 0x25, _) => {
                self.spelling = !self.spelling;
                self.status = if self.spelling { "Spelling marked.".into() } else { "Spelling off.".into() };
            }
            (true, 0x21, _) => { self.bar = Bar::Find; self.field = 0; self.prefill(); self.fresh = true; }
            (true, 0x23, _) => { self.bar = Bar::Replace; self.field = 0; self.prefill(); self.fresh = true; }
            (true, 0x22, _) => { self.find(shift); }
            (true, 0x34, _) => { self.fix_spelling(); return Ask::Nothing; }
            (_, 0x01, _) => { self.tabs[self.active].doc.anchor = None; }
            (true, _, Some(Nav::Left)) => tab.doc.move_to(Motion::WordLeft, shift),
            (true, _, Some(Nav::Right)) => tab.doc.move_to(Motion::WordRight, shift),
            (true, _, Some(Nav::Home)) => tab.doc.move_to(Motion::Start, shift),
            (true, _, Some(Nav::End)) => tab.doc.move_to(Motion::End_, shift),
            (_, _, Some(Nav::Left)) => tab.doc.move_to(Motion::Left, shift),
            (_, _, Some(Nav::Right)) => tab.doc.move_to(Motion::Right, shift),
            (_, _, Some(Nav::Up)) => tab.doc.move_to(Motion::Up, shift),
            (_, _, Some(Nav::Down)) => tab.doc.move_to(Motion::Down, shift),
            (_, _, Some(Nav::Home)) => tab.doc.move_to(Motion::Home, shift),
            (_, _, Some(Nav::End)) => tab.doc.move_to(Motion::End, shift),
            (_, _, Some(Nav::Delete)) => tab.doc.delete(),
            (false, 0x49 | 0x68, _) => tab.doc.move_to(Motion::PageUp(page), shift),
            (false, 0x51 | 0x6D, _) => tab.doc.move_to(Motion::PageDown(page), shift),
            (_, 0x1C, _) => tab.doc.newline(),
            (_, 0x0E, _) => tab.doc.backspace(),
            (false, 0x0F, _) => tab.doc.insert(b"    "),
            (false, ..) => match sex_app::text(key) {
                Some(c) => tab.doc.insert(c.encode_utf8(&mut [0; 4]).as_bytes()),
                None => return Ask::Nothing,
            },
            _ => return Ask::Nothing,
        }
        let tab = &mut self.tabs[self.active];
        if tab.doc.len() != len_before || ctrl {
            let from = before.min(tab.doc.line_of(tab.doc.cursor));
            tab.hl.invalidate(from.saturating_sub(1));
        }
        then
    }

    fn fix_spelling(&mut self) {
        let tab = &mut self.tabs[self.active];
        let start = tab.doc.line_start(tab.doc.cursor);
        let end = tab.doc.line_end(start);
        let line = tab.doc.text()[start..end].to_vec();
        let rel = tab.doc.cursor - start;
        let Some(&(a, b)) = spell::words(&line).iter().rev().find(|(a, _)| *a <= rel) else {
            self.status = "No word here.".into();
            return;
        };
        let word = String::from_utf8_lossy(&line[a..b]).into_owned();
        if self.spell.correct(&word) {
            self.status = format!("\"{}\" is spelt right.", word);
            return;
        }
        let suggestions = self.spell.suggest(&word);
        let Some(best) = suggestions.first().cloned() else {
            self.status = format!("No suggestions for \"{}\".", word);
            return;
        };
        let tab = &mut self.tabs[self.active];
        tab.doc.seal();
        tab.doc.replace(start + a, start + b, best.as_bytes());
        tab.doc.seal();
        tab.hl.invalidate(0);
        sex_pdx::serial_record!("[jot.spell.fixed] from={} to={} others={}", word, best, suggestions.len() - 1);
        self.status = format!(
            "{} -> {}   (also: {})",
            word, best, suggestions[1..].iter().take(4).cloned().collect::<Vec<_>>().join(", ")
        );
    }

    fn pointer_to_offset(&self, px: i32, py: i32) -> Option<usize> {
        let rows = self.rows_clamped();
        let text_y0 = self.doc_top() as i32 + 4;
        if py < text_y0 {
            return None;
        }
        let row = ((py - text_y0) as usize) / self.cell_h;
        if row >= rows {
            return None;
        }
        let t = &self.tabs[self.active];
        if t.preview {
            return None;
        }
        let lay = self.layout(t, rows);
        let r = lay.get(row).or(lay.last())?;
        let text_x0 = self.doc_x0() as i32;
        let want = ((px - text_x0).max(0) as usize) / self.cell_w + if self.wrap { 0 } else { t.left };
        let end = if r.last { r.end } else { r.end.saturating_sub(1).max(r.start) };
        let mut at = r.start;
        while at < end && display_col(t.doc.text(), r.start, sex_textedit::next_char(t.doc.text(), at)) <= want {
            at = sex_textedit::next_char(t.doc.text(), at);
        }
        Some(at)
    }

    fn open_context_menu(&mut self, px: i32, py: i32) {
        let mut items = Vec::new();
        if let Some(at) = self.pointer_to_offset(px, py) {
            let tab = &mut self.tabs[self.active];
            tab.doc.move_to(Motion::To(at), false);
            let prose = matches!(tab.hl.lang, Lang::Plain | Lang::Markdown);
            if self.spelling && prose {
                let line_start = tab.doc.line_start(at);
                let line_end = tab.doc.line_end(line_start);
                let line = tab.doc.text()[line_start..line_end].to_vec();
                let rel = at - line_start;
                if let Some(&(a, b)) = spell::words(&line).iter().find(|&&(a, b)| rel >= a && rel <= b) {
                    let word = String::from_utf8_lossy(&line[a..b]).into_owned();
                    if !self.spell.correct(&word) {
                        let suggestions = self.spell.suggest(&word);
                        let total = suggestions.len();
                        for s in suggestions.into_iter().take(4) {
                            items.push(PopupItem {
                                label: format!("Fix: {s}"),
                                action: PopupAction::SpellFix {
                                    start: line_start + a,
                                    end: line_start + b,
                                    from: word.clone(),
                                    to: s,
                                    others: total.saturating_sub(1),
                                },
                                accent: true,
                            });
                        }
                    }
                }
            }
        }
        items.push(PopupItem { label: "Cut".into(), action: PopupAction::Cut, accent: false });
        items.push(PopupItem { label: "Copy".into(), action: PopupAction::Copy, accent: false });
        items.push(PopupItem { label: "Paste".into(), action: PopupAction::Paste, accent: false });
        items.push(PopupItem { label: "Select All".into(), action: PopupAction::SelectAll, accent: false });
        self.popup = Some(PopupMenu { x: px, y: py, w: 176, items });
    }

    fn toggle_preview(&mut self) {
        let cols = self.cols_clamped();
        let t = &mut self.tabs[self.active];
        t.preview = !t.preview;
        if t.preview {
            let blocks = markdown::parse(&String::from_utf8_lossy(t.doc.text()));
            let line = t.doc.line_of(t.doc.cursor);
            let rows = preview_rows(&blocks, cols);
            t.preview_top = rows.iter().position(|r| r.source >= line).unwrap_or(0);
            if blocks.iter().all(|b| b.line > line) {
                t.preview_top = 0;
            }
            sex_pdx::serial_record!(
                "[jot.preview] blocks={} rows={} headings={}",
                blocks.len(),
                rows.len(),
                blocks.iter().filter(|b| matches!(b.runs.first(), Some((_, Style::Heading(_))))).count()
            );
            self.status = "Preview. Ctrl+M or Esc returns to the text.".into();
        } else {
            self.status.clear();
        }
    }

    fn draw_preview(&mut self, pixels: &mut [u32], rows: usize, cols: usize, gutter: usize, text_y0: usize, text_x0: usize) {
        let (cw, ch, w, h) = (self.cell_w, self.cell_h, self.width, self.height);
        let cols = cols + gutter;
        let t = &self.tabs[self.active];
        let blocks = markdown::parse(&String::from_utf8_lossy(t.doc.text()));
        let lines = preview_rows(&blocks, cols.saturating_sub(2));
        let top = t.preview_top.min(lines.len().saturating_sub(1));
        for (r, line) in lines.iter().skip(top).take(rows).enumerate() {
            let y0 = text_y0 + r * ch;
            let mut x = 1;
            for (c, st) in &line.cells {
                let (fg, bg) = match st {
                    Style::Plain | Style::Bullet => (FG, BG_DOC),
                    Style::Strong => (0xF8FAFC, BG_DOC),
                    Style::Emphasis => (0xF472B6, BG_DOC),
                    Style::Code => (0x2DD4BF, BG_GUTTER),
                    Style::Link => (0x38BDF8, BG_DOC),
                    Style::Strike => (DIM, BG_DOC),
                    Style::Heading(_) => (ROSE, BG_DOC),
                    Style::Quote => (0x94A3B8, BG_DOC),
                    Style::Rule => (MUTED, BG_DOC),
                };
                if x < cols {
                    let x0 = text_x0.saturating_sub(cw) + x * cw;
                    self.font.draw_glyph_mono(pixels, w, h, x0 as i32, y0 as i32, EDITOR_PX, *c, fg, bg);
                    let under = match st {
                        Style::Link => Some(ch - 2),
                        Style::Heading(1) => Some(ch - 1),
                        Style::Strike => Some(ch / 2),
                        _ => None,
                    };
                    if let Some(dy) = under {
                        if y0 + dy < h {
                            for dx in 0..cw {
                                if x0 + dx < w { pixels[(y0 + dy) * w + x0 + dx] = fg; }
                            }
                        }
                    }
                }
                x += 1;
            }
        }
    }

    fn tab_record(&self) {
        sex_pdx::serial_record!("[jot.tab] index={} of={} name={}", self.active, self.tabs.len(), self.tabs[self.active].name());
    }
    fn prefill(&mut self) {
        let sel = self.tabs[self.active].doc.selected().to_vec();
        if !sel.is_empty() && !sel.contains(&b'\n') { self.needle = sel; }
    }
    fn find(&mut self, back: bool) {
        if self.needle.is_empty() { return; }
        let needle = self.needle.clone();
        let tab = &mut self.tabs[self.active];
        let found = find::find_next(&mut tab.doc, &needle, back, Options { ignore_case: true, whole_word: false });
        let at = tab.doc.selection().map_or(0, |s| s.0);
        let line = tab.doc.line_of(at) + 1;
        sex_pdx::serial_record!("[jot.find] term={} found={} line={}", String::from_utf8_lossy(&needle), found as u8, line);
        self.status = if found { format!("Found on line {}.", line) } else { "Not found.".into() };
    }
    fn bar_key(&mut self, key: sex_pdx::app_key::Key) -> bool {
        let o = Options { ignore_case: true, whole_word: false };
        let field = if self.field == 0 { &mut self.needle } else { &mut self.with };
        match (key.control, key.code) {
            (_, 0x01) => { self.bar = Bar::None; self.tabs[self.active].hl.invalidate(0); }
            (false, 0x0F | 0x48 | 0x50 | 0x67 | 0x6C) if self.bar == Bar::Replace => self.field ^= 1,
            (_, 0x1C) if self.bar == Bar::Replace && self.field == 0 => self.field = 1,
            (true, 0x1E) if self.bar == Bar::Replace => {
                let (needle, with) = (self.needle.clone(), self.with.clone());
                let n = find::replace_all(&mut self.tabs[self.active].doc, &needle, &with, o);
                sex_pdx::serial_record!("[jot.replaced] n={}", n);
                self.status = format!("Replaced {}.", n);
                self.tabs[self.active].hl.invalidate(0);
            }
            (_, 0x1C) if self.bar == Bar::Replace && self.field == 1 => {
                let (needle, with) = (self.needle.clone(), self.with.clone());
                find::replace_next(&mut self.tabs[self.active].doc, &needle, &with, o);
                self.tabs[self.active].hl.invalidate(0);
            }
            (_, 0x1C) => self.find(key.shift),
            (_, 0x0E) => { while let Some(b) = field.pop() { if b & 0xC0 != 0x80 { break; } } }
            (false, _) => match sex_app::text(key) {
                Some(c) => {
                    if self.fresh { field.clear(); }
                    field.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                None => return false,
            },
            _ => return false,
        }
        self.fresh = false;
        true
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<HitId> {
        self.hits
            .iter()
            .rev()
            .find(|h| x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)
            .map(|h| h.id)
    }
}

trait DigitLen { fn to_string_len(self) -> usize; }
impl DigitLen for usize {
    fn to_string_len(self) -> usize {
        let (mut n, mut d) = (self, 1);
        while n >= 10 { n /= 10; d += 1; }
        d
    }
}

fn display_col(text: &[u8], start: usize, at: usize) -> usize {
    let mut col = 0;
    let mut i = start;
    while i < at.min(text.len()) {
        col += if text[i] == b'\t' { 4 - col % 4 } else { 1 };
        i = sex_textedit::next_char(text, i);
    }
    col
}

fn read_all(window: &mut Window, path: &[u8]) -> Result<Vec<u8>, files::Error> { read_upto(window, path, MAX_FILE) }
fn read_upto(window: &mut Window, path: &[u8], cap: usize) -> Result<Vec<u8>, files::Error> {
    let st = files::stat(window, path)?;
    let len = (st.size as usize).min(cap);
    let mut data = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let n = files::read_at(window, path, at as u64, &mut data[at..])?;
        if n == 0 { break; }
        at += n;
    }
    data.truncate(at);
    Ok(data)
}
fn load(window: &mut Window, path: &[u8]) -> Tab {
    let size = files::stat(window, path).map(|s| s.size as usize).unwrap_or(0);
    let bytes = match read_all(window, path) {
        Ok(t) => { sex_pdx::serial_record!("[jot.loaded] bytes={} fnv={:016x}", t.len(), fnv(&t)); t }
        Err(e) => { sex_pdx::serial_record!("[jot.loaded] bytes=0 status={}", e); Vec::new() }
    };
    let (text, format) = encoding::decode(&bytes);
    if format != Format::default() {
        sex_pdx::serial_record!("[jot.format.read] encoding={} eol={}", format.encoding.name(), format.eol.name());
    }
    let mut tab = Tab::new(path, text);
    tab.format = format;
    tab.readonly = size > MAX_FILE;
    tab
}
fn save(window: &mut Window, tab: &mut Tab) -> Result<usize, files::Error> {
    if tab.readonly { return Err(sex_pdx::fs::DENIED); }
    let (bytes, lost) = encoding::encode(tab.doc.text(), tab.format);
    files::write(window, &tab.path, &bytes)?;
    sex_pdx::serial_record!("[jot.saved] bytes={} fnv={:016x}", bytes.len(), fnv(&bytes));
    tab.doc.mark_saved();
    tab.format_changed = false;
    Ok(lost)
}
fn saved_message(r: Result<usize, files::Error>, tab: &Tab) -> String {
    match r {
        Ok(0) => "Saved.".into(),
        Ok(n) => format!("Saved; {} characters {} cannot hold became ?.", n, tab.format.encoding.name()),
        Err(_) if tab.readonly => "This file is too big for Jot to save; open it in Quil.".into(),
        Err(e) => {
            sex_pdx::serial_record!("[jot.save.failed] status={}", e);
            String::from_utf8_lossy(files::message(e)).into_owned()
        }
    }
}

fn write_session(window: &mut Window, tabs: &[Tab]) {
    let keep: Vec<Saved> = tabs.iter().filter(|t| t.unsaved() && !t.readonly)
        .map(|t| Saved { path: t.path.clone(), text: t.doc.text().to_vec(), cursor: t.doc.cursor, format: t.format }).collect();
    let bytes = session::write(&keep);
    match files::write(window, SESSION, &bytes).and_then(|()| files::sync(window)) {
        Ok(()) => sex_pdx::serial_record!("[jot.session.saved] tabs={} bytes={}", keep.len(), bytes.len()),
        Err(e) => sex_pdx::serial_record!("[jot.session.failed] status={}", e),
    }
}
fn untitled(tabs: &[Tab]) -> Vec<u8> {
    (1..).map(|n: usize| if n == 1 { b"untitled.txt".to_vec() } else { format!("untitled-{}.txt", n).into_bytes() })
        .find(|name| !tabs.iter().any(|t| &t.path == name)).unwrap()
}
fn restore_session(window: &mut Window, tabs: &mut Vec<Tab>) -> usize {
    let Ok(bytes) = read_upto(window, SESSION, usize::MAX) else { return 0 };
    let Some(saved) = session::read(&bytes) else { return 0 };
    let mut n = 0;
    let mut filled: Vec<usize> = Vec::new();
    for s in saved {
        let i = match tabs.iter().enumerate().position(|(i, t)| !s.path.is_empty() && t.path == s.path && !filled.contains(&i)) {
            Some(i) => i,
            None if s.path.is_empty() || filled.iter().any(|&i| tabs[i].path == s.path) => {
                let name = untitled(tabs);
                tabs.push(Tab::new(&name, Vec::new()));
                tabs.len() - 1
            }
            None => { let t = load(window, &s.path); tabs.push(t); tabs.len() - 1 }
        };
        filled.push(i);
        let t = &mut tabs[i];
        let len = t.doc.len();
        t.doc.replace(0, len, &s.text);
        t.doc.seal();
        t.doc.move_to(Motion::To(s.cursor.min(s.text.len())), false);
        t.format_changed = t.format != s.format;
        t.format = s.format;
        t.hl.invalidate(0);
        n += 1;
        sex_pdx::serial_record!("[jot.restored] path={} bytes={} fnv={:016x}", String::from_utf8_lossy(&t.path), s.text.len(), fnv(&s.text));
    }
    n
}

struct PreviewRow { cells: Vec<(char, Style)>, source: usize }
fn preview_rows(blocks: &[markdown::Block], cols: usize) -> Vec<PreviewRow> {
    let cols = cols.max(8);
    let mut out: Vec<PreviewRow> = Vec::new();
    for b in blocks {
        if !out.is_empty() { out.push(PreviewRow { cells: Vec::new(), source: b.line }); }
        if b.runs.first().is_some_and(|r| r.1 == Style::Rule) {
            out.push(PreviewRow { cells: core::iter::repeat(('─', Style::Rule)).take(cols).collect(), source: b.line });
            continue;
        }
        let pad: Vec<(char, Style)> = core::iter::repeat((' ', Style::Plain)).take(b.indent.saturating_sub(b.marker.chars().count())).collect();
        if b.code {
            for (i, (l, st)) in b.runs.iter().enumerate() {
                let mut cells = pad.clone();
                cells.extend(core::iter::repeat((' ', *st)).take(b.marker.chars().count()));
                cells.extend(l.chars().map(|c| (if c == '\t' { ' ' } else { c }, *st)).take(cols.saturating_sub(cells.len())));
                out.push(PreviewRow { cells, source: b.line + 1 + i });
            }
            continue;
        }
        let mut row: Vec<(char, Style)> = pad.clone();
        row.extend(b.marker.chars().map(|c| (c, Style::Bullet)));
        let start_w = row.len();
        let mut word: Vec<(char, Style)> = Vec::new();
        let flush = |row: &mut Vec<(char, Style)>, word: &mut Vec<(char, Style)>, out: &mut Vec<PreviewRow>| {
            if row.len() + word.len() > cols && row.len() > start_w {
                while row.last().is_some_and(|c| c.0 == ' ') { row.pop(); }
                out.push(PreviewRow { cells: core::mem::take(row), source: b.line });
                row.extend(core::iter::repeat((' ', Style::Plain)).take(start_w));
                while word.first().is_some_and(|c| c.0 == ' ') { word.remove(0); }
            }
            row.append(word);
        };
        for (text, st) in &b.runs {
            for c in text.chars() {
                if c == '\n' {
                    flush(&mut row, &mut word, &mut out);
                    out.push(PreviewRow { cells: core::mem::take(&mut row), source: b.line });
                    row.extend(core::iter::repeat((' ', Style::Plain)).take(start_w));
                    continue;
                }
                word.push((c, *st));
                if c == ' ' { flush(&mut row, &mut word, &mut out); }
            }
        }
        flush(&mut row, &mut word, &mut out);
        if row.len() > start_w { out.push(PreviewRow { cells: row, source: b.line }); }
    }
    out
}

#[no_mangle]
pub extern "C" fn _start() -> ! { main() }

fn handle_ask(window: &mut Window, jot: &mut Jot, ask: Ask) -> bool {
    match ask {
        Ask::Nothing => {}
        Ask::Save => {
            let r = save(window, &mut jot.tabs[jot.active]);
            jot.status = saved_message(r, &jot.tabs[jot.active]);
            write_session(window, &jot.tabs);
        }
        ask @ (Ask::SaveAs | Ask::Open) => {
            let open = matches!(ask, Ask::Open);
            jot.status = "Choose the file in Files...".into();
            jot.draw(window);
            let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
            let suggestion = base_name(&jot.tabs[jot.active].path).to_vec();
            match files::pick(window, if open { files::Pick::Open } else { files::Pick::Save }, &suggestion, &mut out) {
                Ok(path) => {
                    let path = path.to_vec();
                    sex_pdx::serial_record!("[jot.picked] path={}", core::str::from_utf8(&path).unwrap_or("?"));
                    if open {
                        let tab = load(window, &path);
                        jot.tabs.push(tab);
                        jot.active = jot.tabs.len() - 1;
                        jot.tab_record();
                        jot.status = "Opened.".into();
                    } else {
                        let tab = &mut jot.tabs[jot.active];
                        tab.path = path.clone();
                        tab.hl = Highlighter::new(Lang::from_name(&path));
                        tab.readonly = false;
                        let r = save(window, tab);
                        jot.status = saved_message(r, &jot.tabs[jot.active]);
                        write_session(window, &jot.tabs);
                    }
                }
                Err(e) => {
                    sex_pdx::serial_record!("[jot.pick.refused] status={}", e);
                    jot.status = "Nothing chosen.".into();
                }
            }
        }
        Ask::Close => {
            if jot.tabs[jot.active].unsaved() && jot.status != "Unsaved changes: Ctrl+W again closes anyway." {
                jot.status = "Unsaved changes: Ctrl+W again closes anyway.".into();
            } else if jot.tabs.len() > 1 {
                jot.tabs.remove(jot.active);
                jot.active = jot.active.min(jot.tabs.len() - 1);
                write_session(window, &jot.tabs);
                jot.tab_record();
                jot.status.clear();
            } else {
                if jot.tabs[jot.active].unsaved() {
                    jot.tabs[jot.active].doc.mark_saved();
                    jot.tabs[jot.active].format_changed = false;
                }
                write_session(window, &jot.tabs);
                sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task);
                return true;
            }
        }
        Ask::Copy => {
            let copied = core::str::from_utf8(&jot.clipboard).ok().filter(|t| !t.is_empty()).map(|t| sex_app::clipboard::copy(window, t));
            sex_pdx::serial_record!("[jot.clip.copy] bytes={} ok={}", jot.clipboard.len(), copied.is_some_and(|r| r.is_ok()) as u8);
        }
        Ask::Paste => {
            let pasted = sex_app::clipboard::paste(window);
            let text = match pasted { Ok(t) => t.as_bytes().to_vec(), Err(_) => jot.clipboard.clone() };
            sex_pdx::serial_record!("[jot.clip.paste] bytes={} system={} err={:?}", text.len(), pasted.is_ok() as u8, pasted.err());
            let tab = jot.tab();
            let from = tab.doc.line_of(tab.doc.cursor);
            tab.doc.insert(&text);
            tab.doc.seal();
            tab.hl.invalidate(from.saturating_sub(1));
        }
        Ask::Escape => {
            let mut probe = [0u8; 16];
            let result = files::read(window, b"../../bin/jot", &mut probe);
            sex_pdx::serial_record!("[jot.escape] refused={} status={}", result.is_err() as u8, result.err().unwrap_or(0));
            jot.status = "Only this program's folder is reachable.".into();
        }
    }
    false
}

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Jot");
    window.want_keys(sex_pdx::app_key::KEYPAD);
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    let mut paths: Vec<Vec<u8>> = given.split(|b| *b == b' ').filter(|w| w.first() == Some(&b'@')).map(|w| w.to_vec()).collect();
    if paths.is_empty() { paths.push(FILE.to_vec()); }
    let tabs: Vec<Tab> = paths.iter().map(|p| load(&mut window, p)).collect::<Vec<Tab>>();
    let (w, h) = window.size();
    let font = NoirFont::new();
    let cell_w = font.mono_advance(EDITOR_PX);
    let cell_h = font.line_height(FaceKind::Mono, EDITOR_PX);
    let mut jot = Jot {
        tabs,
        active: 0,
        font,
        cell_w,
        cell_h,
        width: w.max(64),
        height: h.max(64),
        numbers: true,
        bar: Bar::None,
        field: 0,
        needle: Vec::new(),
        with: Vec::new(),
        status: "Ctrl+S saves. Ctrl+F finds. Ctrl+Z undoes.".into(),
        clipboard: Vec::new(),
        spell: spell::Checker::new(),
        spelling: true,
        scrolled: false,
        fresh: false,
        wrap: false,
        dirty_at: None,
        seen_edits: 0,
        hits: Vec::new(),
        hover: None,
        popup: None,
    };
    let restored = restore_session(&mut window, &mut jot.tabs);
    if restored > 0 {
        jot.active = jot.tabs.iter().position(|t| t.unsaved()).unwrap_or(0);
        jot.status = format!("Brought back unsaved work in {} tab{}.", restored, if restored == 1 { "" } else { "s" });
    }
    if jot.tabs.iter().any(|t| t.readonly) {
        jot.status = "This file is too big for Jot: shown read-only, cut at 64 MiB. Quil edits files of any size.".into();
    }
    jot.draw(&mut window);
    sex_pdx::serial_record!("[jot.ready] target={} task={} tabs={}", window.target, window.task, jot.tabs.len());
    jot.seen_edits = jot.tabs.iter().fold(jot.tabs.len() as u64, |n, t| n.wrapping_add(t.doc.edits()));
    loop {
        let edits = jot.tabs.iter().fold(jot.tabs.len() as u64, |n, t| n.wrapping_add(t.doc.edits()));
        if edits != jot.seen_edits {
            jot.seen_edits = edits;
            jot.dirty_at = Some(sex_pdx::get_ticks());
        }
        let Some(event) = window.next_within(250) else {
            if jot.dirty_at.is_some_and(|t| sex_pdx::get_ticks().saturating_sub(t) >= SESSION_MS) {
                jot.dirty_at = None;
                write_session(&mut window, &jot.tabs);
            }
            continue;
        };
        match event {
            Event::Close => {
                write_session(&mut window, &jot.tabs);
                sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task);
                window.close()
            }
            Event::Resize { width, height } => {
                jot.width = width.max(64);
                jot.height = height.max(64);
            }
            Event::Pointer(p) => {
                let (px, py) = (p.x as i32, p.y as i32);
                match p.kind {
                    PKind::Wheel { steps } => {
                        jot.scroll_rows(-(steps as isize) * 3);
                        jot.scrolled = true;
                    }
                    PKind::Down { button: 2 } => {
                        jot.open_context_menu(px, py);
                    }
                    PKind::Down { button: 0 } if (p.mods & CONTROL) != 0 && py >= jot.doc_top() as i32 => {
                        jot.open_context_menu(px, py);
                    }
                    PKind::Down { button: 0 } => {
                        if let Some(hit) = jot.hit_test(px, py) {
                            match hit {
                                HitId::PopupItem(idx) => {
                                    if let Some(menu) = jot.popup.take() {
                                        if let Some(item) = menu.items.get(idx) {
                                            match &item.action {
                                                PopupAction::SpellFix { start, end, from, to, others } => {
                                                    let tab = jot.tab();
                                                    tab.doc.seal();
                                                    tab.doc.replace(*start, *end, to.as_bytes());
                                                    tab.doc.seal();
                                                    tab.hl.invalidate(0);
                                                    sex_pdx::serial_record!("[jot.spell.fixed] from={} to={} others={}", from, to, others);
                                                    jot.status = format!("{} -> {}", from, to);
                                                }
                                                PopupAction::Cut => {
                                                    jot.clipboard = jot.tab().doc.cut();
                                                    let _ = handle_ask(&mut window, &mut jot, Ask::Copy);
                                                }
                                                PopupAction::Copy => {
                                                    jot.clipboard = jot.tab().doc.selected().to_vec();
                                                    let _ = handle_ask(&mut window, &mut jot, Ask::Copy);
                                                }
                                                PopupAction::Paste => { let _ = handle_ask(&mut window, &mut jot, Ask::Paste); }
                                                PopupAction::SelectAll => jot.tab().doc.select_all(),
                                            }
                                        }
                                    }
                                }
                                HitId::ToolNew | HitId::TabAdd => {
                                    jot.popup = None;
                                    let name = untitled(&jot.tabs);
                                    jot.tabs.push(Tab::new(&name, Vec::new()));
                                    jot.active = jot.tabs.len() - 1;
                                    jot.tab_record();
                                }
                                HitId::ToolOpen => {
                                    jot.popup = None;
                                    let _ = handle_ask(&mut window, &mut jot, Ask::Open);
                                }
                                HitId::ToolSave => {
                                    jot.popup = None;
                                    let _ = handle_ask(&mut window, &mut jot, Ask::Save);
                                }
                                HitId::ToolFind => {
                                    jot.popup = None;
                                    if jot.bar == Bar::None {
                                        jot.bar = Bar::Find;
                                        jot.field = 0;
                                        jot.prefill();
                                        jot.fresh = true;
                                    } else {
                                        jot.bar = Bar::None;
                                    }
                                }
                                HitId::ToolSpell => {
                                    jot.popup = None;
                                    jot.spelling = !jot.spelling;
                                    jot.status = if jot.spelling { "Spelling marked.".into() } else { "Spelling off.".into() };
                                }
                                HitId::ToolPreview => {
                                    jot.popup = None;
                                    jot.toggle_preview();
                                }
                                HitId::Tab(idx) => {
                                    jot.popup = None;
                                    if idx < jot.tabs.len() {
                                        jot.active = idx;
                                        jot.tab_record();
                                    }
                                }
                            }
                            jot.draw(&mut window);
                            continue;
                        }
                        jot.popup = None;
                        if let Some(at) = jot.pointer_to_offset(px, py) {
                            jot.tabs[jot.active].doc.move_to(Motion::To(at), (p.mods & SHIFT) != 0);
                            jot.scrolled = false;
                        }
                    }
                    PKind::Move => {
                        if p.buttons & 1 != 0 && jot.popup.is_none() {
                            if let Some(at) = jot.pointer_to_offset(px, py) {
                                jot.tabs[jot.active].doc.move_to(Motion::To(at), true);
                                jot.scrolled = false;
                            }
                        } else {
                            let h = jot.hit_test(px, py);
                            if h != jot.hover {
                                jot.hover = h;
                            } else {
                                continue;
                            }
                        }
                    }
                    _ => continue,
                }
            }
            Event::DragEnded { .. } | Event::KeyUp(_) => continue,
            Event::Drop { kind, .. } => {
                let bytes = sex_app::dnd::dropped().to_vec();
                if kind == sex_pdx::dnd::FILE {
                    let mut path = vec![b'@'];
                    path.extend_from_slice(&bytes);
                    let tab = load(&mut window, &path);
                    sex_pdx::serial_record!("[jot.dropped] kind=file bytes={} path={}", tab.doc.text().len(), core::str::from_utf8(&bytes).unwrap_or("?"));
                    jot.tabs.push(tab);
                    jot.active = jot.tabs.len() - 1;
                    jot.status = "Opened the dropped file.".into();
                } else {
                    let tab = jot.tab();
                    tab.doc.insert(&bytes);
                    tab.doc.seal();
                    sex_pdx::serial_record!("[jot.dropped] kind=text bytes={}", bytes.len());
                }
            }
            Event::Key(key) => {
                jot.scrolled = false;
                let ask = jot.key(key);
                if handle_ask(&mut window, &mut jot, ask) {
                    window.close();
                }
            }
        }
        jot.draw(&mut window);
    }
}
