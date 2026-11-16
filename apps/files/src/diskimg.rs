// SPDX-License-Identifier: GPL-3.0-only
//! Disk and CD images in Files: Enter on an `.iso`, `.img`, `.udf`, `.qcow2`, `.vhd`,
//! `.vhdx` or `.vmdk` (or a file with no such name that holds a partition table) browses
//! it read-only, as archives browse. A CD or DVD image (ISO 9660, UDF) is a folder. A
//! disk image lists one folder per partition, named like `1 · FAT32 · EFI System ·
//! 512 MiB`; one holding FAT, exFAT, ext2/3/4 or NTFS opens with the same filesystem
//! code Files mounts sticks with (`sex-volumes`), over the image, never writing to it.
//! Other filesystems (Btrfs, XFS, swap...) are listed and say they can't be opened yet.
//!
//! Ctrl+Shift+X copies the selected item out next to the image (a partition Files can't
//! read is copied out whole as a raw `.img`); on an ISO or UDF image in a folder it
//! extracts the whole disc. As with archives, everything is written under a hidden
//! `.NAME.part` name and gets its real name only when whole.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::{self, direct, Entry};
use sex_app::{Nav, Window};
use sex_archive::{Flow, Progress};
use sex_diskimage::{BlockDevice, ByteSource, EntryKind, FsKind, Image, Node};
use sex_volumes::{Disk, Volume};

use crate::archive::{mkdirs, End, FileSource, Ticker};
use crate::{at, base, join, parent, shown, App, Undo, BACKSPACE, ENTER, ESC};

/// Names Files opens as disk or CD images.
const SUFFIXES: &[&[u8]] = &[b".iso", b".img", b".udf", b".qcow2", b".qcow", b".vhd", b".vhdx", b".vmdk", b".raw", b".dsk", b".hdd"];
/// Names whose content is looked at on Enter: a raw disk dump is often called this.
const MAYBE: &[&[u8]] = &[b".bin", b".dd", b".disk", b".ima"];

fn suffix(name: &[u8]) -> Option<&'static [u8]> {
    let lower = name.to_ascii_lowercase();
    SUFFIXES.iter().copied().find(|s| lower.len() > <[u8]>::len(s) && lower.ends_with(s))
}
pub fn is_image(name: &[u8]) -> bool { suffix(name).is_some() }
/// A name that may be a raw disk image: no extension, or one raw dumps get.
fn maybe_image(name: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.contains(&b'.') || MAYBE.iter().any(|s| lower.ends_with(s))
}
/// `debian.iso` -> `debian`.
fn stem(name: &[u8]) -> &[u8] {
    match suffix(name) { Some(s) => &name[..name.len() - s.len()], None => match name.iter().rposition(|b| *b == b'.') { Some(i) if i > 0 => &name[..i], _ => name } }
}

/// `536870912` -> `512 MiB`, `1610612736` -> `1.5 GiB`.
pub fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut i = 0;
    while i + 1 < UNITS.len() && bytes >= 1u64 << (10 * (i + 1)) { i += 1; }
    let unit = 1u64 << (10 * i);
    let tenths = (bytes as u128 * 10 / unit as u128) as u64;
    if tenths % 10 == 0 { format!("{} {}", tenths / 10, UNITS[i]) } else { format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[i]) }
}

/// Filesystems `sex-volumes` reads.
fn readable(fs: FsKind) -> bool {
    matches!(fs, FsKind::Fat12 | FsKind::Fat16 | FsKind::Fat32 | FsKind::ExFat | FsKind::Ntfs | FsKind::Ext2 | FsKind::Ext3 | FsKind::Ext4)
}

/// One partition (or the whole disk, when it has no table), as a row.
pub struct Part { pub label: String, pub index: u32, pub offset: u64, pub len: u64, pub fs: FsKind }

