// SPDX-License-Identifier: GPL-3.0-only
//! Opening a repository, blob storage, snapshots, browsing and restore.

use crate::backend::{Backend, FileKind};
use crate::chunker::{Chunker, ChunkerParams};
use crate::crypto::{self, sha256, KdfParams, MasterKey, Rng};
use crate::model::{Node, NodeKind, Observer, Progress, Sink, Snapshot, Timestamp, Tree};
use crate::pack::{
    blob_aad, decode_index, encode_index, BlobKind, Config, Lock, PackEntry, Packer, CONFIG_AAD, INDEX_AAD, LOCK_AAD,
    SNAPSHOT_AAD,
};
use crate::{path_components, Error, Id};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Settings fixed when a repository is created.
#[derive(Clone, Debug)]
pub struct InitOptions {
    pub chunker: ChunkerParams,
    /// Target pack size; 16 MiB by default.
    pub pack_size: u32,
    pub compression: bool,
    pub kdf: KdfParams,
}

impl Default for InitOptions {
    fn default() -> Self {
        InitOptions {
            chunker: ChunkerParams::default(),
            pack_size: 16 * 1024 * 1024,
            compression: true,
            kdf: KdfParams::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Loc {
    pub pack: Id,
    pub entry: PackEntry,
}

/// What a restore wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreSummary {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub bytes: u64,
}

/// An unlocked repository.
pub struct Repository<B: Backend> {
    pub(crate) backend: B,
    pub(crate) key: MasterKey,
    pub(crate) key_name: String,
    pub(crate) config: Config,
    pub(crate) chunker: Chunker,
    pub(crate) index: BTreeMap<Id, Loc>,
    pub(crate) data_packer: Packer,
    pub(crate) tree_packer: Packer,
    /// Packs written but not yet recorded in an index file.
    pub(crate) unsaved: Vec<(Id, Vec<PackEntry>)>,
    pub(crate) host: String,
}

impl<B: Backend> Repository<B> {
    /// Creates a repository in an empty store.
    pub fn init(mut backend: B, password: &[u8], rng: &mut dyn Rng, opts: &InitOptions) -> Result<Self, Error> {
        if !opts.chunker.is_valid() {
            return Err(Error::Invalid("chunker sizes need 64 <= min < avg < max <= 64 MiB".into()));
        }
        if opts.pack_size < 1024 || opts.pack_size > 1 << 30 {
            return Err(Error::Invalid("pack size".into()));
        }
        if !backend.list(FileKind::Config)?.is_empty() {
            return Err(Error::AlreadyInitialized);
        }
        let key = MasterKey::generate(rng);
        let mut id = Id::default();
        rng.fill_bytes(&mut id.0);
        let config = Config {
            id,
            chunker: opts.chunker,
            gear_seed: rng.next_u64(),
            pack_size: opts.pack_size,
            compression: opts.compression,
        };
        let keyfile = crypto::wrap_key(&key, password, opts.kdf, rng)?;
        let key_name = sha256(&keyfile).to_hex();
        backend.put(FileKind::Key, &key_name, &keyfile)?;
        let sealed = key.seal(rng, CONFIG_AAD, &config.encode());
        backend.put(FileKind::Config, "config", &sealed)?;
        Ok(Self::assemble(backend, key, key_name, config))
    }

    /// Unlocks a repository with a password; [`Error::WrongPassword`] if no key file opens.
    pub fn open(mut backend: B, password: &[u8]) -> Result<Self, Error> {
        let mut names = backend.list(FileKind::Key)?;
        names.sort();
        if names.is_empty() {
            return Err(Error::NotFound("no key files: not a repository".into()));
        }
        let mut found = None;
        for n in names {
            let file = backend.get(FileKind::Key, &n)?;
            match crypto::unwrap_key(&file, password) {
                Ok(k) => {
                    found = Some((k, n));
                    break;
                }
                // A damaged key file is skipped like one for another password.
                Err(_) => {}
            }
        }
        let Some((key, key_name)) = found else {
            return Err(Error::WrongPassword);
        };
        let sealed = backend.get(FileKind::Config, "config")?;
        let config = Config::decode(&key.open(CONFIG_AAD, &sealed, "config")?)?;
        let mut repo = Self::assemble(backend, key, key_name, config);
        repo.reload_index()?;
        Ok(repo)
    }

    fn assemble(backend: B, key: MasterKey, key_name: String, config: Config) -> Self {
        let chunker = Chunker::new(config.chunker, config.gear_seed);
        Repository {
            backend,
            key,
            key_name,
            config,
            chunker,
            index: BTreeMap::new(),
            data_packer: Packer::default(),
            tree_packer: Packer::default(),
            unsaved: Vec::new(),
            host: String::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn backend(&self) -> &B {
        &self.backend
    }
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
    pub fn into_backend(self) -> B {
        self.backend
    }
    /// Name written into lock records.
    pub fn set_host(&mut self, host: &str) {
        self.host = host.to_string();
    }
    /// Name of the key file this session unlocked.
    pub fn key_name(&self) -> &str {
        &self.key_name
    }

    /// Adds another password that unlocks the same master key; returns the key file name.
    pub fn add_password(&mut self, password: &[u8], kdf: KdfParams, rng: &mut dyn Rng) -> Result<String, Error> {
        let file = crypto::wrap_key(&self.key, password, kdf, rng)?;
        let name = sha256(&file).to_hex();
        self.backend.put(FileKind::Key, &name, &file)?;
        Ok(name)
    }

    /// Removes a key file (not the one in use).
    pub fn remove_password(&mut self, key_name: &str) -> Result<(), Error> {
        if key_name == self.key_name {
            return Err(Error::Invalid("cannot remove the key this session uses".into()));
        }
        self.backend.delete(FileKind::Key, key_name)
    }

    pub(crate) fn reload_index(&mut self) -> Result<(), Error> {
        self.index.clear();
        for (_, packs) in self.read_index_files()? {
            for (pack, entries) in packs {
                for entry in entries {
                    self.index.entry(entry.id).or_insert(Loc { pack, entry });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn read_index_files(&mut self) -> Result<Vec<(String, Vec<(Id, Vec<PackEntry>)>)>, Error> {
        let mut names = self.backend.list(FileKind::Index)?;
        names.sort();
        let mut out = Vec::new();
        for n in names {
            let sealed = self.backend.get(FileKind::Index, &n)?;
            let plain = self.key.open(INDEX_AAD, &sealed, "index")?;
            out.push((n, decode_index(&plain)?));
        }
        Ok(out)
    }

    // ---- blobs ----

    pub(crate) fn has_blob(&self, id: &Id) -> bool {
        self.index.contains_key(id) || self.data_packer.ids.contains(id) || self.tree_packer.ids.contains(id)
    }

    /// Stores a blob unless present; returns its id and the bytes it added to the store.
    pub(crate) fn save_blob(&mut self, rng: &mut dyn Rng, kind: BlobKind, data: &[u8]) -> Result<(Id, u64), Error> {
        let id = self.key.blob_id(data);
        if self.has_blob(&id) {
            return Ok((id, 0));
        }
        let mut compressed = None;
        if self.config.compression && data.len() >= 128 {
            let c = ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest);
            if c.len() + 64 < data.len() {
                compressed = Some(c);
            }
        }
        let payload = compressed.as_deref().unwrap_or(data);
        let sealed = self.key.seal(rng, &blob_aad(&id, kind), payload);
        let stored = sealed.len() as u64;
        let packer = match kind {
            BlobKind::Data => &mut self.data_packer,
            BlobKind::Tree => &mut self.tree_packer,
        };
        packer.add(id, kind, &sealed, data.len() as u32, compressed.is_some());
        if packer.buf.len() >= self.config.pack_size as usize {
            self.flush_packer(kind, rng)?;
        }
        Ok((id, stored))
    }

    pub(crate) fn flush_packer(&mut self, kind: BlobKind, rng: &mut dyn Rng) -> Result<(), Error> {
        let packer = match kind {
            BlobKind::Data => &mut self.data_packer,
            BlobKind::Tree => &mut self.tree_packer,
        };
        if packer.is_empty() {
            return Ok(());
        }
        let (pid, bytes, entries) = packer.finish(&self.key, rng);
        self.backend.put(FileKind::Pack, &pid.to_hex(), &bytes)?;
        for e in &entries {
            self.index.entry(e.id).or_insert(Loc { pack: pid, entry: *e });
        }
        self.unsaved.push((pid, entries));
        Ok(())
    }

    /// Writes open packs and records them in a new index file.
    pub(crate) fn flush(&mut self, rng: &mut dyn Rng) -> Result<(), Error> {
        self.flush_packer(BlobKind::Data, rng)?;
        self.flush_packer(BlobKind::Tree, rng)?;
        if !self.unsaved.is_empty() {
            let packs = core::mem::take(&mut self.unsaved);
            if let Err(e) = self.write_index(&packs, rng) {
                self.unsaved = packs;
                return Err(e);
            }
        }
        Ok(())
    }

    pub(crate) fn write_index(&mut self, packs: &[(Id, Vec<PackEntry>)], rng: &mut dyn Rng) -> Result<String, Error> {
        let sealed = self.key.seal(rng, INDEX_AAD, &encode_index(packs));
        let name = sha256(&sealed).to_hex();
        self.backend.put(FileKind::Index, &name, &sealed)?;
        Ok(name)
    }

    pub(crate) fn open_blob(&self, entry: &PackEntry, sealed: &[u8]) -> Result<Vec<u8>, Error> {
        let short = entry.id.short();
        let plain = self
            .key
            .open(&blob_aad(&entry.id, entry.kind), sealed, "blob")
            .map_err(|_| Error::Corrupt(alloc::format!("blob {short}: authentication failed")))?;
        let plain = if entry.compressed {
            let mut out = Vec::with_capacity(entry.raw_len as usize);
            ruzstd::decoding::FrameDecoder::new()
                .decode_all_to_vec(&plain, &mut out)
                .map_err(|_| Error::Corrupt(alloc::format!("blob {short}: bad zstd data")))?;
            out
        } else {
            plain
        };
        if plain.len() != entry.raw_len as usize || self.key.blob_id(&plain) != entry.id {
            return Err(Error::Corrupt(alloc::format!("blob {short}: hash mismatch")));
        }
        Ok(plain)
    }

    /// Reads, authenticates, decompresses and hash-checks one blob.
    pub(crate) fn load_blob(&mut self, id: &Id) -> Result<Vec<u8>, Error> {
        let loc = *self.index.get(id).ok_or_else(|| Error::NotFound(alloc::format!("blob {}", id.short())))?;
        let sealed =
            self.backend.get_range(FileKind::Pack, &loc.pack.to_hex(), loc.entry.offset as u64, loc.entry.length)?;
        self.open_blob(&loc.entry, &sealed)
    }

    pub fn load_tree(&mut self, id: &Id) -> Result<Tree, Error> {
        Tree::decode(&self.load_blob(id)?)
    }

    // ---- snapshots ----

    pub(crate) fn save_snapshot(&mut self, snap: &Snapshot, rng: &mut dyn Rng) -> Result<Id, Error> {
        let sealed = self.key.seal(rng, SNAPSHOT_AAD, &snap.encode());
        let id = sha256(&sealed);
        self.backend.put(FileKind::Snapshot, &id.to_hex(), &sealed)?;
        Ok(id)
    }

    pub fn snapshot(&mut self, id: &Id) -> Result<Snapshot, Error> {
        let sealed = self.backend.get(FileKind::Snapshot, &id.to_hex()).map_err(|e| match e {
            Error::NotFound(_) => Error::NoSuchSnapshot(id.to_hex()),
            e => e,
        })?;
        Snapshot::decode(&self.key.open(SNAPSHOT_AAD, &sealed, "snapshot")?)
    }

    /// All snapshots, oldest first.
    pub fn snapshots(&mut self) -> Result<Vec<(Id, Snapshot)>, Error> {
        let mut out = Vec::new();
        for n in self.backend.list(FileKind::Snapshot)? {
            let id = Id::from_hex(&n).ok_or_else(|| Error::Corrupt(alloc::format!("snapshot name {n}")))?;
            out.push((id, self.snapshot(&id)?));
        }
        out.sort_by(|a, b| a.1.time.cmp(&b.1.time).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    /// A snapshot by unique hex prefix, or `latest`.
    pub fn find_snapshot(&mut self, what: &str) -> Result<Id, Error> {
        if what == "latest" {
            return self.snapshots()?.last().map(|s| s.0).ok_or_else(|| Error::NoSuchSnapshot(what.into()));
        }
        let names = self.backend.list(FileKind::Snapshot)?;
        let mut m = names.iter().filter(|n| !what.is_empty() && n.starts_with(what));
        match (m.next(), m.next()) {
            (Some(n), None) => Id::from_hex(n).ok_or_else(|| Error::NoSuchSnapshot(what.into())),
            (Some(_), Some(_)) => Err(Error::Invalid(alloc::format!("snapshot prefix {what} is ambiguous"))),
            _ => Err(Error::NoSuchSnapshot(what.into())),
        }
    }

    /// The node at `path` in a snapshot (`/` gives a directory node for the root tree).
    pub fn node(&mut self, snap: &Id, path: &str) -> Result<Node, Error> {
        let s = self.snapshot(snap)?;
        let mut node = Node {
            name: String::new(),
            kind: NodeKind::Dir,
            size: 0,
            mtime: s.time,
            mode: 0,
            content: Vec::new(),
            subtree: Some(s.tree),
            target: String::new(),
        };
        for c in path_components(path)? {
            let sub = match (node.kind, node.subtree) {
                (NodeKind::Dir, Some(t)) => t,
                _ => return Err(Error::NoSuchPath(path.into())),
            };
            let tree = self.load_tree(&sub)?;
            node = tree.get(c).cloned().ok_or_else(|| Error::NoSuchPath(path.into()))?;
        }
        Ok(node)
    }

    /// Lists a directory of a snapshot (a file path gives that one node).
    pub fn ls(&mut self, snap: &Id, path: &str) -> Result<Vec<Node>, Error> {
        let node = self.node(snap, path)?;
        match (node.kind, node.subtree) {
            (NodeKind::Dir, Some(t)) => Ok(self.load_tree(&t)?.nodes),
            _ => Ok(alloc::vec![node]),
        }
    }

    /// Restores `path` of a snapshot (`/` for everything) into `sink`. Paths given to the
    /// sink start at the restored node's own name (or at the root's children for `/`).
    pub fn restore(
        &mut self,
        snap: &Id,
        path: &str,
        sink: &mut dyn Sink,
        obs: &mut dyn Observer,
    ) -> Result<RestoreSummary, Error> {
        let node = self.node(snap, path)?;
        let mut sum = RestoreSummary::default();
        let mut prog = Progress::default();
        if node.name.is_empty() {
            let tree = self.load_tree(&node.subtree.unwrap_or_default())?;
            for n in &tree.nodes {
                self.restore_node(&n.name, n, sink, obs, &mut sum, &mut prog)?;
            }
        } else {
            let name = node.name.clone();
            self.restore_node(&name, &node, sink, obs, &mut sum, &mut prog)?;
        }
        Ok(sum)
    }

    fn restore_node(
        &mut self,
        rel: &str,
        node: &Node,
        sink: &mut dyn Sink,
        obs: &mut dyn Observer,
        sum: &mut RestoreSummary,
        prog: &mut Progress,
    ) -> Result<(), Error> {
        if obs.cancelled() {
            return Err(Error::Cancelled);
        }
        match node.kind {
            NodeKind::Dir => {
                sink.create_dir(rel, node)?;
                let tree = self.load_tree(&node.subtree.unwrap_or_default())?;
                for n in &tree.nodes {
                    let child = crate::join(rel, &n.name);
                    self.restore_node(&child, n, sink, obs, sum, prog)?;
                }
                sink.finish_dir(rel, node)?;
                sum.dirs += 1;
                prog.dirs += 1;
            }
            NodeKind::File => {
                sink.begin_file(rel, node)?;
                let mut written = 0u64;
                for id in &node.content {
                    if obs.cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let data = self.load_blob(id)?;
                    written += data.len() as u64;
                    sink.write(&data)?;
                    prog.bytes += data.len() as u64;
                }
                if written != node.size {
                    return Err(Error::Corrupt(alloc::format!("{rel}: restored {written} of {} bytes", node.size)));
                }
                sink.end_file(rel, node)?;
                sum.files += 1;
                sum.bytes += written;
                prog.files += 1;
            }
            NodeKind::Symlink => {
                sink.symlink(rel, node)?;
                sum.symlinks += 1;
            }
        }
        prog.current = rel.to_string();
        obs.progress(prog);
        Ok(())
    }

    // ---- locks ----

    pub fn locks(&mut self) -> Result<Vec<(String, Lock)>, Error> {
        let mut out = Vec::new();
        for n in self.backend.list(FileKind::Lock)? {
            let sealed = self.backend.get(FileKind::Lock, &n)?;
            out.push((n, Lock::decode(&self.key.open(LOCK_AAD, &sealed, "lock")?)?));
        }
        Ok(out)
    }

    /// Removes locks older than `before` (all locks with `None`): for locks left behind by
    /// a crash. Returns how many were removed.
    pub fn remove_locks(&mut self, before: Option<Timestamp>) -> Result<usize, Error> {
        let mut n = 0;
        for name in self.backend.list(FileKind::Lock)? {
            let stale = match before {
                None => true,
                Some(t) => {
                    let sealed = self.backend.get(FileKind::Lock, &name)?;
                    match self.key.open(LOCK_AAD, &sealed, "lock").and_then(|p| Lock::decode(&p)) {
                        Ok(l) => l.time < t,
                        Err(_) => true,
                    }
                }
            };
            if stale {
                self.backend.delete(FileKind::Lock, &name)?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub(crate) fn lock(&mut self, exclusive: bool, time: Timestamp, rng: &mut dyn Rng) -> Result<String, Error> {
        for (_, l) in self.locks()? {
            if exclusive || l.exclusive {
                return Err(Error::Locked(alloc::format!(
                    "{} lock held by '{}' since {}s",
                    if l.exclusive { "exclusive" } else { "shared" },
                    l.host,
                    l.time.secs
                )));
            }
        }
        let lock = Lock { exclusive, host: self.host.clone(), time };
        let sealed = self.key.seal(rng, LOCK_AAD, &lock.encode());
        let name = sha256(&sealed).to_hex();
        self.backend.put(FileKind::Lock, &name, &sealed)?;
        Ok(name)
    }

    pub(crate) fn unlock(&mut self, name: &str) -> Result<(), Error> {
        self.backend.delete(FileKind::Lock, name)
    }
}
