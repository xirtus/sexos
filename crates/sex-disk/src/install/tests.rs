use super::*;
use crate::MemDev;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> bool { Command::new(name).arg("--version").output().is_ok() }
fn run(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{:?}: {}{}", cmd, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sexdisk-install-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn seeds(n: u8) -> Seeds { Seeds(core::array::from_fn(|i| [n.wrapping_add(i as u8 * 17); 16])) }

/// A boot image like make_boot_image.py builds: FAT, Limine, its menu, a kernel.
fn image(dir: &Path) -> Vec<u8> {
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
fn windows_disk(dir: &Path) -> (PathBuf, u64, u64) {
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
