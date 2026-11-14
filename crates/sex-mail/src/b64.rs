// SPDX-License-Identifier: GPL-3.0-only
//! Standard padded base64 (SASL, MIME).
use alloc::string::String;
use alloc::vec::Vec;

const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 { out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' }); }
    }
    out
}

/// Decodes, skipping whitespace (MIME bodies are wrapped at 76 columns).
pub fn decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        if c.is_ascii_whitespace() { continue; }
        if c == b'=' { break; }
        let v = A.iter().position(|a| *a == c)? as u32;
        acc = acc << 6 | v; bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn vectors() {
        assert_eq!(super::encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::encode(b"fo"), "Zm8=");
        assert_eq!(super::decode(b"Zm9v\r\nYmE=").unwrap(), b"fooba");
    }
}
