// SPDX-License-Identifier: GPL-3.0-only
//! Other systems' disks under `/media`: FAT, exFAT, ext4 and NTFS volumes, through
//! `sex-volumes`. A volume is mounted from a disk image kept on the SexOS filesystem (a
//! loop mount: how a stick's contents arrive today, and how the gates test it); USB mass
//! storage and second disks become one more kind of [`Source`].
//!
//! When the disk itself holds no SexOS filesystem (an Intel RAID volume sexdrive serves
//! read only, usually Windows), its own partitions are mounted here instead, read only:
//! [`attach_device`].
//!
//! The image file's writes are ordinary filesystem writes, batched like any other and
//! committed together, so the copy-on-write filesystem underneath keeps a whole image at
//! every commit. Nothing here holds the filesystem: each request lends it.
use crate::fs_service::Disk as FsDisk;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_pdx::fs as w;
use sex_volumes::{self as sv, Volume};
use sexfs::{Fs, Kind};

pub const MEDIA: &[u8] = b"/media";
const NAME_MAX: usize = 32;
/// The most volumes mounted at once.
const MOUNTS_MAX: usize = 16;

enum Source {
    /// A file of the SexOS filesystem, by inode, `sectors` long.
    Image { id: u64, sectors: u64 },
    /// The block device sexdrive serves, when it holds no SexOS filesystem. Read only.
    Device { sectors: u64 },
}
struct Mount { name: String, source: Source, start: u64, len: u64, volume: Box<dyn Volume> }
impl Mount { fn read_only(&self) -> bool { matches!(self.source, Source::Device { .. }) } }
static mut MOUNTS: Vec<Mount> = Vec::new();
fn mounts() -> &'static mut Vec<Mount> { unsafe { &mut *core::ptr::addr_of_mut!(MOUNTS) } }

/// A file of the SexOS filesystem as a disk.
struct ImageDisk<'a> { fs: &'a mut Fs<'static, FsDisk>, id: u64, sectors: u64 }
impl sv::Disk for ImageDisk<'_> {
    fn sectors(&self) -> u64 { self.sectors }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> sv::Result<()> {
        let mut done = 0;
        while done < buf.len() {
            match self.fs.read(self.id, lba * 512 + done as u64, &mut buf[done..]) {
                // Past the file's end, as on a disk that was never written.
                Ok(0) => { buf[done..].fill(0); break; }
                Ok(n) => done += n,
                Err(_) => return Err(sv::Error::Io),
            }
        }
        Ok(())
    }
    fn write(&mut self, lba: u64, buf: &[u8]) -> sv::Result<()> {
        let mut done = 0;
        while done < buf.len() {
            match self.fs.write(self.id, lba * 512 + done as u64, &buf[done..], 0) {
                Ok(0) => return Err(sv::Error::NoSpace),
                Ok(n) => done += n,
                Err(sexfs::Error::NoSpace) => return Err(sv::Error::NoSpace),
                Err(_) => return Err(sv::Error::Io),
            }
        }
        crate::fs_service::wrote(buf.len() as u64);
        Ok(())
    }
    fn flush(&mut self) -> sv::Result<()> { Ok(()) }
}

/// The block device itself, by absolute sector, through Files' shared block buffer.
/// Nothing is ever written through it.
struct DeviceDisk { sectors: u64 }
impl sv::Disk for DeviceDisk {
    fn sectors(&self) -> u64 { self.sectors }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> sv::Result<()> {
        use crate::backends::diskfs::DiskFs;
        let va = crate::vfs::diskfs_bridge_get_buf_va();
        let room = crate::vfs::diskfs_bridge_buf_len() as usize / sv::SECTOR * sv::SECTOR;
        if va == 0 || va == u64::MAX || room == 0 { return Err(sv::Error::Io); }
        if lba.checked_add((buf.len() / sv::SECTOR) as u64).is_none_or(|end| end > self.sectors) { return Err(sv::Error::Io); }
        let mut done = 0;
        while done < buf.len() {
            let n = (buf.len() - done).min(room);
            if DiskFs::diskfs_block_read(lba * sv::SECTOR as u64 + done as u64, n as u64, sex_pdx::SLOT_BUF_LEND) != 0 { return Err(sv::Error::Io); }
            let shared = unsafe { core::slice::from_raw_parts(va as *const u8, n) };
            buf[done..done + n].copy_from_slice(shared);
            done += n;
        }
        Ok(())
    }
    fn write(&mut self, _: u64, _: &[u8]) -> sv::Result<()> { Err(sv::Error::ReadOnly) }
    fn flush(&mut self) -> sv::Result<()> { Ok(()) }
}

