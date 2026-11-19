// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Synthetic firmware and VBIOS images with the real containers' layouts, so
//! the gate runs without NVIDIA's files (the real ones are tested when
//! NV_GSP_FW_DIR points at scripts/fetch_nvidia_gsp_firmware.sh's output).
#![allow(dead_code)]

pub fn w16(b: &mut [u8], o: usize, v: u16) { b[o..o + 2].copy_from_slice(&v.to_le_bytes()); }
pub fn w32(b: &mut [u8], o: usize, v: u32) { b[o..o + 4].copy_from_slice(&v.to_le_bytes()); }
pub fn w64(b: &mut [u8], o: usize, v: u64) { b[o..o + 8].copy_from_slice(&v.to_le_bytes()); }
pub fn r32(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }
pub fn r64(b: &[u8], o: usize) -> u64 { u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) }

/// One PCI expansion ROM image header at `base`: ROM signature, PCIR, NPDE.
fn rom_image(rom: &mut [u8], base: usize, size: usize, kind: u8, last: bool) {
    rom[base] = 0x55; rom[base + 1] = 0xaa;
    w16(rom, base + 0x18, 0x40);
    let p = base + 0x40;
    rom[p..p + 4].copy_from_slice(b"PCIR");
    w16(rom, p + 4, 0x10de); w16(rom, p + 6, 0x249d);
    w16(rom, p + 0x0a, 0x18);
    w16(rom, p + 0x10, (size / 512) as u16);
    rom[p + 0x14] = kind;
    rom[p + 0x15] = 0x80; // PCIR says last; NPDE overrides, as on real boards
    let n = (p + 0x18 + 0x0f) & !0x0f;
    rom[n..n + 4].copy_from_slice(b"NPDE");
    w16(rom, n + 8, (size / 512) as u16);
    rom[n + 0x0a] = if last { 0x80 } else { 0 };
}

pub const FWSEC_IMEM: usize = 0x200;
pub const FWSEC_DMEM: usize = 0x300;
pub const FWSEC_IF: usize = 0x10; // interface table, DMEM offset
pub const FWSEC_MAPPER: usize = 0x40;
pub const FWSEC_CMDBUF: usize = 0x80;
pub const FWSEC_PKC: usize = 0x100;
pub const SIG: usize = 384;

