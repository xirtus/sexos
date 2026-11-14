//! Creating archives: zip (deflate), tar, tar.gz and tar.zst.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::codec::{DeflateEncoder, GzEncoder};
use crate::io::{Input, IoError, ItemReader, Sink};
use crate::path::sanitize_path;
use crate::zip::{central_record, data_descriptor, end_records, local_header, CdRecord};
use crate::{tar, EntryKind, Error, Flow, Progress, ProgressFn, Result};

/// Formats [`ArchiveBuilder`] can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// zip with deflate, data descriptors, UTF-8 names, zip64 when needed.
    Zip,
    /// POSIX ustar with pax extensions for long names and huge sizes.
    Tar,
    TarGz,
    /// tar in one zstd frame (ruzstd "fastest" level, with checksum).
    TarZst,
}

struct Item<'a> {
    path: String,
    kind: EntryKind,
    size: u64,
    mtime: i64,
    mode: u32,
    reader: Option<Box<dyn ItemReader + 'a>>,
}

/// Collects (path, reader) items, then streams an archive into a sink.
///
/// Readers are only read during [`ArchiveBuilder::write`], one at a time,
/// so they can open files lazily. Each file's `size` must be exact: tar
/// needs it up front, and a reader that yields a different count fails the
/// write with [`Error::SizeMismatch`].
pub struct ArchiveBuilder<'a> {
    format: OutputFormat,
    items: Vec<Item<'a>>,
    level: u8,
    force_zip64: bool,
}

const CHUNK: usize = 64 * 1024;

impl<'a> ArchiveBuilder<'a> {
    pub fn new(format: OutputFormat) -> Self {
        Self { format, items: Vec::new(), level: 6, force_zip64: false }
    }

    /// Deflate level 0..=10 for zip and tar.gz (default 6).
    pub fn deflate_level(&mut self, level: u8) -> &mut Self {
        self.level = level.min(10);
        self
    }

    /// Write zip64 records even when not needed (interop testing).
    pub fn force_zip64(&mut self, on: bool) -> &mut Self {
        self.force_zip64 = on;
        self
    }

    fn clean(path: &str) -> Result<String> {
        let p = sanitize_path(path)?;
        if p.is_empty() {
            return Err(Error::InvalidName(String::from(path)));
        }
        Ok(p)
    }

    /// Add a folder. `mtime` is Unix seconds.
    pub fn add_dir(&mut self, path: &str, mtime: i64) -> Result<&mut Self> {
        let path = Self::clean(path)?;
        self.items.push(Item { path, kind: EntryKind::Dir, size: 0, mtime, mode: 0o755, reader: None });
        Ok(self)
    }

    /// Add a file of exactly `size` bytes read from `reader`.
    pub fn add_file(&mut self, path: &str, size: u64, mtime: i64, reader: impl ItemReader + 'a) -> Result<&mut Self> {
        self.add_file_mode(path, size, mtime, 0o644, reader)
    }

    /// Like [`ArchiveBuilder::add_file`] with explicit permission bits.
    pub fn add_file_mode(
        &mut self,
        path: &str,
        size: u64,
        mtime: i64,
        mode: u32,
        reader: impl ItemReader + 'a,
    ) -> Result<&mut Self> {
        let path = Self::clean(path)?;
        self.items.push(Item { path, kind: EntryKind::File, size, mtime, mode: mode & 0o7777, reader: Some(Box::new(reader)) });
        Ok(self)
    }

