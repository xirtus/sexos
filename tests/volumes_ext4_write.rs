// SPDX-License-Identifier: GPL-3.0-only
//! ext4 written by SexOS, checked by Linux's own tools: volumes made by `mkfs.ext4` (4K
//! and 1K blocks, with and without metadata_csum, 64bit) take creates, writes at odd
//! offsets, holes, files big enough for an extent tree of depth 1 and 2, directories,
//! renames across folders, truncates and removes, against a model. Then `e2fsck -fn`
//! must find nothing to fix and `debugfs` must read the files back as the model has them.
//! Skipped where the host lacks e2fsprogs.
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
use common::*;
use std::collections::BTreeMap;
use sv::{Disk, Error, Kind, Volume};

struct Rng(u64);
impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } fn below(&mut self, n: u64) -> u64 { self.next() % n } }

fn fresh(tag: &str, mib: u64, args: &[&str]) -> Option<(MemDisk, std::path::PathBuf)> {
    let mkfs = tool("mkfs.ext4")?;
    tool("e2fsck")?;
    let path = temp(&format!("ext4w-{tag}.img"));
    let _ = std::fs::remove_file(&path);
    std::fs::File::create(&path).unwrap().set_len(mib << 20).unwrap();
    let mut a = vec!["-q", "-F", "-L", "SEXW", "-E", "lazy_itable_init=1"];
    a.extend_from_slice(args);
    a.push(path.to_str().unwrap());
    let out = run(&mkfs, &a);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some((MemDisk::from_file(&path), path))
}
fn fsck(d: &MemDisk, path: &std::path::Path, what: &str) {
    std::fs::write(path, &d.bytes).unwrap();
    let out = run(&tool("e2fsck").unwrap(), &["-fn", path.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{what}: e2fsck -fn:\n{text}");
}
fn debugfs_cat(path: &std::path::Path, file: &str) -> Vec<u8> {
    let out = run(&tool("debugfs").unwrap(), &["-R", &format!("cat \"/{file}\""), path.to_str().unwrap()]);
    out.stdout
}
fn read_all(v: &mut dyn Volume, d: &mut dyn Disk, path: &str) -> Vec<u8> {
    let size = v.stat(d, path.as_bytes()).unwrap().size as usize;
    let mut buf = vec![0xAA; size];
    assert_eq!(v.read(d, path.as_bytes(), 0, &mut buf).unwrap(), size, "{path}");
    buf
}

type Model = BTreeMap<String, Option<Vec<u8>>>; // None: a directory

fn workload(tag: &str, mib: u64, args: &[&str], seed: u64, steps: usize) {
    let Some((mut d, path)) = fresh(tag, mib, args) else { eprintln!("[volumes.ext4w] {tag}: no e2fsprogs, skipped"); return };
    let mut v = sv::ext4::Ext4::mount(&mut d).unwrap();
    assert!(!v.read_only(), "{tag}: ext4 with extents should be writable");
    v.set_now(1_763_000_000);
    let mut m: Model = BTreeMap::new();
    m.insert("lost+found".into(), None);
    let mut rng = Rng(seed);
    let names = ["a.txt", "b.bin", "dir1", "dir2", "dir1/c.dat", "dir1/sub", "dir1/sub/d", "dir2/e", "a long name with spaces.txt", "dir2/f.bin"];
    for k in 0..steps {
        let name = names[rng.below(names.len() as u64) as usize].to_string();
        let parent_ok = |m: &Model, n: &str| match n.rfind('/') { None => true, Some(i) => matches!(m.get(&n[..i]), Some(None)) };
        match rng.below(8) {
            0..=2 => {
                if !parent_ok(&m, &name) || matches!(m.get(&name), Some(None)) { continue; }
                if !m.contains_key(&name) { v.create(&mut d, name.as_bytes()).unwrap(); m.insert(name.clone(), Some(vec![])); }
                // Mostly small, sometimes a hole far out or a big piece.
                let off = match rng.below(5) { 0 => rng.below(64) , 1 => rng.below(300_000), _ => rng.below(20_000) } as usize;
                let len = if rng.below(6) == 0 { rng.below(400_000) as usize + 1 } else { rng.below(9000) as usize + 1 };
                let data: Vec<u8> = (0..len).map(|i| (i as u64 * 7 + k as u64 * 13 + (i as u64 >> 9)) as u8).collect();
                let mut done = 0;
                while done < len { done += v.write(&mut d, name.as_bytes(), (off + done) as u64, &data[done..]).unwrap(); }
                let f = m.get_mut(&name).unwrap().as_mut().unwrap();
                if f.len() < off + len { f.resize(off + len, 0); }
                f[off..off + len].copy_from_slice(&data);
            }
            3 => if let Some(Some(f)) = m.get_mut(&name) {
                let n = if rng.below(3) == 0 { f.len() + rng.below(50_000) as usize } else { rng.below(f.len() as u64 + 1) as usize };
                v.truncate(&mut d, name.as_bytes(), n as u64).unwrap();
                f.resize(n, 0);
            },
            4 => if !m.contains_key(&name) && parent_ok(&m, &name) && !name.contains('.') {
                v.mkdir(&mut d, name.as_bytes()).unwrap();
                m.insert(name.clone(), None);
            },
            5 => if m.contains_key(&name) {
                let to = names[rng.below(names.len() as u64) as usize].to_string();
                if to == name || m.contains_key(&to) || !parent_ok(&m, &to) || to.starts_with(&format!("{name}/")) { continue; }
                v.rename(&mut d, name.as_bytes(), to.as_bytes()).unwrap();
                let moved: Vec<String> = m.keys().filter(|k| **k == name || k.starts_with(&format!("{name}/"))).cloned().collect();
                for old in moved { let val = m.remove(&old).unwrap(); m.insert(format!("{to}{}", &old[name.len()..]), val); }
            },
            _ => if m.contains_key(&name) {
                let empty = !m.keys().any(|k| k.starts_with(&format!("{name}/")));
                let r = v.remove(&mut d, name.as_bytes());
                if empty { r.unwrap(); m.remove(&name); } else { assert_eq!(r.unwrap_err(), Error::NotEmpty); }
            },
        }
        if k % 50 == 49 { v.sync(&mut d).unwrap(); fsck(&d, &path, &format!("{tag} step {k}")); }
    }
    v.sync(&mut d).unwrap();
    fsck(&d, &path, &format!("{tag} end"));
    // Our own reader, a fresh mount, and Linux's debugfs all agree with the model.
    let mut v = sv::ext4::Ext4::mount(&mut d).unwrap();
    for (name, want) in &m {
        let st = v.stat(&mut d, name.as_bytes()).unwrap_or_else(|e| panic!("{tag}: {name}: {e:?}"));
        match want {
            None => assert_eq!(st.kind, Kind::Dir, "{tag}: {name}"),
            Some(data) => {
                assert_eq!(&read_all(&mut v, &mut d, name), data, "{tag}: {name} read back");
                assert!(debugfs_cat(&path, name) == *data, "{tag}: {name}: debugfs reads it differently");
            }
        }
    }
    let listed: usize = ["", "dir1", "dir2", "dir1/sub"].iter().filter(|p| p.is_empty() || matches!(m.get(**p), Some(None)))
        .map(|p| v.list(&mut d, p.as_bytes()).unwrap().len()).sum();
    assert!(listed <= m.len(), "{tag}: more entries listed than the model has");
    eprintln!("[volumes.ext4w] {tag}: {} entries, {} steps, e2fsck clean", m.len(), steps);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn ext4_default_4k() { workload("4k", 64, &[], 1, 400); }
#[test]
fn ext4_1k_blocks() { workload("1k", 32, &["-b", "1024"], 2, 400); }
#[test]
fn ext4_no_metadata_csum() { workload("nocsum", 32, &["-O", "^metadata_csum"], 3, 300); }
#[test]
fn ext4_64bit() { workload("64bit", 64, &["-O", "64bit"], 4, 300); }

#[test]
fn ext4_big_file_trees() {
    // Many separate extents: every other block, so the tree needs depth 1, then 2.
    let Some((mut d, path)) = fresh("tree", 64, &["-b", "1024"]) else { return };
    let mut v = sv::ext4::Ext4::mount(&mut d).unwrap();
    v.set_now(1_763_000_000);
    v.create(&mut d, b"sparse.bin").unwrap();
    let mut model = vec![];
    for i in 0..9000u64 {
        let at = i * 2048;
        let bytes = [(i & 0xFF) as u8; 7];
        v.write(&mut d, b"sparse.bin", at, &bytes).unwrap();
        model.resize(at as usize + 7, 0);
        model[at as usize..at as usize + 7].copy_from_slice(&bytes);
        if i == 100 || i == 2000 { v.sync(&mut d).unwrap(); fsck(&d, &path, &format!("tree at {i}")); }
    }
    v.sync(&mut d).unwrap();
    fsck(&d, &path, "tree, 9000 extents");
    assert!(debugfs_cat(&path, "sparse.bin") == model, "debugfs reads the sparse file differently");
    v.truncate(&mut d, b"sparse.bin", 5000).unwrap();
    v.sync(&mut d).unwrap();
    fsck(&d, &path, "tree truncated");
    v.remove(&mut d, b"sparse.bin").unwrap();
    v.sync(&mut d).unwrap();
    fsck(&d, &path, "tree removed");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn ext4_many_names_and_full_disk() {
    let Some((mut d, path)) = fresh("full", 8, &[]) else { return };
    let mut v = sv::ext4::Ext4::mount(&mut d).unwrap();
    v.set_now(1_763_000_000);
    v.mkdir(&mut d, b"many").unwrap();
    for i in 0..400 { v.create(&mut d, format!("many/file number {i:04} with a longer name").as_bytes()).unwrap(); }
    assert_eq!(v.list(&mut d, b"many").unwrap().len(), 400);
    for i in (0..400).step_by(3) { v.remove(&mut d, format!("many/file number {i:04} with a longer name").as_bytes()).unwrap(); }
    v.sync(&mut d).unwrap();
    fsck(&d, &path, "many names");
    // Fill it: the write that runs out says so, and the disk is still whole.
    v.create(&mut d, b"fill").unwrap();
    let chunk = vec![0x5Au8; 1 << 20];
    let mut at = 0u64;
    let err = loop {
        match v.write(&mut d, b"fill", at, &chunk) { Ok(n) => { at += n as u64; if n < chunk.len() { break v.write(&mut d, b"fill", at, &chunk).unwrap_err(); } } Err(e) => break e }
    };
    assert_eq!(err, Error::NoSpace);
    v.sync(&mut d).unwrap();
    fsck(&d, &path, "full disk");
    assert_eq!(v.stat(&mut d, b"fill").unwrap().size, at);
    v.remove(&mut d, b"fill").unwrap();
    v.sync(&mut d).unwrap();
    fsck(&d, &path, "full disk emptied");
    let _ = std::fs::remove_file(&path);
}
