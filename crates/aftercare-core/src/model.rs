// SPDX-License-Identifier: GPL-3.0-only
//! Trees, snapshots, and the traits through which the engine reads files ([`Source`]),
//! writes restored files ([`Sink`]) and reports progress ([`Observer`]).

use crate::codec::{Dec, Enc};
use crate::{Error, Id};
use alloc::string::String;
use alloc::vec::Vec;

/// Seconds and nanoseconds since 1970-01-01T00:00:00 UTC. The engine has no clock:
/// the caller supplies every time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

impl Timestamp {
    pub const fn from_secs(secs: i64) -> Self {
        Timestamp { secs, nanos: 0 }
    }
    fn enc(&self, e: &mut Enc) {
        e.i64(self.secs);
        e.u32(self.nanos);
    }
    fn dec(d: &mut Dec) -> Result<Self, Error> {
        Ok(Timestamp { secs: d.i64()?, nanos: d.u32()? })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
}

impl NodeKind {
    fn code(self) -> u8 {
        match self {
            NodeKind::File => 1,
            NodeKind::Dir => 2,
            NodeKind::Symlink => 3,
        }
    }
    fn from_code(c: u8) -> Result<Self, Error> {
        Ok(match c {
            1 => NodeKind::File,
            2 => NodeKind::Dir,
            3 => NodeKind::Symlink,
            _ => return Err(Error::Corrupt("tree: unknown node kind".into())),
        })
    }
}

/// Metadata of one directory entry as the [`Source`] reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Last path component (no `/`).
    pub name: String,
    pub kind: NodeKind,
    /// Bytes (files); 0 otherwise.
    pub size: u64,
    pub mtime: Timestamp,
    /// Permission-like bits, opaque to the engine (SexOS grants, or Unix mode on a host).
    pub mode: u32,
}

/// One entry of a stored directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub mtime: Timestamp,
    pub mode: u32,
    /// Files: chunk ids in order.
    pub content: Vec<Id>,
    /// Directories: id of the child tree.
    pub subtree: Option<Id>,
    /// Symlinks: target.
    pub target: String,
}

impl Node {
    pub(crate) fn from_entry(e: &Entry) -> Self {
        Node {
            name: e.name.clone(),
            kind: e.kind,
            size: e.size,
            mtime: e.mtime,
            mode: e.mode,
            content: Vec::new(),
            subtree: None,
            target: String::new(),
        }
    }
}

const TREE_MAGIC: &[u8; 4] = b"ACTR";
const TREE_VERSION: u16 = 1;

/// A directory: nodes sorted by name, names unique.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn get(&self, name: &str) -> Option<&Node> {
        self.nodes.binary_search_by(|n| n.name.as_str().cmp(name)).ok().map(|i| &self.nodes[i])
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new(TREE_MAGIC, TREE_VERSION);
        e.u32(self.nodes.len() as u32);
        for n in &self.nodes {
            e.str(&n.name);
            e.u8(n.kind.code());
            e.u64(n.size);
            n.mtime.enc(&mut e);
            e.u32(n.mode);
            match n.kind {
                NodeKind::File => {
                    e.u32(n.content.len() as u32);
                    for id in &n.content {
                        e.id(id);
                    }
                }
                NodeKind::Dir => e.id(&n.subtree.unwrap_or_default()),
                NodeKind::Symlink => e.str(&n.target),
            }
        }
        e.buf
    }

    pub(crate) fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut d = Dec::new(data, TREE_MAGIC, TREE_VERSION, "tree")?;
        let n = d.count(30)?;
        let mut nodes: Vec<Node> = Vec::with_capacity(n);
        for _ in 0..n {
            let name = d.str()?;
            let kind = NodeKind::from_code(d.u8()?)?;
            let mut node = Node {
                name,
                kind,
                size: d.u64()?,
                mtime: Timestamp::dec(&mut d)?,
                mode: d.u32()?,
                content: Vec::new(),
                subtree: None,
                target: String::new(),
            };
            match kind {
                NodeKind::File => {
                    let c = d.count(32)?;
                    node.content = (0..c).map(|_| d.id()).collect::<Result<_, _>>()?;
                }
                NodeKind::Dir => node.subtree = Some(d.id()?),
                NodeKind::Symlink => node.target = d.str()?,
            }
            if node.name.is_empty()
                || node.name.contains('/')
                || nodes.last().is_some_and(|p| p.name >= node.name)
            {
                return Err(Error::Corrupt("tree: bad or unsorted names".into()));
            }
            nodes.push(node);
        }
        d.finish()?;
        Ok(Tree { nodes })
    }
}

/// Counters a backup records in its snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackupSummary {
    pub files_new: u64,
    pub files_changed: u64,
    /// Files taken from the parent snapshot by size and mtime, without reading them.
    pub files_unchanged: u64,
    pub dirs: u64,
    /// File bytes in the snapshot.
    pub bytes_total: u64,
    /// File bytes actually read.
    pub bytes_read: u64,
    /// Blobs not already in the repository.
    pub blobs_added: u64,
    /// Plaintext bytes of new blobs.
    pub bytes_added: u64,
    /// Bytes written to the store for new blobs (after compression and encryption).
    pub bytes_stored: u64,
    /// Entries skipped because the source could not read them.
    pub errors: u64,
}

const SNAP_MAGIC: &[u8; 4] = b"ACSN";
const SNAP_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub time: Timestamp,
    /// Root tree id.
    pub tree: Id,
    pub parent: Option<Id>,
    /// Host label (machine name the caller chose).
    pub host: String,
    /// Paths that were backed up, normalized.
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub summary: BackupSummary,
}

