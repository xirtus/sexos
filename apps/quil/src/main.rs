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
//! Ctrl+Shift+V shows tracked changes against a granted project's Git index.
//! The palette switches to Vim mode or Emacs keys.
//! Ctrl+Shift+J checks the current Rust, Python or JavaScript file with Quil's
//! native LSP diagnostic helper through a run-scoped pipe.
//! Ctrl+Shift+Space requests bounded keyword completions from the same helper.
//! Ctrl+Shift+M opens a document outline returned by the helper.
//! Ctrl+Shift+G follows an unambiguous declaration in this or another open tab.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use quil_core::buffer::{Buffer, MemSource, Source};
use quil_core::draft;
use quil_core::git;
use quil_core::editor::{Editor, Motion};
use quil_core::highlight::{self, Kind};
use quil_core::keymap::{self, Command, Keymap, Keys, Lookup, Named, Press, K};
use quil_core::project::{self, Entry, Fs, Query, Tree};
use quil_core::vim::{self, VKey, Vim};
use quil_core::workspace::{SyntaxAt, Tab, Tabs, SYNTAX_LINE_MAX};
use quil_core::fuzzy;
use sex_app::{files, settings, Event, Window};
use sex_pdx::app_key::{us_char, Key};
use sex_pdx::app_pointer::Kind as PKind;
use sex_theme::{Mode, Tokens};
mod lsp;

/// Files up to this size are read into memory when opened (so undo survives saving);
/// bigger ones stay on disk and are read a block at a time.
const IN_MEMORY: u64 = 32 << 20;
const CELL_W: usize = 9;
const CELL_H: usize = 18;
const TREE_COLS: usize = 28;

#[derive(Clone, Copy)]
struct Palette {
    bg: u32, fg: u32, dim: u32, panel: u32, bar: u32,
    curline: u32, selected: u32, accent: u32, cursor: u32, hit: u32,
    keyword: u32, ty: u32, string: u32, number: u32,
}

