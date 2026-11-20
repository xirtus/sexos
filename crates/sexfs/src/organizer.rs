// SPDX-License-Identifier: GPL-3.0-only
//! Durable, bounded placement log. Only the Files server may call these methods
//! for a BOOT-authorized organizer request. The log and move share one COW commit.
use super::{checksum, Device, Error, Fs, Kind, Result};

const ENABLED: &[u8] = b"/.organizer-enabled";
const UNDO: &[u8] = b"/.organizer-undo";
const PINS: &[u8] = b"/.organizer-pins";
const RULES: &[u8] = b"/.organizer-rules";
/// Pinned files (by id) the organizer never moves.
pub const MAX_PINS: u64 = 256;
/// The person's rules, newline separated (`sex_organize::parse_rule`).
pub const RULES_MAX: usize = 4096;
const MAGIC: &[u8; 4] = b"ORG1";
pub const RECORD_BYTES: usize = 1024;
pub const MAX_RECORDS: u64 = 32;
const END_SUM: usize = RECORD_BYTES - 8;

pub fn enabled<D: Device>(fs: &mut Fs<'_, D>) -> Result<bool> {
    match fs.open(ENABLED) {
        Ok(st) if st.kind == Kind::File => Ok(true),
        Ok(_) => Err(Error::Corrupt),
        Err(Error::NotFound) => Ok(false),
        Err(e) => Err(e),
    }
}

/// This change is committed by the Files server before the reply is sent.
pub fn set_enabled<D: Device>(fs: &mut Fs<'_, D>, on: bool) -> Result<()> {
    if on {
        if !enabled(fs)? { fs.create(ENABLED, 0)?; }
    } else if enabled(fs)? {
        fs.remove_path(ENABLED)?;
    }
    Ok(())
}

fn canonical(path: &[u8]) -> bool {
    path.first() == Some(&b'/') && path.len() > 1 && !path.contains(&0) && path.last() != Some(&b'/')
        && path[1..].split(|&b| b == b'/').all(|p| !p.is_empty() && p != b"." && p != b"..")
}
fn file_name(path: &[u8]) -> &[u8] { path.rsplit(|&b| b == b'/').next().unwrap_or(path) }

/// A placement keeps the file's name and only changes its folder. Which folders may be
/// used is the Files server's policy (`sex_organize::ingress`/`destination_allowed`),
/// checked before it asks; the log itself refuses anything that isn't a plain move.
fn allowed(from: &[u8], to: &[u8]) -> bool {
    canonical(from) && canonical(to) && from != to && file_name(from) == file_name(to)
        && !file_name(from).starts_with(b".") && !from.starts_with(b"/.") && !to.starts_with(b"/.")
}

fn encode(from: &[u8], to: &[u8], id: u64) -> Result<[u8; RECORD_BYTES]> {
    if !allowed(from, to) || id == 0 || from.len() + to.len() > END_SUM - 16 {
        return Err(Error::Name);
    }
    let mut record = [0; RECORD_BYTES];
    record[..4].copy_from_slice(MAGIC);
    record[4..12].copy_from_slice(&id.to_le_bytes());
    record[12..14].copy_from_slice(&(from.len() as u16).to_le_bytes());
    record[14..16].copy_from_slice(&(to.len() as u16).to_le_bytes());
    record[16..16 + from.len()].copy_from_slice(from);
    record[16 + from.len()..16 + from.len() + to.len()].copy_from_slice(to);
    let sum = checksum(&record[..END_SUM]);
    record[END_SUM..].copy_from_slice(&sum.to_le_bytes());
    Ok(record)
}

fn decode(record: &[u8; RECORD_BYTES]) -> Result<(&[u8], &[u8], u64)> {
    if &record[..4] != MAGIC || u64::from_le_bytes(record[END_SUM..].try_into().unwrap()) != checksum(&record[..END_SUM]) {
        return Err(Error::Corrupt);
    }
    let id = u64::from_le_bytes(record[4..12].try_into().unwrap());
    let from_len = u16::from_le_bytes(record[12..14].try_into().unwrap()) as usize;
    let to_len = u16::from_le_bytes(record[14..16].try_into().unwrap()) as usize;
    if id == 0 || from_len + to_len > END_SUM - 16 { return Err(Error::Corrupt); }
    let from = &record[16..16 + from_len];
    let to = &record[16 + from_len..16 + from_len + to_len];
    if !allowed(from, to) { return Err(Error::Corrupt); }
    Ok((from, to, id))
}

