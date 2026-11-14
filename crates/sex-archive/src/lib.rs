//! Archive browsing, extraction and creation for SexOS programs.
//!
//! `#![no_std]` + `alloc`. Reads zip (stored, deflate, zstd, xz, lzma;
//! zip64; UTF-8 and CP437 names), tar (ustar, GNU long names, pax), tar
//! wrapped in gzip / zstd / xz, single-file `.gz` / `.zst` / `.xz` and 7z
//! (copy, LZMA, LZMA2 and deflate folders, including compressed headers).
//! Writes zip (deflate), tar, tar.gz and tar.zst.
//!
//! An [`Archive`] reads through a random-access [`ByteSource`], so a file
//! manager can back it with its file handle without loading the archive.
//! Archives inside a compressed stream (tar.gz and friends) cannot be
//! seeked: opening scans the stream once, and each [`Archive::extract`]
//! decompresses from the start again. Use [`Archive::extract_all`] to pull
//! many members out in one pass.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod codec;
mod io;
mod path;
mod sevenz;
mod tar;
mod write;
mod xz;
mod zip;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use codec::{GzDecoder, ZstdDecoder};
use xz::XzDecoder;
pub use io::{ByteSource, IoError, ItemReader, NullSink, Sink, SourceReader, VecReader};
#[cfg(feature = "std")]
pub use io::{StdItem, StdSink, StdSource};
use io::{Input, SrcReader, Take};
pub use path::{sanitize_path, DirItem};
pub use write::{ArchiveBuilder, OutputFormat};

/// Everything that can go wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The byte source or an item reader failed.
    Source(IoError),
    /// The sink failed.
    Sink(IoError),
    /// The bytes are not an archive format this crate knows.
    UnknownFormat,
    /// A known format or feature this crate does not implement.
    Unsupported(&'static str),
    /// The archive is damaged.
    Corrupt(&'static str),
    /// A checksum or length check failed after decompression.
    Checksum,
    /// The member is encrypted.
    Encrypted,
    /// The member's path would escape the extraction folder.
    UnsafePath(String),
    /// No member at that index or path.
    NoSuchEntry,
    /// The member is not a regular file (folder, symlink, device...).
    NotAFile,
    /// `list_dir` was given a path that is a file.
    NotADirectory,
    /// A size or count does not fit (e.g. memory limit of a decoder).
    TooLarge,
    /// An item reader produced a different number of bytes than declared.
    SizeMismatch,
    /// A name cannot be stored in the chosen format.
    InvalidName(String),
    /// The progress callback asked to stop.
    Cancelled,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Source(e) => write!(f, "read failed: {}", e.0),
            Error::Sink(e) => write!(f, "write failed: {}", e.0),
            Error::UnknownFormat => f.write_str("not a recognised archive"),
            Error::Unsupported(w) => write!(f, "unsupported: {w}"),
            Error::Corrupt(w) => write!(f, "damaged archive: {w}"),
            Error::Checksum => f.write_str("checksum mismatch"),
            Error::Encrypted => f.write_str("member is encrypted"),
            Error::UnsafePath(p) => write!(f, "unsafe path: {p}"),
            Error::NoSuchEntry => f.write_str("no such member"),
            Error::NotAFile => f.write_str("member is not a regular file"),
            Error::NotADirectory => f.write_str("not a folder"),
            Error::TooLarge => f.write_str("too large"),
            Error::SizeMismatch => f.write_str("item size does not match"),
            Error::InvalidName(n) => write!(f, "name cannot be stored: {n}"),
            Error::Cancelled => f.write_str("cancelled"),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// Progress report: bytes of member data done out of `total` (0 when the
/// total is not known).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// What the progress callback wants next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// Stop as soon as possible; the operation returns [`Error::Cancelled`].
    /// Whatever was already written to the sink stays written.
    Cancel,
}

/// Progress callback type. Called at least once per 64 KiB.
pub type ProgressFn<'a> = &'a mut dyn FnMut(Progress) -> Flow;

/// A progress callback that never cancels.
pub fn no_progress(_: Progress) -> Flow {
    Flow::Continue
}

/// Container formats this crate can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarZst,
    TarXz,
    /// A single gzip-compressed file.
    Gz,
    Zst,
    Xz,
    SevenZ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compression {
    None,
    Gzip,
    Zstd,
    Xz,
}

/// Kind of member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Hardlink,
    /// Devices, FIFOs and other special members; listed, never extracted.
    Other,
}

