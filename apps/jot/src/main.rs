// SPDX-License-Identifier: GPL-3.0-only
//! Jot: a text editor. Several files in tabs, an undo tree, find and replace, line
//! numbers, syntax colouring for Rust, C, JavaScript, Markdown and TOML, and spelling
//! marked as you type. The editing itself is `crates/sex-textedit`, proven on the host;
//! this file draws it and maps keys.
//!
//! Keys: Ctrl+S save, Ctrl+Shift+S save as, Ctrl+O open in a new tab, Ctrl+N new tab,
//! Ctrl+W close tab, Ctrl+PageUp/PageDown switch tabs, Ctrl+Z undo, Ctrl+Y redo, Ctrl+B
//! other undo branch, Ctrl+F find, Ctrl+H replace, Ctrl+G next, Ctrl+A select all,
//! Ctrl+C/X/V copy, cut, paste, Ctrl+L line numbers, Ctrl+K spelling marks, Ctrl+. fixes the
//! misspelt word at the cursor, Ctrl+R wraps long lines, Ctrl+M previews Markdown, Ctrl+T
//! switches LF/CRLF, Ctrl+Shift+T cycles the encoding, F11 closes.
//!
//! Unsaved work is kept in `.jot-session` a moment after each change and comes back
//! when Jot starts again, after a crash or a reboot too.
#![no_std]
#![no_main]

extern crate alloc;

mod spell;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use sex_app::{files, Event, Nav, Window};
use sex_textedit::find::{self, Options};
use sex_textedit::highlight::{Highlighter, Kind, Lang};
use sex_textedit::encoding::{self, Eol, Format};
use sex_textedit::markdown::{self, Style};
use sex_textedit::session::{self, Saved};
use sex_textedit::{wrap, Doc, Motion};
use sex_vt::render::{cell_for_height, Cache, Painter};

const FILE: &[u8] = b"notes.txt";
/// Bigger files open read-only, cut here: they belong in Quil.
const MAX_FILE: usize = 64 << 20;
const SESSION: &[u8] = b".jot-session";
/// How long after the last change the session is written.
const SESSION_MS: u64 = 800;
static mut CACHE: Cache = Cache::new();

const BG: u32 = 0x1E1E2E;
const FG: u32 = 0xCDD6F4;
const DIM: u32 = 0x6C7086;
const GUTTER: u32 = 0x181825;
const CURRENT: u32 = 0x262637;
const SELECTED: u32 = 0x45475A;
const HIT: u32 = 0x5B4A2A;
const BAR: u32 = 0x11111B;
const ACCENT: u32 = 0x89B4FA;
const MISSPELT: u32 = 0xF38BA8;

fn color(kind: Kind) -> u32 {
    match kind {
        Kind::Plain => FG, Kind::Keyword => 0xCBA6F7, Kind::Type => 0xF9E2AF, Kind::String => 0xA6E3A1,
        Kind::Number => 0xFAB387, Kind::Comment => 0x7F849C, Kind::Punct => 0x9399B2, Kind::Macro => 0x89DCEB,
        Kind::Heading => 0x89B4FA, Kind::Emphasis => 0xF5C2E7, Kind::Code => 0x94E2D5, Kind::Link => 0x74C7EC, Kind::Key => 0x89B4FA,
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[jot.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn fnv(bytes: &[u8]) -> u64 { bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3)) }
fn base_name(path: &[u8]) -> &[u8] { path.rsplit(|b| *b == b'/').next().unwrap_or(path) }

struct Tab {
    doc: Doc,
    path: Vec<u8>,
    hl: Highlighter,
    /// First line and first column shown.
    top: usize,
    left: usize,
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
        Tab { doc, hl: Highlighter::new(Lang::from_name(path)), path: path.to_vec(), top: 0, left: 0, format: Format::default(), format_changed: false, readonly: false, preview: false, preview_top: 0 }
    }
    fn name(&self) -> String { String::from_utf8_lossy(base_name(&self.path)).into_owned() }
    fn unsaved(&self) -> bool { self.doc.modified() || self.format_changed }
}

