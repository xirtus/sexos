// SPDX-License-Identifier: GPL-3.0-only
//! Power cuts, on the host: a workload of creates, writes, renames, truncates and removes,
//! each committed, is cut off after every possible number of device writes. What reached
//! the disk is then what a real disk might keep: everything flushed, and of the writes
//! not yet flushed any subset, some of them torn half-way through a block. The disk must
//! then mount, check clean, and hold exactly the files of the last commit that returned,
//! or of the one in flight if its superblock got there.
use super::*;
use std::collections::BTreeMap;
use std::vec::Vec;

/// A tiny deterministic generator, so a failure names its seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
    fn below(&mut self, n: u64) -> u64 { self.next() % n }
}

const BLOCKS: usize = 2048;

/// The disk: `stable` is what survives; `volatile` holds writes since the last flush.
struct CutDisk { stable: Vec<[u8; BLOCK]>, volatile: Vec<(u64, [u8; BLOCK])>, writes_left: u64, dead: bool }
impl CutDisk {
    /// What survives a power cut now, chosen by `rng`.
    fn cut(mut self, rng: &mut Rng) -> Vec<[u8; BLOCK]> {
        for (block, data) in self.volatile.drain(..) {
            match rng.below(4) {
                0 | 1 => {}
                2 => self.stable[block as usize] = data,
                _ => { let half = (rng.below(7) + 1) as usize * 512; self.stable[block as usize][..half].copy_from_slice(&data[..half]); }
            }
        }
        self.stable
    }
}
impl Device for &mut CutDisk {
    fn blocks(&self) -> u64 { BLOCKS as u64 }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> core::result::Result<(), ()> {
        if self.dead { return Err(()); }
        // Reads see this run's own unflushed writes.
        *out = self.volatile.iter().rev().find(|(b, _)| *b == block).map(|(_, d)| *d).unwrap_or(self.stable[block as usize]);
        Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> core::result::Result<(), ()> {
        if self.dead || self.writes_left == 0 { self.dead = true; return Err(()); }
        self.writes_left -= 1;
        self.volatile.push((block, *data));
        Ok(())
    }
    fn flush(&mut self) -> core::result::Result<(), ()> {
        if self.dead { return Err(()); }
        for (block, data) in self.volatile.drain(..) { self.stable[block as usize] = data; }
        Ok(())
    }
}
struct Plain(Vec<[u8; BLOCK]>);
impl Device for &mut Plain {
    fn blocks(&self) -> u64 { self.0.len() as u64 }
    fn read(&mut self, b: u64, out: &mut [u8; BLOCK]) -> core::result::Result<(), ()> { *out = self.0[b as usize]; Ok(()) }
    fn write(&mut self, b: u64, d: &[u8; BLOCK]) -> core::result::Result<(), ()> { self.0[b as usize] = *d; Ok(()) }
    fn flush(&mut self) -> core::result::Result<(), ()> { Ok(()) }
}

struct Store { cache: Vec<[u8; BLOCK]>, slots: Vec<Slot>, used: Vec<u64>, pending: Vec<u64>, scratch: Box<[u8; BLOCK]> }
impl Store {
    fn new() -> Self { Store { cache: vec![[0; BLOCK]; 24], slots: vec![Slot::EMPTY; 24], used: vec![0; 64], pending: vec![0; 64], scratch: Box::new([0; BLOCK]) } }
    fn mem(&mut self) -> Memory<'_> { Memory { cache: &mut self.cache, slots: &mut self.slots, used: &mut self.used, pending: &mut self.pending, scratch: &mut self.scratch } }
}

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

/// One step of the workload, on the filesystem and the model alike.
fn step(fs: &mut Fs<'_, &mut CutDisk>, model: &mut Model, k: u64, rng: &mut Rng) -> Result<()> {
    let name = |i: u64| format!("/d/f{}", i % 9).into_bytes();
    let path = name(rng.below(9));
    match rng.below(6) {
        0 | 1 | 2 => {
            // Write a few blocks' worth at an offset, into a new or existing file.
            let id = match fs.open(&path) { Ok(st) => st.id, Err(Error::NotFound) => fs.create(&path, k)?, Err(e) => return Err(e) };
            let len = rng.below(3 * BLOCK as u64) as usize + 1;
            let at = rng.below(2 * BLOCK as u64) as usize;
            let data: Vec<u8> = (0..len).map(|i| (k as usize * 31 + i) as u8).collect();
            fs.write(id, at as u64, &data, k)?;
            let file = model.entry(path).or_default();
            if file.len() < at + len { file.resize(at + len, 0); }
            file[at..at + len].copy_from_slice(&data);
        }
        3 => if let Ok(st) = fs.open(&path) {
            let size = rng.below(st.size + 1);
            fs.truncate(st.id, size, k)?;
            model.get_mut(&path).unwrap().resize(size as usize, 0);
        },
        4 => if model.contains_key(&path) {
            let to = name(rng.below(9));
            if to != path && !model.contains_key(&to) {
                fs.rename(&path, &to)?;
                let v = model.remove(&path).unwrap();
                model.insert(to, v);
            }
        },
        _ => if model.contains_key(&path) { fs.remove_path(&path)?; model.remove(&path); },
    }
    fs.commit()
}

fn contents(fs: &mut Fs<'_, &mut Plain>) -> Model {
    let mut out = Model::new();
    let dir = fs.open(b"/d").unwrap().id;
    let (mut cursor, mut name) = (0, [0u8; NAME_MAX]);
    while let Some((len, _, id, next)) = fs.read_dir(dir, cursor, &mut name).unwrap() {
        let st = fs.stat(id).unwrap();
        let mut data = vec![0u8; st.size as usize];
        let n = fs.read(id, 0, &mut data).unwrap();
        assert_eq!(n, data.len());
        let mut path = b"/d/".to_vec();
        path.extend_from_slice(&name[..len]);
        out.insert(path, data);
        cursor = next;
    }
    out
}

/// Runs the workload until the disk dies after `budget` writes; returns the image, and
/// the model after the last commit that returned and after the one in flight.
fn run(seed: u64, budget: u64, steps: u64, base: &[[u8; BLOCK]]) -> (Vec<[u8; BLOCK]>, Model, Model, bool) {
    let mut rng = Rng(seed);
    let mut disk = CutDisk { stable: base.to_vec(), volatile: Vec::new(), writes_left: budget, dead: false };
    let mut store = Store::new();
    let (mut done, mut flight) = (Model::new(), Model::new());
    let mut finished = false;
    {
        let mut fs = Fs::mount(&mut disk, store.mem()).unwrap();
        for k in 0..steps {
            let mut next = done.clone();
            match step(&mut fs, &mut next, k + 2, &mut rng) {
                Ok(()) => done = next,
                Err(_) => { flight = next; break; }
            }
            if k + 1 == steps { finished = true; }
        }
    }
    if finished { flight = done.clone(); }
    let image = disk.cut(&mut rng);
    (image, done, flight, finished)
}

#[test]
fn every_power_cut_leaves_a_whole_commit() {
    // A formatted disk with the folder, committed.
    let mut base = Plain(vec![[0; BLOCK]; BLOCKS]);
    {
        let mut store = Store::new();
        let mut fs = Fs::format(&mut base, store.mem()).unwrap();
        fs.mkdir(b"/d", 1).unwrap();
        fs.commit().unwrap();
    }
    let steps = 120;
    // How many writes the whole workload takes, so every cut point is tried.
    let mut cuts = 0;
    for seed in [1u64, 7, 99, 12345, 4242, 31337, 8, 2026] {
        let (_, _, _, finished) = run(seed, u64::MAX, steps, &base.0);
        assert!(finished);
        let total = {
            let mut rng = Rng(seed);
            let mut disk = CutDisk { stable: base.0.clone(), volatile: Vec::new(), writes_left: u64::MAX, dead: false };
            let mut store = Store::new();
            let mut fs = Fs::mount(&mut disk, store.mem()).unwrap();
            let mut model = Model::new();
            for k in 0..steps { step(&mut fs, &mut model, k + 2, &mut rng).unwrap(); }
            drop(fs);
            u64::MAX - disk.writes_left
        };
        for budget in 0..=total {
            let (image, done, flight, _) = run(seed, budget, steps, &base.0);
            let mut plain = Plain(image);
            let mut store = Store::new();
            let mut fs = Fs::mount(&mut plain, store.mem()).unwrap_or_else(|e| panic!("seed {} cut after {} writes: no mount: {:?}", seed, budget, e));
            fs.verify().unwrap_or_else(|e| panic!("seed {} cut after {} writes: {:?}", seed, budget, e));
            let got = contents(&mut fs);
            assert!(got == done || got == flight, "seed {} cut after {} writes: files are neither the last commit nor the next", seed, budget);
            drop(fs);
            let report = crate::check::check(&mut &mut plain, true).unwrap();
            assert!(report.clean(), "seed {} cut after {} writes: {:?}", seed, budget, report);
            cuts += 1;
        }
    }
    eprintln!("[sexfs.powercut] cuts={} all whole", cuts);
    assert!(cuts > 2000, "too few cut points: {}", cuts);
}
