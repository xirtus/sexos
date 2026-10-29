//! Two independently checksummed banks inside one existing workspace object.
//! Assumes ordered FLUSH and atomic sector metadata for V4 extent growth.
//! Torn journal sectors and incomplete payloads are rejected by checksum.
use sex_pdx::document as w;
pub const CAPACITY: u64 = 65_536;
pub const STRIDE: u64 = 32_768;
const PAYLOAD: u64 = 512;
const IO_CHUNK:usize=sex_pdx::BLOCK_MAX_XFER as usize;
const _:()=assert!(IO_CHUNK>=512 && IO_CHUNK%512==0);
pub trait Io {
    fn read(&mut self, offset: u64, out: &mut [u8]) -> Result<(), u32>;
    fn write(&mut self, offset: u64, bytes: &[u8]) -> Result<(), u32>;
    fn sync(&mut self) -> Result<(), u32>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub bank: u8,
    pub generation: u64,
    pub len: usize,
    pub hash: u64,
}
fn header(object: u8, identity: u32, r: Receipt) -> [u8; 512] {
    let mut h = [0; 512];
    h[..4].copy_from_slice(b"QJ02");
    h[4] = object;
    h[8..12].copy_from_slice(&identity.to_le_bytes());
    h[16..24].copy_from_slice(&r.generation.to_le_bytes());
    h[24..28].copy_from_slice(&(r.len as u32).to_le_bytes());
    h[32..40].copy_from_slice(&r.hash.to_le_bytes());
    let sum = w::hash(&h[..504]);
    h[504..].copy_from_slice(&sum.to_le_bytes());
    h
}
fn candidate(
    io: &mut impl Io,
    object: u8,
    identity: u32,
    bank: u8,
    out: &mut [u8; w::MAX],
) -> Result<Option<Receipt>, u32> {
    let mut h = [0; 512];
    io.read(bank as u64 * STRIDE, &mut h)?;
    if &h[..4] != b"QJ02"
        || h[4] != object
        || u32::from_le_bytes(h[8..12].try_into().unwrap()) != identity
        || w::hash(&h[..504]) != u64::from_le_bytes(h[504..].try_into().unwrap())
    {
        return Ok(None);
    }
    let r = Receipt {
        bank,
        generation: u64::from_le_bytes(h[16..24].try_into().unwrap()),
        len: u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize,
        hash: u64::from_le_bytes(h[32..40].try_into().unwrap()),
    };
    if r.generation == 0 || r.len > w::MAX {
        return Ok(None);
    }
    io.read(bank as u64 * STRIDE + PAYLOAD, &mut out[..r.len])?;
    if w::hash(&out[..r.len]) != r.hash {
        return Ok(None);
    }
    Ok(Some(r))
}
pub fn recover(
    io: &mut impl Io,
    object: u8,
    identity: u32,
    out: &mut [u8; w::MAX],
) -> Result<Option<Receipt>, u32> {
    // Never interpret a read failure as an empty/old bank: effects may exist.
    let a = candidate(io, object, identity, 0, out)?;
    let b = candidate(io, object, identity, 1, out)?;
    let best = match (a, b) {
        (Some(a), Some(b)) if a.generation == b.generation => return Err(w::STALE),
        (Some(a), Some(b)) => Some(if a.generation > b.generation { a } else { b }),
        (a, b) => a.or(b),
    };
    if let Some(r) = best {
        if candidate(io, object, identity, r.bank, out)? != Some(r) {
            return Err(w::IO);
        }
    } else {
        out.fill(0);
    }
    Ok(best)
}
pub fn save(
    io: &mut impl Io,
    object: u8,
    identity: u32,
    bytes: &[u8],
    scratch: &mut [u8; w::MAX],
) -> Result<Receipt, u32> {
    if bytes.len() > w::MAX {
        return Err(w::INVALID);
    }
    let old = recover(io, object, identity, scratch)?;
    // Initial conversion writes bank1, beyond the complete bounded legacy QP01.
    let bank = old.map_or(1, |r| 1 - r.bank);
    let generation = old
        .map_or(Some(1), |r| r.generation.checked_add(1))
        .ok_or(w::STALE)?;
    let r = Receipt {
        bank,
        generation,
        len: bytes.len(),
        hash: w::hash(bytes),
    };
    let base = bank as u64 * STRIDE;
    io.write(base, &[0; 512])?;
    io.sync()?;
    for (i, chunk) in bytes.chunks(IO_CHUNK).enumerate() {
        let mut sector = [0; IO_CHUNK];
        sector[..chunk.len()].copy_from_slice(chunk);
        let padded=(chunk.len()+511)/512*512;
        io.write(base + PAYLOAD + i as u64 * IO_CHUNK as u64, &sector[..padded])?;
    }
    io.sync()?;
    // Verify payload before publishing the only commit record.
    io.read(base + PAYLOAD, &mut scratch[..bytes.len()])?;
    if &scratch[..bytes.len()] != bytes {
        return Err(w::IO);
    }
    let h = header(object, identity, r);
    io.write(base, &h)?;
    io.sync()?;
    let got = candidate(io, object, identity, bank, scratch)?;
    if got != Some(r) || &scratch[..bytes.len()] != bytes {
        return Err(w::IO);
    }
    Ok(r)
}