/// One screen row of text: its line number and byte range in the document.
#[derive(Clone, Copy)]
struct Row { line: usize, start: usize, end: usize, /// Last row of its line.
    last: bool }

#[derive(PartialEq, Eq, Clone, Copy)]
enum Bar { None, Find, Replace }

struct Jot {
    tabs: Vec<Tab>,
    active: usize,
    painter: Painter,
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
}

/// What a key asked for that needs the window.
enum Ask { Nothing, Save, SaveAs, Open, Close, Escape }

impl Jot {
    fn tab(&mut self) -> &mut Tab { &mut self.tabs[self.active] }
    fn rows(&self) -> usize { (self.height / self.painter.cell_h).saturating_sub(2 + (self.bar != Bar::None) as usize).max(1) }
    fn gutter(&self) -> usize { if self.numbers { (self.tabs[self.active].doc.line_count().to_string_len()).max(3) + 1 } else { 0 } }
    fn cols(&self) -> usize { (self.width / self.painter.cell_w).saturating_sub(self.gutter()).max(4) }

    /// The screen rows from the tab's first shown line: whole lines, or wrapped pieces.
    fn layout(&self, t: &Tab, rows: usize) -> Vec<Row> {
        let text = t.doc.text();
        let cols = self.cols();
        let mut out = Vec::new();
        let mut start = t.doc.offset_of_line(t.top);
        let mut line = t.top;
        while out.len() < rows && line < t.doc.line_count() {
            let end = t.doc.line_end(start);
            if self.wrap {
                let parts = wrap::rows(&text[start..end], cols);
                let n = parts.len();
                for (i, (a, b)) in parts.into_iter().enumerate() { out.push(Row { line, start: start + a, end: start + b, last: i + 1 == n }); }
            } else { out.push(Row { line, start, end, last: true }); }
            start = (end + 1).min(text.len());
            line += 1;
        }
        out.truncate(rows);
        out
    }

    /// Keeps the cursor on screen.
    fn follow(&mut self) {
        let (rows, cols) = (self.rows(), self.cols());
        let wrapped = self.wrap;
        let t = &mut self.tabs[self.active];
        let line = t.doc.line_of(t.doc.cursor);
        if line < t.top { t.top = line; }
        if line >= t.top + rows { t.top = line + 1 - rows; }
        if wrapped {
            t.left = 0;
            // Move the top down until the cursor's row is on screen.
            loop {
                let t = &self.tabs[self.active];
                let lay = self.layout(t, rows);
                let c = t.doc.cursor;
                let shown = lay.iter().any(|r| c >= r.start && (c < r.end || (c == r.end && r.last)));
                if shown || t.top >= line { break; }
                self.tabs[self.active].top += 1;
            }
            return;
        }
        let col = display_col(t.doc.text(), t.doc.line_start(t.doc.cursor), t.doc.cursor);
        if col < t.left { t.left = col.saturating_sub(8); }
        if col >= t.left + cols { t.left = col + 8 - cols; }
    }

