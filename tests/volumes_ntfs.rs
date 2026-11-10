// SPDX-License-Identifier: GPL-3.0-only
//! NTFS read only, against volumes made by mkntfs and filled through ntfs-3g:
//! `ntfs.sparse` (2 MiB, 4 KiB clusters) and `ntfs512.sparse` (2 MiB, 512-byte clusters).
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
use sv::{Error, Format, Kind, Volume};

/// The content generator the fixture was filled with.
fn gen(seed: u32, n: usize) -> Vec<u8> { (0..n).map(|i| (((i as u32).wrapping_mul(2654435761) ^ seed) >> 13) as u8).collect() }

fn mount(name: &str) -> (common::MemDisk, sv::ntfs::Ntfs) {
    let mut d = common::fixture(name);
    let v = sv::ntfs::Ntfs::mount(&mut d).unwrap();
    (d, v)
}
fn whole(v: &mut dyn Volume, d: &mut dyn sv::Disk, path: &str) -> Vec<u8> {
    let size = v.stat(d, path.as_bytes()).unwrap().size as usize;
    let mut buf = vec![0xAA; size + 7];
    assert_eq!(v.read(d, path.as_bytes(), 0, &mut buf).unwrap(), size);
    buf.truncate(size);
    buf
}
fn names(v: &mut dyn Volume, d: &mut dyn sv::Disk, path: &str) -> Vec<(String, Kind, u64)> {
    let mut l: Vec<_> = v.list(d, path.as_bytes()).unwrap().into_iter().map(|e| (e.name, e.kind, e.size)).collect();
    l.sort_by(|a, b| a.0.cmp(&b.0));
    l
}

#[test]
fn label_format_and_space() {
    let (mut d, mut v) = mount("ntfs");
    assert_eq!(v.format(), Format::Ntfs);
    assert_eq!(v.label(), "WINDATA");
    assert!(v.read_only());
    let (total, free) = v.statfs(&mut d).unwrap();
    assert!(total > 1_900_000 && total <= 2 << 20, "{total}");
    assert!(free > 0 && free < total, "{free}");
}

#[test]
fn root_listing_hides_metafiles_and_dos_names() {
    let (mut d, mut v) = mount("ntfs");
    let f = |n: &str, s| (n.to_string(), Kind::File, s);
    let dir = |n: &str| (n.to_string(), Kind::Dir, 0);
    let mut want = vec![
        dir("Documents"), f("Grüße-日本.txt", 8), dir("Links"), f("Long name file.txt", 5), dir("Many"),
        f("MixedCase.TXT", 5), dir("Packed"), f("empty.txt", 0), f("hello.txt", 21), f("link", 26), f("sparse.bin", 1 << 20),
    ];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(names(&mut v, &mut d, "/"), want);
    assert_eq!(names(&mut v, &mut d, "Documents"), vec![("Reports".into(), Kind::Dir, 0), f("big.bin", 200 * 1024 + 123)]);
    assert_eq!(names(&mut v, &mut d, "Documents/Reports/2026"), vec![f("q3.txt", 32)]);
}

#[test]
fn contents() {
    let (mut d, mut v) = mount("ntfs");
    assert_eq!(whole(&mut v, &mut d, "hello.txt"), b"Hello from Windows!\r\n");
    assert_eq!(whole(&mut v, &mut d, "empty.txt"), b"");
    assert_eq!(whole(&mut v, &mut d, "Documents/Reports/2026/q3.txt"), b"Q3 revenue: up and to the right\n");
    assert_eq!(whole(&mut v, &mut d, "Grüße-日本.txt"), b"unicode\n");
    assert_eq!(whole(&mut v, &mut d, "Documents/big.bin"), gen(1, 200 * 1024 + 123));
    let mut sparse = vec![0u8; 1 << 20];
    sparse[..4096].copy_from_slice(&gen(2, 4096));
    sparse[768 * 1024..772 * 1024].copy_from_slice(&gen(3, 4096));
    assert_eq!(whole(&mut v, &mut d, "sparse.bin"), sparse);
    assert_eq!(v.stat(&mut d, b"hello.txt").unwrap().mtime, 1767323045);
    assert_eq!(v.stat(&mut d, b"Documents").unwrap().kind, Kind::Dir);
}

