// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The VBIOS: reading it out of the ROM window and finding FWSEC in it.
//!
//! The ROM holds a chain of PCI expansion ROM images: PC/AT (type 0x00, with
//! the BIT table), EFI GOP (0x03), then two FwSec images (0xE0). The BIT
//! table's falcon token ('p') points at the PMU lookup table, whose entry of
//! type 0x85 is FWSEC (production). Offsets past the end of the first image
//! are relative to the first FwSec image (they skip the EFI image), which is
//! the translation nouveau's `nvbios_addr` does and this module copies.
//!
//! FWSEC itself (descriptor v3 on Ampere): code, data, and up to a few RSA
//! signatures; its DMEM carries an "application interface" table whose DMEM
//! mapper entry we patch to run the FRTS command (carve WPR2) or SB.

use crate::fw::{FalconUcode, SigRule};
use crate::regs::{PCI_ROM_SHADOW, PROM, PROM_SIZE};
use crate::{rd16, rd32, rd8, wr32, wr64, Error, Mmio};

const IFR_SIGNATURE: u32 = 0x4947_564e; // "NVGI"
const ROM_DIRECTORY: u32 = 0x4452_4652; // "RFRD"

/// Where the PCI ROM images start inside the ROM window (after an IFR header).
pub fn prom_image_offset(m: &mut impl Mmio) -> Result<u32, Error> {
    let fixed0 = m.read(PROM);
    if fixed0 & 0xffff == 0xaa55 { return Ok(0); }
    if fixed0 != IFR_SIGNATURE { return Err(Error::BadFirmware("vbios: no ROM or IFR signature")); }
    let fixed1 = m.read(PROM + 4);
    let off = match (fixed1 >> 8) & 0xff {
        1 | 2 => {
            let data_size = (fixed1 >> 16) & 0x7fff;
            m.read(PROM + data_size + 4)
        }
        3 => {
            let data_size = m.read(PROM + 8) & 0x000f_ffff;
            let dir = m.read(PROM + data_size).wrapping_add(4096);
            if dir >= PROM_SIZE || m.read(PROM + dir) != ROM_DIRECTORY { return Err(Error::BadFirmware("vbios: no IFR ROM directory")); }
            m.read(PROM + dir + 8)
        }
        _ => return Err(Error::BadFirmware("vbios: unknown IFR version")),
    };
    if off >= PROM_SIZE || off & 3 != 0 || m.read(PROM + off) & 0xffff != 0xaa55 {
        return Err(Error::BadFirmware("vbios: bad PCI ROM offset in IFR"));
    }
    Ok(off)
}

/// Read the VBIOS image chain from the ROM window into `out`; returns its length.
/// The ROM shadow is switched off for the read and back on afterwards.
pub fn read_prom(m: &mut impl Mmio, out: &mut [u8]) -> Result<usize, Error> {
    m.modify(PCI_ROM_SHADOW, 1, 0);
    let r = read_prom_inner(m, out);
    m.modify(PCI_ROM_SHADOW, 0, 1);
    r
}

fn read_prom_inner(m: &mut impl Mmio, out: &mut [u8]) -> Result<usize, Error> {
    let base = prom_image_offset(m)?;
    let window = (PROM_SIZE - base) as usize;
    let mut done = 0usize;
    // Walk the chain, reading just enough of each image to find the next.
    let mut img = 0usize;
    loop {
        fill(m, base, window, img + 0x200, out, &mut done)?;
        let Some(info) = image_at(&out[..done], img) else { break };
        fill(m, base, window, img + info.size as usize, out, &mut done)?;
        img += info.size as usize;
        if info.last || info.size == 0 || img >= window { break; }
    }
    if img == 0 { return Err(Error::BadFirmware("vbios: no images in ROM")); }
    Ok(img.min(done))
}

fn fill(m: &mut impl Mmio, base: u32, window: usize, upto: usize, out: &mut [u8], done: &mut usize) -> Result<(), Error> {
    let upto = upto.min(window);
    if upto > out.len() { return Err(Error::BufferTooSmall { need: upto, have: out.len() }); }
    while *done + 4 <= upto {
        let v = m.read(PROM + base + *done as u32);
        out[*done..*done + 4].copy_from_slice(&v.to_le_bytes());
        *done += 4;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image { pub base: u32, pub size: u32, pub kind: u8, pub last: bool, pub vendor: u16, pub device: u16 }

/// Parse the image header at `base` (ROM signature, PCIR/NPDS, NPDE).
pub fn image_at(rom: &[u8], base: usize) -> Option<Image> {
    match rd16(rom, base)? { 0xaa55 | 0xbb77 | 0x4e56 => {} _ => return None }
    let pcir = base + rd16(rom, base + 0x18)? as usize;
    match rd32(rom, pcir)? { 0x5249_4350 | 0x5349_4752 | 0x5344_504e => {} _ => return None }
    let hdr = rd16(rom, pcir + 0x0a)? as usize;
    let mut size = rd16(rom, pcir + 0x10)? as u32 * 512;
    let kind = rd8(rom, pcir + 0x14)?;
    let mut last = rd8(rom, pcir + 0x15)? & 0x80 != 0;
    if kind != 0x70 {
        let npde = (pcir + hdr + 0x0f) & !0x0f;
        if rd32(rom, npde) == Some(0x4544_504e) {
            size = rd16(rom, npde + 8)? as u32 * 512;
            last = rd8(rom, npde + 0x0a)? & 0x80 != 0;
        }
    } else {
        last = true;
    }
    Some(Image { base: base as u32, size, kind, last, vendor: rd16(rom, pcir + 4)?, device: rd16(rom, pcir + 6)? })
}

/// A parsed VBIOS.
pub struct Vbios<'a> {
    pub data: &'a [u8],
    pub image0_size: u32,
    /// Base of the first FwSec (0xE0) image, 0 if none.
    pub fwsec_base: u32,
    pub bit_offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitEntry { pub id: u8, pub version: u8, pub length: u16, pub offset: u16 }

impl<'a> Vbios<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Vbios<'a>, Error> {
        let first = image_at(data, 0).ok_or(Error::BadFirmware("vbios: no first image"))?;
        let mut v = Vbios { data, image0_size: first.size, fwsec_base: 0, bit_offset: 0 };
        let mut img = first;
        while !img.last && img.size != 0 {
            let Some(next) = image_at(data, (img.base + img.size) as usize) else { break };
            if next.kind == 0xe0 { v.fwsec_base = next.base; break; }
            img = next;
        }
        v.bit_offset = find(data, b"\xff\xb8BIT").ok_or(Error::BadFirmware("vbios: no BIT table"))? as u32;
        Ok(v)
    }

    /// The PCI ids recorded in the first image.
    pub fn pci_ids(&self) -> (u16, u16) { image_at(self.data, 0).map(|i| (i.vendor, i.device)).unwrap_or((0, 0)) }

    fn addr(&self, a: u32, len: usize) -> Option<usize> {
        let mut a = a as usize;
        if a >= self.image0_size as usize && self.fwsec_base != 0 {
            a = a - self.image0_size as usize + self.fwsec_base as usize;
        }
        if a.checked_add(len)? > self.data.len() { return None; }
        Some(a)
    }
    pub fn r8(&self, a: u32) -> Option<u8> { rd8(self.data, self.addr(a, 1)?) }
    pub fn r16(&self, a: u32) -> Option<u16> { rd16(self.data, self.addr(a, 2)?) }
    pub fn r32(&self, a: u32) -> Option<u32> { rd32(self.data, self.addr(a, 4)?) }
    pub fn slice(&self, a: u32, len: usize) -> Option<&'a [u8]> { let o = self.addr(a, len)?; self.data.get(o..o + len) }

    pub fn bit_entry(&self, id: u8) -> Option<BitEntry> {
        let b = self.bit_offset;
        let size = self.r8(b + 9)? as u32;
        let count = self.r8(b + 10)? as u32;
        let mut e = b + 12;
        for _ in 0..count {
            if self.r8(e)? == id {
                return Some(BitEntry { id, version: self.r8(e + 1)?, length: self.r16(e + 2)?, offset: self.r16(e + 4)? });
            }
            e += size;
        }
        None
    }

    /// VBIOS version as printed by NVIDIA tools: 94.04.xx.yy.zz.
    pub fn version(&self) -> Option<[u8; 5]> {
        let i = self.bit_entry(b'i')?;
        if i.length < 5 { return None; }
        let o = i.offset as u32;
        Some([self.r8(o + 3)?, self.r8(o + 2)?, self.r8(o + 1)?, self.r8(o)?, self.r8(o + 4)?])
    }

    /// Address of the falcon ucode of PMU-table type `kind` (0x85 = FWSEC prod).
    pub fn pmu_ucode(&self, kind: u8) -> Option<u32> {
        let p = self.bit_entry(b'p')?;
        if p.version != 2 || p.length < 4 { return None; }
        let table = self.r32(p.offset as u32)?;
        if table == 0 { return None; }
        // Every offset comes from the ROM: a table near u32::MAX must not wrap
        // round to the start of the image.
        let hdr = self.r8(table.checked_add(1)?)? as u32;
        let len = self.r8(table.checked_add(2)?)? as u32;
        let cnt = self.r8(table.checked_add(3)?)? as u32;
        for i in 0..cnt {
            let e = table.checked_add(hdr)?.checked_add(i * len)?;
            if self.r8(e)? == kind { return self.r32(e.checked_add(2)?); }
        }
        None
    }

    /// FWSEC as a loadable ucode (descriptor v3, Ampere and Ada).
    pub fn fwsec(&self) -> Result<Fwsec<'a>, Error> {
        let bad = Error::BadFirmware;
        let desc = self.pmu_ucode(0x85).ok_or(bad("vbios: no FWSEC in PMU table"))?;
        let hdr = self.r32(desc).ok_or(bad("vbios: FWSEC descriptor outside ROM"))?;
        if hdr & 1 == 0 { return Err(bad("vbios: FWSEC descriptor not valid")); }
        let size = hdr >> 16;
        let version = (hdr >> 8) & 0xff;
        if version != 3 { return Err(bad("vbios: FWSEC descriptor version is not 3")); }
        let r = |o: u32| self.r32(desc + o).ok_or(bad("vbios: short FWSEC descriptor"));
        let pkc_data_offset = r(8)?;
        let interface_offset = r(12)?;
        let imem_phys_base = r(16)?;
        let imem_load_size = r(20)?;
        let dmem_phys_base = r(28)?;
        let dmem_load_size = r(32)?;
        let w = r(36)?;
        let engine_id_mask = w & 0xffff;
        let ucode_id = (w >> 16) & 0xff;
        let signature_count = (w >> 24) & 0xff;
        let signature_versions = r(40)? & 0xffff;
        const SIG: u32 = 96 * 4;
        let sigs = self.slice(desc + 0x2c, (signature_count * SIG) as usize).ok_or(bad("vbios: FWSEC signatures outside ROM"))?;
        let image = self.slice(desc + size, (imem_load_size + dmem_load_size) as usize).ok_or(bad("vbios: FWSEC image outside ROM"))?;
        let dmem_size = crate::align_up(dmem_load_size as u64, 256) as u32;
        Ok(Fwsec {
            ucode: FalconUcode {
                image, imem_img_off: 0, imem_size: imem_load_size, imem_base: imem_phys_base,
                dmem_img_off: imem_load_size, dmem_size, dmem_base: dmem_phys_base,
                sig_patch_off: imem_load_size + pkc_data_offset, sigs, sig_size: SIG, sig_count: signature_count,
                fuse_ver: signature_versions, engine_id: engine_id_mask, ucode_id,
                dmem_sign: pkc_data_offset, boot_addr: 0, rule: SigRule::Bitmap,
            },
            interface_offset,
        })
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> { h.windows(n.len()).position(|w| w == n) }

/// FWSEC plus where its application interface table sits in DMEM.
#[derive(Clone, Copy, Debug)]
pub struct Fwsec<'a> { pub ucode: FalconUcode<'a>, pub interface_offset: u32 }

pub const APPIF_ID_DMEMMAPPER: u32 = 4;
pub const DMEMMAPPER_CMD_FRTS: u32 = 0x15;
pub const DMEMMAPPER_CMD_SB: u32 = 0x19;
pub const FRTS_REGION_TYPE_FB: u32 = 2;

/// What FWSEC should do when it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwsecCommand {
    /// Carve WPR2 and write the FRTS data at `addr` (bytes, 4 KiB aligned) in VRAM.
    Frts { addr: u64, size: u64 },
    /// Secure boot step used at unload/suspend.
    Sb,
}

impl<'a> Fwsec<'a> {
    /// Patch the DMEM mapper interface of a staged FWSEC image (after
    /// [`FalconUcode::stage`]) so it runs `cmd`.
    pub fn patch_command(&self, staged: &mut [u8], cmd: FwsecCommand) -> Result<(), Error> {
        let bad = Error::BadFirmware;
        let dmem = self.ucode.dmem_img_off as usize;
        let hdr = dmem + self.interface_offset as usize;
        let rd = |b: &[u8], o: usize| rd8(b, o).ok_or(bad("fwsec: interface outside image"));
        if rd(staged, hdr)? != 1 { return Err(bad("fwsec: interface header is not version 1")); }
        let hsize = rd(staged, hdr + 1)? as usize;
        let esize = rd(staged, hdr + 2)? as usize;
        let count = rd(staged, hdr + 3)? as usize;
        for i in 0..count {
            let e = hdr + hsize + i * esize;
            if rd32(staged, e).ok_or(bad("fwsec: interface outside image"))? != APPIF_ID_DMEMMAPPER { continue; }
            let mapper = dmem + rd32(staged, e + 4).ok_or(bad("fwsec: interface outside image"))? as usize;
            if rd32(staged, mapper).is_none() || mapper + 0x40 > staged.len() { return Err(bad("fwsec: DMEM mapper outside image")); }
            let init = match cmd { FwsecCommand::Frts { .. } => DMEMMAPPER_CMD_FRTS, FwsecCommand::Sb => DMEMMAPPER_CMD_SB };
            wr32(staged, mapper + 0x2c, init); // init_cmd
            let buf = dmem + rd32(staged, mapper + 0x08).unwrap() as usize; // cmd_in_buffer_offset
            if buf + 44 > staged.len() { return Err(bad("fwsec: command buffer outside image")); }
            // read_vbios: ver, hdr, addr(u64), size, flags
            wr32(staged, buf, 1); wr32(staged, buf + 4, 24); wr64(staged, buf + 8, 0); wr32(staged, buf + 16, 0); wr32(staged, buf + 20, 2);
            if let FwsecCommand::Frts { addr, size } = cmd {
                // frts_region: ver, hdr, addr, size, type (4 KiB units)
                let r = buf + 24;
                wr32(staged, r, 1); wr32(staged, r + 4, 20); wr32(staged, r + 8, (addr >> 12) as u32);
                wr32(staged, r + 12, (size >> 12) as u32); wr32(staged, r + 16, FRTS_REGION_TYPE_FB);
            }
            return Ok(());
        }
        Err(bad("fwsec: no DMEM mapper interface"))
    }
}
