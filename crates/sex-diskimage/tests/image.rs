// SPDX-License-Identifier: GPL-3.0-only
//! Hand-built fixtures: always run, no host tools needed.

mod common;

use common::*;
use sex_diskimage::*;

fn open(img: Vec<u8>) -> Image<Vec<u8>> {
    Image::open(img).expect("opens")
}

fn vol(img: Vec<u8>) -> Volume<Vec<u8>> {
    match open(img) {
        Image::Folder(v) => v,
        Image::Disk(_) => panic!("expected a folder"),
    }
}

fn names(v: &[Entry]) -> Vec<&str> {
    v.iter().map(|e| e.name.as_str()).collect()
}

fn read_all<S: ByteSource>(v: &Volume<S>, e: &Entry) -> Vec<u8> {
    v.read_to_vec(&e.node, 1 << 30).unwrap()
}

// ------------------------------------------------------------------ ISO

#[test]
fn iso_rock_ridge_tree() {
    let v = vol(build_iso());
    assert_eq!(v.kind(), Kind::Iso9660);
    assert_eq!(v.label(), "TESTDISC");
    let Volume::Iso(iso) = &v else { panic!() };
    assert!(iso.has_rock_ridge() && iso.has_joliet() && iso.is_bootable());
    assert_eq!(iso.namespace(), Namespace::RockRidge);

    let root = v.list_dir(&v.root()).unwrap();
    assert_eq!(names(&root), ["readme-with-a-long-name.txt", "big.bin", "link-to-inner", "SubDir", "[BOOT]"]);
    let readme = &root[0];
    assert_eq!(readme.kind, EntryKind::File);
    assert_eq!(readme.mode, Some(0o644));
    assert_eq!(readme.mtime, Some(rr_date_unix()), "Rock Ridge TF beats the record date");
    assert_eq!(read_all(&v, readme), README);

    let link = &root[2];
    assert_eq!(link.kind, EntryKind::Symlink("../SubDir/inner.txt".into()));
    assert_eq!(link.mode, Some(0o777));

    let sub = v.lookup("SubDir/inner.txt").unwrap();
    assert_eq!(read_all(&v, &sub), INNER);
    assert_eq!(sub.mtime, Some(rec_date_unix()), "no TF: the record date with its zone");
    assert_eq!(v.lookup("/subdir/INNER.TXT").unwrap().name, "inner.txt", "case-insensitive fallback");
    assert_eq!(v.lookup("nope"), Err(Error::NotFound));
    // Relocated directory (RE/CL) shows at its original place; rr_moved is hidden.
    assert_eq!(read_all(&v, &v.lookup("SubDir/deep/leaf.txt").unwrap()), INNER);
    assert!(v.lookup("SubDir/deep").unwrap().is_dir());
    assert_eq!(v.lookup("readme-with-a-long-name.txt/x"), Err(Error::NotADirectory));
}

#[test]
fn iso_multi_extent_file() {
    let v = vol(build_iso());
    let big = v.lookup("big.bin").unwrap();
    let (a, b) = big_parts();
    assert_eq!(big.size, (a.len() + b.len()) as u64);
    let mut want = a.clone();
    want.extend(&b);
    assert_eq!(read_all(&v, &big), want);
    // A read spanning the extent boundary returns the first part short, then the rest.
    let mut buf = [0u8; 100];
    let n = v.read(&big.node, 2000, &mut buf).unwrap();
    assert_eq!(&buf[..n], &want[2000..2000 + n]);
    let n2 = v.read(&big.node, 2000 + n as u64, &mut buf).unwrap();
    assert_eq!(&buf[..n2], &want[2000 + n..2000 + n + n2]);
    assert_eq!(v.read(&big.node, want.len() as u64, &mut buf).unwrap(), 0);
    // As a ByteSource.
    let src = v.file_source(&big.node).unwrap();
    assert_eq!(src.len(), want.len() as u64);
}

#[test]
fn iso_joliet_and_plain_names() {
    let iso = Iso::open_with(build_iso(), Namespace::Joliet).unwrap();
    assert_eq!(iso.label(), "Joliet Disc");
    let v = Volume::Iso(iso);
    let root = v.list_dir(&v.root()).unwrap();
    assert_eq!(names(&root), ["Readme – Jöliet.txt", "big.bin", "SubDir", "[BOOT]"]);
    assert_eq!(root[1].size, 3048);
    assert_eq!(root[0].mtime, Some(rec_date_unix()));
    assert_eq!(read_all(&v, &v.lookup("SubDir/inner.txt").unwrap()), INNER);

    let v = Volume::Iso(Iso::open_with(build_iso(), Namespace::Plain).unwrap());
    let root = v.list_dir(&v.root()).unwrap();
    assert_eq!(names(&root), ["README.TXT", "BIG.BIN", "LINK", "SUBDIR", "RR_MOVED", "[BOOT]"]);
    assert_eq!(root[2].kind, EntryKind::File, "without Rock Ridge a link is an empty file");
}

