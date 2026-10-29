//! Two independently checksummed banks inside one existing workspace object.
//! Assumes ordered FLUSH and atomic sector metadata for V4 extent growth.
//! Torn journal sectors and incomplete payloads are rejected by checksum.
use sex_pdx::document as w;
pub const CAPACITY: u64 = 65_536;
pub const STRIDE: u64 = 32_768;
const PAYLOAD: u64 = 512;
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
    for (i, chunk) in bytes.chunks(512).enumerate() {
        let mut sector = [0; 512];
        sector[..chunk.len()].copy_from_slice(chunk);
        io.write(base + PAYLOAD + i as u64 * 512, &sector)?;
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