    /// Total bytes of file data to be archived (the progress total).
    pub fn total_size(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    /// Stream the archive into `sink`. Returns the number of bytes written.
    /// On [`Error::Cancelled`] the sink holds an incomplete archive.
    pub fn write(mut self, sink: &mut dyn Sink, progress: ProgressFn<'_>) -> Result<u64> {
        let total = self.total_size();
        let mut cs = CountSink { inner: sink, n: 0 };
        match self.format {
            OutputFormat::Zip => write_zip(&mut self.items, &mut cs, self.level, self.force_zip64, total, progress)?,
            OutputFormat::Tar => {
                let mut ts = TarStream::new(&mut self.items, total, progress);
                let mut buf = vec![0u8; CHUNK];
                loop {
                    let n = ts.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    cs.write_all(&buf[..n]).map_err(Error::Sink)?;
                }
            }
            OutputFormat::TarGz => {
                let mut ts = TarStream::new(&mut self.items, total, progress);
                let mut gz = GzEncoder::new(&mut cs)?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    let n = ts.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    gz.write(&buf[..n])?;
                }
                gz.finish()?;
            }
            OutputFormat::TarZst => {
                let ts = TarStream::new(&mut self.items, total, progress);
                let mut src = ZSrc { inner: ts, err: None };
                let mut drain = ZDrain { inner: &mut cs, err: None };
                {
                    let mut fc = ruzstd::encoding::FrameCompressor::new(ruzstd::encoding::CompressionLevel::Fastest);
                    fc.set_source(&mut src);
                    fc.set_drain(&mut drain);
                    fc.compress();
                }
                if let Some(e) = src.err.take() {
                    return Err(e);
                }
                if let Some(e) = drain.err.take() {
                    return Err(Error::Sink(e));
                }
            }
        }
        Ok(cs.n)
    }
}

struct CountSink<'s> {
    inner: &'s mut dyn Sink,
    n: u64,
}

impl Sink for CountSink<'_> {
    fn write_all(&mut self, data: &[u8]) -> core::result::Result<(), IoError> {
        self.inner.write_all(data)?;
        self.n += data.len() as u64;
        Ok(())
    }
}

fn tick(progress: &mut dyn FnMut(Progress) -> Flow, done: u64, total: u64) -> Result<()> {
    if progress(Progress { done, total }) == Flow::Cancel {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Read exactly `size` bytes from an item, handing chunks to `f`.
fn read_item(
    reader: &mut dyn ItemReader,
    size: u64,
    done: &mut u64,
    total: u64,
    progress: &mut dyn FnMut(Progress) -> Flow,
    f: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let mut buf = vec![0u8; CHUNK.min(size as usize + 1)];
    let mut left = size;
    loop {
        tick(progress, *done, total)?;
        let want = (left.min(buf.len() as u64) as usize).max(1);
        let n = reader.read(&mut buf[..want]).map_err(Error::Source)?;
        if n == 0 {
            return if left == 0 { Ok(()) } else { Err(Error::SizeMismatch) };
        }
        if n as u64 > left {
            return Err(Error::SizeMismatch);
        }
        f(&buf[..n])?;
        left -= n as u64;
        *done += n as u64;
    }
}

fn write_zip(
    items: &mut [Item<'_>],
    sink: &mut CountSink<'_>,
    level: u8,
    force64: bool,
    total: u64,
    progress: &mut dyn FnMut(Progress) -> Flow,
) -> Result<()> {
    let mut cd: Vec<CdRecord> = Vec::with_capacity(items.len());
    let mut done = 0u64;
    for it in items.iter_mut() {
        let is_dir = it.kind == EntryKind::Dir;
        let mut name = it.path.clone().into_bytes();
        if is_dir {
            name.push(b'/');
        }
        if name.len() > 0xFFFF {
            return Err(Error::InvalidName(it.path.clone()));
        }
        let offset = sink.n;
        let zip64 = !is_dir && (force64 || it.size >= 0xFFFF_0000);
        let method = if is_dir { 0 } else { 8 };
        sink.write_all(&local_header(&name, it.mtime, method, zip64, is_dir)).map_err(Error::Sink)?;
        let mut rec = CdRecord {
            name,
            is_dir,
            mtime: it.mtime,
            mode: it.mode,
            method,
            crc: 0,
            csize: 0,
            usize: 0,
            offset,
            zip64_local: zip64,
        };
        if let Some(reader) = it.reader.as_mut() {
            let mut enc = DeflateEncoder::new(level);
            let mut crc = crc32fast::Hasher::new();
            read_item(&mut **reader, it.size, &mut done, total, progress, &mut |b| {
                crc.update(b);
                enc.write(b, sink)
            })?;
            enc.finish(sink)?;
            rec.crc = crc.finalize();
            rec.csize = enc.compressed;
            rec.usize = it.size;
            if !zip64 && (rec.csize >= 0xFFFF_FFFF || rec.usize >= 0xFFFF_FFFF) {
                return Err(Error::TooLarge);
            }
            sink.write_all(&data_descriptor(rec.crc, rec.csize, rec.usize, zip64)).map_err(Error::Sink)?;
        }
        cd.push(rec);
    }
    tick(progress, done, total)?;
    let cd_offset = sink.n;
    for r in &cd {
        sink.write_all(&central_record(r, force64)).map_err(Error::Sink)?;
    }
    let cd_size = sink.n - cd_offset;
    sink.write_all(&end_records(cd.len() as u64, cd_offset, cd_size, force64)).map_err(Error::Sink)?;
    Ok(())
}

/// Pull-style tar byte stream over builder items.
struct TarStream<'b, 'a, 'p> {
    items: &'b mut [Item<'a>],
    next: usize,
    pending: Vec<u8>,
    ppos: usize,
    /// Item whose data is being streamed and bytes left.
    cur: Option<usize>,
    left: u64,
    done: u64,
    total: u64,
    progress: &'p mut dyn FnMut(Progress) -> Flow,
    finished: bool,
}

impl<'b, 'a, 'p> TarStream<'b, 'a, 'p> {
    fn new(items: &'b mut [Item<'a>], total: u64, progress: &'p mut dyn FnMut(Progress) -> Flow) -> Self {
        Self { items, next: 0, pending: Vec::new(), ppos: 0, cur: None, left: 0, done: 0, total, progress, finished: false }
    }
}

