// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Checking a signed SOF firmware image (`sof-tgl-h.ri`) before it is loaded.
//!
//! An IPC3 SOF image for cAVS 1.8+ (Tiger Lake included) is two things back to back:
//!
//! 1. An optional SOF *extended manifest* (`XMan`): a small header and a list of
//!    typed elements (firmware version, mailbox windows, compiler, ...). The host
//!    reads it and strips it; it never goes to the DSP.
//! 2. The *payload*, which the ROM receives over the code-loader DMA stream. It is
//!    an Intel CSE code partition directory (`$CPD`, partition `ADSP`) listing the
//!    signed manifest (`ADSP.man`, a CSS header `$MN2` with the RSA signature), the
//!    metadata (`*.met`) and the firmware module itself. The firmware descriptor
//!    `$AM1` (version, preload size, module count) sits 8 KiB into the payload.
//!
//! The ROM checks the signature; we only check that the structure is sane enough to
//! reject garbage early and to log what is being loaded. Nothing is allocated: the
//! result points into the caller's image. Layout studied from Linux
//! `include/sound/sof/ext_manifest.h`, `sound/soc/sof/ipc3-loader.c` and SOF's
//! `rimage` (BSD-3-Clause).

use crate::regs::{dsp, Region};

/// `XMan`, first word of an extended manifest.
pub const EXT_MAN_MAGIC: u32 = 0x6e61_4d58;
/// Extended manifest version this code understands (major 1; only major is checked).
pub const EXT_MAN_VERSION_MAJOR: u32 = 1;
/// Extended manifest element: firmware version (a `sof_ipc_fw_version`).
pub const EXT_ELEM_FW_VERSION: u32 = 0;
/// Extended manifest element: SRAM window layout (a `sof_ipc_window`).
pub const EXT_ELEM_WINDOW: u32 = 1;
/// `$CPD`, the code partition directory marker.
pub const CPD_MAGIC: [u8; 4] = *b"$CPD";
/// `$MN2`, the CSS manifest header id.
pub const CSS_MAGIC: [u8; 4] = *b"$MN2";
/// `$AM1`, the ADSP firmware descriptor id.
pub const ADSP_FW_MAGIC: [u8; 4] = *b"$AM1";
/// Where rimage puts the `$AM1` descriptor, from the start of the payload (cAVS 1.8+).
pub const ADSP_FW_DESC_OFFSET: usize = 0x2000;
/// CSS header type of a module manifest.
pub const CSS_HEADER_TYPE: u32 = 4;
/// SOF window type (`enum sof_ipc_region`) of the host-to-DSP mailbox.
const REGION_DOWNBOX: u32 = 0;
/// SOF window type of the DSP-to-host mailbox.
const REGION_UPBOX: u32 = 1;
/// SOF caps window lists at this many entries.
const MAX_WINDOWS: u32 = 16;

/// Why an image was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Shorter than the headers it claims to have.
    Truncated,
    /// The extended manifest's sizes do not add up.
    BadExtManifest,
    /// The extended manifest has a major version we do not know.
    ExtManifestVersion(u32),
    /// An extended manifest element at this file offset has an impossible size.
    BadExtElement(usize),
    /// The payload does not start with `$CPD`.
    NoCodePartition,
    /// The partition directory is malformed (header length, entry count, entries
    /// outside the payload).
    BadCodePartition,
    /// No `*.man` entry, or it is not a CSS `$MN2` manifest.
    NoSignedManifest,
    /// No `$AM1` firmware descriptor 8 KiB into the payload.
    NoFirmwareDescriptor,
    /// The payload length is not a multiple of 4 bytes or is too small to DMA.
    BadPayloadSize,
}

/// A version quadruple.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Version {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Micro (SOF) or hotfix (`$AM1`) version.
    pub micro: u16,
    /// Build number.
    pub build: u16,
}

/// The SOF version the extended manifest declares (same fields FW_READY reports).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SofVersion {
    /// Release version.
    pub version: Version,
    /// IPC ABI version: major in bits 31:24, minor 23:12, patch 11:0.
    pub abi: u32,
    /// Short git tag, NUL padded.
    pub tag: [u8; 6],
}

