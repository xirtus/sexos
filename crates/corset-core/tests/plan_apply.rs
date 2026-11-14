mod common;
use common::*;
use corset_core::crypt::CryptDev;
use corset_core::types;
use corset_core::{apply, AsSexfs, BlockDevice, Content, Disk, Error, FsKind, MemDisk, Op, Plan, Refusal, Secret, SexfsMemory, Window, SECTOR};
use sex_crypt::Cost;
use sex_volumes::Volume;

const CHEAP: Cost = Cost { memory_kib: 64, passes: 1, lanes: 1 };

fn blank(mib: u64) -> MemDisk { MemDisk::zeroed((mib * MIB) as usize * SECTOR) }

fn add(p: &mut Plan, first_mib: u64, mib: u64, t: corset_core::Guid, name: &str) -> usize {
    p.push(Op::Add { first: first_mib * MIB, sectors: mib * MIB, type_guid: t, name: name.into() }).unwrap()
}

fn sexfs_put(dev: &mut dyn BlockDevice, path: &[u8], data: &[u8]) {
    let blocks = dev.sectors() / 8;
    let mut mem = SexfsMemory::new(blocks, 16);
    let mut fs = sexfs::Fs::mount(AsSexfs(dev), mem.lend()).unwrap();
    let id = fs.create(path, 1).unwrap();
    assert_eq!(fs.write(id, 0, data, 1).unwrap(), data.len());
    fs.commit().unwrap();
}
fn sexfs_get(dev: &mut dyn BlockDevice, path: &[u8]) -> (Vec<u8>, u64) {
    let blocks = dev.sectors() / 8;
    let mut mem = SexfsMemory::new(blocks, 16);
    let mut fs = sexfs::Fs::mount(AsSexfs(dev), mem.lend()).unwrap();
    let st = fs.open(path).unwrap();
    let mut out = vec![0u8; st.size as usize];
    assert_eq!(fs.read(st.id, 0, &mut out).unwrap(), out.len());
    (out, fs.statfs().total)
}