/// Where a member's data lives (format specific).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Loc {
    None,
    Zip { local_offset: u64, method: u16, csize: u64, crc: u32 },
    /// Offset of the data in the (decompressed) tar stream.
    Tar { offset: u64 },
    /// The whole decompressed stream.
    Single,
    SevenZ { folder: usize, offset: u64, crc: Option<u32> },
}

/// One archive member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Normalised path: `/` separated, no leading `./`, no trailing `/`.
    /// For an unsafe member (see `safe`) this is the raw name.
    pub path: String,
    /// The name exactly as stored (after charset decoding).
    pub raw_path: String,
    pub kind: EntryKind,
    /// Uncompressed size. Exact unless `size_known` is false: for single-file
    /// `.gz` / `.zst` streams it is then a hint from the stream (gzip ISIZE
    /// of the last member, zstd first-frame content size) or 0.
    pub size: u64,
    pub size_known: bool,
    /// Compressed size when the format records it per member.
    pub compressed_size: Option<u64>,
    /// Modification time, seconds since 1970-01-01 UTC.
    pub mtime: Option<i64>,
    /// Unix permission bits when recorded.
    pub mode: Option<u32>,
    /// Target of a symlink or hardlink (tar). Zip symlinks keep their
    /// target as file data: extracting them yields the target text.
    pub link_target: Option<String>,
    pub encrypted: bool,
    /// False when the name contains `..`, is absolute or has a drive
    /// prefix. Unsafe members are listed by `entries()` but hidden from
    /// `list_dir` and refused by `extract`.
    pub safe: bool,
    pub(crate) loc: Loc,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    pub(crate) fn new(raw: String, kind: EntryKind, size: u64, loc: Loc) -> Self {
        let (path, safe) = path::normalize(&raw);
        Entry {
            path,
            raw_path: raw,
            kind,
            size,
            size_known: true,
            compressed_size: None,
            mtime: None,
            mode: None,
            link_target: None,
            encrypted: false,
            safe,
            loc,
        }
    }
}

/// What [`Archive::extract`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Sanitised relative path to create the file at.
    pub path: String,
    pub bytes: u64,
}

/// Receiver for [`Archive::extract_all`].
pub trait ExtractVisitor {
    /// Called for each extractable member, in archive order, with its
    /// sanitised path. Return true to receive its data.
    fn begin(&mut self, entry: &Entry, path: &str) -> bool;
    /// Data of the member most recently accepted by `begin`.
    fn data(&mut self, bytes: &[u8]) -> core::result::Result<(), IoError>;
    /// The accepted member is complete.
    fn end(&mut self, entry: &Entry) -> core::result::Result<(), IoError>;
    /// A member that was skipped because its path is unsafe.
    fn rejected(&mut self, _entry: &Entry) {}
}

pub(crate) enum Inner {
    Zip,
    Tar(Compression),
    Single(Compression),
    SevenZ(sevenz::Info),
}

/// An opened archive.
pub struct Archive<S: ByteSource> {
    src: S,
    format: Format,
    entries: Vec<Entry>,
    inner: Inner,
}

const CHUNK: usize = 64 * 1024;

fn decompressor<'a, S: ByteSource + ?Sized>(src: &'a S, c: Compression) -> Box<dyn Input + 'a> {
    let raw = SrcReader::new(src, 0, src.len());
    match c {
        Compression::None => Box::new(raw),
        Compression::Gzip => Box::new(GzDecoder::new(raw)),
        Compression::Zstd => Box::new(ZstdDecoder::new(raw)),
        Compression::Xz => Box::new(XzDecoder::new(raw)),
    }
}

fn starts(h: &[u8], magic: &[u8]) -> bool {
    h.len() >= magic.len() && &h[..magic.len()] == magic
}

impl<S: ByteSource> Archive<S> {
    /// Open an archive, detecting its format from its contents.
    pub fn open(src: S) -> Result<Self> {
        Self::open_named(src, None)
    }