/// A partition of an image as `sex-volumes` reads disks: whole sectors, never written.
struct PartDisk<'a> { dev: &'a BlockDevice<FileSource>, off: u64, len: u64 }
impl Disk for PartDisk<'_> {
    fn sectors(&self) -> u64 { self.len / 512 }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> sex_volumes::Result<()> {
        let at = lba.checked_mul(512).ok_or(sex_volumes::Error::Io)?;
        if at.saturating_add(buf.len() as u64) > self.len { return Err(sex_volumes::Error::Io); }
        match self.dev.read(self.off + at, buf) { Ok(n) if n == buf.len() => Ok(()), _ => Err(sex_volumes::Error::Io) }
    }
    fn write(&mut self, _: u64, _: &[u8]) -> sex_volumes::Result<()> { Err(sex_volumes::Error::ReadOnly) }
    fn flush(&mut self) -> sex_volumes::Result<()> { Ok(()) }
}

/// Where in the image Files is looking.
enum At {
    /// A disc's folders, from its root: (name, folder).
    Disc(Vec<(String, Node)>),
    /// The partitions of a disk.
    Parts,
    /// Folders inside partition `part` (an index into `parts`).
    Fs { part: usize, dir: Vec<String> },
}

/// Being inside an image.
pub struct Inside {
    img: Image<FileSource>,
    parts: Vec<Part>,
    /// The filesystem of the partition being looked at.
    vol: Option<Box<dyn Volume>>,
    at: At,
    /// The image file, and the folder it is in.
    pub file: Vec<u8>,
    pub home: Vec<u8>,
}

/// Something to copy out: where it goes (relative), and where its bytes are.
enum Src { Disc(Node), Fs(String), Raw { off: u64, len: u64 } }
struct Item { rel: String, dir: bool, size: u64, src: Src }

fn vol_err(e: sex_volumes::Error) -> String {
    use sex_volumes::Error as E;
    String::from(match e {
        E::Io => "the image couldn't be read", E::Corrupt => "the filesystem is damaged", E::NotFound => "not found",
        E::NotDir => "not a folder", E::IsDir => "a folder", E::Unsupported => "it uses a feature Files doesn't read yet",
        E::TooBig => "too big", _ => "it couldn't be read",
    })
}

