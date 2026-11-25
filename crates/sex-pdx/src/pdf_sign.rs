// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Vellum ↔ Shell ↔ Shibari signing route. Only public certificate bytes, a
//! bounded CMS attribute set, and the resulting DER signature cross this route.
//! The private scalar stays in the Shibari program's protection view.

pub const REGISTER: u64 = 0x6A0;
pub const LIST: u64 = 0x6A1;
pub const CERT: u64 = 0x6A2;
pub const POINT: u64 = 0x6A3;
pub const BEGIN: u64 = 0x6A4;
pub const CHUNK: u64 = 0x6A5;
pub const FINISH: u64 = 0x6A6;
pub const STATUS: u64 = 0x6A7;
pub const SIGNATURE: u64 = 0x6A8;
pub const END: u64 = 0x6A9;
pub const MAX_CERT: usize = 32 * 1024;
pub const POINT_BYTES: usize = 65;
pub const MAX_ATTRS: usize = 8192;
pub const MAX_SIGNATURE: usize = 80;
/// Status after the signer explicitly declined or closed.
pub const REFUSED: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signer { pub row: u32, pub certificate_bytes: usize }

pub fn pack(bytes: &[u8]) -> u64 {
    let mut word = [0u8; 8];
    word[..bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
    u64::from_le_bytes(word)
}
pub fn unpack(word: u64, into: &mut [u8]) {
    let n = into.len().min(8);
    into[..n].copy_from_slice(&word.to_le_bytes()[..n]);
}

// Serialized request/reply client operations live in `sex-app::pdf_sign`, which
// can preserve unrelated window messages in its mailbox while waiting.

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_words_preserve_non_aligned_tail_without_exposing_padding() {
        let mut result = [0u8; 13];
        unpack(pack(b"12345678"), &mut result[..8]);
        unpack(pack(b"abcde"), &mut result[8..]);
        assert_eq!(&result, b"12345678abcde");
    }
}
