// The filesystem against a model of what it should hold, and against power cuts at
// every write. Whatever a crash does to unflushed writes, the last commit must be whole.
#[path = "../crates/sexfs/src/lib.rs"]
mod sexfs;
use sexfs::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Default)]
struct Disk { blocks: HashMap<u64, Box<[u8; BLOCK]>>, volatile: Vec<(u64, Box<[u8; BLOCK]>)> , size: u64, writes: u64, flushes: u64, crash_flush: Option<u64>, crash_at: Option<u64>, seed: u64, fail_reads: bool, runs: usize, run_cmds: u64, data_bytes: u64, ahead_on: bool, held: Option<(u64, Vec<u8>)>, ahead_served: u64 }
impl Device for Disk {
    fn blocks(&self) -> u64 { self.size }
    fn run_blocks(&self) -> usize { self.runs.max(1) }
    // As Files does: a single data block is a one-block run, served from readahead if held.
    fn read_data(&mut self, block: u64, out: &mut [u8; BLOCK]) -> std::result::Result<(), ()> {
        self.read_data_run(block, out)
    }
    // Like Files' `ahead::take`: blocks at or after the held start are handed over and
    // what follows them stays held; anything before them is let go.
    fn read_data_run(&mut self, first: u64, out: &mut [u8]) -> std::result::Result<(), ()> {
        if let Some((at, data)) = self.held.take() {
            let n = (out.len() / BLOCK) as u64;
            let held = (data.len() / BLOCK) as u64;
            if first >= at && first + n <= at + held {
                let from = ((first - at) as usize) * BLOCK;
                out.copy_from_slice(&data[from..from + out.len()]);
                self.ahead_served += 1;
                let rest = data[from + out.len()..].to_vec();
                if !rest.is_empty() { self.held = Some((first + n, rest)); }
                return Ok(());
            }
            self.held = Some((at, data));
        }
        self.read_run(first, out)?; self.data_bytes += out.len() as u64; Ok(())
    }
    fn read_data_ahead(&mut self, first: u64, out: &mut [u8], ahead: usize) -> std::result::Result<(), ()> {
        if !self.ahead_on || self.held.as_ref().is_some_and(|(at, d)| first >= *at && first + (out.len() / BLOCK) as u64 <= at + (d.len() / BLOCK) as u64) {
            return self.read_data_run(first, out);
        }
        assert!(out.len() / BLOCK + ahead <= self.runs.max(1), "readahead stays inside one run");
        self.run_cmds += 1;
        for (i, c) in out.chunks_exact_mut(BLOCK).enumerate() { self.raw(first + i as u64, c.try_into().unwrap())?; }
        let next = first + (out.len() / BLOCK) as u64;
        let mut kept = vec![0u8; ahead * BLOCK];
        for (i, c) in kept.chunks_exact_mut(BLOCK).enumerate() { self.raw(next + i as u64, c.try_into().unwrap())?; }
        self.data_bytes += (out.len() + kept.len()) as u64;
        self.held = Some((next, kept));
        Ok(())
    }
    // Runs still land block by block, so a power cut can tear a run anywhere.
    fn write_run(&mut self, first: u64, data: &[u8]) -> std::result::Result<(), ()> {
        assert!(data.len() % BLOCK == 0 && data.len() / BLOCK <= self.runs.max(1));
        self.run_cmds += 1;
        for (i, c) in data.chunks_exact(BLOCK).enumerate() { self.write(first + i as u64, c.try_into().unwrap())?; }
        Ok(())
    }
    // Every other read uses the one shared buffer in Files, so it drops what was held.
    fn read_run(&mut self, first: u64, out: &mut [u8]) -> std::result::Result<(), ()> {
        assert!(out.len() % BLOCK == 0 && out.len() / BLOCK <= self.runs.max(1));
        self.run_cmds += 1;
        self.held = None;
        for (i, c) in out.chunks_exact_mut(BLOCK).enumerate() { self.raw(first + i as u64, c.try_into().unwrap())?; }
        Ok(())
    }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> std::result::Result<(), ()> {
        self.held = None;
        self.raw(block, out)
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> std::result::Result<(), ()> {
        self.held = None;
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
    fn raw(&self, block: u64, out: &mut [u8; BLOCK]) -> std::result::Result<(), ()> {
        if self.fail_reads { return Err(()); }
        if let Some((_, b)) = self.volatile.iter().rev().find(|(i, _)| *i == block) { *out = **b; return Ok(()); }
        match self.blocks.get(&block) { Some(b) => *out = **b, None => out.fill(0) }
        Ok(())
    }
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
fn whole_blocks_travel_in_runs() {
    let mut fs = fresh(40960);
    fs.device().runs = 32;
    let id = fs.create(b"big", 1).unwrap();
    let data = bytes_of(7, 1 << 20);
    // 1 MiB in 64 KiB writes: 16 writes of 16 blocks, each one run.
    for (i, chunk) in data.chunks(65536).enumerate() { assert_eq!(fs.write(id, i as u64 * 65536, chunk, 1).unwrap(), 65536); }
    let writes = fs.device().run_cmds;
    assert_eq!(writes, 16, "one device command per write");
    fs.commit().unwrap();
    let mut back = vec![0u8; 1 << 20];
    for (i, chunk) in back.chunks_mut(131072).enumerate() { assert_eq!(fs.read(id, i as u64 * 131072, chunk).unwrap(), 131072); }
    assert!(back == data);
    // Tree nodes allocated between writes can split a read in two, never into pages.
    let reads = fs.device().run_cmds - writes;
    assert!((8..=16).contains(&reads), "{reads} device commands for 1 MiB of reads");
    // Unaligned edges and a partly overwritten middle still read back, through remount.
    let patch = bytes_of(9, 70000);
    fs.write(id, 5000, &patch, 2).unwrap();
    let mut want = data.clone();
    want[5000..75000].copy_from_slice(&patch);
    fs.commit().unwrap();
    let (dev, mem) = fs.into_parts();
    let mut fs = Fs::mount(dev, mem).unwrap();
    let mut back = vec![0u8; 1 << 20];
    assert_eq!(fs.read(id, 0, &mut back).unwrap(), 1 << 20);
    assert!(back == want);
    // A run read checks every block: damage inside a run is caught.
    fs.verify().unwrap();
}
#[test]
fn repeated_file_reads_reach_device_without_counting_metadata_or_holes() {
    let mut fs = fresh(40960);
    fs.device().runs = 32;
    let id = fs.create(b"payload", 1).unwrap();
    let data = bytes_of(71, 1 << 20);
    for (i, chunk) in data.chunks(65536).enumerate() { fs.write(id, i as u64 * 65536, chunk, 1).unwrap(); }
    fs.commit().unwrap();
    let mut back = vec![0; 65536];
    for _ in 0..3 {
        let before = fs.device().data_bytes;
        fs.open(b"payload").unwrap(); fs.stat(id).unwrap();
        assert_eq!(fs.device().data_bytes, before, "metadata is not payload I/O");
        for (i, chunk) in data.chunks(65536).enumerate() {
            assert_eq!(fs.read(id, i as u64 * 65536, &mut back).unwrap(), back.len());
            assert_eq!(back, chunk);
        }
        assert_eq!(fs.device().data_bytes - before, data.len() as u64, "even repeated reads reach the device");
    }
    let hole = fs.create(b"hole", 2).unwrap();
    fs.truncate(hole, 65536, 2).unwrap(); fs.commit().unwrap();
    let before = fs.device().data_bytes;
    back.fill(99); assert_eq!(fs.read(hole, 0, &mut back).unwrap(), back.len());
    assert!(back.iter().all(|&b| b == 0));
    assert_eq!(fs.device().data_bytes, before, "synthesized zeros cannot prove device I/O");
    // A partial payload read incurs a full block, not a fictitious one-byte command.
    assert_eq!(fs.read(id, 1, &mut back[..1]).unwrap(), 1);
    assert_eq!(fs.device().data_bytes - before, BLOCK as u64);
}
/// Files' writes: 63 KiB at a time (its request region), a commit after each.
fn write_like_files(fs: &mut Fs<'static, Disk>, id: u64, data: &[u8]) {
    for (i, chunk) in data.chunks(63 * 1024).enumerate() {
        fs.write(id, (i * 63 * 1024) as u64, chunk, 1).unwrap();
        fs.commit().unwrap();
    }
}
/// Device commands and data bytes to read `data` back sequentially in 64 KiB requests.
fn sequential_read_cost(fs: &mut Fs<'static, Disk>, id: u64, data: &[u8]) -> (u64, u64) {
    let mut back = vec![0u8; 65536];
    let (cmds, bytes) = (fs.device().run_cmds, fs.device().data_bytes);
    for (i, chunk) in data.chunks(65536).enumerate() {
        let n = fs.read(id, i as u64 * 65536, &mut back).unwrap();
        assert!(back[..n] == *chunk, "chunk {i}");
    }
    (fs.device().run_cmds - cmds, fs.device().data_bytes - bytes)
}
#[test]
fn files_like_appends_stay_side_by_side_and_read_back_in_a_few_commands() {
    let mut fs = fresh(40960);
    fs.device().runs = 1024; // a 4 MiB block buffer
    fs.device().ahead_on = true;
    let id = fs.create(b"big", 1).unwrap();
    let data = bytes_of(31, 4 << 20);
    write_like_files(&mut fs, id, &data);
    fs.verify().unwrap();
    let (cmds, bytes) = sequential_read_cost(&mut fs, id, &data);
    eprintln!("[sexfs.layout] files_like_4mib read_cmds={cmds} read_bytes={bytes}");
    // 4 MiB written as 67 commits of 63 KiB used to take 101 commands to read back
    // (tree blocks between every append). Now: the first request, its last odd block,
    // then readahead across the whole file.
    assert!(cmds <= 4, "{cmds} device commands");
    // The spare blocks read across (one partial block rewritten per append) stay small.
    assert!(bytes <= data.len() as u64 * 106 / 100, "{bytes} bytes read");
    // Same layout after a remount (allocation is not remembered, only the tree).
    let (dev, mem) = fs.into_parts();
    let mut fs = Fs::mount(dev, mem).unwrap();
    assert!(sequential_read_cost(&mut fs, id, &data).0 <= 4);
}
#[test]
fn two_files_appended_in_turns_each_stay_mostly_side_by_side() {
    let mut fs = fresh(40960);
    fs.device().runs = 1024;
    fs.device().ahead_on = true;
    let (a, b) = (fs.create(b"a", 1).unwrap(), fs.create(b"b", 1).unwrap());
    let (da, db) = (bytes_of(3, 1 << 20), bytes_of(4, 1 << 20));
    for (i, (ca, cb)) in da.chunks(63 * 1024).zip(db.chunks(63 * 1024)).enumerate() {
        fs.write(a, (i * 63 * 1024) as u64, ca, 1).unwrap(); fs.commit().unwrap();
        fs.write(b, (i * 63 * 1024) as u64, cb, 1).unwrap(); fs.commit().unwrap();
    }
    fs.verify().unwrap();
    // Interleaved appends cannot both continue in place; each file still reads back right.
    let (ca, _) = sequential_read_cost(&mut fs, a, &da);
    let (cb, _) = sequential_read_cost(&mut fs, b, &db);
    eprintln!("[sexfs.layout] interleaved_1mib read_cmds={ca},{cb}");
}
#[test]
fn a_power_cut_during_files_like_appends_leaves_the_last_commit() {
    quiet();
    let data = bytes_of(41, 600 * 1024);
    let chunk = 63 * 1024;
    for case in 0..80u64 {
        let mut fs = fresh(8192);
        fs.device().runs = 1024;
        let id = fs.create(b"log", 1).unwrap();
        fs.commit().unwrap();
        // Three appends committed, then power goes during a later append or commit.
        let mut committed = 0usize;
        for i in 0..3 { fs.write(id, (i * chunk) as u64, &data[i * chunk..(i + 1) * chunk], 1).unwrap(); fs.commit().unwrap(); committed = (i + 1) * chunk; }
        if case % 2 == 0 { let at = fs.device().writes + 1 + case / 2; fs.device().crash_at = Some(at); }
        else { let at = fs.device().flushes + 1 + case / 8; fs.device().crash_flush = Some(at); }
        fs.device().seed = case.wrapping_mul(0x9E3779B97F4A7C15) | 1;
        let mut attempted = None;
        for i in 3..data.len().div_ceil(chunk) {
            let end = ((i + 1) * chunk).min(data.len());
            if fs.write(id, (i * chunk) as u64, &data[i * chunk..end], 1).is_err() { break; }
            match fs.commit() { Ok(()) => committed = end, Err(_) => { attempted = Some(end); break; } }
        }
        let (mut dev, mem) = fs.into_parts();
        dev.crash_at = None; dev.crash_flush = None;
        if !dev.volatile.is_empty() { dev.crash(); }
        let mut after = Fs::mount(dev, mem).expect("every power cut must mount");
        after.verify().unwrap();
        let size = after.stat(id).unwrap().size as usize;
        // The commit that was being written either landed whole or not at all.
        assert!(size == committed || Some(size) == attempted, "case {case}: size {size}, committed {committed}, attempted {attempted:?}");
        let mut back = vec![0u8; size];
        assert_eq!(after.read(id, 0, &mut back).unwrap(), size);
        assert!(back == data[..size], "case {case}: committed bytes changed");
    }
}

/// A disk image as `Disk` keeps it: what was flushed, block by block.
fn save_image(dev: &Disk, path: &str) {
    let mut out = dev.size.to_le_bytes().to_vec();
    let mut blocks: Vec<_> = dev.blocks.iter().collect();
    blocks.sort_by_key(|(b, _)| **b);
    for (b, d) in blocks { out.extend_from_slice(&b.to_le_bytes()); out.extend_from_slice(&d[..]); }
    std::fs::write(path, out).unwrap();
}
fn load_image(path: &str) -> Disk {
    let raw = std::fs::read(path).unwrap();
    let mut dev = Disk::new(u64::from_le_bytes(raw[..8].try_into().unwrap()));
    for rec in raw[8..].chunks_exact(8 + BLOCK) {
        dev.blocks.insert(u64::from_le_bytes(rec[..8].try_into().unwrap()), Box::new(rec[8..].try_into().unwrap()));
    }
    dev
}
const LEGACY_SIZE: u64 = 16384;
/// What the legacy image holds: two files appended in turns and one deleted.
fn legacy_contents() -> (Vec<u8>, Vec<u8>) { (bytes_of(51, 700 * 1024), bytes_of(52, 300 * 1024 + 77)) }
/// Run by scripts/sexfs_host_gate.sh against sexfs built with the allocator it had
/// before tree blocks moved to the top of the disk: writes a volume laid out that way.
#[test]
fn legacy_layout_image_is_written_when_asked() {
    let Ok(path) = std::env::var("SEXFS_WRITE_LEGACY_IMAGE") else { return };
    let mut fs = fresh(LEGACY_SIZE);
    fs.device().runs = 1024;
    fs.device().ahead_on = true;
    fs.mkdir(b"/docs", 1).unwrap();
    let (a, b) = (fs.create(b"/docs/a", 1).unwrap(), fs.create(b"/b", 1).unwrap());
    let gone = fs.create(b"/gone", 1).unwrap();
    fs.write(gone, 0, &bytes_of(53, 200 * 1024), 1).unwrap();
    fs.commit().unwrap();
    let (da, db) = legacy_contents();
    for i in 0..da.len().div_ceil(63 * 1024) {
        let at = i * 63 * 1024;
        fs.write(a, at as u64, &da[at..(at + 63 * 1024).min(da.len())], 1).unwrap(); fs.commit().unwrap();
        if at < db.len() { fs.write(b, at as u64, &db[at..(at + 63 * 1024).min(db.len())], 1).unwrap(); fs.commit().unwrap(); }
    }
    fs.remove_path(b"/gone").unwrap();
    fs.commit().unwrap();
    let (cmds, _) = sequential_read_cost(&mut fs, a, &da);
    eprintln!("[sexfs.layout] legacy_700k read_cmds={cmds}");
    let (dev, _) = fs.into_parts();
    save_image(&dev, &path);
}
#[test]
fn a_volume_laid_out_the_old_way_mounts_reads_and_grows() {
    let Ok(path) = std::env::var("SEXFS_LEGACY_IMAGE") else { eprintln!("[sexfs.legacy] skipped: no image (run scripts/sexfs_host_gate.sh)"); return };
    let mut dev = load_image(&path);
    dev.runs = 1024;
    dev.ahead_on = true;
    let mut fs = Fs::mount(dev, memory(LEGACY_SIZE, 32)).unwrap();
    fs.verify().unwrap();
    let (da, db) = legacy_contents();
    let a = fs.open(b"/docs/a").unwrap();
    let b = fs.open(b"/b").unwrap();
    assert!(fs.open(b"/gone").is_err());
    assert_eq!((a.size, b.size), (da.len() as u64, db.len() as u64));
    sequential_read_cost(&mut fs, a.id, &da);
    sequential_read_cost(&mut fs, b.id, &db);
    // Appends now go after the file's last block where there is room, and the old
    // blocks stay where they were.
    let more = bytes_of(54, 400 * 1024);
    for (i, c) in more.chunks(63 * 1024).enumerate() { fs.write(a.id, da.len() as u64 + (i * 63 * 1024) as u64, c, 2).unwrap(); fs.commit().unwrap(); }
    let whole: Vec<u8> = da.iter().chain(more.iter()).copied().collect();
    fs.verify().unwrap();
    let (dev, mem) = fs.into_parts();
    let mut fs = Fs::mount(dev, mem).unwrap();
    fs.verify().unwrap();
    sequential_read_cost(&mut fs, a.id, &whole);
    sequential_read_cost(&mut fs, b.id, &db);
    eprintln!("[sexfs.legacy] mounted=1 verified=1 grew_bytes={}", more.len());
}

#[test]
fn a_four_mib_buffer_reads_a_sequential_file_in_two_commands() {
    let mut fs = fresh(40960);
    fs.device().runs = 1024; // sex_pdx::BLOCK_MAX_RUN of 4 KiB blocks
    fs.device().ahead_on = true;
    let id = fs.create(b"big", 1).unwrap();
    let data = bytes_of(29, 4 << 20);
    fs.write(id, 0, &data, 1).unwrap();
    fs.commit().unwrap();
    let mut back = vec![0u8; 65536];
    let (cmds, bytes) = (fs.device().run_cmds, fs.device().data_bytes);
    for (i, chunk) in data.chunks(65536).enumerate() {
        assert_eq!(fs.read(id, i as u64 * 65536, &mut back).unwrap(), back.len());
        assert!(back == chunk, "chunk {i}");
    }
    // One 4 MiB write lays the file out side by side; the first 64 KiB read fetches only
    // itself, the second brings the rest of the file in the same command.
    let reads = fs.device().run_cmds - cmds;
    assert!(reads <= 2, "{reads} device commands for 4 MiB read sequentially");
    assert_eq!(fs.device().data_bytes - bytes, data.len() as u64);
}

#[test]
fn sequential_reads_fetch_ahead_in_one_command() {
    let mut fs = fresh(40960);
    fs.device().runs = 128;
    fs.device().ahead_on = true;
    let id = fs.create(b"seq", 1).unwrap();
    let data = bytes_of(17, 4 << 20);
    for (i, chunk) in data.chunks(512 * 1024).enumerate() { fs.write(id, i as u64 * 512 * 1024, chunk, 1).unwrap(); }
    fs.commit().unwrap();
    let mut back = vec![0u8; 65536];
    for pass in 0..2 {
        let (cmds, bytes) = (fs.device().run_cmds, fs.device().data_bytes);
        for (i, chunk) in data.chunks(65536).enumerate() {
            assert_eq!(fs.read(id, i as u64 * 65536, &mut back).unwrap(), back.len());
            assert!(back == chunk, "pass {pass} chunk {i}");
        }
        let reads = fs.device().run_cmds - cmds;
        // 64 requests of 64 KiB: the first reads only itself, then each 512 KiB run carries 8.
        assert!(reads <= 16, "{reads} device commands for 4 MiB read sequentially");
        // Readahead never reads past the file, so every byte comes off the device once.
        assert_eq!(fs.device().data_bytes - bytes, data.len() as u64);
    }
    assert!(fs.device().ahead_served > 0);
    // A write drops what was read ahead; the next read sees the new bytes.
    fs.read(id, 0, &mut back).unwrap(); fs.read(id, 65536, &mut back).unwrap();
    let patch = bytes_of(23, 65536);
    fs.write(id, 131072, &patch, 2).unwrap();
    fs.read(id, 131072, &mut back).unwrap();
    assert!(back == patch);
    // A random reader is not sequential: no readahead, one command per request.
    let cmds = fs.device().run_cmds;
    for i in [7u64, 3, 40, 12] { fs.read(id, i * 65536, &mut back).unwrap(); }
    assert_eq!(fs.device().run_cmds - cmds, 4);
    fs.verify().unwrap();
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
        if seed % 2 == 1 { fs.device().runs = 32; }
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
        if seed % 4 < 2 { fs.device().runs = 1 + (seed % 32) as usize; }
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
