// SPDX-License-Identifier: GPL-3.0-only
//! Files: the file manager, and the open/save picker other programs ask through.
//!
//! It holds only what the person handed it when starting it (`files` in Spindle gives
//! the whole filesystem, `files DIR` one folder), so it is a program like any other.
//! Copy, cut and paste work between the SexOS disk and USB sticks or other disks under
//! `/media`, in the background with progress, and Esc stops them without leaving half a
//! file behind; Delete moves to the trash of the same disk, which shows where each item
//! came from, restores it, or empties for good; Ctrl+Z undoes, step by step.
//! When another program asks the person for a file, Files shows the question and hands
//! that program the one path chosen, nothing more.
#![no_std]
#![no_main]
extern crate alloc;

mod actions;
mod archive;
mod compare;
mod diskimg;
mod dupes;
mod jobs;
mod loom;
mod session;
mod meta;
mod places;
mod rename;
mod search;
mod thumbs;
mod usage;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::{self, Entry, Pick, Question};
use sex_app::{Event, Nav, Window};
use sex_pdx::fs as w;
use jobs::{Job, Kind, Outcome};
use search::{Index, Search};
use thumbs::Thumbs;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[files.app.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ROWS: usize = 18;
fn split_grid_fits(window: &mut Window) -> bool {
    let (width, height) = window.size();
    loom_core::split::grid_fits(width, height, thumbs::CELL_W, thumbs::CELL_H, thumbs::HEAD, thumbs::FOOT)
}
const TRASH: &[u8] = b".Trash";
const INFO: &[u8] = b".Trash/.info";

const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;
const ESC: u8 = 0x01;
const F2: u8 = 0x3C;
const F3: u8 = 0x3D;
const F5: u8 = 0x3F;
const SPACE: u8 = 0x39;
// Letters, PS/2 set 1, used with Ctrl.
const KEY_C: u8 = 0x2E;
const KEY_X: u8 = 0x2D;
const KEY_V: u8 = 0x2F;
const KEY_Z: u8 = 0x2C;
const KEY_N: u8 = 0x31;
const KEY_Y: u8 = 0x15;
const KEY_T: u8 = 0x14;
const KEY_W: u8 = 0x11;
const KEY_R: u8 = 0x13;
const KEY_H: u8 = 0x23;
const KEY_D: u8 = 0x20;
const KEY_S: u8 = 0x1F;
const KEY_E: u8 = 0x12;
const KEY_I: u8 = 0x17;
const KEY_F: u8 = 0x21;
const KEY_G: u8 = 0x22;
const KEY_B: u8 = 0x30;
const KEY_K: u8 = 0x25;
const HOME: &[u8] = sex_pdx::home_lock::HOME;
/// Size of the encrypted home Ctrl+K makes.
const HOME_MIB: u64 = 256;

/// One step Ctrl+Z can take back.
enum Undo { Moved { from: Vec<u8>, to: Vec<u8> }, MovedSame { from: Vec<u8>, to: Vec<u8> }, RenamedOne { from: Vec<u8>, to: Vec<u8> }, Trashed { from: Vec<u8>, to: Vec<u8>, info: Vec<u8> }, Copied { to: Vec<u8> }, Made { path: Vec<u8> },
    CopiedBatch(Vec<(Vec<u8>, Vec<u8>)>),
    MovedBatch(Vec<(Vec<u8>, Vec<u8>)>),
    TrashedBatch(Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>),
    /// A batch rename in `dir`: the moves that take it back, in order.
    Renamed { dir: Vec<u8>, moves: Vec<RenameMove> } }
enum HistoryJob { Undo(Undo), Redo(Undo) }
#[derive(Clone)]
struct RenameMove { from: Vec<u8>, to: Vec<u8>, id: u64 }
fn inverse_moves(moves: &[RenameMove]) -> Vec<RenameMove> {
    moves.iter().rev().map(|m| RenameMove { from: m.to.clone(), to: m.from.clone(), id: m.id }).collect()
}
enum Input { Rename(Vec<u8>), NewFolder, SaveAs, Search, Password(Secret) }
/// What a password typed into Files is for.
enum Secret { Unlock, New, Repeat(Vec<u8>), Current, ChangeTo(Vec<u8>), ChangeRepeat(Vec<u8>, Vec<u8>) }
struct FolderTab { cwd: Vec<u8>, cursor: usize, top: usize }
struct PicturePreview { path: Vec<u8>, width: usize, height: usize, pixels: Vec<u32> }
/// Something that can't be undone, waiting for the same key a second time.
#[derive(PartialEq, Eq)]
enum Confirm { DeleteForever(Vec<u8>), EmptyTrash(Vec<u8>) }
struct App {
    root: Vec<u8>,
    cwd: Vec<u8>,
    entries: Vec<Entry>,
    cursor: usize,
    selection_anchor: Option<usize>,
    selection_marks: Vec<usize>,
    top: usize,
    tabs: Vec<FolderTab>,
    active_tab: usize,
    /// Local folders shown by Ctrl+Shift+B, scoped to this run's Files grant.
    quick_places: Option<(usize, Vec<loom_core::quick_places::Place>)>,
    previous_folder: Option<Vec<u8>>,
    /// Show this tab beside the next tab in the list; keys act in the active tab.
    split: bool,
    clip: Option<(Vec<u8>, bool)>,
    /// A contiguous copied or cut range; it pastes as one undoable action.
    clip_batch: Option<(Vec<Vec<u8>>, bool)>,
    undo: Vec<Undo>,
    /// Reversible rename, trash and empty-folder steps enter this stack.
    redo: Vec<Undo>,
    input: Option<(Input, Vec<u8>)>,
    status: String,
    question: Option<Question>,
    /// A program asking to use a whole disk raw (Corset): Y allows, N refuses.
    disk_ask: Option<files::disks::Question>,
    /// This run can see questions: it holds the whole filesystem.
    portal: bool,
    /// Copying, moving, deleting or checking in the background.
    job: Option<Job>,
    /// What Ctrl+Z will take back once the job finishes.
    job_undo: Option<Undo>,
    /// Cross-volume history becomes final only when its background transfer completes.
    history_job: Option<HistoryJob>,
    /// Name to select once the job finishes.
    job_select: Option<Vec<u8>>,
    last_drawn: u64,
    theme_generation: u64,
    last_theme_check: u64,
    last_pct: u64,
    confirm: Option<Confirm>,
    /// In a trash folder: where each entry came from, by row.
    origins: Vec<Option<Vec<u8>>>,
    /// The `/media` volume being looked at, while it is there: noticed when it goes.
    watching: Option<Vec<u8>>,
    last_look: u64,
    index: Index,
    /// A search running in the background.
    search: Option<Search>,
    /// The walk at start that fills the index Voyeur asks, between everything else.
    crawl: Option<Search>,
    /// The listing shows search results for this, not the folder.
    found: Option<Vec<u8>>,
    /// Ctrl+G: the folder as a grid of thumbnails.
    thumbs: Thumbs,
    /// The window shows pixels (the grid), not text.
    showing_pixels: bool,
    /// Archives, ratings and tags, batch rename, duplicates, disk usage, compare.
    loom: loom::Loom,
    /// A file Enter opened, until its program has started (or couldn't).
    opening: Option<(u32, Vec<u8>)>,
    /// Startup loads saved tabs before folder navigation starts persisting paths.
    session_ready: bool,
    preview: Option<(Vec<u8>, loom_core::preview::TextPreview)>,
    picture_preview: Option<PicturePreview>,
}

/// The path as Files knows a granted one: `@/absolute/path`.
fn at(abs: &[u8]) -> Vec<u8> { let mut p = vec![b'@']; p.extend_from_slice(abs); p }
fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut p = dir.to_vec();
    if p.last() != Some(&b'/') { p.push(b'/'); }
    p.extend_from_slice(name);
    p
}
fn parent(abs: &[u8]) -> Vec<u8> {
    match abs.iter().rposition(|b| *b == b'/') { Some(0) | None => b"/".to_vec(), Some(i) => abs[..i].to_vec() }
}
fn base(abs: &[u8]) -> &[u8] { abs.rsplit(|b| *b == b'/').next().unwrap_or(abs) }
fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }
/// The disk a path is on: a volume under /media, or the SexOS filesystem.
fn disk_root(abs: &[u8]) -> Vec<u8> {
    if let Some(rest) = abs.strip_prefix(b"/media/") {
        let name = rest.split(|b| *b == b'/').next().unwrap_or(rest);
        return join(b"/media", name);
    }
    b"/".to_vec()
}
fn human(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{} B", b),
        b if b < 1024 * 1024 => format!("{} KiB", b / 1024),
        b if b < 1024 * 1024 * 1024 => format!("{} MiB", b / (1024 * 1024)),
        b => format!("{}.{} GiB", b / (1024 * 1024 * 1024), b % (1024 * 1024 * 1024) * 10 / (1024 * 1024 * 1024)),
    }
}

