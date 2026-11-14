// SPDX-License-Identifier: GPL-3.0-only
//! Backup: walk a [`Source`], chunk files, store new chunks and trees, write a snapshot.

use crate::backend::Backend;
use crate::crypto::Rng;
use crate::model::{BackupSummary, Entry, Node, NodeKind, Observer, Progress, Snapshot, Source, Timestamp, Tree};
use crate::pack::BlobKind;
use crate::repo::Repository;
use crate::{join, normalize_path, path_components, Error, Id};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Which snapshot unchanged files are taken from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParentChoice {
    /// Read every file.
    None,
    /// The newest snapshot with the same host and paths.
    Latest,
    Snapshot(Id),
}

#[derive(Clone, Debug)]
pub struct BackupOptions {
    pub host: String,
    pub tags: Vec<String>,
    /// Snapshot time (the engine has no clock).
    pub time: Timestamp,
    pub parent: ParentChoice,
}

impl Default for BackupOptions {
    fn default() -> Self {
        BackupOptions { host: String::new(), tags: Vec::new(), time: Timestamp::default(), parent: ParentChoice::Latest }
    }
}

enum VNode {
    Leaf(String),
    Dir(BTreeMap<String, VNode>),
}

struct Ctx<'a> {
    src: &'a mut dyn Source,
    rng: &'a mut dyn Rng,
    obs: &'a mut dyn Observer,
    sum: BackupSummary,
    prog: Progress,
    buf: Vec<u8>,
}