/// Move and append a durable undo record in the same uncommitted transaction.
/// The last 32 moves remain undoable; when full, the oldest record rolls off.
pub fn move_and_record<D: Device>(fs: &mut Fs<'_, D>, from: &[u8], to: &[u8], id: u64) -> Result<u64> {
    let record = encode(from, to, id)?;
    let (ledger, size) = match fs.open(UNDO) {
        Ok(st) if st.kind == Kind::File => (st.id, st.size),
        Ok(_) => return Err(Error::Corrupt),
        Err(Error::NotFound) => (fs.create(UNDO, 0)?, 0),
        Err(e) => return Err(e),
    };
    if size % RECORD_BYTES as u64 != 0 { return Err(Error::Corrupt); }
    if size / RECORD_BYTES as u64 > MAX_RECORDS { return Err(Error::Corrupt); }
    fs.move_file_if_id(from, to, id)?;
    let at = if size / RECORD_BYTES as u64 == MAX_RECORDS {
        let mut previous = [0; RECORD_BYTES];
        for index in 1..MAX_RECORDS {
            let source = index * RECORD_BYTES as u64;
            if fs.read(ledger, source, &mut previous)? != RECORD_BYTES { return Err(Error::Corrupt); }
            if fs.write(ledger, source - RECORD_BYTES as u64, &previous, 0)? != RECORD_BYTES { return Err(Error::Io); }
        }
        size - RECORD_BYTES as u64
    } else { size };
    if fs.write(ledger, at, &record, 0)? != RECORD_BYTES { return Err(Error::Io); }
    Ok((at / RECORD_BYTES as u64 + 1).min(MAX_RECORDS))
}

/// Restore the last moved object if its identity still matches and its old name is free.
/// The server commits this with the log truncation as one transaction.
pub fn undo_last<D: Device>(fs: &mut Fs<'_, D>) -> Result<u64> {
    let ledger = fs.open(UNDO)?;
    if ledger.kind != Kind::File || ledger.size == 0 { return Err(Error::NotFound); }
    if ledger.size % RECORD_BYTES as u64 != 0 { return Err(Error::Corrupt); }
    let at = ledger.size - RECORD_BYTES as u64;
    let mut record = [0; RECORD_BYTES];
    if fs.read(ledger.id, at, &mut record)? != RECORD_BYTES { return Err(Error::Corrupt); }
    let (from, to, id) = decode(&record)?;
    fs.move_file_if_id(to, from, id)?;
    fs.truncate(ledger.id, at, 0)?;
    // The restored file stays where the person wanted it: pinned, so automatic
    // organization doesn't move it straight back. `organize unpin` lets it go again.
    pin(fs, id, true)?;
    Ok(ledger.size / RECORD_BYTES as u64 - 1)
}

/// The last move still undoable, as (from, to): for showing before an undo.
pub fn last<D: Device>(fs: &mut Fs<'_, D>, out: &mut [u8; RECORD_BYTES]) -> Result<Option<(usize, usize)>> {
    let ledger = match fs.open(UNDO) { Ok(l) => l, Err(Error::NotFound) => return Ok(None), Err(e) => return Err(e) };
    if ledger.kind != Kind::File || ledger.size == 0 { return Ok(None); }
    if ledger.size % RECORD_BYTES as u64 != 0 { return Err(Error::Corrupt); }
    if fs.read(ledger.id, ledger.size - RECORD_BYTES as u64, out)? != RECORD_BYTES { return Err(Error::Corrupt); }
    let (from, to, _) = decode(out)?;
    Ok(Some((from.len(), to.len())))
}

