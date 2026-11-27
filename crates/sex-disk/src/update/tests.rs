use super::*;
use crate::install::tests::{image, run, scratch, seeds, tool, windows_disk};
use crate::install::{self, Mode};
use crate::MemDev;
use std::process::Command;

fn key() -> ed25519_compact::KeyPair { ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([7u8; 32])) }
fn index_for(img: &[u8], version: u64) -> Vec<u8> {
    let sha: [u8; 32] = Sha256::digest(img).into();
    let hex: String = sha.iter().map(|b| format!("{b:02x}")).collect();
    format!("# SexOS release\nversion={version}\nimage=https://example.invalid/boot.img\nsha256={hex}\nbytes={}\n", img.len()).into_bytes()
}
fn sign(index: &[u8]) -> Vec<u8> {
    let s = key().sk.sign(index, None);
    s.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>().into_bytes()
}

#[test]
fn state_line_round_trip_and_steps() {
    let st = State { version: 3, candidate: 4, current: Slot::B, trial: Trial::Pending, prev: 3 };
    let mut menu = String::from(MENU_MARK);
    menu.push('\n');
    menu.push_str(&st.line());
    assert_eq!(State::parse(menu.as_bytes()).unwrap(), st);
    // The new slot starts: it may not become the default yet.
    let started = st.on_start(Slot::B).unwrap();
    assert_eq!((started.trial, started.default_entry()), (Trial::Started, 3));
    // It answers: it is the default from now on, and its version counts.
    let ok = started.on_healthy(Slot::B).unwrap();
    assert_eq!((ok.trial, ok.version, ok.default_entry()), (Trial::None, 4, 1));
    assert_eq!(ok.on_start(Slot::B), None);
    // It never answered, and the previous system started instead.
    let failed = started.on_start(Slot::A).unwrap();
    assert_eq!((failed.trial, failed.version, failed.default_entry()), (Trial::Failed, 3, 3));
    assert_eq!(failed.on_healthy(Slot::A), None);
}

#[test]
fn index_needs_the_release_key() {
    let img = vec![0x42u8; 4096];
    let idx = index_for(&img, 9);
    let sig = sign(&idx);
    let pk: [u8; 32] = *key().pk;
    let got = verify_index(&idx, &sig, &pk).unwrap();
    assert_eq!((got.version, got.bytes), (9, 4096));
    // Any change to the index, or another key, fails.
    let mut bent = idx.clone();
    *bent.iter_mut().find(|b| **b == b'9').unwrap() = b'8';
    assert_eq!(verify_index(&bent, &sig, &pk).unwrap_err(), Error::BadSignature);
    let other = *ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([8u8; 32])).pk;
    assert_eq!(verify_index(&idx, &sig, &other).unwrap_err(), Error::BadSignature);
}

