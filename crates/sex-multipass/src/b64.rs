// SPDX-License-Identifier: GPL-3.0-only
//! Unpadded base64url, the form OAuth (PKCE, JWT, DPoP) uses.
use alloc::string::String;
use alloc::vec::Vec;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 / 3 + 3);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..chunk.len() + 1 { out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char); }
    }
    out
}

pub fn decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.trim_end_matches('=').bytes() {
        let v = ALPHABET.iter().position(|a| *a == c).or_else(|| match c { b'+' => Some(62), b'/' => Some(63), _ => None })? as u32;
        acc = acc << 6 | v; bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip_and_rfc4648_vectors() {
        for (i, o) in [("", ""), ("f", "Zg"), ("fo", "Zm8"), ("foo", "Zm9v"), ("foob", "Zm9vYg"), ("fooba", "Zm9vYmE"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(super::encode(i.as_bytes()), o);
            assert_eq!(super::decode(o).unwrap(), i.as_bytes());
        }
        assert_eq!(super::encode(&[0xfb, 0xff]), "-_8");
    }
}
