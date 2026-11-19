// SPDX-License-Identifier: GPL-3.0-only
//! Archives in Files: Enter on a zip, tar (plain, gz, zst, xz) or 7z browses into it as a
//! read-only folder; Ctrl+Shift+X extracts it next to itself (or, inside one, copies the
//! selected item out); Ctrl+Shift+A packs the selected file or folder into a zip.
//!
//! The archive is read through Files' own file calls a piece at a time, never loaded
//! whole. What is extracted or packed is written under a hidden `.NAME.part` name and
//! gets its real name only when it is whole, as copies do, so Esc, a closed window or a
//! damaged archive never leaves something that looks finished but is not.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_app::files::{self, direct, Entry};
use sex_app::{Event, Nav, Window};
use sex_archive::{Archive, ArchiveBuilder, ByteSource, EntryKind, ExtractVisitor, Flow, IoError, OutputFormat, Progress, Sink, SourceReader};

use crate::{at, base, human, join, parent, shown, App, Undo, BACKSPACE, ENTER, ESC};

/// Names Files opens as archives, longest first, so `.tar.gz` wins over `.gz`.
const SUFFIXES: &[&[u8]] = &[b".tar.gz", b".tar.zst", b".tar.xz", b".tgz", b".tzst", b".txz", b".tar", b".zip", b".7z", b".gz", b".zst", b".xz"];

/// The archive suffix of `name`, if it is one.
fn suffix(name: &[u8]) -> Option<&'static [u8]> {
    let lower = name.to_ascii_lowercase();
    SUFFIXES.iter().copied().find(|s| lower.len() > <[u8]>::len(s) && lower.ends_with(s))
}
pub fn is_archive(name: &[u8]) -> bool { suffix(name).is_some() }
/// `photos.tar.gz` -> `photos`.
fn stem(name: &[u8]) -> &[u8] { match suffix(name) { Some(s) => &name[..name.len() - <[u8]>::len(s)], None => name } }

/// An archive file read through Files, with a small read cache: opening reads many
/// small headers, and each read is a round trip.
pub struct FileSource { path: Vec<u8>, len: u64, cache: RefCell<(u64, Vec<u8>)> }
const BLOCK: usize = 64 * 1024;
impl FileSource {
    pub fn open(abs: &[u8]) -> Result<Self, u8> {
        let path = at(abs);
        let st = direct::stat(&path)?;
        if st.dir { return Err(sex_pdx::fs::IS_DIR); }
        Ok(FileSource { path, len: st.size, cache: RefCell::new((u64::MAX, Vec::new())) })
    }
}
impl ByteSource for FileSource {
    fn len(&self) -> u64 { self.len }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        if offset >= self.len || buf.is_empty() { return Ok(0); }
        if buf.len() >= BLOCK / 2 {
            return direct::read_at(&self.path, offset, buf).map_err(|_| IoError("couldn't read the archive"));
        }
        let mut c = self.cache.borrow_mut();
        let hit = c.0 != u64::MAX && offset >= c.0 && offset < c.0 + c.1.len() as u64;
        if !hit {
            let start = offset & !4095;
            let mut block = vec![0u8; BLOCK];
            let n = direct::read_at(&self.path, start, &mut block).map_err(|_| IoError("couldn't read the archive"))?;
            block.truncate(n);
            *c = (start, block);
            if offset >= start + n as u64 { return Ok(0); }
        }
        let at = (offset - c.0) as usize;
        let n = (c.1.len() - at).min(buf.len());
        buf[..n].copy_from_slice(&c.1[at..at + n]);
        Ok(n)
    }
}

/// Writes a new file through Files, piece by piece.
pub(crate) struct FileSink { pub(crate) path: Vec<u8>, pub(crate) offset: u64 }
impl Sink for FileSink {
    fn write_all(&mut self, data: &[u8]) -> Result<(), IoError> {
        let n = direct::write_at(&self.path, self.offset, data).map_err(|_| IoError("couldn't write"))?;
        if n < data.len() { return Err(IoError("the disk is full")); }
        self.offset += n as u64;
        Ok(())
    }
}