impl App {
    fn tab_ready(&self) -> bool {
        self.question.is_none() && self.disk_ask.is_none() && self.job.is_none() && self.search.is_none()
            && self.quick_places.is_none()
            && self.found.is_none() && self.loom.inside.is_none() && self.loom.image.is_none()
            && !self.loom.places.active() && matches!(&self.loom.mode, loom::Mode::None)
            && self.loom.prompt.is_none()
    }
    fn remember_tab(&mut self) {
        self.tabs[self.active_tab] = FolderTab { cwd: self.cwd.clone(), cursor: self.cursor, top: self.top };
    }
    fn show_tab(&mut self, window: &mut Window, next: usize) {
        if !self.tab_ready() { self.status = "Finish the current view before changing tabs.".into(); return; }
        self.remember_tab();
        self.active_tab = next;
        let tab = &self.tabs[next];
        let path = tab.cwd.clone();
        let cursor = tab.cursor;
        let top = tab.top;
        if files::stat(window, &at(&path)).is_err() { let root = self.root.clone(); self.go(window, root); }
        else { self.cwd = path; self.cursor = cursor; self.top = top; self.load(window); }
        let disk = disk_root(&self.cwd);
        self.watching = (disk != b"/" && self.status.is_empty()).then_some(disk);
        sex_pdx::serial_record!("[loom.tab.shown] active={} count={} dir={}", next + 1, self.tabs.len(), shown(&self.cwd));
        self.session_save(window);
    }
    fn new_tab(&mut self, window: &mut Window) {
        if !self.tab_ready() { self.status = "Finish the current view before opening a tab.".into(); return; }
        if self.tabs.len() >= 12 { self.status = "Twelve tabs are open.".into(); return; }
        self.remember_tab();
        self.tabs.push(FolderTab { cwd: self.cwd.clone(), cursor: 0, top: 0 });
        self.active_tab = self.tabs.len() - 1;
        self.cursor = 0; self.top = 0; self.load(window);
        sex_pdx::serial_record!("[loom.tab.opened] active={} count={} dir={}", self.active_tab + 1, self.tabs.len(), shown(&self.cwd));
        self.session_save(window);
    }
    fn close_tab(&mut self, window: &mut Window) {
        if !self.tab_ready() { self.status = "Finish the current view before closing a tab.".into(); return; }
        if self.tabs.len() == 1 { self.status = "One tab remains.".into(); return; }
        self.tabs.remove(self.active_tab);
        if self.tabs.len() < 2 { self.split = false; }
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);
        let tab = &self.tabs[self.active_tab];
        let path = tab.cwd.clone(); let cursor = tab.cursor; let top = tab.top;
        if files::stat(window, &at(&path)).is_err() { let root = self.root.clone(); self.go(window, root); }
        else { self.cwd = path; self.cursor = cursor; self.top = top; self.load(window); }
        let disk = disk_root(&self.cwd);
        self.watching = (disk != b"/" && self.status.is_empty()).then_some(disk);
        sex_pdx::serial_record!("[loom.tab.closed] active={} count={} dir={}", self.active_tab + 1, self.tabs.len(), shown(&self.cwd));
        self.session_save(window);
    }
    fn load(&mut self, window: &mut Window) {
        self.found = None;
        match files::list(window, &at(&self.cwd)) {
            Ok(mut list) => {
                // A copy still being written (or cut short by a pulled stick) is not a file yet.
                list.retain(|e| e.name != TRASH && e.name != meta::FILE && e.name != session::FILE && e.name != session::PREVIOUS && !(e.name.first() == Some(&b'.') && e.name.ends_with(b".part")));
                list.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
                self.entries = list;
            }
            Err(e) => {
                self.entries.clear();
                self.status = format!("Can't open {}: {}", shown(&self.cwd), shown(files::message(e)));
                sex_pdx::serial_record!("[files.app.list.failed] dir={} error={}", shown(&self.cwd), e);
            }
        }
        self.origins.clear();
        if base(&self.cwd) == TRASH {
            self.entries.retain(|e| e.name != b".info");
            let info = join(&self.cwd, b".info");
            for i in 0..self.entries.len() {
                let note = join(&info, &self.entries[i].name);
                let mut buf = [0u8; w::PATH_BYTES];
                self.origins.push(files::read(window, &at(&note), &mut buf).ok().map(|n| buf[..n].to_vec()));
            }
        }
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        self.selection_anchor = None;
        self.selection_marks.clear();
        sex_pdx::serial_record!("[files.app.list] dir={} count={}", shown(&self.cwd), self.entries.len());
        self.thumbs.folder(&self.cwd, &self.entries);
    }
    fn go(&mut self, window: &mut Window, dir: Vec<u8>) {
        if self.cwd != dir { self.previous_folder = Some(self.cwd.clone()); }
        self.cwd = dir; self.cursor = 0; self.top = 0; self.load(window);
        let root = disk_root(&self.cwd);
        self.watching = (root != b"/" && self.status.is_empty()).then_some(root);
        if self.session_ready && files::stat(window, &at(&self.cwd)).is_ok_and(|st| st.dir) {
            self.session_save(window);
        }
    }
    fn open_quick_places(&mut self, window: &mut Window) {
        if !self.tab_ready() { self.status = "Finish the current view before opening Places.".into(); return; }
        let choices: Vec<_> = loom_core::quick_places::candidates(&self.root, &self.cwd,
            self.previous_folder.as_deref(), HOME).into_iter()
            .filter(|place| files::stat(window, &at(&place.path)).is_ok_and(|st| st.dir)).collect();
        if choices.is_empty() { self.status = "No folders are available within this grant.".into(); return; }
        sex_pdx::serial_record!("[loom.quick-places.open] count={} dir={}", choices.len(), shown(&self.cwd));
        self.quick_places = Some((0, choices));
    }
    fn quick_places_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let Some((cursor, choices)) = self.quick_places.as_mut() else { return };
        match (key.code, sex_app::nav(key)) {
            (ESC | BACKSPACE, _) => { self.quick_places = None; sex_pdx::serial_record!("[loom.quick-places.closed]"); }
            (_, Some(Nav::Up)) => *cursor = cursor.saturating_sub(1),
            (_, Some(Nav::Down)) => *cursor = (*cursor + 1).min(choices.len() - 1),
            (_, Some(Nav::Home)) => *cursor = 0,
            (_, Some(Nav::End)) => *cursor = choices.len() - 1,
            (ENTER, _) => {
                let place = choices[*cursor].clone();
                self.quick_places = None;
                self.go(window, place.path.clone());
                sex_pdx::serial_record!("[loom.quick-places.go] name={} dir={}", place.label, shown(&place.path));
            }
            _ => {}
        }
    }
    /// Ctrl+E outside a trash: Eject the stick or disk the current folder is on (or the one selected in
    /// /media). Everything is written out first; then it is safe to pull out.
    fn eject(&mut self, window: &mut Window) {
        let target = if self.cwd == w::MEDIA { self.selected() } else { Some(disk_root(&self.cwd)).filter(|r| r != b"/") };
        let Some(vol) = target else { self.status = "Ctrl+E ejects a stick or disk: open it, or pick it in Ctrl+D disks.".into(); return };
        let name = base(&vol).to_vec();
        self.status = format!("Writing out {}...", shown(&name));
        self.render(window);
        let _ = files::sync(window);
        match files::eject(window, &at(&vol)) {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.ejected] name={}", shown(&name));
                self.watching = None;
                if self.root.starts_with(&vol) {
                    // This run was given only that disk: nothing is left to show.
                    self.cwd = self.root.clone(); self.entries.clear(); self.origins.clear();
                } else { let m = w::MEDIA.to_vec(); self.go(window, m); }
                self.status = format!("{} can be pulled out safely now.", shown(&name));
            }
            Err(e) => {
                sex_pdx::serial_record!("[files.app.eject.failed] name={} error={}", shown(&name), e);
                self.status = format!("Couldn't eject {}: {}. Keep it plugged in.", shown(&name), shown(files::message(e)));
            }
        }
    }
    /// The volume being looked at disappeared without Eject: say so, once.
    fn look_for_pulled(&mut self, window: &mut Window) -> bool {
        let Some(vol) = self.watching.clone() else { return false };
        if files::stat(window, &at(&vol)).is_ok() { return false; }
        self.watching = None;
        let name = base(&vol).to_vec();
        sex_pdx::serial_record!("[files.app.pulled] name={}", shown(&name));
        self.entries.clear(); self.origins.clear();
        self.status = format!("{} was pulled out without Ctrl+E Eject: a file being written to it may be cut short. Plug it back in and copy again.", shown(&name));
        true
    }
    fn selected(&self) -> Option<Vec<u8>> {
        let rows = self.selected_rows();
        let row = if self.selection_active() && rows.len() == 1 { rows[0] } else { self.cursor };
        self.entries.get(row).map(|e| join(&self.cwd, &e.name))
    }
    fn select(&mut self, name: &[u8]) { if let Some(i) = self.entries.iter().position(|e| e.name == name) { self.cursor = i; self.selection_anchor = None; self.selection_marks.clear(); } }
    fn selection(&self) -> Option<core::ops::RangeInclusive<usize>> {
        loom_core::selection::rows(self.selection_anchor, self.cursor, self.entries.len())
    }
    fn selected_paths(&self) -> Vec<Vec<u8>> {
        self.selected_rows().into_iter().filter_map(|i| self.entries.get(i))
            .map(|e| join(&self.cwd, &e.name)).collect()
    }
    fn selected_rows(&self) -> Vec<usize> {
        loom_core::selection::selected_rows(self.selection_anchor, self.cursor, &self.selection_marks, self.entries.len())
    }
    fn selection_active(&self) -> bool { self.selection_anchor.is_some() || !self.selection_marks.is_empty() }
    fn marked(&self, row: usize) -> bool {
        self.selection_marks.contains(&row) || self.selection_anchor.is_some()
            && self.selection().is_some_and(|range| range.contains(&row))
    }
    fn toggle_focus(&mut self) {
        if self.entries.is_empty() { return; }
        self.selection_marks = loom_core::selection::toggle_row(self.selection_anchor, self.cursor,
            &self.selection_marks, self.entries.len());
        self.selection_anchor = None;
        sex_pdx::serial_record!("[files.app.selection.disjoint] count={} focus={}", self.selection_marks.len(), self.cursor);
    }
    fn grid_click(&mut self, row: usize, control: bool) {
        let old = self.cursor;
        self.cursor = row;
        if control {
            if !self.selection_active() && old != row { self.selection_marks.push(old); }
            self.toggle_focus();
        } else {
            self.selection_anchor = None;
            self.selection_marks.clear();
        }
        sex_pdx::serial_record!("[files.app.grid.select] focus={} count={} control={}",
            self.cursor, self.selected_rows().len(), control as u8);
    }
    fn move_selection(&mut self, to: usize, extend: bool) {
        let old = self.cursor;
        self.cursor = to.min(self.entries.len().saturating_sub(1));
        if !extend || self.thumbs.on { self.selection_marks.clear(); }
        self.selection_anchor = if extend && !self.thumbs.on && !self.entries.is_empty() {
            Some(self.selection_anchor.unwrap_or(old))
        } else { None };
        if extend {
            let count = self.selected_rows().len();
            sex_pdx::serial_record!("[files.app.selection] count={} first={} last={}", count,
                self.selection().map(|r| *r.start()).unwrap_or(0), self.selection().map(|r| *r.end()).unwrap_or(0));
        }
    }

    /// A pair of live folder listings. Navigation remains in the active tab; the
    /// existing Ctrl+PageUp/PageDown changes which tab receives the keys.
    fn render_split(&mut self, window: &mut Window) {
        let other = (self.active_tab + 1) % self.tabs.len();
        let other_tab = &self.tabs[other];
        let mut t = String::from("Loom split view  Ctrl+PageUp/PageDown active tab  Ctrl+Shift+\\ close split\n");
        t.push_str(&loom_core::split::columns(
            &format!("> [{}] {}", self.active_tab + 1, shown(&self.cwd)),
            &format!("  [{}] {}", other + 1, shown(&other_tab.cwd)), 38));
        t.push_str("\n\n");
        if self.cursor < self.top { self.top = self.cursor; }
        if self.cursor >= self.top + ROWS { self.top = self.cursor + 1 - ROWS; }
        let left: Vec<String> = self.entries.iter().enumerate().skip(self.top).take(ROWS)
            .map(|(i, e)| format!("{}{} {}{}", if i == self.cursor { ">" } else { " " },
                if self.marked(i) { "*" } else { " " },
                shown(&e.name), if e.dir { "/" } else { "" }))
            .collect();
        let right: Vec<String> = match files::list(window, &at(&other_tab.cwd)) {
            Ok(mut entries) => {
                entries.retain(|e| e.name != TRASH && e.name != meta::FILE && e.name != session::FILE && e.name != session::PREVIOUS && !(e.name.first() == Some(&b'.') && e.name.ends_with(b".part")));
                entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
                entries.iter().enumerate().skip(other_tab.top).take(ROWS)
                    .map(|(i, e)| format!("{} {}{}", if i == other_tab.cursor { ">" } else { " " }, shown(&e.name), if e.dir { "/" } else { "" }))
                    .collect()
            }
            Err(_) => vec![String::from("Folder unavailable")],
        };
        for i in 0..ROWS {
            t.push_str(&loom_core::split::columns(left.get(i).map(String::as_str).unwrap_or(""), right.get(i).map(String::as_str).unwrap_or(""), 38));
            t.push('\n');
        }
        if !self.status.is_empty() { t.push('\n'); t.push_str(&self.status); t.push('\n'); }
        window.show(t.as_bytes());
    }

    fn preview_open(&mut self, window: &mut Window) {
        let Some(e) = self.entries.get(self.cursor) else { return };
        if e.dir { self.status = "Select a file for F3 preview.".into(); return; }
        let (name, size) = (e.name.clone(), e.size);
        let path = join(&self.cwd, &name);
        if thumbs::kind(&name, false).is_none() { self.picture_preview_open(window, path, size); return; }
        let max = loom_core::preview::MAX_BYTES;
        let mut bytes = vec![0; (size as usize).min(max)];
        let n = match files::read(window, &at(&path), &mut bytes) {
            Ok(n) => n,
            Err(err) => { self.status = format!("Couldn't preview {}: {}.", shown(&name), shown(files::message(err))); return; }
        };
        bytes.truncate(n);
        match loom_core::preview::TextPreview::parse(&bytes, size > max as u64) {
            Some(preview) => {
                sex_pdx::serial_record!("[files.app.preview.opened] path={} lines={} truncated={}", shown(&path), preview.len(), preview.truncated as u8);
                self.preview = Some((path, preview));
            }
            None => self.status = "F3 preview opens UTF-8 text only; use Enter for this file.".into(),
        }
    }
    fn picture_preview_open(&mut self, window: &mut Window, path: Vec<u8>, size: u64) {
        const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
        if size == 0 || size > MAX_IMAGE_BYTES { self.status = "F3 previews pictures up to 16 MiB.".into(); return; }
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(size as usize).is_err() { self.status = "There isn't room to preview this picture.".into(); return; }
        bytes.resize(size as usize, 0);
        let n = match files::read(window, &at(&path), &mut bytes) {
            Ok(n) => n,
            Err(e) => { self.status = format!("Couldn't preview {}: {}.", shown(base(&path)), shown(files::message(e))); return; }
        };
        bytes.truncate(n);
        let (width, height) = window.size();
        let image = sex_image::decode(&bytes, &sex_image::Limits::PHOTO)
            .and_then(|image| image.fit_within(width.saturating_sub(32).max(1), height.saturating_sub(80).max(1)));
        match image {
            Ok(image) => {
                sex_pdx::serial_record!("[files.app.picture.preview.opened] path={} w={} h={}", shown(&path), image.width, image.height);
                self.picture_preview = Some(PicturePreview { path, width: image.width, height: image.height, pixels: image.pixels });
            }
            Err(_) => self.status = "F3 couldn't decode this picture; use Enter to open it.".into(),
        }
    }

    fn render(&mut self, window: &mut Window) {
        if let Some(p) = &self.picture_preview {
            self.thumbs.preview(window, &p.path, p.width, p.height, &p.pixels);
            self.showing_pixels = true;
            return;
        }
        if let Some((path, preview)) = &self.preview {
            if self.showing_pixels { window.show_pixels(0, 0); self.showing_pixels = false; }
            let mut t = format!("Loom  Preview: {}\n\n", shown(path));
            for (i, line) in preview.page().iter().enumerate() {
                t.push_str(&format!("{:>5}  {}\n", preview.offset + i + 1, line));
            }
            t.push_str(&format!("\nLines {}-{} of {}{}  Up/Down scroll  Home/End jump  Esc close\n",
                preview.offset + 1, (preview.offset + preview.page().len()).min(preview.len()), preview.len(),
                if preview.truncated { "  (first 32 KiB)" } else { "" }));
            window.show(t.as_bytes());
            return;
        }
        if self.loom_render(window) { return; }
        if let Some((cursor, choices)) = &self.quick_places {
            if self.showing_pixels { window.show_pixels(0, 0); self.showing_pixels = false; }
            let mut text = String::from("Loom Places  Up/Down choose  Enter opens  Esc closes\n\n");
            for (i, place) in choices.iter().enumerate() {
                text.push_str(&format!("{} {}  {}\n", if i == *cursor { '>' } else { ' ' }, place.label, shown(&place.path)));
            }
            text.push_str("\nOnly folders available through this Loom window are listed.\n");
            window.show(text.as_bytes());
            return;
        }
        let split_grid = self.split && self.tabs.len() > 1 && self.thumbs.on && self.tab_ready()
            && self.input.is_none();
        if split_grid && !split_grid_fits(window) {
            if self.showing_pixels { window.show_pixels(0, 0); self.showing_pixels = false; }
            self.render_split(window);
            return;
        }
        if split_grid {
            let other = (self.active_tab + 1) % self.tabs.len();
            let other_path = self.tabs[other].cwd.clone();
            let other_cursor = self.tabs[other].cursor;
            let mut other_top = self.tabs[other].top;
            let mut other_entries = files::list(window, &at(&other_path)).unwrap_or_default();
            other_entries.retain(|e| e.name != TRASH && e.name != meta::FILE && e.name != session::FILE
                && e.name != session::PREVIOUS && !(e.name.first() == Some(&b'.') && e.name.ends_with(b".part")));
            other_entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
            let left = format!("> [{}] {}{}", self.active_tab + 1, shown(&self.cwd),
                if self.selection_active() { format!(" · {} marked", self.selected_rows().len()) } else { String::new() });
            let right = format!("  [{}] {}", other + 1, shown(&other_path));
            let selected = self.selected_rows();
            let other_selected = [other_cursor];
            self.thumbs.draw_split(window, (&left, &self.cwd, &self.entries, self.cursor, &selected, &mut self.top),
                (&right, &other_path, &other_entries, other_cursor, &other_selected, &mut other_top),
                "Ctrl+PageUp/PageDown changes pane  ·  Ctrl+G list  ·  Ctrl+Shift+\\ close split");
            self.tabs[other].top = other_top;
            self.showing_pixels = true;
            sex_pdx::serial_record!("[loom.split.grid] active={} pair={} left={} right={}", self.active_tab + 1, other + 1, shown(&self.cwd), shown(&other_path));
            return;
        }
        if self.thumbs.on && self.input.is_none() && self.search.is_none() && self.question.is_none() && self.disk_ask.is_none() {
            let mut head = format!("Loom [{}/{}]  {}", self.active_tab + 1, self.tabs.len(), shown(&self.cwd));
            if let Some(q) = &self.found { head = format!("Found {} for \"{}\" in {}", self.entries.len(), shown(q), shown(&self.cwd)); }
            if self.selection_active() { head.push_str(&format!(" · {} marked", self.selected_rows().len())); }
            let mut foot = String::new();
            if let Some(job) = &self.job { foot.push_str(&job.progress()); foot.push_str("  "); }
            if !self.status.is_empty() { foot.push_str(&self.status); foot.push_str("  "); }
            if self.thumbs.busy() { foot.push_str("Making thumbnails...  "); }
            foot.push_str("Ctrl+click marks  Ctrl+Space marks  Ctrl+G list");
            let mut top = self.top;
            let selected = self.selected_rows();
            self.thumbs.draw(window, &head, &foot, &self.cwd, &self.entries, self.cursor, &selected, &mut top);
            self.top = top;
            self.showing_pixels = true;
            return;
        }
        if self.showing_pixels { window.show_pixels(0, 0); self.showing_pixels = false; self.top = 0; }
        if self.split && self.tabs.len() > 1 && self.tab_ready() {
            self.render_split(window);
            return;
        }
        let mut t = String::new();
        if let Some(sr) = &self.search {
            t.push_str(&format!("Searching {} for \"{}\": {} found in {} files so far. Esc stops.\n\n", shown(&sr.base), shown(&sr.query), sr.results.len(), sr.looked));
        } else if let Some(q) = &self.found {
            t.push_str(&format!("Found {} for \"{}\" in {}. Enter opens its folder, Esc goes back.\n", self.entries.len(), shown(q), shown(&self.cwd)));
        }
        let space = files::space(window, &at(&self.cwd)).ok().filter(|s| s.0 > 0).map(|(total, free)| format!("  {} free of {}", human(free), human(total))).unwrap_or_default();
        t.push_str(&format!("Loom [{}/{}]  {}{}\n", self.active_tab + 1, self.tabs.len(), shown(&self.cwd), space));
        if self.selection_active() {
            let count = self.selected_rows().len();
            t.push_str(&format!("{count} selected  ·  Ctrl+C copies  ·  Ctrl+X cuts  ·  Delete trashes  ·  Ctrl+Z undoes the batch\n"));
        }
        if self.tabs.len() > 1 {
            t.push_str("Tabs:");
            for (i, tab) in self.tabs.iter().enumerate() {
                let path = if i == self.active_tab { &self.cwd } else { &tab.cwd };
                t.push_str(&format!("  {}{}:{}", if i == self.active_tab { ">" } else { "" }, i + 1, shown(base(path))));
            }
            t.push('\n');
        }
        if let Some(q) = &self.question {
            let what = match q.mode { Pick::Open => "open a file: Enter chooses it", Pick::Save => "save a file: Ctrl+S names it here", Pick::Folder => "pick a folder: Ctrl+S chooses this one" };
            t.push_str(&format!(">> {} asks to {}. Esc says no.\n", shown(&q.program), what));
        }
        if let Some(q) = &self.disk_ask {
            t.push_str(&format!(">> {} asks to use the whole disk {} directly: it can change or erase everything on it until it closes. Y allows, N refuses.\n", shown(&q.program), shown(&q.disk)));
        }
        t.push('\n');
        if self.entries.is_empty() { t.push_str("  (empty)\n"); }
        if self.cursor < self.top { self.top = self.cursor; }
        if self.cursor >= self.top + ROWS { self.top = self.cursor + 1 - ROWS; }
        for (i, e) in self.entries.iter().enumerate().skip(self.top).take(ROWS) {
            let marked = self.marked(i);
            let mark = match (i == self.cursor, marked) { (true, true) => ">*", (true, false) => "> ", (false, true) => " *", (false, false) => "  " };
            let mut from = match self.origins.get(i) { Some(Some(o)) => format!("   from {}", shown(o)), _ => String::new() };
            from.push_str(&self.loom_badge(&e.name));
            if e.dir { t.push_str(&format!("{}{}/{}\n", mark, shown(&e.name), from)); } else { t.push_str(&format!("{}{}   {}{}\n", mark, shown(&e.name), human(e.size), from)); }
        }
        if self.entries.len() > self.top + ROWS { t.push_str(&format!("  ... {} more\n", self.entries.len() - self.top - ROWS)); }
        t.push('\n');
        if let Some((kind, text)) = &self.input {
            let label = match kind { Input::Rename(_) => "Rename to", Input::NewFolder => "New folder", Input::SaveAs => "Save as", Input::Search => "Search names and words",
                Input::Password(Secret::Unlock) => "Password for home", Input::Password(Secret::New) => "New password for home (8 or more)", Input::Password(Secret::Repeat(_)) => "The same password again",
                Input::Password(Secret::Current) => "Current password for home", Input::Password(Secret::ChangeTo(_)) => "New password (8 or more)",
                Input::Password(Secret::ChangeRepeat(..)) => "The new password again" };
            // A password shows only how long it is.
            let shown_text = if matches!(kind, Input::Password(_)) { "*".repeat(text.len()) } else { String::from(shown(text)) };
            t.push_str(&format!("{}: {}|   (Enter confirms, Esc cancels)\n", label, shown_text));
        } else if !self.status.is_empty() { t.push_str(&self.status); t.push('\n'); }
        if let Some(job) = &self.job { t.push_str(&job.progress()); t.push('\n'); }
        t.push_str("Enter open  Backspace up  F2 rename  F3 preview  Ctrl+I size and checksum\nCtrl+C/X/V copy, cut, paste  Ctrl+N folder\nDel trash  Shift+Del delete for good  Ctrl+T show trash\nIn the trash: Ctrl+R restore  Ctrl+E empty\nOn a stick: Ctrl+E eject  Ctrl+D disks  Ctrl+Z undo  Ctrl+Y redo move/rename/trash/folder  Ctrl+H home\nCtrl+F search names and words in files  Ctrl+G thumbnails  Ctrl+K home lock  Ctrl+Shift+K its password\nCtrl+Shift+T new tab  Ctrl+PageUp/PageDown switch  Ctrl+Shift+W close tab  Ctrl+Shift+\\ split\nEnter on an archive opens it  Ctrl+Shift+X extract  Ctrl+Shift+F pack as...  Ctrl+Shift+A zip  Ctrl+Shift+N tar  Ctrl+Shift+M tar.zst\nCtrl+0..5 rate  Ctrl+Shift+G tags  Ctrl+Shift+R rename all  Ctrl+Shift+D duplicates\nCtrl+U disk usage  Ctrl+Shift+C compare  Ctrl+Shift+O actions\n");
        window.show(t.as_bytes());
    }

    /// Starts background work; only one job runs at a time.
    fn start(&mut self, window: &mut Window, job: Result<Job, u8>, undo: Option<Undo>, select: Option<Vec<u8>>) {
        match job {
            Ok(job) => {
                sex_pdx::serial_record!("[files.app.job.start] kind={:?} what={} total={}", job.kind, shown(&job.what), job.total);
                self.job = Some(job);
                self.job_undo = undo;
                self.job_select = select;
                self.last_pct = 0;
                self.last_drawn = 0;
            }
            Err(e) => self.status = format!("Couldn't start: {}.", shown(files::message(e))),
        }
        let _ = window;
    }
    /// One piece of the background job. True when the window should be drawn again.
    fn work(&mut self, window: &mut Window) -> bool {
        if self.loom.busy() { return self.loom_work(); }
        if self.search.is_some() { return self.search_step(window); }
        let Some(job) = self.job.as_mut() else {
            if self.thumbs.on && self.thumbs.busy() { return self.thumbs.step(window); }
            if let Some(c) = self.crawl.as_mut() {
                if let search::Step::Done = c.step(window, &mut self.index) {
                    sex_pdx::serial_record!("[files.app.indexed] base={} files={} looked={} read={} ms={}", shown(&c.base), self.index.len(), c.looked, c.read, c.elapsed_ms());
                    self.crawl = None;
                }
                return false;
            }
            return self.thumbs.on && self.thumbs.step(window);
        };
        match job.step(window) {
            Outcome::Running => {
                let pct = job.percent();
                if pct / 10 != self.last_pct / 10 {
                    sex_pdx::serial_record!("[files.app.job.progress] kind={:?} pct={} done={} total={}", job.kind, pct, job.done, job.total);
                }
                self.last_pct = pct;
                let now = sex_pdx::get_ticks();
                if now.saturating_sub(self.last_drawn) >= 100 { self.last_drawn = now; return true; }
                false
            }
            Outcome::Done(kind, summary) => {
                let job = self.job.take().unwrap();
                match kind {
                    Kind::Sum => sex_pdx::serial_record!("[files.app.sum] path={} bytes={} sha256={}", shown(&job.from), job.total, job.sha256().unwrap_or_default()),
                    _ => sex_pdx::serial_record!("[files.app.job.done] kind={:?} what={} from={} to={} bytes={}", kind, shown(&job.what), shown(&job.from), shown(&job.to), job.done),
                }
                if kind == Kind::Move { self.meta_moved(window, &job.from, &job.to); }
                if job.batch_count > 0 {
                    sex_pdx::serial_record!("[files.app.pasted.batch] count={} cut=0", job.batch_count);
                } else if kind == Kind::Copy || kind == Kind::Move {
                    sex_pdx::serial_record!("[files.app.pasted] from={} to={} cut={}", shown(&job.from), shown(&job.to), (kind == Kind::Move) as u8);
                }
                if let Some(history) = self.history_job.take() {
                    match history {
                        HistoryJob::Undo(step) => {
                            if self.redo.len() == 64 { self.redo.remove(0); }
                            self.redo.push(step);
                            sex_pdx::serial_record!("[files.app.undone] left={}", self.undo.len());
                        }
                        HistoryJob::Redo(step) => {
                            if self.undo.len() == 64 { self.undo.remove(0); }
                            self.undo.push(step);
                            sex_pdx::serial_record!("[files.app.redone] left={}", self.redo.len());
                        }
                    }
                } else if let Some(u) = self.job_undo.take() {
                    if let Undo::CopiedBatch(items) = &u {
                        for (from, to) in items { self.meta_copied(window, from, to); }
                    }
                    self.record_undo(u);
                }
                self.status = summary;
                self.load(window);
                if let Some(name) = self.job_select.take() { self.select(&name); }
                true
            }
            Outcome::Failed(kind, why, cleaned) => {
                sex_pdx::serial_record!("[files.app.job.failed] kind={:?} cleaned={} why={}", kind, cleaned as u8, why);
                let committed = self.job.as_ref().is_some_and(|job| job.committed);
                self.job = None;
                self.job_undo = None;
                if let Some(history) = self.history_job.take() {
                    // Once source removal began, both locations may hold data.
                    // The old history step no longer describes a safe replay.
                    if !committed && cleaned { self.restore_history(history); }
                }
                self.status = match (kind, cleaned) {
                    (Kind::Sum, _) => format!("Couldn't finish checking: {}. No files changed.", why),
                    (Kind::Move, _) if committed => format!("Couldn't finish moving: {}. Check both locations before trying again.", why),
                    (Kind::Copy | Kind::Move, true) => format!("Couldn't finish: {}. Copied paths were removed; source files remain.", why),
                    _ => format!("Couldn't finish: {}. Storage state is unconfirmed; check source and destination before retrying.", why),
                };
                if !self.look_for_pulled(window) { self.load(window); }
                true
            }
        }
    }
    /// Voyeur's questions: answered from the index, nothing read for them. A chosen
    /// file opens in its program, or shows here selected (reveal), or with its
    /// actions (open with). Copy is the Shell's and never comes here.
    fn voyeur(&mut self, window: &mut Window) -> bool {
        use sex_app::search as vs;
        while let Some((ticket, query)) = vs::asked() {
            let found = self.index.lookup(window, query.as_bytes(), vs::RESULTS_MAX);
            let records: Vec<vs::Record> = found.iter().filter_map(|(path, by_words)| {
                let name = core::str::from_utf8(base(path)).ok()?;
                let full = core::str::from_utf8(path).ok()?;
                let fits = full.len() <= sex_pdx::search::SUBTITLE_MAX;
                let mut cut = full.len().saturating_sub(sex_pdx::search::SUBTITLE_MAX - 3);
                while !full.is_char_boundary(cut) { cut += 1; }
                let sub = if fits { String::from(full) } else { format!("\u{2026}{}", &full[cut..]) };
                let mut t = name.len().min(sex_pdx::search::TITLE_MAX);
                while !name.is_char_boundary(t) { t -= 1; }
                let (kind, hint) = if *by_words { (vs::KIND_CONTENT, 300) } else { (vs::KIND_FILE, 0) };
                let r = vs::Record::new(kind, vs::ACTION_OPEN, hint, search::key(path), &name[..t], &sub)?;
                Some(if fits { r.with(&[vs::ACTION_OPEN_WITH, vs::ACTION_REVEAL, vs::ACTION_COPY]) } else { r.with(&[vs::ACTION_OPEN_WITH, vs::ACTION_REVEAL]) })
            }).collect();
            let sent = vs::answer(ticket, &records);
            sex_pdx::serial_record!("[files.app.voyeur.answer] ticket={} query={} results={} first={} sent={}",
                ticket, query, records.len(), records.first().map_or("-", |r| r.title()), sent as u8);
        }
        let mut redraw = false;
        while let Some((action, key)) = vs::activated() {
            let Some(path) = self.index.path_for(key) else { sex_pdx::serial_record!("[files.app.voyeur] action={} path=- ok=0", action); continue };
            let name = base(&path).to_vec();
            redraw = true;
            sex_pdx::serial_record!("[files.app.voyeur] action={} path={} ok=1", action, shown(&path));
            match action {
                vs::ACTION_OPEN => self.open_file(&name, &path),
                vs::ACTION_REVEAL | vs::ACTION_OPEN_WITH if self.job.is_some() => self.status = "Busy with the job below: wait for it, or Esc stops it.".into(),
                vs::ACTION_REVEAL | vs::ACTION_OPEN_WITH => {
                    self.search = None;
                    self.go(window, parent(&path));
                    self.select(&name);
                    if action == vs::ACTION_OPEN_WITH { self.actions_open(window); }
                }
                _ => {}
            }
        }
        redraw
    }
    fn search_step(&mut self, window: &mut Window) -> bool {
        let Some(sr) = self.search.as_mut() else { return false };
        if let search::Step::Running = sr.step(window, &mut self.index) {
            let now = sex_pdx::get_ticks();
            if now.saturating_sub(self.last_drawn) >= 150 { self.last_drawn = now; return true; }
            return false;
        }
        let sr = self.search.take().unwrap();
        self.show_found(&sr, true);
        true
    }
    /// Search results in place of the folder's listing: names relative to it.
    fn show_found(&mut self, sr: &Search, finished: bool) {
        sex_pdx::serial_record!("[files.app.search] base={} query={} results={} looked={} read={} ms={} finished={}",
            shown(&sr.base), shown(&sr.query), sr.results.len(), sr.looked, sr.read, sr.elapsed_ms(), finished as u8);
        for f in sr.results.iter().take(20) { sex_pdx::serial_record!("[files.app.found] path={} by={}", shown(&f.path), if f.by_words { "words" } else { "name" }); }
        self.entries = sr.results.iter().map(|f| Entry { name: f.path.clone(), dir: f.dir, size: f.size }).collect();
        self.cwd = sr.base.clone();
        self.origins.clear();
        self.cursor = 0; self.top = 0;
        self.found = Some(sr.query.clone());
        self.status = if finished { format!("{} found in {} files ({} ms).", sr.results.len(), sr.looked, sr.elapsed_ms()) } else { "Stopped searching: what was found so far is below.".into() };
    }
    fn stop_job(&mut self, window: &mut Window) {
        let Some(job) = self.job.as_mut() else { return };
        if job.finishing() { self.status = job.cancel(window); return; }
        self.status = job.cancel(window);
        sex_pdx::serial_record!("[files.app.job.cancelled] kind={:?} what={} pct={}", job.kind, shown(&job.what), job.percent());
        self.job = None;
        self.job_undo = None;
        if let Some(history) = self.history_job.take() { self.restore_history(history); }
        self.load(window);
    }
    fn remove_all(&mut self, window: &mut Window, path: &[u8]) -> Result<(), u8> {
        if files::stat(window, &at(path))?.dir {
            for e in files::list(window, &at(path))? { self.remove_all(window, &join(path, &e.name))?; }
        }
        files::remove(window, &at(path))
    }
    fn restore_history(&mut self, history: HistoryJob) {
        match history {
            HistoryJob::Undo(step) => self.undo.push(step),
            HistoryJob::Redo(step) => self.redo.push(step),
        }
    }
    /// Rename where the disk allows it, else start a background move. The caller
    /// keeps the history entry pending until that move reports completion.
    fn move_path(&mut self, window: &mut Window, from: &[u8], to: &[u8]) -> Result<bool, u8> {
        match files::stat(window, &at(to)) {
            Ok(_) => return Err(w::EXISTS),
            Err(w::NOT_FOUND) => {},
            Err(e) => return Err(e),
        }
        match files::rename(window, &at(from), &at(to)) {
            Err(w::OTHER_DISK) => {
                let job = Job::move_across(window, from, to)?;
                self.start(window, Ok(job), None, None);
                Ok(true)
            }
            Ok(()) => Ok(false),
            Err(e) => Err(e),
        }
    }
    /// A name not yet taken in `dir`: `name`, `name (copy)`, `name (copy 2)`...
    fn free_name(&mut self, window: &mut Window, dir: &[u8], name: &[u8], tag: &str) -> Vec<u8> {
        let try_name = join(dir, name);
        if files::stat(window, &at(&try_name)).is_err() { return try_name; }
        for k in 1..1000 {
            let n = loom_core::selection::collision_name(name, tag, k);
            let p = join(dir, &n);
            if files::stat(window, &at(&p)).is_err() { return p; }
        }
        try_name
    }

    fn paste(&mut self, window: &mut Window) {
        if let Some((paths, cut)) = self.clip_batch.clone() {
            if cut { self.paste_cut_batch(window, &paths); }
            else { self.paste_copy_batch(window, &paths); }
            return;
        }
        let Some((from, cut)) = self.clip.clone() else { self.status = "Nothing to paste: Ctrl+C or Ctrl+X a file first.".into(); return };
        if self.cwd.starts_with(&from) && (self.cwd.len() == from.len() || self.cwd[from.len()] == b'/') { self.status = "A folder can't go inside itself.".into(); return; }
        let to = self.free_name(window, &self.cwd.clone(), base(&from), "copy");
        if !cut {
            let job = Job::copy(window, &from, &to);
            self.start(window, job, Some(Undo::Copied { to: to.clone() }), Some(base(&to).to_vec()));
            return;
        }
        let result = match files::rename(window, &at(&from), &at(&to)) {
            Err(w::OTHER_DISK) => {
                let job = Job::move_across(window, &from, &to);
                self.start(window, job, Some(Undo::Moved { from: from.clone(), to: to.clone() }), Some(base(&to).to_vec()));
                self.clip = None;
                return;
            }
            other => other,
        };
        match result {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.pasted] from={} to={} cut={}", shown(&from), shown(&to), cut as u8);
                if cut { self.meta_moved(window, &from, &to); }
                self.status = format!("{} {}.", if cut { "Moved" } else { "Copied" }, shown(base(&to)));
                self.record_undo(if cut { Undo::MovedSame { from: from.clone(), to: to.clone() } } else { Undo::Copied { to: to.clone() } });
                if cut { self.clip = None; }
                let _ = files::sync(window);
                self.load(window);
                self.select(base(&to));
            }
            Err(e) => self.status = format!("Couldn't paste: {}.", shown(files::message(e))),
        }
    }

    fn paste_copy_batch(&mut self, window: &mut Window, paths: &[Vec<u8>]) {
        if paths.is_empty() || paths.len() > loom_core::selection::MAX_BATCH_COPY {
            self.status = "Select between 2 and 64 text-list items to copy together.".into(); return;
        }
        if paths.iter().any(|p| self.cwd == *p || (self.cwd.starts_with(p) && self.cwd.get(p.len()) == Some(&b'/'))) {
            self.status = "A folder can't be copied inside itself.".into(); return;
        }
        if paths.iter().any(|p| files::stat(window, &at(p)).is_err()) {
            self.status = "A selected item is no longer there; select the range again.".into(); return;
        }
        let cwd = self.cwd.clone();
        let names: Vec<Vec<u8>> = paths.iter().map(|p| base(p).to_vec()).collect();
        let Some(chosen) = loom_core::selection::copy_names(&names, |name| files::stat(window, &at(&join(&cwd, name))).is_ok()) else {
            self.status = "No free destination name; no items copied.".into(); return;
        };
        let places: Vec<(Vec<u8>, Vec<u8>)> = paths.iter().cloned().zip(chosen.iter().map(|name| join(&cwd, name))).collect();
        let destinations = places.clone();
        let job = Job::copy_many(window, &places);
        self.start(window, job, Some(Undo::CopiedBatch(destinations)), None);
    }

    fn paste_cut_batch(&mut self, window: &mut Window, paths: &[Vec<u8>]) {
        if !loom_core::selection::can_move_group_into(paths, &self.cwd) {
            self.status = "Choose another folder outside the selected items before pasting.".into(); return;
        }
        if paths.iter().any(|p| disk_root(p) != disk_root(&self.cwd)) {
            self.status = "Batch cut stays on one disk; move items from another disk one at a time.".into(); return;
        }
        // Validate every source before changing any path. The later rename loop
        // rolls back earlier moves if a disk or permission changes midway.
        if paths.iter().any(|p| files::stat(window, &at(p)).is_err()) {
            self.status = "A selected item is no longer there; select the range again.".into(); return;
        }
        let mut moved: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(paths.len());
        for from in paths {
            let to = self.free_name(window, &self.cwd.clone(), base(from), "copy");
            if files::stat(window, &at(&to)).is_ok() {
                let mut rollback_ok = true;
                for (original, placed) in moved.iter().rev() {
                    rollback_ok &= files::rename(window, &at(placed), &at(original)).is_ok();
                }
                self.status = if rollback_ok { "No free destination name; no items moved.".into() }
                    else { "No free destination name; some items need moving back.".into() };
                self.load(window);
                return;
            }
            if let Err(e) = files::rename(window, &at(from), &at(&to)) {
                let mut rollback_ok = true;
                for (original, placed) in moved.iter().rev() {
                    rollback_ok &= files::rename(window, &at(placed), &at(original)).is_ok();
                }
                self.status = format!("Batch move failed: {}.{}", shown(files::message(e)),
                    if rollback_ok { " No items moved." } else { " Some items need moving back." });
                self.load(window);
                return;
            }
            moved.push((from.clone(), to));
        }
        for (from, to) in &moved { self.meta_moved(window, from, to); }
        sex_pdx::serial_record!("[files.app.pasted.batch] count={} cut=1", moved.len());
        self.status = format!("Moved {} items. Ctrl+Z moves them back together.", moved.len());
        self.record_undo(Undo::MovedBatch(moved));
        self.clip_batch = None;
        let _ = files::sync(window);
        self.load(window);
    }

    fn undo_move_batch(&mut self, window: &mut Window, items: &[(Vec<u8>, Vec<u8>)]) -> Result<(), u8> {
        let mut restored: Vec<(&Vec<u8>, &Vec<u8>)> = Vec::new();
        for (from, to) in items.iter().rev() {
            if let Err(e) = files::rename(window, &at(to), &at(from)) {
                for (original, placed) in restored.iter().rev() {
                    let _ = files::rename(window, &at(original), &at(placed));
                }
                return Err(e);
            }
            restored.push((from, to));
        }
        Ok(())
    }
    fn undo_copy_batch(&mut self, window: &mut Window, items: &[(Vec<u8>, Vec<u8>)]) -> Result<(), u8> {
        // A retry after a partial undo skips copies already removed.
        for (_, to) in items.iter().rev() {
            if files::stat(window, &at(to)).is_ok() { self.remove_all(window, to)?; }
        }
        Ok(())
    }
    fn trash(&mut self, window: &mut Window) {
        let chosen = self.selected_paths();
        if !chosen.is_empty() { self.trash_many(window, &chosen); }
    }
    /// Reserve both the trash entry and its origin note; an orphaned note must
    /// never be overwritten by a later item with the same name.
    fn free_trash_name(&mut self, window: &mut Window, trash: &[u8], info: &[u8], name: &[u8]) -> (Vec<u8>, Vec<u8>) {
        for k in 0..1000 {
            let candidate = if k == 0 { name.to_vec() } else { loom_core::selection::collision_name(name, "trashed", k) };
            let to = join(trash, &candidate);
            let note = join(info, &candidate);
            if files::stat(window, &at(&to)).is_err() && files::stat(window, &at(&note)).is_err() { return (to, note); }
        }
        (Vec::new(), Vec::new())
    }
    /// Commit origin notes before moving items. A power cut after a move can
    /// then always find the source path needed for restore.
    fn trash_many(&mut self, window: &mut Window, paths: &[Vec<u8>]) {
        if base(&self.cwd) == TRASH { self.status = "Use Shift+Delete for permanent deletion from trash.".into(); return; }
        let root = disk_root(&self.cwd);
        let (trash, info) = (join(&root, TRASH), join(&root, INFO));
        if let Err(e) = files::mkdir(window, &at(&trash)).or_else(|e| if e == w::EXISTS { Ok(()) } else { Err(e) }) {
            self.status = format!("Couldn't open trash: {}.", shown(files::message(e))); return;
        }
        if let Err(e) = files::mkdir(window, &at(&info)).or_else(|e| if e == w::EXISTS { Ok(()) } else { Err(e) }) {
            self.status = format!("Couldn't open trash notes: {}.", shown(files::message(e))); return;
        }
        // The last field is the id of a note this transaction actually created.
        // A failed create must never make rollback delete another writer's note.
        let mut prepared: Vec<(Vec<u8>, Vec<u8>, Vec<u8>, Option<u64>)> = Vec::new();
        let mut moved = 0usize;
        let mut attempting = None;
        let result = (|| -> Result<(), u8> {
            for from in paths {
                let (to, note) = self.free_trash_name(window, &trash, &info, base(from));
                if to.is_empty() { return Err(w::EXISTS); }
                // Creation, full contents and commit are one Files request.
                // Removable volumes return no stable ID, so their rollback
                // leaves a stranded note rather than risking a foreign file.
                let note_id = files::create_with_data(window, &at(&note), from)?;
                prepared.push((from.clone(), to.clone(), note, (note_id != 0).then_some(note_id)));
                files::sync(window)?;
                sex_pdx::serial_record!("[files.app.trash.note.synced] from={}", shown(from));
                attempting = Some(moved);
                // SexFS's ordinary file rename can replace a name claimed
                // since free_trash_name checked it.
                if !from.starts_with(b"/media/") && files::stat(window, &at(from)).map(|s| !s.dir).unwrap_or(false) {
                    let id = files::file_id(window, &at(from))?;
                    files::move_if_id(window, &at(from), &at(&to), id)?;
                } else {
                    files::rename(window, &at(from), &at(&to))?;
                }
                moved += 1;
                attempting = None;
            }
            files::sync(window)
        })();
        if let Err(e) = result {
            let mut rollback_ok = true;
            for (i, (from, to, note, note_id)) in prepared.iter().enumerate().rev() {
                // A failed move request can have committed before its reply
                // failed. Keep its note and report uncertainty; a blind retry
                // could overwrite a name claimed by somebody else.
                let restored = if attempting == Some(i) { false }
                    else { i >= moved || files::rename(window, &at(to), &at(from)).is_ok() };
                rollback_ok &= restored;
                // Keep the origin note when a move could not be rolled back or
                // another writer now owns its name.
                if restored {
                    rollback_ok &= note_id.is_some_and(|id| files::remove_if_id(window, &at(note), id).is_ok());
                }
            }
            rollback_ok &= files::sync(window).is_ok();
            self.status = format!("Batch trash failed: {}.{}", shown(files::message(e)),
                if rollback_ok { " No items moved." } else { " Check the folder and trash; storage state is unconfirmed." });
            self.load(window);
            return;
        }
        for (from, to, _, _) in &prepared { self.meta_moved(window, from, to); }
        if prepared.len() == 1 {
            let (from, to, note, _) = prepared.pop().unwrap();
            sex_pdx::serial_record!("[files.app.trashed] from={} to={}", shown(&from), shown(&to));
            self.status = format!("Moved {} to the trash. Ctrl+Z puts it back.", shown(base(&from)));
            self.record_undo(Undo::Trashed { from, to, info: note });
        } else {
            sex_pdx::serial_record!("[files.app.trashed.batch] count={}", prepared.len());
            self.status = format!("Moved {} items to trash. Ctrl+Z restores them together.", prepared.len());
            self.record_undo(Undo::TrashedBatch(prepared.into_iter().map(|(from, to, note, _)| (from, to, note)).collect()));
        }
        self.load(window);
    }

    fn undo_trash_batch(&mut self, window: &mut Window, items: &[(Vec<u8>, Vec<u8>, Vec<u8>)]) -> Result<(), u8> {
        let mut restored: Vec<(&Vec<u8>, &Vec<u8>)> = Vec::new();
        for (from, to, _) in items.iter().rev() {
            if let Err(e) = files::rename(window, &at(to), &at(from)) {
                for (back, original) in restored.iter().rev() { let _ = files::rename(window, &at(back), &at(original)); }
                return Err(e);
            }
            restored.push((from, to));
        }
        for (_, _, note) in items { let _ = files::remove(window, &at(note)); }
        Ok(())
    }
    /// Shift+Del: removes the selected item for good, after a second Shift+Del.
    fn delete_forever(&mut self, window: &mut Window) {
        if self.selected_paths().len() > 1 {
            self.status = "Permanent deletion currently takes one item. Move the selection to trash with Delete.".into();
            return;
        }
        let Some(path) = self.selected() else { return };
        let ask = Confirm::DeleteForever(path.clone());
        if self.confirm.as_ref() != Some(&ask) {
            self.status = format!("Delete {} for good? It won't go to the trash. Shift+Del again to delete, any other key keeps it.", shown(base(&path)));
            self.confirm = Some(ask);
            return;
        }
        self.confirm = None;
        self.redo.clear();
        self.meta_removed(window, &path);
        let mut paths = vec![path.clone()];
        // A trash entry's note goes with it.
        if base(&self.cwd) == TRASH { paths.push(join(&join(&self.cwd, b".info"), base(&path))); }
        let mut kept = Vec::new();
        for p in paths { if files::stat(window, &at(&p)).is_ok() { kept.push(p); } }
        let job = Job::delete(window, base(&path), &kept);
        self.start(window, job, None, None);
    }
    /// Ctrl+E in a trash folder: deletes everything in it for good.
    fn empty_trash(&mut self, window: &mut Window) {
        if base(&self.cwd) != TRASH { self.status = "Ctrl+T opens the trash; Ctrl+E there empties it.".into(); return; }
        let ask = Confirm::EmptyTrash(self.cwd.clone());
        if self.confirm.as_ref() != Some(&ask) {
            let n = self.entries.len();
            self.status = format!("Delete all {} items in the trash for good? Ctrl+E again to empty it, any other key keeps them.", n);
            self.confirm = Some(ask);
            return;
        }
        self.confirm = None;
        self.redo.clear();
        let list = files::list(window, &at(&self.cwd)).unwrap_or_default();
        let paths: Vec<Vec<u8>> = list.iter().map(|e| join(&self.cwd, &e.name)).collect();
        let trash = self.cwd.clone();
        self.meta_removed(window, &trash);
        sex_pdx::serial_record!("[files.app.emptying] dir={} count={}", shown(&self.cwd), self.entries.len());
        let job = Job::delete(window, b"the trash", &paths);
        self.start(window, job, None, None);
    }
    /// Ctrl+I: size, and the SHA-256 checksum read through in the background.
    fn info(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { return };
        match files::stat(window, &at(&path)) {
            Ok(st) if st.dir => {
                let mut total = 0u64; let mut count = 0u64;
                self.measure(window, &path, &mut total, &mut count);
                self.status = format!("{}/: {} files, {}.", shown(base(&path)), count, human(total));
            }
            Ok(_) => { let job = Job::checksum(window, &path); self.start(window, job, None, None); }
            Err(e) => self.status = format!("Can't look at it: {}.", shown(files::message(e))),
        }
    }
    fn measure(&mut self, window: &mut Window, dir: &[u8], total: &mut u64, count: &mut u64) {
        for e in files::list(window, &at(dir)).unwrap_or_default() {
            if e.dir { self.measure(window, &join(dir, &e.name), total, count); } else { *total += e.size; *count += 1; }
        }
    }
    /// Puts the selected item of a trash folder back where it was.
    fn restore(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { return };
        if base(&self.cwd) != TRASH { self.status = "Ctrl+T opens the trash; Ctrl+R there puts the item back.".into(); return; }
        let note = join(&join(&self.cwd, b".info"), base(&path));
        let mut buf = [0u8; w::PATH_BYTES];
        let Ok(n) = files::read(window, &at(&note), &mut buf) else { self.status = "Where this came from isn't known.".into(); return };
        let back = buf[..n].to_vec();
        match files::rename(window, &at(&path), &at(&back)) {
            Ok(()) => {
                let _ = files::remove(window, &at(&note));
                self.meta_moved(window, &path, &back);
                sex_pdx::serial_record!("[files.app.restored] from={} to={}", shown(&path), shown(&back));
                self.status = format!("Restored {}.", shown(&back));
                self.record_undo(Undo::Moved { from: path, to: back });
                let _ = files::sync(window);
                self.load(window);
            }
            Err(e) => self.status = format!("Couldn't restore: {}.", shown(files::message(e))),
        }
    }
    fn record_undo(&mut self, step: Undo) {
        self.redo.clear();
        if self.undo.len() == 64 { self.undo.remove(0); }
        self.undo.push(step);
    }
    fn undo(&mut self, window: &mut Window) {
        let Some(step) = self.undo.pop() else { self.status = "Nothing to undo.".into(); return };
        let result = match &step {
            Undo::Moved { from, to } => match self.move_path(window, to, from) {
                Ok(true) => { self.history_job = Some(HistoryJob::Undo(step)); return; }
                Ok(false) => Ok(()),
                Err(e) => Err(e),
            },
            Undo::MovedSame { from, to } | Undo::RenamedOne { from, to } => match files::stat(window, &at(from)) {
                Ok(_) => Err(w::EXISTS),
                Err(w::NOT_FOUND) => files::rename(window, &at(to), &at(from)),
                Err(e) => Err(e),
            },
            Undo::Trashed { from, to, info } => match files::stat(window, &at(from)) {
                Ok(_) => Err(w::EXISTS),
                Err(w::NOT_FOUND) => files::rename(window, &at(to), &at(from)).map(|_| { let _ = files::remove(window, &at(info)); }),
                Err(e) => Err(e),
            },
            Undo::TrashedBatch(items) => self.undo_trash_batch(window, items),
            Undo::MovedBatch(items) => self.undo_move_batch(window, items),
            Undo::CopiedBatch(items) => self.undo_copy_batch(window, items),
            Undo::Copied { to } => self.remove_all(window, to),
            Undo::Made { path } => files::remove(window, &at(path)),
            Undo::Renamed { dir, moves } => { let (dir, moves) = (dir.clone(), moves.clone()); self.rename_undo(window, &dir, &moves, false) }
        };
        match result {
            Ok(()) => {
                self.meta_undone(window, &step);
                sex_pdx::serial_record!("[files.app.undone] left={}", self.undo.len());
                self.status = "Undone.".into();
                if matches!(&step, Undo::Renamed { .. } | Undo::RenamedOne { .. } | Undo::Moved { .. } | Undo::MovedSame { .. } | Undo::Trashed { .. } | Undo::Made { .. }) {
                    if self.redo.len() == 64 { self.redo.remove(0); }
                    self.redo.push(step);
                } else { self.redo.clear(); }
                let _ = files::sync(window);
                self.load(window);
            }
            Err(e) => {
                self.status = if e == rename::ROLLBACK_UNCONFIRMED {
                    "Couldn't undo: rollback was not confirmed. Inspect the folder before retrying.".into()
                } else {
                    format!("Couldn't undo: {}.{}", shown(files::message(e)),
                        if matches!(&step, Undo::Renamed { .. }) { " Check the folder before retrying." } else { "" })
                };
                self.undo.push(step);
            }
        }
    }
    /// Ctrl+Y: replay the last undone step. A failed replay keeps the step for
    /// retry and does not alter undo history.
    fn redo(&mut self, window: &mut Window) {
        let Some(step) = self.redo.pop() else {
            self.status = "Nothing to redo.".into();
            sex_pdx::serial_record!("[files.app.redo.empty]");
            return;
        };
        let result = match &step {
            Undo::Moved { from, to } => match self.move_path(window, from, to) {
                Ok(true) => { self.history_job = Some(HistoryJob::Redo(step)); return; }
                Ok(false) => Ok(()),
                Err(e) => Err(e),
            },
            // Batch renames replay only with stable object IDs (see rename_undo).
            Undo::Renamed { dir, moves } => { let (dir, forward) = (dir.clone(), inverse_moves(moves)); self.rename_undo(window, &dir, &forward, true) }
            Undo::RenamedOne { from, to } | Undo::MovedSame { from, to } => match files::stat(window, &at(to)) {
                Ok(_) => Err(w::EXISTS),
                Err(w::NOT_FOUND) => files::rename(window, &at(from), &at(to)),
                Err(e) => Err(e),
            },
            Undo::Made { path } => files::mkdir(window, &at(path)),
            Undo::Trashed { from, to, info } => match files::stat(window, &at(to)) {
                Ok(_) => Err(w::EXISTS),
                Err(w::NOT_FOUND) => {
                    // Commit the origin note first, as trash_many does; creation
                    // fails rather than replacing a note another item now owns.
                    match files::create_with_data(window, &at(info), from) {
                        Err(e) => Err(e),
                        Ok(note_id) => match files::sync(window).and_then(|_| {
                            // As in trash_many: SexFS's plain file rename could
                            // replace a name claimed since the check above.
                            if !from.starts_with(b"/media/") && files::stat(window, &at(from)).map(|s| !s.dir).unwrap_or(false) {
                                let id = files::file_id(window, &at(from))?;
                                files::move_if_id(window, &at(from), &at(to), id)
                            } else {
                                files::rename(window, &at(from), &at(to))
                            }
                        }) {
                            Ok(()) => Ok(()),
                            Err(e) => {
                                if note_id != 0 { let _ = files::remove_if_id(window, &at(info), note_id); }
                                Err(e)
                            }
                        },
                    }
                }
                Err(e) => Err(e),
            },
            _ => Err(w::DENIED),
        };
        match result {
            Ok(()) => {
                self.meta_redone(window, &step);
                match &step {
                    Undo::Renamed { moves, .. } => sex_pdx::serial_record!("[files.app.redone] kind=rename count={} left={}", moves.len(), self.redo.len()),
                    _ => sex_pdx::serial_record!("[files.app.redone] left={}", self.redo.len()),
                }
                self.status = "Redone. Ctrl+Z takes it back.".into();
                if self.undo.len() == 64 { self.undo.remove(0); }
                self.undo.push(step);
                let _ = files::sync(window);
                self.load(window);
            }
            Err(e) if e == sex_pdx::fs::UNSUPPORTED => {
                self.status = "This disk has no stable file IDs, so Loom cannot safely redo its batch rename.".into();
            }
            Err(e) => {
                self.status = if e == rename::ROLLBACK_UNCONFIRMED {
                    "Couldn't redo: rollback was not confirmed. Inspect the folder before retrying.".into()
                } else {
                    format!("Couldn't redo: {}. Check the folder before retrying.", shown(files::message(e)))
                };
                self.redo.push(step);
            }
        }
    }
    /// Enter on a file: it opens in its program, which is given that file alone.
    fn open_file(&mut self, name: &[u8], path: &[u8]) {
        let Some(program) = opener(name, path) else { self.status = format!("No program opens {} yet.", shown(name)); return };
        let mut args = [0u8; w::ARGS_MAX];
        let Some(n) = sex_pdx::launch::open_args(path, &mut args) else {
            self.status = format!("{} can't be opened from here yet: its path has a space in it.", shown(name));
            return;
        };
        let cookie = OPENS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        match sex_app::launch::run(&program, &args[..n], false, cookie) {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.open] program={} path={}", shown(&program), shown(path));
                self.status = format!("Opening {} in {}.", shown(name), shown(&program));
                self.opening = Some((cookie, program));
            }
            Err(e) => {
                sex_pdx::serial_record!("[files.app.open.failed] program={} path={} error={:?}", shown(&program), shown(path), e);
                self.status = format!("Couldn't open {} in {}.", shown(name), shown(&program));
            }
        }
    }
    /// A password typed for home.
    fn password(&mut self, window: &mut Window, secret: Secret, text: &[u8]) {
        use sex_pdx::home_lock as h;
        match secret {
            Secret::Unlock => match files::home_unlock(window, text) {
                Ok(()) => {
                    sex_pdx::serial_record!("[files.app.home.unlocked]");
                    self.status = "Home unlocked.".into();
                    self.go(window, HOME.to_vec());
                }
                Err(e) => {
                    sex_pdx::serial_record!("[files.app.home.refused] status={}", e);
                    self.status = format!("{}: home stays locked.", shown(h::message(e)));
                    if e == h::WRONG_PASSWORD { self.input = Some((Input::Password(Secret::Unlock), Vec::new())); }
                }
            },
            Secret::New if text.len() < h::PASSWORD_MIN => { self.status = "At least 8 characters.".into(); self.input = Some((Input::Password(Secret::New), Vec::new())); }
            Secret::New => self.input = Some((Input::Password(Secret::Repeat(text.to_vec())), Vec::new())),
            Secret::Current => self.input = Some((Input::Password(Secret::ChangeTo(text.to_vec())), Vec::new())),
            Secret::ChangeTo(mut old) if text.len() < h::PASSWORD_MIN => { old.fill(0); self.status = "At least 8 characters: nothing changed.".into(); }
            Secret::ChangeTo(old) => self.input = Some((Input::Password(Secret::ChangeRepeat(old, text.to_vec())), Vec::new())),
            Secret::ChangeRepeat(mut old, mut new) => {
                let same = new == text;
                new.fill(0);
                if !same { old.fill(0); self.status = "The two new passwords differ: nothing changed.".into(); return; }
                self.status = "Changing the password...".into();
                let r = files::home_change(window, &old, text);
                old.fill(0);
                match r {
                    Ok(()) => { sex_pdx::serial_record!("[files.app.home.changed]"); self.status = "Home's password is changed.".into(); }
                    Err(e) => { sex_pdx::serial_record!("[files.app.home.change.refused] status={}", e); self.status = format!("Couldn't change the password: {}.", shown(h::message(e))); }
                }
            }
            Secret::Repeat(mut first) => {
                if first != text { self.status = "The two passwords differ: nothing changed.".into(); first.fill(0); return; }
                first.fill(0);
                self.status = "Encrypting home...".into();
                match files::home_setup(window, HOME_MIB, text) {
                    Ok(()) => { sex_pdx::serial_record!("[files.app.home.setup] mib={}", HOME_MIB); self.status = "Home is encrypted and unlocked. Ctrl+K locks it.".into(); self.go(window, HOME.to_vec()); }
                    Err(e) => self.status = format!("Couldn't encrypt home: {}.", shown(h::message(e))),
                }
            }
        }
    }
    fn finish_input(&mut self, window: &mut Window) {
        let Some((kind, mut text)) = self.input.take() else { return };
        if let Input::Password(secret) = kind { self.password(window, secret, &text); text.fill(0); return; }
        if let Input::Search = kind {
            if text.iter().all(|b| *b == b' ') { return; }
            let base = self.cwd.clone();
            sex_pdx::serial_record!("[files.app.search.start] base={} query={}", shown(&base), shown(&text));
            self.search = Some(Search::new(window, &mut self.index, &base, &text));
            self.last_drawn = 0;
            return;
        }
        if text.is_empty() || text.contains(&b'/') || text == b"." || text == b".." { self.status = "That name can't be used.".into(); return; }
        match kind {
            Input::Rename(from) => {
                let to = join(&parent(&from), &text);
                match files::rename(window, &at(&from), &at(&to)) {
                    Ok(()) => {
                        self.meta_moved(window, &from, &to);
                        sex_pdx::serial_record!("[files.app.renamed.one] from={} to={}", shown(&from), shown(&to));
                        self.status = format!("Renamed to {}.", shown(&text));
                        self.record_undo(Undo::RenamedOne { from, to });
                        self.load(window);
                        self.select(&text);
                    }
                    Err(e) => self.status = format!("Couldn't rename: {}.", shown(files::message(e))),
                }
            }
            Input::NewFolder => {
                let path = join(&self.cwd, &text);
                match files::mkdir(window, &at(&path)) {
                    Ok(()) => {
                        sex_pdx::serial_record!("[files.app.made] path={}", shown(&path));
                        self.status = format!("Made {}.", shown(&text));
                        self.record_undo(Undo::Made { path });
                        self.load(window);
                        self.select(&text);
                    }
                    Err(e) => self.status = format!("Couldn't make the folder: {}.", shown(files::message(e))),
                }
            }
            Input::Search | Input::Password(_) => {}
            Input::SaveAs => { let path = join(&self.cwd, &text); self.answer(window, Some(path)); }
        }
        let _ = files::sync(window);
    }
    /// Answers the waiting question with one path, or no.
    fn answer(&mut self, window: &mut Window, path: Option<Vec<u8>>) {
        let Some(q) = self.question.take() else { return };
        let token = path.as_ref().map(|p| at(p));
        match files::answer(window, &q, token.as_deref()) {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.answered] program={} path={}", shown(&q.program), path.as_deref().map(shown).unwrap_or("-"));
                self.status = match &path { Some(p) => format!("Gave {} to {}.", shown(p), shown(&q.program)), None => format!("Told {} no.", shown(&q.program)) };
            }
            Err(e) => self.status = format!("Couldn't answer: {}.", shown(files::message(e))),
        }
    }
    fn poll_questions(&mut self, window: &mut Window) -> bool {
        let disk = self.poll_disk_asks(window);
        if !self.portal || self.question.is_some() { return disk; }
        match files::questions(window) {
            Ok(mut qs) if !qs.is_empty() => {
                let q = qs.remove(0);
                sex_pdx::serial_record!("[files.app.question] program={} mode={:?}", shown(&q.program), q.mode);
                self.question = Some(q);
                true
            }
            Ok(_) => disk,
            Err(_) => { self.portal = false; disk }
        }
    }
    /// A program asking for a whole disk (`sex_pdx::disks`): shown until answered.
    fn poll_disk_asks(&mut self, window: &mut Window) -> bool {
        if !self.portal || self.disk_ask.is_some() { return false; }
        match files::disks::questions(window) {
            Ok(mut qs) if !qs.is_empty() => {
                let q = qs.remove(0);
                sex_pdx::serial_record!("[files.app.disk.question] program={} disk={}", shown(&q.program), shown(&q.disk));
                self.disk_ask = Some(q);
                true
            }
            _ => false,
        }
    }
    /// Y or N to a disk question; any other key goes on as usual.
    fn disk_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> bool {
        if self.input.is_some() || key.control || !matches!(key.code, KEY_Y | KEY_N) { return false; }
        let Some(q) = self.disk_ask.take() else { return false };
        let yes = key.code == KEY_Y;
        match files::disks::answer(window, &q, yes) {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.disk.answered] program={} disk={} yes={}", shown(&q.program), shown(&q.disk), yes as u8);
                self.status = if yes { format!("{} may use {} until it closes.", shown(&q.program), shown(&q.disk)) } else { format!("Told {} no.", shown(&q.program)) };
            }
            Err(e) => self.status = format!("Couldn't answer: {}.", shown(files::message(e))),
        }
        true
    }

    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        if self.disk_key(window, key) { return; }
        let before = self.confirm.is_some();
        self.key_inner(window, key);
        let again = matches!(self.confirm, Some(_)) && (key.code == KEY_E && key.control || matches!(sex_app::nav(key), Some(Nav::Delete)) && key.shift);
        if before && !again && self.confirm.is_some() { self.confirm = None; self.status = "Kept.".into(); }
    }
    fn key_inner(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        if self.picture_preview.is_some() {
            if matches!(key.code, ESC | F3) {
                self.picture_preview = None;
                sex_pdx::serial_record!("[files.app.picture.preview.closed]");
            }
            return;
        }
        if self.preview.is_some() {
            if matches!(key.code, ESC | F3) {
                self.preview = None;
                sex_pdx::serial_record!("[files.app.preview.closed]");
                return;
            }
            let preview = &mut self.preview.as_mut().unwrap().1;
            match sex_app::nav(key) {
                Some(Nav::Up) => preview.up(),
                Some(Nav::Down) => preview.down(),
                Some(Nav::Home) => preview.first(),
                Some(Nav::End) => preview.last(),
                _ => {}
            }
            return;
        }
        if let Some((_, text)) = self.input.as_mut() {
            match key.code {
                ENTER => self.finish_input(window),
                ESC => { self.input = None; }
                BACKSPACE => { while let Some(b) = text.pop() { if b & 0xC0 != 0x80 { break; } } }
                _ => if let Some(c) = sex_app::text(key) { if text.len() + c.len_utf8() <= 255 { text.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()); } },
            }
            return;
        }
        if self.quick_places.is_some() { self.quick_places_key(window, key); return; }
        if self.loom_key(window, key) { return; }
        // Shift+Del and Ctrl+E ask twice; anything else between keeps things.
        let confirming = self.confirm.is_some();
        if let Some(sr) = self.search.take() {
            if key.code == ESC { self.show_found(&sr, false); } else { self.search = Some(sr); self.status = "Searching: Esc stops.".into(); }
            return;
        }
        if self.job.is_some() {
            if key.code == ESC { self.stop_job(window); return; }
            let changes = key.control && matches!(key.code, KEY_V | KEY_Z | KEY_Y | KEY_N | KEY_R | KEY_E | KEY_I)
                || matches!(sex_app::nav(key), Some(Nav::Delete)) || key.code == F2;
            if changes { self.status = "Busy with the job below: wait for it, or Esc stops it.".into(); return; }
        }
        self.status.clear();
        if key.control {
            if !self.entries.is_empty() {
                let cols = if self.thumbs.on {
                    if self.split && self.tabs.len() > 1 && split_grid_fits(window) {
                        ((window.size().0 / 2).saturating_sub(16) / thumbs::CELL_W).max(1)
                    } else { Thumbs::columns(window.size().0) }
                } else { 1 };
                let target = match sex_app::nav(key) {
                    Some(Nav::Left) if self.thumbs.on => Some(self.cursor.saturating_sub(1)),
                    Some(Nav::Right) if self.thumbs.on => Some((self.cursor + 1).min(self.entries.len() - 1)),
                    Some(Nav::Up) => Some(self.cursor.saturating_sub(cols)),
                    Some(Nav::Down) => Some((self.cursor + cols).min(self.entries.len() - 1)),
                    Some(Nav::Home) => Some(0),
                    Some(Nav::End) => Some(self.entries.len() - 1),
                    _ => None,
                };
                if let Some(target) = target {
                    if self.selection_anchor.is_some() {
                        self.selection_marks = self.selected_rows();
                        self.selection_anchor = None;
                    }
                    self.cursor = target;
                    return;
                }
            }
            match key.code {
                SPACE => self.toggle_focus(),
                KEY_B if key.shift => self.open_quick_places(window),
                0x2B if key.shift => {
                    if !self.tab_ready() {
                        self.status = "Finish this view before splitting.".into();
                    } else if self.tabs.len() < 2 {
                        self.status = "Open a second tab before splitting.".into();
                    } else {
                        self.split = !self.split;
                        let other = (self.active_tab + 1) % self.tabs.len();
                        sex_pdx::serial_record!("[loom.split] on={} active={} pair={} dir={} other={}", self.split as u8, self.active_tab + 1, other + 1, shown(&self.cwd), shown(&self.tabs[other].cwd));
                    }
                },
                KEY_T if key.shift => self.new_tab(window),
                KEY_W if key.shift => self.close_tab(window),
                0x49 => { let next = (self.active_tab + self.tabs.len() - 1) % self.tabs.len(); self.show_tab(window, next); }
                0x51 => { let next = (self.active_tab + 1) % self.tabs.len(); self.show_tab(window, next); }
                KEY_C | KEY_X if self.selected_paths().len() > 1 => {
                    let paths = self.selected_paths();
                    let cut = key.code == KEY_X;
                    sex_pdx::serial_record!("[files.app.{}.batch] count={}", if cut { "cut" } else { "copy" }, paths.len());
                    self.status = format!("{} {} items. Open a destination folder and press Ctrl+V.", if cut { "Cut" } else { "Copied" }, paths.len());
                    self.clip_batch = Some((paths, cut)); self.clip = None;
                }
                KEY_C | KEY_X => if let Some(p) = self.selected() {
                    let cut = key.code == KEY_X;
                    self.status = format!("{} {}: Ctrl+V pastes it.", if cut { "Cut" } else { "Copied" }, shown(base(&p)));
                    self.clip = Some((p, cut)); self.clip_batch = None;
                },
                KEY_V => self.paste(window),
                KEY_Z => self.undo(window),
                KEY_Y => self.redo(window),
                KEY_N => self.input = Some((Input::NewFolder, Vec::new())),
                KEY_T => { let t = join(&disk_root(&self.cwd), TRASH); self.go(window, t); }
                KEY_R => self.restore(window),
                // Ctrl+E: Empty in a trash, Eject on a stick or disk.
                KEY_E if base(&self.cwd) != TRASH && (self.cwd == w::MEDIA || disk_root(&self.cwd) != b"/") => self.eject(window),
                KEY_E => self.empty_trash(window),
                KEY_I => self.info(window),
                KEY_F => self.input = Some((Input::Search, Vec::new())),
                // Ctrl+Shift+K: a new password for an encrypted home.
                KEY_K if self.portal && key.shift => match files::home_state(window) {
                    Ok(sex_pdx::home_lock::STATE_NONE) => self.status = "Home isn't encrypted: Ctrl+K encrypts it.".into(),
                    Ok(_) => self.input = Some((Input::Password(Secret::Current), Vec::new())),
                    Err(e) => self.status = format!("Home: {}.", shown(sex_pdx::home_lock::message(e))),
                },
                // Ctrl+K: encrypt home, unlock it, or lock it again.
                KEY_K if self.portal => match files::home_state(window) {
                    Ok(sex_pdx::home_lock::STATE_NONE) => { self.input = Some((Input::Password(Secret::New), Vec::new())); self.status = format!("Home will be encrypted ({} MiB): only this password opens it, and it can't be recovered.", HOME_MIB); }
                    Ok(sex_pdx::home_lock::STATE_LOCKED) => self.input = Some((Input::Password(Secret::Unlock), Vec::new())),
                    Ok(_) => match files::home_lock(window) {
                        Ok(()) => { sex_pdx::serial_record!("[files.app.home.locked]"); self.status = "Home locked: its files can't be read until the password is given.".into(); if self.cwd.starts_with(HOME) { let r = self.root.clone(); self.go(window, r); } }
                        Err(e) => self.status = format!("Couldn't lock home: {}.", shown(sex_pdx::home_lock::message(e))),
                    },
                    Err(e) => self.status = format!("Home: {}.", shown(sex_pdx::home_lock::message(e))),
                },
                KEY_G => {
                    self.thumbs.on = !self.thumbs.on;
                    sex_pdx::serial_record!("[files.app.grid] on={} dir={}", self.thumbs.on as u8, shown(&self.cwd));
                    if self.thumbs.on { self.thumbs.folder(&self.cwd, &self.entries); }
                }
                KEY_H => { let r = self.root.clone(); self.go(window, r); }
                KEY_D => self.go(window, w::MEDIA.to_vec()),
                KEY_S => match self.question.as_ref().map(|q| q.mode) {
                    Some(Pick::Save) => self.input = Some((Input::SaveAs, self.question.as_ref().unwrap().suggestion.clone())),
                    Some(Pick::Folder) => { let c = self.cwd.clone(); self.answer(window, Some(c)); }
                    _ => {}
                },
                _ => {}
            }
            return;
        }
        // In the grid, arrows move across and down by whole rows.
        if self.thumbs.on && !self.entries.is_empty() && (!self.split || split_grid_fits(window)) {
            let cols = if self.split && self.tabs.len() > 1 {
                ((window.size().0 / 2).saturating_sub(16) / thumbs::CELL_W).max(1)
            } else { Thumbs::columns(window.size().0) };
            let last = self.entries.len() - 1;
            match sex_app::nav(key) {
                Some(Nav::Left) => { self.move_selection(self.cursor.saturating_sub(1), false); return; }
                Some(Nav::Right) => { self.move_selection((self.cursor + 1).min(last), false); return; }
                Some(Nav::Up) => { self.move_selection(self.cursor.saturating_sub(cols), false); return; }
                Some(Nav::Down) => { self.move_selection((self.cursor + cols).min(last), false); return; }
                _ => {}
            }
        }
        match (key.code, sex_app::nav(key)) {
            (_, Some(Nav::Up)) => self.move_selection(self.cursor.saturating_sub(1), key.shift),
            (_, Some(Nav::Down)) => self.move_selection(self.cursor + 1, key.shift),
            (_, Some(Nav::Home)) => self.move_selection(0, key.shift),
            (_, Some(Nav::End)) => self.move_selection(self.entries.len().saturating_sub(1), key.shift),
            (_, Some(Nav::Delete)) if key.shift => self.delete_forever(window),
            (_, Some(Nav::Delete)) => self.trash(window),
            (ENTER, _) => {
                let Some(e) = self.entries.get(self.cursor) else { return };
                let path = join(&self.cwd, &e.name);
                if self.found.is_some() && !e.dir {
                    // A search result: open the folder it is in, on it.
                    let name = base(&path).to_vec();
                    self.go(window, parent(&path));
                    self.select(&name);
                    sex_pdx::serial_record!("[files.app.found.opened] path={}", shown(&path));
                    return;
                }
                if e.dir { self.go(window, path); }
                else if matches!(self.question.as_ref().map(|q| q.mode), Some(Pick::Open)) { self.answer(window, Some(path)); }
                else { let name = e.name.clone(); self.open_file(&name, &path); }
            }
            (BACKSPACE, _) => {
                // Never above what this run was given.
                if self.cwd != self.root && self.cwd.len() > 1 {
                    let here = base(&self.cwd).to_vec();
                    let up = parent(&self.cwd);
                    self.go(window, up);
                    self.select(&here);
                }
            }
            (ESC, _) => if self.found.is_some() { self.load(window); } else if self.question.is_some() && !confirming { self.answer(window, None); },
            (F2, _) if self.selected_paths().len() > 1 => {
                self.status = "F2 renames one item. Ctrl+Shift+R previews batch rename.".into();
            }
            (F2, _) => if let Some(p) = self.selected() { let name = base(&p).to_vec(); self.input = Some((Input::Rename(p), name)); },
            (F3, _) if self.question.is_none() && self.job.is_none() => self.preview_open(window),
            (F5, _) => self.load(window),
            _ => {}
        }
    }
}

