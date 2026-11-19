// SPDX-License-Identifier: GPL-3.0-only
//! Virtual disk containers, read-only: raw, qcow2, VHD, VHDX and VMDK.

use alloc::vec::Vec;
use core::cell::RefCell;

use crate::part::{self, FsKind, Partition, Slice};
use crate::util::{be32, be64, le16, le32, le64, read_exact, read_padded, read_vec};
use crate::{ByteSource, Error, IoError, Kind, Result};

/// The container a [`BlockDevice`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Raw,
    Qcow2,
    Vhd,
    Vhdx,
    Vmdk,
}

impl Format {
    pub fn kind(self) -> Kind {
        match self {
            Format::Raw => Kind::Raw,
            Format::Qcow2 => Kind::Qcow2,
            Format::Vhd => Kind::Vhd,
            Format::Vhdx => Kind::Vhdx,
            Format::Vmdk => Kind::Vmdk,
        }
    }
}

/// Container formats recognised by their signatures (raw has none).
pub(crate) fn detect<S: ByteSource + ?Sized>(src: &S) -> Result<Option<Format>> {
    let head = read_padded(src, 0, 512)?;
    if &head[0..4] == b"QFI\xfb" {
        return Ok(Some(Format::Qcow2));
    }
    if &head[0..8] == b"vhdxfile" {
        return Ok(Some(Format::Vhdx));
    }
    if &head[0..4] == b"KDMV" || head.starts_with(b"# Disk DescriptorFile") {
        return Ok(Some(Format::Vmdk));
    }
    if &head[0..8] == b"conectix" {
        return Ok(Some(Format::Vhd));
    }
    if src.len() >= 1024 {
        let foot = read_vec(src, src.len() - 512, 8)?;
        if &foot[..] == b"conectix" {
            return Ok(Some(Format::Vhd));
        }
    }
    Ok(None)
}

enum Inner {
    Raw,
    Qcow2(Qcow2),
    Vhd(Vhd),
    Vhdx(Vhdx),
    Vmdk(Vmdk),
}

/// A virtual disk seen as a run of bytes. Unallocated areas read as zeros.
/// It is a [`ByteSource`], so it can be handed to [`crate::Image::open`]
/// again (an ISO inside a qcow2) or to filesystem code through
/// [`BlockDevice::slice`].
pub struct BlockDevice<S> {
    src: S,
    format: Format,
    inner: Inner,
    size: u64,
    sector: u32,
}

impl<S: ByteSource> BlockDevice<S> {
    /// Detects the container (raw when nothing else matches).
    pub fn open(src: S) -> Result<Self> {
        let f = detect(&src)?.unwrap_or(Format::Raw);
        Self::open_as(src, f)
    }

    pub fn open_as(src: S, format: Format) -> Result<Self> {
        let (inner, size, sector) = match format {
            Format::Raw => (Inner::Raw, src.len(), 512),
            Format::Qcow2 => {
                let q = Qcow2::open(&src)?;
                let s = q.size;
                (Inner::Qcow2(q), s, 512)
            }
            Format::Vhd => {
                let v = Vhd::open(&src)?;
                let s = v.size;
                (Inner::Vhd(v), s, 512)
            }
            Format::Vhdx => {
                let v = Vhdx::open(&src)?;
                let (s, ss) = (v.size, v.sector);
                (Inner::Vhdx(v), s, ss)
            }
            Format::Vmdk => {
                let v = Vmdk::open(&src)?;
                let s = v.size;
                (Inner::Vmdk(v), s, 512)
            }
        };
        Ok(BlockDevice { src, format, inner, size, sector })
    }

    pub fn format(&self) -> Format {
        self.format
    }
    /// Size of the virtual disk in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }
    /// Logical sector size the disk presents (512, or what a VHDX declares).
    pub fn sector_size(&self) -> u32 {
        self.sector
    }
    pub fn source(&self) -> &S {
        &self.src
    }
    pub fn into_source(self) -> S {
        self.src
    }

