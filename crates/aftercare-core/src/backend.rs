// SPDX-License-Identifier: GPL-3.0-only
//! The repository store: named immutable blobs grouped by kind. A USB disk, an SFTP
//! server or memory all fit behind [`Backend`].

use crate::Error;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// What a stored file is. Each kind is its own namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    /// The single `config` file.
    Config,
    /// Password-wrapped master keys.
    Key,
    /// Packs of encrypted blobs.
    Pack,
    /// Index files mapping blob ids to pack locations.
    Index,
    /// Snapshot records.
    Snapshot,
    /// Lock records.
    Lock,
}

impl FileKind {
    pub const ALL: [FileKind; 6] =
        [FileKind::Config, FileKind::Key, FileKind::Pack, FileKind::Index, FileKind::Snapshot, FileKind::Lock];

    /// Directory name used by path-based stores (filesystem, SFTP).
    pub fn dir(self) -> &'static str {
        match self {
            FileKind::Config => "",
            FileKind::Key => "keys",
            FileKind::Pack => "data",
            FileKind::Index => "index",
            FileKind::Snapshot => "snapshots",
            FileKind::Lock => "locks",
        }
    }

    /// Relative path of a file in a path-based store; packs fan out by their first two hex digits.
    pub fn path(self, name: &str) -> String {
        match self {
            FileKind::Config => "config".to_string(),
            FileKind::Pack if name.len() >= 2 => alloc::format!("data/{}/{}", &name[..2], name),
            k => alloc::format!("{}/{}", k.dir(), name),
        }
    }
}

/// A blob store. Files are written once and never modified; names are lowercase hex
/// (or `config`). `put` must be atomic: a reader sees the whole file or none.
pub trait Backend {
    fn put(&mut self, kind: FileKind, name: &str, data: &[u8]) -> Result<(), Error>;
    /// Whole file; [`Error::NotFound`] if absent.
    fn get(&mut self, kind: FileKind, name: &str) -> Result<Vec<u8>, Error>;
    /// `len` bytes at `offset`. Stores that can seek should override this.
    fn get_range(&mut self, kind: FileKind, name: &str, offset: u64, len: u32) -> Result<Vec<u8>, Error> {
        let all = self.get(kind, name)?;
        let (o, l) = (offset as usize, len as usize);
        if o.checked_add(l).map_or(true, |e| e > all.len()) {
            return Err(Error::Corrupt(alloc::format!("{kind:?}/{name}: range past end")));
        }
        Ok(all[o..o + l].to_vec())
    }
    /// Size of a file in bytes.
    fn size(&mut self, kind: FileKind, name: &str) -> Result<u64, Error> {
        Ok(self.get(kind, name)?.len() as u64)
    }
    /// Names of all files of a kind, in any order.
    fn list(&mut self, kind: FileKind) -> Result<Vec<String>, Error>;
    /// Removing an absent file is not an error.
    fn delete(&mut self, kind: FileKind, name: &str) -> Result<(), Error>;
}

impl<B: Backend + ?Sized> Backend for &mut B {
    fn put(&mut self, k: FileKind, n: &str, d: &[u8]) -> Result<(), Error> {
        (**self).put(k, n, d)
    }
    fn get(&mut self, k: FileKind, n: &str) -> Result<Vec<u8>, Error> {
        (**self).get(k, n)
    }
    fn get_range(&mut self, k: FileKind, n: &str, o: u64, l: u32) -> Result<Vec<u8>, Error> {
        (**self).get_range(k, n, o, l)
    }
    fn size(&mut self, k: FileKind, n: &str) -> Result<u64, Error> {
        (**self).size(k, n)
    }
    fn list(&mut self, k: FileKind) -> Result<Vec<String>, Error> {
        (**self).list(k)
    }
    fn delete(&mut self, k: FileKind, n: &str) -> Result<(), Error> {
        (**self).delete(k, n)
    }
}

/// Everything in memory: for tests and previews.
#[derive(Default, Clone)]
pub struct MemBackend {
    pub files: BTreeMap<(FileKind, String), Vec<u8>>,
}

impl MemBackend {
    pub fn new() -> Self {
        Self::default()
    }
    /// Total stored bytes of one kind.
    pub fn bytes(&self, kind: FileKind) -> u64 {
        self.files.iter().filter(|((k, _), _)| *k == kind).map(|(_, v)| v.len() as u64).sum()
    }
    /// Total stored bytes.
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|v| v.len() as u64).sum()
    }
}

impl Backend for MemBackend {
    fn put(&mut self, kind: FileKind, name: &str, data: &[u8]) -> Result<(), Error> {
        self.files.insert((kind, name.to_string()), data.to_vec());
        Ok(())
    }
    fn get(&mut self, kind: FileKind, name: &str) -> Result<Vec<u8>, Error> {
        self.files.get(&(kind, name.to_string())).cloned().ok_or_else(|| Error::NotFound(kind.path(name)))
    }
    fn get_range(&mut self, kind: FileKind, name: &str, offset: u64, len: u32) -> Result<Vec<u8>, Error> {
        let f = self.files.get(&(kind, name.to_string())).ok_or_else(|| Error::NotFound(kind.path(name)))?;
        let (o, l) = (offset as usize, len as usize);
        f.get(o..o.saturating_add(l))
            .map(|s| s.to_vec())
            .ok_or_else(|| Error::Corrupt(alloc::format!("{}: range past end", kind.path(name))))
    }
    fn size(&mut self, kind: FileKind, name: &str) -> Result<u64, Error> {
        self.files
            .get(&(kind, name.to_string()))
            .map(|f| f.len() as u64)
            .ok_or_else(|| Error::NotFound(kind.path(name)))
    }
    fn list(&mut self, kind: FileKind) -> Result<Vec<String>, Error> {
        Ok(self.files.keys().filter(|(k, _)| *k == kind).map(|(_, n)| n.clone()).collect())
    }
    fn delete(&mut self, kind: FileKind, name: &str) -> Result<(), Error> {
        self.files.remove(&(kind, name.to_string()));
        Ok(())
    }
}