impl Ctx<'_> {
    fn check_cancel(&mut self) -> Result<(), Error> {
        if self.obs.cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

fn skippable(e: &Error) -> bool {
    matches!(e, Error::Source(_) | Error::NotFound(_) | Error::Unsupported(_))
}

impl<B: Backend> Repository<B> {
    /// Backs up `paths` (absolute) from `source` and writes a snapshot.
    ///
    /// Unreadable entries below the given paths are skipped with a warning and counted in
    /// `summary.errors`. On cancel (or any error) the chunks stored so far are written to
    /// packs and indexed, so the next backup reuses them; no snapshot is written and the
    /// repository stays consistent.
    pub fn backup(
        &mut self,
        source: &mut dyn Source,
        paths: &[&str],
        opts: &BackupOptions,
        rng: &mut dyn Rng,
        obs: &mut dyn Observer,
    ) -> Result<(Id, Snapshot), Error> {
        if paths.is_empty() {
            return Err(Error::Invalid("nothing to back up".into()));
        }
        let mut norm: Vec<String> = paths.iter().map(|p| normalize_path(p)).collect::<Result<_, _>>()?;
        norm.sort();
        norm.dedup();
        let parent = match &opts.parent {
            ParentChoice::None => None,
            ParentChoice::Snapshot(id) => Some(*id),
            ParentChoice::Latest => self
                .snapshots()?
                .into_iter()
                .rev()
                .find(|(_, s)| s.host == opts.host && s.paths == norm)
                .map(|(id, _)| id),
        };
        let parent_tree = match parent {
            Some(p) => {
                let t = self.snapshot(&p)?.tree;
                Some(self.load_tree(&t)?)
            }
            None => None,
        };
        let lock = self.lock(false, opts.time, rng)?;
        let mut ctx = Ctx { src: source, rng, obs, sum: BackupSummary::default(), prog: Progress::default(), buf: Vec::new() };
        let result = self.backup_root(&norm, parent_tree, &mut ctx);
        // Keep what was stored, whatever happened.
        let flushed = self.flush(ctx.rng);
        let unlocked = self.unlock(&lock);
        let tree = result?;
        flushed?;
        unlocked?;
        let snap = Snapshot {
            time: opts.time,
            tree,
            parent,
            host: opts.host.clone(),
            paths: norm,
            tags: opts.tags.clone(),
            summary: ctx.sum,
        };
        let id = self.save_snapshot(&snap, ctx.rng)?;
        Ok((id, snap))
    }

    fn backup_root(&mut self, paths: &[String], parent: Option<Tree>, ctx: &mut Ctx) -> Result<Id, Error> {
        if paths.iter().any(|p| p == "/") {
            return self.backup_dir("/", parent.as_ref(), ctx);
        }
        let mut root = BTreeMap::new();
        'paths: for p in paths {
            let comps = path_components(p)?;
            let mut map = &mut root;
            for (i, c) in comps.iter().enumerate() {
                if i + 1 == comps.len() {
                    map.insert(c.to_string(), VNode::Leaf(p.clone()));
                    continue 'paths;
                }
                let next = map.entry(c.to_string()).or_insert_with(|| VNode::Dir(BTreeMap::new()));
                match next {
                    VNode::Dir(m) => map = m,
                    VNode::Leaf(_) => continue 'paths, // covered by a shorter path
                }
            }
        }
        self.backup_virtual("/", &root, parent.as_ref(), ctx)
    }

    /// A directory made up only to reach the requested paths.
    fn backup_virtual(
        &mut self,
        vpath: &str,
        children: &BTreeMap<String, VNode>,
        parent: Option<&Tree>,
        ctx: &mut Ctx,
    ) -> Result<Id, Error> {
        let mut nodes = Vec::new();
        for (name, child) in children {
            let path = join(vpath, name);
            let pnode = parent.and_then(|t| t.get(name));
            match child {
                VNode::Leaf(p) => {
                    let mut e = ctx.src.stat(p)?;
                    e.name = name.clone();
                    nodes.push(self.backup_entry(p, &e, pnode, ctx)?);
                }
                VNode::Dir(m) => {
                    let mut e = ctx.src.stat(&path).unwrap_or(Entry {
                        name: String::new(),
                        kind: NodeKind::Dir,
                        size: 0,
                        mtime: Timestamp::default(),
                        mode: 0,
                    });
                    e.name = name.clone();
                    e.kind = NodeKind::Dir;
                    e.size = 0;
                    let ptree = self.parent_subtree(pnode);
                    let mut node = Node::from_entry(&e);
                    node.subtree = Some(self.backup_virtual(&path, m, ptree.as_ref(), ctx)?);
                    nodes.push(node);
                }
            }
        }
        self.save_tree(Tree { nodes }, ctx)
    }

    fn parent_subtree(&mut self, pnode: Option<&Node>) -> Option<Tree> {
        match pnode {
            Some(Node { kind: NodeKind::Dir, subtree: Some(t), .. }) => self.load_tree(t).ok(),
            _ => None,
        }
    }

    fn save_tree(&mut self, tree: Tree, ctx: &mut Ctx) -> Result<Id, Error> {
        let (id, stored) = self.save_blob(ctx.rng, BlobKind::Tree, &tree.encode())?;
        if stored > 0 {
            ctx.sum.blobs_added += 1;
            ctx.sum.bytes_stored += stored;
        }
        Ok(id)
    }

    fn backup_dir(&mut self, path: &str, parent: Option<&Tree>, ctx: &mut Ctx) -> Result<Id, Error> {
        let mut entries = ctx.src.list(path)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries.dedup_by(|a, b| a.name == b.name);
        let mut nodes = Vec::with_capacity(entries.len());
        for e in entries {
            if e.name.is_empty() || e.name.contains('/') || e.name == "." || e.name == ".." {
                continue;
            }
            let child = join(path, &e.name);
            match self.backup_entry(&child, &e, parent.and_then(|t| t.get(&e.name)), ctx) {
                Ok(n) => nodes.push(n),
                Err(err) if skippable(&err) => {
                    ctx.sum.errors += 1;
                    ctx.obs.warning(&alloc::format!("skipped {child}: {err}"));
                }
                Err(err) => return Err(err),
            }
        }
        ctx.sum.dirs += 1;
        ctx.prog.dirs += 1;
        self.save_tree(Tree { nodes }, ctx)
    }

    fn backup_entry(&mut self, path: &str, e: &Entry, pnode: Option<&Node>, ctx: &mut Ctx) -> Result<Node, Error> {
        ctx.check_cancel()?;
        let mut node = Node::from_entry(e);
        match e.kind {
            NodeKind::Dir => {
                node.size = 0;
                let ptree = self.parent_subtree(pnode);
                node.subtree = Some(self.backup_dir(path, ptree.as_ref(), ctx)?);
            }
            NodeKind::File => {
                let same = pnode.filter(|p| {
                    p.kind == NodeKind::File
                        && p.size == e.size
                        && p.mtime == e.mtime
                        && p.content.iter().all(|c| self.has_blob(c))
                });
                if let Some(p) = same {
                    node.content = p.content.clone();
                    ctx.sum.files_unchanged += 1;
                } else {
                    let (content, size) = self.backup_file(path, ctx)?;
                    node.content = content;
                    node.size = size;
                    if pnode.is_some() {
                        ctx.sum.files_changed += 1;
                    } else {
                        ctx.sum.files_new += 1;
                    }
                }
                ctx.sum.bytes_total += node.size;
                ctx.prog.files += 1;
            }
            NodeKind::Symlink => {
                node.size = 0;
                node.target = ctx.src.read_link(path)?;
            }
        }
        ctx.prog.current = path.to_string();
        ctx.obs.progress(&ctx.prog);
        Ok(node)
    }

    fn backup_file(&mut self, path: &str, ctx: &mut Ctx) -> Result<(Vec<Id>, u64), Error> {
        let max = self.chunker.max_size();
        let mut buf = core::mem::take(&mut ctx.buf);
        buf.clear();
        let mut offset = 0u64;
        let mut eof = false;
        let mut content = Vec::new();
        let r = (|| {
            loop {
                while !eof && buf.len() < max {
                    let old = buf.len();
                    buf.resize(max, 0);
                    let n = ctx.src.read(path, offset, &mut buf[old..])?;
                    buf.truncate(old + n);
                    offset += n as u64;
                    ctx.sum.bytes_read += n as u64;
                    if n == 0 {
                        eof = true;
                    }
                }
                if buf.is_empty() {
                    return Ok(());
                }
                let cut = self.chunker.cut(&buf, eof);
                let (id, stored) = self.save_blob(ctx.rng, BlobKind::Data, &buf[..cut])?;
                if stored > 0 {
                    ctx.sum.blobs_added += 1;
                    ctx.sum.bytes_added += cut as u64;
                    ctx.sum.bytes_stored += stored;
                    ctx.prog.bytes_added += cut as u64;
                }
                ctx.prog.bytes += cut as u64;
                content.push(id);
                buf.drain(..cut);
                ctx.obs.progress(&ctx.prog);
                ctx.check_cancel()?;
            }
        })();
        ctx.buf = buf;
        r.map(|()| (content, offset))
    }
}