#[test]
fn odd_offsets_and_lengths() {
    let (mut d, mut v) = mount("ntfs");
    let big = gen(1, 200 * 1024 + 123);
    for (off, len) in [(0usize, 1usize), (1, 511), (511, 2), (4095, 4097), (12345, 54321), (big.len() - 3, 10), (big.len(), 4)] {
        let mut buf = vec![0; len];
        let n = v.read(&mut d, b"Documents/big.bin", off as u64, &mut buf).unwrap();
        let end = (off + len).min(big.len());
        assert_eq!(&buf[..n], &big[off..end], "{off}+{len}");
    }
    let mut buf = [0u8; 9];
    assert_eq!(v.read(&mut d, b"hello.txt", 6, &mut buf).unwrap(), 9);
    assert_eq!(&buf, b"from Wind");
    // Across the hole/data edge of the sparse file.
    let mut buf = vec![0xFFu8; 64];
    assert_eq!(v.read(&mut d, b"sparse.bin", 4096 - 32, &mut buf).unwrap(), 64);
    assert_eq!(&buf[..32], &gen(2, 4096)[4064..]);
    assert!(buf[32..].iter().all(|b| *b == 0));
}

#[test]
fn big_directory_uses_index_blocks() {
    let (mut d, mut v) = mount("ntfs");
    let l = names(&mut v, &mut d, "Many");
    assert_eq!(l.len(), 100);
    for (i, (name, kind, size)) in l.iter().enumerate() {
        assert_eq!((name.as_str(), *kind, *size), (format!("file{i:03}.txt").as_str(), Kind::File, format!("entry {i}\n").len() as u64));
    }
    for i in [0, 37, 63, 99] {
        assert_eq!(whole(&mut v, &mut d, &format!("Many/file{i:03}.txt")), format!("entry {i}\n").as_bytes());
    }
    assert_eq!(v.stat(&mut d, b"Many/file100.txt"), Err(Error::NotFound));
}

#[test]
fn hard_links_spill_into_an_attribute_list() {
    let (mut d, mut v) = mount("ntfs");
    let l = names(&mut v, &mut d, "Links");
    assert_eq!(l.len(), 17);
    assert!(l.iter().all(|(_, k, s)| *k == Kind::File && *s == 10000));
    let link = format!("Links/hard link number 07 {}", "x".repeat(90));
    assert_eq!(whole(&mut v, &mut d, &link), gen(4, 10000));
}

#[test]
fn case_insensitive_lookup() {
    let (mut d, mut v) = mount("ntfs");
    assert_eq!(whole(&mut v, &mut d, "mixedcase.txt"), b"case\n");
    assert_eq!(whole(&mut v, &mut d, "DOCUMENTS/reports/2026/Q3.TXT").len(), 32);
    assert_eq!(whole(&mut v, &mut d, "GRÜßE-日本.TXT"), b"unicode\n");
    // The DOS alias still opens the file, as on Windows.
    assert_eq!(whole(&mut v, &mut d, "longna~1.txt"), b"long\n");
}