impl Palette {
    fn from_tokens(t: Tokens) -> Self {
        let rgb = |c: u32| c & 0x00FF_FFFF;
        let bg = rgb(t.window);
        let accent = rgb(t.accent);
        let (keyword, ty, string, number) = match t.mode {
            Mode::Light => (0x7135A0, 0x795500, 0x236B31, 0xA04014),
            Mode::Dark => (0xCBA6F7, 0xF9E2AF, 0xA6E3A1, 0xFAB387),
            Mode::HighContrast => (0xFFE600, 0x00FFFF, 0x7CFF7C, 0xFFB86C),
        };
        Self {
            bg, fg: rgb(t.text), dim: rgb(t.text_dim), panel: rgb(t.surface),
            bar: rgb(t.field), curline: rgb(t.control_hover),
            selected: blend(bg, accent, (t.selection >> 24) & 0xFF),
            accent, cursor: rgb(t.focus),
            hit: blend(bg, keyword, if t.mode == Mode::HighContrast { 128 } else { 55 }),
            keyword, ty, string, number,
        }
    }
    fn color(self, k: Kind) -> u32 {
        match k {
            Kind::Plain => self.fg, Kind::Keyword => self.keyword,
            Kind::Type => self.ty, Kind::String => self.string,
            Kind::Number => self.number, Kind::Comment => self.dim,
            Kind::Punct => self.dim, Kind::Function => self.accent,
        }
    }
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

fn draft_name(path: &[u8]) -> Vec<u8> { format!("quil-draft-{:016x}", draft::hash(path)).into_bytes() }

/// Read within Quil's existing project grant; Git status never launches a shell.
fn git_read(path: &[u8], max: usize) -> Option<Vec<u8>> {
    let len = usize::try_from(files::direct::stat(path).ok()?.size).ok()?;
    if len > max { return None; }
    let mut bytes = vec![0; len];
    let mut at = 0;
    while at < len {
        let end = (at + sex_pdx::fs::IO_MAX).min(len);
        let n = files::direct::read_at(path, at as u64, &mut bytes[at..end]).ok()?;
        if n == 0 { return None; }
        at += n;
    }
    Some(bytes)
}

fn git_head(root: &[u8]) -> String {
    git_read(&project::join(root, b".git/HEAD"), 256)
        .and_then(|bytes| git::head_label(&bytes))
        .unwrap_or_else(|| String::from("unavailable"))
}

#[derive(Clone)]
struct GitChange { code: char, path: Vec<u8>, id: [u8; 20] }

/// Compare tracked paths with the checked Git index and list bounded files
/// absent from it. Ignore rules are not parsed, so those rows are candidates.
fn git_changes(root: &[u8]) -> Result<Vec<GitChange>, String> {
    let index = project::join(root, b".git/index");
    let bytes = git_read(&index, git::MAX_INDEX).ok_or_else(|| String::from("No readable Git index in this project."))?;
    let tracked = git::parse_index(&bytes).map_err(|e| format!("Unsupported or damaged Git index: {e:?}"))?;
    let walked = project::walk(&mut AppFs, root, 5000);
    let candidates = git::untracked_candidates(&tracked, &walked, 64);
    let mut out = Vec::new();
    for entry in tracked {
        if entry.stage != 0 { out.push(GitChange { code: 'C', path: entry.path, id: entry.id }); continue; }
        let path = project::join(root, &entry.path);
        let code = match files::direct::stat(&path) {
            Err(_) => 'D',
            Ok(st) if st.dir => 'D',
            Ok(st) if st.size > git::MAX_FILE => '?',
            Ok(_) => match git_read(&path, git::MAX_FILE as usize) {
                Some(bytes) if git::blob_id(&bytes) == entry.id => continue,
                Some(_) => 'M',
                None => '?',
            },
        };
        out.push(GitChange { code, path: entry.path, id: entry.id });
    }
    for path in candidates { out.push(GitChange { code: 'N', path, id: [0; 20] }); }
    Ok(out)
}

fn git_tracked_count(changes: &[GitChange]) -> usize { changes.iter().filter(|c| c.code != 'N').count() }
fn git_candidate_count(changes: &[GitChange]) -> usize { changes.iter().filter(|c| c.code == 'N').count() }

fn git_diff(root: &[u8], change: &GitChange) -> Result<Vec<String>, String> {
    if !matches!(change.code, 'M' | 'D') { return Err(String::from("Diff is available for tracked modified or deleted text files.")); }
    let hex = b"0123456789abcdef";
    let mut object = b".git/objects/".to_vec();
    for (i, byte) in change.id.iter().enumerate() {
        if i == 1 { object.push(b'/'); }
        object.push(hex[(byte >> 4) as usize]); object.push(hex[(byte & 15) as usize]);
    }
    let packed = String::from("Indexed blob is packed or unavailable; this diff view currently reads loose objects.");
    let compressed = git_read(&project::join(root, &object), git::MAX_FILE as usize + 64).ok_or(packed)?;
    let before = git::loose_blob(&compressed, &change.id).map_err(|_| String::from("Indexed blob is damaged or too large."))?;
    let after = if change.code == 'D' { Vec::new() } else {
        git_read(&project::join(root, &change.path), git::MAX_FILE as usize)
            .ok_or_else(|| String::from("Changed file is unavailable or too large."))?
    };
    git::diff_text(&before, &after).map_err(|e| match e {
        git::DiffError::Limit => String::from("Diff exceeds 64 KiB or 300 lines per side."),
        git::DiffError::Binary => String::from("Binary or non-UTF-8 files need a separate viewer."),
        git::DiffError::Blob => String::from("Indexed blob is damaged."),
    })
}

fn git_write_new(path: &[u8], bytes: &[u8]) -> Result<(), String> {
    files::direct::create(path).map_err(|_| String::from("Cannot create Git staging file."))?;
    let mut at = 0;
    while at < bytes.len() {
        let end = (at + sex_pdx::fs::IO_MAX).min(bytes.len());
        match files::direct::write_at(path, at as u64, &bytes[at..end]) {
            Ok(n) if n == end - at => at = end,
            _ => { let _ = files::direct::remove(path); return Err(String::from("Git staging write was incomplete.")); }
        }
    }
    if files::direct::sync().is_err() { let _ = files::direct::remove(path); return Err(String::from("Git staging sync failed.")); }
    Ok(())
}

fn git_replace_index(root: &[u8], expected: &[u8], updated: &[u8]) -> Result<(), String> {
    let index = project::join(root, b".git/index");
    let lock = project::join(root, b".git/index.lock");
    if files::direct::stat(&lock).is_ok() { return Err(String::from("Git index is locked by another operation.")); }
    git_write_new(&lock, updated)?;
    if git_read(&index, git::MAX_INDEX).as_deref() != Some(expected) {
        let _ = files::direct::remove(&lock);
        return Err(String::from("Git index changed; reopen tracked changes."));
    }
    if files::direct::rename(&lock, &index).is_err() {
        let _ = files::direct::remove(&lock);
        return Err(String::from("Could not replace Git index."));
    }
    files::direct::sync().map_err(|_| String::from("Git index sync failed."))
}

fn git_stage(root: &[u8], change: &GitChange) -> Result<(Vec<u8>, Vec<u8>), String> {
    if !matches!(change.code, 'M' | 'D') { return Err(String::from("Stage supports modified or deleted tracked files only.")); }
    let index = project::join(root, b".git/index");
    let before = git_read(&index, git::MAX_INDEX).ok_or_else(|| String::from("Git index is unavailable."))?;
    if git::parse_index(&before).ok().and_then(|entries| entries.into_iter().find(|e| e.path == change.path)).map(|e| e.id) != Some(change.id) {
        return Err(String::from("Git panel is stale; reopen tracked changes."));
    }
    if change.code == 'D' {
        if files::direct::stat(&project::join(root, &change.path)).is_ok() {
            return Err(String::from("Deleted file reappeared; reopen tracked changes."));
        }
        let after = git::stage_deleted(&before, &change.path)
            .map_err(|e| format!("Cannot stage this Git index deletion: {e:?}"))?;
        git_replace_index(root, &before, &after)?;
        return Ok((before, after));
    }
    let current = git_read(&project::join(root, &change.path), git::MAX_DIFF_BYTES)
        .ok_or_else(|| String::from("Changed text file exceeds 64 KiB or is unavailable."))?;
    let (after, id, packed) = git::stage_tracked(&before, &change.path, &current)
        .map_err(|e| format!("Cannot stage this Git index entry: {e:?}"))?;
    let hex = b"0123456789abcdef";
    let mut dir = b".git/objects/".to_vec();
    for byte in id[..1].iter() { dir.push(hex[(byte >> 4) as usize]); dir.push(hex[(byte & 15) as usize]); }
    let dir = project::join(root, &dir);
    if files::direct::stat(&dir).is_err() { files::direct::mkdir(&dir).map_err(|_| String::from("Cannot make Git object folder."))?; }
    let mut object = dir.clone(); object.push(b'/');
    for byte in &id[1..] { object.push(hex[(byte >> 4) as usize]); object.push(hex[(byte & 15) as usize]); }
    if files::direct::stat(&object).is_ok() {
        let existing = git_read(&object, git::MAX_DIFF_BYTES + 64).ok_or_else(|| String::from("Existing Git object cannot be read."))?;
        git::loose_blob(&existing, &id).map_err(|_| String::from("Existing Git object is damaged."))?;
    } else {
        let mut temp = object.clone(); temp.extend_from_slice(b".quil-stage");
        if files::direct::stat(&temp).is_ok() { return Err(String::from("Git object staging file already exists.")); }
        git_write_new(&temp, &packed)?;
        if files::direct::rename(&temp, &object).is_err() { let _ = files::direct::remove(&temp); return Err(String::from("Cannot commit Git object.")); }
        files::direct::sync().map_err(|_| String::from("Git object sync failed."))?;
    }
    git_replace_index(root, &before, &after)?;
    Ok((before, after))
}

fn read_draft(path: &[u8]) -> Option<Vec<u8>> {
    if path.is_empty() { return None; }
    let name = draft_name(path);
    let len = usize::try_from(files::direct::stat(&name).ok()?.size).ok()?;
    if len > draft::HEADER + draft::MAX_PATH + draft::MAX_TEXT { return None; }
    let mut packet = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let end = (at + sex_pdx::fs::IO_MAX).min(len);
        let n = files::direct::read_at(&name, at as u64, &mut packet[at..end]).ok()?;
        if n == 0 { return None; }
        at += n;
    }
    draft::decode(path, &packet).map(|text| text.to_vec())
}