impl Snapshot {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new(SNAP_MAGIC, SNAP_VERSION);
        self.time.enc(&mut e);
        e.id(&self.tree);
        match &self.parent {
            Some(p) => {
                e.u8(1);
                e.id(p);
            }
            None => e.u8(0),
        }
        e.str(&self.host);
        e.strs(&self.paths);
        e.strs(&self.tags);
        let s = &self.summary;
        for v in [
            s.files_new,
            s.files_changed,
            s.files_unchanged,
            s.dirs,
            s.bytes_total,
            s.bytes_read,
            s.blobs_added,
            s.bytes_added,
            s.bytes_stored,
            s.errors,
        ] {
            e.u64(v);
        }
        e.buf
    }

    pub(crate) fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut d = Dec::new(data, SNAP_MAGIC, SNAP_VERSION, "snapshot")?;
        let time = Timestamp::dec(&mut d)?;
        let tree = d.id()?;
        let parent = if d.u8()? != 0 { Some(d.id()?) } else { None };
        let host = d.str()?;
        let paths = d.strs()?;
        let tags = d.strs()?;
        let summary = BackupSummary {
            files_new: d.u64()?,
            files_changed: d.u64()?,
            files_unchanged: d.u64()?,
            dirs: d.u64()?,
            bytes_total: d.u64()?,
            bytes_read: d.u64()?,
            blobs_added: d.u64()?,
            bytes_added: d.u64()?,
            bytes_stored: d.u64()?,
            errors: d.u64()?,
        };
        d.finish()?;
        Ok(Snapshot { time, tree, parent, host, paths, tags, summary })
    }
}

/// Where backups read from. Paths are absolute, `/`-separated, without `.` or `..`.
pub trait Source {
    /// Metadata of one path (its `name` is the last component, or empty for `/`).
    fn stat(&mut self, path: &str) -> Result<Entry, Error>;
    /// Entries of a directory, any order.
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Error>;
    /// Reads file bytes at `offset` into `buf`; returns 0 at end of file.
    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, Error>;
    /// Target of a symlink.
    fn read_link(&mut self, _path: &str) -> Result<String, Error> {
        Err(Error::Unsupported("symlinks".into()))
    }
}

/// Where restores write to. Paths are relative (`docs/a.txt`), `/`-separated, and arrive
/// parents first. Every chunk has been authenticated and its keyed hash checked before
/// it is handed over.
pub trait Sink {
    fn create_dir(&mut self, path: &str, node: &Node) -> Result<(), Error>;
    /// After all children of `path` are restored (a place to set directory times).
    fn finish_dir(&mut self, _path: &str, _node: &Node) -> Result<(), Error> {
        Ok(())
    }
    fn begin_file(&mut self, path: &str, node: &Node) -> Result<(), Error>;
    /// Next bytes of the file begun last.
    fn write(&mut self, data: &[u8]) -> Result<(), Error>;
    /// The file is complete and its size matches the snapshot.
    fn end_file(&mut self, path: &str, node: &Node) -> Result<(), Error>;
    fn symlink(&mut self, _path: &str, _node: &Node) -> Result<(), Error> {
        Ok(())
    }
}

/// Running totals during backup or restore.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Plaintext bytes of new blobs (backup only).
    pub bytes_added: u64,
    /// Path being worked on.
    pub current: String,
}

/// Progress, warnings and cancellation. All methods have do-nothing defaults.
pub trait Observer {
    fn progress(&mut self, _p: &Progress) {}
    /// A skipped entry or other non-fatal problem.
    fn warning(&mut self, _msg: &str) {}
    /// Polled between files and chunks; `true` stops the operation with [`Error::Cancelled`].
    fn cancelled(&mut self) -> bool {
        false
    }
}

/// An [`Observer`] that ignores everything.
pub struct Quiet;
impl Observer for Quiet {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_encoding_is_stable() {
        let t = Tree {
            nodes: alloc::vec![
                Node {
                    name: "a".into(),
                    kind: NodeKind::File,
                    size: 3,
                    mtime: Timestamp { secs: 1, nanos: 2 },
                    mode: 0o644,
                    content: alloc::vec![Id([0xab; 32])],
                    subtree: None,
                    target: String::new(),
                },
                Node {
                    name: "l".into(),
                    kind: NodeKind::Symlink,
                    size: 0,
                    mtime: Timestamp::default(),
                    mode: 0,
                    content: Vec::new(),
                    subtree: None,
                    target: "a".into(),
                },
            ],
        };
        let b = t.encode();
        // Golden: magic, version 1, two nodes, then the first node's name and kind.
        assert_eq!(&b[..10], b"ACTR\x01\x00\x02\x00\x00\x00");
        assert_eq!(&b[10..16], b"\x01\x00\x00\x00a\x01");
        assert_eq!(b.len(), 10 + (5 + 1 + 8 + 12 + 4 + 4 + 32) + (5 + 1 + 8 + 12 + 4 + 5));
        assert_eq!(Tree::decode(&b).unwrap(), t);
        // Unsorted names, truncation and unknown versions are refused.
        let mut swapped = t.clone();
        swapped.nodes.reverse();
        assert!(Tree::decode(&swapped.encode()).is_err());
        assert!(Tree::decode(&b[..b.len() - 1]).is_err());
        let mut v2 = b.clone();
        v2[4] = 2;
        assert!(matches!(Tree::decode(&v2), Err(Error::Unsupported(_))));
    }
}
