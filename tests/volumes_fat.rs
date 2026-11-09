// SPDX-License-Identifier: GPL-3.0-only
//! FAT12/16/32 against images made by mkfs.vfat and mtools, and our writes checked by
//! fsck.vfat and read back by mtools when the host has them.
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
use common::*;
use sv::{Disk, Error, Format, Kind, Volume};

fn big() -> Vec<u8> { (0..70000u32).map(|i| ((i * 7 + i / 251) & 255) as u8).collect() }
fn read_all(v: &mut dyn Volume, d: &mut dyn Disk, path: &str) -> Vec<u8> {
    let size = v.stat(d, path.as_bytes()).unwrap().size as usize;
    let mut out = vec![0u8; size + 10];
    let n = v.read(d, path.as_bytes(), 0, &mut out).unwrap();
    out.truncate(n);
    out
}
fn names(v: &mut dyn Volume, d: &mut dyn Disk, path: &str) -> Vec<String> {
    let mut n: Vec<String> = v.list(d, path.as_bytes()).unwrap().into_iter().map(|e| if e.kind == Kind::Dir { e.name + "/" } else { e.name }).collect();
    n.sort();
    n
}

#[test]
fn reads_images_made_by_mtools() {
    for (name, format, label) in [("fat12", Format::Fat12, "FLOPPY"), ("fat16", Format::Fat16, "CAMERA"), ("fat32", Format::Fat32, "STICK")] {
        let mut d = fixture(name);
        let mut found = sv::scan(&mut d).unwrap();
        assert_eq!(found.len(), 1, "{name}");
        let v = found[0].volume.as_mut();
        assert_eq!(v.format(), format, "{name}");
        assert_eq!(v.label(), label);
        assert_eq!(names(v, &mut d, "/"), ["Docs/", "HELLO.TXT", "Long Folder Name/", "lower.txt"], "{name}");
        assert_eq!(read_all(v, &mut d, "HELLO.TXT"), format!("hello from {name}\n").as_bytes());
        assert_eq!(read_all(v, &mut d, "hello.txt"), format!("hello from {name}\n").as_bytes(), "case-insensitive");
        assert_eq!(read_all(v, &mut d, "/Docs/big.bin"), big());
        assert_eq!(read_all(v, &mut d, "Docs/Deep/a very long file name.text"), b"deep\n");
        assert_eq!(read_all(v, &mut d, "Long Folder Name/café ünïcode.txt"), b"uni\n");
        let mut part = [0u8; 1000];
        assert_eq!(v.read(&mut d, b"Docs/big.bin", 69500, &mut part).unwrap(), 500);
        assert_eq!(&part[..500], &big()[69500..]);
        assert_eq!(v.stat(&mut d, b"Docs").unwrap().kind, Kind::Dir);
        assert!(v.stat(&mut d, b"HELLO.TXT").unwrap().mtime > 1_700_000_000);
        assert_eq!(v.stat(&mut d, b"nope").unwrap_err(), Error::NotFound);
        assert_eq!(v.list(&mut d, b"HELLO.TXT").unwrap_err(), Error::NotDir);
        assert_eq!(v.read(&mut d, b"Docs", 0, &mut part).unwrap_err(), Error::IsDir);
        assert_eq!(v.stat(&mut d, b"Docs/../HELLO.TXT").unwrap_err(), Error::Name);
        let (total, free) = v.statfs(&mut d).unwrap();
        assert!(total > free && free > 0);
    }
}

/// A deterministic pseudo-random stream.
struct Rng(u64);
impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } fn below(&mut self, n: u64) -> u64 { self.next() % n } }