/// Makes `dir` and every folder above it that is missing, up to `top` (which exists).
pub(crate) fn mkdirs(top: &[u8], rel: &str, made: &mut BTreeSet<String>) -> Result<(), IoError> {
    let mut so_far = String::new();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        if !so_far.is_empty() { so_far.push('/'); }
        so_far.push_str(part);
        if made.contains(&so_far) { continue; }
        match direct::mkdir(&at(&join(top, so_far.as_bytes()))) {
            Ok(()) | Err(sex_pdx::fs::EXISTS) => { made.insert(so_far.clone()); }
            Err(_) => return Err(IoError("couldn't make a folder")),
        }
    }
    Ok(())
}

/// Receives members from the archive and writes them under `top`.
struct Writer {
    top: Vec<u8>,
    /// Only this member (a file, or a folder and what is in it); everything when empty.
    only: String,
    /// Cut from the front of each member's path: the folder `only` is in.
    strip: String,
    made: BTreeSet<String>,
    file: Option<FileSink>,
    files: u64,
    bytes: u64,
}
impl Writer {
    fn wanted(&self, path: &str) -> Option<String> {
        if !self.only.is_empty() && path != self.only && !(path.starts_with(&self.only) && path.as_bytes().get(self.only.len()) == Some(&b'/')) { return None; }
        let rel = path.strip_prefix(&self.strip).unwrap_or(path).trim_start_matches('/');
        (!rel.is_empty()).then(|| rel.to_string())
    }
}
impl ExtractVisitor for Writer {
    fn begin(&mut self, entry: &sex_archive::Entry, path: &str) -> bool {
        if !matches!(entry.kind, EntryKind::File | EntryKind::Hardlink) { return false; }
        let Some(rel) = self.wanted(path) else { return false };
        if let Some(i) = rel.rfind('/') { if mkdirs(&self.top, &rel[..i], &mut self.made).is_err() { return false; } }
        let dest = at(&join(&self.top, rel.as_bytes()));
        if direct::create(&dest).is_err() && direct::truncate(&dest, 0).is_err() { return false; }
        self.file = Some(FileSink { path: dest, offset: 0 });
        self.files += 1;
        true
    }
    fn data(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        self.bytes += bytes.len() as u64;
        match self.file.as_mut() { Some(f) => f.write_all(bytes), None => Ok(()) }
    }
    fn end(&mut self, _entry: &sex_archive::Entry) -> Result<(), IoError> { self.file = None; Ok(()) }
}

/// Being inside an archive: Files lists its members in place of a folder.
pub struct Inside {
    arch: Archive<FileSource>,
    /// The archive file, and the folder it is in.
    pub file: Vec<u8>,
    pub home: Vec<u8>,
    /// The folder inside the archive being looked at ("" is its top).
    pub dir: String,
}

/// How an operation with a progress line ended.
pub(crate) enum End { Done, Stopped, Failed(String) }

