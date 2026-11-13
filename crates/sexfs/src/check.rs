// SPDX-License-Identifier: GPL-3.0-only
//! Check and repair (feature `repair`).
//!
//! [`check`] reads a device without changing it and reports what is wrong. [`repair`]
//! salvages every record it can read, settles each one against the others, and writes a
//! new tree beside the old one, then a new superblock: like any commit, a power cut
//! during repair leaves the last commit as it was.
//!
//! What it does with damage:
//! - a node that fails its checksum (or is not where it says it is) loses its records;
//!   if the other superblock's commit reads whole, files missing from the newest one are
//!   taken back from it (their data blocks re-verified);
//! - entries naming a missing inode are dropped; two entries of one name get `~N`;
//! - inodes no folder names go to `/lost+found` as `#ID`;
//! - block records past a file's end, for a missing inode, out of the device, or sharing
//!   a block another file uses, are dropped (the file reads zeros there);
//! - data blocks whose checksum fails are reported (and dropped from files taken back
//!   from the older commit).
use super::*;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

/// What was found, and for [`repair`] what was done about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub generation: u64,
    /// Tree nodes read, and how many of them were unreadable or misplaced.
    pub nodes: u64,
    pub bad_nodes: u64,
    /// The newest commit was damaged and files were taken from the one before.
    pub from_older: u64,
    pub bad_superblocks: u64,
    pub bad_inodes: u64,
    pub dangling_entries: u64,
    pub duplicate_names: u64,
    pub wrong_kinds: u64,
    pub orphans: u64,
    pub stray_blocks: u64,
    pub shared_blocks: u64,
    pub bad_data: u64,
    pub files: u64,
    pub dirs: u64,
}
impl Report {
    /// Nothing to repair.
    pub fn clean(&self) -> bool {
        self.bad_nodes + self.bad_superblocks + self.bad_inodes + self.dangling_entries + self.duplicate_names + self.wrong_kinds
            + self.orphans + self.stray_blocks + self.shared_blocks + self.bad_data == 0
    }
}

struct Super { generation: u64, root: u64, next: u64 }

fn superblocks<D: Device>(dev: &mut D, report: &mut Report) -> Result<Vec<Super>> {
    let mut b = [0u8; BLOCK];
    let mut out = Vec::new();
    let mut magic = false;
    for s in 0..SB_SLOTS {
        dev.read(s, &mut b).map_err(|_| Error::Io)?;
        if get_u64(&b, 0) != MAGIC_SB { continue; }
        magic = true;
        if checksum(&b[..56]) != get_u64(&b, 56) || get_u64(&b, 8) != VERSION as u64 { report.bad_superblocks += 1; continue; }
        out.push(Super { generation: get_u64(&b, 16), root: get_u64(&b, 24), next: get_u64(&b, 32) });
    }
    if out.is_empty() { return Err(if magic { Error::Corrupt } else { Error::Unformatted }); }
    out.sort_by(|a, b| b.generation.cmp(&a.generation));
    Ok(out)
}

/// Every leaf record readable under `root`. Unreadable nodes are counted, not followed.
struct Salvage { records: BTreeMap<Key, Vec<u8>>, nodes: BTreeSet<u64>, bad: u64 }
fn salvage<D: Device>(dev: &mut D, total: u64, root: u64) -> Salvage {
    let mut s = Salvage { records: BTreeMap::new(), nodes: BTreeSet::new(), bad: 0 };
    let mut n = [0u8; BLOCK];
    let mut stack = vec![(root, 0usize, None::<u8>)];
    while let Some((block, depth, expect)) = stack.pop() {
        if depth > MAX_DEPTH || block < SB_SLOTS || block >= total || !s.nodes.insert(block) { s.bad += 1; continue; }
        if dev.read(block, &mut n).is_err() || u32::from_le_bytes(n[..4].try_into().unwrap()) != MAGIC_NODE || get_u64(&n, OFF_OWNER) != block
            || get_u64(&n, OFF_SUM) != node_sum(&n) || expect.is_some_and(|e| e != level(&n)) || OFF_SLOTS + 2 * count(&n) > BLOCK {
            s.bad += 1;
            continue;
        }
        let lvl = level(&n);
        for i in 0..count(&n) {
            let at = rec_at(&n, i);
            if at + REC_HEAD > BLOCK || at + REC_HEAD + get_u16(&n, at + KEY_BYTES) > BLOCK { s.bad += 1; break; }
            if lvl > 0 {
                if val_at(&n, i).len() < 8 { s.bad += 1; break; }
                stack.push((child_at(&n, i), depth + 1, Some(lvl - 1)));
            } else {
                s.records.entry(key_at(&n, i)).or_insert_with(|| val_at(&n, i).to_vec());
            }
        }
    }
    s
}