#[test]
fn update_tries_the_other_slot_once() {
    if !tool("sgdisk") || !tool("mformat") { return; }
    let dir = scratch("update");
    let img = image(&dir);
    let (path, _, _) = windows_disk(&dir);
    let mut d = MemDev(std::fs::read(&path).unwrap());
    let mut p = install::plan(&mut d, Mode::FreeSpace(None), &seeds(1)).unwrap();
    install::apply(&mut d, &mut p, &mut MemDev(img.clone()), &mut |_| {}).unwrap();
    let sys = System::find(&mut d).unwrap();
    assert_eq!((sys.a.1, sys.b.1), (p.slot_a_guid, p.slot_b_guid));
    let menu = sys.read_menu(&mut d).unwrap();
    assert_eq!(State::parse(&menu).unwrap(), State::fresh());
    assert!(std::str::from_utf8(&menu).unwrap().contains("DEFAULT_ENTRY=1\n"));

    // A new image: same layout, a new kernel.
    let mut new_img = img.clone();
    let at = new_img.windows(12).position(|w| w == b"kernel bytes").unwrap();
    new_img[at..at + 12].copy_from_slice(b"KERNEL BYTES");
    let idx = index_for(&new_img, 5);
    let index = verify_index(&idx, &sign(&idx), &key().pk).unwrap();
    // An image that is not the one the index names is refused before any write.
    let before = d.0.clone();
    assert_eq!(apply(&mut d, &sys, p.slot_a_guid, &index, &mut MemDev(img.clone()), &mut |_| {}).unwrap_err(), Error::BadSignature);
    assert!(d.0 == before);
    // Running from A, the update goes to B.
    let slot = apply(&mut d, &sys, p.slot_a_guid, &index, &mut MemDev(new_img.clone()), &mut |_| {}).unwrap();
    assert_eq!(slot, Slot::B);
    let b = sys.b.0.first as usize * 512;
    assert!(d.0[b..b + new_img.len()] == new_img[..]);
    let menu = sys.read_menu(&mut d).unwrap();
    let text = std::str::from_utf8(&menu).unwrap();
    let st = State::parse(&menu).unwrap();
    assert_eq!(st, State { version: 0, candidate: 5, current: Slot::B, trial: Trial::Pending, prev: 2 });
    let a_text = std::str::from_utf8(&p.slot_a_guid.text()).unwrap().to_string();
    let b_text = std::str::from_utf8(&p.slot_b_guid.text()).unwrap().to_string();
    assert!(text.contains(&format!(":SexOS\n    PROTOCOL=limine\n    KERNEL_PATH=guid://{b_text}/sexos-kernel")), "{text}");
    assert!(text.contains(&format!("{PREVIOUS_TITLE}\n    PROTOCOL=limine\n    KERNEL_PATH=guid://{a_text}/sexos-kernel")), "{text}");
    assert!(text.contains(":Windows\n    PROTOCOL=efi_chainload"), "{text}");
    assert_eq!(menu.len(), MENU_BYTES);
    // The ESP's FAT still agrees: the menu file reads back as written.
    let file = dir.join("updated.img");
    std::fs::write(&file, &d.0).unwrap();
    let esp = format!("{}@@{}", file.display(), sys.esp.first * 512);
    assert_eq!(run(Command::new("mtype").args(["-i", &esp, "::/EFI/BOOT/LIMINE.CFG"])).as_bytes(), &menu[..]);

    // Before B has had its try, another update is refused: the menu would start a slot
    // being rewritten.
    let idx9 = index_for(&img, 9);
    let index9 = verify_index(&idx9, &sign(&idx9), &key().pk).unwrap();
    assert_eq!(apply(&mut d, &sys, p.slot_a_guid, &index9, &mut MemDev(new_img.clone()), &mut |_| {}).unwrap_err(), Error::Pending);

    // B starts: the trial begins, and the default falls back to A until B answers.
    let started = st.on_start(Slot::B).unwrap();
    sys.write_menu(&mut d, &restate(&menu, &started).unwrap()).unwrap();
    let m = sys.read_menu(&mut d).unwrap();
    assert!(std::str::from_utf8(&m).unwrap().contains("DEFAULT_ENTRY=2\n"));
    assert_eq!(State::parse(&m).unwrap().trial, Trial::Started);
    let ok = State::parse(&m).unwrap().on_healthy(Slot::B).unwrap();
    sys.write_menu(&mut d, &restate(&m, &ok).unwrap()).unwrap();
    let m = sys.read_menu(&mut d).unwrap();
    assert!(std::str::from_utf8(&m).unwrap().contains("DEFAULT_ENTRY=1\n"));
    assert_eq!(State::parse(&m).unwrap().version, 5);

    // The same release again is refused (no rollback), from either slot.
    assert_eq!(apply(&mut d, &sys, p.slot_b_guid, &index, &mut MemDev(new_img.clone()), &mut |_| {}).unwrap_err(), Error::Older);
    // A newer one from B goes to A, and B is now the previous system.
    let idx6 = index_for(&img, 6);
    let index6 = verify_index(&idx6, &sign(&idx6), &key().pk).unwrap();
    assert_eq!(apply(&mut d, &sys, p.slot_b_guid, &index6, &mut MemDev(img.clone()), &mut |_| {}).unwrap(), Slot::A);
    let m = sys.read_menu(&mut d).unwrap();
    let text = std::str::from_utf8(&m).unwrap();
    assert!(text.contains(&format!("{PREVIOUS_TITLE}\n    PROTOCOL=limine\n    KERNEL_PATH=guid://{b_text}/sexos-kernel")), "{text}");
    assert_eq!(text.matches(PREVIOUS_TITLE).count(), 1);
    assert_eq!(text.matches(":Windows").count(), 1);
    std::fs::write(&file, &d.0).unwrap();
    assert!(run(Command::new("sgdisk").arg("-v").arg(&file)).contains("No problems found"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn legacy_16k_menu_upgrades_to_64k_on_large_dual_slot_update() {
    use crate::fat::{self, Fat};
    use crate::install::pad_to;

    // Build a realistic ~10 KiB limine.cfg (like SexOS with many MODULE_PATH entries)
    // where Slot B + Slot A fallback (~20 KiB) exceeds the old 16 KiB MENU_BYTES limit.
    let mut big_cfg = String::from("TIMEOUT=3\nSERIAL=yes\n\n:SexOS\n    PROTOCOL=limine\n    KERNEL_PATH=boot:///KERNEL\n");
    for i in 0..110 {
        big_cfg.push_str(&format!("    MODULE_PATH=boot:///SERVERS/MOD{i:03}    # service module {i:03}\n"));
    }
    assert!(big_cfg.len() > 6_000 && big_cfg.len() < 10_000, "cfg len={}", big_cfg.len());

    // Pure-Rust FAT32 boot image (40 MiB).
    let make_img = |cfg: &str, kernel_tag: &[u8]| -> Vec<u8> {
        let mut img = MemDev(vec![0u8; 40 << 20]);
        fat::format_fat32(&mut img, b"SEXOS      ").unwrap();
        let f = Fat::open(&mut img).unwrap();
        f.mkdir_p(&mut img, "/EFI/BOOT").unwrap();
        f.write_file(&mut img, "/LIMINE.CFG", cfg.as_bytes()).unwrap();
        f.write_file(&mut img, "/EFI/BOOT/BOOTX64.EFI", &[0x5Au8; 4096]).unwrap();
        f.write_file(&mut img, "/KERNEL", kernel_tag).unwrap();
        img.0
    };
    let img_v1 = make_img(&big_cfg, b"kernel-v1");
    let img_v2 = make_img(&big_cfg, b"kernel-v2");

    // Install on a fresh 2 GiB virtual disk.
    let mut d = MemDev(vec![0u8; 2 << 30]);
    let mut p = install::plan(&mut d, Mode::WholeDisk, &seeds(42)).unwrap();
    install::apply(&mut d, &mut p, &mut MemDev(img_v1.clone()), &mut |_| {}).unwrap();

    // Downgrade the ESP's LIMINE.CFG to a legacy 16 KiB file to simulate an already-installed disk.
    {
        let mut esp = Window::new(&mut d, p.esp.first, p.esp.sectors).unwrap();
        let f = Fat::open(&mut esp).unwrap();
        let cur = f.read_file(&mut esp, MENU_PATH).unwrap();
        assert_eq!(cur.len(), MENU_BYTES);
        let text = core::str::from_utf8(&cur).unwrap();
        let mut stripped = String::new();
        for line in text.lines() {
            if line.starts_with('#') && line.len() > 64 && line[1..].bytes().all(|b| b == b' ') { continue; }
            stripped.push_str(line);
            stripped.push('\n');
        }
        let legacy = pad_to(stripped.into_bytes(), LEGACY_MENU_BYTES).unwrap();
        assert_eq!(legacy.len(), LEGACY_MENU_BYTES);
        f.write_file(&mut esp, MENU_PATH, &legacy).unwrap();
    }

    // System::find accepts the legacy 16 KiB menu, and restate keeps it 16 KiB in place.
    let sys_legacy = System::find(&mut d).unwrap();
    assert_eq!(sys_legacy.menu.sectors as usize * SECTOR, LEGACY_MENU_BYTES);
    let old_menu = sys_legacy.read_menu(&mut d).unwrap();
    assert_eq!(old_menu.len(), LEGACY_MENU_BYTES);
    let restated_legacy = restate(&old_menu, &State::fresh()).unwrap();
    assert_eq!(restated_legacy.len(), LEGACY_MENU_BYTES);
    sys_legacy.write_menu(&mut d, &restated_legacy).unwrap();

    // OTA update with a ~10 KiB limine.cfg produces a ~20 KiB dual-slot menu (Slot B + Slot A),
    // which previously failed with Error::NoSpace under 16 KiB, and now upgrades cleanly to 64 KiB.
    let idx = index_for(&img_v2, 2);
    let index = verify_index(&idx, &sign(&idx), &key().pk).unwrap();
    let slot = apply(&mut d, &sys_legacy, p.slot_a_guid, &index, &mut MemDev(img_v2), &mut |_| {}).unwrap();
    assert_eq!(slot, Slot::B);

    // Re-finding the system discovers the new 64 KiB menu cleanly.
    let sys_new = System::find(&mut d).unwrap();
    assert_eq!(sys_new.menu.sectors as usize * SECTOR, MENU_BYTES);
    let upgraded = sys_new.read_menu(&mut d).unwrap();
    assert_eq!(upgraded.len(), MENU_BYTES);
    let text = core::str::from_utf8(&upgraded).unwrap();
    assert!(text.contains(PREVIOUS_TITLE), "missing previous slot fallback");
    assert_eq!(State::parse(&upgraded).unwrap().trial, Trial::Pending);

    // Subsequent in-place restate works at 64 KiB.
    let started = State::parse(&upgraded).unwrap().on_start(Slot::B).unwrap();
    let restated_64k = restate(&upgraded, &started).unwrap();
    assert_eq!(restated_64k.len(), MENU_BYTES);
    sys_new.write_menu(&mut d, &restated_64k).unwrap();
    assert_eq!(State::parse(&sys_new.read_menu(&mut d).unwrap()).unwrap().trial, Trial::Started);
}