fn pins_file<D: Device>(fs: &mut Fs<'_, D>, create: bool) -> Result<Option<(u64, u64)>> {
    match fs.open(PINS) {
        Ok(st) if st.kind == Kind::File && st.size % 8 == 0 && st.size <= MAX_PINS * 8 => Ok(Some((st.id, st.size))),
        Ok(_) => Err(Error::Corrupt),
        Err(Error::NotFound) if create => Ok(Some((fs.create(PINS, 0)?, 0))),
        Err(Error::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}
fn pin_index<D: Device>(fs: &mut Fs<'_, D>, file: u64, size: u64, id: u64) -> Result<Option<u64>> {
    let mut word = [0u8; 8];
    for at in (0..size).step_by(8) {
        if fs.read(file, at, &mut word)? != 8 { return Err(Error::Corrupt); }
        if u64::from_le_bytes(word) == id { return Ok(Some(at)); }
    }
    Ok(None)
}

/// Whether the file with this id is pinned in place.
pub fn pinned<D: Device>(fs: &mut Fs<'_, D>, id: u64) -> Result<bool> {
    let Some((file, size)) = pins_file(fs, false)? else { return Ok(false) };
    Ok(pin_index(fs, file, size, id)?.is_some())
}

/// Pins or unpins a file by id; the server commits it. Full: `NoSpace`.
pub fn pin<D: Device>(fs: &mut Fs<'_, D>, id: u64, on: bool) -> Result<()> {
    if id == 0 { return Err(Error::Name); }
    let Some((file, size)) = pins_file(fs, on)? else { return Ok(()) };
    match (pin_index(fs, file, size, id)?, on) {
        (Some(_), true) | (None, false) => Ok(()),
        (None, true) => {
            if size / 8 >= MAX_PINS { return Err(Error::NoSpace); }
            if fs.write(file, size, &id.to_le_bytes(), 0)? != 8 { return Err(Error::Io); }
            Ok(())
        }
        (Some(at), false) => {
            // The last entry takes the removed one's place.
            let mut word = [0u8; 8];
            if fs.read(file, size - 8, &mut word)? != 8 { return Err(Error::Corrupt); }
            if fs.write(file, at, &word, 0)? != 8 { return Err(Error::Io); }
            fs.truncate(file, size - 8, 0)
        }
    }
}

/// How many files are pinned.
pub fn pin_count<D: Device>(fs: &mut Fs<'_, D>) -> Result<u64> {
    Ok(pins_file(fs, false)?.map_or(0, |(_, size)| size / 8))
}

/// Pinned ids, for listing. Returns how many were written.
pub fn pins<D: Device>(fs: &mut Fs<'_, D>, out: &mut [u64]) -> Result<usize> {
    let Some((file, size)) = pins_file(fs, false)? else { return Ok(0) };
    let mut word = [0u8; 8];
    let mut n = 0;
    for at in (0..size).step_by(8) {
        if n == out.len() { break; }
        if fs.read(file, at, &mut word)? != 8 { return Err(Error::Corrupt); }
        out[n] = u64::from_le_bytes(word); n += 1;
    }
    Ok(n)
}

/// The person's rules text. Returns its length.
pub fn rules<D: Device>(fs: &mut Fs<'_, D>, out: &mut [u8; RULES_MAX]) -> Result<usize> {
    match fs.open(RULES) {
        Ok(st) if st.kind == Kind::File && st.size as usize <= RULES_MAX => fs.read(st.id, 0, &mut out[..st.size as usize]),
        Ok(_) => Err(Error::Corrupt),
        Err(Error::NotFound) => Ok(0),
        Err(e) => Err(e),
    }
}

/// Replaces the rules text (already checked by the server); the server commits it.
pub fn set_rules<D: Device>(fs: &mut Fs<'_, D>, text: &[u8]) -> Result<()> {
    if text.len() > RULES_MAX || text.contains(&0) { return Err(Error::TooBig); }
    let id = match fs.open(RULES) {
        Ok(st) if st.kind == Kind::File => st.id,
        Ok(_) => return Err(Error::Corrupt),
        Err(Error::NotFound) => fs.create(RULES, 0)?,
        Err(e) => return Err(e),
    };
    fs.truncate(id, 0, 0)?;
    if !text.is_empty() && fs.write(id, 0, text, 0)? != text.len() { return Err(Error::Io); }
    Ok(())
}
