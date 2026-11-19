// SPDX-License-Identifier: GPL-3.0-only
//! Remote places for Loom: servers it can browse like folders.
//!
//! Every protocol sits behind [`Place`]: list a folder (name, kind, size, modified
//! time), stat, read a byte range, write (streamed, and all-or-nothing where the
//! protocol allows), make a folder, rename and delete. Paths are `/`-separated and
//! relative to the place's root; `..` is refused.
//!
//! - [`sftp::Sftp`]: SFTP version 3 over any byte [`Stream`] — an SSH `sftp`
//!   subsystem through [`sftp::SshChannel`] (sex-ssh), or a server's pipes.
//! - [`webdav::WebDav`]: WebDAV (Nextcloud, ownCloud, Apache, ...) over [`http::Http`].
//! - [`s3::S3`]: S3-compatible object storage with AWS Signature Version 4.
//!
//! The crate does no I/O itself: the caller supplies connected byte streams
//! ([`Stream`], [`Connect`]), so the same code runs on SexOS's network service and
//! against real servers in host tests. Credentials come from the caller and are
//! never formatted by `Debug` or put in errors.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::{boxed::Box, string::String, vec::Vec};

pub mod http;
pub mod s3;
pub mod sftp;
pub mod sigv4;
pub mod text;
pub mod webdav;
pub mod xml;

/// What went wrong. Messages come from the server and never hold credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    /// The target already exists (mkdir, or a rename that will not overwrite).
    Exists,
    Denied,
    /// A folder that is not empty cannot be deleted.
    NotEmpty,
    /// A path with `..`, an empty name where one is needed, or the root where a file is needed.
    BadPath,
    /// The place or server cannot do this.
    Unsupported,
    /// The connection failed or closed.
    Transport(String),
    /// The server broke the protocol.
    Protocol(&'static str),
    /// The server refused, with its code (SFTP status, HTTP status) and message.
    Remote(u32, String),
    /// The data source given to `write` failed.
    Source,
}
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind { Dir, File, Link, Other }

/// One item in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// Bytes (0 for folders when the server does not say).
    pub size: u64,
    /// Modified time, seconds since 1970-01-01 UTC, when the server gives one.
    pub mtime: Option<u64>,
}

/// What a place can promise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Abilities {
    /// `write` replaces the file all at once: readers see the old or the new bytes, never a mix.
    pub atomic_write: bool,
    /// `rename` of a folder is one server operation (S3 copies every object instead).
    pub atomic_dir_rename: bool,
}

/// A remote folder tree.
pub trait Place {
    fn abilities(&self) -> Abilities;
    /// The items in a folder, sorted folders first then by name.
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>>;
    /// One item; its `name` is the last path component ("" for the root).
    fn stat(&mut self, path: &str) -> Result<Entry>;
    /// Up to `len` bytes from `offset` (fewer only at the end of the file).
    fn read(&mut self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>>;
    /// Writes a whole file from `src`, replacing any old one. `size` is the length when
    /// known (it lets HTTP send Content-Length). Gives the bytes written. When it fails,
    /// the old file (or no file) is left where the protocol allows (see `Abilities`).
    fn write(&mut self, path: &str, src: &mut dyn Source, size: Option<u64>) -> Result<u64>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    /// Moves a file or folder; refuses with `Exists` when `to` exists.
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;
    /// Deletes a file, or an empty folder (`NotEmpty` otherwise).
    fn delete(&mut self, path: &str) -> Result<()>;
}

/// Bytes to upload, read in order.
pub trait Source {
    /// Fills `buf` from the start; 0 means the end.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
}
impl Source for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(self.len());
        buf[..n].copy_from_slice(&self[..n]);
        *self = &self[n..];
        Ok(n)
    }
}
/// Fills `buf` as far as the source goes.
pub fn read_full(src: &mut dyn Source, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() { let k = src.read(&mut buf[n..])?; if k == 0 { break; } n += k; }
    Ok(n)
}

/// A connected, ordered byte stream (TCP, TLS, an SSH channel, a pipe). Blocking.
pub trait Stream {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()>;
    /// Some bytes into `buf`; 0 means the peer closed.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
}
impl<S: Stream + ?Sized> Stream for Box<S> {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> { (**self).write_all(bytes) }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> { (**self).read(buf) }
}

/// Opens new connections to one server (for HTTP, which may need several).
pub trait Connect {
    fn connect(&mut self) -> Result<Box<dyn Stream>>;
}

/// A cleaned path: its components, no empty, `.` or `..`.
pub fn components(path: &str) -> Result<Vec<&str>> {
    let mut v = Vec::new();
    for c in path.split('/') {
        match c { "" | "." => {} ".." => return Err(Error::BadPath), c => v.push(c) }
    }
    Ok(v)
}
/// Sorts folders first, then by name.
pub fn sort_entries(v: &mut [Entry]) { v.sort_by(|a, b| (a.kind != Kind::Dir, &a.name).cmp(&(b.kind != Kind::Dir, &b.name))); }

/// Splits a path into (parent components, last name); the root has no name.
pub(crate) fn split_last(path: &str) -> Result<(Vec<&str>, &str)> {
    let mut c = components(path)?;
    let last = c.pop().ok_or(Error::BadPath)?;
    Ok((c, last))
}

/// A name for the temporary file a write goes to before it replaces `name`.
pub(crate) fn temp_name(name: &str, n: u32) -> String { alloc::format!(".{name}.loom-{n:08x}.part") }
