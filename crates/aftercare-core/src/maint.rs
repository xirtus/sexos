// SPDX-License-Identifier: GPL-3.0-only
//! Maintenance: forget (keep policy), prune (drop and repack unreferenced data), check.

use crate::backend::{Backend, FileKind};
use crate::crypto::{sha256, Rng};
use crate::model::{NodeKind, Snapshot, Timestamp};
use crate::pack::{decode_pack_header, split_pack, BlobKind, PackEntry, Packer, SNAPSHOT_AAD};
use crate::repo::Repository;
use crate::{Error, Id};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

/// Which snapshots `forget` keeps. Snapshots are grouped by host and paths; in each group,
/// newest first, a snapshot is kept if it is among the `last` newest, or is the newest of
/// one of the `hourly`/`daily`/`weekly`/`monthly`/`yearly` most recent periods that have a
/// snapshot, or carries one of `tags`. Everything else is forgotten.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeepPolicy {
    pub last: u32,
    pub hourly: u32,
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
    pub tags: Vec<String>,
    /// Offset added to UTC before cutting days (local midnight); weeks start on Monday.
    pub utc_offset_secs: i32,
}

impl KeepPolicy {
    fn is_empty(&self) -> bool {
        self.last == 0
            && self.hourly == 0
            && self.daily == 0
            && self.weekly == 0
            && self.monthly == 0
            && self.yearly == 0
            && self.tags.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgetPlan {
    pub keep: Vec<Id>,
    pub remove: Vec<Id>,
}

/// (year, month 1-12) of a day count since 1970-01-01 (H. Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m)
}

fn buckets(t: Timestamp, off: i32) -> [i64; 5] {
    let s = t.secs + off as i64;
    let day = s.div_euclid(86_400);
    let (y, m) = civil(day);
    // 1970-01-01 was a Thursday: Monday-based weekday is (day + 3) mod 7.
    let monday = day - (day + 3).rem_euclid(7);
    [s.div_euclid(3600), day, monday, y * 12 + m - 1, y]
}

/// Applies a policy to snapshots (any order).
pub fn apply_keep_policy(snaps: &[(Id, Snapshot)], p: &KeepPolicy) -> ForgetPlan {
    let mut groups: BTreeMap<(String, Vec<String>), Vec<&(Id, Snapshot)>> = BTreeMap::new();
    for s in snaps {
        groups.entry((s.1.host.clone(), s.1.paths.clone())).or_default().push(s);
    }
    let mut out = ForgetPlan::default();
    for (_, mut g) in groups {
        g.sort_by(|a, b| b.1.time.cmp(&a.1.time).then(b.0.cmp(&a.0)));
        let mut last = p.last;
        let mut counts = [p.hourly, p.daily, p.weekly, p.monthly, p.yearly];
        let mut seen: [Option<i64>; 5] = [None; 5];
        for (id, s) in g {
            let mut keep = false;
            if last > 0 {
                last -= 1;
                keep = true;
            }
            let b = buckets(s.time, p.utc_offset_secs);
            for i in 0..5 {
                if counts[i] > 0 && seen[i] != Some(b[i]) {
                    seen[i] = Some(b[i]);
                    counts[i] -= 1;
                    keep = true;
                }
            }
            if s.tags.iter().any(|t| p.tags.contains(t)) {
                keep = true;
            }
            if keep { &mut out.keep } else { &mut out.remove }.push(*id);
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct PruneOptions {
    /// Time written in the exclusive lock.
    pub now: Timestamp,
    /// Repack a pack when more than this percentage of its bytes is unreferenced.
    pub max_unused_percent: u32,
    /// Only compute the summary.
    pub dry_run: bool,
}

impl Default for PruneOptions {
    fn default() -> Self {
        PruneOptions { now: Timestamp::default(), max_unused_percent: 10, dry_run: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruneSummary {
    pub packs_deleted: u64,
    pub packs_repacked: u64,
    pub packs_written: u64,
    /// Packs found in the store but in no index (left by an interrupted run).
    pub packs_orphaned: u64,
    pub blobs_removed: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// Findings of [`Repository::check`].
#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    pub errors: Vec<String>,
    /// Harmless findings, e.g. unreferenced packs that prune would remove.
    pub warnings: Vec<String>,
    pub snapshots: u64,
    pub trees: u64,
    pub packs: u64,
    /// Blobs read and verified (with `read_data`).
    pub blobs_verified: u64,
}

impl CheckReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

enum Fate {
    Keep,
    Repack,
    Delete,
}

impl<B: Backend> Repository<B> {
    /// Which snapshots a policy keeps and removes, without changing anything.
    pub fn plan_forget(&mut self, policy: &KeepPolicy) -> Result<ForgetPlan, Error> {
        if policy.is_empty() {
            return Err(Error::Invalid("keep policy keeps nothing".into()));
        }
        Ok(apply_keep_policy(&self.snapshots()?, policy))
    }

    /// Removes the snapshots a policy does not keep (their data stays until [`prune`](Self::prune)).
    pub fn forget(&mut self, policy: &KeepPolicy, now: Timestamp, rng: &mut dyn Rng) -> Result<ForgetPlan, Error> {
        let p = self.plan_forget(policy)?;
        self.forget_snapshots(&p.remove, now, rng)?;
        Ok(p)
    }

    /// Removes the given snapshots.
    pub fn forget_snapshots(&mut self, ids: &[Id], now: Timestamp, rng: &mut dyn Rng) -> Result<(), Error> {
        let lock = self.lock(true, now, rng)?;
        let r = ids.iter().try_for_each(|id| self.backend.delete(FileKind::Snapshot, &id.to_hex()));
        self.unlock(&lock)?;
        r
    }

    /// Every tree and data blob reachable from a snapshot.
    fn referenced(&mut self) -> Result<BTreeSet<Id>, Error> {
        let mut used = BTreeSet::new();
        let mut stack: Vec<Id> = self.snapshots()?.into_iter().map(|(_, s)| s.tree).collect();
        while let Some(t) = stack.pop() {
            if !used.insert(t) {
                continue;
            }
            for n in self.load_tree(&t)?.nodes {
                match n.kind {
                    NodeKind::File => used.extend(n.content),
                    NodeKind::Dir => stack.extend(n.subtree),
                    NodeKind::Symlink => {}
                }
            }
        }
        Ok(used)
    }

    /// Deletes packs nothing references and repacks those mostly unreferenced. New packs
    /// and the new index are written before anything is deleted, so an interruption leaves
    /// a usable repository.
    pub fn prune(&mut self, opts: &PruneOptions, rng: &mut dyn Rng) -> Result<PruneSummary, Error> {
        self.flush(rng)?;
        let lock = self.lock(true, opts.now, rng)?;
        let r = self.prune_locked(opts, rng);
        let u = self.unlock(&lock);
        let s = r?;
        u?;
        Ok(s)
    }

    fn prune_locked(&mut self, opts: &PruneOptions, rng: &mut dyn Rng) -> Result<PruneSummary, Error> {
        let used = self.referenced()?;
        let index_files = self.read_index_files()?;
        let mut packs: BTreeMap<Id, Vec<PackEntry>> = BTreeMap::new();
        for (_, list) in &index_files {
            for (pid, entries) in list {
                let v = packs.entry(*pid).or_default();
                for e in entries {
                    if !v.iter().any(|x| x.id == e.id) {
                        v.push(*e);
                    }
                }
            }
        }
        let mut sum = PruneSummary::default();
        let mut stored: BTreeMap<Id, u64> = BTreeMap::new();
        for n in self.backend.list(FileKind::Pack)? {
            let size = self.backend.size(FileKind::Pack, &n)?;
            sum.bytes_before += size;
            match Id::from_hex(&n) {
                Some(id) => {
                    stored.insert(id, size);
                }
                None => sum.packs_orphaned += 1,
            }
        }
        packs.retain(|p, _| stored.contains_key(p));
        let orphans: Vec<Id> = stored.keys().filter(|p| !packs.contains_key(p)).copied().collect();
        sum.packs_orphaned += orphans.len() as u64;

        // Pass 1: decide each pack's fate by its referenced bytes.
        let mut fate: BTreeMap<Id, Fate> = BTreeMap::new();
        let mut first_seen: BTreeSet<Id> = BTreeSet::new();
        for (pid, entries) in &packs {
            let (mut live, mut dead) = (0u64, 0u64);
            for e in entries {
                if used.contains(&e.id) && first_seen.insert(e.id) {
                    live += e.length as u64;
                } else {
                    dead += e.length as u64;
                }
            }
            let f = if live == 0 {
                Fate::Delete
            } else if dead * 100 > (live + dead) * opts.max_unused_percent as u64 {
                Fate::Repack
            } else {
                Fate::Keep
            };
            fate.insert(*pid, f);
        }
        // Pass 2: what kept packs hold, and what must be copied out of repacked ones.
        let mut have: BTreeSet<Id> = BTreeSet::new();
        let mut new_index: Vec<(Id, Vec<PackEntry>)> = Vec::new();
        for (pid, entries) in &packs {
            if matches!(fate[pid], Fate::Keep) {
                let kept: Vec<PackEntry> = entries.iter().filter(|e| have.insert(e.id)).copied().collect();
                new_index.push((*pid, kept));
            }
        }
        let mut copy: Vec<(Id, PackEntry)> = Vec::new();
        for (pid, entries) in &packs {
            if matches!(fate[pid], Fate::Repack) {
                for e in entries {
                    if used.contains(&e.id) && have.insert(e.id) {
                        copy.push((*pid, *e));
                    }
                }
            }
        }
        if let Some(missing) = used.iter().find(|id| !have.contains(id)) {
            return Err(Error::Corrupt(alloc::format!(
                "blob {} is referenced but in no pack; run check, prune refuses to delete anything",
                missing.short()
            )));
        }
        let removable: Vec<Id> =
            fate.iter().filter(|(_, f)| !matches!(f, Fate::Keep)).map(|(p, _)| *p).chain(orphans).collect();
        sum.packs_deleted = fate.values().filter(|f| matches!(f, Fate::Delete)).count() as u64;
        sum.packs_repacked = fate.values().filter(|f| matches!(f, Fate::Repack)).count() as u64;
        sum.blobs_removed = packs.values().map(|v| v.len() as u64).sum::<u64>()
            - new_index.iter().map(|(_, v)| v.len() as u64).sum::<u64>()
            - copy.len() as u64;
        if opts.dry_run {
            let freed: u64 = removable.iter().map(|p| stored.get(p).copied().unwrap_or(0)).sum();
            sum.bytes_after = sum.bytes_before - freed + copy.iter().map(|(_, e)| e.length as u64).sum::<u64>();
            return Ok(sum);
        }

        // Copy live blobs as stored: sealed blobs are bound to their id, not to a pack.
        let mut packers = [Packer::default(), Packer::default()];
        let mut cur: Option<(Id, Vec<u8>)> = None;
        for (pid, e) in &copy {
            if cur.as_ref().map(|c| c.0) != Some(*pid) {
                cur = Some((*pid, self.backend.get(FileKind::Pack, &pid.to_hex())?));
            }
            let bytes = &cur.as_ref().unwrap().1;
            let sealed = bytes
                .get(e.offset as usize..e.offset as usize + e.length as usize)
                .ok_or_else(|| Error::Corrupt(alloc::format!("pack {}: blob past end", pid.short())))?;
            let k = (e.kind == BlobKind::Tree) as usize;
            packers[k].add(e.id, e.kind, sealed, e.raw_len, e.compressed);
            if packers[k].buf.len() >= self.config.pack_size as usize {
                self.put_pack(&mut packers[k], &mut new_index, rng, &mut sum)?;
            }
        }
        for p in packers.iter_mut() {
            if !p.is_empty() {
                self.put_pack(p, &mut new_index, rng, &mut sum)?;
            }
        }
        let new_name = self.write_index(&new_index, rng)?;
        for (n, _) in &index_files {
            if *n != new_name {
                self.backend.delete(FileKind::Index, n)?;
            }
        }
        for p in &removable {
            self.backend.delete(FileKind::Pack, &p.to_hex())?;
        }
        self.reload_index()?;
        for n in self.backend.list(FileKind::Pack)? {
            sum.bytes_after += self.backend.size(FileKind::Pack, &n)?;
        }
        Ok(sum)
    }

    fn put_pack(
        &mut self,
        p: &mut Packer,
        index: &mut Vec<(Id, Vec<PackEntry>)>,
        rng: &mut dyn Rng,
        sum: &mut PruneSummary,
    ) -> Result<(), Error> {
        let (pid, bytes, entries) = p.finish(&self.key, rng);
        self.backend.put(FileKind::Pack, &pid.to_hex(), &bytes)?;
        index.push((pid, entries));
        sum.packs_written += 1;
        Ok(())
    }

    /// Checks structure: index files, pack headers against the index, snapshots, and that
    /// every tree loads and every referenced blob is indexed. With `read_data` every pack is
    /// read whole: its name must be its SHA-256 and every blob must authenticate and match
    /// its keyed hash.
    pub fn check(&mut self, read_data: bool) -> Result<CheckReport, Error> {
        let mut rep = CheckReport::default();
        let mut packs: BTreeMap<Id, Vec<PackEntry>> = BTreeMap::new();
        let mut names = self.backend.list(FileKind::Index)?;
        names.sort();
        for n in names {
            let r = self.backend.get(FileKind::Index, &n).and_then(|sealed| {
                if sha256(&sealed).to_hex() != n {
                    return Err(Error::Corrupt("name is not its hash".into()));
                }
                let plain = self.key.open(crate::pack::INDEX_AAD, &sealed, "index")?;
                crate::pack::decode_index(&plain)
            });
            match r {
                Ok(list) => {
                    for (p, es) in list {
                        packs.entry(p).or_default().extend(es);
                    }
                }
                Err(e) => rep.errors.push(alloc::format!("index {n}: {e}")),
            }
        }
        let stored: BTreeSet<String> = self.backend.list(FileKind::Pack)?.into_iter().collect();
        for n in &stored {
            if !Id::from_hex(n).is_some_and(|id| packs.contains_key(&id)) {
                rep.warnings.push(alloc::format!("pack {n} is in no index (prune removes it)"));
            }
        }
        for (pid, entries) in &packs {
            let name = pid.to_hex();
            rep.packs += 1;
            if !stored.contains(&name) {
                rep.errors.push(alloc::format!("pack {name} is indexed but missing"));
                continue;
            }
            let r = if read_data { self.check_pack_data(pid, &mut rep) } else { self.read_pack_header(pid) };
            match r {
                Ok(header) => {
                    for e in entries {
                        if !header.contains(e) {
                            rep.errors.push(alloc::format!(
                                "pack {}: index entry for blob {} does not match the pack",
                                pid.short(),
                                e.id.short()
                            ));
                        }
                    }
                }
                Err(e) => rep.errors.push(alloc::format!("pack {}: {e}", pid.short())),
            }
        }
        // Snapshots and the trees under them.
        let mut seen = BTreeSet::new();
        for n in self.backend.list(FileKind::Snapshot)? {
            rep.snapshots += 1;
            let r = self.backend.get(FileKind::Snapshot, &n).and_then(|sealed| {
                if sha256(&sealed).to_hex() != n {
                    return Err(Error::Corrupt("name is not its hash".into()));
                }
                Snapshot::decode(&self.key.open(SNAPSHOT_AAD, &sealed, "snapshot")?)
            });
            let s = match r {
                Ok(s) => s,
                Err(e) => {
                    rep.errors.push(alloc::format!("snapshot {n}: {e}"));
                    continue;
                }
            };
            let mut stack = alloc::vec![(s.tree, String::from("/"))];
            while let Some((t, path)) = stack.pop() {
                if !seen.insert(t) {
                    continue;
                }
                rep.trees += 1;
                let tree = match self.load_tree(&t) {
                    Ok(t) => t,
                    Err(e) => {
                        rep.errors.push(alloc::format!("snapshot {}: tree {path}: {e}", &n[..8]));
                        continue;
                    }
                };
                for node in tree.nodes {
                    let p = crate::join(&path, &node.name);
                    match node.kind {
                        NodeKind::Dir => match node.subtree {
                            Some(st) => stack.push((st, p)),
                            None => rep.errors.push(alloc::format!("{p}: directory without tree")),
                        },
                        NodeKind::File => {
                            for c in &node.content {
                                if !self.index.contains_key(c) {
                                    rep.errors.push(alloc::format!("{p}: chunk {} not in index", c.short()));
                                }
                            }
                        }
                        NodeKind::Symlink => {}
                    }
                }
            }
        }
        Ok(rep)
    }

    fn read_pack_header(&mut self, pid: &Id) -> Result<Vec<PackEntry>, Error> {
        let name = pid.to_hex();
        let size = self.backend.size(FileKind::Pack, &name)?;
        if size < 4 {
            return Err(Error::Corrupt("too short".into()));
        }
        let tail = self.backend.get_range(FileKind::Pack, &name, size - 4, 4)?;
        let hl = u32::from_le_bytes(tail[..4].try_into().unwrap()) as u64;
        if hl + 4 > size {
            return Err(Error::Corrupt("header length past start".into()));
        }
        let sealed = self.backend.get_range(FileKind::Pack, &name, size - 4 - hl, hl as u32)?;
        decode_pack_header(&self.key, &sealed, size - 4 - hl)
    }

    fn check_pack_data(&mut self, pid: &Id, rep: &mut CheckReport) -> Result<Vec<PackEntry>, Error> {
        let bytes = self.backend.get(FileKind::Pack, &pid.to_hex())?;
        if sha256(&bytes) != *pid {
            rep.errors.push(alloc::format!("pack {}: content does not match its name (SHA-256)", pid.short()));
        }
        let (start, sealed) = split_pack(&bytes)?;
        let header = decode_pack_header(&self.key, sealed, start as u64)?;
        for e in &header {
            let s = &bytes[e.offset as usize..e.offset as usize + e.length as usize];
            match self.open_blob(e, s) {
                Ok(_) => rep.blobs_verified += 1,
                Err(err) => rep.errors.push(alloc::format!("pack {}: {err}", pid.short())),
            }
        }
        Ok(header)
    }
}
