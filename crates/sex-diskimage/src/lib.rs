// SPDX-License-Identifier: GPL-3.0-only
//! Disk and CD images opened read-only, for Loom's "mount anything".
//!
//! `#![no_std]` + `alloc`. Everything reads through a random-access
//! [`ByteSource`] (the same trait `sex-archive` reads archives through, so
//! one file-handle adapter serves both crates); nothing is loaded whole.
//!
//! * **Folders**: ISO 9660 with Joliet and Rock Ridge (long names, symlinks,
//!   modes, mtimes, relocated directories), multi-extent files and the
//!   El Torito boot catalog shown as a virtual `[BOOT]` folder; UDF 1.02 to
//!   2.60 (physical, sparable and metadata partitions, file entries and
//!   extended file entries, short/long/extended allocation descriptors,
//!   embedded data). A hybrid ISO/UDF disc opens as UDF.
//! * **Disks**: raw images, qcow2 (v2/v3, zlib-compressed clusters), VHD
//!   (fixed and dynamic), VHDX and VMDK (monolithicSparse and
//!   streamOptimized), all read-only and without backing/parent files. A
//!   [`BlockDevice`] is itself a [`ByteSource`]; its [`BlockDevice::partitions`]
//!   reads MBR (with logical partitions) or GPT and names each partition's
//!   type and the filesystem found in it, so the caller can open FAT, ext4
//!   and friends with its own filesystem code over [`BlockDevice::slice`].

#![no_std]

extern crate alloc;

mod container;
mod iso;
mod part;
mod udf;
mod util;

use alloc::string::String;
use alloc::vec::Vec;

pub use container::{BlockDevice, Format};
pub use iso::{Iso, Namespace};
pub use part::{probe_filesystem, FsKind, PartType, Partition, Scheme, Slice};
pub use sex_archive::{ByteSource, IoError};
pub use udf::Udf;

/// Why an image could not be opened or read. Messages are short English
/// phrases fit to show the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The underlying source failed.
    Io(IoError),
    /// Not an image format this crate knows.
    NotRecognized,
    /// The image claims a format but its structures are damaged.
    Corrupt(&'static str),
    /// A valid image using a feature this crate does not read.
    Unsupported(&'static str),
    NotFound,
    NotADirectory,
    NotAFile,
    /// The image is a disk, not a folder (or the other way round).
    WrongKind,
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Error::Io(e)
    }
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self {
            Error::Io(e) => e.0,
            Error::NotRecognized => "not a disk or CD image",
            Error::Corrupt(m) | Error::Unsupported(m) => m,
            Error::NotFound => "no such file in the image",
            Error::NotADirectory => "not a folder",
            Error::NotAFile => "not a file",
            Error::WrongKind => "wrong kind of image for this operation",
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Corrupt(m) => write!(f, "damaged image: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            _ => f.write_str(self.message()),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// What [`Image::open`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Iso9660,
    Udf,
    Raw,
    Qcow2,
    Vhd,
    Vhdx,
    Vmdk,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Iso9660 => "ISO 9660 CD image",
            Kind::Udf => "UDF disc image",
            Kind::Raw => "raw disk image",
            Kind::Qcow2 => "QEMU qcow2 disk image",
            Kind::Vhd => "VHD disk image",
            Kind::Vhdx => "VHDX disk image",
            Kind::Vmdk => "VMDK disk image",
        }
    }
    /// Images that browse as a folder of files.
    pub fn is_folder(self) -> bool {
        matches!(self, Kind::Iso9660 | Kind::Udf)
    }
}

/// An opaque handle on a file or folder inside a [`Volume`]. Cheap to clone;
/// valid only for the volume that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node(pub(crate) NodeRef);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeRef {
    /// ISO directory extent: byte offset and length.
    IsoDir { at: u64, len: u64 },
    /// File data as (byte offset, length) runs, in order.
    Extents(Vec<(u64, u64)>),
    /// The virtual `[BOOT]` folder of an El Torito disc.
    BootDir,
    /// A UDF ICB: partition reference and logical block.
    Udf { part: u16, lbn: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// A symbolic link and its target as stored (not resolved).
    Symlink(String),
}

/// One name in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// Bytes of file data (0 for folders and links).
    pub size: u64,
    /// Modification time, seconds since 1970-01-01 UTC.
    pub mtime: Option<i64>,
    /// Permission bits (`0o755` style) when the image records them.
    pub mode: Option<u32>,
    /// Marked hidden by the image.
    pub hidden: bool,
    /// Made up by this crate (the El Torito `[BOOT]` folder and its files).
    pub synthetic: bool,
    pub node: Node,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

/// A browsable disc: ISO 9660 or UDF.
pub enum Volume<S: ByteSource> {
    Iso(Iso<S>),
    Udf(Udf<S>),
}

macro_rules! each {
    ($self:ident, $v:ident => $e:expr) => {
        match $self {
            Volume::Iso($v) => $e,
            Volume::Udf($v) => $e,
        }
    };
}