impl Inside {
    fn open(src: FileSource, file: Vec<u8>) -> Result<Inside, String> {
        let img = Image::open(src).map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        let at = match &img {
            Image::Folder(v) => At::Disc(vec![(String::new(), v.root())]),
            Image::Disk(d) => {
                for p in d.partitions().map_err(|e| e.to_string())? {
                    let mut label = format!("{} · {} · {}", p.index, p.fs.label(), p.type_label);
                    if !p.name.is_empty() && p.name != p.type_label { label.push_str(&format!(" ({})", p.name.replace('/', "-"))); }
                    label.push_str(&format!(" · {}", size_label(p.len)));
                    parts.push(Part { label, index: p.index, offset: p.offset, len: p.len, fs: p.fs });
                }
                // No table: the whole disk may itself be a filesystem (a "superfloppy").
                if parts.is_empty() {
                    let fs = d.filesystem();
                    parts.push(Part { label: format!("whole disk · {} · {}", fs.label(), size_label(d.size())), index: 0, offset: 0, len: d.size(), fs });
                }
                At::Parts
            }
        };
        Ok(Inside { img, parts, vol: None, at, home: parent(&file), file })
    }
    fn kind(&self) -> &'static str { self.img.kind().label() }
    /// The folder being looked at, for the title and the records ("/" at the top).
    fn where_(&self) -> String {
        match &self.at {
            At::Disc(stack) => { let s: Vec<&str> = stack.iter().skip(1).map(|(n, _)| n.as_str()).collect(); if s.is_empty() { "/".into() } else { s.join("/") } }
            At::Parts => "/".into(),
            At::Fs { part, dir } => { let mut s = self.parts[*part].label.clone(); for d in dir { s.push('/'); s.push_str(d); } s }
        }
    }
    fn fs_path(dir: &[String], name: Option<&str>) -> String {
        let mut p = String::new();
        for d in dir.iter().map(String::as_str).chain(name) { p.push('/'); p.push_str(d); }
        if p.is_empty() { p.push('/'); }
        p
    }
    fn with_disk<T>(&mut self, part: usize, f: impl FnOnce(&mut dyn Volume, &mut dyn Disk) -> T) -> Option<T> {
        let dev = self.img.as_disk()?;
        let p = &self.parts[part];
        let mut disk = PartDisk { dev, off: p.offset, len: p.len };
        Some(f(self.vol.as_deref_mut()?, &mut disk))
    }
    fn list(&mut self) -> Result<Vec<Entry>, String> {
        let mut list: Vec<Entry> = match &self.at {
            At::Disc(stack) => {
                let v = self.img.as_volume().ok_or("not a disc")?;
                v.list_dir(&stack.last().unwrap().1).map_err(|e| e.to_string())?.into_iter()
                    .map(|e| Entry { dir: e.is_dir(), size: e.size, name: e.name.into_bytes() }).collect()
            }
            At::Parts => return Ok(self.parts.iter().map(|p| Entry { dir: true, size: p.len, name: p.label.clone().into_bytes() }).collect()),
            At::Fs { part, dir } => {
                let (part, path) = (*part, Self::fs_path(dir, None));
                self.with_disk(part, |v, d| v.list(d, path.as_bytes())).ok_or("not mounted")?.map_err(vol_err)?.into_iter()
                    .filter(|e| e.name != "." && e.name != "..")
                    .map(|e| Entry { dir: e.kind == sex_volumes::Kind::Dir, size: e.size, name: e.name.into_bytes() }).collect()
            }
        };
        list.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
        Ok(list)
    }
    /// Mounts partition `i`'s filesystem, read only.
    fn mount(&mut self, i: usize) -> Result<(), String> {
        let dev = self.img.as_disk().ok_or("not a disk")?;
        let p = &self.parts[i];
        if !readable(p.fs) { return Err(format!("it holds {}, which Files can't open yet", p.fs.label())); }
        let mut disk = PartDisk { dev, off: p.offset, len: p.len };
        self.vol = Some(sex_volumes::probe(&mut disk).map_err(vol_err)?);
        self.at = At::Fs { part: i, dir: Vec::new() };
        Ok(())
    }
    /// Everything under the item `name` of the folder being looked at, itself first.
    fn collect(&mut self, name: &str) -> Result<Vec<Item>, String> {
        let mut out = Vec::new();
        match &self.at {
            At::Disc(stack) => {
                let v = self.img.as_volume().ok_or("not a disc")?;
                let e = v.list_dir(&stack.last().unwrap().1).map_err(|e| e.to_string())?.into_iter().find(|e| e.name == name).ok_or("it's gone")?;
                let mut todo = vec![(String::from(name), e)];
                while let Some((rel, e)) = todo.pop() {
                    if out.len() > 1_000_000 { return Err("too many files".into()); }
                    match e.kind {
                        EntryKind::Dir => {
                            for k in v.list_dir(&e.node).map_err(|e| e.to_string())? { todo.push((format!("{}/{}", rel, k.name), k)); }
                            out.push(Item { rel, dir: true, size: 0, src: Src::Disc(e.node) });
                        }
                        EntryKind::File => out.push(Item { rel, dir: false, size: e.size, src: Src::Disc(e.node) }),
                        EntryKind::Symlink(_) => {}
                    }
                }
            }
            At::Parts => {
                let i = self.parts.iter().position(|p| p.label == name).ok_or("it's gone")?;
                let stem = String::from(shown(stem(base(&self.file))));
                let rel = if self.parts[i].index == 0 { format!("{} disk", stem) } else { format!("{} partition {}", stem, self.parts[i].index) };
                if readable(self.parts[i].fs) && self.mount(i).is_ok() {
                    // The partition's files, into a folder named after it.
                    let r = self.collect_fs(i, Vec::new(), rel, &mut out);
                    self.vol = None;
                    self.at = At::Parts;
                    r?;
                } else {
                    let p = &self.parts[i];
                    out.push(Item { rel: format!("{}.img", rel), dir: false, size: p.len, src: Src::Raw { off: p.offset, len: p.len } });
                }
            }
            At::Fs { part, dir } => {
                let (part, mut path) = (*part, dir.clone());
                path.push(name.into());
                self.collect_fs(part, path, name.into(), &mut out)?;
            }
        }
        // Folders before what is in them.
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    }
    fn collect_fs(&mut self, part: usize, path: Vec<String>, rel: String, out: &mut Vec<Item>) -> Result<(), String> {
        let full = Self::fs_path(&path, None);
        let st = self.with_disk(part, |v, d| v.stat(d, full.as_bytes())).ok_or("not mounted")?.map_err(vol_err)?;
        if st.kind == sex_volumes::Kind::File { out.push(Item { rel, dir: false, size: st.size, src: Src::Fs(full) }); return Ok(()); }
        if path.len() > 64 || out.len() > 1_000_000 { return Err("folders too deep".into()); }
        out.push(Item { rel: rel.clone(), dir: true, size: 0, src: Src::Fs(full.clone()) });
        let kids = self.with_disk(part, |v, d| v.list(d, full.as_bytes())).ok_or("not mounted")?.map_err(vol_err)?;
        for k in kids.into_iter().filter(|k| k.name != "." && k.name != "..") {
            let mut p = path.clone();
            p.push(k.name.clone());
            self.collect_fs(part, p, format!("{}/{}", rel, k.name), out)?;
        }
        Ok(())
    }
    fn read(&mut self, src: &Src, part: Option<usize>, offset: u64, buf: &mut [u8]) -> Result<usize, String> {
        match src {
            Src::Disc(node) => self.img.read(node, offset, buf).map_err(|e| e.to_string()),
            Src::Raw { off, len } => {
                if offset >= *len { return Ok(0); }
                let n = ((*len - offset) as usize).min(buf.len());
                self.img.as_disk().ok_or("not a disk")?.read(off + offset, &mut buf[..n]).map_err(|e| e.to_string())
            }
            Src::Fs(path) => self.with_disk(part.ok_or("not mounted")?, |v, d| v.read(d, path.as_bytes(), offset, buf)).ok_or("not mounted")?.map_err(vol_err),
        }
    }
}