#[test]
fn iso_el_torito_boot_folder() {
    let v = vol(build_iso());
    let boot = v.lookup("[BOOT]").unwrap();
    assert!(boot.synthetic && boot.is_dir());
    let items = v.list_dir(&boot.node).unwrap();
    assert_eq!(names(&items), ["boot.catalog", "Boot-1-BIOS.img", "Boot-2-UEFI.img"]);
    assert_eq!(items[1].size, 4 * 512);
    assert_eq!(items[2].size, 8 * 512);
    assert!(read_all(&v, &items[1]).starts_with(BOOT_IMG));
    assert!(read_all(&v, &items[2]).starts_with(EFI_IMG));
    let cat = read_all(&v, &items[0]);
    assert_eq!(&cat[30..32], &[0x55, 0xAA]);
}

#[test]
fn iso_entries_walk() {
    let v = vol(build_iso());
    let all = v.entries().unwrap();
    let paths: Vec<&str> = all.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            "readme-with-a-long-name.txt",
            "big.bin",
            "link-to-inner",
            "SubDir",
            "[BOOT]",
            "SubDir/inner.txt",
            "SubDir/deep",
            "SubDir/deep/leaf.txt",
            "[BOOT]/boot.catalog",
            "[BOOT]/Boot-1-BIOS.img",
            "[BOOT]/Boot-2-UEFI.img"
        ]
    );
}