/// Cooperative save cursor. Each step performs at most one logical read or
/// write of at most the existing DMA page, or one FLUSH. The VFS may split a
/// logical operation at exact extent boundaries. State remains service-owned
/// between turns; small metadata records remain exactly one sector.
/// Dropping the cursor cancels progress, never rolls back completed writes.
pub struct Save {
    phase: u8,
    object: u8,
    identity: u32,
    bank: u8,
    offset: usize,
    sum: u64,
    candidate: Option<Receipt>,
    old: [Option<Receipt>; 2],
    next: Option<Receipt>,
}
impl Save {
    pub const fn new(object: u8, identity: u32) -> Self {
        Self {
            phase: 0,
            object,
            identity,
            bank: 0,
            offset: 0,
            sum: 0xcbf29ce484222325,
            candidate: None,
            old: [None; 2],
            next: None,
        }
    }
    pub fn step(&mut self, io: &mut impl Io, bytes: &[u8]) -> Result<Option<Receipt>, u32> {
        let result = self.step_inner(io, bytes);
        if result.is_err() {
            self.phase = 12;
        }
        result
    }
    fn step_inner(&mut self, io: &mut impl Io, bytes: &[u8]) -> Result<Option<Receipt>, u32> {
        if bytes.len() > w::MAX {
            return Err(w::INVALID);
        }
        let mut sector = [0; IO_CHUNK];
        match self.phase {
            0 => {
                io.read(self.bank as u64 * STRIDE, &mut sector[..512])?;
                self.candidate = if &sector[..4] == b"QJ02"
                    && sector[4] == self.object
                    && u32::from_le_bytes(sector[8..12].try_into().unwrap()) == self.identity
                    && w::hash(&sector[..504])
                        == u64::from_le_bytes(sector[504..512].try_into().unwrap())
                {
                    let r = Receipt {
                        bank: self.bank,
                        generation: u64::from_le_bytes(sector[16..24].try_into().unwrap()),
                        len: u32::from_le_bytes(sector[24..28].try_into().unwrap()) as usize,
                        hash: u64::from_le_bytes(sector[32..40].try_into().unwrap()),
                    };
                    if r.generation != 0 && r.len <= w::MAX {
                        Some(r)
                    } else {
                        None
                    }
                } else {
                    None
                };
                self.offset = 0;
                self.sum = 0xcbf29ce484222325;
                self.phase = 1;
            }
            1 => {
                if let Some(r) = self.candidate {
                    if self.offset < r.len {
                        let n = (r.len - self.offset).min(IO_CHUNK);
                        io.read(
                            self.bank as u64 * STRIDE + PAYLOAD + self.offset as u64,
                            &mut sector[..n],
                        )?;
                        for &b in &sector[..n] {
                            self.sum = (self.sum ^ b as u64).wrapping_mul(0x100000001b3);
                        }
                        self.offset += n;
                        return Ok(None);
                    }
                    if self.sum == r.hash {
                        self.old[self.bank as usize] = Some(r);
                    }
                }
                if self.bank == 0 {
                    self.bank = 1;
                    self.phase = 0;
                } else {
                    let old = match (self.old[0], self.old[1]) {
                        (Some(a), Some(b)) if a.generation == b.generation => return Err(w::STALE),
                        (Some(a), Some(b)) => Some(if a.generation > b.generation { a } else { b }),
                        (a, b) => a.or(b),
                    };
                    self.bank = old.map_or(1, |r| 1 - r.bank);
                    self.next = Some(Receipt {
                        bank: self.bank,
                        generation: old
                            .map_or(Some(1), |r| r.generation.checked_add(1))
                            .ok_or(w::STALE)?,
                        len: bytes.len(),
                        hash: w::hash(bytes),
                    });
                    self.phase = 2;
                }
            }
            2 => {
                io.write(self.bank as u64 * STRIDE, &sector[..512])?;
                self.phase = 3;
            }
            3 => {
                io.sync()?;
                self.offset = 0;
                self.phase = 4;
            }
            4 => {
                if self.offset < bytes.len() {
                    let n = (bytes.len() - self.offset).min(IO_CHUNK);
                    sector[..n].copy_from_slice(&bytes[self.offset..self.offset + n]);
                    io.write(
                        self.bank as u64 * STRIDE + PAYLOAD + self.offset as u64,
                        &sector[..(n+511)/512*512],
                    )?;
                    self.offset += n;
                } else {
                    self.phase = 5;
                }
            }
            5 => {
                io.sync()?;
                self.offset = 0;
                self.phase = 6;
            }
            6 | 10 => {
                if self.offset < bytes.len() {
                    let n = (bytes.len() - self.offset).min(IO_CHUNK);
                    io.read(
                        self.bank as u64 * STRIDE + PAYLOAD + self.offset as u64,
                        &mut sector[..n],
                    )?;
                    if sector[..n] != bytes[self.offset..self.offset + n] {
                        return Err(w::IO);
                    }
                    self.offset += n;
                } else if self.phase == 6 {
                    self.phase = 7;
                } else {
                    self.phase = 11;
                    return Ok(self.next);
                }
            }
            7 => {
                io.write(
                    self.bank as u64 * STRIDE,
                    &header(self.object, self.identity, self.next.ok_or(w::INVALID)?),
                )?;
                self.phase = 8;
            }
            8 => {
                io.sync()?;
                self.phase = 9;
            }
            9 => {
                io.read(self.bank as u64 * STRIDE, &mut sector[..512])?;
                if sector[..512] != header(self.object, self.identity, self.next.ok_or(w::INVALID)?) {
                    return Err(w::IO);
                }
                self.offset = 0;
                self.phase = 10;
            }
            // A completed or failed cursor must never replay effects.
            _ => return Err(w::STALE),
        }
        Ok(None)
    }
}
