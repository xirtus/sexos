// SPDX-License-Identifier: GPL-3.0-only
//! Intel "DMA Remapping" ACPI table (VT-d spec chapter 8).
//!
//! The table lists remapping hardware units (DRHD), each with the PCI devices
//! it translates, and reserved memory (RMRR) that firmware keeps using for DMA
//! after boot, such as USB keyboard emulation or the graphics stolen memory.
use crate::{u16_at, u64_at, Bdf};

pub const SIGNATURE: &[u8; 4] = b"DMAR";
/// Header (36) + host address width, flags, 10 reserved bytes.
const FIRST_STRUCTURE: usize = 48;

pub const FLAG_INTR_REMAP: u8 = 1 << 0;
pub const FLAG_X2APIC_OPT_OUT: u8 = 1 << 1;
pub const FLAG_DMA_CTRL_PLATFORM_OPT_IN: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, Signature, Length, Structure }

#[derive(Clone, Copy, Debug)]
pub struct Dmar<'a> {
    bytes: &'a [u8],
    /// Highest DMA address bit the platform can generate, plus one.
    pub host_address_width: u8,
    pub flags: u8,
}

/// A remapping hardware unit: one register block translating some devices.
#[derive(Clone, Copy, Debug)]
pub struct Drhd<'a> {
    /// Translates every device on `segment` that no other unit lists.
    pub include_all: bool,
    pub segment: u16,
    pub register_base: u64,
    /// Register block size in bytes (the `size` field, 2^(n+12)).
    pub register_bytes: u64,
    scopes: &'a [u8],
}

/// Memory firmware keeps using for DMA by the listed devices.
#[derive(Clone, Copy, Debug)]
pub struct Rmrr<'a> {
    pub segment: u16,
    pub base: u64,
    /// Inclusive, as the table stores it.
    pub limit: u64,
    scopes: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub enum Structure<'a> {
    Drhd(Drhd<'a>),
    Rmrr(Rmrr<'a>),
    /// ATSR, RHSA, ANDD, SATC and anything newer: not needed for remapping.
    Other(u16),
}

impl<'a> Dmar<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < FIRST_STRUCTURE { return Err(Error::Short); }
        if &bytes[0..4] != SIGNATURE { return Err(Error::Signature); }
        let length = crate::u32_at(bytes, 4).ok_or(Error::Short)? as usize;
        if length < FIRST_STRUCTURE || length > bytes.len() { return Err(Error::Length); }
        let bytes = &bytes[..length];
        let dmar = Self { bytes, host_address_width: bytes[36].saturating_add(1), flags: bytes[37] };
        // Reject a malformed structure list up front, so iteration never has to.
        let mut at = FIRST_STRUCTURE;
        while at < bytes.len() {
            let len = u16_at(bytes, at + 2).ok_or(Error::Structure)? as usize;
            if len < 4 || at + len > bytes.len() { return Err(Error::Structure); }
            at += len;
        }
        Ok(dmar)
    }

    pub fn structures(&self) -> Structures<'a> { Structures { bytes: self.bytes, at: FIRST_STRUCTURE } }

    pub fn units(&self) -> impl Iterator<Item = Drhd<'a>> {
        self.structures().filter_map(|s| match s { Structure::Drhd(d) => Some(d), _ => None })
    }

    pub fn reserved(&self) -> impl Iterator<Item = Rmrr<'a>> {
        self.structures().filter_map(|s| match s { Structure::Rmrr(r) => Some(r), _ => None })
    }

    /// The unit translating `device`: one that lists it, else the segment's
    /// INCLUDE_PCI_ALL unit. `secondary` resolves bridge paths (see `Scope`).
    pub fn unit_for(&self, segment: u16, device: Bdf, secondary: &impl Fn(Bdf) -> Option<u8>) -> Option<Drhd<'a>> {
        self.units().filter(|u| u.segment == segment && !u.include_all)
            .find(|u| u.scopes().any(|s| s.covers(device, secondary)))
            .or_else(|| self.units().find(|u| u.segment == segment && u.include_all))
    }
}

