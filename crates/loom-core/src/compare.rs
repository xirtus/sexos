// SPDX-License-Identifier: GPL-3.0-only
//! Two folder trees side by side, a plan to bring them in step, and `.sha256` sum files.
//!
//! [`compare`] lines up both listings by relative path and gives each a [`Status`], by size
//! and modification time or by contents ([`Contents`]). Folders take the worst status of
//! what is in them. [`plan`] turns the rows into copies, deletions and new folders for one
//! of three modes; in two-way mode with a [`Baseline`] (the state after the last sync) a
//! file changed on both sides since is a conflict, listed and left alone.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::hash::{from_hex, hash_file, to_hex, Digest};
use crate::text::{escape, lines, parse_u64, push_u64, split_unescaped, unescape};
use crate::{is_within, ReadAt};

/// One file or folder of a tree, by its path relative to the tree's top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub path: Vec<u8>, pub dir: bool, pub size: u64, pub mtime: u64 }

impl Entry {
    pub fn file(path: &[u8], size: u64, mtime: u64) -> Entry { Entry { path: path.to_vec(), dir: false, size, mtime } }
    pub fn dir(path: &[u8]) -> Entry { Entry { path: path.to_vec(), dir: true, size: 0, mtime: 0 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side { Left, Right }

/// Content hashes for the content comparison; [`Readers`] hashes through two [`ReadAt`]s.
pub trait Contents {
    fn hash(&mut self, side: Side, path: &[u8]) -> Option<Digest>;
}

/// Hashes files read through one reader per side.
pub struct Readers<'a> { pub left: &'a mut dyn ReadAt, pub right: &'a mut dyn ReadAt }
impl Contents for Readers<'_> {
    fn hash(&mut self, side: Side, path: &[u8]) -> Option<Digest> {
        match side { Side::Left => hash_file(self.left, path), Side::Right => hash_file(self.right, path) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Same size and times within `tolerance` seconds (2 suits FAT) is the same.
    SizeTime { tolerance: u64 },
    /// Same size and same SHA-256; times only say which side is newer.
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Same,
    NewerLeft,
    NewerRight,
    /// Different, and the times don't say which is newer.
    Differs,
    LeftOnly,
    RightOnly,
    /// A file on one side, a folder on the other.
    Kinds,
    /// A file that couldn't be read for the content comparison.
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row { pub path: Vec<u8>, pub status: Status, pub left: Option<Entry>, pub right: Option<Entry> }

/// Compares two listings (paths relative to their tops, in any order). Rows come sorted by path.
pub fn compare(left: &[Entry], right: &[Entry], method: Method, contents: Option<&mut dyn Contents>) -> Vec<Row> {
    let mut all: BTreeMap<Vec<u8>, (Option<Entry>, Option<Entry>)> = BTreeMap::new();
    for e in left { all.entry(e.path.clone()).or_default().0 = Some(e.clone()); }
    for e in right { all.entry(e.path.clone()).or_default().1 = Some(e.clone()); }
    let mut contents = contents;
    let tol = match method { Method::SizeTime { tolerance } => tolerance, Method::Content => 0 };
    let newer = |l: &Entry, r: &Entry, otherwise: Status| {
        if l.mtime > r.mtime.saturating_add(tol) { Status::NewerLeft } else if r.mtime > l.mtime.saturating_add(tol) { Status::NewerRight } else { otherwise }
    };
    let mut rows: Vec<Row> = all.into_iter().map(|(path, (l, r))| {
        let status = match (&l, &r) {
            (Some(_), None) => Status::LeftOnly,
            (None, Some(_)) => Status::RightOnly,
            (Some(a), Some(b)) if a.dir != b.dir => Status::Kinds,
            (Some(a), Some(_)) if a.dir => Status::Same, // settled from its contents below
            (Some(a), Some(b)) => match method {
                Method::SizeTime { .. } => if a.size == b.size { newer(a, b, Status::Same) } else { newer(a, b, Status::Differs) },
                Method::Content => {
                    if a.size != b.size { newer(a, b, Status::Differs) }
                    else if let Some(c) = contents.as_deref_mut() {
                        match (c.hash(Side::Left, &path), c.hash(Side::Right, &path)) {
                            (Some(x), Some(y)) if x == y => Status::Same,
                            (Some(_), Some(_)) => newer(a, b, Status::Differs),
                            _ => Status::Unreadable,
                        }
                    } else { Status::Unreadable }
                }
            },
            (None, None) => Status::Same,
        };
        Row { path, status, left: l, right: r }
    }).collect();
    // A folder on both sides is the same only if everything in it is.
    for i in 0..rows.len() {
        let is_dir_both = rows[i].left.as_ref().is_some_and(|e| e.dir) && rows[i].right.as_ref().is_some_and(|e| e.dir);
        if !is_dir_both { continue; }
        let p = rows[i].path.clone();
        let differs = rows[i + 1..].iter().take_while(|r| is_within(&r.path, &p)).any(|r| r.status != Status::Same);
        if differs { rows[i].status = Status::Differs; }
    }
    rows
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { MirrorLeftToRight, MirrorRightToLeft, TwoWay }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Make a folder on that side.
    MakeDir(Side, Vec<u8>),
    /// Copy a file from the other side onto this side.
    CopyTo(Side, Vec<u8>),
    /// Delete a file or a whole folder on that side.
    Delete(Side, Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict { pub path: Vec<u8>, pub status: Status, /// The newer side, when the times tell.
    pub newer: Option<Side> }

/// What to do: deletions first (deepest first), then folders and copies top down.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Plan { pub actions: Vec<Action>, pub conflicts: Vec<Conflict>, /// Bytes to copy.
    pub bytes: u64 }

/// The state of both trees after a sync: what each path was, so the next two-way sync can
/// tell which side changed it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Baseline { map: BTreeMap<Vec<u8>, (bool, u64, u64)> }

const HEADER: &[u8] = b"loom-sync 1";

impl Baseline {
    /// Records the rows that are the same on both sides (after the sync ran, compare again).
    pub fn from_rows(rows: &[Row]) -> Baseline {
        let map = rows.iter().filter(|r| r.status == Status::Same).filter_map(|r| r.left.as_ref().map(|e| (r.path.clone(), (e.dir, e.size, e.mtime)))).collect();
        Baseline { map }
    }
    fn changed(&self, path: &[u8], now: Option<&Entry>, tol: u64) -> bool {
        match (self.map.get(path), now) {
            (None, None) => false,
            (Some(&(dir, size, mtime)), Some(e)) => dir != e.dir || (!dir && (size != e.size || mtime.abs_diff(e.mtime) > tol)),
            _ => true,
        }
    }
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = HEADER.to_vec();
        out.push(b'\n');
        for (p, &(dir, size, mtime)) in &self.map {
            escape(p, &mut out);
            out.extend_from_slice(if dir { b"\td\t" } else { b"\tf\t" });
            push_u64(&mut out, size);
            out.push(b'\t');
            push_u64(&mut out, mtime);
            out.push(b'\n');
        }
        out
    }
    pub fn from_bytes(raw: &[u8]) -> Option<Baseline> {
        let mut it = lines(raw);
        if it.next()? != HEADER { return None; }
        let mut map = BTreeMap::new();
        for l in it {
            let f = split_unescaped(l, b'\t');
            if f.len() != 4 { continue; }
            let (Some(size), Some(mtime)) = (parse_u64(f[2]), parse_u64(f[3])) else { continue };
            map.insert(unescape(f[0]), (f[1] == b"d", size, mtime));
        }
        Some(Baseline { map })
    }
}

fn other(s: Side) -> Side { match s { Side::Left => Side::Right, Side::Right => Side::Left } }

/// Plans a sync. `tolerance` is the same seconds of slack as the comparison used.
pub fn plan(rows: &[Row], mode: Mode, baseline: Option<&Baseline>, tolerance: u64) -> Plan {
    let mut deletes: Vec<(Side, Vec<u8>)> = Vec::new();
    let mut creates: Vec<Action> = Vec::new();
    let mut conflicts = Vec::new();
    let mut bytes = 0u64;
    // Paths whose whole folder is already being deleted or copied over need nothing more.
    let mut covered: BTreeSet<(Side, Vec<u8>)> = BTreeSet::new();
    let under_covered = |covered: &BTreeSet<(Side, Vec<u8>)>, side: Side, p: &[u8]| covered.iter().any(|(s, c)| *s == side && is_within(p, c) && c.len() < p.len());

    // Make `from`'s entry exist on `to`: a folder (with everything in it, row by row) or a file.
    let push = |to: Side, row: &Row, creates: &mut Vec<Action>, bytes: &mut u64| {
        let src = if to == Side::Right { row.left.as_ref() } else { row.right.as_ref() };
        if let Some(e) = src {
            if e.dir { creates.push(Action::MakeDir(to, row.path.clone())); } else { creates.push(Action::CopyTo(to, row.path.clone())); *bytes += e.size; }
        }
    };

    for row in rows {
        let (l, r) = (row.left.as_ref(), row.right.as_ref());
        // Which side wins this row, and whether to delete the loser's copy first.
        let decide = |winner: Side| -> (Side, bool) {
            let loser_has = if winner == Side::Left { r.is_some() } else { l.is_some() };
            let winner_has = if winner == Side::Left { l.is_some() } else { r.is_some() };
            (winner, loser_has && (!winner_has || row.status == Status::Kinds))
        };
        let choice: Option<(Side, bool)> = match mode {
            Mode::MirrorLeftToRight | Mode::MirrorRightToLeft => {
                let from = if mode == Mode::MirrorLeftToRight { Side::Left } else { Side::Right };
                if row.status == Status::Same || row.status == Status::Unreadable { None } else { Some(decide(from)) }
            }
            Mode::TwoWay => {
                if row.status == Status::Same || row.status == Status::Unreadable { None }
                else if let Some(b) = baseline {
                    let lc = b.changed(&row.path, l, tolerance);
                    let rc = b.changed(&row.path, r, tolerance);
                    match (lc, rc) {
                        (true, false) => Some(decide(Side::Left)),
                        (false, true) => Some(decide(Side::Right)),
                        _ => {
                            // Folders that both hold changes are settled by their contents.
                            if l.is_some_and(|e| e.dir) && r.is_some_and(|e| e.dir) { None }
                            else {
                                let newer = match row.status { Status::NewerLeft => Some(Side::Left), Status::NewerRight => Some(Side::Right), _ => None };
                                conflicts.push(Conflict { path: row.path.clone(), status: row.status, newer });
                                None
                            }
                        }
                    }
                } else {
                    // No baseline: nothing is deleted; the newer side wins.
                    match row.status {
                        Status::LeftOnly | Status::NewerLeft => Some((Side::Left, false)),
                        Status::RightOnly | Status::NewerRight => Some((Side::Right, false)),
                        Status::Differs if l.is_some_and(|e| e.dir) => None,
                        _ => {
                            conflicts.push(Conflict { path: row.path.clone(), status: row.status, newer: None });
                            None
                        }
                    }
                }
            }
        };
        let Some((winner, delete_first)) = choice else { continue };
        let to = other(winner);
        if under_covered(&covered, to, &row.path) { continue; }
        let winner_entry = if winner == Side::Left { l } else { r };
        if delete_first {
            deletes.push((to, row.path.clone()));
            // What was under it is gone, unless a folder takes its place and fills again.
            if !winner_entry.is_some_and(|e| e.dir) { covered.insert((to, row.path.clone())); }
        }
        // Folders on both sides only need their contents synced.
        let both_dirs = l.is_some_and(|e| e.dir) && r.is_some_and(|e| e.dir);
        if winner_entry.is_some() && !both_dirs { push(to, row, &mut creates, &mut bytes); }
    }
    // Deepest first, so a folder's contents go before it; but a deleted folder covers its contents.
    deletes.sort_by(|a, b| b.1.cmp(&a.1));
    let mut actions: Vec<Action> = Vec::new();
    for (i, (s, p)) in deletes.iter().enumerate() {
        if deletes.iter().enumerate().any(|(j, (s2, q))| j != i && s2 == s && q.len() < p.len() && is_within(p, q)) { continue; }
        actions.push(Action::Delete(*s, p.clone()));
    }
    actions.extend(creates);
    Plan { actions, conflicts, bytes }
}

/// One line of a sum file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sum { pub digest: Digest, pub path: Vec<u8> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SumError { pub line: usize }

/// Reads a `sha256sum`-style file: `<64 hex>  <path>` (or ` *<path>` for binary mode), or the
/// BSD form `SHA256 (<path>) = <64 hex>`. Blank lines and `#` comments are skipped.
pub fn parse_sums(raw: &[u8]) -> Result<Vec<Sum>, SumError> {
    let mut out = Vec::new();
    for (n, l) in raw.split(|b| *b == b'\n').enumerate() {
        let l = l.strip_suffix(b"\r").unwrap_or(l);
        if l.trim_ascii().is_empty() || l.starts_with(b"#") { continue; }
        let err = SumError { line: n + 1 };
        if let Some(rest) = l.strip_prefix(b"SHA256 (") {
            let close = rest.windows(4).rposition(|w| w == b") = ").ok_or(err)?;
            out.push(Sum { digest: from_hex(&rest[close + 4..]).ok_or(err)?, path: rest[..close].to_vec() });
            continue;
        }
        if l.len() < 67 || l[64] != b' ' || (l[65] != b' ' && l[65] != b'*') { return Err(err); }
        out.push(Sum { digest: from_hex(&l[..64]).ok_or(err)?, path: l[66..].to_vec() });
    }
    Ok(out)
}

/// Writes a `sha256sum`-compatible file.
pub fn write_sums(sums: &[Sum]) -> Vec<u8> {
    let mut out = Vec::new();
    for s in sums {
        out.extend_from_slice(&to_hex(&s.digest));
        out.extend_from_slice(b"  ");
        out.extend_from_slice(&s.path);
        out.push(b'\n');
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check { Ok, Mismatch, Missing }

/// Checks each listed file; paths are relative to the sum file's folder, `read` resolves them.
pub fn verify(sums: &[Sum], read: &mut dyn ReadAt) -> Vec<Check> {
    sums.iter().map(|s| match hash_file(read, &s.path) {
        Some(d) if d == s.digest => Check::Ok,
        Some(_) => Check::Mismatch,
        None => Check::Missing,
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::mem::MemFs;
    use crate::hash::sha256;
    use alloc::vec;

    fn tree(n: usize) -> Vec<Entry> {
        let mut v = vec![Entry::dir(b"docs"), Entry::dir(b"docs/old"), Entry::dir(b"src")];
        for i in 0..n {
            let dir = ["docs", "docs/old", "src"][i % 3];
            v.push(Entry::file(alloc::format!("{dir}/f{i:03}.txt").as_bytes(), 100 + i as u64, 1_000_000 + i as u64));
        }
        v
    }

    fn statuses(rows: &[Row]) -> BTreeMap<Status, usize> {
        let mut m = BTreeMap::new();
        for r in rows { *m.entry(r.status).or_default() += 1; }
        m
    }

    #[test]
    fn one_changed_among_many() {
        let left = tree(300);
        let mut right = left.clone();
        let i = right.iter().position(|e| e.path == b"docs/old/f151.txt").unwrap();
        right[i].mtime += 3600;
        right[i].size += 1;
        let rows = compare(&left, &right, Method::SizeTime { tolerance: 2 }, None);
        assert_eq!(rows.len(), 303);
        let changed: Vec<&Row> = rows.iter().filter(|r| r.status != Status::Same).collect();
        let paths: Vec<&[u8]> = changed.iter().map(|r| &r.path[..]).collect();
        assert_eq!(paths, [&b"docs"[..], b"docs/old", b"docs/old/f151.txt"]);
        assert_eq!(changed[2].status, Status::NewerRight);
        assert_eq!(changed[0].status, Status::Differs, "folders show what's in them");
        // Mirroring copies exactly that file.
        let p = plan(&rows, Mode::MirrorRightToLeft, None, 2);
        assert_eq!(p.actions, [Action::CopyTo(Side::Left, b"docs/old/f151.txt".to_vec())]);
        assert_eq!(p.bytes, 252);
        let p = plan(&rows, Mode::MirrorLeftToRight, None, 2);
        assert_eq!(p.actions, [Action::CopyTo(Side::Right, b"docs/old/f151.txt".to_vec())]);
        let p = plan(&rows, Mode::TwoWay, None, 2);
        assert_eq!(p.actions, [Action::CopyTo(Side::Left, b"docs/old/f151.txt".to_vec())]);
        // Within the tolerance a time difference alone is the same (FAT keeps 2 s).
        let mut r2 = left.clone();
        r2[5].mtime += 2;
        assert!(compare(&left, &r2, Method::SizeTime { tolerance: 2 }, None).iter().all(|r| r.status == Status::Same));
    }

    #[test]
    fn content_comparison() {
        let mut lfs = MemFs::default();
        let mut rfs = MemFs::default();
        let mut left = Vec::new();
        let mut right = Vec::new();
        for i in 0..50u32 {
            let p = alloc::format!("f{i}");
            let d: Vec<u8> = (0..1000u32).map(|k| (k * i) as u8).collect();
            lfs.add(p.as_bytes(), &d);
            let mut rd = d.clone();
            if i == 17 { rd[500] ^= 0xff; } // same size and time, different bytes
            rfs.add(p.as_bytes(), &rd);
            // Times differ for i == 3 but the contents don't.
            left.push(Entry::file(p.as_bytes(), 1000, 5000));
            right.push(Entry::file(p.as_bytes(), 1000, if i == 3 { 9000 } else { 5000 }));
        }
        left.push(Entry::file(b"gone", 1, 1));
        right.push(Entry::file(b"gone", 1, 1));
        let mut rd = Readers { left: &mut lfs, right: &mut rfs };
        let rows = compare(&left, &right, Method::Content, Some(&mut rd));
        let s = statuses(&rows);
        assert_eq!(s[&Status::Same], 49);
        assert_eq!(s[&Status::Differs], 1);
        assert_eq!(s[&Status::Unreadable], 1);
        assert_eq!(rows.iter().find(|r| r.status == Status::Differs).unwrap().path, b"f17");
        // By size and time the opposite file looks changed.
        let rows2 = compare(&left, &right, Method::SizeTime { tolerance: 0 }, None);
        assert_eq!(rows2.iter().filter(|r| r.status != Status::Same).map(|r| &r.path[..]).collect::<Vec<_>>(), [&b"f3"[..]]);
    }

    #[test]
    fn mirror_plans() {
        let left = vec![
            Entry::dir(b"a"), Entry::file(b"a/1", 10, 5), Entry::dir(b"a/new"), Entry::file(b"a/new/x", 3, 5),
            Entry::file(b"b", 7, 9), Entry::file(b"kind", 1, 1),
        ];
        let right = vec![
            Entry::dir(b"a"), Entry::file(b"a/1", 10, 5), Entry::file(b"b", 7, 2),
            Entry::dir(b"junk"), Entry::file(b"junk/y", 1, 1), Entry::dir(b"junk/z"), Entry::file(b"junk/z/w", 1, 1),
            Entry::dir(b"kind"), Entry::file(b"kind/k", 1, 1),
        ];
        let rows = compare(&left, &right, Method::SizeTime { tolerance: 0 }, None);
        let p = plan(&rows, Mode::MirrorLeftToRight, None, 0);
        assert_eq!(p.actions, vec![
            Action::Delete(Side::Right, b"kind".to_vec()),
            Action::Delete(Side::Right, b"junk".to_vec()),
            Action::MakeDir(Side::Right, b"a/new".to_vec()),
            Action::CopyTo(Side::Right, b"a/new/x".to_vec()),
            Action::CopyTo(Side::Right, b"b".to_vec()),
            Action::CopyTo(Side::Right, b"kind".to_vec()),
        ]);
        assert_eq!(p.bytes, 3 + 7 + 1);
        let p = plan(&rows, Mode::MirrorRightToLeft, None, 0);
        assert_eq!(p.actions, vec![
            Action::Delete(Side::Left, b"kind".to_vec()),
            Action::Delete(Side::Left, b"a/new".to_vec()),
            Action::CopyTo(Side::Left, b"b".to_vec()),
            Action::MakeDir(Side::Left, b"junk".to_vec()),
            Action::CopyTo(Side::Left, b"junk/y".to_vec()),
            Action::MakeDir(Side::Left, b"junk/z".to_vec()),
            Action::CopyTo(Side::Left, b"junk/z/w".to_vec()),
            Action::MakeDir(Side::Left, b"kind".to_vec()),
            Action::CopyTo(Side::Left, b"kind/k".to_vec()),
        ]);
        // Two-way without a baseline never deletes.
        let p = plan(&rows, Mode::TwoWay, None, 0);
        assert!(p.actions.iter().all(|a| !matches!(a, Action::Delete(..))));
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(p.conflicts[0].path, b"kind");
    }

    #[test]
    fn two_way_with_baseline() {
        let base_tree = vec![
            Entry::dir(b"d"), Entry::file(b"d/a", 1, 100), Entry::file(b"d/b", 2, 100), Entry::file(b"c", 3, 100),
            Entry::file(b"del-left", 4, 100), Entry::file(b"del-right", 5, 100), Entry::file(b"both", 6, 100),
        ];
        let rows0 = compare(&base_tree, &base_tree, Method::SizeTime { tolerance: 0 }, None);
        let base = Baseline::from_rows(&rows0);
        assert_eq!(base.len(), 7);
        let base = Baseline::from_bytes(&base.to_bytes()).unwrap();
        // Since then: left edited d/a and deleted del-left, right edited c and deleted del-right,
        // both edited "both", and right added "new".
        let mut left = base_tree.clone();
        left[1].mtime = 200;
        left.retain(|e| e.path != b"del-left");
        let i = left.iter().position(|e| e.path == b"both").unwrap();
        left[i].mtime = 300;
        let mut right = base_tree.clone();
        right[3].mtime = 250;
        right[3].size = 30;
        right.retain(|e| e.path != b"del-right");
        let i = right.iter().position(|e| e.path == b"both").unwrap();
        right[i].mtime = 400;
        right.push(Entry::file(b"new", 9, 500));
        let rows = compare(&left, &right, Method::SizeTime { tolerance: 0 }, None);
        let p = plan(&rows, Mode::TwoWay, Some(&base), 0);
        assert_eq!(p.actions, vec![
            Action::Delete(Side::Left, b"del-right".to_vec()),
            Action::Delete(Side::Right, b"del-left".to_vec()),
            Action::CopyTo(Side::Left, b"c".to_vec()),
            Action::CopyTo(Side::Right, b"d/a".to_vec()),
            Action::CopyTo(Side::Left, b"new".to_vec()),
        ]);
        assert_eq!(p.conflicts, [Conflict { path: b"both".to_vec(), status: Status::NewerRight, newer: Some(Side::Right) }]);
        // Without the baseline the deletions would come back as copies instead.
        let p = plan(&rows, Mode::TwoWay, None, 0);
        assert!(p.actions.contains(&Action::CopyTo(Side::Left, b"del-left".to_vec())));
        assert!(p.conflicts.is_empty());
    }

    #[test]
    fn sum_files() {
        let mut fs = MemFs::default();
        fs.add(b"a.txt", b"hello\n");
        fs.add(b"dir/b b.bin", b"\x00\x01");
        let sums = vec![
            Sum { digest: sha256(b"hello\n"), path: b"a.txt".to_vec() },
            Sum { digest: sha256(b"\x00\x01"), path: b"dir/b b.bin".to_vec() },
            Sum { digest: sha256(b"x"), path: b"missing".to_vec() },
        ];
        let text = write_sums(&sums);
        assert!(text.starts_with(b"5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03  a.txt\n"));
        assert_eq!(parse_sums(&text).unwrap(), sums);
        assert_eq!(verify(&sums, &mut fs), [Check::Ok, Check::Ok, Check::Missing]);
        fs.add(b"a.txt", b"hellO\n");
        assert_eq!(verify(&sums, &mut fs)[0], Check::Mismatch);
        let bsd = b"# made elsewhere\r\nSHA256 (we(ird) = name) = 5891B5B522D5DF086D0FF0B110FBD9D21BB4FC7163AF34D08286A2E846F6BE03\r\n\n5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03 *bin\n";
        let p = parse_sums(bsd).unwrap();
        assert_eq!(p[0].path, b"we(ird) = name");
        assert_eq!(p[1].path, b"bin");
        assert_eq!(parse_sums(b"abc  file\n"), Err(SumError { line: 1 }));
        assert_eq!(parse_sums(b"\nzz5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be  x\n"), Err(SumError { line: 2 }));
    }
}