/// Runs `op` with a mount's volume and its disk. An image needs the SexOS filesystem.
fn with<T>(fs: Option<&mut Fs<'static, FsDisk>>, m: &mut Mount, op: impl FnOnce(&mut dyn Volume, &mut dyn sv::Disk) -> sv::Result<T>) -> sv::Result<T> {
    match m.source {
        Source::Image { id, sectors } => {
            let Some(fs) = fs else { return Err(sv::Error::Io) };
            let mut image = ImageDisk { fs, id, sectors };
            let mut slice = sv::Slice { disk: &mut image, start: m.start, len: m.len };
            op(m.volume.as_mut(), &mut slice)
        }
        Source::Device { sectors } => {
            let mut device = DeviceDisk { sectors };
            let mut slice = sv::Slice { disk: &mut device, start: m.start, len: m.len };
            op(m.volume.as_mut(), &mut slice)
        }
    }
}

pub fn status(e: sv::Error) -> u8 {
    use sv::Error as E;
    match e {
        E::Io => w::IO, E::Corrupt => w::CORRUPT, E::NotFound => w::NOT_FOUND, E::Exists => w::EXISTS, E::NotDir => w::NOT_DIR,
        E::IsDir => w::IS_DIR, E::NotEmpty => w::NOT_EMPTY, E::Name | E::TooBig => w::NAME, E::NoSpace => w::NO_SPACE,
        E::ReadOnly => w::READ_ONLY, E::Unsupported => w::UNSUPPORTED,
    }
}

/// `/media` itself or a path under it.
pub fn owns(path: &[u8]) -> bool { path == MEDIA || path.starts_with(b"/media/") }
/// The mount a `/media/NAME/...` path names, and the rest of the path.
fn find(path: &[u8]) -> Option<(usize, &[u8])> {
    let rest = path.strip_prefix(b"/media/")?;
    let (name, inner) = match rest.iter().position(|b| *b == b'/') { Some(i) => (&rest[..i], &rest[i..]), None => (rest, &rest[rest.len()..]) };
    let index = mounts().iter().position(|m| m.name.as_bytes() == name)?;
    Some((index, inner))
}
/// The image a change through the SexOS filesystem would touch while it is mounted.
pub fn busy(id: u64) -> bool { mounts().iter().any(|m| matches!(m.source, Source::Image { id: i, .. } if i == id)) }

