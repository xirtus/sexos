// SPDX-License-Identifier: GPL-3.0-only
//! Quil: the SexOS code editor. Files of any size (opened lazily, never copied whole),
//! tabs, a file tree, many cursors, Vim and Emacs keys beside VS Code's, a command
//! palette, Go to File and search and replace across the project folder. The editing
//! is `crates/quil-core`, proven by host tests; this file draws it and maps keys.
//!
//! VS Code keys (the default): Ctrl+Shift+P palette, Ctrl+P go to file, Ctrl+Shift+F
//! search the folder, Ctrl+B file tree, Ctrl+S save, Ctrl+O open, Ctrl+Shift+O open
//! folder, Ctrl+N new tab, Ctrl+W close tab, Ctrl+PageUp/PageDown switch tabs, Ctrl+D
//! next occurrence, Ctrl+Shift+L all occurrences, Ctrl+Shift+Up/Down add cursors,
//! Ctrl+/ comment, Ctrl+F find, Ctrl+H replace, Ctrl+L go to line, Escape one cursor.
//! The palette switches to Vim mode or Emacs keys.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use quil_core::buffer::{Buffer, MemSource, Source};
use quil_core::editor::{Editor, Motion};
use quil_core::highlight::{self, Kind};
use quil_core::keymap::{self, Command, Keymap, Keys, Lookup, Named, Press, K};
use quil_core::project::{self, Entry, Fs, Query, Tree};
use quil_core::vim::{self, VKey, Vim};
use quil_core::workspace::{Tab, Tabs};
use quil_core::fuzzy;
use sex_app::{files, Event, Window};
use sex_pdx::app_key::{us_char, Key};
use sex_pdx::app_pointer::Kind as PKind;

/// Files up to this size are read into memory when opened (so undo survives saving);
/// bigger ones stay on disk and are read a block at a time.
const IN_MEMORY: u64 = 32 << 20;
const CELL_W: usize = 9;
const CELL_H: usize = 18;
const TREE_COLS: usize = 28;

const BG: u32 = 0x1E1E2E;
const FG: u32 = 0xCDD6F4;
const DIM: u32 = 0x6C7086;
const PANEL: u32 = 0x181825;
const BAR: u32 = 0x11111B;
const CURLINE: u32 = 0x262637;
const SELECTED: u32 = 0x45475A;
const ACCENT: u32 = 0x89B4FA;
const CURSOR: u32 = 0xF5E0DC;
const HIT: u32 = 0x5B4A2A;

