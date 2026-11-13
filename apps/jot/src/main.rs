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
//! misspelt word at the cursor, F11 closes.
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
use sex_textedit::{Doc, Motion};
use sex_vt::render::{cell_for_height, Cache, Painter};

const FILE: &[u8] = b"notes.txt";
const MAX_FILE: usize = 16 << 20;
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
}
impl Tab {
    fn new(path: &[u8], text: Vec<u8>) -> Tab {
        let mut doc = Doc::new(text);
        doc.move_to(Motion::End_, false);
        Tab { doc, hl: Highlighter::new(Lang::from_name(path)), path: path.to_vec(), top: 0, left: 0 }
    }
    fn name(&self) -> String { String::from_utf8_lossy(base_name(&self.path)).into_owned() }
}

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
}

/// What a key asked for that needs the window.
enum Ask { Nothing, Save, SaveAs, Open, Close, Escape }

impl Jot {
    fn tab(&mut self) -> &mut Tab { &mut self.tabs[self.active] }
    fn rows(&self) -> usize { (self.height / self.painter.cell_h).saturating_sub(2 + (self.bar != Bar::None) as usize).max(1) }
    fn gutter(&self) -> usize { if self.numbers { (self.tabs[self.active].doc.line_count().to_string_len()).max(3) + 1 } else { 0 } }
    fn cols(&self) -> usize { (self.width / self.painter.cell_w).saturating_sub(self.gutter()).max(4) }

    /// Keeps the cursor on screen.
    fn follow(&mut self) {
        let (rows, cols) = (self.rows(), self.cols());
        let t = &mut self.tabs[self.active];
        let line = t.doc.line_of(t.doc.cursor);
        if line < t.top { t.top = line; }
        if line >= t.top + rows { t.top = line + 1 - rows; }
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
            let label = format!(" {}{} ", t.name(), if t.doc.modified() { " •" } else { "" });
            let bg = if i == self.active { BG } else { BAR };
            let fg = if i == self.active { FG } else { DIM };
            if x + label.chars().count() * cw > w { break; }
            x = self.painter.text(cache, pixels, w, x, 0, &label, fg, bg);
        }
        if x < w { for y in 0..ch { pixels[y * w + x..y * w + w].fill(BAR); } }
        for px in 0..w.min(x) { if self.tabs.get(self.active).is_some() { let _ = px; } }