    /// Like [`Archive::open`]; `file_name` (the archive's own name) is used
    /// to name the member of a single-file `.gz`/`.zst`/`.xz` stream when
    /// the stream itself does not.
    pub fn open_named(src: S, file_name: Option<&str>) -> Result<Self> {
        let len = src.len();
        let mut head = vec![0u8; len.min(512) as usize];
        io::read_exact_at(&src, 0, &mut head)?;

        if starts(&head, &sevenz::MAGIC) {
            let (entries, info) = sevenz::open(&src)?;
            return Ok(Self { src, format: Format::SevenZ, entries, inner: Inner::SevenZ(info) });
        }
        if starts(&head, b"PK\x03\x04") || starts(&head, b"PK\x05\x06") || starts(&head, b"PK\x07\x08") {
            let entries = zip::open(&src)?;
            return Ok(Self { src, format: Format::Zip, entries, inner: Inner::Zip });
        }
        let comp = if starts(&head, &[0x1f, 0x8b]) {
            Some(Compression::Gzip)
        } else if starts(&head, &[0x28, 0xb5, 0x2f, 0xfd]) {
            Some(Compression::Zstd)
        } else if starts(&head, &[0xfd, b'7', b'z', b'X', b'Z', 0]) {
            Some(Compression::Xz)
        } else {
            None
        };
        if let Some(c) = comp {
            let mut first = vec![0u8; 512];
            let n = decompressor(&src, c).read_full(&mut first)?;
            if n == 512 && tar::is_header(&first) {
                let entries = tar::scan(&mut *decompressor(&src, c))?;
                let format = match c {
                    Compression::Gzip => Format::TarGz,
                    Compression::Zstd => Format::TarZst,
                    _ => Format::TarXz,
                };
                return Ok(Self { src, format, entries, inner: Inner::Tar(c) });
            }
            let entry = single_entry(&src, c, file_name)?;
            let format = match c {
                Compression::Gzip => Format::Gz,
                Compression::Zstd => Format::Zst,
                _ => Format::Xz,
            };
            return Ok(Self { src, format, entries: vec![entry], inner: Inner::Single(c) });
        }
        if starts(&head, b"BZh") {
            return Err(Error::Unsupported("bzip2"));
        }
        if starts(&head, b"Rar!") {
            return Err(Error::Unsupported("rar"));
        }
        if head.len() == 512 && (tar::is_header(&head) || tar::is_empty_tar(&src)?) {
            let entries = tar::scan(&mut SrcReader::new(&src, 0, len))?;
            return Ok(Self { src, format: Format::Tar, entries, inner: Inner::Tar(Compression::None) });
        }
        // Zip with a prefix (self-extracting) or an unusual first record.
        if let Ok(entries) = zip::open(&src) {
            return Ok(Self { src, format: Format::Zip, entries, inner: Inner::Zip });
        }
        Err(Error::UnknownFormat)
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// All members in archive order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The underlying byte source.
    pub fn source(&self) -> &S {
        &self.src
    }

    /// Index of the last member whose normalised path is `path`.
    pub fn find(&self, path: &str) -> Option<usize> {
        let p = path::trim_dir(path);
        self.entries.iter().rposition(|e| e.safe && e.path == p)
    }

    /// Children of folder `dir` ("" or "/" is the root), including folders
    /// that exist only because deeper paths imply them.
    pub fn list_dir(&self, dir: &str) -> Result<Vec<DirItem>> {
        path::list_dir(&self.entries, dir)
    }

    /// Stream member `index` into `sink`. Returns the sanitised path the
    /// member should be written to. Errors: [`Error::UnsafePath`] for
    /// traversal names (nothing is written), [`Error::NotAFile`] for
    /// folders/symlinks in tar/devices, [`Error::Cancelled`] when `progress`
    /// asks to stop.
    pub fn extract(&self, index: usize, sink: &mut dyn Sink, progress: ProgressFn<'_>) -> Result<Extracted> {
        let e = self.entries.get(index).ok_or(Error::NoSuchEntry)?;
        let path = self.check_extractable(e)?;
        let (data_entry, total) = self.resolve(index)?;
        let mut done = 0u64;
        let mut out = |b: &[u8]| sink.write_all(b);
        self.copy_member(data_entry, &mut out, &mut done, total, progress)?;
        Ok(Extracted { path, bytes: done })
    }

    /// Extract many members in one pass over the archive (one decompression
    /// pass for tar.gz/tar.zst/tar.xz and solid 7z). Progress totals are the
    /// sum of accepted member sizes known when the member is accepted.
    pub fn extract_all(&self, visitor: &mut dyn ExtractVisitor, progress: ProgressFn<'_>) -> Result<()> {
        let total: u64 = self.entries.iter().filter(|e| e.safe && e.kind == EntryKind::File).map(|e| e.size).sum();
        let mut done = 0u64;
        match &self.inner {
            Inner::Tar(c) if *c != Compression::None => {
                let mut input = Counted { inner: decompressor(&self.src, *c), pos: 0 };
                for (i, e) in self.entries.iter().enumerate() {
                    let Some(path) = self.visit_begin(visitor, e)? else { continue };
                    let (data_entry, _) = self.resolve(i)?;
                    let d = &self.entries[data_entry];
                    let Loc::Tar { offset } = d.loc else { return Err(Error::Corrupt("bad tar member")) };
                    if offset < input.pos {
                        // Hardlink to earlier data: restart the stream.
                        input = Counted { inner: decompressor(&self.src, *c), pos: 0 };
                    }
                    let skip = offset - input.pos;
                    input.skip(skip)?;
                    let _ = path;
                    let mut out = |b: &[u8]| visitor.data(b);
                    pump(&mut Take { inner: &mut input, left: d.size }, d.size, &mut out, &mut done, total, progress, None)?;
                    visitor.end(e).map_err(Error::Sink)?;
                }
                Ok(())
            }
            Inner::SevenZ(info) => sevenz::extract_all(self, info, visitor, &mut done, total, progress),
            _ => {
                for (i, e) in self.entries.iter().enumerate() {
                    if self.visit_begin(visitor, e)?.is_none() {
                        continue;
                    }
                    let (data_entry, _) = self.resolve(i)?;
                    let mut out = |b: &[u8]| visitor.data(b);
                    self.copy_member(data_entry, &mut out, &mut done, total, progress)?;
                    visitor.end(e).map_err(Error::Sink)?;
                }
                Ok(())
            }
        }
    }

    /// Whether `extract_all` would offer this member to a visitor.
    pub(crate) fn visit_begin_peek(&self, e: &Entry) -> bool {
        e.safe
            && !e.encrypted
            && (matches!(e.kind, EntryKind::File | EntryKind::Hardlink)
                || (e.kind == EntryKind::Symlink && matches!(e.loc, Loc::Zip { .. })))
    }

    /// Returns Some(sanitised path) when the visitor accepted the member.
    pub(crate) fn visit_begin(&self, visitor: &mut dyn ExtractVisitor, e: &Entry) -> Result<Option<String>> {
        if !e.safe {
            visitor.rejected(e);
            return Ok(None);
        }
        if !self.visit_begin_peek(e) {
            return Ok(None);
        }
        let path = e.path.clone();
        if visitor.begin(e, &path) {
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }

    fn check_extractable(&self, e: &Entry) -> Result<String> {
        if !e.safe {
            return Err(Error::UnsafePath(e.raw_path.clone()));
        }
        let path = e.path.clone();
        match e.kind {
            EntryKind::File | EntryKind::Hardlink => {}
            EntryKind::Symlink if matches!(e.loc, Loc::Zip { .. }) => {}
            _ => return Err(Error::NotAFile),
        }
        if e.encrypted {
            return Err(Error::Encrypted);
        }
        Ok(path)
    }

    /// Follow a tar hardlink to the member holding the data.
    fn resolve(&self, index: usize) -> Result<(usize, u64)> {
        let e = &self.entries[index];
        if e.kind != EntryKind::Hardlink {
            return Ok((index, e.size));
        }
        let target = e.link_target.as_deref().ok_or(Error::NotAFile)?;
        let (t, _) = path::normalize(target);
        let i = self.entries[..index]
            .iter()
            .rposition(|x| x.path == t && x.kind == EntryKind::File)
            .ok_or(Error::NotAFile)?;
        Ok((i, self.entries[i].size))
    }

    fn copy_member(
        &self,
        index: usize,
        out: &mut dyn FnMut(&[u8]) -> core::result::Result<(), IoError>,
        done: &mut u64,
        total: u64,
        progress: ProgressFn<'_>,
    ) -> Result<()> {
        let e = &self.entries[index];
        match (&self.inner, &e.loc) {
            (Inner::Zip, Loc::Zip { .. }) => zip::extract(&self.src, e, out, done, total, progress),
            (Inner::Tar(c), Loc::Tar { offset }) => {
                if *c == Compression::None {
                    let mut r = SrcReader::new(&self.src, *offset, offset + e.size);
                    pump(&mut r, e.size, out, done, total, progress, None)
                } else {
                    let mut d = decompressor(&self.src, *c);
                    d.skip(*offset)?;
                    pump(&mut Take { inner: d, left: e.size }, e.size, out, done, total, progress, None)
                }
            }
            (Inner::Single(c), Loc::Single) => {
                let mut d = decompressor(&self.src, *c);
                // The decoders verify each member/frame/block checksum.
                pump_unsized(&mut *d, out, done, total.max(e.size), progress)?;
                Ok(())
            }
            (Inner::SevenZ(info), _) => sevenz::extract_one(self, info, index, out, done, total, progress),
            _ => Err(Error::NotAFile),
        }
    }
}

/// Input that counts consumed bytes.
pub(crate) struct Counted<I> {
    pub inner: I,
    pub pos: u64,
}

impl<I: Input> Input for Counted<I> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
    fn skip(&mut self, n: u64) -> Result<()> {
        self.inner.skip(n)?;
        self.pos += n;
        Ok(())
    }
}

fn single_entry<S: ByteSource>(src: &S, c: Compression, file_name: Option<&str>) -> Result<Entry> {
    let mut name: Option<String> = None;
    let mut mtime = None;
    let mut size = None;
    match c {
        Compression::Gzip => {
            let h = codec::peek_gz_header(SrcReader::new(src, 0, src.len()))?;
            name = h.name;
            mtime = h.mtime;
            // ISIZE is the length mod 2^32 of the last member; trust it only
            // for a stream that plausibly is a single small member.
            if src.len() >= 18 {
                let mut t = [0u8; 4];
                io::read_exact_at(src, src.len() - 4, &mut t)?;
                let isize = u32::from_le_bytes(t) as u64;
                if src.len() < (1 << 31) {
                    size = Some(isize);
                }
            }
        }
        Compression::Zstd => {
            let mut d = ZstdDecoder::new(SrcReader::new(src, 0, src.len()));
            size = d.first_frame_size()?;
            // Only exact when the stream is one frame; verified on extract.
        }
        Compression::Xz => size = sevenz::xz_size(src).ok(),
        Compression::None => {}
    }
    let name = name
        .and_then(|n| {
            let base = n.rsplit(['/', '\\']).next().unwrap_or("");
            if base.is_empty() || base == "." || base == ".." {
                None
            } else {
                Some(String::from(base))
            }
        })
        .or_else(|| {
            let f = file_name?;
            let base = f.rsplit('/').next().unwrap_or(f);
            let lower = base.to_ascii_lowercase();
            for ext in [".gz", ".gzip", ".zst", ".zstd", ".xz", ".z"] {
                if lower.ends_with(ext) && base.len() > ext.len() {
                    return Some(String::from(&base[..base.len() - ext.len()]));
                }
            }
            None
        })
        .unwrap_or_else(|| String::from("data"));
    let mut e = Entry::new(name, EntryKind::File, size.unwrap_or(0), Loc::Single);
    // Only the xz index is authoritative; gzip/zstd may hold several
    // members/frames and record only one of them.
    e.size_known = c == Compression::Xz && size.is_some();
    e.compressed_size = Some(src.len());
    e.mtime = mtime;
    Ok(e)
}

/// Copy exactly `size` bytes from `input` to `out`, reporting progress.
pub(crate) fn pump(
    input: &mut dyn Input,
    size: u64,
    out: &mut dyn FnMut(&[u8]) -> core::result::Result<(), IoError>,
    done: &mut u64,
    total: u64,
    progress: ProgressFn<'_>,
    mut crc: Option<&mut crc32fast::Hasher>,
) -> Result<()> {
    let mut buf = vec![0u8; CHUNK.min(size.max(1) as usize)];
    let mut left = size;
    if progress(Progress { done: *done, total }) == Flow::Cancel {
        return Err(Error::Cancelled);
    }
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let n = input.read(&mut buf[..want])?;
        if n == 0 {
            return Err(Error::Corrupt("member data is truncated"));
        }
        if let Some(c) = crc.as_deref_mut() {
            c.update(&buf[..n]);
        }
        out(&buf[..n]).map_err(Error::Sink)?;
        left -= n as u64;
        *done += n as u64;
        if progress(Progress { done: *done, total }) == Flow::Cancel {
            return Err(Error::Cancelled);
        }
    }
    Ok(())
}

/// Copy `input` to its end. Returns the byte count.
pub(crate) fn pump_unsized(
    input: &mut dyn Input,
    out: &mut dyn FnMut(&[u8]) -> core::result::Result<(), IoError>,
    done: &mut u64,
    total: u64,
    progress: ProgressFn<'_>,
) -> Result<u64> {
    let mut buf = vec![0u8; CHUNK];
    let mut n_total = 0u64;
    if progress(Progress { done: *done, total }) == Flow::Cancel {
        return Err(Error::Cancelled);
    }
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(n_total);
        }
        out(&buf[..n]).map_err(Error::Sink)?;
        n_total += n as u64;
        *done += n as u64;
        if progress(Progress { done: *done, total }) == Flow::Cancel {
            return Err(Error::Cancelled);
        }
    }
}