    /// Read virtual-disk bytes at `offset`; returns how many were read (fewer at
    /// cluster boundaries, 0 at the end).
    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let max = ((self.size - offset).min(buf.len() as u64)) as usize;
        let buf = &mut buf[..max];
        let mut done = 0;
        while done < buf.len() {
            let at = offset + done as u64;
            let b = &mut buf[done..];
            let n = match &self.inner {
                Inner::Raw => {
                    let n = self.src.read_at(at, b)?;
                    if n == 0 {
                        return Err(Error::Corrupt("image ends early"));
                    }
                    n
                }
                Inner::Qcow2(q) => q.read(&self.src, at, b)?,
                Inner::Vhd(v) => v.read(&self.src, at, b)?,
                Inner::Vhdx(v) => v.read(&self.src, at, b)?,
                Inner::Vmdk(v) => v.read(&self.src, at, b)?,
            };
            done += n;
        }
        Ok(done)
    }

    /// The partitions of an MBR or GPT disk (empty when the disk has no table).
    /// Tables are read in 512-byte sectors.
    pub fn partitions(&self) -> Result<Vec<Partition>> {
        if self.sector != 512 {
            return Err(Error::Unsupported("partition tables on 4096-byte-sector disks"));
        }
        part::read_partitions(self)
    }

    /// One partition as its own byte source, for filesystem code.
    pub fn slice(&self, p: &Partition) -> Slice<'_, Self> {
        Slice::new(self, p.offset, p.len)
    }

    /// The filesystem written straight onto the disk, if there is no table.
    pub fn filesystem(&self) -> FsKind {
        part::probe_filesystem(self, 0, self.size)
    }
}

impl<S: ByteSource> ByteSource for BlockDevice<S> {
    fn len(&self) -> u64 {
        self.size
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        self.read(offset, buf).map_err(crate::util::io_err)
    }
}

fn zero(buf: &mut [u8], n: usize) -> Result<usize> {
    buf[..n].fill(0);
    Ok(n)
}

// ---------------------------------------------------------------- qcow2

struct Qcow2 {
    size: u64,
    cluster_bits: u32,
    l1_offset: u64,
    l1_size: u64,
    /// Last decompressed cluster: (host offset of the compressed data, bytes).
    cache: RefCell<Option<(u64, Vec<u8>)>>,
}

const QCOW_OFFSET_MASK: u64 = 0x00ff_ffff_ffff_fe00;

impl Qcow2 {
    fn open<S: ByteSource + ?Sized>(src: &S) -> Result<Self> {
        let h = read_padded(src, 0, 112)?;
        let version = be32(&h, 4);
        if version != 2 && version != 3 {
            return Err(Error::Unsupported("qcow2 version"));
        }
        if be64(&h, 8) != 0 {
            return Err(Error::Unsupported("qcow2 images with a backing file"));
        }
        let cluster_bits = be32(&h, 20);
        if !(9..=21).contains(&cluster_bits) {
            return Err(Error::Corrupt("qcow2 cluster size"));
        }
        let size = be64(&h, 24);
        if be32(&h, 32) != 0 {
            return Err(Error::Unsupported("encrypted qcow2 images"));
        }
        let l1_size = be32(&h, 36) as u64;
        let l1_offset = be64(&h, 40);
        if version == 3 {
            let incompat = be64(&h, 72);
            if incompat & (1 << 2) != 0 {
                return Err(Error::Unsupported("qcow2 with an external data file"));
            }
            if incompat & (1 << 3) != 0 && be32(&h, 100) > 104 && h[104] != 0 {
                return Err(Error::Unsupported("qcow2 compressed with zstd"));
            }
            if incompat & (1 << 4) != 0 {
                return Err(Error::Unsupported("qcow2 with subclusters"));
            }
            if incompat & !0x1f != 0 {
                return Err(Error::Unsupported("qcow2 feature"));
            }
        }
        let cs = 1u64 << cluster_bits;
        let per_l2 = cs / 8;
        let need = size.div_ceil(cs).div_ceil(per_l2);
        if l1_size < need || l1_offset.checked_add(l1_size * 8).is_none_or(|e| e > src.len()) {
            return Err(Error::Corrupt("qcow2 L1 table"));
        }
        Ok(Qcow2 { size, cluster_bits, l1_offset, l1_size, cache: RefCell::new(None) })
    }

