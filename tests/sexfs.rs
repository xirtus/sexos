// The filesystem against a model of what it should hold, and against power cuts at
// every write. Whatever a crash does to unflushed writes, the last commit must be whole.
#[path = "../crates/sexfs/src/lib.rs"]
mod sexfs;
use sexfs::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Default)]
struct Disk { blocks: HashMap<u64, Box<[u8; BLOCK]>>, volatile: Vec<(u64, Box<[u8; BLOCK]>)> , size: u64, writes: u64, flushes: u64, crash_flush: Option<u64>, crash_at: Option<u64>, seed: u64, fail_reads: bool }
impl Device for Disk {
    fn blocks(&self) -> u64 { self.size }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> std::result::Result<(), ()> {
        if self.fail_reads { return Err(()); }
        if let Some((_, b)) = self.volatile.iter().rev().find(|(i, _)| *i == block) { *out = **b; return Ok(()); }
        match self.blocks.get(&block) { Some(b) => *out = **b, None => out.fill(0) }
        Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> std::result::Result<(), ()> {
        self.writes += 1;
        if self.crash_at == Some(self.writes) { self.crash(); return Err(()); }
        if self.crash_at.is_some_and(|c| self.writes > c) { return Err(()); }
        self.volatile.push((block, Box::new(*data)));
        Ok(())
    }
    fn flush(&mut self) -> std::result::Result<(), ()> {
        self.flushes += 1;
        if self.crash_flush == Some(self.flushes) { self.crash(); self.crash_at = Some(0); return Err(()); }
        if self.crash_at.is_some_and(|c| self.writes >= c) { return Err(()); }
        for (b, d) in self.volatile.drain(..) { self.blocks.insert(b, d); }
        Ok(())
    }
}
impl Disk {
    fn new(size: u64) -> Self { Disk { size, ..Default::default() } }
    /// Power cut: some unflushed writes reached the disk, in any order, and one may be torn.
    fn crash(&mut self) {
        let mut pending = std::mem::take(&mut self.volatile);
        for (i, (b, d)) in pending.drain(..).enumerate() {
            self.seed = self.seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(i as u64 + 1);
            match self.seed >> 62 {
                0 => {}
                1 => {
                    let mut torn = self.blocks.get(&b).map(|o| **o).unwrap_or([0; BLOCK]);
                    // Superblocks are small: tear them inside the part that counts.
                    let cut = if b < 2 { (self.seed >> 40) as usize % 8 * 8 } else { (self.seed >> 40) as usize % 128 * 32 };
                    torn[..cut].copy_from_slice(&d[..cut]);
                    self.blocks.insert(b, Box::new(torn));
                }
                _ => { self.blocks.insert(b, d); }
            }
        }
    }
}
fn memory(blocks: u64, cache: usize) -> Memory<'static> {
    let words = (blocks as usize).div_ceil(64) + 1;
    Memory {
        cache: Box::leak(vec![[0u8; BLOCK]; cache].into_boxed_slice()),
        slots: Box::leak((0..cache).map(|_| Slot::EMPTY).collect::<Vec<_>>().into_boxed_slice()),
        used: Box::leak(vec![0u64; words].into_boxed_slice()),
        pending: Box::leak(vec![0u64; words].into_boxed_slice()),
        scratch: Box::leak(Box::new([0u8; BLOCK])),
    }
}
fn fresh(size: u64) -> Fs<'static, Disk> { Fs::format(Disk::new(size), memory(size, 32)).unwrap() }

#[derive(Clone, PartialEq, Debug)]
enum Node { File(Vec<u8>), Dir }
type Model = BTreeMap<String, Node>;
fn bytes_of(state: u64, n: usize) -> Vec<u8> {
    let mut s = state; (0..n).map(|_| { s = s.wrapping_mul(0x100000001B3).wrapping_add(0x9E37); (s >> 40) as u8 }).collect()
}
fn check_model(fs: &mut Fs<'static, Disk>, model: &Model) {
    fs.verify().unwrap();
    for (path, node) in model {
        let st = fs.open(path.as_bytes()).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        match node {
            Node::Dir => assert_eq!(st.kind, Kind::Dir, "{path}"),
            Node::File(want) => {
                assert_eq!(st.size, want.len() as u64, "{path}");
                let mut got = vec![0; want.len()];
                assert_eq!(fs.read(st.id, 0, &mut got).unwrap(), want.len());
                assert!(got == *want, "{path}: contents differ");
            }
        }
    }
    // Directory listings match too, and nothing extra exists.
    let mut dirs = vec![String::new()];
    dirs.extend(model.iter().filter(|(_, n)| **n == Node::Dir).map(|(p, _)| p.clone()));
    for dir in dirs {
        let id = if dir.is_empty() { ROOT } else { fs.open(dir.as_bytes()).unwrap().id };
        let (mut cursor, mut name, mut seen) = (0, [0u8; NAME_MAX], vec![]);
        while let Some((len, _, _, next)) = fs.read_dir(id, cursor, &mut name).unwrap() {
            let leaf = String::from_utf8(name[..len].to_vec()).unwrap();
            seen.push(if dir.is_empty() { leaf } else { format!("{dir}/{leaf}") });
            cursor = next;
        }
        seen.sort();
        let want: Vec<String> = model.keys().filter(|p| match p.rfind('/') { Some(i) => p[..i] == *dir, None => dir.is_empty() }).cloned().collect();
        assert_eq!(seen, want, "listing of {dir:?}");
    }
}
struct Rng(u64);
impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } fn below(&mut self, n: u64) -> u64 { self.next() % n } }

