// SPDX-License-Identifier: GPL-3.0-only
//! Durable, bounded placement log. Only the Files server may call these methods
//! for a BOOT-authorized organizer request. The log and move share one COW commit.
use crate::{checksum, Device, Error, Fs, Kind, Result};

const ENABLED: &[u8] = b"/.organizer-enabled";
const UNDO: &[u8] = b"/.organizer-undo";
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

fn allowed(from: &[u8], to: &[u8]) -> bool {
    let Some(name) = from.strip_prefix(b"/Inbox/") else { return false };
    !name.is_empty() && !name.contains(&b'/') &&
        (to.strip_prefix(b"/Documents/") == Some(name) ||
         to.strip_prefix(b"/Pictures/") == Some(name))
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

/// Restore the last moved object if its identity still matches and Inbox is free.
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
    // Restoring into Inbox must not be immediately reversed by auto mode.
    // A person can explicitly re-enable it after reviewing the restored file.
    set_enabled(fs, false)?;
    Ok(ledger.size / RECORD_BYTES as u64 - 1)
}
