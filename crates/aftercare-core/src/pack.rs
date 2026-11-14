// SPDX-License-Identifier: GPL-3.0-only
//! Packs (many sealed blobs plus a sealed header), index files, config and locks.

use crate::codec::{Dec, Enc};
use crate::crypto::{sha256, MasterKey, Rng};
use crate::model::Timestamp;
use crate::{ChunkerParams, Error, Id};
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlobKind {
    Data,
    Tree,
}

impl BlobKind {
    fn code(self) -> u8 {
        match self {
            BlobKind::Data => 0,
            BlobKind::Tree => 1,
        }
    }
    fn from_code(c: u8) -> Result<Self, Error> {
        match c {
            0 => Ok(BlobKind::Data),
            1 => Ok(BlobKind::Tree),
            _ => Err(Error::Corrupt("unknown blob kind".into())),
        }
    }
}

/// Authenticated data for a sealed blob: binds it to its id and kind, so blobs can be
/// copied between packs unchanged but not swapped for one another.
pub(crate) fn blob_aad(id: &Id, kind: BlobKind) -> [u8; 33] {
    let mut a = [0u8; 33];
    a[..32].copy_from_slice(&id.0);
    a[32] = kind.code();
    a
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackEntry {
    pub id: Id,
    pub kind: BlobKind,
    pub offset: u32,
    /// Stored (sealed) length.
    pub length: u32,
    /// Plaintext length after decompression.
    pub raw_len: u32,
    pub compressed: bool,
}

const FLAG_ZSTD: u8 = 1;
const PACK_HEADER_MAGIC: &[u8; 4] = b"ACPH";
const PACK_VERSION: u16 = 1;
pub(crate) const PACK_HEADER_AAD: &[u8] = b"aftercare/pack-header";

#[derive(Default)]
pub(crate) struct Packer {
    pub buf: Vec<u8>,
    pub entries: Vec<PackEntry>,
    pub ids: BTreeSet<Id>,
}

impl Packer {
    pub fn add(&mut self, id: Id, kind: BlobKind, sealed: &[u8], raw_len: u32, compressed: bool) {
        self.entries.push(PackEntry {
            id,
            kind,
            offset: self.buf.len() as u32,
            length: sealed.len() as u32,
            raw_len,
            compressed,
        });
        self.ids.insert(id);
        self.buf.extend_from_slice(sealed);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The finished pack's bytes and name.
    pub fn finish(&mut self, key: &MasterKey, rng: &mut dyn Rng) -> (Id, Vec<u8>, Vec<PackEntry>) {
        let mut e = Enc::new(PACK_HEADER_MAGIC, PACK_VERSION);
        e.u32(self.entries.len() as u32);
        for en in &self.entries {
            e.u8(en.kind.code());
            e.u8(if en.compressed { FLAG_ZSTD } else { 0 });
            e.id(&en.id);
            e.u32(en.length);
            e.u32(en.raw_len);
        }
        let header = key.seal(rng, PACK_HEADER_AAD, &e.buf);
        let mut pack = core::mem::take(&mut self.buf);
        pack.extend_from_slice(&header);
        pack.extend_from_slice(&(header.len() as u32).to_le_bytes());
        self.ids.clear();
        (sha256(&pack), pack, core::mem::take(&mut self.entries))
    }
}

/// Parses a sealed pack header (the bytes before the trailing length).
pub(crate) fn decode_pack_header(key: &MasterKey, sealed: &[u8], data_len: u64) -> Result<Vec<PackEntry>, Error> {
    let plain = key.open(PACK_HEADER_AAD, sealed, "pack header")?;
    let mut d = Dec::new(&plain, PACK_HEADER_MAGIC, PACK_VERSION, "pack header")?;
    let n = d.count(42)?;
    let mut off: u64 = 0;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = BlobKind::from_code(d.u8()?)?;
        let flags = d.u8()?;
        let id = d.id()?;
        let length = d.u32()?;
        let raw_len = d.u32()?;
        out.push(PackEntry { id, kind, offset: off as u32, length, raw_len, compressed: flags & FLAG_ZSTD != 0 });
        off += length as u64;
    }
    d.finish()?;
    if off != data_len {
        return Err(Error::Corrupt("pack header does not cover the pack".into()));
    }
    Ok(out)
}

/// Splits a whole pack into (blob area length, sealed header).
pub(crate) fn split_pack(pack: &[u8]) -> Result<(usize, &[u8]), Error> {
    if pack.len() < 4 {
        return Err(Error::Corrupt("pack too short".into()));
    }
    let hl = u32::from_le_bytes(pack[pack.len() - 4..].try_into().unwrap()) as usize;
    if hl + 4 > pack.len() {
        return Err(Error::Corrupt("pack header length past start".into()));
    }
    let start = pack.len() - 4 - hl;
    Ok((start, &pack[start..pack.len() - 4]))
}

const INDEX_MAGIC: &[u8; 4] = b"ACIX";
const INDEX_VERSION: u16 = 1;
pub(crate) const INDEX_AAD: &[u8] = b"aftercare/index";

pub(crate) fn encode_index(packs: &[(Id, Vec<PackEntry>)]) -> Vec<u8> {
    let mut e = Enc::new(INDEX_MAGIC, INDEX_VERSION);
    e.u32(packs.len() as u32);
    for (pack, entries) in packs {
        e.id(pack);
        e.u32(entries.len() as u32);
        for en in entries {
            e.u8(en.kind.code());
            e.u8(if en.compressed { FLAG_ZSTD } else { 0 });
            e.id(&en.id);
            e.u32(en.offset);
            e.u32(en.length);
            e.u32(en.raw_len);
        }
    }
    e.buf
}

pub(crate) fn decode_index(plain: &[u8]) -> Result<Vec<(Id, Vec<PackEntry>)>, Error> {
    let mut d = Dec::new(plain, INDEX_MAGIC, INDEX_VERSION, "index")?;
    let np = d.count(36)?;
    let mut out = Vec::with_capacity(np);
    for _ in 0..np {
        let pack = d.id()?;
        let n = d.count(46)?;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = BlobKind::from_code(d.u8()?)?;
            let flags = d.u8()?;
            entries.push(PackEntry {
                id: d.id()?,
                kind,
                offset: d.u32()?,
                length: d.u32()?,
                raw_len: d.u32()?,
                compressed: flags & FLAG_ZSTD != 0,
            });
        }
        out.push((pack, entries));
    }
    d.finish()?;
    Ok(out)
}

