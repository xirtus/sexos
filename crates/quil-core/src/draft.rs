// SPDX-License-Identifier: GPL-3.0-only
//! Bounded, self-checking recovery snapshots for one named Quil document.

use alloc::vec::Vec;

const MAGIC: &[u8; 4] = b"QDR1";
pub const MAX_TEXT: usize = 4 << 20;
pub const MAX_PATH: usize = 1024;
pub const HEADER: usize = 18;

pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}

pub fn encode(path: &[u8], text: &[u8]) -> Option<Vec<u8>> {
    if path.is_empty() || path.len() > MAX_PATH || text.len() > MAX_TEXT { return None; }
    let mut out = Vec::with_capacity(HEADER + path.len() + text.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(path.len() as u16).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&hash(text).to_le_bytes());
    out.extend_from_slice(path);
    out.extend_from_slice(text);
    Some(out)
}

pub fn decode<'a>(expected_path: &[u8], bytes: &'a [u8]) -> Option<&'a [u8]> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC { return None; }
    let path_len = u16::from_le_bytes(bytes[4..6].try_into().ok()?) as usize;
    let text_len = u32::from_le_bytes(bytes[6..10].try_into().ok()?) as usize;
    let check = u64::from_le_bytes(bytes[10..18].try_into().ok()?);
    if path_len == 0 || path_len > MAX_PATH || text_len > MAX_TEXT ||
        bytes.len() != HEADER.checked_add(path_len)?.checked_add(text_len)? { return None; }
    if &bytes[HEADER..HEADER + path_len] != expected_path { return None; }
    let text = &bytes[HEADER + path_len..];
    if hash(text) != check { return None; }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_rejects_truncation_corruption_and_wrong_file() {
        let packet = encode(b"project/main.rs", b"fn main() {}\n").unwrap();
        assert_eq!(decode(b"project/main.rs", &packet), Some(&b"fn main() {}\n"[..]));
        assert!(decode(b"project/other.rs", &packet).is_none());
        for len in 0..packet.len() { assert!(decode(b"project/main.rs", &packet[..len]).is_none()); }
        let mut corrupt = packet;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode(b"project/main.rs", &corrupt).is_none());
    }
}
