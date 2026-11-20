// SPDX-License-Identifier: GPL-3.0-only
//! Intel Wi-Fi firmware files as the driver loads them from disk: the `.ucode`
//! TLV file (sections, capabilities, command versions), the `.pnvm` radio
//! calibration file and the compressed firmware pack the image ships them in.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_wifi::{pack, ucode};
use sexos_driver_fuzz::Src;

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let sku = [s.u32(), s.u32(), s.u32()];
    let (mac, rf) = (s.u16(), s.u16());
    let file = s.rest();
    if let Ok(fw) = ucode::Firmware::parse(file) {
        let _ = (fw.sections().len(), fw.has_capa(sku[0] as usize % 256), fw.has_api(sku[1] as usize % 256));
        let img = fw.image();
        let _ = (img.lmac.len(), img.umac.len(), img.paging.len());
        for g in [0u8, 1, 2, 3, 4, 5, 0x10] { let _ = fw.command_version(g, (sku[2] & 0xff) as u8); }
    }
    let _ = ucode::pnvm_for_sku(file, sku);
    let _ = ucode::pnvm_select(file, sku, mac, rf);
    let _ = ucode::pnvm_image(file, mac, rf);
    if let Ok(p) = pack::Pack::parse(file) {
        let mut scratch = vec![0u8; 256 * 1024];
        let mut inflater = pack::Inflater::new();
        for i in 0..p.len().min(16) {
            if let Some(e) = p.entry(i) {
                if let Ok(name) = core::str::from_utf8(e.name) {
                    let _ = p.file(name);
                    let _ = p.open(name, &mut scratch, &mut inflater);
                }
            }
        }
    }
}

fn seeds() -> Vec<Vec<u8>> {
    // The TLV ucode layout: zero word, magic, 64-byte version text, version, build,
    // then TLVs (type, length, data, padded to 4).
    let mut f = vec![0u8; 14];
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&ucode::MAGIC.to_le_bytes());
    let mut text = [0u8; 64];
    text[..12].copy_from_slice(b"77.test.sexs");
    f.extend_from_slice(&text);
    f.extend_from_slice(&(77u32 << 8 | 1 << 24).to_le_bytes());
    f.extend_from_slice(&[0u8; 12]);
    for (kind, len) in [(19u32, 64usize), (20, 64), (29, 12), (30, 8), (31, 8), (48, 16), (54, 8)] {
        f.extend_from_slice(&kind.to_le_bytes());
        f.extend_from_slice(&(len as u32).to_le_bytes());
        f.extend(std::iter::repeat(1u8).take(len));
    }
    vec![f]
}

sexos_driver_fuzz::driver_target!(run, seeds);
