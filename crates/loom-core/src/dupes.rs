// SPDX-License-Identifier: GPL-3.0-only
//! Duplicate files: grouped by size, then by a hash of the first and last 4 KiB, then by a
//! SHA-256 of everything. Most files differ in size or in their ends, so few are read whole.
//!
//! [`Finder`] works in steps with a byte budget, so Loom can run it between keys and show
//! progress; [`find`] runs it to the end. Hard links (same nonzero file id) are one file,
//! not duplicates. [`same_name`] reports files that share a name but not their contents.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use sha2::{Digest as _, Sha256};

use crate::hash::{Digest, CHUNK};
use crate::ReadAt;

/// A file to consider: its path, size and file id (0 when unknown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File { pub path: Vec<u8>, pub size: u64, pub id: u64 }

impl File {
    pub fn new(path: &[u8], size: u64) -> File { File { path: path.to_vec(), size, id: 0 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Smaller files are left out; 0 also groups empty files.
    pub min_size: u64,
    /// Bytes hashed from each end in the partial pass.
    pub edge: usize,
}
impl Default for Options { fn default() -> Self { Options { min_size: 1, edge: 4096 } } }

/// Files with the same contents. `files` index the list given to the finder, by path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group { pub size: u64, pub digest: Digest, pub files: Vec<usize>, pub wasted: u64 }

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Report {
    /// Largest waste first.
    pub groups: Vec<Group>,
    /// Bytes that removing all but one of each group would free.
    pub wasted: u64,
    /// Files that couldn't be read, or changed size while being read.
    pub unreadable: Vec<usize>,
    /// Hard links left out because another link to the same file is in the list.
    pub links: Vec<usize>,
    pub bytes_read: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Partial, Full, Done }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Running, Done }

pub struct Finder {
    files: Vec<File>,
    opts: Options,
    stage: Stage,
    todo: Vec<usize>,
    next: usize,
    /// (size, partial digest, whole file already hashed) per file.
    partial: BTreeMap<usize, (Digest, bool)>,
    full: BTreeMap<usize, Digest>,
    current: Option<(usize, Sha256, u64)>,
    report: Report,
    buf: Vec<u8>,
    /// Bytes still to read, as far as known now.
    pub remaining: u64,
}

impl Finder {
    pub fn new(files: Vec<File>, opts: Options) -> Finder {
        let mut seen_ids: BTreeMap<u64, usize> = BTreeMap::new();
        let mut by_size: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        let mut links = Vec::new();
        for (i, f) in files.iter().enumerate() {
            if f.size < opts.min_size { continue; }
            if f.id != 0 {
                if seen_ids.contains_key(&f.id) { links.push(i); continue; }
                seen_ids.insert(f.id, i);
            }
            by_size.entry(f.size).or_default().push(i);
        }
        // Larger sizes first: they waste the most and show up early in the results.
        let todo: Vec<usize> = by_size.into_values().rev().filter(|g| g.len() > 1).flatten().collect();
        let remaining = todo.iter().map(|&i| files[i].size.min(2 * opts.edge as u64)).sum();
        Finder { files, opts, stage: Stage::Partial, todo, next: 0, partial: BTreeMap::new(), full: BTreeMap::new(), current: None,
            report: Report { links, ..Report::default() }, buf: vec![0u8; CHUNK], remaining }
    }

    pub fn stage(&self) -> Stage { self.stage }
    pub fn files(&self) -> &[File] { &self.files }

    fn read_exact(&mut self, src: &mut dyn ReadAt, i: usize, at: u64, len: usize) -> Option<()> {
        let mut got = 0;
        while got < len {
            let n = src.read_at(&self.files[i].path, at + got as u64, &mut self.buf[got..len])?;
            if n == 0 { return None; }
            got += n;
        }
        self.report.bytes_read += len as u64;
        Some(())
    }

