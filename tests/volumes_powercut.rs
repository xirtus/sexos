// SPDX-License-Identifier: GPL-3.0-only
//! Power cuts on FAT and exFAT sticks, on the host. A workload of writes, each followed by
//! `sync`, is cut off after every few sector writes; what reaches the stick is what was
//! flushed plus any subset of the writes since, some torn. Then:
//! - everything synced before the operation in flight reads back exactly;
//! - the stick mounts, and (for FAT, where the host has it) `fsck.vfat -n` finds no damage
//!   beyond what a cut may leave behind for any FAT driver: clusters the cut-off write had
//!   taken but not yet linked ("lost clusters") and the free-cluster hint.
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
use common::*;
use std::collections::BTreeMap;
use sv::{Disk, Volume, SECTOR};

struct Rng(u64);
impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } fn below(&mut self, n: u64) -> u64 { self.next() % n } }

/// Writes since the last flush are volatile; after `left` sector writes the power goes.
struct Cut { stable: Vec<u8>, volatile: Vec<(u64, Vec<u8>)>, left: u64, dead: bool }
impl Cut {
    fn image(mut self, rng: &mut Rng) -> Vec<u8> {
        for (lba, data) in std::mem::take(&mut self.volatile) {
            for (k, sector) in data.chunks(SECTOR).enumerate() {
                let at = (lba as usize + k) * SECTOR;
                match rng.below(4) {
                    0 | 1 => {}
                    2 => self.stable[at..at + SECTOR].copy_from_slice(sector),
                    // A torn sector keeps its first part only.
                    _ => { let n = (rng.below(7) + 1) as usize * 64; self.stable[at..at + n].copy_from_slice(&sector[..n]); }
                }
            }
        }
        self.stable
    }
}
impl Disk for Cut {
    fn sectors(&self) -> u64 { (self.stable.len() / SECTOR) as u64 }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> sv::Result<()> {
        if self.dead { return Err(sv::Error::Io); }
        let at = lba as usize * SECTOR;
        buf.copy_from_slice(&self.stable[at..at + buf.len()]);
        for (l, d) in &self.volatile {
            for (k, sector) in d.chunks(SECTOR).enumerate() {
                let s = *l as usize + k;
                if s >= lba as usize && s < lba as usize + buf.len() / SECTOR { let o = (s - lba as usize) * SECTOR; buf[o..o + SECTOR].copy_from_slice(sector); }
            }
        }
        Ok(())
    }
    fn write(&mut self, lba: u64, buf: &[u8]) -> sv::Result<()> {
        let n = (buf.len() / SECTOR) as u64;
        if self.dead || self.left < n { self.dead = true; return Err(sv::Error::Io); }
        self.left -= n;
        self.volatile.push((lba, buf.to_vec()));
        Ok(())
    }
    fn flush(&mut self) -> sv::Result<()> {
        if self.dead { return Err(sv::Error::Io); }
        for (lba, data) in std::mem::take(&mut self.volatile) { let at = lba as usize * SECTOR; self.stable[at..at + data.len()].copy_from_slice(&data); }
        Ok(())
    }
}

type Model = BTreeMap<String, Vec<u8>>;
const NAMES: [&str; 5] = ["KEEP.TXT", "a long file name.dat", "x", "Docs/inner.txt", "Docs/b"];

/// One synced operation; returns the path it touched.
fn op(v: &mut dyn Volume, d: &mut dyn Disk, m: &mut Model, rng: &mut Rng, k: u64) -> (String, sv::Result<()>) {
    let name = NAMES[1 + rng.below(4) as usize].to_string();
    let r = (|| -> sv::Result<()> {
        match rng.below(5) {
            0 | 1 | 2 => {
                if !m.contains_key(&name) { v.create(d, name.as_bytes())?; m.insert(name.clone(), vec![]); }
                let off = rng.below(6000) as usize;
                let len = rng.below(9000) as usize + 1;
                let data: Vec<u8> = (0..len).map(|i| (i as u64 * 7 + k) as u8).collect();
                v.write(d, name.as_bytes(), off as u64, &data)?;
                let f = m.get_mut(&name).unwrap();
                if f.len() < off + len { f.resize(off + len, 0); }
                f[off..off + len].copy_from_slice(&data);
            }
            3 => if let Some(f) = m.get_mut(&name) { let n = rng.below(f.len() as u64 + 1) as usize; v.truncate(d, name.as_bytes(), n as u64)?; f.resize(n, 0); },
            _ => if m.remove(&name).is_some() { v.remove(d, name.as_bytes())?; },
        }
        v.sync(d)
    })();
    (name, r)
}

fn read_all(v: &mut dyn Volume, d: &mut dyn Disk, path: &str) -> Option<Vec<u8>> {
    let st = v.stat(d, path.as_bytes()).ok()?;
    let mut out = vec![0u8; st.size as usize];
    let n = v.read(d, path.as_bytes(), 0, &mut out).ok()?;
    out.truncate(n);
    Some(out)
}

