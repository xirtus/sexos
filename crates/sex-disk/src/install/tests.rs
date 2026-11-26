use super::*;
use crate::MemDev;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn tool(name: &str) -> bool { Command::new(name).arg("--version").output().is_ok() }
pub(crate) fn run(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{:?}: {}{}", cmd, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}
pub(crate) fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sexdisk-install-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
pub(crate) fn seeds(n: u8) -> Seeds { Seeds(core::array::from_fn(|i| [n.wrapping_add(i as u8 * 17); 16])) }

/// A boot image like make_boot_image.py builds: FAT, Limine, its menu, a kernel.
pub(crate) fn image(dir: &Path) -> Vec<u8> {
    let img = dir.join("boot.img");
    std::fs::write(&img, vec![0u8; 24576 * 512]).unwrap();
    run(Command::new("mformat").args(["-i"]).arg(&img).args(["-T", "24576", "-h", "64", "-s", "32", "-v", "SEXOS", "::"]));
    let cfg = dir.join("limine.cfg");
    std::fs::write(&cfg, "TIMEOUT=3\nSERIAL=yes\n\n:SexOS\n    PROTOCOL=limine\n    KERNEL_PATH=boot:///sexos-kernel\n    MODULE_PATH=boot:///servers/sexfiles\n    RESOLUTION=1280x800x32\n").unwrap();
    let efi = dir.join("BOOTX64.EFI");
    std::fs::write(&efi, (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect::<Vec<_>>()).unwrap();
    let kernel = dir.join("sexos-kernel");
    std::fs::write(&kernel, b"kernel bytes").unwrap();
    for d in ["::/EFI", "::/EFI/BOOT"] { run(Command::new("mmd").arg("-i").arg(&img).arg(d)); }
    run(Command::new("mcopy").arg("-i").arg(&img).arg(&efi).arg("::/EFI/BOOT/BOOTX64.EFI"));
    run(Command::new("mcopy").arg("-i").arg(&img).arg(&cfg).arg("::/limine.cfg"));
    run(Command::new("mcopy").arg("-i").arg(&img).arg(&kernel).arg("::/sexos-kernel"));
    std::fs::read(&img).unwrap()
}

/// A disk as Windows leaves it: GPT, a FAT32 ESP with the Windows boot manager and a
/// fallback loader, a "Windows" partition full of data, and free space after it.
pub(crate) fn windows_disk(dir: &Path) -> (PathBuf, u64, u64) {
    let disk = dir.join("disk.img");
    std::fs::write(&disk, vec![0u8; 4 << 30 >> 1]).unwrap(); // 2 GiB
    run(Command::new("sgdisk").args(["-n1:2048:+100M", "-t1:ef00", "-c1:EFI system partition", "-n2:0:+50M", "-t2:0700", "-c2:Basic data partition"]).arg(&disk));
    let esp_off = 2048u64 * 512;
    let target = format!("{}@@{}", disk.display(), esp_off);
    run(Command::new("mformat").args(["-i", &target, "-F", "-T", &(100 * 2048).to_string(), "-v", "ESP", "::"]));
    let boot = dir.join("bootmgfw.efi");
    std::fs::write(&boot, b"windows boot manager").unwrap();
    for d in ["::/EFI", "::/EFI/Microsoft", "::/EFI/Microsoft/Boot", "::/EFI/Boot"] { run(Command::new("mmd").args(["-i", &target, d])); }
    run(Command::new("mcopy").args(["-i", &target]).arg(&boot).arg("::/EFI/Microsoft/Boot/bootmgfw.efi"));
    run(Command::new("mcopy").args(["-i", &target]).arg(&boot).arg("::/EFI/Boot/bootx64.efi"));
    // The Windows partition: recognisable bytes all through it.
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(&disk).unwrap();
    f.seek(SeekFrom::Start((2048 + 100 * 2048) * 512)).unwrap();
    f.write_all(&(0..50 * 1024 * 1024usize).map(|i| (i % 253) as u8 ^ 0x5A).collect::<Vec<_>>()).unwrap();
    (disk, 2048, 100 * 2048)
}

#[test]
fn installs_beside_windows_without_touching_it() {
    if !tool("sgdisk") || !tool("mformat") { return; }
    let dir = scratch("win");
    let img = image(&dir);
    let (path, esp_first, esp_sectors) = windows_disk(&dir);
    let before = std::fs::read(&path).unwrap();
    let mut d = MemDev(before.clone());
    let mut p = plan(&mut d, Mode::FreeSpace(None), &seeds(1)).unwrap();
    assert!(!p.new_table && !p.new_esp && !p.fallback_free);
    assert_eq!(p.esp, Extent { first: esp_first, sectors: esp_sectors });
    assert_eq!(p.others, vec![Other { name: "Windows".into(), loader: "/EFI/Microsoft/Boot/bootmgfw.efi".into() }]);
    let mut steps = Vec::new();
    let done = apply(&mut d, &mut p, &mut MemDev(img.clone()), &mut |s| steps.push(String::from(s))).unwrap();
    std::fs::write(&path, &d.0).unwrap();

    // The table is valid and keeps both Windows partitions where they were.
    let v = run(Command::new("sgdisk").arg("-v").arg(&path));
    assert!(v.contains("No problems found"), "{v}");
    let table = run(Command::new("sgdisk").arg("-p").arg(&path));
    assert!(table.contains("SexOS system A") && table.contains("Basic data partition"), "{table}");
    // Not one byte of the Windows partition changed.
    let win = (2048 + 100 * 2048) * 512..(2048 + 150 * 2048) * 512;
    assert!(d.0[win.clone()] == before[win], "Windows partition changed");
    // The ESP is clean, keeps Windows' files and has ours.
    let esp = format!("{}@@{}", path.display(), esp_first * 512);
    assert_eq!(run(Command::new("mtype").args(["-i", &esp, "::/EFI/Microsoft/Boot/bootmgfw.efi"])), "windows boot manager");
    assert_eq!(run(Command::new("mtype").args(["-i", &esp, "::/EFI/Boot/bootx64.efi"])), "windows boot manager");
    let esp_bytes = dir.join("esp.img");
    std::fs::write(&esp_bytes, &d.0[esp_first as usize * 512..(esp_first + esp_sectors) as usize * 512]).unwrap();
    if Path::new("/sbin/fsck.vfat").exists() { run(Command::new("/sbin/fsck.vfat").arg("-n").arg(&esp_bytes)); }
    let menu = run(Command::new("mtype").args(["-i", &esp, "::/EFI/BOOT/LIMINE.CFG"]));
    assert!(menu.starts_with(MENU_MARK));
    assert!(menu.contains(&format!("KERNEL_PATH=guid://{}/sexos-kernel", std::str::from_utf8(&p.slot_a_guid.text()).unwrap())), "{menu}");
    assert!(menu.contains(":Windows\n    PROTOCOL=efi_chainload\n    IMAGE_PATH=boot:///EFI/Microsoft/Boot/bootmgfw.efi"), "{menu}");
    assert_eq!(menu.len(), MENU_BYTES);
    // The menu's sectors are where apply said, for in-place rewrites.
    let at = done.menu.first_lba as usize * 512;
    assert_eq!(&d.0[at..at + MENU_MARK.len()], MENU_MARK.as_bytes());
    // Slot A holds the image; the store starts cleared; the probe now finds the store.
    let a = p.slot_a.first as usize * 512;
    assert!(d.0[a..a + img.len()] == img[..]);
    assert_eq!(probe::classify(&mut d).unwrap(), Layout::Sexos { first: p.store.first, sectors: p.store.sectors });
    // Installing again is refused: SexOS is already there.
    assert_eq!(plan(&mut d, Mode::FreeSpace(None), &seeds(2)).unwrap_err(), Error::Exists);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn whole_disk_and_sizes() {
    if !tool("sgdisk") || !tool("mformat") { return; }
    let dir = scratch("whole");
    let img = image(&dir);
    let mut d = MemDev(vec![0u8; 2 << 30]);
    let mut p = plan(&mut d, Mode::WholeDisk, &seeds(3)).unwrap();
    assert!(p.new_table && p.new_esp && p.fallback_free && p.others.is_empty());
    apply(&mut d, &mut p, &mut MemDev(img), &mut |_| {}).unwrap();
    let path = dir.join("whole.img");
    std::fs::write(&path, &d.0).unwrap();
    assert!(run(Command::new("sgdisk").arg("-v").arg(&path)).contains("No problems found"));
    let esp = format!("{}@@{}", path.display(), p.esp.first * 512);
    assert!(run(Command::new("mdir").args(["-i", &esp, "::/EFI/BOOT"])).contains("BOOTX64"));
    // A size choice; too little space is refused before anything is written.
    let mut small = MemDev(vec![0u8; 1 << 30]);
    assert_eq!(plan(&mut small, Mode::FreeSpace(None), &seeds(4)).unwrap_err(), Error::NoSpace);
    let mut e = MemDev(vec![0u8; 3 << 30]);
    let p = plan(&mut e, Mode::FreeSpace(Some(1200 * 2048)), &seeds(5)).unwrap();
    assert_eq!(p.store.sectors, 1200 * 2048);
    assert!(e.0.iter().all(|&b| b == 0), "planning wrote to the disk");
    // Another system's disk is never taken whole.
    let (win, _, _) = windows_disk(&dir);
    let mut w = MemDev(std::fs::read(&win).unwrap());
    assert_eq!(plan(&mut w, Mode::WholeDisk, &seeds(6)).unwrap_err(), Error::Exists);
    // A damaged primary table (readable only from its backup) is refused before any write.
    w.0[512 + 20] ^= 0xFF;
    let copy = w.0.clone();
    assert_eq!(plan(&mut w, Mode::FreeSpace(None), &seeds(7)).unwrap_err(), Error::Corrupt);
    assert!(w.0 == copy);
    std::fs::remove_dir_all(&dir).ok();
}

/// Pure-Rust FAT32 boot image (runs on any host without mformat/sgdisk).
fn pure_boot_image() -> Vec<u8> {
    let mut img = MemDev(vec![0u8; 40 << 20]); // 40 MiB FAT32
    fat::format_fat32(&mut img, b"SEXOS      ").unwrap();
    let f = Fat::open(&mut img).unwrap();
    f.mkdir_p(&mut img, "/EFI/BOOT").unwrap();
    let cfg = b"TIMEOUT=3\nSERIAL=yes\n\n:SexOS\n    PROTOCOL=limine\n    KERNEL_PATH=boot:///KERNEL\n";
    let efi: Vec<u8> = (0..65_536u32).map(|i| (i * 13 % 251) as u8).collect();
    f.write_file(&mut img, "/LIMINE.CFG", cfg).unwrap();
    f.write_file(&mut img, "/EFI/BOOT/BOOTX64.EFI", &efi).unwrap();
    f.write_file(&mut img, "/KERNEL", b"sexos-kernel-bytes").unwrap();
    img.0
}

struct FourOsLayout {
    esp: (u64, u64),
    msr: (u64, u64),
    windows: (u64, u64),
    void_os: (u64, u64),
    garuda: (u64, u64),
    tiny: (u64, u64),
}

/// Builds a simulated multi-boot NVMe GPT disk:
/// #1 ESP (40 MiB) + #2 MSR (16 MiB) + #3 Windows (64 MiB) + #4 Void (64 MiB)
/// + #5 Garuda (1700 MiB) + #6 Tiny Linux (64 MiB).
fn four_os_disk() -> (MemDev<Vec<u8>>, FourOsLayout) {
    let total_mib = 2048usize; // 2.0 GiB total
    let mut d = MemDev(vec![0u8; total_mib << 20]);
    let total = d.sectors();
    let mut g = Gpt::new_empty(total, Guid::random([0x11; 16])).unwrap();

    let mut cur = 2048u64; // 1 MiB
    let mut alloc_part = |mib: u64| -> (u64, u64) {
        let first = cur;
        let sectors = mib * 2048;
        cur += sectors;
        (first, sectors)
    };

    let esp = alloc_part(40);
    let msr = alloc_part(16);
    let windows = alloc_part(64);
    let void_os = alloc_part(64);
    let garuda = alloc_part(1700);
    let tiny = alloc_part(64);

    g.add(TYPE_ESP, Guid::random([0x21; 16]), esp.0, esp.1, "EFI system partition").unwrap();
    g.add(TYPE_MSR, Guid::random([0x22; 16]), msr.0, msr.1, "Microsoft reserved").unwrap();
    g.add(TYPE_BASIC_DATA, Guid::random([0x23; 16]), windows.0, windows.1, "Windows").unwrap();
    g.add(TYPE_LINUX_FS, Guid::random([0x24; 16]), void_os.0, void_os.1, "Void Linux").unwrap();
    g.add(TYPE_LINUX_FS, Guid::random([0x25; 16]), garuda.0, garuda.1, "Garuda Linux").unwrap();
    g.add(TYPE_LINUX_FS, Guid::random([0x26; 16]), tiny.0, tiny.1, "Tiny Boot").unwrap();
    g.write(&mut d).unwrap();

    // Format ESP with Windows + Void loaders (8.3 FAT names).
    {
        let mut w = Window::new(&mut d, esp.0, esp.1).unwrap();
        fat::format_fat32(&mut w, b"ESP        ").unwrap();
        let f = Fat::open(&mut w).unwrap();
        f.mkdir_p(&mut w, "/EFI/WIN").unwrap();
        f.write_file(&mut w, "/EFI/WIN/BOOTMGFW.EFI", b"windows-bootmgfw").unwrap();
        f.mkdir_p(&mut w, "/EFI/VOID").unwrap();
        f.write_file(&mut w, "/EFI/VOID/GRUBX64.EFI", b"void-grub").unwrap();
    }

    // Fill MSR, Windows, Void, Garuda, and Tiny with distinct recognizable patterns.
    let fill_part = |buf: &mut [u8], (first, sectors): (u64, u64), tag: u8| {
        let start = first as usize * SECTOR;
        let len = sectors as usize * SECTOR;
        for (i, b) in buf[start..start + len].iter_mut().enumerate() {
            *b = ((i as u8).wrapping_mul(31)) ^ tag;
        }
    };
    fill_part(&mut d.0, msr, 0xA1);
    fill_part(&mut d.0, windows, 0xB2);
    fill_part(&mut d.0, void_os, 0xC3);
    fill_part(&mut d.0, garuda, 0xD4);
    fill_part(&mut d.0, tiny, 0xE5);

    (d, FourOsLayout { esp, msr, windows, void_os, garuda, tiny })
}

#[test]
fn replace_partition_refuses_esp_msr_tiny_and_replaces_only_garuda() {
    let (mut d, lay) = four_os_disk();
    let before = d.0.clone();
    let before_gpt = Gpt::read(&mut d).unwrap();

    // 1. list_partitions accurately classifies all 6 partitions.
    let (total_sectors, parts, _free) = list_partitions(&mut d).unwrap();
    assert_eq!(total_sectors, d.sectors());
    assert_eq!(parts.len(), 6);
    assert_eq!(parts[0].kind, PartKind::Esp);
    assert!(!parts[0].replaceable);
    assert_eq!(parts[1].kind, PartKind::Msr);
    assert!(!parts[1].replaceable);
    assert_eq!(parts[2].kind, PartKind::Windows);
    assert!(!parts[2].replaceable); // 64 MiB < 1.5 GiB
    assert_eq!(parts[3].kind, PartKind::Linux);
    assert!(!parts[3].replaceable); // 64 MiB < 1.5 GiB
    assert_eq!(parts[4].kind, PartKind::Linux);
    assert_eq!(parts[4].name, "Garuda Linux");
    assert!(parts[4].replaceable);
    assert_eq!(parts[5].kind, PartKind::Linux);
    assert!(!parts[5].replaceable);

    // 2. Protected & invalid targets are refused with zero disk writes.
    assert_eq!(plan(&mut d, Mode::ReplacePartition(0), &seeds(10)).unwrap_err(), Error::Invalid);
    assert_eq!(plan(&mut d, Mode::ReplacePartition(1), &seeds(11)).unwrap_err(), Error::Invalid);
    assert_eq!(plan(&mut d, Mode::ReplacePartition(5), &seeds(12)).unwrap_err(), Error::NoSpace);
    assert_eq!(plan(&mut d, Mode::ReplacePartition(99), &seeds(13)).unwrap_err(), Error::NotFound);
    assert_eq!(d.0, before, "refused plans must not write a single byte");

    // 3. Plan replacing ONLY Partition #5 (index 4: Garuda Linux).
    let mut p = plan(&mut d, Mode::ReplacePartition(4), &seeds(14)).unwrap();
    assert!(!p.new_table && !p.new_esp);
    assert_eq!(p.esp, Extent { first: lay.esp.0, sectors: lay.esp.1 });
    let rep = p.replaced.as_ref().expect("replaced metadata present");
    assert_eq!(rep.index, 4);
    assert_eq!(rep.name, "Garuda Linux");
    assert_eq!(p.untouched.len(), 5);

    // Verify Slot A, Slot B, and Store lie strictly inside Garuda's LBA bounds and are 1 MiB aligned.
    let garuda_first = lay.garuda.0;
    let garuda_last = lay.garuda.0 + lay.garuda.1 - 1;
    assert_eq!(p.slot_a.first % ALIGN, 0);
    assert_eq!(p.slot_b.first % ALIGN, 0);
    assert_eq!(p.store.first % ALIGN, 0);
    assert!(p.slot_a.first >= garuda_first);
    assert_eq!(p.slot_b.first, p.slot_a.first + SLOT_SECTORS);
    assert_eq!(p.store.first, p.slot_b.first + SLOT_SECTORS);
    assert!(p.store.first + p.store.sectors - 1 <= garuda_last);

    // Verify describe() clearly states replaced partition and untouched partitions.
    let lines = describe(&p);
    assert!(lines.iter().any(|l| l.contains("REPLACES ONLY Partition #5 (Garuda Linux")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("UNTOUCHED: #1 EFI system partition")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("UNTOUCHED: #3 Windows")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("UNTOUCHED: #4 Void Linux")), "{lines:?}");

    // 4. Execute atomic install.
    let img = pure_boot_image();
    let done = apply(&mut d, &mut p, &mut MemDev(img.clone()), &mut |_| {}).unwrap();
    assert_eq!(done.image_sectors as usize * SECTOR, img.len());

    // 5. Verify GPT entries for ESP, MSR, Windows, Void, and Tiny are byte-for-byte identical.
    let after_gpt = Gpt::read(&mut d).unwrap();
    after_gpt.check().unwrap();
    for keep_idx in [0usize, 1, 2, 3, 5] {
        assert_eq!(
            after_gpt.entries[keep_idx].raw, before_gpt.entries[keep_idx].raw,
            "GPT entry {keep_idx} was modified!"
        );
    }

    // 6. Verify every single on-disk data byte of MSR, Windows, Void, and Tiny is 100% untouched.
    let check_untouched = |name: &str, (first, sectors): (u64, u64)| {
        let start = first as usize * SECTOR;
        let end = start + sectors as usize * SECTOR;
        assert!(d.0[start..end] == before[start..end], "{name} partition data changed!");
    };
    check_untouched("MSR", lay.msr);
    check_untouched("Windows", lay.windows);
    check_untouched("Void Linux", lay.void_os);
    check_untouched("Tiny Boot", lay.tiny);

    // 7. Verify ESP kept Windows & Void bootloaders and gained SexOS Limine + menu.
    {
        let mut w = Window::new(&mut d, lay.esp.0, lay.esp.1).unwrap();
        let f = Fat::open(&mut w).unwrap();
        assert_eq!(f.read_file(&mut w, "/EFI/WIN/BOOTMGFW.EFI").unwrap(), b"windows-bootmgfw");
        assert_eq!(f.read_file(&mut w, "/EFI/VOID/GRUBX64.EFI").unwrap(), b"void-grub");
        let menu = f.read_file(&mut w, MENU_PATH).unwrap();
        assert!(menu.starts_with(MENU_MARK.as_bytes()));
        let menu_str = core::str::from_utf8(&menu).unwrap();
        assert!(menu_str.contains(":WIN"), "{menu_str}");
        assert!(menu_str.contains(":VOID"), "{menu_str}");
        assert!(f.find(&mut w, LOADER_PATH).is_ok());
    }

    // 8. Verify Slot A holds the boot image and probe::classify finds SexOS Store.
    let slot_a_off = p.slot_a.first as usize * SECTOR;
    assert!(d.0[slot_a_off..slot_a_off + img.len()] == img[..]);
    assert_eq!(
        probe::classify(&mut d).unwrap(),
        Layout::Sexos { first: p.store.first, sectors: p.store.sectors }
    );
}