#[test]
fn errors_and_read_only() {
    let (mut d, mut v) = mount("ntfs");
    assert_eq!(v.stat(&mut d, b"nope"), Err(Error::NotFound));
    assert_eq!(v.list(&mut d, b"hello.txt"), Err(Error::NotDir));
    assert_eq!(v.stat(&mut d, b"hello.txt/x"), Err(Error::NotDir));
    assert_eq!(v.read(&mut d, b"Documents", 0, &mut [0; 4]), Err(Error::IsDir));
    assert_eq!(v.write(&mut d, b"hello.txt", 0, b"x"), Err(Error::ReadOnly));
    assert_eq!(v.create(&mut d, b"new.txt"), Err(Error::ReadOnly));
    assert_eq!(v.mkdir(&mut d, b"New"), Err(Error::ReadOnly));
    // Compressed files list and stat, but don't read.
    assert_eq!(names(&mut v, &mut d, "Packed"), vec![("squash.txt".into(), Kind::File, 20000)]);
    assert_eq!(v.read(&mut d, b"Packed/squash.txt", 0, &mut [0; 4]), Err(Error::Unsupported));
    // ntfs-3g's symlinks are Interix ones: a plain system file naming the target.
    let link = whole(&mut v, &mut d, "link");
    assert_eq!(&link[..8], b"IntxLNK\x01");
}

#[test]
fn small_clusters() {
    let (mut d, mut v) = mount("ntfs512");
    assert_eq!(v.label(), "SMALL");
    assert_eq!(whole(&mut v, &mut d, "hello.txt"), b"small clusters\n");
    let l = names(&mut v, &mut d, "Dir");
    assert_eq!(l.len(), 41);
    assert_eq!(l[0], ("data.bin".into(), Kind::File, 5000));
    assert_eq!(l[40], ("name 39.txt".into(), Kind::File, 3));
    assert_eq!(whole(&mut v, &mut d, "dir/DATA.BIN"), gen(5, 5000));
    assert_eq!(whole(&mut v, &mut d, "Dir/name 17.txt"), b"17\n");
    let (total, free) = v.statfs(&mut d).unwrap();
    assert!(free < total);
}

#[test]
fn scan_finds_it_bare_and_in_an_mbr_partition() {
    let mut bare = common::fixture("ntfs");
    let found = sv::scan(&mut bare).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].start, found[0].volume.format(), found[0].volume.label()), (0, Format::Ntfs, "WINDATA".to_string()));

    let start = 2048u64;
    let len = (bare.bytes.len() / sv::SECTOR) as u64;
    let mut disk = common::MemDisk::new(start + len + 16);
    disk.bytes[start as usize * sv::SECTOR..][..bare.bytes.len()].copy_from_slice(&bare.bytes);
    let e = &mut disk.bytes[446..462];
    e[4] = 0x07;
    e[8..12].copy_from_slice(&(start as u32).to_le_bytes());
    e[12..16].copy_from_slice(&(len as u32).to_le_bytes());
    disk.bytes[510] = 0x55;
    disk.bytes[511] = 0xAA;
    let mut found = sv::scan(&mut disk).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].start, found[0].len), (start, len));
    let mut slice = sv::Slice { disk: &mut disk, start, len };
    assert_eq!(whole(&mut *found[0].volume, &mut slice, "hello.txt"), b"Hello from Windows!\r\n");
}

#[test]
fn corrupt_images_fail_without_panicking() {
    let base = common::fixture("ntfs");
    // Flip bytes across the boot sector and the first MFT records; every call must
    // return, whatever it returns.
    let mft = 4096 * u64::from_le_bytes(base.bytes[0x30..0x38].try_into().unwrap()) as usize;
    let mut seed = 0x1234_5678u32;
    for round in 0..300 {
        let mut d = common::MemDisk { bytes: base.bytes.clone(), flushes: 0 };
        for _ in 0..4 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let at = if round % 3 == 0 { (seed >> 8) as usize % 512 } else { mft + (seed >> 8) as usize % (16 * 1024) };
            d.bytes[at] ^= (seed >> 3) as u8 | 1;
        }
        if let Ok(mut v) = sv::ntfs::Ntfs::mount(&mut d) {
            let _ = v.statfs(&mut d);
            let _ = v.list(&mut d, b"/");
            let _ = v.list(&mut d, b"Many");
            let mut buf = [0u8; 4096];
            let _ = v.read(&mut d, b"Documents/big.bin", 1000, &mut buf);
            let _ = v.read(&mut d, b"Links/linked.bin", 0, &mut buf);
        }
    }
}