    fn read<S: ByteSource + ?Sized>(&self, src: &S, off: u64, buf: &mut [u8]) -> Result<usize> {
        let cb = self.cluster_bits;
        let cs = 1u64 << cb;
        let within = off & (cs - 1);
        let n = ((cs - within) as usize).min(buf.len());
        let idx = off >> cb;
        let per_l2 = cs / 8;
        let l1i = idx / per_l2;
        if l1i >= self.l1_size {
            return zero(buf, n);
        }
        let mut e = [0u8; 8];
        read_exact(src, self.l1_offset + l1i * 8, &mut e)?;
        let l2 = u64::from_be_bytes(e) & QCOW_OFFSET_MASK;
        if l2 == 0 {
            return zero(buf, n);
        }
        read_exact(src, l2 + (idx % per_l2) * 8, &mut e)?;
        let l2e = u64::from_be_bytes(e);
        if l2e & (1 << 62) != 0 {
            let x = 62 - (cb - 8);
            let host = l2e & ((1u64 << x) - 1);
            let sectors = ((l2e >> x) & ((1u64 << (cb - 8)) - 1)) + 1;
            let csize = (sectors * 512 - (host & 511)) as usize;
            let mut cache = self.cache.borrow_mut();
            if cache.as_ref().is_none_or(|c| c.0 != host) {
                let data = read_padded(src, host, csize)?;
                let mut out = miniz_oxide::inflate::decompress_to_vec_with_limit(&data, cs as usize)
                    .map_err(|_| Error::Corrupt("qcow2 compressed cluster"))?;
                out.resize(cs as usize, 0);
                *cache = Some((host, out));
            }
            let c = &cache.as_ref().unwrap().1;
            buf[..n].copy_from_slice(&c[within as usize..within as usize + n]);
            return Ok(n);
        }
        let host = l2e & QCOW_OFFSET_MASK;
        if host == 0 || l2e & 1 != 0 {
            return zero(buf, n);
        }
        read_exact(src, host + within, &mut buf[..n])?;
        Ok(n)
    }
}

// ---------------------------------------------------------------- VHD

struct Vhd {
    size: u64,
    /// Dynamic disks: (BAT offset, entries, block size).
    dynamic: Option<(u64, u64, u64)>,
}

fn vhd_footer_ok(f: &[u8]) -> bool {
    if f.len() < 512 || &f[0..8] != b"conectix" {
        return false;
    }
    let sum = f[..512].iter().enumerate().filter(|(i, _)| !(64..68).contains(i)).fold(0u32, |s, (_, b)| s.wrapping_add(*b as u32));
    !sum == be32(f, 64)
}

impl Vhd {
    fn open<S: ByteSource + ?Sized>(src: &S) -> Result<Self> {
        let len = src.len();
        let tail = if len >= 512 { read_vec(src, len - 512, 512)? } else { Vec::new() };
        let head = read_padded(src, 0, 512)?;
        let f = if vhd_footer_ok(&tail) {
            tail
        } else if vhd_footer_ok(&head) {
            head
        } else {
            return Err(Error::Corrupt("VHD footer checksum"));
        };
        let size = be64(&f, 48);
        match be32(&f, 60) {
            2 => {
                if size > len.saturating_sub(512) {
                    return Err(Error::Corrupt("VHD shorter than its size"));
                }
                Ok(Vhd { size, dynamic: None })
            }
            3 => {
                let dh = read_vec(src, be64(&f, 16), 1024)?;
                if &dh[0..8] != b"cxsparse" {
                    return Err(Error::Corrupt("VHD dynamic header"));
                }
                let bat = be64(&dh, 16);
                let entries = be32(&dh, 28) as u64;
                let block = be32(&dh, 32) as u64;
                if !block.is_power_of_two() || block < 512 || entries < size.div_ceil(block) {
                    return Err(Error::Corrupt("VHD block table"));
                }
                Ok(Vhd { size, dynamic: Some((bat, entries, block)) })
            }
            4 => Err(Error::Unsupported("differencing VHD (needs its parent)")),
            _ => Err(Error::Unsupported("VHD disk type")),
        }
    }