/// One request on a path under `/media`.
pub fn run(fs: Option<&mut Fs<'static, FsDisk>>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, u8> {
    if path == MEDIA || path == b"/media/" {
        return match r.kind {
            w::STAT => { data[..32].fill(0); data[0] = 1; Ok(32) }
            w::LIST => {
                let entries: Vec<(String, bool, u64)> = mounts().iter().map(|m| (m.name.clone(), true, 0)).collect();
                Ok(pack(&entries, r.offset, data))
            }
            w::STATFS => Ok(statfs_out(0, 0, data)),
            _ => Err(w::DENIED),
        };
    }
    let Some((index, inner)) = find(path) else { return Err(w::NOT_FOUND) };
    let m = &mut mounts()[index];
    if m.read_only() && !matches!(r.kind, w::STAT | w::LIST | w::READ | w::STATFS | w::SYNC) { return Err(w::READ_ONLY); }
    if m.read_only() && r.kind == w::SYNC { return Ok(0); }
    let result = match r.kind {
        w::STAT => with(fs, m, |v, d| v.stat(d, inner)).map(|st| {
            data[..32].fill(0);
            data[..8].copy_from_slice(&((st.kind == sv::Kind::Dir) as u64).to_le_bytes());
            data[8..16].copy_from_slice(&st.size.to_le_bytes());
            data[16..24].copy_from_slice(&st.mtime.to_le_bytes());
            32
        }),
        w::LIST => with(fs, m, |v, d| v.list(d, inner)).map(|list| {
            let entries: Vec<(String, bool, u64)> = list.into_iter().map(|e| (e.name, e.kind == sv::Kind::Dir, e.size)).collect();
            pack(&entries, r.offset, data)
        }),
        w::READ => {
            let want = (r.length as usize).min(data.len()).min(w::IO_MAX);
            let mut own = vec![0u8; want];
            with(fs, m, |v, d| v.read(d, inner, r.offset, &mut own)).map(|n| { data[..n].copy_from_slice(&own[..n]); n as u32 })
        }
        // The client can change its region at any time: copy before the volume sees it.
        w::WRITE => {
            let total = r.length as usize;
            if total > data.len() || total > w::IO_MAX { return Err(w::NAME); }
            let own = data[..total].to_vec();
            with(fs, m, |v, d| v.write(d, inner, r.offset, &own)).map(|n| n as u32)
        }
        w::CREATE => with(fs, m, |v, d| v.create(d, inner)).map(|_| 0),
        w::MKDIR => with(fs, m, |v, d| v.mkdir(d, inner)).map(|_| 0),
        w::REMOVE if inner.iter().all(|b| *b == b'/') => return Err(w::DENIED),
        w::REMOVE => with(fs, m, |v, d| v.remove(d, inner)).map(|_| 0),
        w::TRUNCATE => with(fs, m, |v, d| v.truncate(d, inner, r.offset)).map(|_| 0),
        w::SYNC => with(fs, m, |v, d| v.sync(d)).map(|_| 0),
        w::STATFS => with(fs, m, |v, d| v.statfs(d)).map(|(total, free)| statfs_out(total / 4096, free / 4096, data)),
        _ => return Err(w::INVALID),
    };
    result.map_err(status)
}
/// A rename with both halves under `/media`: only within one volume.
pub fn rename(fs: Option<&mut Fs<'static, FsDisk>>, from: &[u8], to: &[u8]) -> Result<u32, u8> {
    let (Some((a, inner_from)), Some((b, inner_to))) = (find(from), find(to)) else { return Err(if owns(from) && owns(to) { w::NOT_FOUND } else { w::OTHER_DISK }) };
    if a != b { return Err(w::OTHER_DISK); }
    if inner_from.iter().all(|c| *c == b'/') { return Err(w::DENIED); }
    if mounts()[a].read_only() { return Err(w::READ_ONLY); }
    with(fs, &mut mounts()[a], |v, d| v.rename(d, inner_from, inner_to)).map(|_| 0).map_err(status)
}
fn statfs_out(total: u64, free: u64, data: &mut [u8]) -> u32 {
    data[..24].fill(0);
    data[..8].copy_from_slice(&total.to_le_bytes());
    data[8..16].copy_from_slice(&free.to_le_bytes());
    24
}
/// LIST's packing (see `sex_pdx::fs`), resuming at entry `cursor`.
fn pack(entries: &[(String, bool, u64)], cursor: u64, data: &mut [u8]) -> u32 {
    let mut used = w::LIST_HEAD;
    let mut next = 0u64;
    for (i, (name, dir, size)) in entries.iter().enumerate().skip(cursor as usize) {
        let name = &name.as_bytes()[..name.len().min(255)];
        if used + w::ENTRY_HEAD + name.len() > 4000 { next = i as u64; break; }
        data[used] = *dir as u8;
        data[used + 1] = name.len() as u8;
        data[used + 2..used + 10].copy_from_slice(&size.to_le_bytes());
        data[used + w::ENTRY_HEAD..used + w::ENTRY_HEAD + name.len()].copy_from_slice(name);
        used += w::ENTRY_HEAD + name.len();
    }
    data[..8].copy_from_slice(&next.to_le_bytes());
    used as u32
}

/// A name usable as one path part: the volume's label, else the image's name, made unique.
fn mount_name(label: &str, image: &[u8], part: usize) -> String {
    let base_name = image.rsplit(|b| *b == b'/').next().unwrap_or(image);
    let stem = match base_name.iter().rposition(|b| *b == b'.') { Some(i) if i > 0 => &base_name[..i], _ => base_name };
    let mut name = String::new();
    let source: String = if label.trim().is_empty() { String::from_utf8_lossy(stem).into_owned() } else { String::from(label.trim()) };
    for c in source.chars() {
        if name.len() + c.len_utf8() > NAME_MAX - 4 { break; }
        name.push(if c == '/' || (c as u32) < 0x20 { '_' } else { c });
    }
    if name.is_empty() { name.push_str("disk"); }
    if part > 0 { name.push('-'); name.push((b'1' + part as u8) as char); }
    let taken = |n: &str| mounts().iter().any(|m| m.name == n);
    if !taken(&name) { return name; }
    for k in 2..100u32 {
        let mut candidate = name.clone();
        candidate.push(' ');
        let _ = core::fmt::Write::write_fmt(&mut candidate, format_args!("{}", k));
        if !taken(&candidate) { return candidate; }
    }
    name
}