#[test]
fn blank_disk_to_fat32_and_sexfs_seen_by_other_tools_and_by_files() {
    let mut d = blank(256);
    let disk = Disk::read(&mut d, "QEMU blank").unwrap();
    assert_eq!(disk.layout, sex_disk::probe::Layout::Blank);
    assert!(disk.gpt.is_none() && disk.whole == Content::Empty);
    let mut p = Plan::new(disk);
    assert_eq!(p.push(Op::Add { first: MIB, sectors: MIB, type_guid: types::BASIC_DATA, name: "x".into() }), Err(Refusal::NoTable));
    p.push(Op::CreateTable).unwrap();
    let fat = add(&mut p, 1, 100, types::BASIC_DATA, "SHARED");
    p.push(Op::Format { slot: fat, fs: FsKind::Fat32, label: "Corset".into() }).unwrap();
    let start = p.first_fit(100 * MIB).unwrap();
    assert_eq!(start, 101 * MIB);
    let sx = p.push(Op::Add { first: start, sectors: 100 * MIB, type_guid: types::SEXFS, name: "SexOS data".into() }).unwrap();
    p.push(Op::Format { slot: sx, fs: FsKind::Sexfs, label: String::new() }).unwrap();
    assert!(!p.destructive(), "nothing on a blank disk is lost");
    assert!(p.confirms(""));
    let text = p.confirmation();
    assert!(text.contains("On disk \"QEMU blank\" (256 MiB)"), "{text}");
    assert!(text.contains("Add partition 1 \"SHARED\" (Basic data (FAT, exFAT, NTFS)), 100 MiB from 1 MiB to 101 MiB."), "{text}");
    assert!(text.contains("Format partition 1 \"SHARED\" (100 MiB) as FAT32 labelled \"Corset\"."), "{text}");

    let mut seen = Vec::new();
    let r = apply(&mut d, &p, &mut rng(1), &mut |pr| seen.push((pr.step, pr.steps, pr.text.to_string()))).unwrap();
    assert!(r.table_written);
    // A blank disk is all free space: both formats land before the table.
    assert_eq!((r.before_table, r.after_table), (2, 0));
    assert_eq!(seen.last().unwrap().0, 3);
    assert!(seen.iter().any(|s| s.2.starts_with("Write the partition table")));
    let parts = &r.disk.partitions;
    assert_eq!(parts.len(), 2);
    assert_eq!((parts[0].first, parts[0].sectors, parts[0].content, parts[0].name.as_str()), (MIB, 100 * MIB, Content::Fat32, "SHARED"));
    assert_eq!((parts[1].first, parts[1].content, parts[1].type_guid), (101 * MIB, Content::Sexfs, types::SEXFS));
    assert!(!parts[0].part_guid.is_zero() && parts[0].part_guid != parts[1].part_guid);

    if let Some(t) = sgdisk_ok(&d, "two") {
        assert!(t.contains("SHARED") && t.contains("SexOS data"), "{t}");
    }
    if let Some(t) = fsck_fat_ok(&d, MIB, 100 * MIB, "two") { assert!(t.contains("204800 sectors total"), "{t}"); }

    // Files' view: sex-volumes finds the FAT partition and writes a file to it.
    let found = sex_volumes::scan(&mut Sv(&mut d)).unwrap();
    assert_eq!(found.len(), 1, "sex-volumes mounts FAT; sexfs partitions are not its business");
    assert_eq!(found[0].start, MIB);
    assert_eq!(found[0].volume.label(), "CORSET");
    let mut w = Window::new(&mut d, MIB, 100 * MIB).unwrap();
    let mut v = sex_volumes::fat::Fat::mount(&mut Sv(&mut w)).unwrap();
    v.create(&mut Sv(&mut w), b"/hello.txt").unwrap();
    v.write(&mut Sv(&mut w), b"/hello.txt", 0, b"written through sex-volumes").unwrap();
    v.sync(&mut Sv(&mut w)).unwrap();
    let mut buf = [0u8; 64];
    let n = v.read(&mut Sv(&mut w), b"/hello.txt", 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"written through sex-volumes");
    drop(w);
    fsck_fat_ok(&d, MIB, 100 * MIB, "two-after");

    // The sexfs partition mounts, takes a file, and gives it back after a remount.
    let mut w = Window::new(&mut d, 101 * MIB, 100 * MIB).unwrap();
    sexfs_put(&mut w, b"/note", b"sexfs on its own partition");
    assert_eq!(sexfs_get(&mut w, b"/note").0, b"sexfs on its own partition");
}