/// Key polling and redrawing while a long archive operation runs.
pub(crate) struct Ticker { verb: &'static str, what: String, last: u64, pct: u64, closing: bool, started: u64 }
impl Ticker {
    pub(crate) fn new(verb: &'static str, what: &[u8]) -> Self { Ticker { verb, what: shown(what).into(), last: 0, pct: 101, closing: false, started: sex_pdx::get_ticks() } }
    pub(crate) fn tick(&mut self, app: &mut App, window: &mut Window, p: Progress) -> Flow {
        let pct = if p.total == 0 { 0 } else { p.done.min(p.total) * 100 / p.total };
        if pct / 10 != self.pct / 10 { sex_pdx::serial_record!("[files.app.archive.progress] kind={} pct={} done={} total={}", self.verb, pct, p.done, p.total); }
        self.pct = pct;
        // Keys that arrived meanwhile: Esc stops; closing the window stops and then closes.
        while let Some(e) = window.poll() {
            match e {
                Event::Key(k) if k.code == ESC => return Flow::Cancel,
                Event::Close => { self.closing = true; return Flow::Cancel; }
                _ => {}
            }
        }
        let now = sex_pdx::get_ticks();
        if now.saturating_sub(self.last) >= 150 {
            self.last = now;
            let filled = (pct / 5) as usize;
            let bar: String = (0..20).map(|i| if i < filled { '#' } else { '.' }).collect();
            let ms = now.saturating_sub(self.started).max(1);
            app.status = format!("{} {} [{}] {}%  {} of {}, {}/s  (Esc stops)", self.verb, self.what, bar, pct, human(p.done), human(p.total), human(p.done * 1000 / ms));
            app.render(window);
        }
        Flow::Continue
    }
    /// The window was closed during the operation: it has been cleaned up, so go.
    pub(crate) fn finish(&self, window: &mut Window) {
        if self.closing { let _ = files::sync(window); sex_pdx::serial_record!("[files.app.closed]"); sex_pdx::task_finish(0); }
    }
}

impl App {
    /// Enter on an archive: open it and list its top.
    pub(crate) fn archive_open(&mut self, window: &mut Window, path: Vec<u8>) {
        let name = base(&path).to_vec();
        self.status = format!("Opening {}...", shown(&name));
        self.render(window);
        let opened = FileSource::open(&path).map_err(|e| String::from(shown(files::message(e)))).and_then(|src| {
            Archive::open_named(src, core::str::from_utf8(&name).ok()).map_err(|e| e.to_string())
        });
        match opened {
            Ok(arch) => {
                sex_pdx::serial_record!("[files.app.archive.opened] path={} format={:?} entries={}", shown(&path), arch.format(), arch.entries().len());
                let home = parent(&path);
                self.loom.inside = Some(Inside { arch, file: path, home, dir: String::new() });
                self.status = "Inside an archive, read only. Ctrl+Shift+X copies the selected item out next to it; Backspace leaves.".into();
                self.archive_list();
            }
            Err(why) => {
                sex_pdx::serial_record!("[files.app.archive.unreadable] path={} why={}", shown(&path), why);
                self.status = format!("Can't open {} as an archive: {}.", shown(&name), why);
            }
        }
    }
    /// Lists the folder of the archive being looked at in place of a real one.
    fn archive_list(&mut self) {
        let Some(inside) = self.loom.inside.as_ref() else { return };
        let mut list: Vec<Entry> = inside.arch.list_dir(&inside.dir).unwrap_or_default().into_iter()
            .map(|d| Entry { dir: d.is_dir(), size: d.size, name: d.name.into_bytes() }).collect();
        list.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
        let mut cwd = inside.file.clone();
        if !inside.dir.is_empty() { cwd.push(b'/'); cwd.extend_from_slice(inside.dir.as_bytes()); }
        sex_pdx::serial_record!("[files.app.archive.list] archive={} dir={} count={}", shown(&inside.file), if inside.dir.is_empty() { "/" } else { &inside.dir }, list.len());
        self.entries = list;
        self.cwd = cwd;
        self.found = None;
        self.origins.clear();
        self.cursor = 0;
        self.top = 0;
    }
    fn archive_leave(&mut self, window: &mut Window) {
        let Some(inside) = self.loom.inside.take() else { return };
        sex_pdx::serial_record!("[files.app.archive.left] path={}", shown(&inside.file));
        let name = base(&inside.file).to_vec();
        self.go(window, inside.home);
        self.select(&name);
    }
    /// Keys while inside an archive. False lets Files handle the key (moving the cursor).
    pub(crate) fn archive_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> bool {
        if matches!(sex_app::nav(key), Some(Nav::Up | Nav::Down | Nav::Home | Nav::End | Nav::Left | Nav::Right)) { return false; }
        if key.control && key.shift && key.code == crate::loom::KEY_X { self.extract(window); return true; }
        // Going somewhere else leaves the archive first.
        if key.control && !key.shift && matches!(key.code, crate::KEY_H | crate::KEY_D | crate::KEY_T | crate::KEY_G) { self.loom.inside = None; return false; }
        match key.code {
            ESC => self.archive_leave(window),
            BACKSPACE => {
                let inside = self.loom.inside.as_mut().unwrap();
                if inside.dir.is_empty() { self.archive_leave(window); }
                else {
                    let here = match inside.dir.rfind('/') { Some(i) => { let h = inside.dir[i + 1..].to_string(); inside.dir.truncate(i); h } None => core::mem::take(&mut inside.dir) };
                    self.archive_list();
                    self.select(here.as_bytes());
                }
            }
            ENTER => {
                let Some(e) = self.entries.get(self.cursor) else { return true };
                if e.dir {
                    let inside = self.loom.inside.as_mut().unwrap();
                    if !inside.dir.is_empty() { inside.dir.push('/'); }
                    inside.dir.push_str(shown(&e.name));
                    self.archive_list();
                } else {
                    self.status = format!("{}: {} in the archive. Ctrl+Shift+X copies it out next to the archive.", shown(&e.name), human(e.size));
                }
            }
            _ => self.status = "This is inside an archive, read only: Ctrl+Shift+X copies the selected item out, Backspace leaves.".into(),
        }
        true
    }

