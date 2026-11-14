// SPDX-License-Identifier: GPL-3.0-only
//! Checksums Kodi repositories publish: `addons.xml.md5` and a `<zip>.md5`, `.sha1`,
//! `.sha256` or `.sha512` beside each zip. MD5 and SHA-1 are only integrity checks
//! here (a download cut short or corrupted); the repository's TLS connection is what
//! says who sent it.

use alloc::string::String;
use alloc::vec::Vec;
use sha2::Digest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash { Md5, Sha1, Sha256, Sha512 }

impl Hash {
    pub fn from_name(name: &str) -> Option<Hash> {
        match name.trim().to_ascii_lowercase().as_str() {
            "md5" => Some(Hash::Md5),
            "sha1" => Some(Hash::Sha1),
            "sha256" => Some(Hash::Sha256),
            "sha512" => Some(Hash::Sha512),
            _ => None,
        }
    }

    /// Also the checksum file's extension.
    pub fn name(self) -> &'static str {
        match self { Hash::Md5 => "md5", Hash::Sha1 => "sha1", Hash::Sha256 => "sha256", Hash::Sha512 => "sha512" }
    }

    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Hash::Md5 => md5(data).to_vec(),
            Hash::Sha1 => sha1(data).to_vec(),
            Hash::Sha256 => sha2::Sha256::digest(data).to_vec(),
            Hash::Sha512 => sha2::Sha512::digest(data).to_vec(),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        for n in [b >> 4, b & 15] { s.push(char::from_digit(n as u32, 16).unwrap_or('0')); }
    }
    s
}

/// The hash in a checksum file: its first word, as `md5sum` writes it (`<hex>  name`).
pub fn parse_checksum(text: &str) -> Option<Vec<u8>> {
    let word = text.split_whitespace().next()?;
    if word.len() % 2 != 0 || word.len() < 32 { return None; }
    (0..word.len()).step_by(2).map(|i| u8::from_str_radix(word.get(i..i + 2)?, 16).ok()).collect()
}

/// Whether `data` matches the checksum file's text.
pub fn verify(data: &[u8], hash: Hash, checksum_file: &str) -> bool {
    parse_checksum(checksum_file).is_some_and(|want| want == hash.digest(data))
}

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// Pads a message the way MD5 and SHA-1 both do, with the bit length in the given order.
fn padded(data: &[u8], big_endian: bool) -> Vec<u8> {
    let mut m = Vec::with_capacity(data.len() + 72);
    m.extend_from_slice(data);
    m.push(0x80);
    while m.len() % 64 != 56 { m.push(0); }
    let bits = (data.len() as u64).wrapping_mul(8);
    m.extend_from_slice(&if big_endian { bits.to_be_bytes() } else { bits.to_le_bytes() });
    m
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    let k = &MD5_K;
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in padded(data, false).chunks_exact(64) {
        let w: [u32; 16] = core::array::from_fn(|i| u32::from_le_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]));
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(w[g]);
            a = d; d = c; c = b;
            b = b.wrapping_add(f.rotate_left(MD5_S[i]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) { *x = x.wrapping_add(y); }
    }
    let mut out = [0u8; 16];
    for (i, v) in h.iter().enumerate() { out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes()); }
    out
}

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    for block in padded(data, true).chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 { w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]); }
        for i in 16..80 { w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1); }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5A827999),
                1 => (b ^ c ^ d, 0x6ED9EBA1),
                2 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d; d = c; c = b.rotate_left(30); b = a; a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) { *x = x.wrapping_add(y); }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() { out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes()); }
    out
}