#[test]
fn checks_alignment_overlap_sizes_and_limits() {
    let mut d = blank(128);
    let mut p = Plan::new(Disk::read(&mut d, "disk").unwrap());
    p.push(Op::CreateTable).unwrap();
    let e = |first, sectors| Op::Add { first, sectors, type_guid: types::BASIC_DATA, name: "p".into() };
    assert_eq!(p.push(e(MIB + 1, MIB)), Err(Refusal::Unaligned));
    assert_eq!(p.push(e(MIB, MIB + 8)), Err(Refusal::Unaligned));
    assert_eq!(p.push(e(MIB, 0)), Err(Refusal::Empty));
    assert_eq!(p.push(e(0, MIB)), Err(Refusal::OutsideDisk), "the first MiB holds the table");
    assert_eq!(p.push(e(127 * MIB, MIB)), Err(Refusal::OutsideDisk), "the last 33 sectors hold the backup");
    assert_eq!(p.push(Op::Add { first: MIB, sectors: MIB, type_guid: corset_core::Guid::ZERO, name: "p".into() }), Err(Refusal::ZeroType));
    assert_eq!(p.push(Op::Add { first: MIB, sectors: MIB, type_guid: types::SEXOS_STORE, name: "p".into() }), Err(Refusal::SystemType));
    assert_eq!(p.push(Op::Add { first: MIB, sectors: MIB, type_guid: types::BASIC_DATA, name: "x".repeat(37) }), Err(Refusal::NameTooLong));
    let a = add(&mut p, 1, 32, types::BASIC_DATA, "a");
    assert_eq!(p.push(e(32 * MIB, 4 * MIB)), Err(Refusal::Overlaps(a)));
    // FAT32 needs 65525 clusters: 32 MiB is too small, 33 MiB is enough.
    match p.push(Op::Format { slot: a, fs: FsKind::Fat32, label: String::new() }) {
        Err(Refusal::TooSmall { fs: "FAT32", min }) => assert_eq!(min, 33 * MIB),
        other => panic!("{other:?}"),
    }
    p.push(Op::Resize { slot: a, sectors: 33 * MIB }).unwrap();
    p.push(Op::Format { slot: a, fs: FsKind::Fat32, label: String::new() }).unwrap();
    assert_eq!(p.push(Op::Format { slot: a, fs: FsKind::Fat32, label: "twelve chars".into() }), Err(Refusal::BadLabel));
    assert_eq!(p.push(Op::Format { slot: a, fs: FsKind::Sexfs, label: "x".into() }), Err(Refusal::BadLabel));
    // A filesystem without a grower in the tree is neither grown nor shrunk with its partition.
    assert_eq!(p.push(Op::Resize { slot: a, sectors: 40 * MIB }), Err(Refusal::CannotGrow(Content::Fat32)));
    assert_eq!(p.push(Op::Resize { slot: a, sectors: 32 * MIB }), Err(Refusal::CannotShrink(Content::Fat32)));
    assert_eq!(p.push(Op::SetAttrs { slot: a, attrs: 1 << 20 }), Err(Refusal::ReservedAttrs));
    assert_eq!(p.push(Op::Delete { slot: 9 }), Err(Refusal::NoSuchPartition(9)));
    assert_eq!(p.push(Op::Encrypt { slot: a, password: Secret(vec![]), cost: CHEAP, inner: None }), Err(Refusal::EmptyPassword));
    assert_eq!(p.push(Op::Encrypt { slot: a, password: Secret(b"pw".to_vec()), cost: Cost { memory_kib: 1, passes: 1, lanes: 1 }, inner: None }), Err(Refusal::BadCost));
    // 128 entries, then the table is full.
    let mut q = Plan::new(Disk::read(&mut blank(200), "big").unwrap());
    q.push(Op::CreateTable).unwrap();
    for i in 0..128 { add(&mut q, 1 + i, 1, types::LINUX_FS, "n"); }
    assert_eq!(q.push(e(150 * MIB, MIB)), Err(Refusal::TableFull));
    // Undo goes back one step and the layout with it.
    let before = p.partitions();
    p.push(Op::SetName { slot: a, name: "renamed".into() }).unwrap();
    assert_eq!(p.partition(a).unwrap().name, "renamed");
    assert!(matches!(p.pop(), Some(Op::SetName { .. })));
    assert_eq!(p.partitions(), before);
    assert!(format!("{:?}", Op::Encrypt { slot: 0, password: Secret(b"hunter2".to_vec()), cost: CHEAP, inner: None }).contains("Secret(..)"));
}

#[test]
fn grow_sexfs_and_metadata_round_trip_with_foreign_entries_kept() {
    let mut d = blank(128);
    let mut p = Plan::new(Disk::read(&mut d, "disk").unwrap());
    p.push(Op::CreateTable).unwrap();
    let s = add(&mut p, 1, 16, types::SEXFS, "grow me");
    p.push(Op::Format { slot: s, fs: FsKind::Sexfs, label: String::new() }).unwrap();
    let k = add(&mut p, 40, 8, types::LINUX_FS, "keep");
    apply(&mut d, &p, &mut rng(2), &mut quiet).unwrap();
    sexfs_put(&mut Window::new(&mut d, MIB, 16 * MIB).unwrap(), b"/kept", b"still here after growing");
    let disk = Disk::read(&mut d, "disk").unwrap();
    let keep_raw = disk.gpt.as_ref().unwrap().entries[k].raw.clone();

    let mut p = Plan::new(disk);
    assert_eq!(p.push(Op::Resize { slot: s, sectors: 40 * MIB }), Err(Refusal::Overlaps(k)));
    let t = p.push(Op::Resize { slot: s, sectors: 32 * MIB }).unwrap();
    assert_eq!(t, s);
    assert!(p.steps()[0].text.contains("Grow partition 1 \"grow me\" from 16 MiB to 32 MiB; sexfs takes the new space at its next mount"), "{}", p.steps()[0].text);
    assert_eq!(p.push(Op::Resize { slot: s, sectors: 8 * MIB }), Err(Refusal::CannotShrink(Content::Sexfs)));
    p.push(Op::SetName { slot: s, name: "grown ✓".into() }).unwrap();
    p.push(Op::SetType { slot: s, type_guid: types::LINUX_FS }).unwrap();
    p.push(Op::SetAttrs { slot: s, attrs: types::attr::HIDDEN | types::attr::NO_AUTOMOUNT }).unwrap();
    assert!(!p.destructive());
    let r = apply(&mut d, &p, &mut rng(3), &mut quiet).unwrap();
    assert_eq!((r.before_table, r.after_table, r.table_written), (0, 0, true));
    let g = &r.disk.partitions[0];
    assert_eq!((g.sectors, g.name.as_str(), g.type_guid, g.attrs), (32 * MIB, "grown ✓", types::LINUX_FS, types::attr::HIDDEN | types::attr::NO_AUTOMOUNT));
    assert_eq!(r.disk.gpt.as_ref().unwrap().entries[k].raw, keep_raw, "an entry the plan did not touch is kept byte for byte");
    let (data, total) = sexfs_get(&mut Window::new(&mut d, MIB, 32 * MIB).unwrap(), b"/kept");
    assert_eq!(data, b"still here after growing");
    assert_eq!(total, 32 * MIB / 8, "sexfs sees the grown partition");
    sgdisk_ok(&d, "grow");
}

