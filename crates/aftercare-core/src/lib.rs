// SPDX-License-Identifier: GPL-3.0-only
//! Aftercare's backup engine: encrypted, deduplicated, content-defined-chunked snapshots
//! kept in any blob store. It does no I/O of its own: files come from a [`Source`],
//! restored files go to a [`Sink`], the repository lives behind a [`Backend`], time and
//! randomness are supplied by the caller.
//!
//! ```text
//! Repository::init(backend, password, rng, opts)   -> new repo (config + first key)
//! Repository::open(backend, password)              -> unlock (Error::WrongPassword)
//! repo.backup(source, paths, opts, rng, observer)  -> snapshot id (+ summary)
//! repo.snapshots() / repo.ls(snap, path)
//! repo.restore(snap, path, sink, observer)         -> every chunk hash-verified
//! repo.forget(policy, dry_run) / repo.prune(opts, rng) / repo.check(read_data)
//! ```
//!
//! The on-disk format (version 1) is described in the crate's `README.md`, included
//! below.
#![doc = include_str!("../README.md")]
#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![forbid(unsafe_op_in_unsafe_fn)]
extern crate alloc;

mod backend;
mod backup;
mod chunker;
mod codec;
mod crypto;
#[cfg(any(feature = "std", test))]
mod fs;
mod maint;
mod model;
mod pack;
mod repo;

pub use backend::{Backend, FileKind, MemBackend};
pub use backup::{BackupOptions, ParentChoice};
pub use chunker::{Chunker, ChunkerParams};
pub use crypto::{KdfParams, Rng};
#[cfg(any(feature = "std", test))]
pub use fs::{FsBackend, FsSink, FsSource};
pub use maint::{apply_keep_policy, CheckReport, ForgetPlan, KeepPolicy, PruneOptions, PruneSummary};
pub use model::{
    BackupSummary, Entry, Node, NodeKind, Observer, Progress, Quiet, Sink, Snapshot, Source, Timestamp, Tree,
};
pub use pack::{BlobKind, Config, Lock};
pub use repo::{InitOptions, Repository, RestoreSummary};

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// A 32-byte identifier: a keyed blob hash, or the SHA-256 of a stored file.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub [u8; 32]);

impl Id {
    pub fn to_hex(&self) -> String {
        const H: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(H[(b >> 4) as usize] as char);
            s.push(H[(b & 15) as usize] as char);
        }
        s
    }
    pub fn from_hex(s: &str) -> Option<Id> {
        let b = s.as_bytes();
        if b.len() != 64 {
            return None;
        }
        let nib = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = (nib(b[2 * i])? << 4) | nib(b[2 * i + 1])?;
        }
        Some(Id(out))
    }
    /// First 8 hex digits, for display.
    pub fn short(&self) -> String {
        let mut h = self.to_hex();
        h.truncate(8);
        h
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Id({})", self.short())
    }
}
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No key file opens with this password.
    WrongPassword,
    /// The store has no such file.
    NotFound(String),
    /// Data failed authentication, a hash check, or decoding.
    Corrupt(String),
    /// A newer format or feature this engine does not know.
    Unsupported(String),
    /// Bad arguments (parameters, empty keep policy, ...).
    Invalid(String),
    /// The store failed (I/O, network).
    Backend(String),
    /// The source failed.
    Source(String),
    /// The sink failed.
    Sink(String),
    /// No such path in the snapshot.
    NoSuchPath(String),
    /// No snapshot matches.
    NoSuchSnapshot(String),
    /// Another operation holds a conflicting lock.
    Locked(String),
    /// The observer asked to stop.
    Cancelled,
    /// `init` on a store that already has a config.
    AlreadyInitialized,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::WrongPassword => f.write_str("wrong password"),
            Error::NotFound(s) => write!(f, "not found: {s}"),
            Error::Corrupt(s) => write!(f, "corrupt repository data: {s}"),
            Error::Unsupported(s) => write!(f, "unsupported: {s}"),
            Error::Invalid(s) => write!(f, "invalid: {s}"),
            Error::Backend(s) => write!(f, "store error: {s}"),
            Error::Source(s) => write!(f, "source error: {s}"),
            Error::Sink(s) => write!(f, "restore target error: {s}"),
            Error::NoSuchPath(s) => write!(f, "no such path in snapshot: {s}"),
            Error::NoSuchSnapshot(s) => write!(f, "no such snapshot: {s}"),
            Error::Locked(s) => write!(f, "repository is locked: {s}"),
            Error::Cancelled => f.write_str("cancelled"),
            Error::AlreadyInitialized => f.write_str("repository already initialized"),
        }
    }
}

#[cfg(any(feature = "std", test))]
impl std::error::Error for Error {}

/// Normalizes an absolute path: `/a//b/./c/` -> `/a/b/c`. Rejects `..` and relative paths.
pub fn normalize_path(p: &str) -> Result<String, Error> {
    let comps = path_components(p)?;
    if comps.is_empty() {
        return Ok("/".into());
    }
    let mut s = String::new();
    for c in comps {
        s.push('/');
        s.push_str(c);
    }
    Ok(s)
}

pub(crate) fn path_components(p: &str) -> Result<Vec<&str>, Error> {
    if !p.starts_with('/') {
        return Err(Error::Invalid(alloc::format!("path must be absolute: {p}")));
    }
    let mut v = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => return Err(Error::Invalid(alloc::format!("path may not contain ..: {p}"))),
            c => v.push(c),
        }
    }
    Ok(v)
}

pub(crate) fn join(parent: &str, name: &str) -> String {
    let mut s = String::from(parent);
    if !s.ends_with('/') {
        s.push('/');
    }
    s.push_str(name);
    s
}