const CONFIG_MAGIC: &[u8; 4] = b"ACCF";
const CONFIG_VERSION: u16 = 1;
pub(crate) const CONFIG_AAD: &[u8] = b"aftercare/config";

/// Repository-wide settings, fixed at init.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Random repository id.
    pub id: Id,
    pub chunker: ChunkerParams,
    /// Seed of the chunker's gear table (secret: only in the sealed config).
    pub(crate) gear_seed: u64,
    /// Target pack size in bytes.
    pub pack_size: u32,
    /// zstd (level Fastest) on blobs where it helps.
    pub compression: bool,
}

impl Config {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new(CONFIG_MAGIC, CONFIG_VERSION);
        e.id(&self.id);
        e.u32(self.chunker.min);
        e.u32(self.chunker.avg);
        e.u32(self.chunker.max);
        e.u64(self.gear_seed);
        e.u32(self.pack_size);
        e.u8(self.compression as u8);
        e.buf
    }
    pub(crate) fn decode(plain: &[u8]) -> Result<Self, Error> {
        let mut d = Dec::new(plain, CONFIG_MAGIC, CONFIG_VERSION, "config")?;
        let c = Config {
            id: d.id()?,
            chunker: ChunkerParams { min: d.u32()?, avg: d.u32()?, max: d.u32()? },
            gear_seed: d.u64()?,
            pack_size: d.u32()?,
            compression: d.u8()? != 0,
        };
        d.finish()?;
        if !c.chunker.is_valid() || c.pack_size == 0 {
            return Err(Error::Corrupt("config: invalid parameters".into()));
        }
        Ok(c)
    }
}

const LOCK_MAGIC: &[u8; 4] = b"ACLK";
const LOCK_VERSION: u16 = 1;
pub(crate) const LOCK_AAD: &[u8] = b"aftercare/lock";

/// A lock record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// Exclusive locks (forget, prune) exclude all others.
    pub exclusive: bool,
    pub host: String,
    pub time: Timestamp,
}

impl Lock {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new(LOCK_MAGIC, LOCK_VERSION);
        e.u8(self.exclusive as u8);
        e.str(&self.host);
        e.i64(self.time.secs);
        e.u32(self.time.nanos);
        e.buf
    }
    pub(crate) fn decode(plain: &[u8]) -> Result<Self, Error> {
        let mut d = Dec::new(plain, LOCK_MAGIC, LOCK_VERSION, "lock")?;
        let l = Lock { exclusive: d.u8()? != 0, host: d.str()?, time: Timestamp { secs: d.i64()?, nanos: d.u32()? } };
        d.finish()?;
        Ok(l)
    }
}

pub(crate) const SNAPSHOT_AAD: &[u8] = b"aftercare/snapshot";