/// Mounts every volume found in an image file. The data area gets their names, one per
/// line with format and size; the reply's value is that text's length.
pub fn mount(fs: &mut Fs<'static, FsDisk>, path: &[u8], data: &mut [u8]) -> Result<u32, u8> {
    let st = fs.open(path).map_err(crate::fs_service::status)?;
    if st.kind == Kind::Dir { return Err(w::IS_DIR); }
    if busy(st.id) { return Err(w::EXISTS); }
    let sectors = st.size / 512;
    if sectors == 0 { return Err(w::UNSUPPORTED); }
    let mut image = ImageDisk { fs, id: st.id, sectors };
    let found = sv::scan(&mut image).map_err(status)?;
    if found.is_empty() { return Err(w::UNSUPPORTED); }
    if mounts().len() + found.len() > MOUNTS_MAX { return Err(w::NO_SPACE); }
    let mut out = String::new();
    for (part, f) in found.into_iter().enumerate() {
        let name = mount_name(&f.volume.label(), path, part);
        let _ = core::fmt::Write::write_fmt(&mut out, format_args!("{} {} {} MiB{}\n", name, f.volume.format().name(), f.len / 2048, if f.volume.read_only() { " read-only" } else { "" }));
        sex_pdx::serial_record!("[files.media.mounted] name={} format={} sectors={} start={} read_only={}", name, f.volume.format().name(), f.len, f.start, f.volume.read_only() as u8);
        mounts().push(Mount { name, source: Source::Image { id: st.id, sectors }, start: f.start, len: f.len, volume: f.volume });
    }
    let n = out.len().min(data.len());
    data[..n].copy_from_slice(&out.as_bytes()[..n]);
    Ok(n as u32)
}
/// Writes out what a volume holds back and forgets it. The image stays where it was.
pub fn unmount(fs: &mut Fs<'static, FsDisk>, path: &[u8]) -> Result<u32, u8> {
    let Some((index, inner)) = find(path) else { return Err(w::NOT_FOUND) };
    if !inner.iter().all(|b| *b == b'/') { return Err(w::INVALID); }
    if !mounts()[index].read_only() { with(Some(fs), &mut mounts()[index], |v, d| v.sync(d)).map_err(status)?; }
    let m = mounts().remove(index);
    sex_pdx::serial_record!("[files.media.unmounted] name={}", m.name);
    Ok(0)
}
/// SYNC with no path reaches every volume too.
pub fn sync_all(fs: &mut Fs<'static, FsDisk>) {
    for m in mounts().iter_mut().filter(|m| !m.read_only()) {
        if let Err(e) = with(Some(&mut *fs), m, |v, d| v.sync(d)) { sex_pdx::serial_record!("[files.media.sync.failed] name={} error={:?}", m.name, e); }
    }
}

/// The disk holds no SexOS filesystem: mounts every volume on it, read only, under
/// `/media` (the partitions of a Windows RAID volume, say). Named by label, else "disk".
pub fn attach_device(sectors: u64) {
    if mounts().iter().any(|m| m.read_only()) { return; }
    let mut device = DeviceDisk { sectors };
    let found = match sv::scan(&mut device) {
        Ok(found) => found,
        Err(e) => { sex_pdx::serial_record!("[files.media.device.none] sectors={} error={:?}", sectors, e); return; }
    };
    sex_pdx::serial_record!("[files.media.device] sectors={} volumes={}", sectors, found.len());
    for (part, f) in found.into_iter().enumerate() {
        if mounts().len() >= MOUNTS_MAX { break; }
        let name = mount_name(&f.volume.label(), b"disk", part);
        sex_pdx::serial_record!("[files.media.mounted] name={} format={} sectors={} start={} read_only=1", name, f.volume.format().name(), f.len, f.start);
        mounts().push(Mount { name, source: Source::Device { sectors }, start: f.start, len: f.len, volume: f.volume });
    }
}