    fn partial_one(&mut self, src: &mut dyn ReadAt, i: usize) -> Option<(Digest, bool)> {
        let size = self.files[i].size;
        let edge = self.opts.edge;
        if size <= 2 * edge as u64 {
            let n = size as usize;
            if self.buf.len() < n { self.buf.resize(n, 0); }
            self.read_exact(src, i, 0, n)?;
            // A short file must end where it should; one byte more means it grew.
            let mut probe = [0u8; 1];
            if src.read_at(&self.files[i].path, size, &mut probe)? != 0 { return None; }
            return Some((Sha256::digest(&self.buf[..n]).into(), true));
        }
        let mut h = Sha256::new();
        h.update(size.to_le_bytes());
        self.read_exact(src, i, 0, edge)?;
        h.update(&self.buf[..edge]);
        self.read_exact(src, i, size - edge as u64, edge)?;
        h.update(&self.buf[..edge]);
        Some((h.finalize().into(), false))
    }

    /// Works until about `budget` bytes have been read, or to the end.
    pub fn step(&mut self, src: &mut dyn ReadAt, budget: u64) -> Step {
        let start = self.report.bytes_read;
        while self.report.bytes_read - start < budget.max(1) {
            match self.stage {
                Stage::Done => return Step::Done,
                Stage::Partial => {
                    if self.next >= self.todo.len() { self.end_partial(); continue; }
                    let i = self.todo[self.next];
                    self.next += 1;
                    let before = self.report.bytes_read;
                    match self.partial_one(src, i) {
                        Some(d) => { self.partial.insert(i, d); }
                        None => self.report.unreadable.push(i),
                    }
                    let est = self.files[i].size.min(2 * self.opts.edge as u64);
                    self.remaining = self.remaining.saturating_sub(est.max(self.report.bytes_read - before));
                }
                Stage::Full => {
                    if self.current.is_none() {
                        if self.next >= self.todo.len() { self.finish(); continue; }
                        self.current = Some((self.todo[self.next], Sha256::new(), 0));
                        self.next += 1;
                    }
                    let Some((i, mut h, at)) = self.current.take() else { continue };
                    let size = self.files[i].size;
                    let n = ((size - at) as usize).min(CHUNK);
                    if n == 0 {
                        let mut probe = [0u8; 1];
                        match src.read_at(&self.files[i].path, size, &mut probe) {
                            Some(0) => { self.full.insert(i, h.finalize().into()); }
                            _ => self.report.unreadable.push(i),
                        }
                        continue;
                    }
                    if self.read_exact(src, i, at, n).is_none() {
                        self.report.unreadable.push(i);
                        self.remaining = self.remaining.saturating_sub(size - at);
                        continue;
                    }
                    h.update(&self.buf[..n]);
                    self.remaining = self.remaining.saturating_sub(n as u64);
                    self.current = Some((i, h, at + n as u64));
                }
            }
        }
        if self.stage == Stage::Done { Step::Done } else { Step::Running }
    }

    fn end_partial(&mut self) {
        let mut groups: BTreeMap<(u64, Digest), Vec<usize>> = BTreeMap::new();
        for (&i, &(d, _)) in &self.partial { groups.entry((self.files[i].size, d)).or_default().push(i); }
        let mut todo = Vec::new();
        for ((_, d), g) in groups.into_iter().rev() {
            if g.len() < 2 { continue; }
            if self.partial[&g[0]].1 {
                // Small files were hashed whole already.
                for &i in &g { self.full.insert(i, d); }
            } else {
                todo.extend(g);
            }
        }
        self.remaining = todo.iter().map(|&i| self.files[i].size).sum();
        self.todo = todo;
        self.next = 0;
        self.stage = Stage::Full;
    }