/// A GA104-like VBIOS: PC/AT (BIT), EFI, FwSec, FwSec (PMU table + FWSEC v3).
pub fn vbios() -> Vec<u8> {
    let mut rom = vec![0u8; 0x2800];
    rom_image(&mut rom, 0, 0x1000, 0x00, false);
    rom_image(&mut rom, 0x1000, 0x800, 0x03, false);
    rom_image(&mut rom, 0x1800, 0x400, 0xe0, false);
    rom_image(&mut rom, 0x1c00, 0xc00, 0xe0, true);
    // BIT table at 0x100: 'i' (version) and 'p' (falcon data) tokens.
    rom[0x100..0x106].copy_from_slice(b"\xff\xb8BIT\0");
    rom[0x109] = 6; rom[0x10a] = 2;
    let e = 0x10c;
    rom[e] = b'i'; rom[e + 1] = 0; w16(&mut rom, e + 2, 5); w16(&mut rom, e + 4, 0x200);
    rom[e + 6] = b'p'; rom[e + 7] = 2; w16(&mut rom, e + 8, 4); w16(&mut rom, e + 10, 0x210);
    rom[0x200..0x205].copy_from_slice(&[0x05, 0x04, 0x30, 0x94, 0x07]);
    // Addresses past image 0 (0x1000) are relative to the first FwSec image (0x1800).
    let xl = |file: usize| (file - 0x1800 + 0x1000) as u32;
    w32(&mut rom, 0x210, xl(0x1d00));
    // PMU lookup table: version, header 6, entry 6, count 2.
    let t = 0x1d00;
    rom[t..t + 4].copy_from_slice(&[1, 6, 6, 2]);
    rom[t + 6] = 0x45; w32(&mut rom, t + 8, 0xdead);
    rom[t + 12] = 0x85; w32(&mut rom, t + 14, xl(0x1e00));
    // FWSEC descriptor v3 at 0x1e00, two signatures, then the image.
    let d = 0x1e00;
    let size = 0x2c + 2 * SIG;
    w32(&mut rom, d, ((size as u32) << 16) | (3 << 8) | 1);
    w32(&mut rom, d + 4, (FWSEC_IMEM + FWSEC_DMEM) as u32);
    w32(&mut rom, d + 8, FWSEC_PKC as u32);
    w32(&mut rom, d + 12, FWSEC_IF as u32);
    w32(&mut rom, d + 16, 0); // IMEMPhysBase
    w32(&mut rom, d + 20, FWSEC_IMEM as u32);
    w32(&mut rom, d + 24, 0);
    w32(&mut rom, d + 28, 0); // DMEMPhysBase
    w32(&mut rom, d + 32, FWSEC_DMEM as u32);
    w32(&mut rom, d + 36, 0x400 | (1 << 16) | (2 << 24)); // engine GSP, ucode 1, 2 sigs
    w32(&mut rom, d + 40, 0b11); // signature versions
    for s in 0..2 { rom[d + 0x2c + s * SIG..d + 0x2c + (s + 1) * SIG].fill(0xa0 + s as u8); }
    let img = d + size;
    for i in 0..FWSEC_IMEM { rom[img + i] = (i as u8) ^ 0x5a; }
    let dm = img + FWSEC_IMEM;
    rom[dm + FWSEC_IF..dm + FWSEC_IF + 4].copy_from_slice(&[1, 4, 8, 1]);
    w32(&mut rom, dm + FWSEC_IF + 4, 4); // DMEMMAPPER
    w32(&mut rom, dm + FWSEC_IF + 8, FWSEC_MAPPER as u32);
    w32(&mut rom, dm + FWSEC_MAPPER, u32::from_le_bytes(*b"DMAP"));
    w32(&mut rom, dm + FWSEC_MAPPER + 8, FWSEC_CMDBUF as u32);
    rom
}

/// nvfw_bin_hdr + payload.
fn bin(header: &[u8], data: &[u8], tail: &[u8]) -> Vec<u8> {
    let header_offset = 0x18;
    let data_offset = 0x400;
    let mut b = vec![0u8; data_offset + data.len() + tail.len()];
    w32(&mut b, 0, 0x10de); w32(&mut b, 4, 1); let n = b.len() as u32; w32(&mut b, 8, n);
    w32(&mut b, 12, header_offset as u32); w32(&mut b, 16, data_offset as u32); w32(&mut b, 20, data.len() as u32);
    b[header_offset..header_offset + header.len()].copy_from_slice(header);
    b[data_offset..data_offset + data.len()].copy_from_slice(data);
    b[data_offset + data.len()..].copy_from_slice(tail);
    b
}

pub const BOOTER_IMEM: (u32, u32) = (0x100, 0x300); // offset, size
pub const BOOTER_DMEM: (u32, u32) = (0x400, 0x200);
pub const BOOTER_LOC: u32 = 0x410;

/// A booter_load-shaped file: same headers as the real one, tiny payload.
pub fn booter() -> Vec<u8> {
    // Header area (offsets are absolute in the file): hs_v2 at 0x18, then
    // the indirection words, meta, and load header.
    let mut h = vec![0u8; 0x3e8];
    let data_len = 0x600usize;
    let sigs_at = 0x400 + data_len; // tail = signatures
    // hs header v2 (absolute offsets)
    w32(&mut h, 0, sigs_at as u32); // sig_prod_offset
    w32(&mut h, 4, 2 * SIG as u32); // sig_prod_size
    w32(&mut h, 8, 0x100); // patch_loc -> u32 at 0x100
    w32(&mut h, 12, 0x104); // patch_sig -> u32 at 0x104
    w32(&mut h, 16, 0x108); // meta_data_offset
    w32(&mut h, 20, 12);
    w32(&mut h, 24, 0x114); // num_sig -> u32 at 0x114
    w32(&mut h, 28, 0x120); // header_offset (load header)
    w32(&mut h, 32, 0x24);
    let a = |o: usize| o - 0x18;
    w32(&mut h, a(0x100), BOOTER_LOC);
    w32(&mut h, a(0x104), 0);
    w32(&mut h, a(0x108), 1); w32(&mut h, a(0x10c), 1); w32(&mut h, a(0x110), 3); // fuse_ver, engine SEC2, ucode 3
    w32(&mut h, a(0x114), 2);
    let lh = a(0x120);
    w32(&mut h, lh, 0); w32(&mut h, lh + 4, 0x100);
    w32(&mut h, lh + 8, BOOTER_DMEM.0); w32(&mut h, lh + 12, BOOTER_DMEM.1);
    w32(&mut h, lh + 16, 1);
    w32(&mut h, lh + 20, BOOTER_IMEM.0); w32(&mut h, lh + 24, BOOTER_IMEM.1);
    let mut data = vec![0u8; data_len];
    for (i, b) in data.iter_mut().enumerate() { *b = (i * 7) as u8; }
    let mut sigs = vec![0u8; 2 * SIG];
    sigs[..SIG].fill(0xb0); sigs[SIG..].fill(0xb1);
    bin(&h, &data, &sigs)
}

pub const BL_APP_VERSION: u32 = 0x0222_8b83;

/// A bootloader-shaped file with an RM_RISCV_UCODE_DESC.
pub fn bootloader() -> Vec<u8> {
    let mut d = vec![0u8; 84];
    w32(&mut d, 0, 5);
    w32(&mut d, 28, BL_APP_VERSION);
    w32(&mut d, 32, 0x800); w32(&mut d, 36, 0x100); // manifest
    w32(&mut d, 40, 0x400); w32(&mut d, 44, 0x400); // monitor data
    w32(&mut d, 48, 0x000); w32(&mut d, 52, 0x400); // monitor code
    bin(&d, &vec![0x11u8; 0x1000], &[])
}

/// A GSP-RM-shaped ELF: `.fwimage` of `pages` pages and the GA10x signature.
pub fn gsp_elf(pages: usize) -> Vec<u8> {
    let names = b"\0.fwimage\0.fwsignature_ga10x\0.shstrtab\0";
    let img_len = pages * 4096;
    let sig_len = 0x1000;
    let img_off = 0x40usize;
    let sig_off = img_off + img_len;
    let str_off = sig_off + sig_len;
    let sh_off = (str_off + names.len() + 7) & !7;
    let mut b = vec![0u8; sh_off + 4 * 0x40];
    b[0..4].copy_from_slice(b"\x7fELF"); b[4] = 2; b[5] = 1; b[6] = 1;
    w16(&mut b, 16, 1); w16(&mut b, 18, 243);
    w64(&mut b, 0x28, sh_off as u64);
    w16(&mut b, 0x34, 0x40); w16(&mut b, 0x3a, 0x40); w16(&mut b, 0x3c, 4); w16(&mut b, 0x3e, 3);
    for i in 0..img_len { b[img_off + i] = (i / 4096) as u8 ^ (i as u8); }
    b[sig_off..sig_off + sig_len].fill(0x5e);
    b[str_off..str_off + names.len()].copy_from_slice(names);
    let sh = |b: &mut Vec<u8>, i: usize, name: u32, off: usize, size: usize| {
        let o = sh_off + i * 0x40;
        w32(b, o, name); w32(b, o + 4, 1); w64(b, o + 0x18, off as u64); w64(b, o + 0x20, size as u64);
    };
    sh(&mut b, 1, 1, img_off, img_len);
    sh(&mut b, 2, 10, sig_off, sig_len);
    sh(&mut b, 3, 29, str_off, names.len());
    b
}

/// Real firmware from NV_GSP_FW_DIR, if the gate fetched it.
pub fn real_firmware() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let dir = std::env::var("NV_GSP_FW_DIR").ok()?;
    let rd = |n: &str| std::fs::read(format!("{dir}/{n}-535.113.01.bin")).ok();
    Some((rd("booter_load")?, rd("bootloader")?, rd("gsp")?))
}