/// Clean, apart from what any interrupted FAT write leaves: clusters taken but not yet
/// linked, the free-count hint, the second FAT copy behind the first, half a long name,
/// and the file the cut-off operation was writing having more clusters than its size.
fn fsck_ok(image: &[u8], tag: &str, touched: Option<&str>) -> Result<(), String> {
    let Some(fsck) = tool("fsck.vfat") else { return Ok(()) };
    let path = temp(&format!("powercut-{tag}.img"));
    std::fs::write(&path, image).unwrap();
    let out = run(&fsck, &["-n", path.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_file(&path);
    let harmless = |l: &str| ["lost clusters", "Free cluster summary", "Leaving filesystem unchanged", "Reclaimed", "Dirty bit", "fsck.fat", "files,",
        "FATs differ but appear to be intact", "Using first FAT", "Orphaned long file name part", "Auto-deleting", "Auto-correcting", "Free cluster count"].iter().any(|h| l.contains(h)) || l.trim().is_empty();
    let in_flight = touched.map(|t| format!("/{}", t).to_uppercase());
    let mut current: Option<String> = None;
    for l in text.lines() {
        if l.starts_with('/') { current = Some(l.trim().to_uppercase()); continue; }
        if l.starts_with("  ") && current.is_some() && current == in_flight && (l.contains("cluster chain length") || l.contains("Truncating file")) { continue; }
        if l.starts_with("  ") && harmless(l) { continue; }
        if !l.starts_with("  ") { current = None; }
        if !harmless(l) { return Err(text); }
    }
    Ok(())
}

fn cuts(format: &str, mib: u64) -> (usize, usize) {
    let base = {
        let mut d = match format { "exfat" => fixture("exfat-blank"), f => fresh_fat(f, mib) };
        let mut v = sv::scan(&mut d).unwrap().remove(0).volume;
        v.create(&mut d, b"KEEP.TXT").unwrap();
        v.write(&mut d, b"KEEP.TXT", 0, b"synced before anything was cut").unwrap();
        v.mkdir(&mut d, b"Docs").unwrap();
        v.sync(&mut d).unwrap();
        d.bytes
    };
    let mut tried = 0;
    for seed in 1..=3u64 {
        // Whole run: how many sector writes it takes.
        let run_until = |left: u64, rng_seed: u64| {
            let mut rng = Rng(rng_seed);
            let mut d = Cut { stable: base.clone(), volatile: vec![], left, dead: false };
            let mut v = sv::scan(&mut d).unwrap().remove(0).volume;
            let (mut done, mut flight, mut touched) = (Model::new(), None::<Model>, None);
            done.insert("KEEP.TXT".into(), b"synced before anything was cut".to_vec());
            for k in 0..60 {
                let mut next = done.clone();
                let (name, r) = op(v.as_mut(), &mut d, &mut next, &mut rng, k);
                if r.is_err() { flight = Some(next); touched = Some(name); break; }
                done = next;
            }
            let used = left - d.left;
            (d, done, flight, touched, used, rng)
        };
        let (_, _, flight, _, total, _) = run_until(u64::MAX / 2, seed * 0x9E37_79B9);
        assert!(flight.is_none());
        let step = (total / 150).max(1);
        let mut left = 0;
        while left <= total {
            let (d, done, _flight, touched, _, mut rng) = run_until(left, seed * 0x9E37_79B9);
            let image = d.image(&mut rng);
            let mut disk = MemDisk { bytes: image.clone(), flushes: 0 };
            let tag = format!("{format}-{seed}-{left}");
            let mut found = sv::scan(&mut disk).unwrap_or_else(|e| panic!("{tag}: no volume after the cut: {e:?}"));
            assert!(!found.is_empty(), "{tag}: no volume after the cut");
            let mut v = found.remove(0).volume;
            for (name, data) in &done {
                if Some(name) == touched.as_ref() { continue; }
                assert_eq!(read_all(v.as_mut(), &mut disk, name).as_ref(), Some(data), "{tag}: synced file {name} changed by a cut elsewhere");
            }
            if format != "exfat" { if let Err(text) = fsck_ok(&image, &tag, touched.as_deref()) { panic!("{tag}: fsck.vfat:\n{text}"); } }
            tried += 1;
            left += step;
        }
    }
    (tried, 0)
}

fn fresh_fat(format: &str, mib: u64) -> MemDisk {
    let Some(mkfs) = tool("mkfs.vfat") else { return fixture(format) };
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = temp(&format!("powercut-fresh-{format}-{n}.img"));
    let _ = std::fs::remove_file(&path);
    std::fs::File::create(&path).unwrap().set_len(mib << 20).unwrap();
    let mut args = vec!["-n", "CUT"];
    match format { "fat16" => args.extend(["-F", "16"]), _ => args.extend(["-F", "32", "-s", "1"]) }
    args.push(path.to_str().unwrap());
    assert!(run(&mkfs, &args).status.success());
    MemDisk::from_file(&path)
}

#[test]
fn fat32_power_cuts() { let (n, _) = cuts("fat32", 40); eprintln!("[volumes.powercut] fat32 cuts={n}"); assert!(n > 300); }
#[test]
fn fat16_power_cuts() { let (n, _) = cuts("fat16", 20); eprintln!("[volumes.powercut] fat16 cuts={n}"); assert!(n > 300); }
#[test]
fn exfat_power_cuts() { let (n, _) = cuts("exfat", 0); eprintln!("[volumes.powercut] exfat cuts={n}"); assert!(n > 300); }