    fn read<S: ByteSource + ?Sized>(&self, src: &S, off: u64, buf: &mut [u8]) -> Result<usize> {
        let Some((bat, entries, bs)) = self.dynamic else {
            let n = src.read_at(off, buf)?;
            return if n == 0 { Err(Error::Corrupt("image ends early")) } else { Ok(n) };
        };
        let bi = off / bs;
        let within = off % bs;
        let n = ((bs - within) as usize).min(buf.len());
        if bi >= entries {
            return zero(buf, n);
        }
        let mut e = [0u8; 4];
        read_exact(src, bat + bi * 4, &mut e)?;
        let sector = u32::from_be_bytes(e);
        if sector == u32::MAX {
            return zero(buf, n);
        }
        let bitmap = (bs / 512).div_ceil(8).next_multiple_of(512);
        read_exact(src, sector as u64 * 512 + bitmap + within, &mut buf[..n])?;
        Ok(n)
    }
}

// ---------------------------------------------------------------- VHDX

/// CRC-32C (Castagnoli), as VHDX checksums its headers.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
        }
    }
    !c
}

const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
}

pub(crate) const VHDX_BAT: [u8; 16] = guid(0x2DC27766, 0xF623, 0x4200, [0x9D, 0x64, 0x11, 0x5E, 0x9B, 0xFD, 0x4A, 0x08]);
pub(crate) const VHDX_META: [u8; 16] = guid(0x8B7CA206, 0x4790, 0x4B9A, [0xB8, 0xFE, 0x57, 0x5F, 0x05, 0x0F, 0x88, 0x6E]);
pub(crate) const VHDX_FILE_PARAMS: [u8; 16] = guid(0xCAA16737, 0xFA36, 0x4D43, [0xB3, 0xB6, 0x33, 0xF0, 0xAA, 0x44, 0xE7, 0x6B]);
pub(crate) const VHDX_DISK_SIZE: [u8; 16] = guid(0x2FA54224, 0xCD1B, 0x4876, [0xB2, 0x11, 0x5D, 0xBE, 0xD8, 0x3B, 0xF4, 0xB8]);
pub(crate) const VHDX_LOGICAL_SECTOR: [u8; 16] = guid(0x8141BF1D, 0xA96F, 0x4709, [0xBA, 0x47, 0xF2, 0x33, 0xA8, 0xFA, 0xAB, 0x5F]);

struct Vhdx {
    size: u64,
    sector: u32,
    block: u64,
    chunk_ratio: u64,
    bat: u64,
    bat_len: u64,
}

