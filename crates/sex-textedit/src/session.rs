// SPDX-License-Identifier: GPL-3.0-only
//! Unsaved work that outlives a crash or a reboot. The editor writes the tabs that have
//! unsaved changes (their path, the text as it is, the cursor and how the file is stored)
//! to a file in its own folder a moment after each change, and reads them back when it
//! starts. A tab with nothing unsaved is not kept: the file on disk already is the text.

use crate::encoding::{Encoding, Eol, Format};
use alloc::vec::Vec;

const MAGIC: &[u8] = b"JOTSESSION1\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// Where it saves to (empty for a tab never saved).
    pub path: Vec<u8>,
    pub text: Vec<u8>,
    pub cursor: usize,
    pub format: Format,
}

fn put(out: &mut Vec<u8>, n: u64) { out.extend_from_slice(&n.to_le_bytes()); }

pub fn write(tabs: &[Saved]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    put(&mut out, tabs.len() as u64);
    for t in tabs {
        put(&mut out, t.path.len() as u64);
        out.extend_from_slice(&t.path);
        put(&mut out, t.cursor as u64);
        out.push(t.format.encoding.to_byte());
        out.push((t.format.eol == Eol::CrLf) as u8);
        put(&mut out, t.text.len() as u64);
        out.extend_from_slice(&t.text);
    }
    // A checksum, so a session cut short by a crash mid-write is ignored, not half read.
    let sum = crate::fnv(&out);
    put(&mut out, sum);
    out
}

/// The tabs in a session file, or nothing if it is damaged or from another version.
pub fn read(bytes: &[u8]) -> Option<Vec<Saved>> {
    if bytes.len() < MAGIC.len() + 16 || !bytes.starts_with(MAGIC) { return None; }
    let (body, sum) = bytes.split_at(bytes.len() - 8);
    if crate::fnv(body) != u64::from_le_bytes(sum.try_into().ok()?) { return None; }
    let mut at = MAGIC.len();
    let mut take = |n: usize| -> Option<&[u8]> { let s = body.get(at..at + n)?; at += n; Some(s) };
    let num = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap()) as usize;
    let count = num(take(8)?);
    let mut tabs = Vec::new();
    for _ in 0..count.min(1024) {
        let plen = num(take(8)?);
        let path = take(plen)?.to_vec();
        let cursor = num(take(8)?);
        let enc = take(1)?[0];
        let crlf = take(1)?[0] == 1;
        let tlen = num(take(8)?);
        let text = take(tlen)?.to_vec();
        tabs.push(Saved { path, text, cursor, format: Format { encoding: Encoding::from_byte(enc), eol: if crlf { Eol::CrLf } else { Eol::Lf } } });
    }
    Some(tabs)
}