/// The settled contents: inodes, entries per folder, block maps.
struct Plan {
    inodes: BTreeMap<u64, (Kind, u64, u64)>,
    /// (folder, name) -> (child, kind), in name order per folder.
    entries: Vec<(u64, Vec<u8>, u64, Kind)>,
    blocks: Vec<(u64, u64, u64, u64)>,
    next_id: u64,
    /// Every block the old trees used, kept clear of the new tree's nodes.
    old_nodes: BTreeSet<u64>,
    generation: u64,
    total: u64,
}

fn plan<D: Device>(dev: &mut D, deep: bool, report: &mut Report) -> Result<Plan> {
    let supers = superblocks(dev, report)?;
    let mut total = dev.blocks();
    {
        let mut b = [0u8; BLOCK];
        dev.read(supers[0].generation % SB_SLOTS, &mut b).map_err(|_| Error::Io)?;
        let recorded = get_u64(&b, 40);
        if recorded != 0 { total = total.min(recorded); }
    }
    let newest = &supers[0];
    report.generation = newest.generation;
    let main = salvage(dev, total, newest.root);
    report.nodes = main.nodes.len() as u64;
    report.bad_nodes = main.bad;
    let mut old_nodes = main.nodes.clone();
    let mut records = main.records;
    let mut next_id = newest.next;
    // Files the damaged newest commit lost, from the one before it.
    let mut from_older: BTreeSet<u64> = BTreeSet::new();
    // Records the newest commit lost, taken from the one before.
    let mut older_keys: BTreeSet<Key> = BTreeSet::new();
    if main.bad > 0 {
        if let Some(older) = supers.get(1) {
            let prev = salvage(dev, total, older.root);
            old_nodes.extend(prev.nodes.iter().copied());
            let have: BTreeSet<u64> = records.keys().filter(|k| k.kind == K_INODE).map(|k| k.id).collect();
            for (k, v) in prev.records {
                // A file the newest commit still has keeps its own inode; its block
                // records and entries that went with the damaged node come back.
                if have.contains(&k.id) && k.kind == K_INODE { continue; }
                if records.contains_key(&k) { continue; }
                if k.kind == K_INODE { from_older.insert(k.id); }
                older_keys.insert(k);
                records.insert(k, v);
            }
            next_id = next_id.max(older.next);
            report.from_older = from_older.len() as u64;
        }
    }
    // Inodes.
    let mut inodes = BTreeMap::new();
    for (k, v) in records.iter().filter(|(k, _)| k.kind == K_INODE) {
        match decode_inode(v) { Ok(i) if k.off == 0 => { inodes.insert(k.id, i); } _ => report.bad_inodes += 1 }
    }
    if !matches!(inodes.get(&ROOT), Some((Kind::Dir, _, _))) { report.bad_inodes += 1; inodes.insert(ROOT, (Kind::Dir, 0, 0)); }
    // Block maps: inside the file, inside the device, used once. The newest commit's
    // files claim their blocks first; files taken from the older commit get what is left,
    // and only blocks that still hold what they held then.
    let mut used: BTreeSet<u64> = BTreeSet::new();
    let mut blocks = Vec::new();
    let mut data = [0u8; BLOCK];
    for older in [false, true] {
        for (k, v) in records.iter().filter(|(k, _)| k.kind == K_BLOCK && older_keys.contains(k) == older) {
            let Some(&(kind, size, _)) = inodes.get(&k.id) else { report.stray_blocks += 1; continue };
            if v.len() != 16 || kind != Kind::File || k.off.saturating_mul(BLOCK as u64) >= size { report.stray_blocks += 1; continue; }
            let (block, sum) = (get_u64(v, 0), get_u64(v, 8));
            if block < SB_SLOTS || block >= total { report.stray_blocks += 1; continue; }
            // Kept clear of the new tree's nodes whatever happens to the record.
            old_nodes.insert(block);
            if used.contains(&block) || main.nodes.contains(&block) { report.shared_blocks += 1; continue; }
            if (deep || older) && (dev.read_data(block, &mut data).is_err() || checksum(&data) != sum) {
                report.bad_data += 1;
                if older { continue; }
            }
            used.insert(block);
            blocks.push((k.id, k.off, block, sum));
        }
    }
    // Entries: to an inode that exists, of its kind, one per name.
    let mut by_dir: BTreeMap<u64, BTreeMap<Vec<u8>, (u64, Kind)>> = BTreeMap::new();
    for (k, v) in records.iter().filter(|(k, _)| k.kind == K_DIRENT) {
        if v.len() < 10 || v.len() > 9 + NAME_MAX { report.dangling_entries += 1; continue; }
        let (child, name) = (get_u64(v, 0), &v[9..]);
        if !matches!(inodes.get(&k.id), Some((Kind::Dir, _, _))) || name.iter().any(|b| *b == b'/' || *b == 0) || name == b"." || name == b".." { report.dangling_entries += 1; continue; }
        let Some(&(kind, _, _)) = inodes.get(&child) else { report.dangling_entries += 1; continue };
        if child == ROOT { report.dangling_entries += 1; continue; }
        if (v[8] == 1) != (kind == Kind::Dir) { report.wrong_kinds += 1; }
        let names = by_dir.entry(k.id).or_default();
        let mut name = name.to_vec();
        if names.contains_key(&name) {
            report.duplicate_names += 1;
            let mut n = 2;
            loop {
                let mut alt = name.clone();
                alt.extend_from_slice(alloc::format!("~{}", n).as_bytes());
                if alt.len() <= NAME_MAX && !names.contains_key(&alt) { name = alt; break; }
                n += 1;
            }
        }
        names.insert(name, (child, kind));
    }
    // Reachable from the root, each folder once; the rest go to lost+found.
    let mut seen: BTreeSet<u64> = BTreeSet::from([ROOT]);
    let mut entries = Vec::new();
    let mut queue = alloc::collections::VecDeque::from([ROOT]);
    while let Some(dir) = queue.pop_front() {
        let Some(names) = by_dir.get(&dir) else { continue };
        for (name, &(child, kind)) in names {
            if !seen.insert(child) {
                // A second name for a folder would make a loop; a file may not have two either.
                report.dangling_entries += 1;
                continue;
            }
            entries.push((dir, name.clone(), child, kind));
            if kind == Kind::Dir { queue.push_back(child); }
        }
    }
    // Inodes no reachable folder names. Those another unreachable folder names come
    // along with it; the rest (and any loop of folders) go to /lost+found.
    let unseen: BTreeSet<u64> = inodes.keys().copied().filter(|id| !seen.contains(id)).collect();
    if !unseen.is_empty() {
        let named: BTreeSet<u64> = by_dir.iter().filter(|(d, _)| unseen.contains(d)).flat_map(|(_, n)| n.values().map(|(c, _)| *c)).collect();
        let mut tops: Vec<u64> = unseen.iter().copied().filter(|id| !named.contains(id)).collect();
        tops.extend(unseen.iter().copied().filter(|id| named.contains(id)));
        let lost = match by_dir.get(&ROOT).and_then(|n| n.get(&b"lost+found"[..])) {
            Some(&(id, Kind::Dir)) if seen.contains(&id) => id,
            _ => {
                let id = next_id.max(inodes.keys().max().copied().unwrap_or(ROOT) + 1);
                next_id = id + 1;
                inodes.insert(id, (Kind::Dir, 0, 0));
                seen.insert(id);
                let mut name = b"lost+found".to_vec();
                while by_dir.get(&ROOT).is_some_and(|n| n.contains_key(&name)) { name.push(b'~'); }
                entries.push((ROOT, name, id, Kind::Dir));
                id
            }
        };
        for id in tops {
            if !seen.insert(id) { continue; }
            report.orphans += 1;
            let kind = inodes[&id].0;
            entries.push((lost, alloc::format!("#{}", id).into_bytes(), id, kind));
            let mut queue = alloc::collections::VecDeque::from([id]);
            while let Some(dir) = queue.pop_front() {
                let Some(names) = by_dir.get(&dir) else { continue };
                for (name, &(child, kind)) in names {
                    if !seen.insert(child) { continue; }
                    entries.push((dir, name.clone(), child, kind));
                    if kind == Kind::Dir { queue.push_back(child); }
                }
            }
        }
    }
    report.files = inodes.values().filter(|i| i.0 == Kind::File).count() as u64;
    report.dirs = inodes.values().filter(|i| i.0 == Kind::Dir).count() as u64;
    next_id = next_id.max(inodes.keys().max().copied().unwrap_or(ROOT) + 1);
    old_nodes.extend(main.nodes.iter().copied());
    Ok(Plan { inodes, entries, blocks, next_id, old_nodes, generation: newest.generation, total })
}

