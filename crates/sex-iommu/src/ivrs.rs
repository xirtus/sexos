// SPDX-License-Identifier: GPL-3.0-only
//! AMD "I/O Virtualization Reporting Structure" ACPI table (AMD IOMMU spec 5.2).
//!
//! IVHD blocks describe each IOMMU and the device ids it translates; IVMD
//! blocks are memory firmware keeps using for DMA (the AMD form of RMRR).
//! Firmware often describes one IOMMU several times (types 0x10, 0x11, 0x40);
//! like every OS we read only the newest type the table offers.
use crate::{u16_at, u32_at, u64_at};

pub const SIGNATURE: &[u8; 4] = b"IVRS";
const FIRST_BLOCK: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, Signature, Length, Block }

#[derive(Clone, Copy, Debug)]
pub struct Ivrs<'a> { bytes: &'a [u8], pub info: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit<'a> {
    /// 0x10, 0x11 or 0x40: newer types carry more feature words.
    pub kind: u8,
    pub flags: u8,
    /// The IOMMU's own PCI requester id.
    pub device_id: u16,
    pub capability_offset: u16,
    pub register_base: u64,
    pub segment: u16,
    entries: &'a [u8],
}

/// Memory firmware keeps using for DMA by device ids first..=last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reserved {
    pub first: u16,
    pub last: u16,
    /// Page-aligned [start, end).
    pub start: u64,
    pub end: u64,
    pub read: bool,
    pub write: bool,
}

fn is_unit(kind: u8) -> bool { matches!(kind, 0x10 | 0x11 | 0x40) }

/// Length of the device entry starting with `kind` (`b` is the rest of the block).
fn entry_len(kind: u8, b: &[u8]) -> Option<usize> {
    Some(match kind {
        // ACPI HID entry: fixed 22 bytes, then a UID of the length in byte 21.
        0xf0 => 22 + *b.get(21)? as usize,
        k => 4 << (k >> 6),
    })
}

impl<'a> Ivrs<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < FIRST_BLOCK { return Err(Error::Short); }
        if &bytes[0..4] != SIGNATURE { return Err(Error::Signature); }
        let length = u32_at(bytes, 4).ok_or(Error::Short)? as usize;
        if length < FIRST_BLOCK || length > bytes.len() { return Err(Error::Length); }
        let bytes = &bytes[..length];
        let mut at = FIRST_BLOCK;
        while at < bytes.len() {
            let len = u16_at(bytes, at + 2).ok_or(Error::Block)? as usize;
            if len < 4 || at + len > bytes.len() { return Err(Error::Block); }
            at += len;
        }
        Ok(Self { bytes, info: u32_at(bytes, 36).ok_or(Error::Short)? })
    }

    /// Highest virtual address bits the units translate (IVinfo VASize; 0 if unstated).
    pub fn virtual_address_bits(&self) -> u8 { ((self.info >> 15) & 0x7f) as u8 }

    fn blocks(&self) -> impl Iterator<Item = &'a [u8]> + 'a {
        let bytes = self.bytes;
        let mut at = FIRST_BLOCK;
        core::iter::from_fn(move || {
            let len = u16_at(bytes, at + 2)? as usize;
            let b = bytes.get(at..at + len)?;
            at += len;
            Some(b)
        })
    }

    /// The newest IVHD type present; units of older types describe the same hardware.
    fn unit_kind(&self) -> Option<u8> { self.blocks().map(|b| b[0]).filter(|k| is_unit(*k)).max() }

    pub fn units(&self) -> impl Iterator<Item = Unit<'a>> + 'a {
        let kind = self.unit_kind();
        self.blocks().filter_map(move |b| {
            let header = if b[0] == 0x10 { 24 } else { 40 };
            if Some(b[0]) != kind || b.len() < header { return None; }
            Some(Unit { kind: b[0], flags: b[1], device_id: u16_at(b, 4)?, capability_offset: u16_at(b, 6)?,
                register_base: u64_at(b, 8)?, segment: u16_at(b, 16)?, entries: &b[header..] })
        })
    }

    /// The unit translating `device` (a requester id on segment 0).
    pub fn unit_for(&self, device: u16) -> Option<Unit<'a>> {
        self.units().find(|u| u.segment == 0 && u.covers(device))
    }

    pub fn reserved(&self) -> impl Iterator<Item = Reserved> + 'a {
        self.blocks().filter_map(|b| {
            if !matches!(b[0], 0x20..=0x22) || b.len() < 32 { return None; }
            let flags = b[1];
            // Unity mapped or an exclusion range: either way the device must keep reaching it.
            if flags & (1 | 8) == 0 { return None; }
            let (id, aux) = (u16_at(b, 4)?, u16_at(b, 6)?);
            let (first, last) = match b[0] { 0x20 => (0, 0xffff), 0x21 => (id, id), _ => (id, aux) };
            let base = u64_at(b, 16)?;
            let start = base & !(crate::PAGE - 1);
            let end = base.checked_add(u64_at(b, 24)?)?.checked_add(crate::PAGE - 1)? & !(crate::PAGE - 1);
            // Exclusion ranges grant both directions.
            let all = flags & 8 != 0;
            (end > start && first <= last).then_some(Reserved {
                first, last, start, end, read: all || flags & 2 != 0, write: all || flags & 4 != 0 })
        })
    }
}

impl<'a> Unit<'a> {
    /// Device entries: (kind, device id, alias id). Range starts carry the
    /// range end in the following entry; `covers` and `alias` walk them.
    fn entries(&self) -> impl Iterator<Item = (u8, u16, u16)> + 'a {
        let mut b = self.entries;
        core::iter::from_fn(move || {
            let kind = *b.first()?;
            let len = entry_len(kind, b)?;
            let e = b.get(..len)?;
            b = &b[len..];
            let id = if kind == 0x48 { u16_at(e, 5)? } else { u16_at(e, 1)? };
            let alias = if matches!(kind, 0x42 | 0x43) { u16_at(e, 5)? } else { id };
            Some((kind, id, alias))
        })
    }

    /// Every (first, last, alias of first..=last or None) run this unit lists.
    fn runs(&self) -> impl Iterator<Item = (u16, u16, Option<u16>)> + 'a {
        let mut entries = self.entries();
        core::iter::from_fn(move || loop {
            let (kind, id, alias) = entries.next()?;
            match kind {
                0x01 => return Some((0, 0xffff, None)),
                0x02 | 0x46 => return Some((id, id, None)),
                0x42 => return Some((id, id, Some(alias))),
                0x03 | 0x43 | 0x47 => {
                    let (end_kind, end, _) = entries.next()?;
                    if end_kind != 0x04 || end < id { continue; }
                    return Some((id, end, (kind == 0x43).then_some(alias)));
                }
                _ => continue,
            }
        })
    }

    pub fn covers(&self, device: u16) -> bool { self.runs().any(|(a, b, _)| a <= device && device <= b) }

    /// The requester id the device's DMA arrives with, when firmware says it
    /// is aliased (devices behind a PCIe-to-PCI bridge).
    pub fn alias(&self, device: u16) -> Option<u16> {
        self.runs().find(|(a, b, alias)| alias.is_some() && *a <= device && device <= *b).and_then(|r| r.2)
    }
}