impl App {
    /// Enter on a file in a folder: if it is a disk or CD image, open it. True when it was one.
    pub(crate) fn image_enter(&mut self, window: &mut Window, name: &[u8]) -> bool {
        let path = join(&self.cwd, name);
        if !is_image(name) {
            if !maybe_image(name) { return false; }
            // A raw disk by its content: a partition table, or a format that says what it is.
            let Ok(src) = FileSource::open(&path) else { return false };
            let yes = match Image::detect(&src) {
                Ok(Some(_)) => true,
                _ => src.len() >= 64 * 1024 && src.len() % 512 == 0 && BlockDevice::open(src).is_ok_and(|d| d.partitions().is_ok_and(|p| !p.is_empty())),
            };
            if !yes { return false; }
        }
        self.image_open(window, path);
        true
    }
    fn image_open(&mut self, window: &mut Window, path: Vec<u8>) {
        let name = base(&path).to_vec();
        self.status = format!("Opening {}...", shown(&name));
        self.render(window);
        let opened = FileSource::open(&path).map_err(|e| String::from(shown(files::message(e)))).and_then(|src| Inside::open(src, path.clone()));
        match opened {
            Ok(inside) => {
                sex_pdx::serial_record!("[files.app.image.opened] path={} kind={} partitions={}", shown(&path), inside.kind(), inside.parts.len());
                for p in &inside.parts { sex_pdx::serial_record!("[files.app.image.partition] index={} fs={} offset={} bytes={} name={}", p.index, p.fs.label(), p.offset, p.len, p.label); }
                self.status = format!("Inside {}, read only. Ctrl+Shift+X copies the selected item out next to it; Backspace leaves.", inside.kind());
                self.loom.image = Some(inside);
                self.image_list();
            }
            Err(why) => {
                sex_pdx::serial_record!("[files.app.image.unreadable] path={} why={}", shown(&path), why);
                self.status = format!("Can't open {} as a disk image: {}.", shown(&name), why);
            }
        }
    }
    fn image_list(&mut self) {
        let Some(inside) = self.loom.image.as_mut() else { return };
        let list = match inside.list() { Ok(l) => l, Err(why) => { self.status = format!("Can't read this folder of the image: {}.", why); Vec::new() } };
        let here = inside.where_();
        let mut cwd = inside.file.clone();
        if here != "/" { cwd.push(b'/'); cwd.extend_from_slice(here.as_bytes()); }
        sex_pdx::serial_record!("[files.app.image.list] image={} dir={} count={}", shown(&inside.file), here, list.len());
        self.entries = list;
        self.cwd = cwd;
        self.found = None;
        self.origins.clear();
        self.cursor = 0;
        self.top = 0;
    }
    fn image_leave(&mut self, window: &mut Window) {
        let Some(inside) = self.loom.image.take() else { return };
        sex_pdx::serial_record!("[files.app.image.left] path={}", shown(&inside.file));
        let name = base(&inside.file).to_vec();
        self.go(window, inside.home);
        self.select(&name);
    }
    /// Keys while inside an image. False lets Files handle the key (moving the cursor).
    pub(crate) fn image_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> bool {
        if matches!(sex_app::nav(key), Some(Nav::Up | Nav::Down | Nav::Home | Nav::End | Nav::Left | Nav::Right)) { return false; }
        if key.control && key.shift && key.code == crate::loom::KEY_X { self.image_copy_out(window, false); return true; }
        if key.control && !key.shift && matches!(key.code, crate::KEY_H | crate::KEY_D | crate::KEY_T | crate::KEY_G) { self.loom.image = None; return false; }
        match key.code {
            ESC => self.image_leave(window),
            BACKSPACE => {
                let inside = self.loom.image.as_mut().unwrap();
                let back: Option<String> = match &mut inside.at {
                    At::Disc(stack) if stack.len() > 1 => stack.pop().map(|(n, _)| n),
                    At::Fs { dir, .. } if !dir.is_empty() => dir.pop(),
                    At::Fs { part, .. } => { let l = inside.parts[*part].label.clone(); inside.vol = None; inside.at = At::Parts; Some(l) }
                    _ => None,
                };
                match back { Some(here) => { self.image_list(); self.select(here.as_bytes()); } None => self.image_leave(window) }
            }
            ENTER => {
                let Some(e) = self.entries.get(self.cursor) else { return true };
                let (name, dir, size) = (String::from(shown(&e.name)), e.dir, e.size);
                let inside = self.loom.image.as_mut().unwrap();
                if !dir { self.status = format!("{}: {} in the image. Ctrl+Shift+X copies it out next to the image.", name, crate::human(size)); return true; }
                match &mut inside.at {
                    At::Disc(stack) => {
                        let found = inside.img.list_dir(&stack.last().unwrap().1).ok().and_then(|l| l.into_iter().find(|k| k.name == name));
                        if let Some(k) = found { stack.push((name, k.node)); }
                        self.image_list();
                    }
                    At::Fs { dir, .. } => { dir.push(name); self.image_list(); }
                    At::Parts => {
                        let i = inside.parts.iter().position(|p| p.label == name).unwrap_or(0);
                        let (index, fs) = (inside.parts[i].index, inside.parts[i].fs);
                        match inside.mount(i) {
                            Ok(()) => {
                                let v = inside.vol.as_ref().unwrap();
                                sex_pdx::serial_record!("[files.app.image.mounted] image={} index={} fs={} label={}", shown(&inside.file), index, v.format().name(), v.label());
                                self.image_list();
                            }
                            Err(why) => {
                                sex_pdx::serial_record!("[files.app.image.partition.unreadable] image={} index={} fs={} why={}", shown(&inside.file), index, fs.label(), why);
                                self.status = format!("Can't open partition {}: {}. Ctrl+Shift+X copies it out whole as a .img.", index, why);
                            }
                        }
                    }
                }
            }
            _ => self.status = "This is inside a disk image, read only: Ctrl+Shift+X copies the selected item out, Backspace leaves.".into(),
        }
        true
    }