#[test]
fn delete_and_format_in_old_space_wait_for_the_table() {
    let mut d = blank(128);
    let mut p = Plan::new(Disk::read(&mut d, "Stick").unwrap());
    p.push(Op::CreateTable).unwrap();
    let a = add(&mut p, 1, 40, types::BASIC_DATA, "old");
    p.push(Op::Format { slot: a, fs: FsKind::Fat32, label: "OLD".into() }).unwrap();
    apply(&mut d, &p, &mut rng(4), &mut quiet).unwrap();

    let mut p = Plan::new(Disk::read(&mut d, "Stick").unwrap());
    p.push(Op::Delete { slot: a }).unwrap();
    assert!(p.destructive());
    assert!(!p.confirms("") && !p.confirms("stick") && p.confirms("Stick"));
    assert!(p.confirmation().contains("Delete partition 1 \"old\" (40 MiB, FAT32); its data is lost. [destroys data]"), "{}", p.confirmation());
    assert!(p.confirmation().contains("Type the disk's name, \"Stick\", to apply."));
    let b = add(&mut p, 1, 60, types::BASIC_DATA, "new");
    assert_ne!(b, a, "a slot freed in this plan is not reused while others are free");
    p.push(Op::Format { slot: b, fs: FsKind::ExFat, label: "Neu".into() }).unwrap();
    let r = apply(&mut d, &p, &mut rng(5), &mut quiet).unwrap();
    assert_eq!((r.before_table, r.after_table), (0, 1), "the format overlaps the old partition, so it follows the table");
    assert_eq!(r.disk.partitions.len(), 1);
    assert_eq!(r.disk.partitions[0].content, Content::ExFat);
    sgdisk_ok(&d, "replace");
}

#[test]
fn exfat_volume_is_read_and_written_by_files() {
    let mut d = blank(64);
    let mut p = Plan::new(Disk::read(&mut d, "card").unwrap());
    p.push(Op::CreateTable).unwrap();
    let a = add(&mut p, 1, 60, types::BASIC_DATA, "card");
    p.push(Op::Format { slot: a, fs: FsKind::ExFat, label: "Kärtchen".into() }).unwrap();
    apply(&mut d, &p, &mut rng(6), &mut quiet).unwrap();
    let found = sex_volumes::scan(&mut Sv(&mut d)).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].volume.format(), sex_volumes::Format::ExFat);
    assert_eq!(found[0].volume.label(), "Kärtchen");
    let mut w = Window::new(&mut d, MIB, 60 * MIB).unwrap();
    let mut v = sex_volumes::exfat::ExFat::mount(&mut Sv(&mut w)).unwrap();
    let (total, free) = v.statfs(&mut Sv(&mut w)).unwrap();
    assert!(total > 59 << 20 && free > total - (64 << 10), "total {total} free {free}");
    v.mkdir(&mut Sv(&mut w), b"/Photos").unwrap();
    let big: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    v.create(&mut Sv(&mut w), b"/Photos/a.raw").unwrap();
    v.write(&mut Sv(&mut w), b"/Photos/a.raw", 0, &big).unwrap();
    v.sync(&mut Sv(&mut w)).unwrap();
    let mut v = sex_volumes::exfat::ExFat::mount(&mut Sv(&mut w)).unwrap();
    let mut back = vec![0u8; big.len()];
    assert_eq!(v.read(&mut Sv(&mut w), b"/photos/A.RAW", 0, &mut back).unwrap(), big.len(), "names match without case");
    assert_eq!(back, big);
    let (_, free2) = v.statfs(&mut Sv(&mut w)).unwrap();
    assert!(free2 < free - 290_000);
}

