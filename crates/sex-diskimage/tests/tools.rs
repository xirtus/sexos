// SPDX-License-Identifier: GPL-3.0-only
//! Fixtures made by host tools at test time. Each test checks for its tools
//! and skips (with a note on stderr) when they are missing, so CI without
//! them still passes; `tools_available` prints what this host has.
//!
//! Ubuntu packages: xorriso genisoimage udftools qemu-utils gdisk fdisk
//! dosfstools e2fsprogs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use sex_diskimage::*;

const TOOLS: &[&str] = &["xorriso", "genisoimage", "mkisofs", "mkudffs", "qemu-img", "sgdisk", "sfdisk", "mkfs.fat", "mkfs.ext4"];

fn have(tool: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {tool}")).output().is_ok_and(|o| o.status.success())
}

fn skip(test: &str, tool: &str) -> bool {
    if have(tool) {
        return false;
    }
    eprintln!("SKIP {test}: `{tool}` not installed");
    true
}

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("tool starts");
    assert!(out.status.success(), "{:?} failed: {}", cmd, String::from_utf8_lossy(&out.stderr));
}

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sex-diskimage").join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn tools_available() {
    let (yes, no): (Vec<&str>, Vec<&str>) = TOOLS.iter().partition(|t| have(t));
    eprintln!("image tools present: {yes:?}; missing: {no:?}");
}

fn walk_host(root: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let mut items: Vec<_> = fs::read_dir(root.join(rel)).unwrap().map(|e| e.unwrap()).collect();
    items.sort_by_key(|e| e.file_name());
    for e in items {
        let name = e.file_name().into_string().unwrap();
        let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        out.push((r.clone(), e.path()));
        if e.file_type().unwrap().is_dir() {
            walk_host(root, &r, out);
        }
    }
}

/// A tree with long and non-ASCII names, deep folders, links, modes and mtimes.
fn make_tree(dir: &Path) -> SystemTime {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_650_000_000);
    let t = dir.join("tree");
    let mut deep = t.clone();
    for i in 0..10 {
        deep = deep.join(format!("level{i}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("bottom.txt"), b"at the bottom\n").unwrap();
    fs::write(t.join("A long file name with spaces and ünïcödé.txt"), b"long name\n").unwrap();
    fs::write(t.join("random.bin"), common::pattern(300_000, 5)).unwrap();
    fs::write(t.join("empty"), b"").unwrap();
    fs::create_dir(t.join("docs")).unwrap();
    fs::write(t.join("docs/x.txt"), b"x\n").unwrap();
    symlink("docs/x.txt", t.join("rel-link")).unwrap();
    symlink("/etc/hostname", t.join("abs-link")).unwrap();
    let f = t.join("docs/x.txt");
    fs::set_permissions(&f, fs::Permissions::from_mode(0o751)).unwrap();
    fs::File::options().write(true).open(&f).unwrap().set_modified(when).unwrap();
    fs::write(dir.join("boot.img"), common::pattern(2048, 11)).unwrap();
    fs::write(dir.join("efi.img"), common::pattern(64 * 1024, 12)).unwrap();
    when
}

#[test]
fn xorriso_rock_ridge_joliet_el_torito() {
    if skip("xorriso_rock_ridge_joliet_el_torito", "xorriso") {
        return;
    }
    use std::os::unix::fs::MetadataExt;
    let d = scratch("xorriso");
    let when = make_tree(&d);
    fs::copy(d.join("boot.img"), d.join("tree/boot.img")).unwrap();
    fs::copy(d.join("efi.img"), d.join("tree/efi.img")).unwrap();
    let iso = d.join("out.iso");
    run(Command::new("xorriso").current_dir(&d).args(["-as", "mkisofs", "-quiet", "-input-charset", "utf-8", "-R", "-J", "-joliet-long", "-V", "SEXTEST"]).args([
        "-b", "boot.img", "-no-emul-boot", "-boot-load-size", "4", "-c", "boot.cat", "-eltorito-alt-boot", "-e", "efi.img", "-no-emul-boot", "-o",
    ]).arg(&iso).arg("tree"));
    let bytes = fs::read(&iso).unwrap();
    let img = Image::open(bytes.clone()).unwrap();
    assert_eq!(img.kind(), Kind::Iso9660);
    let v = img.as_volume().unwrap();
    assert_eq!(v.label(), "SEXTEST");

    // Every host file is there with the same bytes, kind, link target and mode.
    let mut host = Vec::new();
    walk_host(&d.join("tree"), "", &mut host);
    for (rel, path) in &host {
        let e = v.lookup(rel).unwrap_or_else(|err| panic!("{rel}: {err}"));
        let meta = fs::symlink_metadata(path).unwrap();
        if meta.file_type().is_symlink() {
            assert_eq!(e.kind, EntryKind::Symlink(fs::read_link(path).unwrap().to_str().unwrap().into()), "{rel}");
        } else if meta.is_dir() {
            assert!(e.is_dir(), "{rel}");
        } else {
            assert_eq!(v.read_to_vec(&e.node, 1 << 24).unwrap(), fs::read(path).unwrap(), "{rel}");
            assert_eq!(e.mode, Some(meta.mode() & 0o7777), "{rel}");
        }
    }
    let x = v.lookup("docs/x.txt").unwrap();
    assert_eq!(x.mtime, Some(when.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as i64));
    assert_eq!(x.mode, Some(0o751));
    let listed: Vec<_> = v.entries().unwrap().into_iter().filter(|(_, e)| !e.synthetic).map(|(p, _)| p).collect();
    let mut want: Vec<String> = host.iter().map(|(r, _)| r.clone()).collect();
    want.push("boot.cat".into());
    let mut got = listed.clone();
    got.sort();
    want.sort();
    assert_eq!(got, want, "nothing extra, nothing missing (rr_moved hidden)");

    // El Torito: both images, with the bytes the catalog points at.
    let boot = v.list_dir(&v.lookup("[BOOT]").unwrap().node).unwrap();
    let names: Vec<_> = boot.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["boot.catalog", "Boot-1-BIOS.img", "Boot-2-UEFI.img"]);
    assert_eq!(v.read_to_vec(&boot[1].node, 1 << 20).unwrap(), fs::read(d.join("boot.img")).unwrap());
    let efi = v.read_to_vec(&boot[2].node, 1 << 20).unwrap();
    assert!(fs::read(d.join("efi.img")).unwrap().starts_with(&efi));

    // Joliet view of the same disc.
    let j = Volume::Iso(Iso::open_with(bytes, Namespace::Joliet).unwrap());
    let e = j.lookup("A long file name with spaces and ünïcödé.txt").unwrap();
    assert_eq!(j.read_to_vec(&e.node, 100).unwrap(), b"long name\n");
    assert_eq!(j.read_to_vec(&j.lookup("random.bin").unwrap().node, 1 << 20).unwrap(), common::pattern(300_000, 5));
}

#[test]
fn genisoimage_udf_hybrid() {
    let tool = ["genisoimage", "mkisofs"].into_iter().find(|t| have(t));
    let Some(tool) = tool else {
        eprintln!("SKIP genisoimage_udf_hybrid: neither `genisoimage` nor `mkisofs` installed");
        return;
    };
    let d = scratch("udf-hybrid");
    make_tree(&d);
    let iso = d.join("udf.iso");
    let out = Command::new(tool).args(["-quiet", "-udf", "-R", "-J", "-V", "HYBRID", "-o"]).arg(&iso).arg(d.join("tree")).output().unwrap();
    if !out.status.success() {
        eprintln!("SKIP genisoimage_udf_hybrid: `{tool} -udf` unsupported: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let img = Image::open(fs::read(&iso).unwrap()).unwrap();
    assert_eq!(img.kind(), Kind::Udf, "a hybrid disc opens as UDF");
    let v = img.as_volume().unwrap();
    assert_eq!(v.read_to_vec(&v.lookup("random.bin").unwrap().node, 1 << 20).unwrap(), common::pattern(300_000, 5));
    assert_eq!(v.read_to_vec(&v.lookup("docs/x.txt").unwrap().node, 100).unwrap(), b"x\n");
    assert!(v.lookup("level0/level1/level2/level3/level4/level5/level6/level7/level8/level9/bottom.txt").is_ok());
}

#[test]
fn mkudffs_revisions() {
    if skip("mkudffs_revisions", "mkudffs") {
        return;
    }
    let d = scratch("mkudffs");
    for rev in ["0x0102", "0x0201", "0x0250", "0x0260"] {
        let f = d.join(format!("udf-{rev}.img"));
        fs::File::create(&f).unwrap().set_len(32 << 20).unwrap();
        let out = Command::new("mkudffs").args(["--media-type=hd", &format!("--udfrev={rev}"), "--label=SEXUDF"]).arg(&f).output().unwrap();
        if !out.status.success() {
            eprintln!("SKIP mkudffs {rev}: {}", String::from_utf8_lossy(&out.stderr));
            continue;
        }
        let img = Image::open(fs::read(&f).unwrap()).unwrap_or_else(|e| panic!("rev {rev}: {e}"));
        assert_eq!(img.kind(), Kind::Udf);
        let v = img.as_volume().unwrap();
        assert_eq!(v.label(), "SEXUDF", "rev {rev}");
        assert!(v.list_dir(&v.root()).unwrap().iter().all(|e| e.name == "lost+found" || e.is_dir()), "rev {rev}");
    }
}

/// A raw disk with a GPT made by sgdisk, a FAT partition made by mkfs.fat
/// and an ext4 one made by mkfs.ext4 when those are present.
fn sgdisk_disk(d: &Path) -> Option<Vec<u8>> {
    if !have("sgdisk") {
        return None;
    }
    let f = d.join("gpt.img");
    fs::File::create(&f).unwrap().set_len(24 << 20).unwrap();
    run(Command::new("sgdisk").args(["-n", "1:2048:+8M", "-t", "1:ef00", "-c", "1:EFI stuff", "-n", "2:0:+8M", "-t", "2:8300", "-c", "2:linux root", "-n", "3:0:0", "-t", "3:8200"]).arg(&f));
    let mut disk = fs::read(&f).unwrap();
    if have("mkfs.fat") {
        let p = d.join("fat.img");
        let _ = fs::remove_file(&p);
        run(Command::new("mkfs.fat").args(["-F", "16", "-s", "1", "-C"]).arg(&p).arg("8192"));
        let fat = fs::read(&p).unwrap();
        disk[2048 * 512..2048 * 512 + fat.len()].copy_from_slice(&fat);
    }
    if have("mkfs.ext4") {
        let p = d.join("ext4.img");
        fs::File::create(&p).unwrap().set_len(8 << 20).unwrap();
        run(Command::new("mkfs.ext4").args(["-q", "-F"]).arg(&p));
        let e = fs::read(&p).unwrap();
        let at = (2048 + 16384) * 512;
        disk[at..at + e.len()].copy_from_slice(&e);
    }
    let mut p = common::pattern(100_000, 77);
    p[0] = 1;
    disk[(24 << 20) - 150_000..(24 << 20) - 50_000].copy_from_slice(&p);
    Some(disk)
}

#[test]
fn sgdisk_gpt_partitions() {
    if skip("sgdisk_gpt_partitions", "sgdisk") {
        return;
    }
    let d = scratch("sgdisk");
    let disk = sgdisk_disk(&d).unwrap();
    let img = Image::open(disk).unwrap();
    let parts = img.partitions().unwrap();
    let got: Vec<_> = parts.iter().map(|p| (p.index, p.type_label, p.name.as_str(), p.offset / 512, p.len / 512)).collect();
    assert_eq!(got[0], (1, "EFI System", "EFI stuff", 2048, 16384));
    assert_eq!(got[1], (2, "Linux filesystem", "linux root", 2048 + 16384, 16384));
    assert_eq!((got[2].0, got[2].1), (3, "Linux swap"));
    if have("mkfs.fat") {
        assert_eq!(parts[0].fs, FsKind::Fat16);
    }
    if have("mkfs.ext4") {
        assert_eq!(parts[1].fs, FsKind::Ext4);
    }
}

#[test]
fn sfdisk_mbr_logical_partitions() {
    if skip("sfdisk_mbr_logical_partitions", "sfdisk") {
        return;
    }
    let d = scratch("sfdisk");
    let f = d.join("mbr.img");
    fs::File::create(&f).unwrap().set_len(16 << 20).unwrap();
    let mut c = Command::new("sfdisk");
    c.arg("-q").arg(&f).stdin(std::process::Stdio::piped());
    let mut child = c.spawn().unwrap();
    use std::io::Write;
    // Explicit starts inside the extended partition make both Linux entries logical (5, 6);
    // with a default start sfdisk places the last one after the extended as primary 3.
    child.stdin.take().unwrap().write_all(b"label: dos\n2048,4096,c,*\n6144,20000,5\n8192,4096,83\n12800,4096,82\n").unwrap();
    assert!(child.wait().unwrap().success());
    let img = Image::open(fs::read(&f).unwrap()).unwrap();
    let got: Vec<_> = img.partitions().unwrap().iter().map(|p| (p.index, p.type_label, p.bootable)).collect();
    assert_eq!(got, [(1, "FAT32", true), (5, "Linux", false), (6, "Linux swap", false)]);
}

#[test]
fn qemu_img_containers_match_raw() {
    if skip("qemu_img_containers_match_raw", "qemu-img") {
        return;
    }
    let d = scratch("qemu-img");
    let raw = sgdisk_disk(&d).unwrap_or_else(common::build_mbr_disk);
    let rp = d.join("disk.raw");
    fs::write(&rp, &raw).unwrap();
    let cases: &[(&str, &[&str], Kind)] = &[
        ("qcow2", &["-O", "qcow2"], Kind::Qcow2),
        ("qcow2c", &["-c", "-O", "qcow2"], Kind::Qcow2),
        ("qcow2v2", &["-O", "qcow2", "-o", "compat=0.10"], Kind::Qcow2),
        ("vhd-dyn", &["-O", "vpc"], Kind::Vhd),
        ("vhd-fixed", &["-O", "vpc", "-o", "subformat=fixed"], Kind::Vhd),
        ("vhdx", &["-O", "vhdx"], Kind::Vhdx),
        ("vmdk", &["-O", "vmdk"], Kind::Vmdk),
        ("vmdk-stream", &["-O", "vmdk", "-o", "subformat=streamOptimized"], Kind::Vmdk),
    ];
    for (name, args, kind) in cases {
        let out = d.join(format!("disk.{name}"));
        run(Command::new("qemu-img").arg("convert").args(["-f", "raw"]).args(*args).arg(&rp).arg(&out));
        let img = Image::open(fs::read(&out).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(img.kind(), *kind, "{name}");
        let dev = img.as_disk().unwrap();
        // VPC rounds the size up to its CHS geometry.
        assert!(dev.size() >= raw.len() as u64, "{name}");
        let mut got = vec![0u8; raw.len()];
        let mut at = 0;
        while at < got.len() {
            at += dev.read(at as u64, &mut got[at..]).unwrap();
        }
        assert!(got == raw, "{name}: bytes differ");
        assert_eq!(img.partitions().unwrap().len(), Image::open(raw.clone()).unwrap().partitions().unwrap().len(), "{name}");
    }
}