    /// Ctrl+Shift+X in a folder: a CD/DVD image is extracted whole; anything else is an
    /// archive's business.
    pub(crate) fn extract_any(&mut self, window: &mut Window) {
        let is_disc = self.entries.get(self.cursor).is_some_and(|e| !e.dir && is_image(&e.name));
        if !is_disc { return self.extract(window); }
        let path = self.selected().unwrap();
        let opened = FileSource::open(&path).map_err(|e| String::from(shown(files::message(e)))).and_then(|s| Inside::open(s, path.clone()));
        match opened {
            Ok(inside) if inside.img.as_volume().is_some() => { self.loom.image = Some(inside); self.image_copy_out(window, true); }
            Ok(_) => self.status = format!("{} is a disk: Enter it and copy a partition or its files out with Ctrl+Shift+X.", shown(base(&path))),
            Err(why) => self.status = format!("Can't open {} as a disk image: {}.", shown(base(&path)), why),
        }
    }

    /// Copies the selected item (or with `whole`, the whole disc) out next to the image.
    fn image_copy_out(&mut self, window: &mut Window, whole: bool) {
        let Some(mut inside) = self.loom.image.take() else { return };
        let selected = if whole { None } else { match self.entries.get(self.cursor) { Some(e) => Some(String::from(shown(&e.name))), None => { self.loom.image = Some(inside); return } } };
        let part = match inside.at { At::Fs { part, .. } => Some(part), _ => None };
        let items = match &selected {
            Some(name) => inside.collect(name),
            None => {
                // The disc's root folder's contents, under a folder named after the image.
                let top = String::from(shown(stem(base(&inside.file))));
                let mut all = vec![Item { rel: top.clone(), dir: true, size: 0, src: Src::Disc(inside.img.as_volume().unwrap().root()) }];
                let names: Vec<String> = inside.list().map(|l| l.into_iter().map(|e| String::from(shown(&e.name))).collect()).unwrap_or_default();
                let mut r = Ok(());
                for n in names { match inside.collect(&n) { Ok(v) => all.extend(v.into_iter().map(|mut i| { i.rel = format!("{}/{}", top, i.rel); i })), Err(e) => { r = Err(e); break; } } }
                r.map(|_| all)
            }
        };
        let member = selected.clone().unwrap_or_else(|| "*".into());
        let items = match items {
            Ok(i) if !i.is_empty() => i,
            Ok(_) => { self.status = "Nothing to copy out.".into(); return self.image_restore(inside, whole); }
            Err(why) => { self.status = format!("Couldn't copy out: {}.", why); return self.image_restore(inside, whole); }
        };
        let home = inside.home.clone();
        let top_rel = items[0].rel.clone();
        let to = self.free_name(window, &home, top_rel.as_bytes(), if whole { "extracted" } else { "copy" });
        let mut part_name = b".".to_vec();
        part_name.extend_from_slice(base(&to));
        part_name.extend_from_slice(b".part");
        let part_path = join(&home, &part_name);
        let _ = self.remove_all(window, &part_path);
        let total: u64 = items.iter().map(|i| i.size).sum();
        sex_pdx::serial_record!("[files.app.image.copy.start] image={} member={} to={} items={} bytes={}", shown(&inside.file), member, shown(&to), items.len(), total);
        let mut ticker = Ticker::new(if whole { "Extracting" } else { "Copying out" }, base(&to));
        let (mut done, mut files_n) = (0u64, 0u64);
        let mut made = alloc::collections::BTreeSet::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut end = End::Done;
        'all: for (n, item) in items.iter().enumerate() {
            // The first item is the top: it is written as the `.part` itself.
            let dest = if n == 0 { part_path.clone() } else { join(&part_path, item.rel[top_rel.len() + 1..].as_bytes()) };
            if item.dir {
                if n == 0 { if let Err(e) = direct::mkdir(&at(&dest)) { end = End::Failed(shown(files::message(e)).into()); break; } }
                else if mkdirs(&part_path, &item.rel[top_rel.len() + 1..], &mut made).is_err() { end = End::Failed("couldn't make a folder".into()); break; }
                continue;
            }
            if direct::create(&at(&dest)).is_err() && direct::truncate(&at(&dest), 0).is_err() { end = End::Failed("couldn't make a file".into()); break; }
            files_n += 1;
            let mut off = 0u64;
            while off < item.size {
                let want = ((item.size - off) as usize).min(buf.len());
                let got = match inside.read(&item.src, part, off, &mut buf[..want]) { Ok(0) => { end = End::Failed(format!("{} ends early", item.rel)); break 'all; } Ok(k) => k, Err(why) => { end = End::Failed(why); break 'all; } };
                match direct::write_at(&at(&dest), off, &buf[..got]) {
                    Ok(w) if w == got => {}
                    Ok(_) => { end = End::Failed("the disk is full".into()); break 'all; }
                    Err(e) => { end = End::Failed(shown(files::message(e)).into()); break 'all; }
                }
                off += got as u64;
                done += got as u64;
                if let Flow::Cancel = ticker.tick(self, window, Progress { done, total }) { end = End::Stopped; break 'all; }
            }
        }
        if let End::Done = end {
            if let Err(e) = files::rename(window, &at(&part_path), &at(&to)) { end = End::Failed(shown(files::message(e)).into()); }
        }
        let _ = files::sync(window);
        match end {
            End::Done => {
                sex_pdx::serial_record!("[files.app.image.copied] image={} member={} to={} files={} bytes={}", shown(&inside.file), member, shown(&to), files_n, done);
                self.status = format!("Copied out {} ({} files, {}). Ctrl+Z takes it back.", shown(base(&to)), files_n, crate::human(done));
                self.undo.push(Undo::Copied { to: to.clone() });
            }
            End::Stopped => {
                let _ = self.remove_all(window, &part_path);
                sex_pdx::serial_record!("[files.app.image.cancelled] image={} member={}", shown(&inside.file), member);
                self.status = "Stopped copying out: nothing half-copied was left.".into();
            }
            End::Failed(why) => {
                let _ = self.remove_all(window, &part_path);
                sex_pdx::serial_record!("[files.app.image.failed] image={} member={} why={}", shown(&inside.file), member, why);
                self.status = format!("Couldn't copy out: {}. Nothing half-copied was left.", why);
            }
        }
        let _ = files::sync(window);
        ticker.finish(window);
        let status = core::mem::take(&mut self.status);
        if whole { self.load(window); self.select(base(&to)); } else { self.image_restore(inside, false); }
        self.status = status;
    }
    fn image_restore(&mut self, inside: Inside, whole: bool) {
        if whole { return; }
        let cursor = self.cursor;
        self.loom.image = Some(inside);
        self.image_list();
        self.cursor = cursor.min(self.entries.len().saturating_sub(1));
    }
}
