// SexOS: std's Hermit target (x86_64-sex-std.json). Times come from std's metadata;
// setting them is not offered, so callers that try get Unsupported.
use crate::FileTime;
use std::fs;
use std::io;
use std::path::Path;

fn unsupported() -> io::Error { io::Error::new(io::ErrorKind::Unsupported, "file times cannot be set on SexOS") }

pub fn set_symlink_file_times(_p: &Path, _atime: FileTime, _mtime: FileTime) -> io::Result<()> { Err(unsupported()) }
pub fn open(path: &Path) -> io::Result<fs::File> { fs::OpenOptions::new().write(true).open(path) }

pub fn from_last_modification_time(meta: &fs::Metadata) -> FileTime {
    meta.modified().map(FileTime::from_system_time).unwrap_or_else(|_| FileTime::zero())
}
pub fn from_last_access_time(meta: &fs::Metadata) -> FileTime {
    meta.accessed().map(FileTime::from_system_time).unwrap_or_else(|_| from_last_modification_time(meta))
}
pub fn from_creation_time(meta: &fs::Metadata) -> Option<FileTime> {
    meta.created().ok().map(FileTime::from_system_time)
}