impl<S: ByteSource> Volume<S> {
    pub fn kind(&self) -> Kind {
        match self {
            Volume::Iso(_) => Kind::Iso9660,
            Volume::Udf(_) => Kind::Udf,
        }
    }
    /// The volume name the disc was given.
    pub fn label(&self) -> &str {
        each!(self, v => v.label())
    }
    pub fn root(&self) -> Node {
        each!(self, v => v.root())
    }
    pub fn list_dir(&self, dir: &Node) -> Result<Vec<Entry>> {
        each!(self, v => v.list_dir(dir))
    }
    /// Read file bytes at `offset`; returns how many were read (0 at the end).
    pub fn read(&self, file: &Node, offset: u64, buf: &mut [u8]) -> Result<usize> {
        each!(self, v => v.read(file, offset, buf))
    }
    /// Size in bytes of a file node.
    pub fn file_len(&self, file: &Node) -> Result<u64> {
        each!(self, v => v.file_len(file))
    }
    /// The entry at a `/`-separated path from the root (`""` or `"/"` is the root
    /// folder itself, returned with an empty name). Links are not followed.
    pub fn lookup(&self, path: &str) -> Result<Entry> {
        util::lookup(path, self.root(), |n| self.list_dir(n))
    }
    /// Every entry in the volume with its full path, folders before their
    /// contents. Stops at 64 levels and one million entries.
    pub fn entries(&self) -> Result<Vec<(String, Entry)>> {
        util::walk(self.root(), |n| self.list_dir(n))
    }
    /// Whole file into memory, refusing files larger than `limit` bytes.
    pub fn read_to_vec(&self, file: &Node, limit: u64) -> Result<Vec<u8>> {
        util::read_to_vec(self.file_len(file)?, limit, |o, b| self.read(file, o, b))
    }
    /// A file of the volume as a [`ByteSource`], to open nested archives or images.
    pub fn file_source(&self, file: &Node) -> Result<FileSource<'_, S>> {
        Ok(FileSource { vol: self, node: file.clone(), len: self.file_len(file)? })
    }
}

/// A file inside a [`Volume`] seen as a [`ByteSource`].
pub struct FileSource<'a, S: ByteSource> {
    vol: &'a Volume<S>,
    node: Node,
    len: u64,
}

impl<S: ByteSource> ByteSource for FileSource<'_, S> {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        self.vol.read(&self.node, offset, buf).map_err(util::io_err)
    }
}

/// An opened image: a folder (ISO/UDF) or a disk (raw and container formats).
pub enum Image<S: ByteSource> {
    Folder(Volume<S>),
    Disk(BlockDevice<S>),
}

impl<S: ByteSource> Image<S> {
    /// Detects the format and opens it. Container formats are checked first,
    /// then UDF (so hybrid discs open as UDF), then ISO 9660; anything else
    /// that is a whole number of 512-byte sectors opens as a raw disk.
    pub fn open(src: S) -> Result<Self> {
        if let Some(f) = container::detect(&src)? {
            return Ok(Image::Disk(BlockDevice::open_as(src, f)?));
        }
        if udf::detect(&src)? {
            return Ok(Image::Folder(Volume::Udf(Udf::open(src)?)));
        }
        if iso::detect(&src)? {
            return Ok(Image::Folder(Volume::Iso(Iso::open(src)?)));
        }
        if src.len() >= 512 && src.len() % 512 == 0 {
            return Ok(Image::Disk(BlockDevice::open_as(src, Format::Raw)?));
        }
        Err(Error::NotRecognized)
    }

    /// Detection only, without opening.
    pub fn detect(src: &S) -> Result<Option<Kind>> {
        if let Some(f) = container::detect(src)? {
            return Ok(Some(f.kind()));
        }
        if udf::detect(src)? {
            return Ok(Some(Kind::Udf));
        }
        if iso::detect(src)? {
            return Ok(Some(Kind::Iso9660));
        }
        Ok(None)
    }

    pub fn kind(&self) -> Kind {
        match self {
            Image::Folder(v) => v.kind(),
            Image::Disk(d) => d.format().kind(),
        }
    }
    pub fn as_volume(&self) -> Option<&Volume<S>> {
        match self {
            Image::Folder(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_disk(&self) -> Option<&BlockDevice<S>> {
        match self {
            Image::Disk(d) => Some(d),
            _ => None,
        }
    }
    pub fn list_dir(&self, dir: &Node) -> Result<Vec<Entry>> {
        self.as_volume().ok_or(Error::WrongKind)?.list_dir(dir)
    }
    pub fn entries(&self) -> Result<Vec<(String, Entry)>> {
        self.as_volume().ok_or(Error::WrongKind)?.entries()
    }
    pub fn read(&self, file: &Node, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.as_volume().ok_or(Error::WrongKind)?.read(file, offset, buf)
    }
    pub fn partitions(&self) -> Result<Vec<Partition>> {
        self.as_disk().ok_or(Error::WrongKind)?.partitions()
    }
}