/// Reads the whole filesystem and says what is wrong; changes nothing. `deep` also reads
/// every data block against its checksum.
pub fn check<D: Device>(dev: &mut D, deep: bool) -> Result<Report> {
    let mut report = Report::default();
    plan(dev, deep, &mut report)?;
    Ok(report)
}

/// Repairs what [`check`] finds and mounts the result. When the filesystem is clean
/// nothing is written.
pub fn repair<'a, D: Device>(mut dev: D, mem: Memory<'a>, deep: bool) -> Result<(Fs<'a, D>, Report)> {
    let mut report = Report::default();
    let plan = plan(&mut dev, deep, &mut report)?;
    if report.clean() {
        let fs = Fs::mount(dev, mem)?;
        return Ok((fs, report));
    }
    let total = plan.total.min(dev.blocks()).min(Fs::<D>::capacity(&mem));
    let mut fs = Fs::empty(dev, mem, total);
    fs.reset_maps();
    // Nothing the old trees or the kept files use is written over.
    for &b in plan.old_nodes.iter().chain(plan.blocks.iter().map(|(_, _, b, _)| b)) {
        if b >= SB_SLOTS && b < total && !bit(fs.mem.used, b) { set_bit(fs.mem.used, b, true); fs.free -= 1; }
    }
    for s in fs.mem.slots.iter_mut() { *s = Slot::EMPTY; }
    fs.generation = plan.generation;
    fs.tx = plan.generation + 1;
    let (leaf, _) = fs.new_node(0)?;
    fs.unpin_all();
    fs.root = leaf;
    fs.next_id = plan.next_id;
    for (&id, &(kind, size, mtime)) in &plan.inodes {
        fs.insert(Key { id, kind: K_INODE, off: 0 }, &encode_inode(kind, size, mtime))?;
    }
    for &(id, index, block, sum) in &plan.blocks {
        let mut v = [0u8; 16];
        put_u64(&mut v, 0, block);
        put_u64(&mut v, 8, sum);
        fs.insert(Key { id, kind: K_BLOCK, off: index }, &v)?;
    }
    for (dir, name, child, kind) in &plan.entries { fs.add_entry(*dir, name, *child, *kind)?; }
    fs.changed = true;
    fs.commit()?;
    // Mount again, so free space comes from the new tree alone.
    let (dev, mem) = fs.into_parts();
    let mut fs = Fs::mount(dev, mem)?;
    fs.verify()?;
    Ok((fs, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec as StdVec;

    struct Ram(StdVec<[u8; BLOCK]>);
    impl Device for Ram {
        fn blocks(&self) -> u64 { self.0.len() as u64 }
        fn read(&mut self, b: u64, out: &mut [u8; BLOCK]) -> core::result::Result<(), ()> { *out = self.0[b as usize]; Ok(()) }
        fn write(&mut self, b: u64, d: &[u8; BLOCK]) -> core::result::Result<(), ()> { self.0[b as usize] = *d; Ok(()) }
        fn flush(&mut self) -> core::result::Result<(), ()> { Ok(()) }
    }
    impl Device for &mut Ram {
        fn blocks(&self) -> u64 { self.0.len() as u64 }
        fn read(&mut self, b: u64, out: &mut [u8; BLOCK]) -> core::result::Result<(), ()> { *out = self.0[b as usize]; Ok(()) }
        fn write(&mut self, b: u64, d: &[u8; BLOCK]) -> core::result::Result<(), ()> { self.0[b as usize] = *d; Ok(()) }
        fn flush(&mut self) -> core::result::Result<(), ()> { Ok(()) }
    }
    struct Store { cache: StdVec<[u8; BLOCK]>, slots: StdVec<Slot>, used: StdVec<u64>, pending: StdVec<u64>, scratch: Box<[u8; BLOCK]> }
    impl Store {
        fn new() -> Self { Store { cache: vec![[0; BLOCK]; 64], slots: vec![Slot::EMPTY; 64], used: vec![0; 64], pending: vec![0; 64], scratch: Box::new([0; BLOCK]) } }
        fn mem(&mut self) -> Memory<'_> { Memory { cache: &mut self.cache, slots: &mut self.slots, used: &mut self.used, pending: &mut self.pending, scratch: &mut self.scratch } }
    }
    const BLOCKS: usize = 4096;

    /// A small tree: /docs/a.txt (2 blocks), /docs/b.txt, /notes.txt, /deep/x/y.txt,
    /// then a second commit that changes /notes.txt.
    fn sample(store: &mut Store) -> Ram {
        let mut fs = Fs::format(Ram(vec![[0; BLOCK]; BLOCKS]), store.mem()).unwrap();
        fs.mkdir(b"/docs", 1).unwrap();
        let a = fs.create(b"/docs/a.txt", 1).unwrap();
        fs.write(a, 0, &[b'a'; BLOCK + 100], 1).unwrap();
        let b = fs.create(b"/docs/b.txt", 1).unwrap();
        fs.write(b, 0, b"bee", 1).unwrap();
        let n = fs.create(b"/notes.txt", 1).unwrap();
        fs.write(n, 0, b"first", 1).unwrap();
        fs.mkdir(b"/deep", 1).unwrap();
        fs.mkdir(b"/deep/x", 1).unwrap();
        let y = fs.create(b"/deep/x/y.txt", 1).unwrap();
        fs.write(y, 0, b"why", 1).unwrap();
        fs.commit().unwrap();
        fs.write(n, 0, b"second", 2).unwrap();
        fs.commit().unwrap();
        fs.into_parts().0
    }
    fn read(fs: &mut Fs<'_, impl Device>, path: &[u8]) -> Option<StdVec<u8>> {
        let st = fs.open(path).ok()?;
        let mut out = vec![0u8; st.size as usize];
        let n = fs.read(st.id, 0, &mut out).ok()?;
        out.truncate(n);
        Some(out)
    }
    fn names(fs: &mut Fs<'_, impl Device>, path: &[u8]) -> StdVec<String> {
        let dir = fs.open(path).unwrap().id;
        let mut out = StdVec::new();
        let mut cursor = 0;
        let mut name = [0u8; NAME_MAX];
        while let Some((len, _, _, next)) = fs.read_dir(dir, cursor, &mut name).unwrap() { out.push(String::from_utf8_lossy(&name[..len]).into_owned()); cursor = next; }
        out.sort();
        out
    }
    /// Changes the image under an open filesystem through its own tree code, committing.
    fn tamper(dev: Ram, store: &mut Store, f: impl FnOnce(&mut Fs<'_, Ram>)) -> Ram {
        let mut fs = Fs::mount(dev, store.mem()).unwrap();
        f(&mut fs);
        fs.changed = true;
        fs.commit().unwrap();
        fs.into_parts().0
    }

    #[test]
    fn clean_image_checks_clean_and_repair_writes_nothing() {
        let mut store = Store::new();
        let mut dev = sample(&mut store);
        let report = check(&mut dev, true).unwrap();
        assert!(report.clean(), "{:?}", report);
        assert_eq!((report.files, report.dirs), (4, 4));
        let before = dev.0.clone();
        let (fs, report) = repair(dev, store.mem(), true).unwrap();
        assert!(report.clean());
        assert!(fs.into_parts().0 .0 == before, "a clean filesystem was written to");
    }

    #[test]
    fn entry_to_missing_inode_is_dropped() {
        let mut store = Store::new();
        let dev = sample(&mut store);
        let dev = tamper(dev, &mut store, |fs| { let id = fs.open(b"/docs/b.txt").unwrap().id; fs.remove(Key { id, kind: K_INODE, off: 0 }).unwrap(); });
        let mut dev = dev;
        let r = check(&mut dev, false).unwrap();
        assert_eq!(r.dangling_entries, 1, "{:?}", r);
        let (mut fs, r) = repair(dev, store.mem(), false).unwrap();
        assert!(!r.clean());
        assert_eq!(names(&mut fs, b"/docs"), ["a.txt"]);
        assert_eq!(read(&mut fs, b"/docs/a.txt").unwrap(), vec![b'a'; BLOCK + 100]);
        assert!(check(fs.device(), true).unwrap().clean());
    }

    #[test]
    fn unnamed_inode_goes_to_lost_and_found_with_its_contents() {
        let mut store = Store::new();
        let dev = sample(&mut store);
        let dev = tamper(dev, &mut store, |fs| { let (_, off) = fs.lookup_in(ROOT, b"deep").unwrap().unwrap(); fs.drop_entry(ROOT, off).unwrap(); });
        let (mut fs, r) = repair(dev, store.mem(), false).unwrap();
        assert_eq!(r.orphans, 1, "{:?}", r);
        let lost = names(&mut fs, b"/lost+found");
        assert_eq!(lost.len(), 1);
        let path = format!("/lost+found/{}/x/y.txt", lost[0]);
        assert_eq!(read(&mut fs, path.as_bytes()).unwrap(), b"why");
        assert!(check(fs.device(), true).unwrap().clean());
    }

    #[test]
    fn blocks_past_the_end_and_shared_blocks_are_dropped() {
        let mut store = Store::new();
        let dev = sample(&mut store);
        let dev = tamper(dev, &mut store, |fs| {
            let a = fs.open(b"/docs/a.txt").unwrap().id;
            let b = fs.open(b"/docs/b.txt").unwrap().id;
            // b.txt (3 bytes) claims a second block, and a block of a.txt.
            fs.get(Key { id: a, kind: K_BLOCK, off: 0 }).unwrap();
            let shared = fs.value[..16].to_vec();
            fs.insert(Key { id: b, kind: K_BLOCK, off: 5 }, &shared).unwrap();
            // Raise b.txt's size so the shared block is inside it.
            fs.insert(Key { id: b, kind: K_INODE, off: 0 }, &encode_inode(Kind::File, 6 * BLOCK as u64, 1)).unwrap();
        });
        let (mut fs, r) = repair(dev, store.mem(), true).unwrap();
        assert_eq!(r.shared_blocks, 1, "{:?}", r);
        assert_eq!(read(&mut fs, b"/docs/a.txt").unwrap(), vec![b'a'; BLOCK + 100]);
        assert!(check(fs.device(), true).unwrap().clean());
    }

    #[test]
    fn damaged_leaf_takes_lost_files_from_the_older_commit() {
        let mut store = Store::new();
        let mut dev = sample(&mut store);
        // Break the newest root node (a single leaf in this small tree).
        let mut b = [0u8; BLOCK];
        let mut supers = superblocks(&mut dev, &mut Report::default()).unwrap();
        let newest = supers.remove(0);
        dev.read(newest.root, &mut b).unwrap();
        b[BLOCK - 1] ^= 0xFF;
        dev.write(newest.root, &b).unwrap();
        assert!(Fs::mount(&mut dev, store.mem()).is_err());
        let r = check(&mut dev, false).unwrap();
        assert!(r.bad_nodes >= 1 && r.from_older > 0, "{:?}", r);
        let (mut fs, _) = repair(dev, store.mem(), false).unwrap();
        // Everything is back, as of the commit before.
        assert_eq!(read(&mut fs, b"/notes.txt").unwrap(), b"first");
        assert_eq!(read(&mut fs, b"/deep/x/y.txt").unwrap(), b"why");
        assert_eq!(read(&mut fs, b"/docs/a.txt").unwrap(), vec![b'a'; BLOCK + 100]);
        assert!(check(fs.device(), true).unwrap().clean());
        // And it takes new writes.
        let id = fs.create(b"/after.txt", 3).unwrap();
        fs.write(id, 0, b"ok", 3).unwrap();
        fs.commit().unwrap();
    }

    #[test]
    fn damaged_leaf_in_a_big_tree_loses_nothing_older() {
        let mut store = Store::new();
        let mut fs = Fs::format(Ram(vec![[0; BLOCK]; BLOCKS]), store.mem()).unwrap();
        fs.mkdir(b"/many", 1).unwrap();
        for i in 0..300 {
            let id = fs.create(format!("/many/file-{:03}.txt", i).as_bytes(), 1).unwrap();
            fs.write(id, 0, format!("content {}", i).as_bytes(), 1).unwrap();
        }
        fs.commit().unwrap();
        let id = fs.create(b"/many/late.txt", 2).unwrap();
        fs.write(id, 0, b"late", 2).unwrap();
        fs.commit().unwrap();
        let mut dev = fs.into_parts().0;
        // Break a leaf the newest commit wrote (the older commit has its own copy).
        let newest = superblocks(&mut dev, &mut Report::default()).unwrap().remove(0);
        let leaves: StdVec<(u64, u64)> = {
            let s = salvage(&mut dev, BLOCKS as u64, newest.root);
            let mut b = [0u8; BLOCK];
            s.nodes.iter().copied().filter_map(|n| { dev.read(n, &mut b).unwrap(); (level(&b) == 0).then(|| (n, gen_of(&b))) }).collect()
        };
        assert!(leaves.len() > 3, "tree too small: {}", leaves.len());
        let shared: StdVec<u64> = leaves.iter().filter(|l| l.1 < newest.generation).map(|l| l.0).collect();
        let victim = leaves.iter().find(|l| l.1 == newest.generation).unwrap().0;
        let mut shared_dev = Ram(dev.0.clone());
        dev.0[victim as usize][100] ^= 0xFF;
        let r = check(&mut dev, false).unwrap();
        assert_eq!(r.bad_nodes, 1, "{:?}", r);
        let (mut fs, _) = repair(dev, store.mem(), false).unwrap();
        for i in 0..300 {
            let got = read(&mut fs, format!("/many/file-{:03}.txt", i).as_bytes());
            assert_eq!(got.as_deref(), Some(format!("content {}", i).as_bytes()), "file {}", i);
        }
        assert!(check(fs.device(), true).unwrap().clean());

        // A leaf both commits share: what it held is gone, the rest is whole and named.
        shared_dev.0[shared[shared.len() / 2] as usize][100] ^= 0xFF;
        let mut store = Store::new();
        let (mut fs, r) = repair(shared_dev, store.mem(), false).unwrap();
        assert!(r.bad_nodes == 1 && !r.clean(), "{:?}", r);
        assert!(check(fs.device(), true).unwrap().clean());
        let mut found = 0;
        let lost = if fs.open(b"/lost+found").is_ok() { names(&mut fs, b"/lost+found") } else { StdVec::new() };
        for i in 0..300 {
            if let Some(got) = read(&mut fs, format!("/many/file-{:03}.txt", i).as_bytes()) { assert_eq!(got, format!("content {}", i).as_bytes()); found += 1; }
        }
        for name in &lost {
            let got = read(&mut fs, format!("/lost+found/{}", name).as_bytes()).unwrap();
            assert!(got.is_empty() || got.starts_with(b"content "), "{:?}", got);
            found += 1;
        }
        assert!(found >= 250, "only {} of 300 files survived one bad leaf", found);
    }

    #[test]
    fn bad_data_is_reported_by_a_deep_check() {
        let mut store = Store::new();
        let mut dev = sample(&mut store);
        let block = {
            let mut fs = Fs::mount(&mut dev, store.mem()).unwrap();
            let id = fs.open(b"/docs/b.txt").unwrap().id;
            fs.get(Key { id, kind: K_BLOCK, off: 0 }).unwrap();
            get_u64(&fs.value, 0)
        };
        dev.0[block as usize][0] ^= 1;
        assert!(check(&mut dev, false).unwrap().clean());
        assert_eq!(check(&mut dev, true).unwrap().bad_data, 1);
    }

    #[test]
    fn both_superblocks_gone_is_reported_not_guessed() {
        let mut store = Store::new();
        let mut dev = sample(&mut store);
        dev.0[0][60] ^= 1;
        dev.0[1][60] ^= 1;
        assert_eq!(check(&mut dev, false), Err(Error::Corrupt));
    }
}
