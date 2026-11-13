// SPDX-License-Identifier: GPL-3.0-only
//! The parts of `std::fs` that std's Hermit port refuses before it ever calls this
//! crate, offered here instead.
//!
//! std's Hermit backend (library/std/src/sys/fs/hermit.rs on this toolchain) answers
//! `rename`, `remove_dir_all`, `canonicalize`, `File::sync_all`, `File::sync_data`,
//! `File::set_len` and `File::set_permissions` itself, with "unsupported" or EINVAL,
//! and `fs::copy` copies the bytes and then fails on `set_permissions`. None of them
//! reaches a `sys_*` call, so no answer here can change what `std::fs` says. A program
//! that needs them calls these, which do the same job over this crate's calls:
//!
//! ```ignore
//! use sex_std::fs::FileExt;
//! sex_std::fs::rename("draft.txt", "final.txt")?;
//! let f = std::fs::OpenOptions::new().write(true).open("final.txt")?;
//! f.truncate(10)?;   // File::set_len
//! f.fsync()?;        // File::sync_all: Files' SYNC request
//! ```
//!
//! `std::fs::read_dir` on an empty folder fails (std's Hermit backend reads "no
//! entries" as an error); [`read_dir`] here returns an empty list instead.
//!
//! Everything else in `std::fs` works as it is: `create_dir`, `create_dir_all`,
//! `remove_dir`, `remove_file`, `read_dir`, `metadata` (file type and length; modified
//! time where the volume keeps one), `read`, `write`, `OpenOptions`.
use std::io;
use std::path::{Path, PathBuf};

/// Calls a negative-error-number function and makes its answer an `io::Result`.
fn cvt(r: i32) -> io::Result<()> { if r < 0 { Err(io::Error::from_raw_os_error(-r)) } else { Ok(()) } }

fn c_path(p: &Path) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// `std::fs::rename`: moves a file or folder within the program's folder, replacing a
/// file already at `to`.
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let (a, b) = (c_path(from.as_ref())?, c_path(to.as_ref())?);
    cvt(unsafe { crate::sys::sys_rename(a.as_ptr(), b.as_ptr()) })
}

/// `std::fs::copy`: copies a file's bytes; returns how many. (There are no permissions
/// to copy.)
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    let mut reader = std::fs::File::open(from)?;
    if !reader.metadata()?.is_file() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file")); }
    let mut writer = std::fs::File::create(to)?;
    io::copy(&mut reader, &mut writer)
}

/// `std::fs::read_dir`, collected, and empty for an empty folder. (std's Hermit
/// `read_dir` takes "no entries" for an error, so on an empty folder it fails with
/// OS error 0; this works around that without changing what a listing holds.)
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<std::fs::DirEntry>> {
    match std::fs::read_dir(path) {
        Ok(d) => d.collect(),
        Err(e) if e.raw_os_error() == Some(0) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// `std::fs::remove_dir_all`: a folder and everything in it.
pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    for child in read_dir(path)? {
        let r = if child.file_type()?.is_dir() { remove_dir_all(child.path()) } else { std::fs::remove_file(child.path()) };
        match r { Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e), _ => {} }
    }
    match std::fs::remove_dir(path) { Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()), r => r }
}

/// `std::fs::canonicalize`: the path from the program's folder (`/`), with `.` and
/// `..` worked out; it must exist.
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let p = canonical(path.as_ref().as_os_str().as_encoded_bytes())?;
    std::fs::metadata(&p)?;
    Ok(p)
}

/// The canonical spelling of `path`, without asking whether it exists.
pub fn canonical(path: &[u8]) -> io::Result<PathBuf> {
    let r = crate::path::resolve(path).map_err(|e| match e {
        crate::path::Refused::AboveFolder => io::Error::from(io::ErrorKind::PermissionDenied),
        crate::path::Refused::NotUtf8OrNul => io::Error::from(io::ErrorKind::InvalidInput),
    })?;
    let mut s = String::from("/");
    s.push_str(&String::from_utf8_lossy(&r));
    Ok(PathBuf::from(s))
}

/// What `std::fs::File` cannot do on this target by itself.
pub trait FileExt {
    /// `File::sync_all` / `sync_data`: everything the program wrote is durable when
    /// this returns (Files commits all of a program's writes at once).
    fn fsync(&self) -> io::Result<()>;
    /// `File::set_len`: cuts the file to `len` bytes, or extends it with zeros.
    fn truncate(&self, len: u64) -> io::Result<()>;
}

impl FileExt for std::fs::File {
    fn fsync(&self) -> io::Result<()> {
        use std::os::hermit::io::AsRawFd;
        cvt(crate::sys::sys_fsync(self.as_raw_fd()))
    }
    fn truncate(&self, len: u64) -> io::Result<()> {
        use std::os::hermit::io::AsRawFd;
        let len = i64::try_from(len).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        cvt(crate::sys::sys_ftruncate(self.as_raw_fd(), len))
    }
}
