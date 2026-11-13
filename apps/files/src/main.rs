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

mod jobs;
mod search;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::{self, Entry, Pick, Question};
use sex_app::{Event, Nav, Window};
use sex_pdx::fs as w;
use jobs::{Job, Kind, Outcome};
use search::{Index, Search};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[files.app.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ROWS: usize = 18;
const TRASH: &[u8] = b".Trash";
const INFO: &[u8] = b".Trash/.info";

const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;
const ESC: u8 = 0x01;
const F2: u8 = 0x3C;
const F5: u8 = 0x3F;
// Letters, PS/2 set 1, used with Ctrl.
const KEY_C: u8 = 0x2E;
const KEY_X: u8 = 0x2D;
const KEY_V: u8 = 0x2F;
const KEY_Z: u8 = 0x2C;
const KEY_N: u8 = 0x31;
const KEY_T: u8 = 0x14;
const KEY_R: u8 = 0x13;
const KEY_H: u8 = 0x23;
const KEY_D: u8 = 0x20;
const KEY_S: u8 = 0x1F;
const KEY_E: u8 = 0x12;
const KEY_I: u8 = 0x17;
const KEY_F: u8 = 0x21;

/// One step Ctrl+Z can take back.
enum Undo { Moved { from: Vec<u8>, to: Vec<u8> }, Trashed { from: Vec<u8>, to: Vec<u8>, info: Vec<u8> }, Copied { to: Vec<u8> }, Made { path: Vec<u8> } }
enum Input { Rename(Vec<u8>), NewFolder, SaveAs, Search }
/// Something that can't be undone, waiting for the same key a second time.
#[derive(PartialEq, Eq)]
enum Confirm { DeleteForever(Vec<u8>), EmptyTrash(Vec<u8>) }
struct App {
    root: Vec<u8>,
    cwd: Vec<u8>,
    entries: Vec<Entry>,
    cursor: usize,
    top: usize,
    clip: Option<(Vec<u8>, bool)>,
    undo: Vec<Undo>,
    input: Option<(Input, Vec<u8>)>,
    status: String,
    question: Option<Question>,
    /// This run can see questions: it holds the whole filesystem.
    portal: bool,
    /// Copying, moving, deleting or checking in the background.
    job: Option<Job>,
    /// What Ctrl+Z will take back once the job finishes.
    job_undo: Option<Undo>,
    /// Name to select once the job finishes.
    job_select: Option<Vec<u8>>,
    last_drawn: u64,
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
    /// The listing shows search results for this, not the folder.
    found: Option<Vec<u8>>,
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
    fn load(&mut self, window: &mut Window) {
        self.found = None;
        match files::list(window, &at(&self.cwd)) {
            Ok(mut list) => {
                // A copy still being written (or cut short by a pulled stick) is not a file yet.
                list.retain(|e| e.name != TRASH && !(e.name.first() == Some(&b'.') && e.name.ends_with(b".part")));
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
        sex_pdx::serial_record!("[files.app.list] dir={} count={}", shown(&self.cwd), self.entries.len());
    }
    fn go(&mut self, window: &mut Window, dir: Vec<u8>) {
        self.cwd = dir; self.cursor = 0; self.top = 0; self.load(window);
        let root = disk_root(&self.cwd);
        self.watching = (root != b"/" && self.status.is_empty()).then_some(root);
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
    fn selected(&self) -> Option<Vec<u8>> { self.entries.get(self.cursor).map(|e| join(&self.cwd, &e.name)) }
    fn select(&mut self, name: &[u8]) { if let Some(i) = self.entries.iter().position(|e| e.name == name) { self.cursor = i; } }

    fn render(&mut self, window: &mut Window) {
        let mut t = String::new();
        if let Some(sr) = &self.search {
            t.push_str(&format!("Searching {} for \"{}\": {} found in {} files so far. Esc stops.\n\n", shown(&sr.base), shown(&sr.query), sr.results.len(), sr.looked));
        } else if let Some(q) = &self.found {
            t.push_str(&format!("Found {} for \"{}\" in {}. Enter opens its folder, Esc goes back.\n", self.entries.len(), shown(q), shown(&self.cwd)));
        }
        let space = files::space(window, &at(&self.cwd)).ok().filter(|s| s.0 > 0).map(|(total, free)| format!("  {} free of {}", human(free), human(total))).unwrap_or_default();
        t.push_str(&format!("Files  {}{}\n", shown(&self.cwd), space));
        if let Some(q) = &self.question {
            let what = match q.mode { Pick::Open => "open a file: Enter chooses it", Pick::Save => "save a file: Ctrl+S names it here", Pick::Folder => "pick a folder: Ctrl+S chooses this one" };
            t.push_str(&format!(">> {} asks to {}. Esc says no.\n", shown(&q.program), what));
        }
        t.push('\n');
        if self.entries.is_empty() { t.push_str("  (empty)\n"); }
        if self.cursor < self.top { self.top = self.cursor; }
        if self.cursor >= self.top + ROWS { self.top = self.cursor + 1 - ROWS; }
        for (i, e) in self.entries.iter().enumerate().skip(self.top).take(ROWS) {
            let mark = if i == self.cursor { "> " } else { "  " };
            let from = match self.origins.get(i) { Some(Some(o)) => format!("   from {}", shown(o)), _ => String::new() };
            if e.dir { t.push_str(&format!("{}{}/{}\n", mark, shown(&e.name), from)); } else { t.push_str(&format!("{}{}   {}{}\n", mark, shown(&e.name), human(e.size), from)); }
        }
        if self.entries.len() > self.top + ROWS { t.push_str(&format!("  ... {} more\n", self.entries.len() - self.top - ROWS)); }
        t.push('\n');
        if let Some((kind, text)) = &self.input {
            let label = match kind { Input::Rename(_) => "Rename to", Input::NewFolder => "New folder", Input::SaveAs => "Save as", Input::Search => "Search names and words" };
            t.push_str(&format!("{}: {}|   (Enter confirms, Esc cancels)\n", label, shown(text)));
        } else if !self.status.is_empty() { t.push_str(&self.status); t.push('\n'); }
        if let Some(job) = &self.job { t.push_str(&job.progress()); t.push('\n'); }
        t.push_str("Enter open  Backspace up  F2 rename  Ctrl+I size and checksum\nCtrl+C/X/V copy, cut, paste  Ctrl+N folder\nDel trash  Shift+Del delete for good  Ctrl+T show trash\nIn the trash: Ctrl+R restore  Ctrl+E empty\nOn a stick: Ctrl+E eject  Ctrl+D disks  Ctrl+Z undo  Ctrl+H home\nCtrl+F search names and words in files\n");
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
        if self.search.is_some() { return self.search_step(window); }
        let Some(job) = self.job.as_mut() else { return false };
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
                if kind == Kind::Copy || kind == Kind::Move {
                    sex_pdx::serial_record!("[files.app.pasted] from={} to={} cut={}", shown(&job.from), shown(&job.to), (kind == Kind::Move) as u8);
                }
                if let Some(u) = self.job_undo.take() { self.undo.push(u); }
                self.status = summary;
                self.load(window);
                if let Some(name) = self.job_select.take() { self.select(&name); }
                true
            }
            Outcome::Failed(kind, why) => {
                sex_pdx::serial_record!("[files.app.job.failed] kind={:?} why={}", kind, why);
                self.job = None;
                self.job_undo = None;
                self.status = format!("Couldn't finish: {}. Nothing half-done was left.", why);
                if !self.look_for_pulled(window) { self.load(window); }
                true
            }
        }
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
        self.load(window);
    }
    fn remove_all(&mut self, window: &mut Window, path: &[u8]) -> Result<(), u8> {
        if files::stat(window, &at(path))?.dir {
            for e in files::list(window, &at(path))? { self.remove_all(window, &join(path, &e.name))?; }
        }
        files::remove(window, &at(path))
    }
    /// Rename where the disk allows it, else a background move.
    fn move_path(&mut self, window: &mut Window, from: &[u8], to: &[u8]) -> Result<(), u8> {
        match files::rename(window, &at(from), &at(to)) {
            Err(w::OTHER_DISK) => { let job = Job::move_across(window, from, to); self.start(window, job, None, None); Ok(()) }
            other => other,
        }
    }
    /// A name not yet taken in `dir`: `name`, `name (copy)`, `name (copy 2)`...
    fn free_name(&mut self, window: &mut Window, dir: &[u8], name: &[u8], tag: &str) -> Vec<u8> {
        let try_name = join(dir, name);
        if files::stat(window, &at(&try_name)).is_err() { return try_name; }
        let (stem, ext) = match name.iter().rposition(|b| *b == b'.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name, &name[name.len()..]) };
        for k in 1..1000 {
            let mut n = stem.to_vec();
            n.extend_from_slice(if k == 1 { format!(" ({})", tag) } else { format!(" ({} {})", tag, k) }.as_bytes());
            n.extend_from_slice(ext);
            let p = join(dir, &n);
            if files::stat(window, &at(&p)).is_err() { return p; }
        }
        try_name
    }

    fn paste(&mut self, window: &mut Window) {
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
                self.status = format!("{} {}.", if cut { "Moved" } else { "Copied" }, shown(base(&to)));
                self.undo.push(if cut { Undo::Moved { from: from.clone(), to: to.clone() } } else { Undo::Copied { to: to.clone() } });
                if cut { self.clip = None; }
                let _ = files::sync(window);
                self.load(window);
                self.select(base(&to));
            }
            Err(e) => self.status = format!("Couldn't paste: {}.", shown(files::message(e))),
        }
    }
    fn trash(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { return };
        let root = disk_root(&path);
        let (trash, info) = (join(&root, TRASH), join(&root, INFO));
        let _ = files::mkdir(window, &at(&trash));
        let _ = files::mkdir(window, &at(&info));
        let to = self.free_name(window, &trash, base(&path), "trashed");
        match files::rename(window, &at(&path), &at(&to)) {
            Ok(()) => {
                // Where it came from, for Ctrl+R.
                let note = join(&info, base(&to));
                let _ = files::write(window, &at(&note), &path);
                sex_pdx::serial_record!("[files.app.trashed] from={} to={}", shown(&path), shown(&to));
                self.status = format!("Moved {} to the trash. Ctrl+Z puts it back.", shown(base(&path)));
                self.undo.push(Undo::Trashed { from: path, to, info: note });
                let _ = files::sync(window);
                self.load(window);
            }
            Err(e) => self.status = format!("Couldn't move to the trash: {}.", shown(files::message(e))),
        }
    }
    /// Shift+Del: removes the selected item for good, after a second Shift+Del.
    fn delete_forever(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { return };
        let ask = Confirm::DeleteForever(path.clone());
        if self.confirm.as_ref() != Some(&ask) {
            self.status = format!("Delete {} for good? It won't go to the trash. Shift+Del again to delete, any other key keeps it.", shown(base(&path)));
            self.confirm = Some(ask);
            return;
        }
        self.confirm = None;
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
        let list = files::list(window, &at(&self.cwd)).unwrap_or_default();
        let paths: Vec<Vec<u8>> = list.iter().map(|e| join(&self.cwd, &e.name)).collect();
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
            Ok(()) => { let _ = files::remove(window, &at(&note)); sex_pdx::serial_record!("[files.app.restored] from={} to={}", shown(&path), shown(&back)); self.status = format!("Restored {}.", shown(&back)); self.undo.push(Undo::Moved { from: path, to: back }); let _ = files::sync(window); self.load(window); }
            Err(e) => self.status = format!("Couldn't restore: {}.", shown(files::message(e))),
        }
    }
    fn undo(&mut self, window: &mut Window) {
        let Some(step) = self.undo.pop() else { self.status = "Nothing to undo.".into(); return };
        let result = match &step {
            Undo::Moved { from, to } => self.move_path(window, to, from),
            Undo::Trashed { from, to, info } => self.move_path(window, to, from).map(|_| { let _ = files::remove(window, &at(info)); }),
            Undo::Copied { to } => self.remove_all(window, to),
            Undo::Made { path } => files::remove(window, &at(path)),
        };
        match result {
            Ok(()) => { sex_pdx::serial_record!("[files.app.undone] left={}", self.undo.len()); self.status = "Undone.".into(); let _ = files::sync(window); self.load(window); }
            Err(e) => { self.status = format!("Couldn't undo: {}.", shown(files::message(e))); self.undo.push(step); }
        }
    }
    fn finish_input(&mut self, window: &mut Window) {
        let Some((kind, text)) = self.input.take() else { return };
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
                    Ok(()) => { self.status = format!("Renamed to {}.", shown(&text)); self.undo.push(Undo::Moved { from, to }); self.load(window); self.select(&text); }
                    Err(e) => self.status = format!("Couldn't rename: {}.", shown(files::message(e))),
                }
            }
            Input::NewFolder => {
                let path = join(&self.cwd, &text);
                match files::mkdir(window, &at(&path)) {
                    Ok(()) => { self.status = format!("Made {}.", shown(&text)); self.undo.push(Undo::Made { path }); self.load(window); self.select(&text); }
                    Err(e) => self.status = format!("Couldn't make the folder: {}.", shown(files::message(e))),
                }
            }
            Input::Search => {}
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
        if !self.portal || self.question.is_some() { return false; }
        match files::questions(window) {
            Ok(mut qs) if !qs.is_empty() => {
                let q = qs.remove(0);
                sex_pdx::serial_record!("[files.app.question] program={} mode={:?}", shown(&q.program), q.mode);
                self.question = Some(q);
                true
            }
            Ok(_) => false,
            Err(_) => { self.portal = false; false }
        }
    }

    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let before = self.confirm.is_some();
        self.key_inner(window, key);
        let again = matches!(self.confirm, Some(_)) && (key.code == KEY_E && key.control || matches!(sex_app::nav(key), Some(Nav::Delete)) && key.shift);
        if before && !again && self.confirm.is_some() { self.confirm = None; self.status = "Kept.".into(); }
    }
    fn key_inner(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        if let Some((_, text)) = self.input.as_mut() {
            match key.code {
                ENTER => self.finish_input(window),
                ESC => { self.input = None; }
                BACKSPACE => { while let Some(b) = text.pop() { if b & 0xC0 != 0x80 { break; } } }
                _ => if let Some(c) = sex_app::typed(key) { if text.len() < 255 { text.push(c); } },
            }
            return;
        }
        // Shift+Del and Ctrl+E ask twice; anything else between keeps things.
        let confirming = self.confirm.is_some();
        if let Some(sr) = self.search.take() {
            if key.code == ESC { self.show_found(&sr, false); } else { self.search = Some(sr); self.status = "Searching: Esc stops.".into(); }
            return;
        }
        if self.job.is_some() {
            if key.code == ESC { self.stop_job(window); return; }
            let changes = key.control && matches!(key.code, KEY_V | KEY_Z | KEY_N | KEY_R | KEY_E | KEY_I)
                || matches!(sex_app::nav(key), Some(Nav::Delete)) || key.code == F2;
            if changes { self.status = "Busy with the job below: wait for it, or Esc stops it.".into(); return; }
        }
        self.status.clear();
        if key.control {
            match key.code {
                KEY_C | KEY_X => if let Some(p) = self.selected() {
                    let cut = key.code == KEY_X;
                    self.status = format!("{} {}: Ctrl+V pastes it.", if cut { "Cut" } else { "Copied" }, shown(base(&p)));
                    self.clip = Some((p, cut));
                },
                KEY_V => self.paste(window),
                KEY_Z => self.undo(window),
                KEY_N => self.input = Some((Input::NewFolder, Vec::new())),
                KEY_T => { let t = join(&disk_root(&self.cwd), TRASH); self.go(window, t); }
                KEY_R => self.restore(window),
                // Ctrl+E: Empty in a trash, Eject on a stick or disk.
                KEY_E if base(&self.cwd) != TRASH && (self.cwd == w::MEDIA || disk_root(&self.cwd) != b"/") => self.eject(window),
                KEY_E => self.empty_trash(window),
                KEY_I => self.info(window),
                KEY_F => self.input = Some((Input::Search, Vec::new())),
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
        match (key.code, sex_app::nav(key)) {
            (_, Some(Nav::Up)) => self.cursor = self.cursor.saturating_sub(1),
            (_, Some(Nav::Down)) => self.cursor = (self.cursor + 1).min(self.entries.len().saturating_sub(1)),
            (_, Some(Nav::Home)) => self.cursor = 0,
            (_, Some(Nav::End)) => self.cursor = self.entries.len().saturating_sub(1),
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
                else { self.status = format!("{}: {}. Programs open it with run NAME @{}.", shown(&e.name), human(e.size), shown(&path)); }
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
            (F2, _) => if let Some(p) = self.selected() { let name = base(&p).to_vec(); self.input = Some((Input::Rename(p), name)); },
            (F5, _) => self.load(window),
            _ => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Files");
    // The folder the person gave: `@/` is everything.
    let mut args = [0u8; w::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let root = given.split(|b| *b == b' ').find_map(|t| t.strip_prefix(b"@")).filter(|p| p.first() == Some(&b'/')).map(|p| p.to_vec());
    let Some(root) = root else {
        window.show(b"Files needs a folder to look after. Start it from Spindle with:  files  (everything)  or  files DIR\n\nF11 closes.");
        sex_pdx::serial_record!("[files.app.ready] root=none");
        loop { if let Event::Close = window.next() { window.close() } }
    };
    let portal = root == b"/";
    let mut app = App { cwd: root.clone(), root, entries: Vec::new(), cursor: 0, top: 0, clip: None, undo: Vec::new(), input: None, status: String::new(), question: None, portal,
                        job: None, job_undo: None, job_select: None, last_drawn: 0, last_pct: 0, confirm: None, origins: Vec::new(), watching: None, last_look: 0, index: Index::new(), search: None, found: None };
    let start = app.root.clone();
    app.go(&mut window, start);
    app.poll_questions(&mut window);
    app.render(&mut window);
    sex_pdx::serial_record!("[files.app.ready] target={} task={} root={} portal={}", window.target, window.task, shown(&app.root), portal as u8);
    loop {
        // While a job runs, look for keys between its pieces instead of sleeping.
        let event = if app.job.is_some() || app.search.is_some() {
            match window.poll() { Some(e) => Some(e), None => { if app.work(&mut window) { app.render(&mut window); } continue; } }
        } else { window.next_within(400) };
        match event {
            Some(Event::Close) => {
                // Nothing half-copied stays behind a closed window.
                if let Some(job) = app.job.as_mut() { while job.finishing() { if !matches!(job.step(&mut window), Outcome::Running) { break; } } let _ = job.cancel(&mut window); }
                let _ = files::sync(&mut window); sex_pdx::serial_record!("[files.app.closed]"); window.close()
            }
            Some(Event::Key(key)) => { app.key(&mut window, key); app.render(&mut window); }
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