/// The host and DSP mailboxes the image declares (also repeated in FW_READY).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mailboxes {
    /// Host-to-DSP requests and their replies (SOF `DOWNBOX`, "outbox").
    pub host: Region,
    /// DSP-to-host notifications and FW_READY (SOF `UPBOX`, "inbox").
    pub dsp: Region,
}

/// What a valid image looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    /// Bytes of extended manifest in front of the payload (0 when absent).
    pub payload_offset: usize,
    /// Bytes the ROM receives over DMA.
    pub payload_len: usize,
    /// From the extended manifest, when present.
    pub sof: Option<SofVersion>,
    /// From the extended manifest window element, when present.
    pub mailboxes: Option<Mailboxes>,
    /// Firmware version from the `$AM1` descriptor.
    pub fw_version: Version,
    /// Number of module entries in `$AM1`.
    pub modules: u32,
    /// 4 KiB pages the ROM preloads.
    pub preload_pages: u32,
    /// Number of code partition directory entries.
    pub partition_entries: u32,
}

impl Info {
    /// The bytes to copy into the code-loader DMA buffer.
    pub fn payload<'a>(&self, image: &'a [u8]) -> &'a [u8] {
        &image[self.payload_offset..self.payload_offset + self.payload_len]
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Checks `image` and describes it.
pub fn check(image: &[u8]) -> Result<Info, Error> {
    let mut info = Info {
        payload_offset: 0,
        payload_len: 0,
        sof: None,
        mailboxes: None,
        fw_version: Version::default(),
        modules: 0,
        preload_pages: 0,
        partition_entries: 0,
    };
    if u32_at(image, 0).ok_or(Error::Truncated)? == EXT_MAN_MAGIC {
        ext_manifest(image, &mut info)?;
    }
    let payload = &image[info.payload_offset..];
    info.payload_len = payload.len();
    if payload.len() < ADSP_FW_DESC_OFFSET || payload.len() % 4 != 0 {
        return Err(Error::BadPayloadSize);
    }
    partition(payload, &mut info)?;
    descriptor(payload, &mut info)?;
    Ok(info)
}

fn ext_manifest(image: &[u8], info: &mut Info) -> Result<(), Error> {
    let full = u32_at(image, 4).ok_or(Error::Truncated)? as usize;
    let header = u32_at(image, 8).ok_or(Error::Truncated)? as usize;
    let version = u32_at(image, 12).ok_or(Error::Truncated)?;
    if header < 16 || full < header || full >= image.len() {
        return Err(Error::BadExtManifest);
    }
    if version >> 24 != EXT_MAN_VERSION_MAJOR {
        return Err(Error::ExtManifestVersion(version));
    }
    let mut at = header;
    while full - at > 8 {
        let kind = u32_at(image, at).ok_or(Error::Truncated)?;
        let size = u32_at(image, at + 4).ok_or(Error::Truncated)? as usize;
        if size < 8 || size > full - at {
            return Err(Error::BadExtElement(at));
        }
        let elem = &image[at..at + size];
        match kind {
            EXT_ELEM_FW_VERSION => info.sof = sof_version(elem),
            EXT_ELEM_WINDOW => info.mailboxes = windows(elem),
            _ => {}
        }
        at += size;
    }
    if at != full {
        return Err(Error::BadExtManifest);
    }
    info.payload_offset = full;
    Ok(())
}

/// Element header (8) + `sof_ipc_fw_version`: size (4), major, minor, micro, build
/// (2 each), date[12], time[10], tag[6], abi_version (4).
fn sof_version(e: &[u8]) -> Option<SofVersion> {
    let v = 8;
    let mut tag = [0u8; 6];
    tag.copy_from_slice(e.get(v + 34..v + 40)?);
    Some(SofVersion {
        version: Version {
            major: u16_at(e, v + 4)?,
            minor: u16_at(e, v + 6)?,
            micro: u16_at(e, v + 8)?,
            build: u16_at(e, v + 10)?,
        },
        abi: u32_at(e, v + 40)?,
        tag,
    })
}