    fn finish(&mut self) {
        let mut groups: BTreeMap<(u64, Digest), Vec<usize>> = BTreeMap::new();
        for (&i, &d) in &self.full { groups.entry((self.files[i].size, d)).or_default().push(i); }
        let files = &self.files;
        let mut out: Vec<Group> = groups.into_iter().filter(|(_, g)| g.len() > 1).map(|((size, digest), mut g)| {
            g.sort_by(|a, b| files[*a].path.cmp(&files[*b].path));
            Group { size, digest, wasted: size * (g.len() as u64 - 1), files: g }
        }).collect();
        out.sort_by(|a, b| b.wasted.cmp(&a.wasted).then_with(|| files[a.files[0]].path.cmp(&files[b.files[0]].path)));
        self.report.wasted = out.iter().map(|g| g.wasted).sum();
        self.report.groups = out;
        self.report.unreadable.sort_unstable();
        self.remaining = 0;
        self.stage = Stage::Done;
    }

    pub fn report(&self) -> &Report { &self.report }
    pub fn into_report(self) -> (Vec<File>, Report) { (self.files, self.report) }
}

/// Runs a [`Finder`] to the end.
pub fn find(files: Vec<File>, opts: Options, src: &mut dyn ReadAt) -> (Vec<File>, Report) {
    let mut f = Finder::new(files, opts);
    while f.step(src, u64::MAX) == Step::Running {}
    f.into_report()
}

/// Files that share a name (last path part) but not their contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameClash { pub name: Vec<u8>, pub files: Vec<usize>, pub versions: usize }

/// Same name, different content, from a finished report over the same `files`.
pub fn same_name(files: &[File], report: &Report) -> Vec<NameClash> {
    // What each file's contents are known as: its duplicate group, the file it is a hard
    // link of, or (empty files) all the same; otherwise itself.
    let mut key: Vec<(u8, u64)> = (0..files.len()).map(|i| (0u8, i as u64)).collect();
    for (g, grp) in report.groups.iter().enumerate() {
        for &i in &grp.files { key[i] = (1, g as u64); }
    }
    let mut first_by_id: BTreeMap<u64, usize> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        if f.size == 0 { key[i] = (2, 0); continue; }
        if f.id == 0 { continue; }
        match first_by_id.get(&f.id) { Some(&j) => key[i] = key[j], None => { first_by_id.insert(f.id, i); } }
    }
    let mut by_name: BTreeMap<&[u8], Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() { by_name.entry(crate::base_name(&f.path)).or_default().push(i); }
    by_name.into_iter().filter_map(|(name, list)| {
        let mut keys: Vec<(u8, u64)> = list.iter().map(|&i| key[i]).collect();
        keys.sort_unstable();
        keys.dedup();
        (list.len() > 1 && keys.len() > 1).then(|| NameClash { name: name.to_vec(), files: list, versions: keys.len() })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::mem::MemFs;
    use crate::hash::sha256;

    fn data(seed: u32, len: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..len).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }).collect()
    }

    fn setup(fs: &mut MemFs, list: &[(&str, Vec<u8>)]) -> Vec<File> {
        list.iter().map(|(p, d)| { fs.add(p.as_bytes(), d); File::new(p.as_bytes(), d.len() as u64) }).collect()
    }

    #[test]
    fn groups_and_waste() {
        let mut fs = MemFs::default();
        let big = data(1, 100_000);
        let mut big_mid = big.clone();
        big_mid[50_000] ^= 1; // same size, same ends, different middle
        let small = data(2, 3000);
        let files = setup(&mut fs, &[
            ("a/big1", big.clone()), ("b/big2", big.clone()), ("c/big3", big.clone()), ("d/bigmid", big_mid),
            ("a/small", small.clone()), ("b/small", small.clone()),
            ("lonely", data(3, 777)), ("same-size-a", data(4, 500)), ("same-size-b", data(5, 500)),
            ("empty1", Vec::new()), ("empty2", Vec::new()),
        ]);
        let (files, r) = find(files, Options::default(), &mut fs);
        assert_eq!(r.groups.len(), 2);
        let g = &r.groups[0];
        assert_eq!(g.size, 100_000);
        assert_eq!(g.digest, sha256(&big));
        assert_eq!(g.files.iter().map(|&i| &files[i].path[..]).collect::<Vec<_>>(), [&b"a/big1"[..], b"b/big2", b"c/big3"]);
        assert_eq!(g.wasted, 200_000);
        assert_eq!(r.groups[1].files.len(), 2);
        assert_eq!(r.groups[1].digest, sha256(&small));
        assert_eq!(r.wasted, 203_000);
        assert!(r.unreadable.is_empty());
        // 4 big files partially (8 KiB each), 3 big ones that still matched in full plus
        // the one differing in the middle, the small and same-size files whole.
        assert_eq!(r.bytes_read, 4 * 8192 + 4 * 100_000 + 2 * 3000 + 2 * 500);
        // Empty files group only when asked.
        let (_, r0) = find(files.clone(), Options { min_size: 0, ..Options::default() }, &mut fs);
        assert_eq!(r0.groups.len(), 3);
        assert_eq!(r0.groups[2].size, 0);
    }

    #[test]
    fn partial_hash_spares_reads() {
        let mut fs = MemFs::default();
        // 50 files of 1 MiB, each with distinct first bytes: only edges get read.
        let list: Vec<(String, Vec<u8>)> = (0..50).map(|i| (format!("f{i}"), data(i, 1 << 20))).collect();
        let files: Vec<File> = list.iter().map(|(p, d)| { fs.add(p.as_bytes(), d); File::new(p.as_bytes(), d.len() as u64) }).collect();
        let (_, r) = find(files, Options::default(), &mut fs);
        assert!(r.groups.is_empty());
        assert_eq!(r.bytes_read, 50 * 8192);
    }

    #[test]
    fn stepwise_matches_one_go_and_handles_trouble() {
        let mut fs = MemFs::default();
        let d = data(9, 300_000);
        let mut files = setup(&mut fs, &[("x1", d.clone()), ("x2", d.clone()), ("x3", d.clone()), ("gone", d.clone()), ("grew", d.clone()), ("link", d.clone())]);
        fs.files.remove(&b"gone"[..].to_vec());
        let mut longer = d.clone();
        longer.push(0);
        fs.files.insert(b"grew".to_vec(), longer);
        files[0].id = 77;
        files[5].id = 77; // a hard link of x1
        let mut f = Finder::new(files.clone(), Options::default());
        let mut steps = 0;
        while f.step(&mut fs, 10_000) == Step::Running { steps += 1; assert!(steps < 10_000); }
        assert!(steps > 20, "ran in many small steps");
        let r = f.report().clone();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].files, [0, 1, 2]);
        assert_eq!(r.unreadable, [3, 4]);
        assert_eq!(r.links, [5]);
        let (_, r2) = find(files, Options::default(), &mut fs);
        assert_eq!(r2.groups, r.groups);
    }

    #[test]
    fn same_name_different_content() {
        let mut fs = MemFs::default();
        let a = data(1, 2000);
        let files = setup(&mut fs, &[
            ("one/readme.txt", a.clone()), ("two/readme.txt", a.clone()), ("three/readme.txt", data(2, 2000)),
            ("one/notes", a.clone()), ("two/notes", a.clone()),
            ("x/config", data(3, 10)), ("y/config", data(4, 11)),
            ("x/.keep", Vec::new()), ("y/.keep", Vec::new()),
            ("solo", data(5, 5)),
        ]);
        let (files, r) = find(files, Options::default(), &mut fs);
        let clashes = same_name(&files, &r);
        let names: Vec<&[u8]> = clashes.iter().map(|c| &c.name[..]).collect();
        assert_eq!(names, [&b"config"[..], b"readme.txt"]);
        assert_eq!(clashes[1].files.len(), 3);
        assert_eq!(clashes[1].versions, 2);
    }
}