#[test]
fn encrypt_then_reopen_with_the_password() {
    let mut d = blank(64);
    let mut p = Plan::new(Disk::read(&mut d, "vault").unwrap());
    p.push(Op::CreateTable).unwrap();
    let a = add(&mut p, 1, 32, types::SEXCRYPT, "private");
    assert!(matches!(p.push(Op::Encrypt { slot: a, password: Secret(b"pw".to_vec()), cost: CHEAP, inner: Some((FsKind::Fat32, String::new())) }), Err(Refusal::TooSmall { fs: "FAT32", .. })));
    p.push(Op::Encrypt { slot: a, password: Secret(b"correct horse".to_vec()), cost: CHEAP, inner: Some((FsKind::Sexfs, String::new())) }).unwrap();
    assert!(p.steps()[2].text.contains("Encrypt partition 1 \"private\" (32 MiB) with a password (XTS-AES-256, Argon2id 0 MiB), then format it inside as sexfs."), "{}", p.steps()[2].text);
    let r = apply(&mut d, &p, &mut rng(7), &mut quiet).unwrap();
    assert_eq!(r.disk.partitions[0].content, Content::Encrypted);
    let mut w = Window::new(&mut d, MIB, 32 * MIB).unwrap();
    assert_eq!(CryptDev::open(&mut w, b"wrong horse").err(), Some(Error::WrongPassword));
    {
        let mut c = CryptDev::open(&mut w, b"correct horse").unwrap();
        assert_eq!(c.sectors(), 32 * MIB - 8, "one 4 KiB unit holds the header");
        sexfs_put(&mut c, b"/secret", b"the plaintext nobody should see on the disk");
    }
    // Reopen: a new unlock, a new mount.
    let mut c = CryptDev::open(&mut w, b"correct horse").unwrap();
    assert_eq!(sexfs_get(&mut c, b"/secret").0, b"the plaintext nobody should see on the disk");
    drop(c);
    drop(w);
    assert!(!d.0.windows(13).any(|x| x == b"the plaintext"), "nothing readable reaches the disk");
    // Growing an encrypted sexfs made by this plan is allowed; one read back is not known.
    let mut p = Plan::new(Disk::read(&mut d, "vault").unwrap());
    assert_eq!(p.push(Op::Resize { slot: a, sectors: 40 * MIB }), Err(Refusal::CannotGrow(Content::Encrypted)));
}

#[test]
fn encrypted_fat32_inside_reads_with_sex_volumes() {
    let mut d = blank(64);
    let mut p = Plan::new(Disk::read(&mut d, "x").unwrap());
    p.push(Op::CreateTable).unwrap();
    let a = add(&mut p, 1, 40, types::SEXCRYPT, "f");
    p.push(Op::Encrypt { slot: a, password: Secret(b"pw".to_vec()), cost: CHEAP, inner: Some((FsKind::Fat32, "INSIDE".into())) }).unwrap();
    apply(&mut d, &p, &mut rng(8), &mut quiet).unwrap();
    let mut w = Window::new(&mut d, MIB, 40 * MIB).unwrap();
    let mut c = CryptDev::open(&mut w, b"pw").unwrap();
    let v = sex_volumes::probe(&mut Sv(&mut c)).unwrap();
    assert_eq!((v.format(), v.label().as_str()), (sex_volumes::Format::Fat32, "INSIDE"));
}