/// One random operation applied to both.
fn step(fs: &mut Fs<'static, Disk>, model: &mut Model, r: &mut Rng, tick: u64) -> sexfs::Result<()> {
    let dirs: Vec<String> = std::iter::once(String::new()).chain(model.iter().filter(|(_, n)| **n == Node::Dir).map(|(p, _)| p.clone())).collect();
    let files: Vec<String> = model.iter().filter(|(_, n)| matches!(n, Node::File(_))).map(|(p, _)| p.clone()).collect();
    let dir = &dirs[r.below(dirs.len() as u64) as usize];
    let leaf = format!("n{}", r.below(40));
    let path = if dir.is_empty() { leaf } else { format!("{dir}/{leaf}") };
    match r.below(10) {
        0 | 1 => if !model.contains_key(&path) { fs.create(path.as_bytes(), tick)?; model.insert(path, Node::File(vec![])); },
        2 => if !model.contains_key(&path) && model.len() < 200 { fs.mkdir(path.as_bytes(), tick)?; model.insert(path, Node::Dir); },
        3 | 4 | 5 if !files.is_empty() => {
            let p = &files[r.below(files.len() as u64) as usize];
            let id = fs.open(p.as_bytes())?.id;
            let cap = if r.below(4) == 0 { 40000 } else { 300 };
            let (off, len) = (r.below(30000), r.below(cap) as usize);
            let data = bytes_of(r.next(), len);
            if fs.write(id, off, &data, tick)? != len { return Ok(()); }
            if let Some(Node::File(c)) = model.get_mut(p) {
                if len > 0 { if c.len() < off as usize + len { c.resize(off as usize + len, 0); } c[off as usize..off as usize + len].copy_from_slice(&data); }
            }
        }
        6 if !files.is_empty() => {
            let p = &files[r.below(files.len() as u64) as usize];
            let id = fs.open(p.as_bytes())?.id;
            let size = r.below(20000);
            fs.truncate(id, size, tick)?;
            if let Some(Node::File(c)) = model.get_mut(p) { c.resize(size as usize, 0); }
        }
        7 if !files.is_empty() => {
            let p = files[r.below(files.len() as u64) as usize].clone();
            fs.remove_path(p.as_bytes())?;
            model.remove(&p);
        }
        8 if !files.is_empty() => {
            let p = files[r.below(files.len() as u64) as usize].clone();
            if matches!(model.get(&path), None | Some(Node::File(_))) {
                fs.rename(p.as_bytes(), path.as_bytes())?;
                if p != path { let n = model.remove(&p).unwrap(); model.insert(path, n); }
            }
        }
        9 => {
            let empties: Vec<String> = dirs.iter().filter(|d| !d.is_empty() && !model.keys().any(|k| k.starts_with(&format!("{d}/")))).cloned().collect();
            if !empties.is_empty() {
                let p = empties[r.below(empties.len() as u64) as usize].clone();
                fs.remove_path(p.as_bytes())?;
                model.remove(&p);
            }
        }
        _ => {}
    }
    Ok(())
}