    /// Ctrl+Shift+X: inside an archive, copies the selected member out next to the
    /// archive; on an archive in a folder, extracts all of it into a new folder named
    /// after it.
    pub(crate) fn extract(&mut self, window: &mut Window) {
        let (arch, file, home, only, inside_dir) = match self.loom.inside.take() {
            Some(inside) => {
                let Some(e) = self.entries.get(self.cursor) else { self.loom.inside = Some(inside); return };
                let mut only = inside.dir.clone();
                if !only.is_empty() { only.push('/'); }
                only.push_str(shown(&e.name));
                let dir = inside.dir.clone();
                (inside.arch, inside.file, inside.home, only, Some(dir))
            }
            None => {
                let Some(path) = self.selected() else { return };
                if !self.entries.get(self.cursor).is_some_and(|e| !e.dir && is_archive(&e.name)) {
                    self.status = "Ctrl+Shift+X extracts an archive (zip, tar, tar.gz, tar.zst, tar.xz, 7z): select one.".into();
                    return;
                }
                self.status = format!("Opening {}...", shown(base(&path)));
                self.render(window);
                let opened = FileSource::open(&path).map_err(|e| String::from(shown(files::message(e))))
                    .and_then(|s| Archive::open_named(s, core::str::from_utf8(base(&path)).ok()).map_err(|e| e.to_string()));
                match opened {
                    Ok(a) => (a, path.clone(), parent(&path), String::new(), None),
                    Err(why) => { self.status = format!("Can't open {} as an archive: {}.", shown(base(&path)), why); return; }
                }
            }
        };
        let whole = only.is_empty();
        // The name it gets once whole, and the hidden one it is written under until then.
        let final_name: Vec<u8> = if whole { stem(base(&file)).to_vec() } else { base(only.as_bytes()).to_vec() };
        let to = self.free_name(window, &home, &final_name, if whole { "extracted" } else { "copy" });
        let mut part_name = b".".to_vec();
        part_name.extend_from_slice(base(&to));
        part_name.extend_from_slice(b".part");
        let part = join(&home, &part_name);
        let _ = self.remove_all(window, &part);
        let member_is_file = !whole && arch.find(&only).is_some_and(|i| arch.entries()[i].kind != EntryKind::Dir);
        // A whole archive or a folder goes into the hidden folder; a single file is
        // written there and moved out.
        if let Err(e) = files::mkdir(window, &at(&part)) {
            self.status = format!("Couldn't extract: {}.", shown(files::message(e)));
            self.restore_inside(arch, file, home, inside_dir);
            return;
        }
        let strip = if whole { String::new() } else { match only.rfind('/') { Some(i) => only[..i + 1].to_string(), None => String::new() } };
        let mut writer = Writer { top: part.clone(), only: only.clone(), strip, made: BTreeSet::new(), file: None, files: 0, bytes: 0 };
        // Empty folders are members too.
        for e in arch.entries().iter().filter(|e| e.safe && e.kind == EntryKind::Dir) {
            if let Some(rel) = writer.wanted(&e.path) { let mut made = core::mem::take(&mut writer.made); let _ = mkdirs(&part, &rel, &mut made); writer.made = made; }
        }
        sex_pdx::serial_record!("[files.app.archive.extract.start] archive={} member={} to={}", shown(&file), if whole { "*" } else { &only }, shown(&to));
        let mut ticker = Ticker::new("Extracting", base(&to));
        let result = {
            let mut cb = |p: Progress| ticker.tick(self, window, p);
            arch.extract_all(&mut writer, &mut cb)
        };
        let end = match result {
            Ok(()) if writer.files == 0 && !whole && member_is_file => End::Failed("nothing could be read for it".into()),
            Ok(()) => {
                let from = if whole || !member_is_file { part.clone() } else { join(&part, base(only.as_bytes())) };
                match files::rename(window, &at(&from), &at(&to)) {
                    Ok(()) => { if from != part { let _ = self.remove_all(window, &part); } End::Done }
                    Err(e) => End::Failed(shown(files::message(e)).into()),
                }
            }
            Err(sex_archive::Error::Cancelled) => End::Stopped,
            Err(e) => End::Failed(e.to_string()),
        };
        let _ = files::sync(window);
        match end {
            End::Done => {
                sex_pdx::serial_record!("[files.app.archive.extracted] archive={} member={} to={} files={} bytes={}", shown(&file), if whole { "*" } else { &only }, shown(&to), writer.files, writer.bytes);
                self.status = format!("Extracted {} ({} files, {}). Ctrl+Z takes it back.", shown(base(&to)), writer.files, human(writer.bytes));
                self.undo.push(Undo::Copied { to: to.clone() });
            }
            End::Stopped => {
                let _ = self.remove_all(window, &part);
                sex_pdx::serial_record!("[files.app.archive.cancelled] kind=extract archive={}", shown(&file));
                self.status = "Stopped extracting: nothing half-extracted was left.".into();
            }
            End::Failed(why) => {
                let _ = self.remove_all(window, &part);
                sex_pdx::serial_record!("[files.app.archive.failed] kind=extract archive={} why={}", shown(&file), why);
                self.status = format!("Couldn't extract: {}. Nothing half-extracted was left.", why);
            }
        }
        let _ = files::sync(window);
        ticker.finish(window);
        let status = core::mem::take(&mut self.status);
        match inside_dir {
            Some(_) => self.restore_inside(arch, file, home, inside_dir),
            None => { self.load(window); self.select(base(&to)); }
        }
        self.status = status;
    }
    /// Back inside the archive at the folder being looked at, after copying out of it.
    fn restore_inside(&mut self, arch: Archive<FileSource>, file: Vec<u8>, home: Vec<u8>, dir: Option<String>) {
        let Some(dir) = dir else { return };
        let cursor = self.cursor;
        self.loom.inside = Some(Inside { arch, file, home, dir });
        self.archive_list();
        self.cursor = cursor.min(self.entries.len().saturating_sub(1));
    }

