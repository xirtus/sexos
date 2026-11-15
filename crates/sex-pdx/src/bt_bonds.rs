// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bluetooth bonds kept by Files for Cufflinks: one small file, `/system/bluetooth.bonds`
//! (the `sex_bt::bonds` format: magic, records, CRC-32), which only Cufflinks
//! (`bt_hci::CUFFLINKS_PD`) may read or write. The kernel names the caller; Files checks it.
//!
//! Request: `type_id` [`OP`], arg0 = verb | chunk << 8, arg1 per verb.
//! - [`PUT`]: arg1 = 8 bytes at `chunk * 8` of the staged file. No reply.
//! - [`SAVE`]: arg1 = length: write the staged bytes as the file. Reply: status.
//! - [`LOAD`]: read the file into the staging buffer. Reply: status | length << 8.
//! - [`GET`]: the 7 bytes at `chunk * 7` of what LOAD read. Reply: the bytes.
//!
//! Replies: [`TAG`] << 56 | 56 bits of payload, so a stray reply is never taken for one.
//! The keys are stored as they are (like any desktop's Bluetooth store): whoever can read
//! `/system` can read them.

pub const OP: u64 = 0x2C9;
pub const PATH: &[u8] = b"/system/bluetooth.bonds";
/// The largest file (`sex_bt::bonds::MAX_BYTES` rounded up).
pub const MAX_BYTES: usize = 1600;

pub const PUT: u8 = 1;
pub const SAVE: u8 = 2;
pub const LOAD: u8 = 3;
pub const GET: u8 = 4;

pub const OK: u8 = 0;
pub const NONE: u8 = 1;
pub const DENIED: u8 = 2;
pub const INVALID: u8 = 3;
pub const NOT_SAVED: u8 = 4;

pub const TAG: u64 = 0xB7;

pub fn request(verb: u8, chunk: u16) -> u64 { verb as u64 | (chunk as u64) << 8 }
pub fn decode_request(arg0: u64) -> (u8, u16) { (arg0 as u8, (arg0 >> 8) as u16) }
pub fn reply(payload: u64) -> u64 { TAG << 56 | payload & 0x00FF_FFFF_FFFF_FFFF }
pub fn decode_reply(word: u64) -> Option<u64> { (word >> 56 == TAG).then_some(word & 0x00FF_FFFF_FFFF_FFFF) }

/// Files' side: the staged file and the rules. `load`/`save` read and replace the file.
pub struct Store { buf: [u8; MAX_BYTES], len: usize }

impl Default for Store { fn default() -> Self { Self::new() } }

impl Store {
    pub const fn new() -> Store { Store { buf: [0; MAX_BYTES], len: 0 } }

    /// One request from `caller`; the word to reply with, or None (PUT).
    pub fn handle(&mut self, caller: u32, arg0: u64, arg1: u64, load: &mut dyn FnMut(&mut [u8]) -> Option<usize>,
        save: &mut dyn FnMut(&[u8]) -> bool) -> Option<u64> {
        let (verb, chunk) = decode_request(arg0);
        if caller != crate::bt_hci::CUFFLINKS_PD { return (verb != PUT).then(|| reply(DENIED as u64)); }
        let chunk = chunk as usize;
        match verb {
            PUT => {
                if let Some(d) = self.buf.get_mut(chunk * 8..chunk * 8 + 8) { d.copy_from_slice(&arg1.to_le_bytes()); }
                None
            }
            SAVE => {
                let n = arg1 as usize;
                if n > MAX_BYTES { return Some(reply(INVALID as u64)); }
                let ok = save(&self.buf[..n]);
                self.len = n;
                Some(reply(if ok { OK } else { NOT_SAVED } as u64))
            }
            LOAD => {
                self.buf.fill(0);
                self.len = load(&mut self.buf).unwrap_or(0).min(MAX_BYTES);
                Some(reply(if self.len == 0 { NONE as u64 } else { OK as u64 | (self.len as u64) << 8 }))
            }
            GET => {
                let mut b = [0u8; 8];
                if chunk * 7 < self.len { let end = (chunk * 7 + 7).min(self.len); b[..end - chunk * 7].copy_from_slice(&self.buf[chunk * 7..end]); }
                Some(reply(u64::from_le_bytes(b)))
            }
            _ => Some(reply(INVALID as u64)),
        }
    }
}

/// Cufflinks's side: the (arg0, arg1) requests that store `bytes` (PUTs, then SAVE).
pub fn save_requests(bytes: &[u8]) -> impl Iterator<Item = (u64, u64)> + '_ {
    let n = bytes.len().min(MAX_BYTES);
    bytes[..n].chunks(8).enumerate().map(|(i, c)| {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        (request(PUT, i as u16), u64::from_le_bytes(w))
    }).chain(core::iter::once((request(SAVE, 0), n as u64)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_through_the_store() {
        let mut disk: Vec<u8> = Vec::new();
        let mut s = Store::new();
        let data: Vec<u8> = (0..123u8).collect();
        let me = crate::bt_hci::CUFFLINKS_PD;
        let mut replies = Vec::new();
        for (a, b) in save_requests(&data) {
            let r = s.handle(me, a, b, &mut |_| None, &mut |bytes| { disk = bytes.to_vec(); true });
            if let Some(r) = r { replies.push(decode_reply(r).unwrap()); }
        }
        assert_eq!(replies, [OK as u64]);
        assert_eq!(disk, data);
        let mut s = Store::new();
        let loaded = s.handle(me, request(LOAD, 0), 0, &mut |out| { out[..disk.len()].copy_from_slice(&disk); Some(disk.len()) }, &mut |_| false);
        let word = decode_reply(loaded.unwrap()).unwrap();
        assert_eq!((word & 0xff, word >> 8), (OK as u64, 123));
        let mut back = Vec::new();
        for chunk in 0..(123 + 6) / 7 {
            let w = decode_reply(s.handle(me, request(GET, chunk), 0, &mut |_| None, &mut |_| false).unwrap()).unwrap();
            back.extend_from_slice(&w.to_le_bytes()[..7]);
        }
        back.truncate(123);
        assert_eq!(back, data);
        // Anyone else: refused, and PUT from them changes nothing.
        assert_eq!(s.handle(3, request(LOAD, 0), 0, &mut |_| Some(1), &mut |_| true).map(decode_reply), Some(Some(DENIED as u64)));
        assert_eq!(s.handle(3, request(PUT, 0), 0xffff, &mut |_| None, &mut |_| true), None);
        let mut empty = Store::new();
        assert_eq!(decode_reply(empty.handle(me, request(LOAD, 0), 0, &mut |_| None, &mut |_| true).unwrap()), Some(NONE as u64));
        assert_eq!(decode_reply(0x1234), None);
    }
}