fn fresh(format: &str, mib: u64) -> MemDisk {
    match tool("mkfs.vfat") {
        Some(mkfs) => {
            let path = temp(&format!("fresh-{format}.img"));
            let _ = std::fs::remove_file(&path);
            std::fs::File::create(&path).unwrap().set_len(mib << 20).unwrap();
            let mut args = vec!["-n", "FRESH"];
            match format { "fat12" => args.extend(["-F", "12"]), "fat16" => args.extend(["-F", "16"]), _ => args.extend(["-F", "32", "-s", "1"]) }
            args.push(path.to_str().unwrap());
            assert!(run(&mkfs, &args).status.success());
            MemDisk::from_file(&path)
        }
        None => fixture(format),
    }
}
/// fsck.vfat must find nothing to fix, and mtools must read what we wrote.
fn check_with_host(d: &MemDisk, tag: &str, expect: &[(String, Vec<u8>)]) {
    let path = temp(&format!("check-{tag}.img"));
    std::fs::write(&path, &d.bytes).unwrap();
    if let Some(fsck) = tool("fsck.vfat") {
        let out = run(&fsck, &["-n", "-v", path.to_str().unwrap()]);
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "fsck.vfat {tag}: {text}");
        for bad in ["Free cluster summary wrong", "Orphan", "lost", "Contains a free cluster", "differ", "Bad short file name", "Duplicate"] {
            assert!(!text.contains(bad), "fsck.vfat {tag}: {text}");
        }
    }
    if let Some(mtype) = tool("mtype") {
        for (name, data) in expect {
            let out = std::process::Command::new(&mtype).env("MTOOLS_SKIP_CHECK", "1").env("LC_ALL", "C.UTF-8")
                .args(["-i", path.to_str().unwrap(), &format!("::/{name}")]).output().unwrap();
            assert!(out.status.success(), "mtype {name}: {}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(&out.stdout, data, "mtools read of {name} ({tag})");
        }
    }
}

#[test]
fn writes_that_fsck_and_mtools_accept() {
    for (format, mib) in [("fat12", 1), ("fat16", 20), ("fat32", 40)] {
        let mut d = fresh(format, mib);
        let mut v = sv::fat::Fat::mount(&mut d).unwrap();
        v.set_now(1_762_700_000);
        let (_, free0) = v.statfs(&mut d).unwrap();
        v.mkdir(&mut d, b"Photos").unwrap();
        v.mkdir(&mut d, b"Photos/2026 Summer Trip").unwrap();
        assert_eq!(v.mkdir(&mut d, b"photos").unwrap_err(), Error::Exists);
        v.create(&mut d, b"Photos/2026 Summer Trip/beach day.jpg").unwrap();
        let data: Vec<u8> = (0..150_000u32).map(|i| (i % 253) as u8).collect();
        v.write(&mut d, b"Photos/2026 Summer Trip/beach day.jpg", 0, &data[..100_000]).unwrap();
        v.write(&mut d, b"Photos/2026 Summer Trip/beach day.jpg", 100_000, &data[100_000..]).unwrap();
        v.create(&mut d, b"notes.txt").unwrap();
        v.write(&mut d, b"notes.txt", 0, b"first").unwrap();
        // A write past the end leaves zeros between.
        v.write(&mut d, b"notes.txt", 10, b"later").unwrap();
        assert_eq!(read_all(&mut v, &mut d, "notes.txt"), b"first\0\0\0\0\0later");
        v.truncate(&mut d, b"notes.txt", 5).unwrap();
        v.create(&mut d, b"README").unwrap();
        v.write(&mut d, b"README", 0, b"short name\n").unwrap();
        // Rename in place, across folders, and a folder across folders.
        v.rename(&mut d, b"notes.txt", b"Notes For Later.txt").unwrap();
        v.mkdir(&mut d, b"Archive").unwrap();
        v.rename(&mut d, b"Photos/2026 Summer Trip", b"Archive/Trip").unwrap();
        assert_eq!(v.rename(&mut d, b"Archive", b"Archive/Trip/inside").unwrap_err(), Error::Name);
        assert_eq!(v.rename(&mut d, b"README", b"Archive").unwrap_err(), Error::Exists);
        v.rename(&mut d, b"README", b"readme").unwrap();
        assert_eq!(v.remove(&mut d, b"Archive").unwrap_err(), Error::NotEmpty);
        // Enough entries to grow a directory past one cluster (FAT12/16's root is fixed).
        v.mkdir(&mut d, b"Many").unwrap();
        for i in 0..120 {
            let name = format!("Many/file number {i:03} with a long name.txt");
            v.create(&mut d, name.as_bytes()).unwrap();
            v.write(&mut d, name.as_bytes(), 0, name.as_bytes()).unwrap();
        }
        for i in (0..120).step_by(3) { v.remove(&mut d, format!("Many/file number {i:03} with a long name.txt").as_bytes()).unwrap(); }
        v.sync(&mut d).unwrap();
        // Everything again from a fresh mount.
        let mut v = sv::fat::Fat::mount(&mut d).unwrap();
        assert_eq!(names(&mut v, &mut d, "/"), ["Archive/", "Many/", "Notes For Later.txt", "Photos/", "readme"]);
        assert_eq!(read_all(&mut v, &mut d, "Archive/Trip/beach day.jpg"), data);
        assert_eq!(read_all(&mut v, &mut d, "notes for later.TXT"), b"first");
        assert_eq!(read_all(&mut v, &mut d, "readme"), b"short name\n");
        assert_eq!(v.list(&mut d, b"Many").unwrap().len(), 80);
        assert!(names(&mut v, &mut d, "Photos").is_empty());
        let mut expect = vec![("Archive/Trip/beach day.jpg".to_string(), data.clone()), ("Notes For Later.txt".to_string(), b"first".to_vec())];
        expect.push(("Many/file number 119 with a long name.txt".into(), b"Many/file number 119 with a long name.txt".to_vec()));
        check_with_host(&d, format, &expect);
        // Removing it all gives every cluster back.
        v.remove(&mut d, b"Archive/Trip/beach day.jpg").unwrap();
        v.remove(&mut d, b"Archive/Trip").unwrap();
        v.remove(&mut d, b"Archive").unwrap();
        for i in 0..120 { let _ = v.remove(&mut d, format!("Many/file number {i:03} with a long name.txt").as_bytes()); }
        v.remove(&mut d, b"Many").unwrap();
        v.remove(&mut d, b"Photos").unwrap();
        v.remove(&mut d, b"Notes For Later.txt").unwrap();
        v.remove(&mut d, b"readme").unwrap();
        v.sync(&mut d).unwrap();
        let mut v = sv::fat::Fat::mount(&mut d).unwrap();
        assert_eq!(v.statfs(&mut d).unwrap().1, free0, "{format}: clusters leaked");
        check_with_host(&d, &format!("{format}-empty"), &[]);
    }
}

#[test]
fn fills_up_cleanly() {
    let mut d = fresh("fat12", 1);
    let mut v = sv::fat::Fat::mount(&mut d).unwrap();
    v.create(&mut d, b"fill").unwrap();
    let chunk = vec![0xAB; 64 * 1024];
    let mut written = 0u64;
    loop {
        match v.write(&mut d, b"fill", written, &chunk) {
            Ok(n) => written += n as u64,
            Err(e) => { assert_eq!(e, Error::NoSpace); break; }
        }
    }
    assert!(written > 900_000, "{written}");
    assert_eq!(v.stat(&mut d, b"fill").unwrap().size, written);
    v.sync(&mut d).unwrap();
    check_with_host(&d, "full", &[]);
    v.remove(&mut d, b"fill").unwrap();
    v.create(&mut d, b"after").unwrap();
    v.write(&mut d, b"after", 0, b"room again").unwrap();
}

/// Random operations against a model, remounting now and then.
#[test]
fn random_model() {
    use std::collections::BTreeMap;
    for seed in 1..=4u64 {
        let mut d = fresh("fat16", 20);
        let mut v = sv::fat::Fat::mount(&mut d).unwrap();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut model: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let dirs = ["", "a/", "a/b/", "Some Folder/"];
        for dir in &dirs[1..] { v.mkdir(&mut d, dir.trim_end_matches('/').as_bytes()).unwrap(); }
        for step in 0..400 {
            let dir = dirs[rng.below(4) as usize];
            let name = format!("{dir}{}", ["x", "Y.TXT", "long file name number one", "data.bin", "ünï.txt"][rng.below(5) as usize]);
            match rng.below(6) {
                0 => { let r = v.create(&mut d, name.as_bytes()); if model.contains_key(&name) { assert_eq!(r, Err(Error::Exists)); } else { r.unwrap(); model.insert(name, vec![]); } }
                1 | 2 => if let Some(m) = model.get_mut(&name) {
                    let off = rng.below(m.len() as u64 + 3000);
                    let len = rng.below(9000) as usize;
                    let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                    v.write(&mut d, name.as_bytes(), off, &data).unwrap();
                    let end = off as usize + len;
                    if m.len() < end { m.resize(end, 0); }
                    m[off as usize..end].copy_from_slice(&data);
                },
                3 => if let Some(m) = model.get_mut(&name) { let n = rng.below(m.len() as u64 + 5000) as usize; v.truncate(&mut d, name.as_bytes(), n as u64).unwrap(); m.resize(n, 0); },
                4 => { let r = v.remove(&mut d, name.as_bytes()); if model.remove(&name).is_some() { r.unwrap(); } else { assert_eq!(r, Err(Error::NotFound)); } }
                _ => {
                    let to = format!("{}{}", dirs[rng.below(4) as usize], ["moved", "Renamed File.dat", "z"][rng.below(3) as usize]);
                    if model.contains_key(&name) && !model.contains_key(&to) { v.rename(&mut d, name.as_bytes(), to.as_bytes()).unwrap(); let m = model.remove(&name).unwrap(); model.insert(to, m); }
                }
            }
            if step % 97 == 0 { v.sync(&mut d).unwrap(); v = sv::fat::Fat::mount(&mut d).unwrap(); }
        }
        v.sync(&mut d).unwrap();
        let mut v = sv::fat::Fat::mount(&mut d).unwrap();
        for (name, data) in &model { assert_eq!(&read_all(&mut v, &mut d, name), data, "seed {seed} {name}"); }
        let listed: usize = dirs.iter().map(|dir| v.list(&mut d, dir.as_bytes()).unwrap().into_iter().filter(|e| e.kind == Kind::File).count()).sum();
        assert_eq!(listed, model.len());
        let expect: Vec<(String, Vec<u8>)> = model.into_iter().collect();
        check_with_host(&d, &format!("model{seed}"), &expect);
    }
}

#[test]
fn found_inside_partitions() {
    // The FAT32 fixture behind an MBR at sector 2048, and behind a GPT.
    let inner = fixture("fat32");
    for gpt in [false, true] {
        let mut d = MemDisk::new(2048 + inner.sectors() + 64);
        d.bytes[2048 * 512..2048 * 512 + inner.bytes.len()].copy_from_slice(&inner.bytes);
        let len = inner.sectors();
        let mbr = &mut d.bytes[..512];
        mbr[510] = 0x55; mbr[511] = 0xAA;
        if gpt {
            mbr[446 + 4] = 0xEE; mbr[446 + 8..446 + 12].copy_from_slice(&1u32.to_le_bytes());
            let h = &mut d.bytes[512..1024];
            h[..8].copy_from_slice(b"EFI PART");
            h[72..80].copy_from_slice(&2u64.to_le_bytes());
            h[80..84].copy_from_slice(&128u32.to_le_bytes());
            h[84..88].copy_from_slice(&128u32.to_le_bytes());
            let e = &mut d.bytes[1024 + 128..1024 + 256]; // second entry
            e[..16].copy_from_slice(&[0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7]);
            e[16] = 1;
            e[32..40].copy_from_slice(&2048u64.to_le_bytes());
            e[40..48].copy_from_slice(&(2048 + len - 1).to_le_bytes());
        } else {
            mbr[446 + 4] = 0x0C;
            mbr[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
            mbr[446 + 12..446 + 16].copy_from_slice(&(len as u32).to_le_bytes());
        }
        let mut found = sv::scan(&mut d).unwrap();
        assert_eq!(found.len(), 1, "gpt={gpt}");
        assert_eq!(found[0].start, 2048);
        let start = found[0].start;
        let mut slice = sv::Slice { disk: &mut d, start, len };
        assert_eq!(read_all(found[0].volume.as_mut(), &mut slice, "Docs/big.bin"), big());
    }
}