    /// Ctrl+Shift+A: packs the selected file or folder into `NAME.zip` beside it.
    pub(crate) fn make_zip(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { self.status = "Ctrl+Shift+A packs the selected file or folder into a zip: select one.".into(); return };
        let dir_item = self.entries.get(self.cursor).is_some_and(|e| e.dir);
        let name = base(&path).to_vec();
        let mut zip_name = if dir_item { name.clone() } else {
            match name.iter().rposition(|b| *b == b'.') { Some(i) if i > 0 => name[..i].to_vec(), _ => name.clone() }
        };
        zip_name.extend_from_slice(b".zip");
        let to = self.free_name(window, &self.cwd.clone(), &zip_name, "copy");
        let mut part_name = b".".to_vec();
        part_name.extend_from_slice(base(&to));
        part_name.extend_from_slice(b".part");
        let part = join(&parent(&to), &part_name);
        // Everything to pack, relative to the folder the item is in.
        let mut items: Vec<(Vec<u8>, bool, u64, u64)> = Vec::new();
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            let Ok(st) = direct::stat(&at(&p)) else { continue };
            items.push((p.clone(), st.dir, st.size, st.modified));
            if st.dir {
                let mut kids = direct::list(&at(&p)).unwrap_or_default();
                kids.retain(|e| !(e.name.first() == Some(&b'.') && e.name.ends_with(b".part")) && e.name != crate::TRASH);
                kids.sort_by(|a, b| b.name.cmp(&a.name));
                for k in kids { stack.push(join(&p, &k.name)); }
            }
        }
        let cut = parent(&path).len() + if parent(&path) == b"/" { 0 } else { 1 };
        let mut builder = ArchiveBuilder::new(OutputFormat::Zip);
        let mut files_n = 0u64;
        for (p, dir, size, mtime) in &items {
            let rel = shown(&p[cut..]);
            let r = if *dir { builder.add_dir(rel, *mtime as i64).map(|_| ()) } else {
                files_n += 1;
                match FileSource::open(p) { Ok(src) => builder.add_file(rel, *size, *mtime as i64, SourceReader::new(src)).map(|_| ()), Err(_) => Err(sex_archive::Error::Source(IoError("couldn't read a file"))) }
            };
            if let Err(e) = r { self.status = format!("Couldn't pack {}: {}.", rel, e); return; }
        }
        let _ = files::remove(window, &at(&part));
        if let Err(e) = files::create(window, &at(&part)) { self.status = format!("Couldn't make the zip: {}.", shown(files::message(e))); return; }
        sex_pdx::serial_record!("[files.app.archive.create.start] from={} to={} items={}", shown(&path), shown(&to), items.len());
        let mut sink = FileSink { path: at(&part), offset: 0 };
        let mut ticker = Ticker::new("Packing", base(&to));
        let result = {
            let mut cb = |p: Progress| ticker.tick(self, window, p);
            builder.write(&mut sink, &mut cb)
        };
        let end = match result {
            Ok(_) => match files::rename(window, &at(&part), &at(&to)) { Ok(()) => End::Done, Err(e) => End::Failed(shown(files::message(e)).into()) },
            Err(sex_archive::Error::Cancelled) => End::Stopped,
            Err(e) => End::Failed(e.to_string()),
        };
        match end {
            End::Done => {
                sex_pdx::serial_record!("[files.app.archive.created] from={} to={} files={} bytes={}", shown(&path), shown(&to), files_n, sink.offset);
                self.status = format!("Packed {} into {} ({}). Ctrl+Z takes it back.", shown(&name), shown(base(&to)), human(sink.offset));
                self.undo.push(Undo::Copied { to: to.clone() });
            }
            End::Stopped => {
                let _ = files::remove(window, &at(&part));
                sex_pdx::serial_record!("[files.app.archive.cancelled] kind=create to={}", shown(&to));
                self.status = "Stopped packing: no half-made zip was left.".into();
            }
            End::Failed(why) => {
                let _ = files::remove(window, &at(&part));
                sex_pdx::serial_record!("[files.app.archive.failed] kind=create to={} why={}", shown(&to), why);
                self.status = format!("Couldn't pack {}: {}. No half-made zip was left.", shown(&name), why);
            }
        }
        let _ = files::sync(window);
        ticker.finish(window);
        let status = core::mem::take(&mut self.status);
        self.load(window);
        self.select(base(&to));
        self.status = status;
    }
}