#[test]
fn iso_damage_is_an_error_not_a_panic() {
    let good = build_iso();
    // Truncations at many points, and byte flips in the directory area.
    for cut in [17 * 2048, 21 * 2048 + 40, 25 * 2048, 28 * 2048] {
        if let Ok(Image::Folder(v)) = Image::open(good[..cut].to_vec()) {
            let _ = v.entries();
        }
    }
    for i in (21 * 2048..26 * 2048).step_by(7) {
        let mut bad = good.clone();
        bad[i] ^= 0xFF;
        if let Ok(Image::Folder(v)) = Image::open(bad) {
            if let Ok(all) = v.entries() {
                for (_, e) in all.iter().filter(|(_, e)| e.kind == EntryKind::File).take(4) {
                    let _ = v.read_to_vec(&e.node, 1 << 20);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ UDF

fn check_udf(v: &Volume<Vec<u8>>) {
    assert_eq!(v.kind(), Kind::Udf);
    assert_eq!(v.label(), "UDF Test Volume");
    let root = v.list_dir(&v.root()).unwrap();
    assert_eq!(names(&root), ["embedded.txt", "日本語.bin", "Sub Folder", "link", "abs-link"]);
    assert_eq!(read_all(v, &root[0]), UDF_EMBEDDED);
    assert_eq!(root[0].mode, Some(0o544));
    assert_eq!(root[0].mtime, Some(udf_ts_unix()));
    assert_eq!(read_all(v, &root[1]), udf_short_data());
    assert!(root[2].is_dir());
    assert_eq!(root[3].kind, EntryKind::Symlink("../Sub Folder/long.dat".into()));
    assert_eq!(root[4].kind, EntryKind::Symlink("/etc/hosts".into()));
    assert!(root[4].hidden && !root[3].hidden);
    let long = v.lookup("Sub Folder/long.dat").unwrap();
    assert_eq!(long.size, udf_long_data().len() as u64);
    assert_eq!(read_all(v, &long), udf_long_data());
    let mut buf = [0xEEu8; 64];
    let n = v.read(&long.node, 2048 + 100, &mut buf).unwrap();
    assert!(n > 0 && buf[..n].iter().all(|&b| b == 0), "unallocated run reads as zeros");
    assert_eq!(v.list_dir(&root[0].node), Err(Error::NotADirectory));
    assert_eq!(v.entries().unwrap().len(), 6);
}

#[test]
fn udf_physical_partition() {
    for rev in [0x0102, 0x0201] {
        let v = vol(build_udf(false, rev));
        let Volume::Udf(u) = &v else { panic!() };
        assert_eq!(u.revision(), rev);
        check_udf(&v);
    }
}

#[test]
fn udf_metadata_partition() {
    for rev in [0x0250, 0x0260] {
        check_udf(&vol(build_udf(true, rev)));
    }
}

#[test]
fn udf_hybrid_with_iso_prefers_udf() {
    // Put an ISO 9660 PVD in front of the UDF recognition sequence.
    let mut img = build_udf(false, 0x0201);
    let iso = build_iso();
    img[16 * 2048..17 * 2048].copy_from_slice(&iso[16 * 2048..17 * 2048]);
    // The VRS must still have BEA01 after the ISO descriptors: move it along.
    img[17 * 2048..20 * 2048].fill(0);
    for (i, id) in [b"BEA01", b"NSR03", b"TEA01"].iter().enumerate() {
        let at = (18 + i) * 2048;
        img[at + 1..at + 6].copy_from_slice(*id);
    }
    img[17 * 2048] = 255;
    img[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
    assert_eq!(Image::detect(&img).unwrap(), Some(Kind::Udf));
    check_udf(&vol(img));
}

#[test]
fn udf_damage_is_an_error_not_a_panic() {
    let good = build_udf(true, 0x0250);
    for i in (272 * 2048..(272 + 48) * 2048).step_by(97) {
        let mut bad = good.clone();
        bad[i] ^= 0x5A;
        if let Ok(Image::Folder(v)) = Image::open(bad) {
            if let Ok(all) = v.entries() {
                for (_, e) in all.iter().filter(|(_, e)| e.kind == EntryKind::File) {
                    let _ = v.read_to_vec(&e.node, 1 << 20);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ disks

fn test_disk() -> Vec<u8> {
    let mut d = build_mbr_disk();
    // Recognisable data spread over clusters, including unaligned edges.
    let p = pattern(300_000, 42);
    d[300_001..600_001].copy_from_slice(&p);
    let q = pattern(70_000, 43);
    d[7_000_000..7_070_000].copy_from_slice(&q);
    d
}

fn check_partitions(parts: &[Partition]) {
    let summary: Vec<(u32, &str, FsKind, u64, u64, bool)> = parts.iter().map(|p| (p.index, p.type_label, p.fs, p.offset / 512, p.len / 512, p.bootable)).collect();
    assert_eq!(
        summary,
        [
            (1, "FAT16", FsKind::Fat16, 2048, 2048, true),
            (5, "Linux", FsKind::Ext4, 4096 + 64, 1024, false),
            (6, "Linux swap", FsKind::Swap, 4096 + 2048 + 64, 1024, false),
            (3, "NTFS/exFAT", FsKind::Ntfs, 12288, 4096, false),
        ]
    );
    assert!(parts.iter().all(|p| p.scheme == Scheme::Mbr));
}

fn same_bytes<S: ByteSource>(dev: &BlockDevice<S>, want: &[u8]) {
    assert_eq!(dev.size(), want.len() as u64);
    let mut got = vec![0u8; want.len()];
    let mut at = 0;
    // Odd-sized reads cross every cluster and block boundary.
    while at < got.len() {
        let end = (at + 7777).min(got.len());
        let mut done = at;
        while done < end {
            let n = dev.read(done as u64, &mut got[done..end]).unwrap();
            assert!(n > 0);
            done += n;
        }
        at = end;
    }
    assert!(got == want, "virtual disk bytes differ");
    let mut b = [0u8; 4];
    assert_eq!(dev.read(want.len() as u64, &mut b).unwrap(), 0);
}

#[test]
fn raw_mbr_with_logical_partitions() {
    let d = test_disk();
    let img = open(d.clone());
    assert_eq!(img.kind(), Kind::Raw);
    check_partitions(&img.partitions().unwrap());
    let dev = img.as_disk().unwrap();
    same_bytes(dev, &d);
    let parts = dev.partitions().unwrap();
    let s = dev.slice(&parts[0]);
    assert_eq!(s.len(), 2048 * 512);
    let mut boot = [0u8; 512];
    s.read_at(0, &mut boot).unwrap();
    assert_eq!(boot.to_vec(), fat16_boot());
}

#[test]
fn raw_gpt_via_sex_disk() {
    let d = build_gpt_disk();
    let img = open(d);
    let parts = img.partitions().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!((parts[0].type_label, parts[0].name.as_str(), parts[0].fs), ("EFI System", "EFI system", FsKind::Fat16));
    assert_eq!((parts[1].type_label, parts[1].name.as_str(), parts[1].fs), ("Linux filesystem", "Wurzel – root", FsKind::Ext4));
    assert_eq!((parts[1].offset, parts[1].len), (4096 * 512, 4096 * 512));
    assert!(parts.iter().all(|p| p.scheme == Scheme::Gpt));
}

#[test]
fn raw_without_table_and_unknown_data() {
    let mut d = vec![0u8; 1 << 20];
    d[..512].copy_from_slice(&fat16_boot());
    let img = open(d);
    assert_eq!(img.partitions().unwrap(), vec![]);
    assert_eq!(img.as_disk().unwrap().filesystem(), FsKind::Fat16);
    assert!(matches!(Image::open(vec![1u8; 1000]), Err(Error::NotRecognized)));
}

#[test]
fn qcow2_plain_and_compressed_clusters() {
    let d = test_disk();
    let img = open(build_qcow2(&d));
    assert_eq!(img.kind(), Kind::Qcow2);
    same_bytes(img.as_disk().unwrap(), &d);
    check_partitions(&img.partitions().unwrap());
}

#[test]
fn qcow2_refuses_backing_files_and_encryption() {
    let d = test_disk();
    let mut q = build_qcow2(&d);
    q[8..16].copy_from_slice(&4096u64.to_be_bytes());
    assert!(matches!(Image::open(q), Err(Error::Unsupported(m)) if m.contains("backing")));
    let mut q = build_qcow2(&d);
    q[32..36].copy_from_slice(&1u32.to_be_bytes());
    assert!(matches!(Image::open(q), Err(Error::Unsupported(_))));
    let mut q = build_qcow2(&d);
    q[20..24].copy_from_slice(&40u32.to_be_bytes());
    assert!(matches!(Image::open(q), Err(Error::Corrupt(_))));
}

#[test]
fn vhd_fixed_and_dynamic() {
    let d = test_disk();
    for (img, label) in [(build_vhd_fixed(&d), "fixed"), (build_vhd_dynamic(&d), "dynamic")] {
        let img = open(img);
        assert_eq!(img.kind(), Kind::Vhd, "{label}");
        same_bytes(img.as_disk().unwrap(), &d);
        check_partitions(&img.partitions().unwrap());
    }
    let mut bad = build_vhd_fixed(&d);
    let n = bad.len();
    bad[n - 512 + 60] = 4;
    assert!(Image::open(bad).is_err(), "checksum no longer matches");
}

#[test]
fn vhdx_blocks_and_states() {
    let d = test_disk();
    let img = open(build_vhdx(&d));
    assert_eq!(img.kind(), Kind::Vhdx);
    same_bytes(img.as_disk().unwrap(), &d);
    check_partitions(&img.partitions().unwrap());
    let mut dirty = build_vhdx(&d);
    // A log GUID in both headers means a log to replay.
    for at in [64 * 1024, 128 * 1024] {
        dirty[at + 48] = 1;
        dirty[at + 4..at + 8].fill(0);
        let c = crc32c(&dirty[at..at + 4096]);
        dirty[at + 4..at + 8].copy_from_slice(&c.to_le_bytes());
    }
    assert!(matches!(Image::open(dirty), Err(Error::Unsupported(_))));
}

#[test]
fn vmdk_sparse_and_stream_optimized() {
    let d = test_disk();
    for stream in [false, true] {
        let img = open(build_vmdk(&d, stream, false));
        assert_eq!(img.kind(), Kind::Vmdk);
        same_bytes(img.as_disk().unwrap(), &d);
        check_partitions(&img.partitions().unwrap());
    }
    assert!(matches!(Image::open(build_vmdk(&d, false, true)), Err(Error::Unsupported(m)) if m.contains("parent")));
    let text = b"# Disk DescriptorFile\nversion=1\ncreateType=\"monolithicFlat\"\n".to_vec();
    assert!(matches!(Image::open(text), Err(Error::Unsupported(_))));
}

#[test]
fn iso_inside_a_qcow2() {
    // A BlockDevice is a ByteSource: images nest.
    let mut iso = build_iso();
    iso.resize(iso.len().next_multiple_of(65536), 0);
    let img = open(build_qcow2(&iso));
    let Image::Disk(dev) = img else { panic!() };
    let inner = Image::open(dev).unwrap();
    assert_eq!(inner.kind(), Kind::Iso9660);
    let v = inner.as_volume().unwrap();
    assert_eq!(v.read_to_vec(&v.lookup("SubDir/inner.txt").unwrap().node, 100).unwrap(), INNER);
}

#[test]
fn container_damage_is_an_error_not_a_panic() {
    let d = test_disk();
    let images = [build_qcow2(&d), build_vhd_dynamic(&d), build_vhdx(&d), build_vmdk(&d, false, false), build_vmdk(&d, true, false)];
    for good in images {
        for i in (0..good.len().min(3 << 20)).step_by(16411) {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            if let Ok(Image::Disk(dev)) = Image::open(bad) {
                let mut buf = vec![0u8; 65536];
                let mut at = 0;
                while at < dev.size().min(4 << 20) {
                    let _ = dev.read(at, &mut buf);
                    at += 65536 * 7;
                }
                let _ = dev.partitions();
            }
        }
    }
}
