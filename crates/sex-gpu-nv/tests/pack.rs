// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The firmware pack sexgpu-nv reads (`nvidia-gsp.pack`), and carving its DMA pool.
use sex_gpu_nv::pack::{self, Pack};
use sex_gpu_nv::{boot, chip::Chip, Error};
mod common;

fn unhex(h: &str) -> [u8; 32] {
    let mut o = [0u8; 32];
    for i in 0..32 { o[i] = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap(); }
    o
}

/// The same layout as scripts/make_nvidia_firmware_pack.py.
fn build(files: &[(&str, &[u8], [u8; 32])]) -> Vec<u8> {
    let mut h = b"SEXNVFW1".to_vec();
    h.extend((files.len() as u32).to_le_bytes());
    h.extend(0u32.to_le_bytes());
    let start = (16 + 88 * files.len()).div_ceil(4096) * 4096;
    let mut data = vec![];
    for (name, raw, sha) in files {
        let mut n = name.as_bytes().to_vec();
        n.resize(44, 0);
        h.extend(n);
        h.extend(0u32.to_le_bytes());
        h.extend(((start + data.len()) as u32).to_le_bytes());
        h.extend((raw.len() as u32).to_le_bytes());
        h.extend(sha);
        data.extend(*raw);
        data.resize(data.len().div_ceil(4096) * 4096, 0);
    }
    h.resize(start, 0);
    h.extend(data);
    h
}

#[test]
fn finds_files_and_checks_pins() {
    let gsp_pin = unhex(pack::PINS[3].1);
    let blob = build(&[("LICENCE.nvidia", b"licence", [7; 32]), (pack::GSP_RM, b"gsp image", gsp_pin), (pack::BOOTLOADER, b"bl", [1; 32])]);
    let p = Pack::parse(&blob).unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(p.file("LICENCE.nvidia").unwrap().data, b"licence");
    let g = p.pinned(pack::GSP_RM).unwrap();
    assert_eq!(g.data, b"gsp image");
    assert_eq!(g.sha256, gsp_pin);
    // Recorded hash is not the pin: another firmware version.
    assert_eq!(p.pinned(pack::BOOTLOADER).unwrap_err(), Error::BadFirmware("pack: file is not the pinned version"));
    assert_eq!(p.pinned(pack::BOOTER_LOAD).unwrap_err(), Error::BadFirmware("pack: pinned file missing"));
    assert_eq!(p.pinned("LICENCE.nvidia").unwrap_err(), Error::BadFirmware("pack: file has no pin"));
}

#[test]
fn refuses_broken_packs() {
    assert!(Pack::parse(b"SEXWIFW2\0\0\0\0\0\0\0\0").is_err());
    let mut blob = build(&[(pack::GSP_RM, b"x", [0; 32])]);
    // Length past the end.
    blob[16 + 52..16 + 56].copy_from_slice(&0x10_0000u32.to_le_bytes());
    assert_eq!(Pack::parse(&blob).unwrap_err(), Error::BadFirmware("pack: entry outside the pack"));
    let mut blob = build(&[(pack::GSP_RM, b"x", [0; 32])]);
    blob[16 + 44] = 1; // compressed: not in this format
    assert!(Pack::parse(&blob).is_err());
    let empty = build(&[]);
    assert!(Pack::parse(&empty).unwrap().is_empty());
}

#[test]
fn hex_compare() {
    let h = pack::PINS[0].1;
    assert!(pack::hex_eq(h, &unhex(h)));
    let mut b = unhex(h);
    b[31] ^= 1;
    assert!(!pack::hex_eq(h, &b));
    assert!(!pack::hex_eq("00", &b));
}

#[test]
fn carves_the_pool_in_page_steps() {
    let (bl, bt, gsp) = (common::bootloader(), common::booter(), common::gsp_elf(40));
    let f = boot::Firmware { booter_load: &bt, booter_unload: None, bootloader: &bl, gsp: &gsp };
    let chip = Chip::decode(0x174000a1).unwrap();
    let sizes = boot::Sizes::for_firmware(&chip, &f).unwrap();
    let mut pool = vec![0u8; sizes.total()];
    let mut scratch = vec![0u8; 2 * sex_gpu_nv::msgq::MAX_MSG];
    let m = boot::Memory::carve(&mut pool, 0x1_0000_0000, &sizes, &mut scratch).unwrap();
    assert_eq!(m.fwsec.bus, 0x1_0000_0000);
    assert_eq!(m.booter.bus, 0x1_0000_0000 + sizes.fwsec_max as u64);
    assert_eq!(m.shm.bus + m.shm.len() as u64, 0x1_0000_0000 + sizes.total() as u64);
    for d in [&m.rm_image, &m.radix3_l2, &m.signature, &m.libos, &m.rmargs] { assert_eq!(d.bus % 4096, 0); }
    drop(m);
    let mut small = vec![0u8; sizes.total() - 4096];
    assert!(matches!(boot::Memory::carve(&mut small, 0, &sizes, &mut scratch), Err(Error::BufferTooSmall { .. })));
    assert!(boot::Memory::carve(&mut pool, 0x800, &sizes, &mut scratch).is_err());
}

#[test]
fn real_pack_holds_the_pinned_firmware() {
    let Ok(path) = std::env::var("NV_GSP_PACK") else { eprintln!("NV_GSP_PACK unset: skipped"); return };
    let blob = std::fs::read(path).unwrap();
    let p = Pack::parse(&blob).unwrap();
    for (name, _) in pack::PINS { p.pinned(name).unwrap(); }
    let f = boot::Firmware { booter_load: p.pinned(pack::BOOTER_LOAD).unwrap().data, booter_unload: Some(p.pinned(pack::BOOTER_UNLOAD).unwrap().data), bootloader: p.pinned(pack::BOOTLOADER).unwrap().data, gsp: p.pinned(pack::GSP_RM).unwrap().data };
    let sizes = boot::Sizes::for_firmware(&Chip::decode(0x174000a1).unwrap(), &f).unwrap();
    // The service's pool (sex_pdx::nvidia_gpu::DMA_BYTES) holds the whole boot.
    assert!(sizes.total() <= 64 << 20, "boot needs {} bytes", sizes.total());
    assert!(p.file("LICENCE.nvidia").is_some());
}