#[test]
fn move_copies_into_free_space_and_refuses_unsafe_moves() {
    let mut d = blank(128);
    let mut p = Plan::new(Disk::read(&mut d, "m").unwrap());
    p.push(Op::CreateTable).unwrap();
    let a = add(&mut p, 1, 34, types::BASIC_DATA, "data");
    p.push(Op::Format { slot: a, fs: FsKind::Fat32, label: "MOVEME".into() }).unwrap();
    apply(&mut d, &p, &mut rng(9), &mut quiet).unwrap();
    {
        let mut w = Window::new(&mut d, MIB, 34 * MIB).unwrap();
        let mut v = sex_volumes::fat::Fat::mount(&mut Sv(&mut w)).unwrap();
        v.create(&mut Sv(&mut w), b"/f.bin").unwrap();
        v.write(&mut Sv(&mut w), b"/f.bin", 0, &[7u8; 100_000]).unwrap();
        v.sync(&mut Sv(&mut w)).unwrap();
    }
    let mut p = Plan::new(Disk::read(&mut d, "m").unwrap());
    assert_eq!(p.push(Op::Move { slot: a, first: 20 * MIB }), Err(Refusal::MoveOntoItself));
    assert_eq!(p.push(Op::Move { slot: a, first: 120 * MIB }), Err(Refusal::OutsideDisk));
    p.push(Op::Move { slot: a, first: 50 * MIB }).unwrap();
    assert_eq!(p.push(Op::Move { slot: a, first: 90 * MIB }), Err(Refusal::MoveAfterWrite));
    let n = add(&mut p, 1, 10, types::LINUX_FS, "into the old place");
    assert_eq!(p.push(Op::Move { slot: n, first: 90 * MIB }), Err(Refusal::MoveNewPartition));
    let r = apply(&mut d, &p, &mut rng(10), &mut quiet).unwrap();
    assert_eq!((r.before_table, r.after_table), (1, 1), "the copy goes first; the wipe of the old place follows the table");
    let moved = r.disk.partitions.iter().find(|x| x.slot == a).unwrap();
    assert_eq!((moved.first, moved.content), (50 * MIB, Content::Fat32));
    fsck_fat_ok(&d, 50 * MIB, 34 * MIB, "moved");
    let mut w = Window::new(&mut d, 50 * MIB, 34 * MIB).unwrap();
    let mut v = sex_volumes::fat::Fat::mount(&mut Sv(&mut w)).unwrap();
    let mut b = vec![0u8; 100_000];
    assert_eq!(v.read(&mut Sv(&mut w), b"/f.bin", 0, &mut b).unwrap(), 100_000);
    assert!(b.iter().all(|x| *x == 7));
    // A move into space a deletion in the same plan frees is refused.
    let mut p = Plan::new(r.disk.clone());
    p.push(Op::Delete { slot: n }).unwrap();
    assert_eq!(p.push(Op::Move { slot: a, first: MIB }), Err(Refusal::MoveIntoUsedSpace));
}

#[test]
fn a_disk_changed_since_the_plan_is_not_written() {
    let mut d = blank(64);
    let mut p = Plan::new(Disk::read(&mut d, "x").unwrap());
    p.push(Op::CreateTable).unwrap();
    d.0[1000] = 1; // someone wrote to the table area meanwhile
    let before = d.0.clone();
    assert_eq!(apply(&mut d, &p, &mut rng(11), &mut quiet).err(), Some(Error::Changed));
    assert!(d.0 == before);
}

#[test]
fn system_disks_and_partitions_are_left_alone() {
    let mut d = blank(64);
    let mut g = sex_disk::gpt::Gpt::new_empty(d.0.len() as u64 / 512, sex_disk::gpt::Guid::random([1; 16])).unwrap();
    g.add(types::SEXOS_STORE, sex_disk::gpt::Guid::random([2; 16]), MIB, 20 * MIB, "SexOS").unwrap();
    g.write(&mut sex_disk::MemDev(&mut d.0[..])).unwrap();
    let mut p = Plan::new(Disk::read(&mut d, "system").unwrap());
    assert!(p.disk().is_system());
    assert_eq!(p.push(Op::CreateTable), Err(Refusal::SystemDisk));
    assert_eq!(p.push(Op::Delete { slot: 0 }), Err(Refusal::SystemPartition(0)));
    assert_eq!(p.push(Op::Format { slot: 0, fs: FsKind::Sexfs, label: String::new() }), Err(Refusal::SystemPartition(0)));
    // Free space next to it may still be used.
    add(&mut p, 30, 10, types::BASIC_DATA, "spare");
}