        // The text.
        let t = &mut self.tabs[self.active];
        let text = t.doc.text();
        let spans = t.hl.visible(text, t.top, rows);
        let cursor_line = t.doc.line_of(t.doc.cursor);
        let sel = t.doc.selection();
        let hits: Vec<(usize, usize)> = if self.bar != Bar::None && !self.needle.is_empty() { find::all(text, &self.needle, Options { ignore_case: true, whole_word: false }) } else { Vec::new() };
        let mut start = t.doc.offset_of_line(t.top);
        for r in 0..rows {
            let n = t.top + r;
            if n >= t.doc.line_count() { break; }
            let end = t.doc.line_end(start);
            let y0 = (r + 1) * ch;
            let line_bg = if n == cursor_line { CURRENT } else { BG };
            if line_bg != BG { for y in 0..ch { pixels[(y0 + y) * w + gutter * cw..(y0 + y) * w + w].fill(line_bg); } }
            if gutter > 0 {
                let num = format!("{:>width$} ", n + 1, width = gutter - 1);
                self.painter.text(cache, pixels, w, 0, y0, &num, if n == cursor_line { FG } else { DIM }, GUTTER);
            }
            // Prose only: in code the dictionary would mark every identifier.
            let prose = matches!(t.hl.lang, Lang::Plain | Lang::Markdown);
            let misspelt = if self.spelling && prose { self.spell.misspelt(&text[start..end]) } else { Vec::new() };
            let line_spans = spans.get(r).cloned().unwrap_or_default();
            let mut col = 0;
            let mut at = start;
            while at < end {
                let c_end = sex_textedit::next_char(text, at);
                let c = core::str::from_utf8(&text[at..c_end]).ok().and_then(|s| s.chars().next()).unwrap_or('?');
                let width = if c == '\t' { 4 - col % 4 } else { 1 };
                let rel = at - start;
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
            if n == cursor_line {
                let vis = display_col(text, start, t.doc.cursor);
                if vis >= t.left && vis <= t.left + cols {
                    let x0 = (gutter + vis - t.left) * cw;
                    for y in 1..ch - 1 { if x0 + 1 < w { pixels[(y0 + y) * w + x0] = ACCENT; pixels[(y0 + y) * w + x0 + 1] = ACCENT; } }
                }
            }
            start = (end + 1).min(text.len());
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
            let right = format!("Ln {}, Col {}  {}  {}{} ", line, col, t.hl.lang.name(), if self.spelling { "ABC✓  " } else { "" }, String::from_utf8_lossy(&t.path));
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
            (false, ..) => match sex_app::typed(key) { Some(c) => tab.doc.insert(&[c]), None => return Ask::Nothing },
            _ => return Ask::Nothing,
        }
        // Colouring below the first changed line may differ now.
        let tab = &mut self.tabs[self.active];
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
            (_, 0x0E) => { field.pop(); }
            (false, _) => match sex_app::typed(key) {
                Some(c) => { if self.fresh { field.clear(); } field.push(c); }
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
    let text = match read_all(window, path) {
        Ok(t) => { sex_pdx::serial_record!("[jot.loaded] bytes={} fnv={:016x}", t.len(), fnv(&t)); t }
        Err(e) => { sex_pdx::serial_record!("[jot.loaded] bytes=0 status={}", e); Vec::new() }
    };
    Tab::new(path, text)
}
fn save(window: &mut Window, tab: &mut Tab) -> Result<(), files::Error> {
    let text = tab.doc.text();
    files::write(window, &tab.path, text)?;
    sex_pdx::serial_record!("[jot.saved] bytes={} fnv={:016x}", text.len(), fnv(text));
    tab.doc.mark_saved();
    Ok(())
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
    let tabs: Vec<Tab> = paths.iter().map(|p| load(&mut window, p)).collect();
    let (w, h) = window.size();
    let (cw, ch) = cell_for_height(18);
    let mut jot = Jot {
        tabs, active: 0, painter: Painter::new(cw, ch), width: w.max(64), height: h.max(64), numbers: true, bar: Bar::None, field: 0,
        needle: Vec::new(), with: Vec::new(), status: "Ctrl+S saves. Ctrl+F finds. Ctrl+Z undoes.".into(), clipboard: Vec::new(),
        spell: spell::Checker::new(), spelling: true, scrolled: false, fresh: false,
    };
    jot.draw(&mut window);
    sex_pdx::serial_record!("[jot.ready] target={} task={} tabs={}", window.target, window.task, jot.tabs.len());
    loop {
        match window.next() {
            Event::Close => {
                sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task);
                window.close()
            }
            Event::Resize { width, height } => { jot.width = width.max(64); jot.height = height.max(64); }
            Event::Pointer(p) => {
                use sex_pdx::app_pointer::Kind as P;
                let rows = jot.rows();
                let (cw, ch, gutter) = (jot.painter.cell_w, jot.painter.cell_h, jot.gutter());
                let t = jot.tab();
                match p.kind {
                    P::Wheel { steps } => { let max = t.doc.line_count().saturating_sub(1); t.top = (t.top as isize - steps as isize * 3).clamp(0, max as isize) as usize; jot.scrolled = true; }
                    P::Down { button: 0 } | P::Move if p.kind != P::Move || p.buttons & 1 != 0 => {
                        let row = (p.y.max(0) as usize) / ch;
                        if row == 0 || row > rows { continue; }
                        let line = (t.top + row - 1).min(t.doc.line_count() - 1);
                        let start = t.doc.offset_of_line(line);
                        let want = ((p.x.max(0) as usize) / cw).saturating_sub(gutter) + t.left;
                        let end = t.doc.line_end(start);
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
                    jot.status = match r { Ok(()) => "Saved.".into(), Err(e) => { sex_pdx::serial_record!("[jot.save.failed] status={}", e); String::from_utf8_lossy(files::message(e)).into_owned() } };
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
                                jot.status = match save(&mut window, tab) { Ok(()) => "Saved.".into(), Err(e) => String::from_utf8_lossy(files::message(e)).into_owned() };
                            }
                        }
                        Err(e) => { sex_pdx::serial_record!("[jot.pick.refused] status={}", e); jot.status = "Nothing chosen.".into(); }
                    }
                }
                Ask::Close => {
                    if jot.tabs[jot.active].doc.modified() && jot.status != "Unsaved changes: Ctrl+W again closes anyway." {
                        jot.status = "Unsaved changes: Ctrl+W again closes anyway.".into();
                    } else if jot.tabs.len() > 1 {
                        jot.tabs.remove(jot.active);
                        jot.active = jot.active.min(jot.tabs.len() - 1);
                        jot.tab_record();
                        jot.status.clear();
                    } else {
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
