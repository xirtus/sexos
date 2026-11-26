// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Firmware containers, ELF sections, VBIOS walking, FWSEC, signature choice.
mod common;
use common::*;
use sex_gpu_nv::chip::{Chip, Family};
use sex_gpu_nv::fw::{self, SigRule};
use sex_gpu_nv::vbios::{self, FwsecCommand, Vbios};
use sex_gpu_nv::Error;

#[test]
fn chip_decode_ga104() {
    let c = Chip::decode(0x174000a1).unwrap();
    assert_eq!((c.chipset, c.revision, c.name()), (0x174, 0xa1, "GA104"));
    assert_eq!(c.family(), Family::Ampere);
    assert!(c.gsp_boot_supported());
    assert_eq!(c.signature_section(), ".fwsignature_ga10x");
    assert!(Chip::decode(0xffff_ffff).is_none(), "a powered-off GPU reads all ones");
    assert!(!Chip::decode(0x170000a1).unwrap().gsp_boot_supported(), "GA100 boots differently");
    assert_eq!(Chip::decode(0x194000a1).unwrap().signature_section(), ".fwsignature_ad10x");
}

#[test]
fn booter_container() {
    let b = booter();
    let u = fw::booter(&b).unwrap();
    assert_eq!((u.imem_img_off, u.imem_size), BOOTER_IMEM);
    assert_eq!((u.dmem_img_off, u.dmem_size), BOOTER_DMEM);
    assert_eq!((u.sig_size, u.sig_count, u.sig_patch_off, u.dmem_sign), (SIG as u32, 2, BOOTER_LOC, 0x10));
    assert_eq!((u.fuse_ver, u.engine_id, u.ucode_id, u.boot_addr), (1, 1, 3, 0x100));
    assert_eq!(u.fuse_register(), Some(0x824140 + 8));
    assert_eq!(u.rule, SigRule::Version);
    let mut staged = vec![0u8; u.staged_len()];
    u.stage(&mut staged, 1).unwrap();
    assert!(staged[BOOTER_LOC as usize..BOOTER_LOC as usize + SIG].iter().all(|&x| x == 0xb1));
    let mut bad = b.clone();
    bad[0] = 0;
    assert_eq!(fw::booter(&bad).unwrap_err(), Error::BadFirmware("fw: bad bin magic"));
}

#[test]
fn signature_rules() {
    let b = booter();
    let mut u = fw::booter(&b).unwrap();
    // Version rule: fuse 0 -> last signature; fuse 1 (fls 1) with fuse_ver 1 -> index 0.
    assert_eq!(u.signature_index(0).unwrap(), 1);
    assert_eq!(u.signature_index(1).unwrap(), 0);
    assert!(u.signature_index(0b110).is_err(), "fuse newer than the ucode");
    u.fuse_ver = 2;
    assert_eq!(u.signature_index(1).unwrap(), 1);
    // Bitmap rule (FWSEC): versions 0b11.
    u.rule = SigRule::Bitmap;
    u.fuse_ver = 0b11;
    assert_eq!(u.signature_index(0).unwrap(), 0);
    assert_eq!(u.signature_index(1).unwrap(), 1);
    assert!(u.signature_index(0b11).is_err(), "no signature for version 2");
    u.fuse_ver = 0b1010; // signatures for versions 1 and 3
    assert_eq!(u.signature_index(0b1).unwrap(), 0);
    assert_eq!(u.signature_index(0b100).unwrap(), 1);
}

#[test]
fn bootloader_and_elf() {
    let blf = bootloader();
    let (d, data) = fw::bootloader(&blf).unwrap();
    assert_eq!((d.version, d.app_version, d.manifest_offset, d.monitor_data_offset, d.monitor_code_offset), (5, BL_APP_VERSION, 0x800, 0x400, 0));
    assert_eq!(data.len(), 0x1000);
    let elf = gsp_elf(3);
    let rm = fw::gsp_rm(&elf, ".fwsignature_ga10x").unwrap();
    assert_eq!((rm.image.len(), rm.signature.len()), (3 * 4096, 0x1000));
    assert_eq!(rm.image[4096 + 1], 1 ^ 1);
    assert_eq!(fw::gsp_rm(&elf, ".fwsignature_ad10x").err(), Some(Error::BadFirmware("gsp: section not found")));
    assert!(sex_gpu_nv::elf::section(&elf[..0x30], ".fwimage").is_err());
}

#[test]
fn vbios_walk_bit_and_fwsec() {
    let rom = vbios();
    let v = Vbios::parse(&rom).unwrap();
    assert_eq!((v.image0_size, v.fwsec_base, v.bit_offset), (0x1000, 0x1800, 0x100));
    assert_eq!(v.pci_ids(), (0x10de, 0x249d));
    assert_eq!(v.version(), Some([0x94, 0x30, 0x04, 0x05, 0x07]));
    let f = v.fwsec().unwrap();
    let u = &f.ucode;
    assert_eq!((u.imem_size, u.dmem_img_off, u.dmem_size, u.sig_count, u.ucode_id, u.engine_id), (0x200, 0x200, 0x300, 2, 1, 0x400));
    assert_eq!(u.fuse_register(), Some(0x8241c0));
    assert_eq!(u.sig_patch_off as usize, FWSEC_IMEM + FWSEC_PKC);
    assert_eq!(u.image[0], 0x5a);
    let mut staged = vec![0u8; u.staged_len()];
    u.stage(&mut staged, 1).unwrap();
    assert!(staged[FWSEC_IMEM + FWSEC_PKC..][..SIG].iter().all(|&x| x == 0xa1));
    f.patch_command(&mut staged, FwsecCommand::Frts { addr: 0x1_ffe0_0000, size: 0x10_0000 }).unwrap();
    let dm = FWSEC_IMEM;
    assert_eq!(r32(&staged, dm + FWSEC_MAPPER + 0x2c), vbios::DMEMMAPPER_CMD_FRTS);
    let cb = dm + FWSEC_CMDBUF;
    assert_eq!([r32(&staged, cb), r32(&staged, cb + 4), r32(&staged, cb + 20)], [1, 24, 2]);
    assert_eq!([r32(&staged, cb + 24), r32(&staged, cb + 28), r32(&staged, cb + 32), r32(&staged, cb + 36), r32(&staged, cb + 40)], [1, 20, 0x1ffe00, 0x100, 2]);
}

#[test]
fn vbios_rejects_garbage() {
    assert!(Vbios::parse(&[0u8; 512]).is_err());
    let mut rom = vbios();
    rom[0x100] = 0; // no BIT
    assert!(Vbios::parse(&rom).is_err());
    let mut rom = vbios();
    let d = 0x1e00;
    rom[d + 1] = 2; // descriptor v2 needs the ACR bootloader: not on Ampere
    assert_eq!(Vbios::parse(&rom).unwrap().fwsec().err(), Some(Error::BadFirmware("vbios: FWSEC descriptor version is not 3")));
}

struct Rom { data: Vec<u8>, shadow: u32, reads: usize }
impl sex_gpu_nv::Mmio for Rom {
    fn read(&mut self, o: u32) -> u32 {
        if o == sex_gpu_nv::regs::PCI_ROM_SHADOW { return self.shadow; }
        assert_eq!(self.shadow & 1, 0, "ROM read with the shadow on");
        self.reads += 1;
        let i = (o - sex_gpu_nv::regs::PROM) as usize;
        if i + 4 <= self.data.len() { r32(&self.data, i) } else { 0xffff_ffff }
    }
    fn write(&mut self, o: u32, v: u32) { assert_eq!(o, sex_gpu_nv::regs::PCI_ROM_SHADOW); self.shadow = v; }
}

#[test]
fn prom_read_plain_and_ifr() {
    let img = vbios();
    let mut m = Rom { data: img.clone(), shadow: 1, reads: 0 };
    let mut out = vec![0u8; 1 << 20];
    let n = vbios::read_prom(&mut m, &mut out).unwrap();
    assert_eq!(n, img.len());
    assert_eq!(&out[..n], &img[..]);
    assert_eq!(m.shadow, 1, "shadow restored");
    assert_eq!(m.reads, img.len() / 4 + 1, "reads only the image chain");
    // IFR v3 header in front of the images (as GA100-style boards ship).
    let off = 0x4000usize;
    let mut rom = vec![0u8; off + img.len()];
    w32(&mut rom, 0, 0x4947_564e);
    w32(&mut rom, 4, 3 << 8);
    w32(&mut rom, 8, 0x100);
    w32(&mut rom, 0x100, 0x2000 - 4096); // flash status offset; +4096 = directory
    rom[0x2000..0x2004].copy_from_slice(b"RFRD");
    w32(&mut rom, 0x2008, off as u32);
    rom[off..].copy_from_slice(&img);
    let mut m = Rom { data: rom, shadow: 0, reads: 0 };
    assert_eq!(vbios::prom_image_offset(&mut m).unwrap() as usize, off);
    let n = vbios::read_prom(&mut m, &mut out).unwrap();
    assert_eq!(&out[..n], &img[..]);
}

/// NVIDIA's real files (CI fetches them; skipped when absent).
#[test]
fn real_535_firmware() {
    let Some((booter_load, bl, gsp)) = real_firmware() else { eprintln!("NV_GSP_FW_DIR not set; skipping"); return };
    let u = fw::booter(&booter_load).unwrap();
    assert_eq!((u.imem_img_off, u.imem_size, u.dmem_img_off, u.dmem_size), (0x100, 0x8300, 0x8400, 0x6200));
    assert_eq!((u.sig_size, u.sig_count, u.sig_patch_off, u.dmem_sign), (0x180, 2, 0x8410, 0x10));
    assert_eq!((u.fuse_ver, u.engine_id, u.ucode_id, u.boot_addr), (1, 1, 3, 0x100));
    let (d, data) = fw::bootloader(&bl).unwrap();
    assert_eq!((d.version, d.app_version, d.manifest_offset, d.monitor_code_offset, d.monitor_data_offset, data.len()), (5, 0, 0, 0x1800, 0x800, 0x5000));
    let rm = fw::gsp_rm(&gsp, Chip::decode(0x174000a1).unwrap().signature_section()).unwrap();
    assert_eq!((rm.image.len(), rm.signature.len()), (0x2448000, 0x1000));
    assert_eq!(sex_gpu_nv::elf::section(&gsp, ".fwversion").unwrap(), b"535.113.01\0");
    let sizes = sex_gpu_nv::boot::Sizes::for_firmware(&Chip::decode(0x174000a1).unwrap(), &sex_gpu_nv::boot::Firmware { booter_load: &booter_load, booter_unload: None, bootloader: &bl, gsp: &gsp }).unwrap();
    assert!(sizes.total() < 40 << 20, "GPU-visible memory for the boot: {} bytes", sizes.total());
}