    fn draw(&mut self, window: &mut Window) {
        if !self.scrolled { self.follow(); }
        let (w, h) = (self.width, self.height);
        let pixels = &mut window.pixels()[..w * h];
        pixels.fill(BG);
        let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
        let (cw, ch) = (self.painter.cell_w, self.painter.cell_h);
        let (rows, cols, gutter) = (self.rows(), self.cols(), self.gutter());

        // Tabs along the top.
        let mut x = 0;
        for (i, t) in self.tabs.iter().enumerate() {
            let label = format!(" {}{}{} ", t.name(), if t.unsaved() { " •" } else { "" }, if t.readonly { " (read-only)" } else { "" });
            let bg = if i == self.active { BG } else { BAR };
            let fg = if i == self.active { FG } else { DIM };
            if x + label.chars().count() * cw > w { break; }
            x = self.painter.text(cache, pixels, w, x, 0, &label, fg, bg);
        }
        if x < w { for y in 0..ch { pixels[y * w + x..y * w + w].fill(BAR); } }
        for px in 0..w.min(x) { if self.tabs.get(self.active).is_some() { let _ = px; } }

        // The text, or its Markdown preview.
        if self.tabs[self.active].preview {
            self.draw_preview(pixels, rows, cols, gutter);
        } else {
        let layout = { let t = &self.tabs[self.active]; self.layout(t, rows) };
        let t = &mut self.tabs[self.active];
        let text = t.doc.text();
        let spans = t.hl.visible(text, t.top, rows);
        let cursor_line = t.doc.line_of(t.doc.cursor);
        let sel = t.doc.selection();
        let hits: Vec<(usize, usize)> = if self.bar != Bar::None && !self.needle.is_empty() { find::all(text, &self.needle, Options { ignore_case: true, whole_word: false }) } else { Vec::new() };
        // Prose only: in code the dictionary would mark every identifier.
        let prose = matches!(t.hl.lang, Lang::Plain | Lang::Markdown);
        let mut misspelt: Vec<(usize, usize)> = Vec::new();
        let mut misspelt_line = usize::MAX;
        for (r, row) in layout.iter().enumerate() {
            let n = row.line;
            let line_start = t.doc.line_start(row.start);
            let y0 = (r + 1) * ch;
            let line_bg = if n == cursor_line { CURRENT } else { BG };
            if line_bg != BG { for y in 0..ch { pixels[(y0 + y) * w + gutter * cw..(y0 + y) * w + w].fill(line_bg); } }
            if gutter > 0 && row.start == line_start {
                let num = format!("{:>width$} ", n + 1, width = gutter - 1);
                self.painter.text(cache, pixels, w, 0, y0, &num, if n == cursor_line { FG } else { DIM }, GUTTER);
            }
            if misspelt_line != n {
                misspelt_line = n;
                misspelt = if self.spelling && prose { let e = t.doc.line_end(line_start); self.spell.misspelt(&text[line_start..e]) } else { Vec::new() };
            }
            let line_spans = spans.get(n - t.top).cloned().unwrap_or_default();
            // Columns count from the row's start when wrapped, from the line's start otherwise.
            let mut col = if self.wrap { 0 } else { display_col(text, line_start, row.start) };
            let mut at = row.start;
            while at < row.end {
                let c_end = sex_textedit::next_char(text, at);
                let c = core::str::from_utf8(&text[at..c_end]).ok().and_then(|s| s.chars().next()).unwrap_or('?');
                let width = if c == '\t' { 4 - col % 4 } else { 1 };
                let rel = at - line_start;
                let fg = line_spans.iter().find(|s| s.start <= rel && rel < s.end).map_or(FG, |s| color(s.kind));
                let mut bg = line_bg;
                if hits.iter().any(|h| h.0 <= at && at < h.1) { bg = HIT; }
                if sel.is_some_and(|(a, b)| a <= at && at < b) { bg = SELECTED; }
                for k in 0..width {
                    let vis = col + k;
                    if vis >= t.left && vis < t.left + cols {
                        let x0 = (gutter + vis - t.left) * cw;
                        self.painter.glyph(cache, pixels, w, x0, y0, if c == '\t' { ' ' } else { c }, fg, bg);
                        if misspelt.iter().any(|m| m.0 <= rel && rel < m.1) {
                            // A dotted line under words the dictionary does not know.
                            for dx in (0..cw).step_by(2) { pixels[(y0 + ch - 2) * w + x0 + dx] = MISSPELT; }
                        }
                    }
                }
                col += width;
                at = c_end;
            }
            // The cursor: a bar before its character.
            let cur = t.doc.cursor;
            if cur >= row.start && (cur < row.end || (cur == row.end && row.last)) {
                let vis = if self.wrap { display_col(text, row.start, cur) } else { display_col(text, line_start, cur) };
                if vis >= t.left && vis <= t.left + cols {
                    let x0 = (gutter + vis - t.left) * cw;
                    for y in 1..ch - 1 { if x0 + 1 < w { pixels[(y0 + y) * w + x0] = ACCENT; pixels[(y0 + y) * w + x0 + 1] = ACCENT; } }
                }
            }
        }
        }

        // The find bar, then the status line.
        let mut y = (rows + 1) * ch;
        if self.bar != Bar::None {
            for yy in 0..ch { pixels[(y + yy) * w..(y + yy) * w + w].fill(BAR); }
            let find = format!(" Find: {}{}", String::from_utf8_lossy(&self.needle), if self.field == 0 { "▏" } else { "" });
            let mut x = self.painter.text(cache, pixels, w, 0, y, &find, FG, BAR);
            if self.bar == Bar::Replace {
                let with = format!("   Replace with: {}{}", String::from_utf8_lossy(&self.with), if self.field == 1 { "▏" } else { "" });
                x = self.painter.text(cache, pixels, w, x, y, &with, FG, BAR);
            }
            let help = if self.bar == Bar::Replace { "   Up/Down field · Enter replace · Ctrl+A all · Esc" } else { "   Enter next · Shift+Enter back · Esc" };
            self.painter.text(cache, pixels, w, x, y, help, DIM, BAR);
            y += ch;
        }
        if y + ch <= h {
            for yy in 0..ch { pixels[(y + yy) * w..(y + yy) * w + w].fill(GUTTER); }
            let t = &self.tabs[self.active];
            let line = t.doc.line_of(t.doc.cursor) + 1;
            let col = t.doc.column(t.doc.cursor) + 1;
            let right = format!("Ln {}, Col {}  {}  {}  {}  {}{}{} ", line, col, t.hl.lang.name(), t.format.encoding.name(), t.format.eol.name(),
                if self.wrap { "Wrap  " } else { "" }, if self.spelling { "ABC✓  " } else { "" }, String::from_utf8_lossy(&t.path));
            let left = format!(" {}", self.status);
            self.painter.text(cache, pixels, w, 0, y, &left, FG, GUTTER);
            let rx = w.saturating_sub(right.chars().count() * cw);
            self.painter.text(cache, pixels, w, rx, y, &right, DIM, GUTTER);
        }
        window.show_pixels(w, h);
    }