pub struct Structures<'a> { bytes: &'a [u8], at: usize }

impl<'a> Iterator for Structures<'a> {
    type Item = Structure<'a>;
    fn next(&mut self) -> Option<Structure<'a>> {
        let b = self.bytes;
        let kind = u16_at(b, self.at)?;
        let len = u16_at(b, self.at + 2)? as usize;
        let s = b.get(self.at..self.at + len)?;
        self.at += len;
        Some(match kind {
            0 if len >= 16 => Structure::Drhd(Drhd {
                include_all: s[4] & 1 != 0,
                register_bytes: 1u64 << (12 + (s[5] & 0x0f) as u32),
                segment: u16_at(s, 6)?,
                register_base: u64_at(s, 8)?,
                scopes: &s[16..],
            }),
            1 if len >= 24 => Structure::Rmrr(Rmrr {
                segment: u16_at(s, 6)?, base: u64_at(s, 8)?, limit: u64_at(s, 16)?, scopes: &s[24..],
            }),
            other => Structure::Other(other),
        })
    }
}

impl<'a> Drhd<'a> {
    pub fn scopes(&self) -> Scopes<'a> { Scopes { bytes: self.scopes } }
}

impl<'a> Rmrr<'a> {
    pub fn scopes(&self) -> Scopes<'a> { Scopes { bytes: self.scopes } }
    /// Page-aligned [start, end) span, or None for a nonsensical entry.
    pub fn span(&self) -> Option<(u64, u64)> {
        let start = self.base & !(crate::PAGE - 1);
        let end = self.limit.checked_add(1)?.checked_add(crate::PAGE - 1)? & !(crate::PAGE - 1);
        (end > start).then_some((start, end))
    }
}

/// Device scope entry types.
pub const SCOPE_ENDPOINT: u8 = 1;
pub const SCOPE_BRIDGE: u8 = 2;
pub const SCOPE_IOAPIC: u8 = 3;
pub const SCOPE_HPET: u8 = 4;
pub const SCOPE_ACPI: u8 = 5;

#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    pub kind: u8,
    pub enumeration_id: u8,
    pub start_bus: u8,
    /// (device, function) pairs, one per bridge hop, last one the target.
    path: &'a [u8],
}

impl<'a> Scope<'a> {
    pub fn path(&self) -> impl Iterator<Item = (u8, u8)> + 'a {
        self.path.chunks_exact(2).map(|p| (p[0], p[1]))
    }

    /// Walk the path to the device it names. Each hop but the last is a bridge
    /// whose secondary bus number `secondary` reads from config space.
    pub fn resolve(&self, secondary: &impl Fn(Bdf) -> Option<u8>) -> Option<Bdf> {
        let mut bus = self.start_bus;
        let mut hops = self.path().peekable();
        while let Some((dev, func)) = hops.next() {
            let here = Bdf::new(bus, dev, func);
            if hops.peek().is_none() { return Some(here); }
            bus = secondary(here)?;
        }
        None
    }

    /// An endpoint scope names exactly that device; a bridge scope also covers
    /// everything below the bridge (buses secondary..=subordinate are not in
    /// the table, so this checks the bridge's own secondary bus only).
    pub fn covers(&self, device: Bdf, secondary: &impl Fn(Bdf) -> Option<u8>) -> bool {
        let Some(target) = self.resolve(secondary) else { return false };
        match self.kind {
            SCOPE_ENDPOINT => target == device,
            SCOPE_BRIDGE => target == device || secondary(target).is_some_and(|b| b == device.bus),
            _ => false,
        }
    }
}

pub struct Scopes<'a> { bytes: &'a [u8] }

impl<'a> Iterator for Scopes<'a> {
    type Item = Scope<'a>;
    fn next(&mut self) -> Option<Scope<'a>> {
        let b = self.bytes;
        let len = *b.get(1)? as usize;
        if len < 6 || len > b.len() { self.bytes = &[]; return None; }
        let s = &b[..len];
        self.bytes = &b[len..];
        Some(Scope { kind: s[0], enumeration_id: s[4], start_bus: s[5], path: &s[6..] })
    }
}