fn color(k: Kind) -> u32 {
    match k { Kind::Plain => FG, Kind::Keyword => 0xCBA6F7, Kind::Type => 0xF9E2AF, Kind::String => 0xA6E3A1, Kind::Number => 0xFAB387,
        Kind::Comment => 0x7F849C, Kind::Punct => 0x9399B2, Kind::Function => 0x89B4FA }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[quil.code.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn fnv(h: &mut u64, bytes: &[u8]) { for b in bytes { *h = (*h ^ *b as u64).wrapping_mul(0x100000001b3); } }
fn lossy(b: &[u8]) -> String { String::from_utf8_lossy(b).into_owned() }

// ---- Files ----

/// A file read on demand through Files.
struct FileSource { path: Vec<u8>, len: usize }
impl Source for FileSource {
    fn len(&self) -> usize { self.len }
    fn read(&self, at: usize, out: &mut [u8]) -> usize { files::direct::read_at(&self.path, at as u64, out).unwrap_or(0) }
}

/// The project folder through Files.
struct AppFs;
impl Fs for AppFs {
    fn list(&mut self, path: &[u8]) -> Option<Vec<Entry>> {
        files::direct::list(path).ok().map(|v| v.into_iter().map(|e| Entry { name: e.name, dir: e.dir, size: e.size }).collect())
    }
    fn read(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        let st = files::direct::stat(path).ok()?;
        let mut v = vec![0u8; st.size as usize];
        let n = files::direct::read_at(path, 0, &mut v).ok()?;
        v.truncate(n);
        Some(v)
    }
    fn write(&mut self, path: &[u8], data: &[u8]) -> bool {
        if files::direct::stat(path).is_err() && files::direct::create(path).is_err() { return false; }
        files::direct::truncate(path, 0).is_ok() && files::direct::write_at(path, 0, data).is_ok_and(|n| n == data.len())
    }
}

fn open_buffer(path: &[u8]) -> Result<Buffer, u8> {
    let st = files::direct::stat(path)?;
    if st.dir { return Err(files::NOT_FOUND); }
    if st.size <= IN_MEMORY {
        let mut v = vec![0u8; st.size as usize];
        let n = files::direct::read_at(path, 0, &mut v)?;
        v.truncate(n);
        Ok(Buffer::new(Box::new(MemSource(v))))
    } else {
        Ok(Buffer::new(Box::new(FileSource { path: path.to_vec(), len: st.size as usize })))
    }
}

/// Writes the buffer to `path`, streaming it; returns (bytes, FNV-1a hash).
fn write_buffer(buf: &Buffer, path: &[u8]) -> Result<(usize, u64), u8> {
    match files::direct::stat(path) { Ok(_) => files::direct::truncate(path, 0)?, Err(_) => files::direct::create(path)? }
    let mut at = 0usize;
    let mut h = 0xcbf29ce484222325u64;
    buf.chunks(|c| {
        let n = files::direct::write_at(path, at as u64, c)?;
        if n < c.len() { return Err(sex_pdx::fs::NO_SPACE); }
        fnv(&mut h, c);
        at += n;
        Ok(())
    })?;
    Ok((at, h))
}

// ---- Drawing ----

struct Painter {
    px: &'static mut [u32],
    font: sex_font::Font<'static>,
    acc: Vec<i32>,
    glyphs: BTreeMap<u32, Vec<u8>>,
    w: usize,
    h: usize,
}
impl Painter {
    fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: u32) {
        let p = &mut *self.px;
        if self.w * self.h > p.len() { return; }
        for row in y.min(self.h)..(y + h).min(self.h) { let s = row * self.w; p[s + x.min(self.w)..s + (x + w).min(self.w)].fill(c); }
    }
    /// One character in cell (col, row).
    fn ch(&mut self, col: usize, row: usize, c: char, fg: u32, bg: u32) {
        let (x0, y0) = (col * CELL_W, row * CELL_H);
        if x0 + CELL_W > self.w || y0 + CELL_H > self.h { return; }
        self.rect(x0, y0, CELL_W, CELL_H, bg);
        if c == ' ' { return; }
        let scalar = c as u32;
        if !self.glyphs.contains_key(&scalar) {
            let mut cover = vec![0u8; CELL_W * CELL_H];
            self.acc.fill(0);
            let ok = sex_font::draw_cell(&self.font, scalar, CELL_W, CELL_H, &mut self.acc, &mut cover);
            if !ok { cover.fill(0); for y in 3..CELL_H - 3 { cover[y * CELL_W + 1] = 160; cover[y * CELL_W + CELL_W - 2] = 160; } }
            if self.glyphs.len() > 4096 { self.glyphs.clear(); }
            self.glyphs.insert(scalar, cover);
        }
        let cover = &self.glyphs[&scalar];
        let p = &mut *self.px;
        for y in 0..CELL_H {
            for x in 0..CELL_W {
                let a = cover[y * CELL_W + x] as u32;
                if a == 0 { continue; }
                let i = (y0 + y) * self.w + x0 + x;
                p[i] = blend(bg, fg, a);
            }
        }
    }
    fn text(&mut self, col: usize, row: usize, max: usize, s: &str, fg: u32, bg: u32) -> usize {
        let mut n = 0;
        for c in s.chars() { if n >= max { break; } self.ch(col + n, row, c, fg, bg); n += 1; }
        n
    }
}
fn blend(bg: u32, fg: u32, a: u32) -> u32 {
    let mix = |s: u32| { let (b, f) = ((bg >> s) & 0xFF, (fg >> s) & 0xFF); ((b * (255 - a) + f * a) / 255) << s };
    mix(16) | mix(8) | mix(0)
}

// ---- The editor ----

#[derive(PartialEq, Eq, Clone, Copy)]
enum Focus { Editor, Tree }

enum Overlay {
    None,
    Palette { query: String, sel: usize },
    QuickOpen { query: String, sel: usize, files: Vec<Vec<u8>> },
    Find { query: String },
    Replace { find: String, with: String, field: usize },
    GotoLine { query: String },
    Search { query: String, with: String, field: usize, hits: Vec<project::Hit>, sel: usize, note: String },
}

struct App {
    tabs: Tabs,
    tree: Tree,
    show_tree: bool,
    focus: Focus,
    root: Vec<u8>,
    keymap: Keymap,
    keys: Keys,
    vim: Vim,
    overlay: Overlay,
    clipboard: (Vec<u8>, bool),
    status: String,
    numbers: bool,
    find: Vec<u8>,
    /// Emacs mark: selections extend while it is set.
    mark: bool,
    quit_armed: bool,
    drag: bool,
    p: Painter,
}

fn press(key: Key) -> Option<Press> {
    let named = match key.code {
        0x01 => Some(Named::Esc), 0x0E => Some(Named::Backspace), 0x0F => Some(Named::Tab), 0x1C => Some(Named::Enter),
        0x48 | 0x67 => Some(Named::Up), 0x50 | 0x6C => Some(Named::Down), 0x4B | 0x69 => Some(Named::Left), 0x4D | 0x6A => Some(Named::Right),
        0x47 | 0x66 => Some(Named::Home), 0x4F | 0x6B => Some(Named::End), 0x49 | 0x68 => Some(Named::PageUp), 0x51 | 0x6D => Some(Named::PageDown),
        0x53 | 0x6F => Some(Named::Delete), 0x3B..=0x44 => Some(Named::F(key.code - 0x3A)), 0x57 => Some(Named::F(11)), 0x58 => Some(Named::F(12)),
        _ => None,
    };
    if let Some(n) = named { return Some(Press { k: K::Named(n), ctrl: key.control, shift: key.shift }); }
    if key.control {
        // The letter the key is, whatever Ctrl does to its text.
        let c = us_char(Key::new(key.code, false, false))? as char;
        let c = if key.shift { match c { '/' => '/', '-' => '_', c => c } } else { c };
        return Some(Press { k: K::Char(c), ctrl: true, shift: key.shift });
    }
    key.char().map(|c| Press { k: K::Char(c), ctrl: false, shift: false })
}

impl App {
    fn rows(&self) -> usize { (self.p.h / CELL_H).saturating_sub(2).max(1) }
    fn cols(&self) -> usize { self.p.w / CELL_W }
    fn tree_cols(&self) -> usize { if self.show_tree { TREE_COLS.min(self.cols() / 3) } else { 0 } }
    fn gutter(&self) -> usize { if self.numbers { 7 } else { 1 } }

    fn tab(&mut self) -> &mut Tab {
        if self.tabs.is_empty() { self.tabs.add(Tab::new(b"", Editor::from_bytes(b""))); }
        let i = self.tabs.active;
        &mut self.tabs.list[i]
    }

    fn open(&mut self, path: &[u8]) {
        if let Some(i) = self.tabs.find(path) { self.tabs.active = i; return; }
        match open_buffer(path) {
            Ok(buf) => {
                let len = buf.len();
                let mut ed = Editor::new(buf);
                ed.indent = b"    ".to_vec();
                // Replace an empty, untouched untitled tab rather than keep it.
                if self.tabs.len() == 1 && self.tabs.list[0].path.is_empty() && self.tabs.list[0].ed.buf.is_empty() && !self.tabs.list[0].ed.modified() { self.tabs.close(0); }
                self.tabs.add(Tab::new(path, ed));
                sex_pdx::serial_record!("[quil.code.opened] path={} bytes={} lazy={}", lossy(path), len, (len as u64 > IN_MEMORY) as u8);
                self.status = format!("Opened {} ({} bytes)", lossy(path), len);
            }
            Err(e) => {
                // A new file: an empty tab that saves to that path.
                if e == files::NOT_FOUND {
                    self.tabs.add(Tab::new(path, Editor::from_bytes(b"")));
                    self.status = format!("New file {}", lossy(path));
                } else { self.status = lossy(files::message(e)); }
            }
        }
    }

    fn save(&mut self, as_new: bool, window: &mut Window) {
        let mut path = self.tab().path.clone();
        if path.is_empty() || as_new {
            let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
            let name = if path.is_empty() { b"untitled.txt".to_vec() } else { self.tab().name().to_vec() };
            match files::pick(window, files::Pick::Save, &name, &mut out) {
                Ok(p) => path = p.to_vec(),
                Err(_) => { self.status = String::from("Not saved."); return; }
            }
        }
        let lazy = self.tab().ed.buf.len() as u64 > IN_MEMORY && !as_new;
        // A big file is read from the very file we would overwrite: write beside it, then
        // swap, then read from the new one.
        let target = if lazy { let mut t = path.clone(); t.extend_from_slice(b".quil-save"); t } else { path.clone() };
        let result = write_buffer(&self.tab().ed.buf, &target);
        match result {
            Ok((n, h)) => {
                if lazy {
                    let _ = files::direct::remove(&path);
                    if let Err(e) = files::direct::rename(&target, &path) { self.status = format!("Saved as {}: {}", lossy(&target), lossy(files::message(e))); return; }
                    let cursor = self.tab().ed.cursor();
                    let t = self.tab();
                    t.ed.buf = Buffer::new(Box::new(FileSource { path: path.clone(), len: n }));
                    t.ed.forget_history();
                    t.ed.set_cursor(cursor);
                }
                let _ = files::direct::sync();
                let t = self.tab();
                t.path = path.clone();
                t.lang = highlight::Lang::from_path(&path);
                t.ed.mark_saved();
                sex_pdx::serial_record!("[quil.code.saved] path={} bytes={} hash={:016x}", lossy(&path), n, h);
                self.status = format!("Saved {} bytes", n);
            }
            Err(e) => { sex_pdx::serial_record!("[quil.code.save.failed] status={}", e); self.status = lossy(files::message(e)); }
        }
    }

    fn set_root(&mut self, root: &[u8]) {
        self.root = root.to_vec();
        self.tree = Tree::new(root);
        self.tree.refresh(&mut AppFs);
        self.show_tree = true;
    }

    fn run(&mut self, c: Command, window: &mut Window) {
        let rows = self.rows();
        self.tab();
        let i = self.tabs.active;
        let comment = self.tabs.list[i].lang.comment();
        if c != Command::Quit { self.quit_armed = false; }
        match c {
            Command::Save => self.save(false, window),
            Command::SaveAs => self.save(true, window),
            Command::Open => {
                let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                if let Ok(p) = files::pick(window, files::Pick::Open, b"", &mut out) { let p = p.to_vec(); self.open(&p); }
            }
            Command::OpenFolder => {
                let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                if let Ok(p) = files::pick(window, files::Pick::Folder, b"", &mut out) { let p = p.to_vec(); self.set_root(&p); }
            }
            Command::NewTab => { self.tabs.add(Tab::new(b"", Editor::from_bytes(b""))); }
            Command::CloseTab => {
                if self.tab().ed.modified() && !self.quit_armed { self.quit_armed = true; self.status = String::from("Unsaved changes: Close Tab again to discard."); return; }
                self.quit_armed = false;
                let i = self.tabs.active;
                self.tabs.close(i);
            }
            Command::NextTab => self.tabs.next(),
            Command::PrevTab => self.tabs.prev(),
            Command::Quit => {
                if self.tabs.list.iter().any(|t| t.ed.modified()) && !self.quit_armed { self.quit_armed = true; self.status = String::from("Unsaved changes: Quit again to discard them."); return; }
                sex_pdx::serial_record!("[quil.code.closed] target={} task={}", window.target, window.task);
                sex_pdx::task_finish(0);
            }
            Command::Undo => { self.tabs.list[i].ed.undo(); }
            Command::Redo => { self.tabs.list[i].ed.redo(); }
            Command::Copy => { self.clipboard = self.tabs.list[i].ed.copy(); self.mark = false; }
            Command::Cut => { self.clipboard = self.tabs.list[i].ed.cut(); self.mark = false; }
            Command::Paste => { let (t, w) = self.clipboard.clone(); self.tabs.list[i].ed.paste(&t, w); }
            Command::SelectAll => self.tabs.list[i].ed.select_all(),
            Command::Find => self.overlay = Overlay::Find { query: lossy(&self.find) },
            Command::FindNext | Command::FindPrev => self.find_step(c == Command::FindNext),
            Command::Replace => self.overlay = Overlay::Replace { find: lossy(&self.find), with: String::new(), field: 0 },
            Command::GotoLine => self.overlay = Overlay::GotoLine { query: String::new() },
            Command::Palette => self.overlay = Overlay::Palette { query: String::new(), sel: 0 },
            Command::QuickOpen => {
                let root = self.root.clone();
                let files: Vec<Vec<u8>> = project::walk(&mut AppFs, &root, 50_000).into_iter().map(|x| x.0).collect();
                sex_pdx::serial_record!("[quil.code.quickopen] files={}", files.len());
                self.overlay = Overlay::QuickOpen { query: String::new(), sel: 0, files };
            }
            Command::ProjectSearch => {
                let q = if self.tabs.list[i].ed.primary_sel().is_empty() { String::new() } else { { let e = &self.tabs.list[i].ed; lossy(&e.selected(&e.primary_sel())) } };
                self.overlay = Overlay::Search { query: q, with: String::new(), field: 0, hits: Vec::new(), sel: 0, note: String::new() };
            }
            Command::ToggleTree => {
                if !self.show_tree { self.show_tree = true; self.tree.refresh(&mut AppFs); self.focus = Focus::Tree; }
                else if self.focus == Focus::Tree { self.show_tree = false; self.focus = Focus::Editor; } else { self.focus = Focus::Tree; }
            }
            Command::ToggleNumbers => self.numbers = !self.numbers,
            Command::AddCursorAbove => self.tabs.list[i].ed.add_cursor(false),
            Command::AddCursorBelow => self.tabs.list[i].ed.add_cursor(true),
            Command::AddNextOccurrence => { self.tabs.list[i].ed.add_next_occurrence(); }
            Command::SelectAllOccurrences => { let n = self.tabs.list[i].ed.select_all_occurrences(); self.status = format!("{n} selections"); }
            Command::SplitIntoLines => self.tabs.list[i].ed.split_into_lines(),
            Command::Collapse => { self.tabs.list[i].ed.collapse(); self.mark = false; }
            Command::Indent => self.tabs.list[i].ed.indent_lines(),
            Command::Outdent => self.tabs.list[i].ed.outdent_lines(),
            Command::ToggleComment => self.tabs.list[i].ed.toggle_comment(comment),
            Command::DuplicateLine => self.tabs.list[i].ed.duplicate_lines(),
            Command::DeleteLine => self.tabs.list[i].ed.delete_lines(),
            Command::MoveLineUp => self.tabs.list[i].ed.move_lines(false),
            Command::MoveLineDown => self.tabs.list[i].ed.move_lines(true),
            Command::JoinLines => self.tabs.list[i].ed.join_lines(),
            Command::DeleteWordBack => self.tabs.list[i].ed.delete_word_back(),
            Command::DeleteForward => self.tabs.list[i].ed.delete_forward(),
            Command::KillLine => {
                let (h, e) = (self.tabs.list[i].ed.cursor(), self.tabs.list[i].ed.buf.line_end(self.tabs.list[i].ed.cursor()));
                let e = if e == h { self.tabs.list[i].ed.buf.next_char(h) } else { e };
                self.tabs.list[i].ed.set_selection(h, e);
                self.clipboard = self.tabs.list[i].ed.cut();
            }
            Command::SetMark => { self.mark = true; let h = self.tabs.list[i].ed.cursor(); self.tabs.list[i].ed.set_selection(h, h); self.status = String::from("Mark set"); }
            Command::KeymapVsCode | Command::KeymapVim | Command::KeymapEmacs => {
                self.keymap = match c { Command::KeymapVim => Keymap::Vim, Command::KeymapEmacs => Keymap::Emacs, _ => Keymap::VsCode };
                self.vim = Vim::new();
                sex_pdx::serial_record!("[quil.code.keymap] {}", match self.keymap { Keymap::Vim => "vim", Keymap::Emacs => "emacs", Keymap::VsCode => "vscode" });
                self.status = String::from(c.name());
            }
            Command::Left | Command::Right | Command::Up | Command::Down | Command::WordLeft | Command::WordRight | Command::LineStart | Command::LineEnd
            | Command::DocStart | Command::DocEnd | Command::PageUp | Command::PageDown => {
                let m = match c { Command::Left => Motion::Left, Command::Right => Motion::Right, Command::Up => Motion::Up, Command::Down => Motion::Down,
                    Command::WordLeft => Motion::WordLeft, Command::WordRight => Motion::WordRight, Command::LineStart => Motion::Home, Command::LineEnd => Motion::End,
                    Command::DocStart => Motion::DocStart, Command::DocEnd => Motion::DocEnd, Command::PageUp => Motion::PageUp(rows), _ => Motion::PageDown(rows) };
                let extend = self.mark;
                self.tab().ed.move_all(m, extend);
            }
        }
    }

    fn find_step(&mut self, forward: bool) {
        let needle = self.find.clone();
        if needle.is_empty() { return; }
        let ed = &mut self.tab().ed;
        let s = ed.primary_sel();
        let hit = if forward { ed.buf.find(&needle, s.end(), true).or_else(|| ed.buf.find(&needle, 0, true)) }
                  else { ed.buf.rfind(&needle, s.start(), true).or_else(|| ed.buf.rfind(&needle, ed.buf.len(), true)) };
        match hit {
            Some(at) => { ed.set_selection(at, at + needle.len()); sex_pdx::serial_record!("[quil.code.find] at={}", at); }
            None => self.status = String::from("No match."),
        }
    }

    // ---- Keys ----

    fn key(&mut self, key: Key, window: &mut Window) {
        let Some(p) = press(key) else { return };
        if !matches!(self.overlay, Overlay::None) { self.overlay_key(p, window); return; }
        if self.focus == Focus::Tree && !p.ctrl {
            match p.k {
                K::Named(Named::Up) => self.tree.up(),
                K::Named(Named::Down) => self.tree.down(),
                K::Named(Named::Enter) | K::Named(Named::Right) | K::Named(Named::Left) => {
                    if let Some(path) = self.tree.activate(&mut AppFs) { let full = project::join(&self.root, &path); self.open(&full); self.focus = Focus::Editor; }
                }
                K::Named(Named::Esc) => self.focus = Focus::Editor,
                _ => {}
            }
            return;
        }
        if self.keymap == Keymap::Vim {
            let vk = match p.k {
                K::Char(c) if !p.ctrl => Some(VKey::Char(c)),
                K::Char(c) if p.ctrl && !p.shift && matches!(c, 'r' | 'd' | 'u' | 'v' | 'w') => Some(VKey::Ctrl(c)),
                K::Named(Named::Esc) => Some(VKey::Esc), K::Named(Named::Enter) => Some(VKey::Enter), K::Named(Named::Backspace) if !p.ctrl => Some(VKey::Backspace),
                K::Named(Named::Delete) => Some(VKey::Delete), K::Named(Named::Tab) => Some(VKey::Tab),
                K::Named(Named::Left) if !p.ctrl => Some(VKey::Left), K::Named(Named::Right) if !p.ctrl => Some(VKey::Right),
                K::Named(Named::Up) if !p.ctrl => Some(VKey::Up), K::Named(Named::Down) if !p.ctrl => Some(VKey::Down),
                _ => None,
            };
            if let Some(vk) = vk {
                self.vim.half_page = self.rows() / 2;
                self.tab();
                let i = self.tabs.active;
                let act = self.vim.key(&mut self.tabs.list[i].ed, vk);
                sex_pdx::serial_record!("[quil.code.vim] mode={} len={}", self.vim.label(), self.tab().ed.buf.len());
                match act {
                    vim::Action::None => {}
                    vim::Action::Save => self.save(false, window),
                    vim::Action::SaveQuit => { self.save(false, window); self.quit_armed = true; self.run(Command::Quit, window); }
                    vim::Action::Quit => { self.quit_armed = true; self.run(Command::Quit, window); }
                    vim::Action::ForceQuit => { self.quit_armed = true; self.run(Command::Quit, window); }
                    vim::Action::Message(m) => self.status = m,
                }
                return;
            }
        }
        match self.keys.lookup(self.keymap, p) {
            Lookup::Run(c) => { self.run(c, window); return; }
            Lookup::Pending => { self.status = String::from(self.keys.pending().unwrap_or("")); return; }
            Lookup::Unbound => {}
        }
        let rows = self.rows();
        let mark = self.mark;
        let ed = &mut self.tab().ed;
        let extend = p.shift || mark;
        match p.k {
            K::Char(c) if !p.ctrl => { let mut b = [0u8; 4]; ed.type_text(c.encode_utf8(&mut b).as_bytes()); }
            K::Named(Named::Enter) => ed.type_text(b"\n"),
            K::Named(Named::Backspace) => ed.backspace(),
            K::Named(Named::Delete) => ed.delete_forward(),
            K::Named(Named::Tab) => if p.shift { ed.outdent_lines() } else { ed.tab() },
            K::Named(Named::Esc) => { ed.collapse(); self.mark = false; }
            K::Named(n) => {
                let m = match (n, p.ctrl) {
                    (Named::Left, false) => Motion::Left, (Named::Right, false) => Motion::Right, (Named::Left, true) => Motion::WordLeft, (Named::Right, true) => Motion::WordRight,
                    (Named::Up, _) => Motion::Up, (Named::Down, _) => Motion::Down, (Named::Home, _) => Motion::Home, (Named::End, _) => Motion::End,
                    (Named::PageUp, _) => Motion::PageUp(rows), (Named::PageDown, _) => Motion::PageDown(rows),
                    _ => return,
                };
                ed.move_all(m, extend);
            }
            _ => {}
        }
    }

    fn overlay_key(&mut self, p: Press, window: &mut Window) {
        let esc = p.k == K::Named(Named::Esc);
        if esc { self.overlay = Overlay::None; return; }
        let typed = match p.k { K::Char(c) if !p.ctrl => Some(c), _ => None };
        let bs = p.k == K::Named(Named::Backspace);
        let enter = p.k == K::Named(Named::Enter);
        let updown = match p.k { K::Named(Named::Up) => -1, K::Named(Named::Down) => 1, _ => 0 };
        let edit = |s: &mut String| { if let Some(c) = typed { s.push(c) } else if bs { s.pop(); } };
        let mut take = Overlay::None;
        core::mem::swap(&mut take, &mut self.overlay);
        let mut keep = true;
        match &mut take {
            Overlay::Palette { query, sel } => {
                edit(query);
                let names: Vec<&str> = Command::ALL.iter().map(|c| c.name()).collect();
                let ranked = fuzzy::rank(query, &names, 200);
                if typed.is_some() || bs { *sel = 0; }
                *sel = (*sel as isize + updown).clamp(0, ranked.len().saturating_sub(1) as isize) as usize;
                if enter {
                    keep = false;
                    if let Some((i, _)) = ranked.get(*sel) { let c = Command::ALL[*i]; sex_pdx::serial_record!("[quil.code.palette] run={}", c.name()); self.overlay = Overlay::None; self.run(c, window); }
                }
            }
            Overlay::QuickOpen { query, sel, files } => {
                edit(query);
                let ranked = fuzzy::rank(query, files, 200);
                if typed.is_some() || bs { *sel = 0; }
                *sel = (*sel as isize + updown).clamp(0, ranked.len().saturating_sub(1) as isize) as usize;
                if enter {
                    keep = false;
                    if let Some((i, _)) = ranked.get(*sel) { let path = project::join(&self.root, &files[*i]); self.open(&path); self.tree.reveal(&mut AppFs, &files[*i]); }
                }
            }
            Overlay::Find { query } => {
                edit(query);
                self.find = query.as_bytes().to_vec();
                if enter || typed.is_some() {
                    // Search as you type, from the selection's start.
                    if typed.is_some() { let s = self.tab().ed.primary_sel().start(); self.tab().ed.set_cursor(s); }
                    self.find_step(true);
                }
                if p.k == K::Named(Named::Up) { self.find_step(false); }
            }
            Overlay::Replace { find, with, field } => {
                if p.k == K::Named(Named::Tab) || updown != 0 { *field ^= 1; }
                edit(if *field == 0 { find } else { with });
                self.find = find.as_bytes().to_vec();
                if enter {
                    let (f, w) = (find.as_bytes().to_vec(), with.as_bytes().to_vec());
                    let ed = &mut self.tab().ed;
                    // Replace the current match, then go to the next.
                    let s = ed.primary_sel();
                    if !f.is_empty() && ed.selected(&s).eq_ignore_ascii_case(&f) { ed.edit(|_, s| (s.start()..s.end(), w.clone(), quil_core::editor::After::End)); }
                    self.find_step(true);
                }
                if p.ctrl && p.k == K::Char('a') {
                    let (f, w) = (find.as_bytes().to_vec(), with.as_bytes().to_vec());
                    let ed = &mut self.tab().ed;
                    ed.set_selection(0, 0);
                    let mut v = Vec::new();
                    let mut at = 0;
                    while let Some(h) = ed.buf.find(&f, at, true) { v.push(quil_core::editor::Sel { anchor: h, head: h + f.len(), col: None }); at = h + f.len().max(1); }
                    let n = v.len();
                    if n > 0 { ed.sels = v; ed.primary = 0; ed.edit(|_, s| (s.start()..s.end(), w.clone(), quil_core::editor::After::End)); ed.collapse(); }
                    self.status = format!("Replaced {n}");
                }
            }
            Overlay::GotoLine { query } => {
                if typed.is_some_and(|c| c.is_ascii_digit()) || bs { edit(query); }
                if enter {
                    keep = false;
                    let n: usize = query.parse().unwrap_or(1);
                    let ed = &mut self.tab().ed;
                    match ed.buf.offset_of_line(n.saturating_sub(1)) { Some(at) => ed.set_cursor(at), None => {
                        // The index isn't done: finish it now (a big file takes a moment).
                        while !ed.buf.index_step(256) {}
                        let at = ed.buf.offset_of_line(n.saturating_sub(1)).unwrap_or(0);
                        ed.set_cursor(at);
                    } }
                }
            }
            Overlay::Search { query, with, field, hits, sel, note } => {
                if p.k == K::Named(Named::Tab) { *field ^= 1; }
                edit(if *field == 0 { query } else { with });
                if !hits.is_empty() { *sel = (*sel as isize + updown).clamp(0, hits.len() as isize - 1) as usize; }
                let q = Query { needle: query.as_bytes().to_vec(), fold_case: true, whole_word: false };
                if p.ctrl && p.k == K::Char('a') && !q.needle.is_empty() {
                    let root = self.root.clone();
                    let (f, n) = project::replace_all(&mut AppFs, &root, &q, with.as_bytes());
                    *note = format!("Replaced {n} in {f} files");
                    sex_pdx::serial_record!("[quil.code.replace.project] files={} count={}", f, n);
                    hits.clear();
                } else if enter && (hits.is_empty() || typed.is_some()) || (enter && *field == 0 && *sel == 0 && note.is_empty()) {
                    let root = self.root.clone();
                    let t = sex_pdx::get_ticks();
                    let (h, read) = project::search(&mut AppFs, &root, &q, 2000);
                    let ms = sex_pdx::get_ticks().saturating_sub(t);
                    *note = format!("{} matches in {} files read, {} ms", h.len(), read, ms);
                    sex_pdx::serial_record!("[quil.code.search] matches={} files={} ms={}", h.len(), read, ms);
                    *hits = h;
                    *sel = 0;
                    if hits.is_empty() { /* stay */ } else { *note = format!("{} (Enter again opens; Up/Down choose)", note); }
                } else if enter {
                    if let Some(h) = hits.get(*sel).cloned() {
                        keep = false;
                        let path = project::join(&self.root, &h.path);
                        self.open(&path);
                        let n = q.needle.len();
                        self.tab().ed.set_selection(h.offset, h.offset + n);
                    }
                }
            }
            Overlay::None => keep = false,
        }
        if keep && matches!(self.overlay, Overlay::None) { self.overlay = take; }
    }

    // ---- Pointer ----

    fn pointer(&mut self, ptr: sex_pdx::app_pointer::Pointer) {
        let (col, row) = ((ptr.x.max(0) as usize) / CELL_W, (ptr.y.max(0) as usize) / CELL_H);
        let tc = self.tree_cols();
        match ptr.kind {
            PKind::Wheel { steps } => {
                if col < tc { return; }
                let rows = self.rows() as isize;
                let t = self.tab();
                t.scroll(-(steps as isize) * 3.min(rows));
            }
            PKind::Down { button: 0 } => {
                if row == 0 {
                    // The tab bar: which tab is under the pointer.
                    let mut x = tc;
                    for (i, t) in self.tabs.list.iter().enumerate() {
                        let w = t.name().len() + 4;
                        if col >= x && col < x + w { self.tabs.active = i; return; }
                        x += w;
                    }
                    return;
                }
                if col < tc {
                    let i = row - 1;
                    if i < self.tree.rows.len() {
                        self.tree.selected = i;
                        if let Some(path) = self.tree.activate(&mut AppFs) { let full = project::join(&self.root, &path); self.open(&full); }
                    }
                    self.focus = Focus::Tree;
                    return;
                }
                self.focus = Focus::Editor;
                if let Some(at) = self.offset_at(col, row) { self.tab().ed.set_cursor(at); self.drag = true; }
            }
            PKind::Move if self.drag && ptr.buttons & 1 != 0 => {
                if let Some(at) = self.offset_at(col, row) { let a = self.tab().ed.primary_sel().anchor; self.tab().ed.set_selection(a, at); }
            }
            PKind::Up { .. } => self.drag = false,
            _ => {}
        }
    }
    /// The text offset under a cell of the editor area.
    fn offset_at(&mut self, col: usize, row: usize) -> Option<usize> {
        if row == 0 || row > self.rows() { return None; }
        let x0 = self.tree_cols() + self.gutter();
        let t = self.tab();
        let mut line = t.top;
        for _ in 1..row { let n = t.ed.buf.next_line(line); if n == line || (n >= t.ed.buf.len() && t.ed.buf.line_end(line) >= t.ed.buf.len()) { break; } line = n; }
        let want = (col.saturating_sub(x0)) + t.left;
        // Walk the line counting display columns (tabs are four wide).
        let end = t.ed.buf.line_end(line);
        let mut at = line;
        let mut dc = 0;
        while at < end && dc < want { dc += if t.ed.buf.byte(at) == Some(b'\t') { 4 } else { 1 }; at = t.ed.buf.next_char(at); }
        Some(at)
    }

    // ---- Drawing ----

    fn draw(&mut self, window: &mut Window) {
        let (w, h) = window.size();
        self.p.w = w;
        self.p.h = h;
        let rows = self.rows();
        let cols = self.cols();
        let tc = self.tree_cols();
        let gutter = self.gutter();
        self.p.rect(0, 0, w, h, BG);
        // Tab bar.
        self.p.rect(0, 0, w, CELL_H, BAR);
        if tc > 0 { self.p.text(0, 0, tc, " FOLDER", DIM, BAR); }
        let mut x = tc;
        for i in 0..self.tabs.len() {
            let t = &self.tabs.list[i];
            let label = format!(" {}{} ", lossy(t.name()), if t.ed.modified() { " *" } else { "  " });
            let (fg, bg) = if i == self.tabs.active { (FG, BG) } else { (DIM, BAR) };
            x += self.p.text(x, 0, cols.saturating_sub(x), &label, fg, bg);
            if x < cols { self.p.ch(x, 0, '|', PANEL, BAR); x += 1; }
        }
        // File tree.
        if tc > 0 {
            self.p.rect(0, CELL_H, tc * CELL_W, rows * CELL_H, PANEL);
            let first = self.tree.selected.saturating_sub(rows.saturating_sub(1));
            for (r, i) in (first..self.tree.rows.len()).enumerate().take(rows) {
                let row = self.tree.rows[i].clone();
                let sel = i == self.tree.selected;
                let bg = if sel { if self.focus == Focus::Tree { SELECTED } else { CURLINE } } else { PANEL };
                self.p.rect(0, (r + 1) * CELL_H, tc * CELL_W, CELL_H, bg);
                let icon = if row.dir { if row.open { "v " } else { "> " } } else { "  " };
                let s = format!("{:width$}{}{}", "", icon, lossy(&row.name), width = row.depth * 2);
                self.p.text(0, r + 1, tc - 1, &s, if row.dir { ACCENT } else { FG }, bg);
            }
        }
        // Editor.
        let ex = tc + gutter;
        let text_cols = cols.saturating_sub(ex);
        let numbers = self.numbers;
        let vim_block = self.keymap == Keymap::Vim && self.vim.block_cursor();
        {
            let t = self.tab();
            t.follow(rows);
            // Horizontal scroll follows the primary cursor.
            let c = t.ed.cursor();
            let ls = t.ed.buf.line_start(c);
            let line = t.ed.buf.slice(ls..c);
            let ccol = display_cols(&line);
            if ccol < t.left { t.left = ccol.saturating_sub(8); }
            if ccol >= t.left + text_cols { t.left = ccol + 8 - text_cols; }
        }
        let i = self.tabs.active;
        let t = &self.tabs.list[i];
        let first_line = t.ed.buf.line_of(t.top);
        let primary_line = t.ed.buf.line_start(t.ed.cursor());
        let mut sels: Vec<(usize, usize)> = t.ed.sels.iter().map(|s| (s.start(), s.end())).collect();
        sels.sort_unstable();
        let heads: Vec<usize> = { let mut v: Vec<usize> = t.ed.sels.iter().map(|s| s.head).collect(); v.sort_unstable(); v };
        let lang = t.lang;
        let left = t.left;
        let find = self.find.clone();
        let mut spans = Vec::new();
        let mut line = t.top;
        let mut in_comment = false;
        let mut cells: Vec<(usize, usize, char, u32, u32)> = Vec::new();
        let mut cursors: Vec<(usize, usize)> = Vec::new();
        for r in 0..rows {
            let y = r + 1;
            let end = t.ed.buf.line_end(line);
            let shown_end = end.min(line + 4096);
            let bytes = t.ed.buf.slice(line..shown_end);
            let is_cur = line == primary_line;
            let row_bg = if is_cur { CURLINE } else { BG };
            if is_cur { self.p.rect(ex * CELL_W, y * CELL_H, text_cols * CELL_W, CELL_H, CURLINE); }
            if numbers {
                let label = match first_line { Some(n) => format!("{:>5} ", n + r + 1), None => String::from("    ? ") };
                self.p.text(tc, y, gutter, &label, if is_cur { FG } else { DIM }, BG);
            }
            in_comment = highlight::line(lang, &bytes, in_comment, &mut spans);
            // Find matches on this line, for highlighting.
            let hits: Vec<usize> = if find.len() >= 2 { project::matches(&bytes, &Query { needle: find.clone(), fold_case: true, whole_word: false }) } else { Vec::new() };
            let mut dc = 0usize;
            let mut k = 0usize;
            let mut si = 0;
            while k < bytes.len() {
                let off = line + k;
                let clen = match bytes[k] { 0..=0x7F => 1, 0xC0..=0xDF => 2, 0xE0..=0xEF => 3, _ => 4 }.min(bytes.len() - k);
                let c = core::str::from_utf8(&bytes[k..k + clen]).ok().and_then(|s| s.chars().next()).unwrap_or('\u{FFFD}');
                while si < spans.len() && spans[si].1 <= k { si += 1; }
                let fg = if si < spans.len() && spans[si].0 <= k { color(spans[si].2) } else { FG };
                let selected = in_sels(&sels, off);
                let hit = hits.iter().any(|h| k >= *h && k < h + find.len());
                let bg = if selected { SELECTED } else if hit { HIT } else { row_bg };
                if heads.binary_search(&off).is_ok() { cursors.push((dc, y)); }
                let width = if c == '\t' { 4 - dc % 4 } else { 1 };
                for wx in 0..width {
                    let col = dc + wx;
                    if col >= left && col < left + text_cols { cells.push((ex + col - left, y, if c == '\t' { ' ' } else { c }, fg, bg)); }
                }
                dc += width;
                k += clen;
            }
            if heads.binary_search(&(line + bytes.len())).is_ok() && shown_end == end { cursors.push((dc, y)); }
            if end >= t.ed.buf.len() { break; }
            line = end + 1;
        }
        for (x, y, c, fg, bg) in cells { self.p.ch(x, y, c, fg, bg); }
        for (dc, y) in cursors {
            if dc < left || dc >= left + text_cols { continue; }
            let x = (ex + dc - left) * CELL_W;
            if vim_block { self.p.rect(x, y * CELL_H + CELL_H - 3, CELL_W, 3, CURSOR); } else { self.p.rect(x, y * CELL_H + 1, 2, CELL_H - 2, CURSOR); }
        }
        // Status line.
        let sy = rows + 1;
        self.p.rect(0, sy * CELL_H, w, CELL_H, BAR);
        let t = &self.tabs.list[self.tabs.active];
        let c = t.ed.cursor();
        let ln = t.ed.buf.line_of(c).map_or(String::from("?"), |n| format!("{}", n + 1));
        let col = t.ed.buf.column(c) + 1;
        let mode = match self.keymap { Keymap::Vim => self.vim.label(), Keymap::Emacs => "EMACS", Keymap::VsCode => "" };
        let mut left_s = String::new();
        if !mode.is_empty() { left_s.push_str(&format!(" {mode} ")); }
        if self.keymap == Keymap::Vim && matches!(self.vim.mode, vim::Mode::Command | vim::Mode::Search) {
            left_s.push_str(&format!("{}{}", if self.vim.mode == vim::Mode::Command { ":" } else { "/" }, self.vim.cmdline));
        } else { left_s.push_str(&format!(" {}", self.status)); }
        let right = format!("Ln {ln}, Col {col}  {}{}  {} bytes  {} ", if t.ed.sels.len() > 1 { format!("{} cursors  ", t.ed.sels.len()) } else { String::new() },
            t.lang.name(), t.ed.buf.len(), if t.ed.buf.indexed() { "" } else { "indexing" });
        let n = self.p.text(0, sy, cols, &left_s, if mode.is_empty() { FG } else { ACCENT }, BAR);
        let rx = cols.saturating_sub(right.chars().count());
        if rx > n { self.p.text(rx, sy, cols - rx, &right, DIM, BAR); }
        self.draw_overlay(cols, rows);
        window.show_pixels(w, h);
    }

    fn draw_overlay(&mut self, cols: usize, rows: usize) {
        let ow = (cols * 3 / 5).clamp(30.min(cols), 100.min(cols));
        let ox = (cols - ow) / 2;
        let mut lines: Vec<(String, bool)> = Vec::new();
        let (title, input): (&str, String) = match &self.overlay {
            Overlay::None => return,
            Overlay::Palette { query, sel } => {
                let names: Vec<&str> = Command::ALL.iter().map(|c| c.name()).collect();
                for (k, (i, _)) in fuzzy::rank(query, &names, 12).iter().enumerate() {
                    let c = Command::ALL[*i];
                    let key = Keys::binding(self.keymap, c).map(keymap::describe).unwrap_or_default();
                    lines.push((format!("{:<width$}{}", c.name(), key, width = ow.saturating_sub(key.len() + 2)), k == *sel));
                }
                ("> ", query.clone())
            }
            Overlay::QuickOpen { query, sel, files } => {
                for (k, (i, _)) in fuzzy::rank(query, files, 12).iter().enumerate() { lines.push((lossy(&files[*i]), k == *sel)); }
                if files.is_empty() { lines.push((String::from("No files in this folder. Ctrl+Shift+O opens another."), false)); }
                ("Go to file: ", query.clone())
            }
            Overlay::Find { query } => ("Find: ", query.clone()),
            Overlay::Replace { find, with, field } => {
                lines.push((format!("{} With: {}", if *field == 1 { ">" } else { " " }, with), false));
                lines.push((String::from("Enter replaces and finds next; Ctrl+A replaces all; Tab switches field"), false));
                (if *field == 0 { "> Find: " } else { "  Find: " }, find.clone())
            }
            Overlay::GotoLine { query } => ("Go to line: ", query.clone()),
            Overlay::Search { query, with, field, hits, sel, note } => {
                lines.push((format!("{} Replace with: {}", if *field == 1 { ">" } else { " " }, with), false));
                lines.push((if note.is_empty() { String::from("Enter searches; Tab: replace field; Ctrl+A replaces in every file") } else { note.clone() }, false));
                let first = sel.saturating_sub(9);
                for (k, h) in hits.iter().enumerate().skip(first).take(10) {
                    lines.push((format!("{}:{}: {}", lossy(&h.path), h.line + 1, lossy(&h.preview).trim()), k == *sel));
                }
                (if *field == 0 { "> Search folder: " } else { "  Search folder: " }, query.clone())
            }
        };
        let oh = (lines.len() + 1).min(rows);
        self.p.rect(ox * CELL_W - 2, CELL_H - 2, ow * CELL_W + 4, oh * CELL_H + 4, ACCENT);
        self.p.rect(ox * CELL_W, CELL_H, ow * CELL_W, oh * CELL_H, PANEL);
        let s = format!("{title}{input}_");
        self.p.text(ox, 1, ow, &s, FG, PANEL);
        for (r, (l, sel)) in lines.iter().enumerate().take(oh.saturating_sub(1)) {
            let bg = if *sel { SELECTED } else { PANEL };
            self.p.rect(ox * CELL_W, (r + 2) * CELL_H, ow * CELL_W, CELL_H, bg);
            self.p.text(ox + 1, r + 2, ow - 1, l, if *sel { FG } else { DIM }, bg);
        }
    }

    /// The palette's "Developer: Large File Check": writes a file of `mb` MiB, opens it
    /// lazily, jumps to the end, types there, saves, and reports the saved hash.
    fn large_file_check(&mut self, window: &mut Window, mb: usize) {
        let path = b"large-check.txt".to_vec();
        let len = mb << 20;
        let _ = files::direct::remove(&path);
        if files::direct::create(&path).is_err() { self.status = String::from("Cannot create large-check.txt"); return; }
        let mut chunk = vec![0u8; sex_pdx::fs::IO_MAX];
        let mut at = 0;
        let t0 = sex_pdx::get_ticks();
        while at < len {
            let n = chunk.len().min(len - at);
            for (k, b) in chunk[..n].iter_mut().enumerate() { let i = at + k; *b = if i % 80 == 79 { b'\n' } else { b'a' + (i % 26) as u8 }; }
            match files::direct::write_at(&path, at as u64, &chunk[..n]) { Ok(w) if w == n => at += n, _ => { self.status = String::from("Disk full."); return; } }
        }
        let wrote_ms = sex_pdx::get_ticks().saturating_sub(t0);
        let t1 = sex_pdx::get_ticks();
        self.open(&path);
        let open_ms = sex_pdx::get_ticks().saturating_sub(t1);
        let t = self.tab();
        t.ed.move_all(Motion::DocEnd, false);
        t.ed.type_text(b"quil was here\n");
        t.ed.move_all(Motion::DocStart, false);
        t.ed.type_text(b"// top\n");
        sex_pdx::serial_record!("[quil.code.large] mb={} write_ms={} open_ms={} pieces={}", mb, wrote_ms, open_ms, t.ed.buf.pieces());
        self.save(false, window);
    }
}

fn in_sels(sels: &[(usize, usize)], off: usize) -> bool {
    let i = sels.partition_point(|s| s.0 <= off);
    i > 0 && off < sels[i - 1].1
}
fn display_cols(line: &[u8]) -> usize {
    let mut dc = 0;
    for &b in line { if b & 0xC0 == 0x80 { continue; } dc += if b == b'\t' { 4 - dc % 4 } else { 1 }; }
    dc
}

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Quil");
    let font = sex_font::Font::new(sex_font::GEIST_MONO).unwrap_or_else(|| sex_pdx::task_finish(4));
    let (w, h) = window.size();
    let p = Painter { px: window.pixels(), font, acc: vec![0; sex_font::raster::scratch_len(CELL_W, CELL_H)], glyphs: BTreeMap::new(), w, h };
    let mut app = App {
        tabs: Tabs::default(), tree: Tree::new(b""), show_tree: false, focus: Focus::Editor, root: Vec::new(), keymap: Keymap::VsCode,
        keys: Keys::default(), vim: Vim::new(), overlay: Overlay::None, clipboard: (Vec::new(), false), status: String::from("Ctrl+Shift+P: all commands"),
        numbers: true, find: Vec::new(), mark: false, quit_armed: false, drag: false, p,
    };
    // `quil @/path` opens a file or a folder; words like `vim` or `emacs` pick the keys.
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    for word in given.split(|b| *b == b' ').filter(|w| !w.is_empty()) {
        match word {
            b"vim" => app.keymap = Keymap::Vim,
            b"emacs" => app.keymap = Keymap::Emacs,
            w => {
                let path = w.strip_prefix(b"@").map(|p| p.to_vec()).unwrap_or_else(|| w.to_vec());
                match files::direct::stat(&path) {
                    Ok(st) if st.dir => app.set_root(&path),
                    _ => app.open(&path),
                }
            }
        }
    }
    if !app.show_tree && files::direct::list(b"").is_ok_and(|v| !v.is_empty()) { app.set_root(b""); app.show_tree = false; }
    app.tab();
    app.draw(&mut window);
    sex_pdx::serial_record!("[quil.code.ready] target={} task={} keymap={}", window.target, window.task, match app.keymap { Keymap::Vim => "vim", Keymap::Emacs => "emacs", Keymap::VsCode => "vscode" });
    loop {
        match window.next_within(100) {
            Some(Event::Close) => { sex_pdx::serial_record!("[quil.code.closed] target={} task={}", window.target, window.task); window.close() }
            Some(Event::Resize { .. }) => {}
            Some(Event::Pointer(ptr)) => app.pointer(ptr),
            Some(Event::Key(key)) => {
                // The palette's developer check is the one command that needs a size.
                if let Overlay::Palette { query, .. } = &app.overlay {
                    if key.code == 0x1C && query.starts_with("large") {
                        let mb = query.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(200);
                        app.overlay = Overlay::None;
                        app.large_file_check(&mut window, mb);
                        app.draw(&mut window);
                        continue;
                    }
                }
                app.key(key, &mut window);
            }
            None => {
                // Idle: count more of a big file's lines, a few blocks at a time.
                let t = app.tab();
                if t.ed.buf.indexed() { continue; }
                t.ed.buf.index_step(8);
            }
        }
        app.draw(&mut window);
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }
