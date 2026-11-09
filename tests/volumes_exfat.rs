// SPDX-License-Identifier: GPL-3.0-only
//! exFAT: reads of a stick written by Linux's exfat-fuse (tests/volumes/exfat.sparse),
//! writes to a fresh `mkfs.exfat` image checked by `fsck.exfat` where the host has it.
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
use common::MemDisk;
use std::sync::atomic::{AtomicUsize, Ordering};
use sv::exfat::ExFat;
use sv::{Error, Format, Kind, Volume};

const UNICODE: &str = "Ünïcödé — a rather long file name with 日本語 characters.txt";

fn mount(d: &mut MemDisk) -> ExFat { ExFat::mount(d).unwrap() }
fn read_all(v: &mut ExFat, d: &mut MemDisk, path: &str) -> Vec<u8> {
    let size = v.stat(d, path.as_bytes()).unwrap().size as usize;
    let mut buf = vec![0xEE; size + 7];
    assert_eq!(v.read(d, path.as_bytes(), 0, &mut buf).unwrap(), size);
    buf.truncate(size);
    buf
}
fn names(v: &mut ExFat, d: &mut MemDisk, path: &str) -> Vec<String> {
    let mut n: Vec<String> = v.list(d, path.as_bytes()).unwrap().into_iter().map(|e| e.name).collect();
    n.sort();
    n
}
fn pattern(len: usize, seed: u8) -> Vec<u8> { (0..len).map(|i| (i % 253) as u8 ^ seed).collect() }

/// A fresh volume made by the host's own mkfs, or the committed blank one.
fn blank() -> MemDisk {
    static N: AtomicUsize = AtomicUsize::new(0);
    let Some(mkfs) = common::tool("mkfs.exfat") else { return common::fixture("exfat-blank") };
    let path = common::temp(&format!("exfat-blank-{}.img", N.fetch_add(1, Ordering::SeqCst)));
    std::fs::write(&path, vec![0u8; 8 << 20]).unwrap();
    let out = common::run(&mkfs, &["-q", "-L", "BLANK", path.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let d = MemDisk::from_file(&path);
    std::fs::remove_file(&path).ok();
    d
}
/// The host's fsck.exfat must find nothing to fix.
fn fsck(d: &MemDisk) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let Some(fsck) = common::tool("fsck.exfat") else { return };
    let path = common::temp(&format!("exfat-fsck-{}.img", N.fetch_add(1, Ordering::SeqCst)));
    std::fs::write(&path, &d.bytes).unwrap();
    let out = common::run(&fsck, &["-n", path.to_str().unwrap()]);
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    std::fs::remove_file(&path).ok();
    assert!(out.status.success() && text.contains("clean"), "fsck.exfat:\n{text}");
}

#[test]
fn exfat_fixture_label_and_listing() {
    let mut d = common::fixture("exfat");
    let mut v = mount(&mut d);
    assert_eq!((v.format(), v.label().as_str(), v.read_only()), (Format::ExFat, "STICK", false));
    assert_eq!(names(&mut v, &mut d, "/"), ["big.bin", "docs", "frag.bin", "hello.txt", "many", UNICODE]);
    assert_eq!(names(&mut v, &mut d, "docs"), ["nested", "readme.md"]);
    assert_eq!(names(&mut v, &mut d, "/docs/nested/"), ["deep.txt"]);
    // 60 entry sets of three entries: the directory spans two (chained) clusters.
    assert_eq!(v.list(&mut d, b"many").unwrap().len(), 60);
    let (total, free) = v.statfs(&mut d).unwrap();
    assert_eq!(total, 1536 * 4096);
    assert!(free < total && free > total - 64 * 4096, "{free}");
}

#[test]
fn exfat_fixture_contents() {
    let mut d = common::fixture("exfat");
    let mut v = mount(&mut d);
    assert_eq!(read_all(&mut v, &mut d, "hello.txt"), b"Hello from exFAT\n");
    assert_eq!(read_all(&mut v, &mut d, "docs/readme.md"), b"# Readme\nexFAT fixture\n");
    assert_eq!(read_all(&mut v, &mut d, "docs/nested/deep.txt"), b"deep\n");
    assert_eq!(read_all(&mut v, &mut d, UNICODE), b"unicode\n");
    // Five contiguous clusters (NoFatChain), and three in a fragmented FAT chain.
    let big: Vec<u8> = (0..20000).map(|i| (i % 251) as u8).collect();
    assert_eq!(read_all(&mut v, &mut d, "big.bin"), big);
    let frag: Vec<u8> = (0..10000).map(|i| (i * 7 % 256) as u8).collect();
    assert_eq!(read_all(&mut v, &mut d, "frag.bin"), frag);
    let mut buf = [0u8; 100];
    assert_eq!(v.read(&mut d, b"frag.bin", 4050, &mut buf).unwrap(), 100);
    assert_eq!(&buf[..], &frag[4050..4150]);
    assert_eq!(v.read(&mut d, b"frag.bin", 9950, &mut buf).unwrap(), 50);
    assert_eq!(v.read(&mut d, b"frag.bin", 10000, &mut buf).unwrap(), 0);
    let st = v.stat(&mut d, b"hello.txt").unwrap();
    assert_eq!((st.kind, st.size, st.mtime), (Kind::File, 17, 1710506096)); // 2024-03-15 12:34:56 UTC
    assert_eq!(v.stat(&mut d, b"docs").unwrap().kind, Kind::Dir);
}

#[test]
fn exfat_fixture_lookup_is_case_insensitive() {
    let mut d = common::fixture("exfat");
    let mut v = mount(&mut d);
    assert_eq!(read_all(&mut v, &mut d, "HELLO.TXT"), b"Hello from exFAT\n");
    assert_eq!(read_all(&mut v, &mut d, "Docs/NESTED/Deep.Txt"), b"deep\n");
    // Ü/ü and the other accented letters go through the volume's up-case table.
    assert_eq!(read_all(&mut v, &mut d, &UNICODE.to_uppercase()), b"unicode\n");
    assert_eq!(v.stat(&mut d, b"nope").unwrap_err(), Error::NotFound);
    assert_eq!(v.stat(&mut d, b"hello.txt/x").unwrap_err(), Error::NotDir);
    assert_eq!(v.read(&mut d, b"docs", 0, &mut [0; 4]).unwrap_err(), Error::IsDir);
    assert_eq!(v.stat(&mut d, b"a/../b").unwrap_err(), Error::Name);
}

#[test]
fn exfat_scan_finds_bare_and_partitioned() {
    let stick = common::fixture("exfat");
    let mut d = MemDisk { bytes: stick.bytes.clone(), flushes: 0 };
    let found = sv::scan(&mut d).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].start, found[0].volume.format(), found[0].volume.label()), (0, Format::ExFat, "STICK".into()));
    // The same volume as the first partition of an MBR disk.
    let sectors = (stick.bytes.len() / 512) as u32;
    let mut d = MemDisk::new(2048 + sectors as u64);
    d.bytes[2048 * 512..].copy_from_slice(&stick.bytes);
    let e = &mut d.bytes[446..462];
    e[4] = 0x07;
    e[8..12].copy_from_slice(&2048u32.to_le_bytes());
    e[12..16].copy_from_slice(&sectors.to_le_bytes());
    d.bytes[510] = 0x55;
    d.bytes[511] = 0xAA;
    let mut found = sv::scan(&mut d).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].start, found[0].len), (2048, sectors as u64));
    let mut slice = sv::Slice { disk: &mut d, start: 2048, len: sectors as u64 };
    let mut buf = [0u8; 17];
    found[0].volume.read(&mut slice, b"hello.txt", 0, &mut buf).unwrap();
    assert_eq!(&buf, b"Hello from exFAT\n");
}

#[test]
fn exfat_refuses_corrupt_boot_sector() {
    let mut d = common::fixture("exfat");
    d.bytes[108] = 20; // 1 MiB sectors
    assert_eq!(ExFat::mount(&mut d).err(), Some(Error::Corrupt));
    let mut d = common::fixture("exfat");
    d.bytes[92..96].copy_from_slice(&u32::MAX.to_le_bytes()); // more clusters than the volume
    assert_eq!(ExFat::mount(&mut d).err(), Some(Error::Corrupt));
}

#[test]
fn exfat_create_write_read_back_and_remount() {
    let mut d = blank();
    let mut v = mount(&mut d);
    assert_eq!(v.label(), "BLANK");
    let (_, free0) = v.statfs(&mut d).unwrap();
    v.create(&mut d, b"note.txt").unwrap();
    assert_eq!(v.stat(&mut d, b"note.txt").unwrap().size, 0);
    assert_eq!(v.write(&mut d, b"note.txt", 0, b"hello exfat").unwrap(), 11);
    v.write(&mut d, b"note.txt", 6, b"EXFAT").unwrap();
    assert_eq!(read_all(&mut v, &mut d, "note.txt"), b"hello EXFAT");
    // Big: 300 KiB over 4 KiB clusters, written in odd-sized pieces.
    let big = pattern(300 << 10, 0x5A);
    v.create(&mut d, b"big.bin").unwrap();
    for (i, chunk) in big.chunks(7777).enumerate() { v.write(&mut d, b"big.bin", (i * 7777) as u64, chunk).unwrap(); }
    assert_eq!(read_all(&mut v, &mut d, "big.bin"), big);
    // Append past the end leaves zeros in the gap.
    v.write(&mut d, b"note.txt", 20, b"tail").unwrap();
    let mut want = b"hello EXFAT".to_vec();
    want.resize(20, 0);
    want.extend_from_slice(b"tail");
    assert_eq!(read_all(&mut v, &mut d, "note.txt"), want);
    let mut uni = UNICODE.to_string();
    uni.push_str(&"x".repeat(255 - uni.encode_utf16().count()));
    v.create(&mut d, uni.as_bytes()).unwrap();
    v.write(&mut d, uni.as_bytes(), 0, b"long").unwrap();
    v.sync(&mut d).unwrap();
    assert!(d.flushes > 0);
    let (_, free1) = v.statfs(&mut d).unwrap();
    assert_eq!(free0 - free1, (75 + 1 + 1) * 4096);

    let mut v = mount(&mut d);
    assert_eq!(names(&mut v, &mut d, ""), ["big.bin", "note.txt", uni.as_str()]);
    assert_eq!(read_all(&mut v, &mut d, "big.bin"), big);
    assert_eq!(read_all(&mut v, &mut d, "note.txt"), want);
    assert_eq!(read_all(&mut v, &mut d, &uni), b"long");
    assert_eq!(v.statfs(&mut d).unwrap().1, free1);
    let st = v.stat(&mut d, b"note.txt").unwrap();
    assert_eq!(st.mtime, 315532800); // the fixed 1980-01-01 stamp
    fsck(&d);
}

#[test]
fn exfat_truncate_and_contiguity() {
    let mut d = blank();
    let mut v = mount(&mut d);
    let (_, free0) = v.statfs(&mut d).unwrap();
    // a gets a contiguous run, b sits right after it, so a's growth must become a chain.
    v.create(&mut d, b"a").unwrap();
    v.create(&mut d, b"b").unwrap();
    let a = pattern(10000, 1);
    v.write(&mut d, b"a", 0, &a).unwrap();
    v.write(&mut d, b"b", 0, &pattern(5000, 2)).unwrap();
    let mut a2 = a.clone();
    a2.extend(pattern(20000, 3));
    v.write(&mut d, b"a", 10000, &a2[10000..]).unwrap();
    assert_eq!(read_all(&mut v, &mut d, "a"), a2);
    assert_eq!(read_all(&mut v, &mut d, "b"), pattern(5000, 2));
    // Shrink (frees clusters), then grow again: the new bytes read as zeros.
    v.truncate(&mut d, b"a", 5000).unwrap();
    assert_eq!(read_all(&mut v, &mut d, "a"), a[..5000]);
    v.truncate(&mut d, b"a", 50000).unwrap();
    let mut grown = a[..5000].to_vec();
    grown.resize(50000, 0);
    assert_eq!(read_all(&mut v, &mut d, "a"), grown);
    v.write(&mut d, b"a", 30000, b"mid").unwrap();
    grown[30000..30003].copy_from_slice(b"mid");
    assert_eq!(read_all(&mut v, &mut d, "a"), grown);
    fsck(&d);
    let mut v = mount(&mut d);
    assert_eq!(read_all(&mut v, &mut d, "a"), grown);
    v.truncate(&mut d, b"a", 0).unwrap();
    v.truncate(&mut d, b"b", 1).unwrap();
    assert_eq!(read_all(&mut v, &mut d, "b"), [2]);
    assert_eq!(v.statfs(&mut d).unwrap().1, free0 - 4096);
    assert_eq!(v.truncate(&mut d, b"/", 0).unwrap_err(), Error::IsDir);
    fsck(&d);
}

#[test]
fn exfat_dirs_rename_remove() {
    let mut d = blank();
    let mut v = mount(&mut d);
    let (_, free0) = v.statfs(&mut d).unwrap();
    v.mkdir(&mut d, b"photos").unwrap();
    v.mkdir(&mut d, b"photos/2024").unwrap();
    v.mkdir(&mut d, b"photos/2024/summer").unwrap();
    v.create(&mut d, b"photos/2024/summer/beach.jpg").unwrap();
    v.write(&mut d, b"photos/2024/summer/beach.jpg", 0, &pattern(9000, 9)).unwrap();
    assert_eq!(v.stat(&mut d, b"photos/2024").unwrap().kind, Kind::Dir);
    assert_eq!(v.mkdir(&mut d, b"PHOTOS").unwrap_err(), Error::Exists);
    assert_eq!(v.create(&mut d, b"photos/2024/summer/BEACH.JPG").unwrap_err(), Error::Exists);
    assert_eq!(v.create(&mut d, b"photos/2024/summer/beach.jpg/x").unwrap_err(), Error::NotDir);
    assert_eq!(v.create(&mut d, b"missing/x").unwrap_err(), Error::NotFound);
    for bad in ["a:b", "a*", "q?", "<x>", "pipe|", "back\\slash", "quo\"te", "tab\t"] {
        assert_eq!(v.create(&mut d, bad.as_bytes()).unwrap_err(), Error::Name, "{bad}");
    }
    assert_eq!(v.create(&mut d, "n".repeat(256).as_bytes()).unwrap_err(), Error::Name);
    assert_eq!(v.write(&mut d, b"photos", 0, b"x").unwrap_err(), Error::IsDir);
    assert_eq!(v.remove(&mut d, b"photos/2024").unwrap_err(), Error::NotEmpty);
    // Within a directory, a change of case only, and across directories.
    v.rename(&mut d, b"photos/2024/summer/beach.jpg", b"photos/2024/summer/sea.jpg").unwrap();
    v.rename(&mut d, b"photos/2024/summer/sea.jpg", b"photos/2024/summer/Sea.JPG").unwrap();
    assert_eq!(names(&mut v, &mut d, "photos/2024/summer"), ["Sea.JPG"]);
    v.rename(&mut d, b"photos/2024/summer/sea.jpg", b"top.jpg").unwrap();
    assert_eq!(read_all(&mut v, &mut d, "TOP.jpg"), pattern(9000, 9));
    v.create(&mut d, b"other").unwrap();
    assert_eq!(v.rename(&mut d, b"top.jpg", b"OTHER").unwrap_err(), Error::Exists);
    assert_eq!(v.rename(&mut d, b"photos", b"photos/2024/inside").unwrap_err(), Error::Name);
    v.rename(&mut d, b"photos/2024/summer", b"summer").unwrap();
    v.rename(&mut d, b"top.jpg", b"summer/top.jpg").unwrap();
    assert_eq!(names(&mut v, &mut d, "photos/2024"), Vec::<String>::new());
    assert_eq!(names(&mut v, &mut d, "summer"), ["top.jpg"]);
    fsck(&d);

    let mut v = mount(&mut d);
    assert_eq!(names(&mut v, &mut d, "/"), ["other", "photos", "summer"]);
    assert_eq!(read_all(&mut v, &mut d, "summer/top.jpg"), pattern(9000, 9));
    v.remove(&mut d, b"photos/2024").unwrap();
    v.remove(&mut d, b"photos").unwrap();
    v.remove(&mut d, b"summer/top.jpg").unwrap();
    v.remove(&mut d, b"summer").unwrap();
    v.remove(&mut d, b"other").unwrap();
    assert_eq!(v.remove(&mut d, b"other").unwrap_err(), Error::NotFound);
    assert!(v.list(&mut d, b"/").unwrap().is_empty());
    assert_eq!(v.statfs(&mut d).unwrap().1, free0);
    fsck(&d);
}

#[test]
fn exfat_many_files_grow_directories() {
    let mut d = blank();
    let mut v = mount(&mut d);
    v.mkdir(&mut d, b"sub").unwrap();
    // 150 sets of 3-4 entries: several clusters of entries in the root and in `sub`.
    for i in 0..150 {
        for dir in ["", "sub/"] {
            let path = format!("{dir}a fairly long file name number {i:03}.txt");
            v.create(&mut d, path.as_bytes()).unwrap();
            v.write(&mut d, path.as_bytes(), 0, format!("file {i}").as_bytes()).unwrap();
        }
    }
    v.remove(&mut d, b"a fairly long file name number 007.txt").unwrap();
    v.create(&mut d, b"reuses a hole").unwrap();
    fsck(&d);
    let mut v = mount(&mut d);
    assert_eq!(v.list(&mut d, b"").unwrap().len(), 151);
    assert_eq!(v.list(&mut d, b"sub").unwrap().len(), 150);
    assert!(v.stat(&mut d, b"sub").unwrap().size > 4096);
    for i in [0, 77, 149] {
        let path = format!("SUB/A FAIRLY LONG FILE NAME NUMBER {i:03}.TXT");
        assert_eq!(read_all(&mut v, &mut d, &path), format!("file {i}").as_bytes());
    }
}

#[test]
fn exfat_no_space_when_full() {
    let mut d = blank();
    let mut v = mount(&mut d);
    let (_, free0) = v.statfs(&mut d).unwrap();
    v.create(&mut d, b"fill").unwrap();
    let chunk = vec![0xAB; 64 << 10];
    let mut at = 0u64;
    let err = loop {
        match v.write(&mut d, b"fill", at, &chunk) { Ok(n) => at += n as u64, Err(e) => break e }
    };
    assert_eq!(err, Error::NoSpace);
    assert_eq!(v.stat(&mut d, b"fill").unwrap().size, at);
    let (_, free) = v.statfs(&mut d).unwrap();
    assert!(free < 64 << 10);
    // Fill to the last cluster, then nothing more fits, not even a directory.
    v.write(&mut d, b"fill", at, &vec![0xCD; free as usize]).unwrap();
    assert_eq!(v.statfs(&mut d).unwrap().1, 0);
    assert_eq!(v.write(&mut d, b"fill", at + free, b"x").unwrap_err(), Error::NoSpace);
    assert_eq!(v.mkdir(&mut d, b"d").unwrap_err(), Error::NoSpace);
    fsck(&d);
    v.remove(&mut d, b"fill").unwrap();
    assert_eq!(v.statfs(&mut d).unwrap().1, free0);
    fsck(&d);
}

#[test]
fn exfat_writes_to_a_linux_stick() {
    let mut d = common::fixture("exfat");
    let mut v = mount(&mut d);
    // frag.bin is a FAT chain, big.bin a NoFatChain run; grow both and move things about.
    let mut frag: Vec<u8> = (0..10000).map(|i| (i * 7 % 256) as u8).collect();
    v.write(&mut d, b"frag.bin", 10000, &pattern(9000, 4)).unwrap();
    frag.extend(pattern(9000, 4));
    let mut big: Vec<u8> = (0..20000).map(|i| (i % 251) as u8).collect();
    v.write(&mut d, b"big.bin", 20000, &pattern(30000, 5)).unwrap();
    big.extend(pattern(30000, 5));
    v.rename(&mut d, UNICODE.as_bytes(), "docs/日本語.txt".as_bytes()).unwrap();
    v.remove(&mut d, b"many/file1").unwrap();
    v.remove(&mut d, b"hello.txt").unwrap();
    let mut v = mount(&mut d);
    assert_eq!(read_all(&mut v, &mut d, "frag.bin"), frag);
    assert_eq!(read_all(&mut v, &mut d, "big.bin"), big);
    assert_eq!(read_all(&mut v, &mut d, "docs/日本語.txt"), b"unicode\n");
    assert_eq!(v.list(&mut d, b"many").unwrap().len(), 59);
    fsck(&d);
}