/// Element header (8) + `sof_ipc_window`: cmd header (8), type (4), num_windows (4),
/// then `num_windows` x { size, type, id, flags, size, offset } (24 bytes).
fn windows(e: &[u8]) -> Option<Mailboxes> {
    let count = u32_at(e, 20)?.min(MAX_WINDOWS) as usize;
    let (mut host, mut dsp_box) = (None, None);
    for i in 0..count {
        let w = 24 + 24 * i;
        let kind = u32_at(e, w + 4)?;
        let id = u32_at(e, w + 8)?;
        let size = u32_at(e, w + 16)?;
        let offset = u32_at(e, w + 20)?;
        if id >= dsp::SRAM_WINDOWS || offset.checked_add(size)? > dsp::SRAM_WINDOW_STRIDE {
            continue;
        }
        let region = Region { offset: dsp::sram_window(id) + offset, size };
        match kind {
            REGION_DOWNBOX => host = Some(region),
            REGION_UPBOX => dsp_box = Some(region),
            _ => {}
        }
    }
    Some(Mailboxes { host: host?, dsp: dsp_box? })
}

/// `$CPD` header: marker (4), entry count (4), header version (1), entry version
/// (1), header length (1), checksum (1), partition name (4), [crc32 (4)]; then
/// entries of name[12], offset (4, low 25 bits), length (4), reserved (4).
fn partition(p: &[u8], info: &mut Info) -> Result<(), Error> {
    if p.get(0..4) != Some(&CPD_MAGIC[..]) {
        return Err(Error::NoCodePartition);
    }
    let count = u32_at(p, 4).ok_or(Error::Truncated)?;
    let header = *p.get(10).ok_or(Error::Truncated)? as usize;
    if !(1..=32).contains(&count) || header < 0x10 {
        return Err(Error::BadCodePartition);
    }
    let mut signed = false;
    for i in 0..count as usize {
        let e = header + 24 * i;
        let name = p.get(e..e + 12).ok_or(Error::BadCodePartition)?;
        let offset = (u32_at(p, e + 12).ok_or(Error::BadCodePartition)? & 0x01ff_ffff) as usize;
        let len = u32_at(p, e + 16).ok_or(Error::BadCodePartition)? as usize;
        if offset.checked_add(len).map_or(true, |end| end > p.len()) {
            return Err(Error::BadCodePartition);
        }
        let n = name.iter().position(|&c| c == 0).unwrap_or(12);
        if name[..n].ends_with(b".man") {
            signed = css(&p[offset..offset + len]);
        }
    }
    if !signed {
        return Err(Error::NoSignedManifest);
    }
    info.partition_entries = count;
    Ok(())
}

/// CSS header: type (4) = 4, header length in dwords (4), header version (4),
/// module id (4), vendor (4), date (4), size in dwords (4), id `$MN2`.
fn css(m: &[u8]) -> bool {
    let (Some(kind), Some(hlen), Some(size)) = (u32_at(m, 0), u32_at(m, 4), u32_at(m, 24)) else {
        return false;
    };
    let (hlen, size) = (hlen as usize * 4, size as usize * 4);
    kind == CSS_HEADER_TYPE && m.get(28..32) == Some(&CSS_MAGIC[..]) && hlen <= size && size <= m.len()
}

/// `$AM1`: id (4), header length (4), name[8], preload pages (4), image flags (4),
/// feature mask (4), major, minor, hotfix, build (2 each), module count (4), ...
fn descriptor(p: &[u8], info: &mut Info) -> Result<(), Error> {
    let d = &p[ADSP_FW_DESC_OFFSET..];
    if d.get(0..4) != Some(&ADSP_FW_MAGIC[..]) || u32_at(d, 4).map_or(true, |l| l < 0x34) {
        return Err(Error::NoFirmwareDescriptor);
    }
    let get = |at| u16_at(d, at).ok_or(Error::NoFirmwareDescriptor);
    info.preload_pages = u32_at(d, 16).ok_or(Error::NoFirmwareDescriptor)?;
    info.fw_version = Version { major: get(28)?, minor: get(30)?, micro: get(32)?, build: get(34)? };
    info.modules = u32_at(d, 36).ok_or(Error::NoFirmwareDescriptor)?;
    Ok(())
}