/// The program Enter opens a file with: the one Settings chose for its kind (Default
/// apps), else the system's for its type.
fn opener(name: &[u8], path: &[u8]) -> Option<Vec<u8>> {
    let chosen = sex_config::kind_of(name).and_then(|kind| {
        let k = sex_config::key(kind)?;
        let v = sex_app::settings::get(kind).unwrap_or(k.default);
        match k.kind { sex_config::Kind::Choice(names) => names.get(v as usize).copied(), _ => None }
    });
    chosen.map(|p| p.as_bytes().to_vec()).or_else(|| sex_pdx::launch::program_for(path).map(|p| p.to_vec()))
}
static OPENS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0x0F11_0000);

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Loom");
    let _ = window.want_keys(sex_pdx::app_key::DESKTOP_KEYS);
    // The folder the person gave: `@/` is everything.
    let mut args = [0u8; w::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let root = given.split(|b| *b == b' ').find_map(|t| t.strip_prefix(b"@")).filter(|p| p.first() == Some(&b'/')).map(|p| p.to_vec());
    let Some(root) = root else {
        window.show(b"Loom needs a folder to look after. Start it from Spindle with:  loom  (everything)  or  loom DIR\n\nF11 closes.");
        sex_pdx::serial_record!("[files.app.ready] root=none");
        loop { if let Event::Close = window.next() { window.close() } }
    };
    let portal = root == b"/";
    let mut app = App { cwd: root.clone(), tabs: vec![FolderTab { cwd: root.clone(), cursor: 0, top: 0 }], active_tab: 0, quick_places: None, previous_folder: None, split: false, root, entries: Vec::new(), cursor: 0, selection_anchor: None, selection_marks: Vec::new(), top: 0, clip: None, clip_batch: None, undo: Vec::new(), redo: Vec::new(), input: None, status: String::new(), question: None, disk_ask: None, portal,
                        job: None, job_undo: None, history_job: None, job_select: None, last_drawn: 0, theme_generation: sex_app::settings::generation().unwrap_or(0), last_theme_check: sex_pdx::get_ticks(), last_pct: 0, confirm: None, origins: Vec::new(), watching: None, last_look: 0, index: Index::new(), search: None, crawl: None, found: None, thumbs: Thumbs::new(), showing_pixels: false, loom: loom::Loom::new(), opening: None, session_ready: false, preview: None, picture_preview: None };
    app.meta_load(&mut window);
    app.loom.actions.load(&mut window);
    let start = app.root.clone();
    app.go(&mut window, start);
    app.session_load(&mut window);
    app.session_ready = true;
    // A locked home asks for its password first: Files opened at login unlocks it.
    if portal && files::home_state(&mut window) == Ok(sex_pdx::home_lock::STATE_LOCKED) {
        sex_pdx::serial_record!("[files.app.home] state=locked");
        app.input = Some((Input::Password(Secret::Unlock), Vec::new()));
        app.status = "Home is locked. Type its password, or Esc to leave it locked.".into();
    }
    app.poll_questions(&mut window);
    // Voyeur asks this run about its files: their names and the words inside them.
    app.crawl = Some(Search::crawl(&mut window, &mut app.index, &app.root));
    let registered = sex_app::search::register("loom", &[sex_app::search::KIND_FILE, sex_app::search::KIND_CONTENT]);
    sex_pdx::serial_record!("[files.app.voyeur.register] sent={}", registered as u8);
    app.render(&mut window);
    sex_pdx::serial_record!("[files.app.theme] mode={:?}", sex_app::settings::theme().mode);
    sex_pdx::serial_record!("[files.app.ready] target={} task={} root={} portal={}", window.target, window.task, shown(&app.root), portal as u8);
    // Where the left button went down, until a drag starts or it comes up.
    let mut press: Option<(i32, i32)> = None;
    loop {
        let now = sex_pdx::get_ticks();
        if now.saturating_sub(app.last_theme_check) >= 500 {
            app.last_theme_check = now;
            let generation = sex_app::settings::generation().unwrap_or(app.theme_generation);
            if generation != app.theme_generation {
                app.theme_generation = generation;
                app.render(&mut window);
                sex_pdx::serial_record!("[files.app.theme] mode={:?}", sex_app::settings::theme().mode);
            }
        }
        if let Some((cookie, program)) = app.opening.clone() {
            match sex_app::launch::poll(cookie) {
                Some(sex_app::launch::Event::Refused { code, .. }) => {
                    app.opening = None;
                    sex_pdx::serial_record!("[files.app.open.refused] program={} code={}", shown(&program), code);
                    app.status = if code == sex_pdx::program_store::NOT_FOUND { format!("{} isn't installed.", shown(&program)) } else { format!("{} couldn't start.", shown(&program)) };
                    app.render(&mut window);
                }
                Some(_) => app.opening = None,
                None => {}
            }
        }
        if app.voyeur(&mut window) { app.render(&mut window); }
        // While a job runs, look for keys between its pieces instead of sleeping.
        let event = if app.job.is_some() || app.search.is_some() || app.crawl.is_some() || app.thumbs.busy() || app.loom.busy() {
            match window.poll() { Some(e) => Some(e), None => { if app.work(&mut window) { app.render(&mut window); } continue; } }
        } else { window.next_within(400) };
        match event {
            Some(Event::Close) => {
                // Nothing half-copied stays behind a closed window.
                if let Some(job) = app.job.as_mut() { while job.finishing() { if !matches!(job.step(&mut window), Outcome::Running) { break; } } let _ = job.cancel(&mut window); }
                app.session_save(&mut window);
                let _ = files::sync(&mut window); sex_pdx::serial_record!("[files.app.closed]"); window.close()
            }
            Some(Event::Key(key)) => { app.key(&mut window, key); app.render(&mut window); }
            Some(Event::Resize { .. }) if app.thumbs.on || app.picture_preview.is_some() => app.render(&mut window),
            // Dragging out of the window carries the selected item to another program.
            Some(Event::Pointer(p)) => {
                use sex_pdx::app_pointer::Kind as K;
                match p.kind {
                    K::Down { button: 0 } => {
                        if app.split && app.thumbs.on && app.tabs.len() > 1 && app.tab_ready()
                            && split_grid_fits(&mut window) {
                            let other = (app.active_tab + 1) % app.tabs.len();
                            let other_len = files::list(&mut window, &at(&app.tabs[other].cwd)).map_or(0, |v| v.len());
                            let (width, height) = window.size();
                            let (pane, mut index) = loom_core::split::grid_hit(width, p.x.max(0) as usize, p.y.max(0) as usize,
                                thumbs::CELL_W, thumbs::CELL_H, thumbs::HEAD, app.top, app.tabs[other].top,
                                app.entries.len(), other_len);
                            if p.y < thumbs::HEAD as i16 || p.y as usize >= height.saturating_sub(thumbs::FOOT) { index = None; }
                            if pane == 1 { app.show_tab(&mut window, other); }
                            index = index.filter(|&row| row < app.entries.len());
                            if let Some(index) = index { app.grid_click(index, p.mods & sex_pdx::app_pointer::CONTROL != 0); }
                            sex_pdx::serial_record!("[loom.split.focus] pane={} active={} dir={} item={}", pane + 1,
                                app.active_tab + 1, shown(&app.cwd), index.map_or(0, |i| i + 1));
                            app.render(&mut window);
                            press = None;
                        } else if app.thumbs.on && app.tab_ready() {
                            let (width, height) = window.size();
                            let index = loom_core::split::single_grid_hit(width, height, p.x.max(0) as usize,
                                p.y.max(0) as usize, thumbs::CELL_W, thumbs::CELL_H, thumbs::HEAD,
                                thumbs::FOOT, app.top, app.entries.len());
                            if let Some(index) = index {
                                let control = p.mods & sex_pdx::app_pointer::CONTROL != 0;
                                app.grid_click(index, control);
                                app.render(&mut window);
                                press = (!control).then_some((p.x as i32, p.y as i32));
                            } else { press = None; }
                        } else { press = Some((p.x as i32, p.y as i32)); }
                    }
                    K::Move if p.buttons & 1 != 0 => if let Some((x0, y0)) = press {
                        if (p.x as i32 - x0).abs() + (p.y as i32 - y0).abs() > 8 {
                            press = None;
                            if let Some(path) = app.selected() {
                                let mut token = vec![b'@'];
                                token.extend_from_slice(&path);
                                match sex_app::dnd::drag_file(&mut window, &token) {
                                    Ok(()) => { sex_pdx::serial_record!("[files.app.drag] path={}", shown(&path)); app.status = format!("Drop {} on a program window.", shown(&path)); }
                                    Err(e) => { sex_pdx::serial_record!("[files.app.drag.refused] why={:?}", e); app.status = "That can't be dragged.".into(); }
                                }
                                app.render(&mut window);
                            }
                        }
                    },
                    K::Up { .. } => press = None,
                    _ => {}
                }
            }
            Some(Event::DragEnded { target }) => {
                sex_pdx::serial_record!("[files.app.dropped] target={}", target);
                app.status = if target != 0 { "Dropped.".into() } else { String::new() };
                app.render(&mut window);
            }
            Some(_) => {}
            // Quiet: another program may be asking the person for a file.
            None => {
                let mut redraw = app.poll_questions(&mut window);
                let now = sex_pdx::get_ticks();
                if now.saturating_sub(app.last_look) >= 1000 { app.last_look = now; redraw |= app.look_for_pulled(&mut window); }
                if redraw { app.render(&mut window); }
            }
        }
    }
}
