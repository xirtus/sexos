// SPDX-License-Identifier: GPL-3.0-only
//! Developer-only bounded cross-parent rendezvous, not a production app/service.
#![no_std]
pub const SECRET: &[u8] = b"Ribbon Seal inherited-file fixture: 2026-11-15\n";
pub const MARKER: &[u8] = b"ribbon-owner-20261115";
pub const CHANNEL: &[u8] = b"@/ribbon-xchg";
pub const PRIVATE: &[u8] = b"@/ribbon-private";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record(pub [u32; 12]);
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}
impl Record {
    pub const fn empty() -> Self {
        Self([0; 12])
    }
    pub fn encode(self) -> [u8; 64] {
        let mut b = [0; 64];
        b[..8].copy_from_slice(b"RIBSEAL1");
        for (i, v) in self.0.iter().enumerate() {
            b[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes())
        }
        let h = hash(&b[..56]);
        b[56..].copy_from_slice(&h.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 64
            || &b[..8] != b"RIBSEAL1"
            || u64::from_le_bytes(b[56..].try_into().ok()?) != hash(&b[..56])
        {
            return None;
        }
        let mut r = Self::empty();
        for i in 0..12 {
            r.0[i] = u32::from_le_bytes(b[8 + i * 4..12 + i * 4].try_into().ok()?)
        }
        if r.0[0] > 6 || r.0[9..].iter().any(|v| *v != 0) {
            return None;
        }
        Some(r)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_record_and_torn_wrong_version_fail_closed() {
        let mut r = Record::empty();
        r.0[..4].copy_from_slice(&[1, 77, 42, 1073741824]);
        let b = r.encode();
        assert_eq!(Record::decode(&b), Some(r));
        assert_eq!(Record::decode(&b[..63]), None);
        for i in 0..64 {
            let mut bad = b;
            bad[i] ^= 1;
            assert_eq!(Record::decode(&bad), None)
        }
    }
}