impl Input for TarStream<'_, '_, '_> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.ppos < self.pending.len() {
                let n = (self.pending.len() - self.ppos).min(out.len());
                out[..n].copy_from_slice(&self.pending[self.ppos..self.ppos + n]);
                self.ppos += n;
                return Ok(n);
            }
            if let Some(i) = self.cur {
                let reader = self.items[i].reader.as_mut().ok_or(Error::SizeMismatch)?;
                if self.left > 0 {
                    tick(self.progress, self.done, self.total)?;
                    let want = self.left.min(out.len() as u64) as usize;
                    let n = reader.read(&mut out[..want]).map_err(Error::Source)?;
                    if n == 0 {
                        return Err(Error::SizeMismatch);
                    }
                    self.left -= n as u64;
                    self.done += n as u64;
                    return Ok(n);
                }
                let mut probe = [0u8; 1];
                if reader.read(&mut probe).map_err(Error::Source)? != 0 {
                    return Err(Error::SizeMismatch);
                }
                self.items[i].reader = None; // close it early
                self.cur = None;
                self.pending = vec![0u8; tar::padding(self.items[i].size)];
                self.ppos = 0;
                continue;
            }
            if self.next < self.items.len() {
                let i = self.next;
                self.next += 1;
                let it = &self.items[i];
                self.pending = tar::member_headers(&it.path, it.kind, it.size, it.mtime, it.mode);
                self.ppos = 0;
                if it.kind == EntryKind::File {
                    self.cur = Some(i);
                    self.left = it.size;
                }
                continue;
            }
            if !self.finished {
                self.finished = true;
                tick(self.progress, self.done, self.total)?;
                self.pending = vec![0u8; 1024];
                self.ppos = 0;
                continue;
            }
            return Ok(0);
        }
    }
}

struct ZSrc<I> {
    inner: I,
    err: Option<Error>,
}

impl<I: Input> ruzstd::io::Read for ZSrc<I> {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, ruzstd::io::Error> {
        if self.err.is_some() {
            return Ok(0);
        }
        match self.inner.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                // The compressor cannot take errors; end the stream and
                // report after it returns.
                self.err = Some(e);
                Ok(0)
            }
        }
    }
}

struct ZDrain<'s, S: Sink + ?Sized> {
    inner: &'s mut S,
    err: Option<IoError>,
}

impl<S: Sink + ?Sized> ruzstd::io::Write for ZDrain<'_, S> {
    fn write(&mut self, buf: &[u8]) -> core::result::Result<usize, ruzstd::io::Error> {
        if self.err.is_none() {
            if let Err(e) = self.inner.write_all(buf) {
                self.err = Some(e);
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> core::result::Result<(), ruzstd::io::Error> {
        Ok(())
    }
}