impl Vhdx {
    fn open<S: ByteSource + ?Sized>(src: &S) -> Result<Self> {
        const KB: u64 = 1024;
        let mut best: Option<(u64, Vec<u8>)> = None;
        for at in [64 * KB, 128 * KB] {
            let Ok(mut h) = read_vec(src, at, 4096) else { continue };
            if &h[0..4] != b"head" {
                continue;
            }
            let sum = le32(&h, 4);
            h[4..8].fill(0);
            if crc32c(&h) != sum {
                continue;
            }
            let seq = le64(&h, 8);
            if best.as_ref().is_none_or(|b| seq > b.0) {
                best = Some((seq, h));
            }
        }
        let (_, h) = best.ok_or(Error::Corrupt("no valid VHDX header"))?;
        if h[48..64].iter().any(|&b| b != 0) {
            return Err(Error::Unsupported("VHDX with a log to replay (not shut down cleanly)"));
        }
        let mut regions: Option<Vec<u8>> = None;
        for at in [192 * KB, 256 * KB] {
            let Ok(mut r) = read_vec(src, at, 64 * KB as usize) else { continue };
            if &r[0..4] != b"regi" {
                continue;
            }
            let sum = le32(&r, 4);
            r[4..8].fill(0);
            if crc32c(&r) == sum {
                regions = Some(r);
                break;
            }
        }
        let r = regions.ok_or(Error::Corrupt("no valid VHDX region table"))?;
        let (mut bat, mut meta) = (None, None);
        for i in 0..(le32(&r, 8) as usize).min(2047) {
            let e = &r[16 + i * 32..48 + i * 32];
            let off = le64(e, 16);
            let len = le32(e, 24) as u64;
            if e[0..16] == VHDX_BAT {
                bat = Some((off, len));
            } else if e[0..16] == VHDX_META {
                meta = Some((off, len));
            } else if le32(e, 28) & 1 != 0 {
                return Err(Error::Unsupported("VHDX required region"));
            }
        }
        let (bat, bat_len) = bat.ok_or(Error::Corrupt("VHDX has no block table"))?;
        let (meta, meta_len) = meta.ok_or(Error::Corrupt("VHDX has no metadata"))?;
        let mt = read_vec(src, meta, 64 * KB as usize)?;
        if &mt[0..8] != b"metadata" {
            return Err(Error::Corrupt("VHDX metadata table"));
        }
        let item = |id: &[u8; 16], len: usize| -> Result<Option<Vec<u8>>> {
            for i in 0..(le16(&mt, 10) as usize).min(2047) {
                let e = &mt[32 + i * 32..64 + i * 32];
                if &e[0..16] == id {
                    let off = le32(e, 16) as u64;
                    if off + len as u64 > meta_len {
                        return Err(Error::Corrupt("VHDX metadata item"));
                    }
                    return Ok(Some(read_vec(src, meta + off, len)?));
                }
            }
            Ok(None)
        };
        let fp = item(&VHDX_FILE_PARAMS, 8)?.ok_or(Error::Corrupt("VHDX file parameters"))?;
        let block = le32(&fp, 0) as u64;
        if le32(&fp, 4) & 2 != 0 {
            return Err(Error::Unsupported("differencing VHDX (needs its parent)"));
        }
        let size = le64(&item(&VHDX_DISK_SIZE, 8)?.ok_or(Error::Corrupt("VHDX disk size"))?, 0);
        let sector = item(&VHDX_LOGICAL_SECTOR, 4)?.map(|v| le32(&v, 0)).unwrap_or(512);
        if !block.is_power_of_two() || !(1 << 20..=256 << 20).contains(&block) || !matches!(sector, 512 | 4096) {
            return Err(Error::Corrupt("VHDX block or sector size"));
        }
        let chunk_ratio = ((1u64 << 23) * sector as u64) / block;
        Ok(Vhdx { size, sector, block, chunk_ratio, bat, bat_len })
    }

    fn read<S: ByteSource + ?Sized>(&self, src: &S, off: u64, buf: &mut [u8]) -> Result<usize> {
        let bi = off / self.block;
        let within = off % self.block;
        let n = ((self.block - within) as usize).min(buf.len());
        let idx = bi + bi / self.chunk_ratio;
        if (idx + 1) * 8 > self.bat_len {
            return Err(Error::Corrupt("VHDX block table too short"));
        }
        let mut e = [0u8; 8];
        read_exact(src, self.bat + idx * 8, &mut e)?;
        let e = u64::from_le_bytes(e);
        match e & 7 {
            6 => {
                read_exact(src, (e >> 20 << 20) + within, &mut buf[..n])?;
                Ok(n)
            }
            7 => Err(Error::Unsupported("partially present VHDX block (needs its parent)")),
            _ => zero(buf, n),
        }
    }
}