#[test]
fn basics() {
    let mut fs = fresh(4096);
    fs.mkdir(b"/docs", 1).unwrap();
    let id = fs.create(b"/docs/a.txt", 2).unwrap();
    assert_eq!(fs.write(id, 0, b"hello", 3).unwrap(), 5);
    assert_eq!(fs.create(b"/docs/a.txt", 3), Err(Error::Exists));
    assert_eq!(fs.create(b"/none/a", 3), Err(Error::NotFound));
    assert_eq!(fs.create(b"/docs/a.txt/b", 3), Err(Error::NotDir));
    assert_eq!(fs.remove_path(b"/docs"), Err(Error::NotEmpty));
    let mut out = [0; 16];
    assert_eq!(fs.read(id, 0, &mut out).unwrap(), 5);
    assert_eq!(&out[..5], b"hello");
    assert_eq!(fs.open(b"/docs/a.txt").unwrap(), Stat { id, kind: Kind::File, size: 5, mtime: 3 });
    assert_eq!(fs.create(b"/docs/..", 1), Err(Error::Name));
    assert_eq!(fs.create(b"/", 1), Err(Error::Name));
    assert_eq!(fs.rename(b"/docs", b"/docs/x"), Err(Error::Name));
    fs.commit().unwrap();
}
#[test]
fn holes_and_large_files_read_back() {
    let mut fs = fresh(40960);
    let id = fs.create(b"big", 1).unwrap();
    let data = bytes_of(7, 50_000_000);
    assert_eq!(fs.write(id, 4096 * 3 + 7, &data, 1).unwrap(), data.len());
    fs.commit().unwrap();
    let mut got = vec![0; data.len()];
    assert_eq!(fs.read(id, 4096 * 3 + 7, &mut got).unwrap(), data.len());
    assert!(got == data);
    let mut hole = [9; 100];
    fs.read(id, 0, &mut hole).unwrap();
    assert_eq!(hole, [0; 100]);
    assert!(fs.verify().unwrap().1 >= (data.len() / BLOCK) as u64);
}
#[test]
fn deleting_gives_all_space_back_after_commit() {
    let mut fs = fresh(8192);
    let start = fs.statfs().free;
    for round in 0..3 {
        for i in 0..50 {
            let id = fs.create(format!("f{i}").as_bytes(), round).unwrap();
            fs.write(id, 0, &bytes_of(i, 60000), round).unwrap();
        }
        fs.commit().unwrap();
        for i in 0..50 { fs.remove_path(format!("f{i}").as_bytes()).unwrap(); }
        fs.commit().unwrap();
        // The tree's own shape may differ by a few blocks.
        fs.commit().unwrap();
        assert!(fs.statfs().free + 8 >= start, "round {round}: {} of {start}", fs.statfs().free);
    }
}
#[test]
fn a_full_disk_refuses_cleanly_and_stays_usable() {
    let mut fs = fresh(1024);
    let id = fs.create(b"full", 1).unwrap();
    let data = vec![7u8; BLOCK * 2000];
    let wrote = fs.write(id, 0, &data, 1).unwrap();
    assert!(wrote < data.len());
    fs.commit().unwrap();
    // A commit frees what the last one used, so a little more may fit before the end.
    let mut at = wrote as u64;
    loop {
        match fs.write(id, at, &data, 1) { Ok(n) => { at += n as u64; fs.commit().unwrap(); } Err(e) => { assert_eq!(e, Error::NoSpace); break; } }
    }
    assert_eq!(fs.create(b"more", 1), Err(Error::NoSpace));
    fs.remove_path(b"full").unwrap();
    fs.commit().unwrap();
    fs.create(b"more", 1).unwrap();
    fs.commit().unwrap();
    fs.verify().unwrap();
}
#[test]
fn matches_a_model_through_remounts() {
    for seed in 1..=8u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let mut fs = fresh(20000);
        let mut model = Model::new();
        for tick in 0..4000 {
            step(&mut fs, &mut model, &mut r, tick).unwrap();
            if r.below(50) == 0 { fs.commit().unwrap(); }
            if r.below(400) == 0 {
                fs.commit().unwrap();
                let (dev, mem) = fs.into_parts();
                fs = Fs::mount(dev, mem).unwrap();
                check_model(&mut fs, &model);
            }
        }
        fs.commit().unwrap();
        check_model(&mut fs, &model);
    }
}
#[test]
fn rollback_returns_to_the_last_commit() {
    let mut r = Rng(0xABCDEF);
    let mut fs = fresh(20000);
    let mut model = Model::new();
    for tick in 0..500 { step(&mut fs, &mut model, &mut r, tick).unwrap(); }
    fs.commit().unwrap();
    let saved = model.clone();
    for tick in 0..500 { step(&mut fs, &mut model, &mut r, tick).unwrap(); }
    fs.rollback().unwrap();
    check_model(&mut fs, &saved);
    let id = fs.create(b"after", 1).unwrap();
    fs.write(id, 0, b"x", 1).unwrap();
    fs.commit().unwrap();
}
fn quiet() { std::panic::set_hook(Box::new(|i| { if !i.to_string().contains("sexfs.rs:7") { eprintln!("{i}"); } })); }
#[test]
fn a_power_cut_at_any_write_leaves_a_whole_commit() {
    quiet();
    let mut crashes = 0;
    let mut advanced = 0;
    for seed in 1..=400u64 {
        let mut r = Rng(seed.wrapping_mul(0xD1342543DE82EF95) | 1);
        let mut fs = fresh(20000);
        let mut model = Model::new();
        let mut committed = Model::new();
        // Run far enough to have real state, then cut power at a random write.
        for tick in 0..300 { step(&mut fs, &mut model, &mut r, tick).unwrap(); if r.below(40) == 0 { fs.commit().unwrap(); committed = model.clone(); } }
        fs.commit().unwrap();
        committed = { let _ = committed; model.clone() };
        if seed % 2 == 0 {
            let at = fs.device().writes + 1 + r.below(120);
            fs.device().crash_at = Some(at);
        } else {
            let at = fs.device().flushes + 1 + r.below(12);
            fs.device().crash_flush = Some(at);
        }
        fs.device().seed = r.next();
        let mut tentative = committed.clone();
        let mut finished = None;
        for tick in 0..2000 {
            if step(&mut fs, &mut model, &mut r, tick).is_err() { break; }
            if r.below(10) == 0 {
                let snapshot = model.clone();
                match fs.commit() {
                    Ok(()) => { tentative = snapshot; }
                    Err(_) => { finished = Some(snapshot); break; }
                }
            }
        }
        let attempted = finished;
        let (mut dev, mem) = fs.into_parts();
        dev.crash_at = None;
        dev.crash_flush = None;
        if !dev.volatile.is_empty() { dev.crash(); }
        crashes += 1;
        let mut after = Fs::mount(dev, mem).expect("every power cut must mount");
        // Durable state is the last commit, or the one being written if its superblock landed.
        let matches_old = catch(&mut after, &tentative);
        let matches_new = attempted.as_ref().is_some_and(|m| catch(&mut after, m));
        assert!(matches_old || matches_new, "seed {seed}: neither commit is intact");
        if matches_new && !matches_old { advanced += 1; }
        // And it still works.
        let id = after.create(b"post-crash", 1).unwrap();
        after.write(id, 0, b"ok", 1).unwrap();
        after.commit().unwrap();
        after.verify().unwrap();
    }
    assert!(crashes == 400);
    assert!(advanced > 0, "no cut landed between a superblock write and its flush");
}
fn catch(fs: &mut Fs<'static, Disk>, m: &Model) -> bool {
    let fs_ptr = fs as *mut Fs<'static, Disk>;
    let m2 = m.clone();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || check_model(unsafe { &mut *fs_ptr }, &m2))).is_ok()
}
#[test]
fn damage_is_detected_not_returned() {
    let mut fs = fresh(4096);
    let id = fs.create(b"a", 1).unwrap();
    fs.write(id, 0, &bytes_of(3, 8192), 1).unwrap();
    fs.commit().unwrap();
    let (mut dev, mem) = fs.into_parts();
    // Flip one bit in every block in turn. Each must read true or report damage.
    let want = bytes_of(3, 8192);
    let blocks: Vec<u64> = dev.blocks.keys().copied().filter(|b| *b >= 2).collect();
    let mut mem = Some(mem);
    for b in blocks {
        let original = (*dev.blocks[&b]).clone();
        dev.blocks.get_mut(&b).unwrap()[100] ^= 4;
        let copy = dev.clone();
        let m = mem.take().unwrap();
        match Fs::mount(copy, m) {
            Ok(mut f) => {
                let st = f.open(b"a");
                if let Ok(st) = st { let mut got = vec![0; 8192]; match f.read(st.id, 0, &mut got) { Ok(_) => assert!(got == want, "block {b} returned wrong bytes"), Err(e) => assert_eq!(e, Error::Corrupt) } }
                let (_, m) = f.into_parts();
                mem = Some(m);
            }
            Err(e) => { assert_eq!(e, Error::Corrupt); mem = Some(memory(4096, 32)); }
        }
        dev.blocks.get_mut(&b).unwrap().copy_from_slice(&original);
    }
    let _ = dev.fail_reads;
}
#[test]
fn an_unformatted_disk_says_so() {
    assert!(matches!(Fs::mount(Disk::new(4096), memory(4096, 32)), Err(Error::Unformatted)));
}