    fn key(&mut self, key: sex_pdx::app_key::Key) -> Ask {
        let (ctrl, shift, code) = (key.control, key.shift, key.code);
        let page = self.rows().saturating_sub(1).max(1);
        if self.bar != Bar::None && self.bar_key(key) { return Ask::Nothing; }
        if self.tabs[self.active].preview {
            // The preview scrolls; Ctrl+M or Escape goes back to the text.
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
        match (ctrl, code, nav) {
            (true, 0x1F, _) => return if shift { Ask::SaveAs } else { Ask::Save },
            (true, 0x18, _) => return Ask::Open,
            (true, 0x11, _) => return Ask::Close,
            (true, 0x12, _) => return Ask::Escape,
            (true, 0x31, _) => { self.tabs.push(Tab::new(b"untitled.txt", Vec::new())); self.active = self.tabs.len() - 1; self.tab_record(); return Ask::Nothing; }
            (true, 0x49 | 0x68, _) => { self.active = (self.active + self.tabs.len() - 1) % self.tabs.len(); self.tab_record(); return Ask::Nothing; }
            (true, 0x51 | 0x6D, _) => { self.active = (self.active + 1) % self.tabs.len(); self.tab_record(); return Ask::Nothing; }
            (true, 0x2C, _) if shift => { if tab.doc.redo() { self.status = "Redone.".into(); } }
            (true, 0x2C, _) => { if tab.doc.undo() { sex_pdx::serial_record!("[jot.undo] depth={} bytes={}", tab.doc.undo_tree().depth(), tab.doc.len()); } else { self.status = "Nothing to undo.".into(); } }
            (true, 0x15, _) => { if tab.doc.redo() { sex_pdx::serial_record!("[jot.redo] depth={} bytes={}", tab.doc.undo_tree().depth(), tab.doc.len()); } else { self.status = "Nothing to redo.".into(); } }
            (true, 0x30, _) => { self.status = if tab.doc.switch_branch() { "Redo now takes the other branch.".into() } else { "No other branch here.".into() }; }
            (true, 0x1E, _) => tab.doc.select_all(),
            (true, 0x2E, _) => { self.clipboard = tab.doc.selected().to_vec(); self.status = format!("Copied {} bytes.", self.clipboard.len()); }
            (true, 0x2D, _) => { self.clipboard = tab.doc.cut(); }
            (true, 0x2F, _) => { let c = self.clipboard.clone(); tab.doc.insert(&c); tab.doc.seal(); }
            (true, 0x26, _) => self.numbers = !self.numbers,
            (true, 0x13, _) => {
                self.wrap = !self.wrap;
                if self.wrap { self.tabs[self.active].left = 0; }
                sex_pdx::serial_record!("[jot.wrap] on={} rows={}", self.wrap as u8, { let t = &self.tabs[self.active]; self.layout(t, self.rows()).len() });
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
            (true, 0x25, _) => { self.spelling = !self.spelling; self.status = if self.spelling { "Spelling marked.".into() } else { "Spelling off.".into() }; }
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
            (false, ..) => match sex_app::text(key) { Some(c) => tab.doc.insert(c.encode_utf8(&mut [0; 4]).as_bytes()), None => return Ask::Nothing },
            _ => return Ask::Nothing,
        }
        // Colouring below the first changed line may differ now.
        let tab = &mut self.tabs[self.active];
        if tab.doc.len() != len_before || ctrl { self.dirty_at = Some(sex_pdx::get_ticks()); }
        if tab.doc.len() != len_before || ctrl { let from = before.min(tab.doc.line_of(tab.doc.cursor)); tab.hl.invalidate(from.saturating_sub(1)); }
        Ask::Nothing
    }
    /// Ctrl+.: the misspelt word at or before the cursor becomes the dictionary's first
    /// suggestion. One undo step.
    fn fix_spelling(&mut self) {
        let tab = &mut self.tabs[self.active];
        let start = tab.doc.line_start(tab.doc.cursor);
        let end = tab.doc.line_end(start);
        let line = tab.doc.text()[start..end].to_vec();
        let rel = tab.doc.cursor - start;
        let Some(&(a, b)) = spell::words(&line).iter().rev().find(|(a, _)| *a <= rel) else { self.status = "No word here.".into(); return };
        let word = String::from_utf8_lossy(&line[a..b]).into_owned();
        if self.spell.correct(&word) { self.status = format!("\"{}\" is spelt right.", word); return; }
        let suggestions = self.spell.suggest(&word);
        let Some(best) = suggestions.first().cloned() else { self.status = format!("No suggestions for \"{}\".", word); return };
        let tab = &mut self.tabs[self.active];
        tab.doc.seal();
        tab.doc.replace(start + a, start + b, best.as_bytes());
        tab.doc.seal();
        tab.hl.invalidate(0);
        sex_pdx::serial_record!("[jot.spell.fixed] from={} to={} others={}", word, best, suggestions.len() - 1);
        self.status = format!("{} → {}   (also: {})", word, best, suggestions[1..].iter().take(4).cloned().collect::<Vec<_>>().join(", "));
    }
    fn toggle_preview(&mut self) {
        let cols = self.cols();
        let t = &mut self.tabs[self.active];
        t.preview = !t.preview;
        if t.preview {
            let blocks = markdown::parse(&String::from_utf8_lossy(t.doc.text()));
            // Start at the block holding the cursor's line.
            let line = t.doc.line_of(t.doc.cursor);
            let rows = preview_rows(&blocks, cols);
            t.preview_top = rows.iter().position(|r| r.source >= line).unwrap_or(0).saturating_sub(0);
            if blocks.iter().all(|b| b.line > line) { t.preview_top = 0; }
            sex_pdx::serial_record!("[jot.preview] blocks={} rows={} headings={}", blocks.len(), rows.len(), blocks.iter().filter(|b| matches!(b.runs.first(), Some((_, Style::Heading(_))))).count());
            self.status = "Preview. Ctrl+M or Esc returns to the text.".into();
        } else { self.status.clear(); }
    }
    fn draw_preview(&mut self, pixels: &mut [u32], rows: usize, cols: usize, gutter: usize) {
        let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
        let (cw, ch, w) = (self.painter.cell_w, self.painter.cell_h, self.width);
        let cols = cols + gutter;
        let t = &self.tabs[self.active];
        let blocks = markdown::parse(&String::from_utf8_lossy(t.doc.text()));
        let lines = preview_rows(&blocks, cols.saturating_sub(2));
        let top = t.preview_top.min(lines.len().saturating_sub(1));
        for (r, line) in lines.iter().skip(top).take(rows).enumerate() {
            let y0 = (r + 1) * ch;
            let mut x = 1;
            for (c, st) in &line.cells {
                let (fg, bg) = match st {
                    Style::Plain | Style::Bullet => (FG, BG), Style::Strong => (0xF5E0DC, BG), Style::Emphasis => (0xF5C2E7, BG),
                    Style::Code => (0x94E2D5, GUTTER), Style::Link => (0x74C7EC, BG), Style::Strike => (DIM, BG),
                    Style::Heading(_) => (ACCENT, BG), Style::Quote => (0xA6ADC8, BG), Style::Rule => (DIM, BG),
                };
                if x < cols {
                    let x0 = x * cw;
                    self.painter.glyph(cache, pixels, w, x0, y0, *c, fg, bg);
                    let under = match st { Style::Link => Some(ch - 2), Style::Heading(1) => Some(ch - 1), Style::Strike => Some(ch / 2), _ => None };
                    if let Some(dy) = under { for dx in 0..cw { pixels[(y0 + dy) * w + x0 + dx] = fg; } }
                }
                x += 1;
            }
        }
    }
    fn tab_record(&self) { sex_pdx::serial_record!("[jot.tab] index={} of={} name={}", self.active, self.tabs.len(), self.tabs[self.active].name()); }
    /// The bar starts with the selection, when there is one on a single line.
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
    /// Keys while the find bar is open. True if the bar took the key.
    fn bar_key(&mut self, key: sex_pdx::app_key::Key) -> bool {
        let o = Options { ignore_case: true, whole_word: false };
        let field = if self.field == 0 { &mut self.needle } else { &mut self.with };
        match (key.control, key.code) {
            (_, 0x01) => { self.bar = Bar::None; self.tabs[self.active].hl.invalidate(0); }
            // Shell keeps Tab: Up and Down, or Enter in the find field, move between fields.
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
                Some(c) => { if self.fresh { field.clear(); } field.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()); }
                None => return false,
            },
            _ => return false,
        }
        self.fresh = false;
        true
    }
}