// ---------------------------------------------------------------- VMDK

struct Vmdk {
    size: u64,
    grain: u64,
    gtes: u64,
    gd: u64,
    compressed: bool,
    cache: RefCell<Option<(u64, Vec<u8>)>>,
}

impl Vmdk {
    fn open<S: ByteSource + ?Sized>(src: &S) -> Result<Self> {
        let mut h = read_padded(src, 0, 512)?;
        if h.starts_with(b"# Disk DescriptorFile") {
            return Err(Error::Unsupported("VMDK descriptor with separate extent files"));
        }
        if &h[0..4] != b"KDMV" {
            return Err(Error::NotRecognized);
        }
        let flags = le32(&h, 8);
        let desc_off = le64(&h, 28);
        let desc_len = le64(&h, 36);
        if le64(&h, 56) == u64::MAX && src.len() >= 1024 {
            // streamOptimized: the real header is the footer near the end.
            let f = read_vec(src, src.len() - 1024, 512)?;
            if &f[0..4] == b"KDMV" {
                h = f;
            }
        }
        if desc_off > 0 && desc_len > 0 {
            let d = read_padded(src, desc_off * 512, (desc_len * 512).min(64 * 1024) as usize)?;
            if let Some(p) = find(&d, b"parentCID=") {
                let v = &d[p + 10..];
                if !v.starts_with(b"ffffffff") {
                    return Err(Error::Unsupported("VMDK with a parent disk"));
                }
            }
        }
        let size = le64(&h, 12) * 512;
        let grain = le64(&h, 20) * 512;
        let gtes = le32(&h, 44) as u64;
        let gd = le64(&h, 56);
        let compressed = flags & (1 << 16) != 0;
        if compressed && le16(&h, 77) != 1 {
            return Err(Error::Unsupported("VMDK compression"));
        }
        if !grain.is_power_of_two() || grain < 4096 || gtes == 0 || gtes > 65536 || gd == 0 || gd == u64::MAX {
            return Err(Error::Corrupt("VMDK header"));
        }
        Ok(Vmdk { size, grain, gtes, gd: gd * 512, compressed, cache: RefCell::new(None) })
    }

    fn read<S: ByteSource + ?Sized>(&self, src: &S, off: u64, buf: &mut [u8]) -> Result<usize> {
        let gi = off / self.grain;
        let within = off % self.grain;
        let n = ((self.grain - within) as usize).min(buf.len());
        let mut e = [0u8; 4];
        read_exact(src, self.gd + gi / self.gtes * 4, &mut e)?;
        let gt = u32::from_le_bytes(e) as u64;
        if gt == 0 {
            return zero(buf, n);
        }
        read_exact(src, gt * 512 + gi % self.gtes * 4, &mut e)?;
        let g = u32::from_le_bytes(e) as u64;
        if g <= 1 {
            return zero(buf, n);
        }
        if !self.compressed {
            read_exact(src, g * 512 + within, &mut buf[..n])?;
            return Ok(n);
        }
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.0 != g) {
            let m = read_vec(src, g * 512, 12)?;
            let clen = le32(&m, 8) as usize;
            if clen as u64 > self.grain * 2 {
                return Err(Error::Corrupt("VMDK compressed grain"));
            }
            let data = read_vec(src, g * 512 + 12, clen)?;
            let mut out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, self.grain as usize)
                .map_err(|_| Error::Corrupt("VMDK compressed grain"))?;
            out.resize(self.grain as usize, 0);
            *cache = Some((g, out));
        }
        let c = &cache.as_ref().unwrap().1;
        buf[..n].copy_from_slice(&c[within as usize..within as usize + n]);
        Ok(n)
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