fn write_draft(path: &[u8], text: &[u8]) -> bool {
    let Some(packet) = draft::encode(path, text) else { return false };
    let name = draft_name(path);
    let mut stage = name.clone();
    stage.extend_from_slice(b".part");
    let _ = files::direct::remove(&stage);
    if write_buffer(&Buffer::from_bytes(&packet), &stage).is_err() { return false; }
    if files::direct::sync().is_err() { return false; }
    if files::direct::rename(&stage, &name).is_err() { return false; }
    files::direct::sync().is_ok()
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
    Completion { items: Vec<String>, sel: usize, start: usize, cursor: usize, version: u64, path: Vec<u8> },
    Diagnostics { items: Vec<quil_core::lsp::Diagnostic>, sel: usize, version: u64, path: Vec<u8> },
    Symbols { items: Vec<quil_core::lsp::Symbol>, sel: usize, version: u64, path: Vec<u8> },
    Help { scroll: usize },
    Git { changes: Vec<GitChange>, sel: usize, note: String, head: String },
    GitDiff { path: Vec<u8>, lines: Vec<String>, scroll: usize, note: String },
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
    palette: Palette,
    theme_generation: u64,
    theme_next: u64,
    draft_next: u64,
    drafted: BTreeMap<Vec<u8>, u64>,
    /// Last index edit only; undo refuses if another Git actor touched it.
    git_stage_undo: Option<(Vec<u8>, Vec<u8>)>,
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
    fn free_untitled_slot(&self) -> Option<u8> {
        (0..draft::UNTITLED_SLOTS).find(|slot| {
            if self.tabs.list.iter().any(|tab| tab.untitled_slot == Some(*slot)) { return false; }
            let key = draft::untitled_key(*slot).unwrap();
            matches!(files::direct::stat(&draft_name(&key)), Err(files::NOT_FOUND))
        })
    }

    fn new_untitled_tab(&mut self) {
        let mut tab = Tab::new(b"", Editor::from_bytes(b""));
        tab.untitled_slot = self.free_untitled_slot();
        if tab.untitled_slot.is_none() {
            self.status = String::from("Untitled recovery unavailable; save this tab to a file.");
        }
        self.tabs.add(tab);
    }

    fn restore_untitled_tabs(&mut self) {
        for slot in 0..draft::UNTITLED_SLOTS {
            let key = draft::untitled_key(slot).unwrap();
            let Some(text) = read_draft(&key).filter(|text| !text.is_empty()) else { continue };
            let mut ed = Editor::from_bytes(b"");
            ed.type_text(&text);
            let version = ed.version();
            let mut tab = Tab::new(b"", ed);
            tab.untitled_slot = Some(slot);
            self.tabs.add(tab);
            self.drafted.insert(key, version);
            sex_pdx::serial_record!("[quil.code.draft.recovered] untitled={} bytes={}", slot, text.len());
            self.status = format!("Recovered untitled tab {}", slot + 1);
        }
    }
    fn complete_language(&mut self) {
        if self.focus != Focus::Editor { return; }
        let len = self.tab().ed.buf.len();
        if len > 256 * 1024 { self.status = String::from("Language completion supports files up to 256 KiB"); return; }
        let (path, source, cursor, version, one_cursor) = {
            let tab = self.tab();
            (tab.path.clone(), tab.ed.buf.slice(0..len), tab.ed.cursor(), tab.ed.version(), tab.ed.sels.len() == 1 && tab.ed.primary_sel().is_empty())
        };
        if !one_cursor { self.status = String::from("Completion needs one cursor without a selection"); return; }
        let language = if path.ends_with(b".rs") { "rust" }
            else if path.ends_with(b".py") { "python" }
            else if path.ends_with(b".js") { "javascript" }
            else { self.status = String::from("Completion supports .rs, .py and .js"); return; };
        let Ok(source) = core::str::from_utf8(&source) else { self.status = String::from("Completion needs UTF-8 text"); return; };
        let (start, _) = quil_core::lsp::complete(source, language, cursor);
        if start == cursor { self.status = String::from("Type a word prefix first"); return; }
        match lsp::complete(&path, language, source, cursor) {
            Ok(items) => {
                sex_pdx::serial_record!("[quil.lsp.completion] language={} count={} prefix={}", language, items.len(), &source[start..cursor]);
                if items.is_empty() { self.status = String::from("No completions for this prefix"); }
                else { self.overlay = Overlay::Completion { items, sel: 0, start, cursor, version, path }; }
            }
            Err(why) => { self.status = String::from(why); sex_pdx::serial_record!("[quil.lsp.unavailable] reason={why}"); }
        }
    }

    fn show_symbols(&mut self) {
        if self.focus != Focus::Editor { return; }
        let len = self.tab().ed.buf.len();
        if len > 256 * 1024 { self.status = String::from("Language symbols support files up to 256 KiB"); return; }
        let (path, source, version) = {
            let tab = self.tab();
            (tab.path.clone(), tab.ed.buf.slice(0..len), tab.ed.version())
        };
        let language = if path.ends_with(b".rs") { "rust" }
            else if path.ends_with(b".py") { "python" }
            else if path.ends_with(b".js") { "javascript" }
            else { self.status = String::from("Symbols support .rs, .py and .js"); return; };
        let Ok(source) = core::str::from_utf8(&source) else { self.status = String::from("Symbols need UTF-8 text"); return; };
        match lsp::symbols(&path, language, source) {
            Ok(items) => {
                sex_pdx::serial_record!("[quil.lsp.symbols] language={} count={}", language, items.len());
                if items.is_empty() { self.status = String::from("No declarations found"); }
                else { self.overlay = Overlay::Symbols { items, sel: 0, version, path }; }
            }
            Err(why) => { self.status = String::from(why); sex_pdx::serial_record!("[quil.lsp.unavailable] reason={why}"); }
        }
    }

    fn go_to_definition(&mut self) {
        if self.focus != Focus::Editor { return; }
        let len = self.tab().ed.buf.len();
        if len > 256 * 1024 { self.status = String::from("Language definition supports files up to 256 KiB"); return; }
        let (path, source, cursor, version, one_cursor) = {
            let tab = self.tab();
            (tab.path.clone(), tab.ed.buf.slice(0..len), tab.ed.cursor(), tab.ed.version(), tab.ed.sels.len() == 1 && tab.ed.primary_sel().is_empty())
        };
        if !one_cursor { self.status = String::from("Definition needs one cursor without a selection"); return; }
        let language = if path.ends_with(b".rs") { "rust" }
            else if path.ends_with(b".py") { "python" }
            else if path.ends_with(b".js") { "javascript" }
            else { self.status = String::from("Definitions support .rs, .py and .js"); return; };
        let Ok(source) = core::str::from_utf8(&source) else { self.status = String::from("Definitions need UTF-8 text"); return; };
        let mut peers = Vec::new();
        let mut total = source.len();
        for tab in &self.tabs.list {
            if tab.path == path || tab.path.is_empty() || !tab.path.ends_with(match language { "rust" => b".rs", "python" => b".py", _ => b".js" }) { continue; }
            let n = tab.ed.buf.len();
            if n > 256 * 1024 || total + n > 512 * 1024 || peers.len() == 8 { continue; }
            let bytes = tab.ed.buf.slice(0..n);
            let Ok(text) = core::str::from_utf8(&bytes) else { continue; };
            peers.push(lsp::Peer { path: tab.path.clone(), text: String::from(text), version: tab.ed.version() });
            total += n;
        }
        match lsp::definition(&path, language, source, cursor, &peers) {
            Ok(Some((target, line, character))) => {
                let current = self.tab();
                if current.path != path || current.ed.version() != version || current.ed.cursor() != cursor {
                    self.status = String::from("Definition expired; request it again");
                    return;
                }
                let (target_path, target_text, target_version) = if target == 0 { (&path, source, version) }
                    else if let Some(peer) = peers.get(target - 1) { (&peer.path, peer.text.as_str(), peer.version) }
                    else { self.status = String::from("Language service returned an unknown tab"); return; };
                let Some(at) = quil_core::lsp::offset_at_position(target_text, line as u64, character as u64) else {
                    self.status = String::from("Language service sent an invalid definition"); return;
                };
                let Some(i) = self.tabs.find(target_path) else { self.status = String::from("Definition tab closed; request it again"); return; };
                if self.tabs.list[i].ed.version() != target_version { self.status = String::from("Definition changed; request it again"); return; }
                self.tabs.active = i;
                self.tabs.list[i].ed.set_cursor(at);
                self.status = format!("Definition in {} at line {}", lossy(target_path), line + 1);
                sex_pdx::serial_record!("[quil.lsp.definition] language={} line={} at={} tab={}", language, line + 1, at, i + 1);
            }
            Ok(None) => self.status = String::from("No unique declaration in the open tabs"),
            Err(why) => { self.status = String::from(why); sex_pdx::serial_record!("[quil.lsp.unavailable] reason={why}"); }
        }
    }

    fn show_hover(&mut self) {
        if self.focus != Focus::Editor { return; }
        let len = self.tab().ed.buf.len();
        if len > 256 * 1024 { self.status = String::from("Language hover supports files up to 256 KiB"); return; }
        let (path, source, cursor, version, one_cursor) = {
            let tab = self.tab();
            (tab.path.clone(), tab.ed.buf.slice(0..len), tab.ed.cursor(), tab.ed.version(), tab.ed.sels.len() == 1 && tab.ed.primary_sel().is_empty())
        };
        if !one_cursor { self.status = String::from("Hover needs one cursor without a selection"); return; }
        let language = if path.ends_with(b".rs") { "rust" }
            else if path.ends_with(b".py") { "python" }
            else if path.ends_with(b".js") { "javascript" }
            else { self.status = String::from("Hover supports .rs, .py and .js"); return; };
        let Ok(source) = core::str::from_utf8(&source) else { self.status = String::from("Hover needs UTF-8 text"); return; };
        match lsp::hover(&path, language, source, cursor) {
            Ok(Some(summary)) => {
                let tab = self.tab();
                if tab.path != path || tab.ed.version() != version || tab.ed.cursor() != cursor {
                    self.status = String::from("Hover expired; request it again");
                } else {
                    self.status = format!("Hover: {summary}");
                    sex_pdx::serial_record!("[quil.lsp.hover] language={} text={}", language, summary);
                }
            }
            Ok(None) => self.status = String::from("No unique declaration in this file"),
            Err(why) => { self.status = String::from(why); sex_pdx::serial_record!("[quil.lsp.unavailable] reason={why}"); }
        }
    }

    fn check_language(&mut self) {
        let len = self.tab().ed.buf.len();
        if len > 256 * 1024 {
            self.status = String::from("Language check supports files up to 256 KiB");
            return;
        }
        let (path, source, version) = {
            let tab = self.tab();
            (tab.path.clone(), tab.ed.buf.slice(0..len), tab.ed.version())
        };
        let language = if path.ends_with(b".rs") { "rust" }
            else if path.ends_with(b".py") { "python" }
            else if path.ends_with(b".js") { "javascript" }
            else { self.status = String::from("Language check supports .rs, .py and .js"); return; };
        let Ok(source) = core::str::from_utf8(&source) else {
            self.status = String::from("Language check needs UTF-8 text");
            return;
        };
        match lsp::check(&path, language, source) {
            Ok(result) => {
                self.status = match (result.first_line, result.first.as_deref()) {
                    (Some(line), Some(message)) => format!("LSP: {} issue(s); line {}: {}", result.count, line + 1, message),
                    _ => format!("LSP: {} issue(s)", result.count),
                };
                sex_pdx::serial_record!("[quil.lsp.diagnostics] language={} count={} first_line={}", language, result.count, result.first_line.map_or(0, |n| n + 1));
                if !result.items.is_empty() {
                    let tab = self.tab();
                    if tab.path != path || tab.ed.version() != version {
                        self.status = String::from("Diagnostics expired; request them again");
                    } else {
                        self.overlay = Overlay::Diagnostics { items: result.items, sel: 0, version, path };
                    }
                }
            }
            Err(why) => { self.status = String::from(why); sex_pdx::serial_record!("[quil.lsp.unavailable] reason={why}"); }
        }
    }

    fn rows(&self) -> usize { (self.p.h / CELL_H).saturating_sub(2).max(1) }
    fn cols(&self) -> usize { self.p.w / CELL_W }
    fn tree_cols(&self) -> usize { if self.show_tree { TREE_COLS.min(self.cols() / 3) } else { 0 } }
    fn gutter(&self) -> usize { if self.numbers { 7 } else { 1 } }

    fn tab(&mut self) -> &mut Tab {
        if self.tabs.is_empty() { self.new_untitled_tab(); }
        let i = self.tabs.active;
        &mut self.tabs.list[i]
    }

    fn open(&mut self, path: &[u8]) {
        if let Some(i) = self.tabs.find(path) { self.tabs.active = i; return; }
        match open_buffer(path) {
            Ok(buf) => {
                let len = buf.len();
                let mut ed = Editor::new(buf);
                if let Some(draft) = read_draft(path) {
                    if ed.buf.len() != draft.len() || ed.buf.to_vec() != draft {
                        ed.select_all();
                        ed.type_text(&draft);
                        sex_pdx::serial_record!("[quil.code.draft.recovered] path={} bytes={}", lossy(path), draft.len());
                        self.status = format!("Recovered draft of {}", lossy(path));
                    }
                }
                ed.indent = b"    ".to_vec();
                // Replace an empty, untouched untitled tab rather than keep it.
                if self.tabs.len() == 1 && self.tabs.list[0].path.is_empty() && self.tabs.list[0].ed.buf.is_empty() && !self.tabs.list[0].ed.modified() { self.tabs.close(0); }
                self.tabs.add(Tab::new(path, ed));
                sex_pdx::serial_record!("[quil.code.opened] path={} bytes={} lazy={}", lossy(path), len, (len as u64 > IN_MEMORY) as u8);
                if !self.tab().ed.modified() { self.status = format!("Opened {} ({} bytes)", lossy(path), len); }
            }
            Err(e) => {
                // A new file: an empty tab that saves to that path.
                if e == files::NOT_FOUND {
                    let mut ed = Editor::from_bytes(b"");
                    if let Some(draft) = read_draft(path) {
                        ed.type_text(&draft);
                        sex_pdx::serial_record!("[quil.code.draft.recovered] path={} bytes={}", lossy(path), draft.len());
                    }
                    self.tabs.add(Tab::new(path, ed));
                    self.status = if self.tab().ed.modified() { format!("Recovered draft of {}", lossy(path)) } else { format!("New file {}", lossy(path)) };
                } else { self.status = lossy(files::message(e)); }
            }
        }
    }

    fn save(&mut self, as_new: bool, window: &mut Window) {
        let mut path = self.tab().path.clone();
        let old_untitled = self.tab().untitled_slot.and_then(draft::untitled_key);
        if path.is_empty() || as_new {
            let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
            let name = if path.is_empty() { b"untitled.txt".to_vec() } else { self.tab().name().to_vec() };
            match files::pick(window, files::Pick::Save, &name, &mut out) {
                Ok(p) => path = p.to_vec(),
                Err(_) => { self.status = String::from("Not saved."); return; }
            }
        }
        let lazy = self.tab().ed.buf.len() as u64 > IN_MEMORY;
        // Stage in Quil's own folder, where an exact file grant still permits a
        // write. Keep the old file readable while a lazy buffer supplies chunks.
        let target = format!("quil-save-{}", window.task).into_bytes();
        let _ = files::direct::remove(&target);
        let result = write_buffer(&self.tab().ed.buf, &target);
        match result {
            Ok((n, h)) => {
                if let Err(e) = files::direct::sync() {
                    self.status = format!("Save not committed: {}", lossy(files::message(e)));
                    return;
                }
                if let Err(e) = files::direct::rename(&target, &path) {
                    self.status = format!("Save not committed ({}); draft at {}", lossy(files::message(e)), lossy(&target));
                    return;
                }
                let committed = files::direct::sync();
                if lazy {
                    let cursor = self.tab().ed.cursor();
                    let t = self.tab();
                    t.ed.buf = Buffer::new(Box::new(FileSource { path: path.clone(), len: n }));
                    t.ed.forget_history();
                    t.ed.set_cursor(cursor);
                }
                if let Err(e) = committed {
                    self.status = format!("Save sync failed: {}", lossy(files::message(e)));
                    return;
                }
                let t = self.tab();
                t.path = path.clone();
                t.untitled_slot = None;
                t.lang = highlight::Lang::from_path(&path);
                t.ed.mark_saved();
                let _ = files::direct::remove(&draft_name(&path));
                if let Some(key) = &old_untitled { let _ = files::direct::remove(&draft_name(key)); self.drafted.remove(key); }
                let _ = files::direct::sync();
                self.drafted.remove(&path);
                sex_pdx::serial_record!("[quil.code.saved] path={} bytes={} hash={:016x}", lossy(&path), n, h);
                sex_pdx::serial_record!("[quil.code.save.committed] path={} bytes={}", lossy(&path), n);
                self.status = format!("Saved {} bytes", n);
            }
            Err(e) => { let _ = files::direct::remove(&target); sex_pdx::serial_record!("[quil.code.save.failed] status={}", e); self.status = lossy(files::message(e)); }
        }
    }

    fn autosave_drafts(&mut self) {
        for i in 0..self.tabs.list.len() {
            let tab = &self.tabs.list[i];
            if !tab.ed.modified() || tab.ed.buf.len() > draft::MAX_TEXT { continue; }
            let path = if tab.path.is_empty() {
                let Some(key) = tab.untitled_slot.and_then(draft::untitled_key) else { continue };
                key
            } else { tab.path.clone() };
            let version = tab.ed.version();
            if self.drafted.get(&path) == Some(&version) { continue; }
            let text = tab.ed.text();
            if write_draft(&path, &text) {
                self.drafted.insert(path.clone(), version);
                if let Some(slot) = tab.untitled_slot {
                    sex_pdx::serial_record!("[quil.code.draft.saved] untitled={} bytes={}", slot, text.len());
                } else {
                    sex_pdx::serial_record!("[quil.code.draft.saved] path={} bytes={}", lossy(&path), text.len());
                }
            } else {
                self.status = String::from("Recovery draft could not be saved; save this tab to a file.");
            }
        }
    }

    fn set_root(&mut self, root: &[u8]) {
        self.root = root.to_vec();
        self.git_stage_undo = None;
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
            Command::NewTab => self.new_untitled_tab(),
            Command::CloseTab => {
                if self.tab().ed.modified() && !self.quit_armed { self.quit_armed = true; self.status = String::from("Unsaved changes: Close Tab again to discard."); return; }
                self.quit_armed = false;
                let i = self.tabs.active;
                if let Some(tab) = self.tabs.close(i) {
                    let key = if tab.path.is_empty() { tab.untitled_slot.and_then(draft::untitled_key) } else { Some(tab.path) };
                    if let Some(key) = key { let _ = files::direct::remove(&draft_name(&key)); self.drafted.remove(&key); let _ = files::direct::sync(); }
                }
            }
            Command::NextTab => self.tabs.next(),
            Command::PrevTab => self.tabs.prev(),
            Command::Quit => {
                if self.tabs.list.iter().any(|t| t.ed.modified()) && !self.quit_armed { self.quit_armed = true; self.status = String::from("Unsaved changes: Quit again to discard them."); return; }
                for tab in &self.tabs.list {
                    let key = if tab.path.is_empty() { tab.untitled_slot.and_then(draft::untitled_key) } else { Some(tab.path.clone()) };
                    if let Some(key) = key { let _ = files::direct::remove(&draft_name(&key)); }
                }
                let _ = files::direct::sync();
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
            Command::AddNextOccurrence => {
                let ed = &mut self.tabs.list[i].ed;
                let added = ed.add_next_occurrence();
                sex_pdx::serial_record!("[quil.code.multicursor] action=next added={} count={}", added as u8, ed.sels.len());
            }
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
        if p.ctrl && p.shift && p.k == K::Char(' ') { self.complete_language(); return; }
        if p == Press::ctrl_shift('j') { self.check_language(); return; }
        if p == Press::ctrl_shift('m') { self.show_symbols(); return; }
        if p == Press::ctrl_shift('g') { self.go_to_definition(); return; }
        if p == Press::ctrl_shift('h') { self.show_hover(); return; }
        if p == Press::ctrl_shift('/') {
            self.overlay = if matches!(self.overlay, Overlay::Help { .. }) { Overlay::None } else { Overlay::Help { scroll: 0 } };
            sex_pdx::serial_record!("[quil.code.shortcuts] open={}", matches!(self.overlay, Overlay::Help { .. }) as u8);
            return;
        }
        if p == Press::ctrl_shift('v') {
            if matches!(self.overlay, Overlay::Git { .. }) { self.overlay = Overlay::None; }
            else {
                let (changes, note) = match git_changes(&self.root) {
                    Ok(changes) => (changes, String::new()),
                    Err(note) => (Vec::new(), note),
                };
                sex_pdx::serial_record!("[quil.code.git] tracked_changes={} note={}", git_tracked_count(&changes), note);
                sex_pdx::serial_record!("[quil.code.git.candidates] count={}", git_candidate_count(&changes));
                let head = git_head(&self.root);
                sex_pdx::serial_record!("[quil.code.git.head] label={head}");
                self.overlay = Overlay::Git { changes, sel: 0, note, head };
            }
            return;
        }
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
            Overlay::Diagnostics { items, sel, version, path } => {
                *sel = (*sel as isize + updown).clamp(0, items.len().saturating_sub(1) as isize) as usize;
                if enter {
                    keep = false;
                    let issue = items[*sel].clone();
                    let tab = self.tab();
                    if tab.path != *path || tab.ed.version() != *version {
                        self.status = String::from("Diagnostics expired; request them again");
                    } else {
                        let bytes = tab.ed.buf.slice(0..tab.ed.buf.len());
                        if let Ok(text) = core::str::from_utf8(&bytes) {
                            if let Some(at) = quil_core::lsp::offset_at_position(text, issue.line as u64, issue.character as u64) {
                                tab.ed.set_cursor(at);
                                self.status = format!("Line {}: {}", issue.line.saturating_add(1), issue.message);
                                sex_pdx::serial_record!("[quil.lsp.diagnostic.jump] line={} character={} at={}", issue.line.saturating_add(1), issue.character, at);
                            } else { self.status = String::from("Language service sent an invalid diagnostic location"); }
                        }
                    }
                }
            }
            Overlay::Completion { items, sel, start, cursor, version, path } => {
                *sel = (*sel as isize + updown).clamp(0, items.len().saturating_sub(1) as isize) as usize;
                if enter {
                    keep = false;
                    let label = items[*sel].clone();
                    let tab = self.tab();
                    if tab.path != *path || tab.ed.version() != *version || tab.ed.cursor() != *cursor || tab.ed.sels.len() != 1 {
                        self.status = String::from("Completion expired; request it again");
                    } else {
                        tab.ed.break_group();
                        tab.ed.edit(|_, _| (*start..*cursor, label.as_bytes().to_vec(), quil_core::editor::After::End));
                        let new_version = tab.ed.version();
                        self.status = format!("Completed {label}");
                        sex_pdx::serial_record!("[quil.lsp.inserted] label={} version={}", label, new_version);
                    }
                }
            }
            Overlay::Symbols { items, sel, version, path } => {
                *sel = (*sel as isize + updown).clamp(0, items.len().saturating_sub(1) as isize) as usize;
                if enter {
                    keep = false;
                    let symbol = items[*sel].clone();
                    let tab = self.tab();
                    if tab.path != *path || tab.ed.version() != *version {
                        self.status = String::from("Outline expired; request it again");
                    } else {
                        let bytes = tab.ed.buf.slice(0..tab.ed.buf.len());
                        if let Ok(text) = core::str::from_utf8(&bytes) {
                            if let Some(at) = quil_core::lsp::offset_at_position(text, symbol.line as u64, symbol.character as u64) {
                                tab.ed.set_cursor(at);
                                self.status = format!("{} at line {}", symbol.name, symbol.line + 1);
                                sex_pdx::serial_record!("[quil.lsp.symbol.jump] line={} name={}", symbol.line + 1, symbol.name);
                            }
                        }
                    }
                }
            }
            Overlay::Help { scroll } => {
                let visible = self.rows().saturating_sub(2).max(1);
                let max = keymap::shortcut_rows(self.keymap).len().saturating_sub(visible);
                *scroll = match p.k {
                    K::Named(Named::Up) => scroll.saturating_sub(1),
                    K::Named(Named::Down) => scroll.saturating_add(1).min(max),
                    K::Named(Named::PageUp) => scroll.saturating_sub(visible),
                    K::Named(Named::PageDown) => scroll.saturating_add(visible).min(max),
                    _ => *scroll,
                };
            }
            Overlay::Git { changes, sel, note, head: _ } => {
                if matches!(typed, Some('u' | 'U')) {
                    let result = self.git_stage_undo.as_ref().ok_or_else(|| String::from("No Quil stage to undo."))
                        .and_then(|(before, after)| git_replace_index(&self.root, after, before));
                    match result {
                        Ok(()) => {
                            self.git_stage_undo = None;
                            let refreshed = git_changes(&self.root).unwrap_or_default();
                            sex_pdx::serial_record!("[quil.code.git.unstaged] tracked_changes={}", git_tracked_count(&refreshed));
                            self.overlay = Overlay::Git { changes: refreshed, sel: 0, note: String::from("Undid last Quil stage."), head: git_head(&self.root) };
                            keep = false;
                        }
                        Err(reason) => *note = reason,
                    }
                } else if matches!(typed, Some('s' | 'S')) && !changes.is_empty() {
                    let change = changes[*sel].clone();
                    match git_stage(&self.root, &change) {
                        Ok(history) => {
                            self.git_stage_undo = Some(history);
                            let refreshed = git_changes(&self.root).unwrap_or_default();
                            sex_pdx::serial_record!("[quil.code.git.staged] path={} tracked_changes={}", lossy(&change.path), git_tracked_count(&refreshed));
                            self.overlay = Overlay::Git { changes: refreshed, sel: 0, note: format!("Staged {}. U undoes this stage.", lossy(&change.path)), head: git_head(&self.root) };
                            keep = false;
                        }
                        Err(reason) => *note = reason,
                    }
                }
                if !keep { /* New panel already installed. */ }
                else {
                if !changes.is_empty() {
                    *sel = (*sel as isize + updown).clamp(0, changes.len() as isize - 1) as usize;
                    if p.k == K::Named(Named::Right) {
                        let change = &changes[*sel];
                        let (lines, note) = match git_diff(&self.root, change) {
                            Ok(lines) => (lines, String::new()), Err(note) => (Vec::new(), note),
                        };
                        sex_pdx::serial_record!("[quil.code.git.diff] path={} lines={} note={}", lossy(&change.path), lines.len(), note);
                        self.overlay = Overlay::GitDiff { path: change.path.clone(), lines, scroll: 0, note };
                    }
                    if enter && changes[*sel].code != 'D' {
                        let path = project::join(&self.root, &changes[*sel].path);
                        self.open(&path);
                        keep = false;
                    }
                }
                }
            }
            Overlay::GitDiff { lines, scroll, .. } => {
                let visible = self.rows().saturating_sub(3).max(1);
                let max = lines.len().saturating_sub(visible);
                *scroll = match p.k {
                    K::Named(Named::Up) => scroll.saturating_sub(1),
                    K::Named(Named::Down) => scroll.saturating_add(1).min(max),
                    K::Named(Named::PageUp) => scroll.saturating_sub(visible),
                    K::Named(Named::PageDown) => scroll.saturating_add(visible).min(max),
                    _ => *scroll,
                };
            }
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
                        let w = t.name().len() + if t.untitled_slot.is_some() { 6 } else { 4 };
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
        let palette = self.palette;
        let (w, h) = window.size();
        self.p.w = w;
        self.p.h = h;
        let rows = self.rows();
        let cols = self.cols();
        let tc = self.tree_cols();
        let gutter = self.gutter();
        self.p.rect(0, 0, w, h, palette.bg);
        // Tab bar.
        self.p.rect(0, 0, w, CELL_H, palette.bar);
        if tc > 0 { self.p.text(0, 0, tc, " FOLDER", palette.dim, palette.bar); }
        let mut x = tc;
        for i in 0..self.tabs.len() {
            let t = &self.tabs.list[i];
            let name = t.untitled_slot.map(|slot| format!("untitled {}", slot + 1)).unwrap_or_else(|| lossy(t.name()));
            let label = format!(" {}{} ", name, if t.ed.modified() { " *" } else { "  " });
            let (fg, bg) = if i == self.tabs.active { (palette.fg, palette.bg) } else { (palette.dim, palette.bar) };
            x += self.p.text(x, 0, cols.saturating_sub(x), &label, fg, bg);
            if x < cols { self.p.ch(x, 0, '|', palette.panel, palette.bar); x += 1; }
        }
        // File tree.
        if tc > 0 {
            self.p.rect(0, CELL_H, tc * CELL_W, rows * CELL_H, palette.panel);
            let first = self.tree.selected.saturating_sub(rows.saturating_sub(1));
            for (r, i) in (first..self.tree.rows.len()).enumerate().take(rows) {
                let row = self.tree.rows[i].clone();
                let sel = i == self.tree.selected;
                let bg = if sel { if self.focus == Focus::Tree { palette.selected } else { palette.curline } } else { palette.panel };
                self.p.rect(0, (r + 1) * CELL_H, tc * CELL_W, CELL_H, bg);
                let icon = if row.dir { if row.open { "v " } else { "> " } } else { "  " };
                let s = format!("{:width$}{}{}", "", icon, lossy(&row.name), width = row.depth * 2);
                self.p.text(0, r + 1, tc - 1, &s, if row.dir { palette.accent } else { palette.fg }, bg);
            }
        }
        // Editor.
        let ex = tc + gutter;
        let text_cols = cols.saturating_sub(ex);
        let numbers = self.numbers;
        let vim_block = self.keymap == Keymap::Vim && self.vim.block_cursor();
        let syntax_at = {
            let t = self.tab();
            t.follow(rows);
            // Horizontal scroll follows the primary cursor.
            let c = t.ed.cursor();
            let ls = t.ed.buf.line_start(c);
            let line = t.ed.buf.slice(ls..c);
            let ccol = display_cols(&line);
            if ccol < t.left { t.left = ccol.saturating_sub(8); }
            if ccol >= t.left + text_cols { t.left = ccol + 8 - text_cols; }
            t.syntax_state_at_top()
        };
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
        let mut syntax_state = match syntax_at { SyntaxAt::Ready(state) => state, _ => highlight::State::Normal };
        let mut syntax_exact = matches!(syntax_at, SyntaxAt::Ready(_));
        let mut cells: Vec<(usize, usize, char, u32, u32)> = Vec::new();
        let mut cursors: Vec<(usize, usize)> = Vec::new();
        for r in 0..rows {
            let y = r + 1;
            let end_or_long = t.syntax_line_end(line);
            let end = end_or_long.unwrap_or_else(|| (line + SYNTAX_LINE_MAX + 1).min(t.ed.buf.len()));
            let shown_end = end.min(line + 4096);
            let bytes = t.ed.buf.slice(line..shown_end);
            let is_cur = line == primary_line;
            let row_bg = if is_cur { palette.curline } else { palette.bg };
            if is_cur { self.p.rect(ex * CELL_W, y * CELL_H, text_cols * CELL_W, CELL_H, palette.curline); }
            if numbers {
                let label = match first_line { Some(n) => format!("{:>5} ", n + r + 1), None => String::from("    ? ") };
                self.p.text(tc, y, gutter, &label, if is_cur { palette.fg } else { palette.dim }, palette.bg);
            }
            // Lex the whole logical line so an opener beyond the 4096 drawn bytes
            // still carries into the following line. Very long lines stay plain.
            if syntax_exact && end_or_long.is_some() {
                let lex_bytes = if shown_end == end { bytes.clone() } else { t.ed.buf.slice(line..end) };
                syntax_state = highlight::line_state(lang, &lex_bytes, syntax_state, &mut spans);
            } else {
                syntax_exact = false;
                spans.clear();
            }
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
                let fg = if si < spans.len() && spans[si].0 <= k { palette.color(spans[si].2) } else { palette.fg };
                let selected = in_sels(&sels, off);
                let hit = hits.iter().any(|h| k >= *h && k < h + find.len());
                let bg = if selected { palette.selected } else if hit { palette.hit } else { row_bg };
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
            if end_or_long.is_none() || end >= t.ed.buf.len() { break; }
            line = end + 1;
        }
        for (x, y, c, fg, bg) in cells { self.p.ch(x, y, c, fg, bg); }
        for (dc, y) in cursors {
            if dc < left || dc >= left + text_cols { continue; }
            let x = (ex + dc - left) * CELL_W;
            if vim_block { self.p.rect(x, y * CELL_H + CELL_H - 3, CELL_W, 3, palette.cursor); } else { self.p.rect(x, y * CELL_H + 1, 2, CELL_H - 2, palette.cursor); }
        }
        // Status line.
        let sy = rows + 1;
        self.p.rect(0, sy * CELL_H, w, CELL_H, palette.bar);
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
        let syntax_note = if !syntax_exact { if matches!(syntax_at, SyntaxAt::Unavailable) || matches!(syntax_at, SyntaxAt::Ready(_)) { "syntax unavailable" } else { "syntax indexing" } } else { "" };
        let right = format!("Ln {ln}, Col {col}  {}{}  {} bytes  {} {} ", if t.ed.sels.len() > 1 { format!("{} cursors  ", t.ed.sels.len()) } else { String::new() },
            t.lang.name(), t.ed.buf.len(), if t.ed.buf.indexed() { "" } else { "indexing" }, syntax_note);
        let n = self.p.text(0, sy, cols, &left_s, if mode.is_empty() { palette.fg } else { palette.accent }, palette.bar);
        let rx = cols.saturating_sub(right.chars().count());
        if rx > n { self.p.text(rx, sy, cols - rx, &right, palette.dim, palette.bar); }
        self.draw_overlay(cols, rows);
        window.show_pixels(w, h);
    }

    fn draw_overlay(&mut self, cols: usize, rows: usize) {
        let palette = self.palette;
        let ow = (cols * 3 / 5).clamp(30.min(cols), 100.min(cols));
        let ox = (cols - ow) / 2;
        let mut lines: Vec<(String, bool)> = Vec::new();
        let (title, input): (&str, String) = match &self.overlay {
            Overlay::None => return,
            Overlay::Diagnostics { items, sel, .. } => {
                let first = sel.saturating_sub(9);
                for (i, issue) in items.iter().enumerate().skip(first).take(10) {
                    lines.push((format!("{}:{}  {}", issue.line.saturating_add(1), issue.character.saturating_add(1), issue.message), i == *sel));
                }
                ("Quil diagnostics  ", String::from("Up/Down choose; Enter jumps; Esc closes"))
            }
            Overlay::Completion { items, sel, .. } => {
                for (i, item) in items.iter().enumerate() { lines.push((item.clone(), i == *sel)); }
                ("Quil completions  ", String::from("Up/Down choose; Enter inserts; Esc closes"))
            }
            Overlay::Symbols { items, sel, .. } => {
                let first = sel.saturating_sub(9);
                for (i, symbol) in items.iter().enumerate().skip(first).take(10) {
                    lines.push((format!("{}  line {}", symbol.name, symbol.line + 1), i == *sel));
                }
                ("Quil symbols  ", String::from("Up/Down choose; Enter jumps; Esc closes"))
            }
            Overlay::Help { scroll } => {
                let mut all = keymap::shortcut_rows(self.keymap);
                all.insert(0, String::from("Ctrl+Shift+V  tracked Git changes; Right shows diff"));
                all.insert(0, String::from("Language diagnostics  Ctrl+Shift+J"));
                all.insert(0, String::from("Language completion  Ctrl+Shift+Space"));
                all.insert(0, String::from("Document symbols  Ctrl+Shift+M"));
                all.insert(0, String::from("Go to definition  Ctrl+Shift+G"));
                all.insert(0, String::from("Describe declaration  Ctrl+Shift+H"));
                for row in all.into_iter().skip(*scroll).take(rows.saturating_sub(2)) { lines.push((row, false)); }
                ("Quil shortcuts  ", format!("{}  (Up/Down, Esc)", match self.keymap { Keymap::Vim => "Vim", Keymap::Emacs => "Emacs", Keymap::VsCode => "Default" }))
            }
            Overlay::Git { changes, sel, note, head } => {
                lines.push((format!("HEAD: {head}"), false));
                if git_candidate_count(changes) > 0 {
                    lines.push((String::from("N = outside index; ignore rules not checked"), false));
                }
                if !note.is_empty() { lines.push((note.clone(), false)); }
                else if changes.is_empty() { lines.push((String::from("Indexed files match the worktree."), false)); }
                let first = sel.saturating_sub(9);
                for (i, change) in changes.iter().enumerate().skip(first).take(10) {
                    lines.push((format!("{}  {}", change.code, lossy(&change.path)), i == *sel));
                }
                ("Quil tracked changes  ", String::from("S stage modified or deleted; U undo last stage; Right diff; Enter opens"))
            }
            Overlay::GitDiff { path, lines: diff, scroll, note } => {
                if !note.is_empty() { lines.push((note.clone(), false)); }
                else if diff.is_empty() { lines.push((String::from("No text lines changed."), false)); }
                for line in diff.iter().skip(*scroll).take(rows.saturating_sub(3)) { lines.push((line.clone(), false)); }
                ("Quil diff  ", format!("{}  Up/Down scroll; Esc closes", lossy(path)))
            }
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
        self.p.rect(ox * CELL_W - 2, CELL_H - 2, ow * CELL_W + 4, oh * CELL_H + 4, palette.accent);
        self.p.rect(ox * CELL_W, CELL_H, ow * CELL_W, oh * CELL_H, palette.panel);
        let s = format!("{title}{input}_");
        self.p.text(ox, 1, ow, &s, palette.fg, palette.panel);
        for (r, (l, sel)) in lines.iter().enumerate().take(oh.saturating_sub(1)) {
            let bg = if *sel { palette.selected } else { palette.panel };
            self.p.rect(ox * CELL_W, (r + 2) * CELL_H, ow * CELL_W, CELL_H, bg);
            self.p.text(ox + 1, r + 2, ow - 1, l, if *sel { palette.fg } else { palette.dim }, bg);
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
    let theme = settings::theme();
    let palette = Palette::from_tokens(theme);
    let theme_generation = settings::generation().unwrap_or(0);
    sex_pdx::serial_record!("[quil.theme] mode={:?} accent={:06x}", theme.mode, palette.accent);
    let font = sex_font::Font::new(sex_font::GEIST_MONO).unwrap_or_else(|| sex_pdx::task_finish(4));
    let (w, h) = window.size();
    let p = Painter { px: window.pixels(), font, acc: vec![0; sex_font::raster::scratch_len(CELL_W, CELL_H)], glyphs: BTreeMap::new(), w, h };
    let mut app = App {
        tabs: Tabs::default(), tree: Tree::new(b""), show_tree: false, focus: Focus::Editor, root: Vec::new(), keymap: Keymap::VsCode,
        keys: Keys::default(), vim: Vim::new(), overlay: Overlay::None, clipboard: (Vec::new(), false), status: String::from("Ctrl+Shift+P: all commands"),
        numbers: true, find: Vec::new(), mark: false, quit_armed: false, drag: false,
        palette, theme_generation, theme_next: sex_pdx::get_ticks().saturating_add(1000),
        draft_next: sex_pdx::get_ticks().saturating_add(2000), drafted: BTreeMap::new(), git_stage_undo: None, p,
    };
    app.restore_untitled_tabs();
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
        let now = sex_pdx::get_ticks();
        let mut theme_changed = false;
        if now >= app.theme_next {
            app.theme_next = now.saturating_add(1000);
            if let Ok(generation) = settings::generation() {
                if generation != app.theme_generation {
                    app.theme_generation = generation;
                    let theme = settings::theme();
                    app.palette = Palette::from_tokens(theme);
                    theme_changed = true;
                    sex_pdx::serial_record!("[quil.theme] mode={:?} accent={:06x}", theme.mode, app.palette.accent);
                }
            }
        }
        if now >= app.draft_next {
            app.draft_next = now.saturating_add(2000);
            app.autosave_drafts();
        }
        match window.next_within(100) {
            Some(Event::Close) => { app.autosave_drafts(); sex_pdx::serial_record!("[quil.code.closed] target={} task={}", window.target, window.task); window.close() }
            Some(Event::Resize { .. }) | Some(Event::Drop { .. }) | Some(Event::DragEnded { .. } | Event::KeyUp(_)) => {}
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
                if t.ed.buf.indexed() && !theme_changed && !t.syntax_pending { continue; }
                t.ed.buf.index_step(8);
            }
        }
        app.draw(&mut window);
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }
