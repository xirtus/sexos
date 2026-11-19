// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! NVIDIA's firmware file containers (linux-firmware `nvidia/ga102/gsp/`).
//!
//! - `booter_load-*.bin` / `booter_unload-*.bin`: heavy-secure falcon ucode
//!   for SEC2, in a `nvfw_bin_hdr` + `nvfw_hs_header_v2` container with
//!   several RSA-3K signatures, one per fuse version.
//! - `bootloader-*.bin`: the GSP RISC-V bootloader, described by an
//!   `RM_RISCV_UCODE_DESC`.
//! - `gsp-*.bin`: the GSP-RM ELF (see [`crate::elf`]).

use crate::{rd32, Error};

pub const BIN_MAGIC: u32 = 0x0000_10de;

/// `nvfw_bin_hdr`: the outer header of every file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BinHdr { pub magic: u32, pub version: u32, pub size: u32, pub header_offset: u32, pub data_offset: u32, pub data_size: u32 }

pub fn bin_hdr(b: &[u8]) -> Result<BinHdr, Error> {
    let r = |o| rd32(b, o).ok_or(Error::BadFirmware("fw: short bin header"));
    let h = BinHdr { magic: r(0)?, version: r(4)?, size: r(8)?, header_offset: r(12)?, data_offset: r(16)?, data_size: r(20)? };
    if h.magic != BIN_MAGIC { return Err(Error::BadFirmware("fw: bad bin magic")); }
    if (h.data_offset as usize).checked_add(h.data_size as usize).map_or(true, |e| e > b.len()) {
        return Err(Error::BadFirmware("fw: data outside file"));
    }
    Ok(h)
}

/// A falcon ucode ready to load: an image (IMEM part and DMEM part inside it),
/// where to put each, and the signatures to patch in before loading.
#[derive(Clone, Copy, Debug)]
pub struct FalconUcode<'a> {
    /// The bytes to DMA from (copied into GPU-visible memory and patched).
    pub image: &'a [u8],
    pub imem_img_off: u32,
    pub imem_size: u32,
    /// Falcon IMEM address the code goes to.
    pub imem_base: u32,
    pub dmem_img_off: u32,
    pub dmem_size: u32,
    pub dmem_base: u32,
    /// Where (offset in `image`) the chosen signature is written.
    pub sig_patch_off: u32,
    /// All signatures back to back, `sig_size` bytes each.
    pub sigs: &'a [u8],
    pub sig_size: u32,
    pub sig_count: u32,
    /// Fuse version bitmap/number the signatures were made for (meaning
    /// depends on [`SigRule`]).
    pub fuse_ver: u32,
    pub engine_id: u32,
    pub ucode_id: u32,
    /// Offset of the signature inside DMEM (what the boot ROM is told).
    pub dmem_sign: u32,
    pub boot_addr: u32,
    pub rule: SigRule,
}

/// How a ucode's signature index follows from the fuse register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigRule {
    /// Booter on SEC2 (GA100 rule): `fuse_ver` is a version number;
    /// index = fuse_ver - fls(fuse), or the last one when the fuse is 0.
    Version,
    /// FWSEC on GSP (GA102 rule): `fuse_ver` is a bitmap of versions with a
    /// signature; walk to the bit just above the fuse's highest bit.
    Bitmap,
}

impl<'a> FalconUcode<'a> {
    /// The fuse register that decides the signature, or None if the engine is unknown.
    pub fn fuse_register(&self) -> Option<u32> {
        let idx = self.ucode_id.checked_sub(1)? * 4;
        if self.engine_id & 0x001 != 0 { Some(crate::regs::FUSE_SEC2_UCODE_VERSION + idx) }
        else if self.engine_id & 0x004 != 0 { Some(crate::regs::FUSE_NVDEC_UCODE_VERSION + idx) }
        else if self.engine_id & 0x400 != 0 { Some(crate::regs::FUSE_GSP_UCODE_VERSION + idx) }
        else { None }
    }

    /// Which signature matches the chip's fuse value.
    pub fn signature_index(&self, fuse: u32) -> Result<u32, Error> {
        let idx = match self.rule {
            SigRule::Version => {
                if fuse == 0 { self.sig_count.checked_sub(1).ok_or(Error::NoSignature("no signatures"))? }
                else {
                    let r = crate::fls(fuse);
                    if self.fuse_ver < r { return Err(Error::NoSignature("ucode older than fuse")); }
                    self.fuse_ver - r
                }
            }
            SigRule::Bitmap => {
                let mut reg = 1u32.checked_shl(crate::fls(fuse)).unwrap_or(0);
                let mut sig = self.fuse_ver;
                if reg & sig == 0 { return Err(Error::NoSignature("no signature for fuse version")); }
                let mut idx = 0;
                while reg & sig & 1 == 0 { idx += sig & 1; reg >>= 1; sig >>= 1; }
                idx
            }
        };
        if idx >= self.sig_count { return Err(Error::NoSignature("signature index past the end")); }
        Ok(idx)
    }

    /// Copy the image into `dst` (GPU-visible) and patch signature `idx` in.
    pub fn stage(&self, dst: &mut [u8], idx: u32) -> Result<(), Error> {
        let n = self.image.len();
        if dst.len() < n { return Err(Error::BufferTooSmall { need: n, have: dst.len() }); }
        dst[..n].copy_from_slice(self.image);
        let s = self.sig_size as usize;
        let src = idx as usize * s;
        let sig = self.sigs.get(src..src + s).ok_or(Error::BadFirmware("signature outside file"))?;
        let at = self.sig_patch_off as usize;
        dst.get_mut(at..at + s).ok_or(Error::BadFirmware("signature patch outside image"))?.copy_from_slice(sig);
        Ok(())
    }

    /// Bytes of GPU-visible memory `stage` needs (DMEM rounded to 256).
    pub fn staged_len(&self) -> usize {
        let end = (self.dmem_img_off + self.dmem_size) as usize;
        end.max(self.image.len())
    }
}

/// Parse a GA10x Booter (`booter_load` / `booter_unload`) for SEC2.
pub fn booter<'a>(b: &'a [u8]) -> Result<FalconUcode<'a>, Error> {
    let bad = Error::BadFirmware;
    let h = bin_hdr(b)?;
    let hs = h.header_offset as usize;
    let r = |o: usize| rd32(b, o).ok_or(bad("booter: short header"));
    // nvfw_hs_header_v2
    let sig_prod_offset = r(hs)?;
    let sig_prod_size = r(hs + 4)?;
    let patch_loc = r(hs + 8)? as usize;
    let patch_sig = r(hs + 12)? as usize;
    let meta_off = r(hs + 16)? as usize;
    let num_sig = r(hs + 24)? as usize;
    let load_off = r(hs + 28)? as usize;
    let loc = r(patch_loc)?;
    let sig = r(patch_sig)?;
    let cnt = r(num_sig)?;
    if cnt == 0 || sig_prod_size % cnt != 0 { return Err(bad("booter: bad signature count")); }
    let meta = [r(meta_off)?, r(meta_off + 4)?, r(meta_off + 8)?];
    // nvfw_hs_load_header_v2
    let os_data_offset = r(load_off + 8)?;
    let os_data_size = r(load_off + 12)?;
    let num_apps = r(load_off + 16)?;
    if num_apps == 0 { return Err(bad("booter: no apps")); }
    let app0_off = r(load_off + 20)?;
    let app0_size = r(load_off + 24)?;
    let image = &b[h.data_offset as usize..(h.data_offset + h.data_size) as usize];
    let sigs_start = (sig_prod_offset + sig) as usize;
    let sigs = b.get(sigs_start..sigs_start + sig_prod_size as usize).ok_or(bad("booter: signatures outside file"))?;
    if loc < os_data_offset || (os_data_offset + os_data_size) as usize > image.len() || (app0_off + app0_size) as usize > image.len() {
        return Err(bad("booter: sections outside image"));
    }
    Ok(FalconUcode {
        image, imem_img_off: app0_off, imem_size: app0_size, imem_base: 0,
        dmem_img_off: os_data_offset, dmem_size: os_data_size, dmem_base: 0,
        sig_patch_off: loc, sigs, sig_size: sig_prod_size / cnt, sig_count: cnt,
        fuse_ver: meta[0], engine_id: meta[1], ucode_id: meta[2],
        dmem_sign: loc - os_data_offset, boot_addr: app0_off, rule: SigRule::Version,
    })
}

/// `RM_RISCV_UCODE_DESC` fields Booter needs (via `GspFwWprMeta`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiscvDesc {
    pub version: u32,
    pub app_version: u32,
    pub manifest_offset: u32,
    pub manifest_size: u32,
    pub monitor_data_offset: u32,
    pub monitor_data_size: u32,
    pub monitor_code_offset: u32,
    pub monitor_code_size: u32,
}

/// Parse the GSP RISC-V bootloader: its descriptor and the image bytes.
pub fn bootloader(b: &[u8]) -> Result<(RiscvDesc, &[u8]), Error> {
    let h = bin_hdr(b)?;
    let o = h.header_offset as usize;
    let r = |x: usize| rd32(b, o + x).ok_or(Error::BadFirmware("bootloader: short descriptor"));
    let d = RiscvDesc {
        version: r(0)?, app_version: r(28)?, manifest_offset: r(32)?, manifest_size: r(36)?,
        monitor_data_offset: r(40)?, monitor_data_size: r(44)?, monitor_code_offset: r(48)?, monitor_code_size: r(52)?,
    };
    let data = &b[h.data_offset as usize..(h.data_offset + h.data_size) as usize];
    if d.monitor_code_offset as usize > data.len() || d.manifest_offset as usize > data.len() {
        return Err(Error::BadFirmware("bootloader: offsets outside image"));
    }
    Ok((d, data))
}

/// GSP-RM's two parts: the RISC-V image and the chip's signature for it.
pub struct GspRm<'a> { pub image: &'a [u8], pub signature: &'a [u8] }

pub fn gsp_rm<'a>(elf: &'a [u8], sig_section: &str) -> Result<GspRm<'a>, Error> {
    let image = crate::elf::section(elf, ".fwimage")?;
    let signature = crate::elf::section(elf, sig_section)?;
    if image.is_empty() { return Err(Error::BadFirmware("gsp: empty .fwimage")); }
    Ok(GspRm { image, signature })
}