#[test]
fn sgdisk_made_table_with_off_grid_partition_is_respected() {
    if std::process::Command::new("sgdisk").arg("--version").output().is_err() { return; }
    let dir = scratch("foreign");
    let img = dir.join("d.img");
    std::fs::write(&img, vec![0u8; 64 << 20]).unwrap();
    // An odd start (sector 2049) and an attribute other tools set.
    let ok = std::process::Command::new("sgdisk").args(["-a", "1", "-n1:2049:+10M", "-t1:8300", "-c1:linux root", "-A1:set:2"]).arg(&img).output().unwrap();
    assert!(ok.status.success());
    let mut d = MemDisk(std::fs::read(&img).unwrap());
    std::fs::remove_dir_all(&dir).ok();
    let disk = Disk::read(&mut d, "foreign").unwrap();
    let p0 = &disk.partitions[0];
    assert_eq!((p0.first, p0.name.as_str(), p0.attrs, p0.content), (2049, "linux root", types::attr::LEGACY_BOOT, Content::Empty));
    let mut p = Plan::new(disk);
    // Its length is off the grid too; a resize to whole MiB keeps the odd start.
    p.push(Op::Resize { slot: 0, sectors: 12 * MIB }).unwrap();
    let r = apply(&mut d, &p, &mut rng(12), &mut quiet).unwrap();
    assert_eq!((r.disk.partitions[0].first, r.disk.partitions[0].sectors, r.disk.partitions[0].attrs), (2049, 12 * MIB, types::attr::LEGACY_BOOT));
    sgdisk_ok(&d, "foreign-after");
}

/// Power cut after every possible write: the disk always reads as the old layout or the
/// new one, and what was there is intact while the old layout still shows.
#[test]
fn a_power_cut_at_any_write_leaves_old_or_new_layout() {
    let mut d = blank(48);
    let mut p = Plan::new(Disk::read(&mut d, "c").unwrap());
    p.push(Op::CreateTable).unwrap();
    let a = add(&mut p, 1, 8, types::SEXFS, "keep");
    p.push(Op::Format { slot: a, fs: FsKind::Sexfs, label: String::new() }).unwrap();
    let doomed = add(&mut p, 10, 8, types::LINUX_FS, "doomed");
    apply(&mut d, &p, &mut rng(13), &mut quiet).unwrap();
    sexfs_put(&mut Window::new(&mut d, MIB, 8 * MIB).unwrap(), b"/x", b"survives");
    let old = Disk::read(&mut d, "c").unwrap();

    let mut p = Plan::new(old.clone());
    p.push(Op::Delete { slot: doomed }).unwrap();
    p.push(Op::Resize { slot: a, sectors: 16 * MIB }).unwrap();
    let n = add(&mut p, 20, 8, types::SEXFS, "new");
    p.push(Op::Format { slot: n, fs: FsKind::Sexfs, label: String::new() }).unwrap();
    let mut probe = d.clone_disk();
    let mut cut = Cut { d: &mut probe, left: usize::MAX, writes: 0 };
    let new = apply(&mut cut, &p, &mut rng(14), &mut quiet).unwrap().disk;
    let total = cut.writes;
    assert!(total > 5, "{total}");
    let shape = |x: &Disk| x.partitions.iter().map(|q| (q.slot, q.first, q.sectors)).collect::<Vec<_>>();
    for k in 0..total {
        let mut t = d.clone_disk();
        let r = apply(&mut Cut { d: &mut t, left: k, writes: 0 }, &p, &mut rng(14), &mut quiet);
        assert!(r.is_err(), "cut at {k}");
        let now = Disk::read(&mut t, "c").unwrap();
        let s = shape(&now);
        assert!(s == shape(&old) || s == shape(&new), "cut at write {k} of {total}: {s:?}");
        let keep = now.partitions.iter().find(|q| q.slot == a).unwrap();
        assert_eq!(sexfs_get(&mut Window::new(&mut t, MIB, keep.sectors).unwrap(), b"/x").0, b"survives", "cut at {k}");
    }
}

trait CloneDisk { fn clone_disk(&self) -> MemDisk; }
impl CloneDisk for MemDisk { fn clone_disk(&self) -> MemDisk { MemDisk(self.0.clone()) } }