trait DigitLen { fn to_string_len(self) -> usize; }
impl DigitLen for usize { fn to_string_len(self) -> usize { let (mut n, mut d) = (self, 1); while n >= 10 { n /= 10; d += 1; } d } }

/// Screen column of byte `at` on a line starting at `start`, tabs to multiples of four.
fn display_col(text: &[u8], start: usize, at: usize) -> usize {
    let mut col = 0;
    let mut i = start;
    while i < at.min(text.len()) {
        col += if text[i] == b'\t' { 4 - col % 4 } else { 1 };
        i = sex_textedit::next_char(text, i);
    }
    col
}

fn read_all(window: &mut Window, path: &[u8]) -> Result<Vec<u8>, files::Error> {
    let st = files::stat(window, path)?;
    let len = (st.size as usize).min(MAX_FILE);
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
    if format != Format::default() { sex_pdx::serial_record!("[jot.format.read] encoding={} eol={}", format.encoding.name(), format.eol.name()); }
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
        Err(e) => { sex_pdx::serial_record!("[jot.save.failed] status={}", e); String::from_utf8_lossy(files::message(e)).into_owned() }
    }
}

/// Writes the unsaved tabs to the session file (or empties it when there are none).
fn write_session(window: &mut Window, tabs: &[Tab]) {
    let keep: Vec<Saved> = tabs.iter().filter(|t| t.unsaved() && !t.readonly)
        .map(|t| Saved { path: t.path.clone(), text: t.doc.text().to_vec(), cursor: t.doc.cursor, format: t.format }).collect();
    let bytes = session::write(&keep);
    // Synced, so it is there after a power cut, not only after a crash of Jot itself.
    match files::write(window, SESSION, &bytes).and_then(|()| files::sync(window)) {
        Ok(()) => sex_pdx::serial_record!("[jot.session.saved] tabs={} bytes={}", keep.len(), bytes.len()),
        Err(e) => sex_pdx::serial_record!("[jot.session.failed] status={}", e),
    }
}
/// Brings back unsaved work: into the tab of the same file if open, else a new tab.
fn restore_session(window: &mut Window, tabs: &mut Vec<Tab>) -> usize {
    let Ok(bytes) = read_all(window, SESSION) else { return 0 };
    let Some(saved) = session::read(&bytes) else { return 0 };
    let mut n = 0;
    for s in saved {
        let i = match tabs.iter().position(|t| !s.path.is_empty() && t.path == s.path) {
            Some(i) => i,
            None if s.path.is_empty() => { tabs.push(Tab::new(b"untitled.txt", Vec::new())); tabs.len() - 1 }
            None => { let t = load(window, &s.path); tabs.push(t); tabs.len() - 1 }
        };
        let t = &mut tabs[i];
        // An edit from the file's text to the saved text: unsaved, and undo goes back to disk.
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

/// The preview laid out as rows of styled cells, each remembering its source line.
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
        // Words with their styles, wrapped at `cols`; `\n` is a hard break.
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
                if c == '\n' { flush(&mut row, &mut word, &mut out); out.push(PreviewRow { cells: core::mem::take(&mut row), source: b.line }); row.extend(core::iter::repeat((' ', Style::Plain)).take(start_w)); continue; }
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
pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Jot");
    // Files the person gave, as `@/path` words, or the notes in Jot's own folder.
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    let mut paths: Vec<Vec<u8>> = given.split(|b| *b == b' ').filter(|w| w.first() == Some(&b'@')).map(|w| w.to_vec()).collect();
    if paths.is_empty() { paths.push(FILE.to_vec()); }
    let tabs: Vec<Tab> = paths.iter().map(|p| load(&mut window, p)).collect::<Vec<Tab>>();
    let (w, h) = window.size();
    let (cw, ch) = cell_for_height(18);
    let mut jot = Jot {
        tabs, active: 0, painter: Painter::new(cw, ch), width: w.max(64), height: h.max(64), numbers: true, bar: Bar::None, field: 0,
        needle: Vec::new(), with: Vec::new(), status: "Ctrl+S saves. Ctrl+F finds. Ctrl+Z undoes.".into(), clipboard: Vec::new(),
        spell: spell::Checker::new(), spelling: true, scrolled: false, fresh: false, wrap: false, dirty_at: None,
    };
    let restored = restore_session(&mut window, &mut jot.tabs);
    if restored > 0 {
        jot.active = jot.tabs.iter().position(|t| t.unsaved()).unwrap_or(0);
        jot.status = format!("Brought back unsaved work in {} tab{}.", restored, if restored == 1 { "" } else { "s" });
    }
    if jot.tabs.iter().any(|t| t.readonly) { jot.status = "This file is too big for Jot: shown read-only, cut at 64 MiB. Quil edits files of any size.".into(); }
    jot.draw(&mut window);
    sex_pdx::serial_record!("[jot.ready] target={} task={} tabs={}", window.target, window.task, jot.tabs.len());
    loop {
        let Some(event) = window.next_within(250) else {
            // Quiet for a moment after a change: keep the unsaved work safe.
            if jot.dirty_at.is_some_and(|t| sex_pdx::get_ticks().saturating_sub(t) >= SESSION_MS) {
                jot.dirty_at = None;
                write_session(&mut window, &jot.tabs);
            }
            continue;
        };
        match event {
            Event::Close => {
                // Closing keeps unsaved tabs for next time, as the session does after a crash.
                write_session(&mut window, &jot.tabs);
                sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task);
                window.close()
            }
            Event::Resize { width, height } => { jot.width = width.max(64); jot.height = height.max(64); }
            Event::Pointer(p) => {
                use sex_pdx::app_pointer::Kind as P;
                let rows = jot.rows();
                let (cw, ch, gutter) = (jot.painter.cell_w, jot.painter.cell_h, jot.gutter());
                let lay = jot.layout(&jot.tabs[jot.active], rows);
                let wrapped = jot.wrap;
                let t = jot.tab();
                match p.kind {
                    P::Wheel { steps } => { let max = t.doc.line_count().saturating_sub(1); t.top = (t.top as isize - steps as isize * 3).clamp(0, max as isize) as usize; jot.scrolled = true; }
                    P::Down { button: 0 } | P::Move if p.kind != P::Move || p.buttons & 1 != 0 => {
                        let row = (p.y.max(0) as usize) / ch;
                        if row == 0 || row > rows || t.preview { continue; }
                        // The screen row under the pointer, from the same layout the drawing used.
                        let Some(r) = lay.get(row - 1).or(lay.last()) else { continue };
                        let start = r.start;
                        let want = ((p.x.max(0) as usize) / cw).saturating_sub(gutter) + if wrapped { 0 } else { t.left };
                        // A wrapped row's end is where the next row starts: stop just before it.
                        let end = if r.last { r.end } else { r.end.saturating_sub(1).max(start) };
                        let mut at = start;
                        while at < end && display_col(t.doc.text(), start, sex_textedit::next_char(t.doc.text(), at)) <= want { at = sex_textedit::next_char(t.doc.text(), at); }
                        t.doc.move_to(Motion::To(at), p.kind == P::Move);
                        jot.scrolled = false;
                    }
                    _ => continue,
                }
            }
            Event::Key(key) => match { jot.scrolled = false; jot.key(key) } {
                Ask::Nothing => {}
                Ask::Save => {
                    let r = save(&mut window, &mut jot.tabs[jot.active]);
                    jot.status = saved_message(r, &jot.tabs[jot.active]);
                    write_session(&mut window, &jot.tabs);
                }
                ask @ (Ask::SaveAs | Ask::Open) => {
                    let open = matches!(ask, Ask::Open);
                    jot.status = "Choose the file in Files...".into();
                    jot.draw(&mut window);
                    let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                    let suggestion = base_name(&jot.tabs[jot.active].path).to_vec();
                    match files::pick(&mut window, if open { files::Pick::Open } else { files::Pick::Save }, &suggestion, &mut out) {
                        Ok(path) => {
                            let path = path.to_vec();
                            sex_pdx::serial_record!("[jot.picked] path={}", core::str::from_utf8(&path).unwrap_or("?"));
                            if open {
                                let tab = load(&mut window, &path);
                                jot.tabs.push(tab);
                                jot.active = jot.tabs.len() - 1;
                                jot.tab_record();
                                jot.status = "Opened.".into();
                            } else {
                                let tab = &mut jot.tabs[jot.active];
                                tab.path = path.clone();
                                tab.hl = Highlighter::new(Lang::from_name(&path));
                                tab.readonly = false;
                                let r = save(&mut window, tab);
                                jot.status = saved_message(r, &jot.tabs[jot.active]);
                                write_session(&mut window, &jot.tabs);
                            }
                        }
                        Err(e) => { sex_pdx::serial_record!("[jot.pick.refused] status={}", e); jot.status = "Nothing chosen.".into(); }
                    }
                }
                Ask::Close => {
                    if jot.tabs[jot.active].unsaved() && jot.status != "Unsaved changes: Ctrl+W again closes anyway." {
                        jot.status = "Unsaved changes: Ctrl+W again closes anyway.".into();
                    } else if jot.tabs.len() > 1 {
                        jot.tabs.remove(jot.active);
                        jot.active = jot.active.min(jot.tabs.len() - 1);
                        write_session(&mut window, &jot.tabs);
                        jot.tab_record();
                        jot.status.clear();
                    } else {
                        // Closed on purpose, changes discarded: nothing to bring back.
                        if jot.tabs[jot.active].unsaved() { jot.tabs[jot.active].doc.mark_saved(); jot.tabs[jot.active].format_changed = false; }
                        write_session(&mut window, &jot.tabs);
                        sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task);
                        window.close()
                    }
                }
                // Ctrl+E tries to leave its folder, which Files must refuse.
                Ask::Escape => {
                    let mut probe = [0u8; 16];
                    let result = files::read(&mut window, b"../../bin/jot", &mut probe);
                    sex_pdx::serial_record!("[jot.escape] refused={} status={}", result.is_err() as u8, result.err().unwrap_or(0));
                    jot.status = "Only this program's folder is reachable.".into();
                }
            },
        }
        jot.draw(&mut window);
    }
}
