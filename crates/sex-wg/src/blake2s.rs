// SPDX-License-Identifier: GPL-3.0-only
//! BLAKE2s (RFC 7693), with the HMAC and HKDF WireGuard builds on it.

const IV: [u32; 8] = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4], [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13], [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11], [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5], [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

pub struct State { h: [u32; 8], t: u64, buf: [u8; 64], len: usize, out: usize }

impl State {
    pub fn new(out: usize, key: &[u8]) -> Self {
        assert!((1..=32).contains(&out) && key.len() <= 32);
        let mut h = IV;
        h[0] ^= 0x0101_0000 ^ ((key.len() as u32) << 8) ^ out as u32;
        let mut s = Self { h, t: 0, buf: [0; 64], len: 0, out };
        if !key.is_empty() { s.buf[..key.len()].copy_from_slice(key); s.len = 64; }
        s
    }
    fn compress(&mut self, last: bool) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() { *w = u32::from_le_bytes(self.buf[i * 4..i * 4 + 4].try_into().unwrap()); }
        let mut v = [0u32; 16];
        v[..8].copy_from_slice(&self.h);
        v[8..].copy_from_slice(&IV);
        v[12] ^= self.t as u32;
        v[13] ^= (self.t >> 32) as u32;
        if last { v[14] = !v[14]; }
        let g = |v: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32| {
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(x); v[d] = (v[d] ^ v[a]).rotate_right(16);
            v[c] = v[c].wrapping_add(v[d]); v[b] = (v[b] ^ v[c]).rotate_right(12);
            v[a] = v[a].wrapping_add(v[b]).wrapping_add(y); v[d] = (v[d] ^ v[a]).rotate_right(8);
            v[c] = v[c].wrapping_add(v[d]); v[b] = (v[b] ^ v[c]).rotate_right(7);
        };
        for s in SIGMA.iter() {
            g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]); g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]); g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]); g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]); g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }
        for i in 0..8 { self.h[i] ^= v[i] ^ v[i + 8]; }
    }
    pub fn update(&mut self, mut data: &[u8]) -> &mut Self {
        while !data.is_empty() {
            // A full buffer is compressed only once more input follows (the last block is special).
            if self.len == 64 { self.t += 64; self.compress(false); self.len = 0; }
            let n = (64 - self.len).min(data.len());
            self.buf[self.len..self.len + n].copy_from_slice(&data[..n]);
            self.len += n;
            data = &data[n..];
        }
        self
    }
    pub fn finish(&mut self, out: &mut [u8]) {
        self.t += self.len as u64;
        self.buf[self.len..].fill(0);
        self.compress(true);
        let mut full = [0u8; 32];
        for (i, w) in self.h.iter().enumerate() { full[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes()); }
        out.copy_from_slice(&full[..self.out]);
    }
}

pub fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut s = State::new(32, &[]);
    for p in parts { s.update(p); }
    let mut o = [0u8; 32];
    s.finish(&mut o);
    o
}

/// Keyed BLAKE2s with a 16-byte output: WireGuard's MAC.
pub fn mac(key: &[u8], data: &[u8]) -> [u8; 16] {
    let mut o = [0u8; 16];
    State::new(16, key).update(data).finish(&mut o);
    o
}

pub fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 { k[..32].copy_from_slice(&hash(&[key])); } else { k[..key.len()].copy_from_slice(key); }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 { ipad[i] ^= k[i]; opad[i] ^= k[i]; }
    let mut inner = State::new(32, &[]);
    inner.update(&ipad);
    for p in parts { inner.update(p); }
    let mut ih = [0u8; 32];
    inner.finish(&mut ih);
    hash(&[&opad, &ih])
}

/// HKDF with HMAC-BLAKE2s, giving `N` keys (WireGuard's KDF1/2/3).
pub fn kdf<const N: usize>(key: &[u8; 32], input: &[u8]) -> [[u8; 32]; N] {
    let t0 = hmac(key, &[input]);
    let mut out = [[0u8; 32]; N];
    let mut prev: [u8; 32] = [0; 32];
    for i in 0..N {
        prev = if i == 0 { hmac(&t0, &[&[1]]) } else { hmac(&t0, &[&prev, &[i as u8 + 1]]) };
        out[i] = prev;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> alloc::vec::Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
    #[test]
    fn rfc7693_and_keyed_vectors() {
        assert_eq!(hash(&[b"abc"]).to_vec(), hex("508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982"));
        assert_eq!(hash(&[]).to_vec(), hex("69217a3079908094e11121d042354a7c1f55b6482ca1a51e1b250dfd1ed0eef9"));
        // BLAKE2s reference KAT: key 00..1f, message 00..fe (255 bytes).
        let key: alloc::vec::Vec<u8> = (0..32).collect();
        let msg: alloc::vec::Vec<u8> = (0..255).collect();
        let mut o = [0u8; 32];
        State::new(32, &key).update(&msg).finish(&mut o);
        assert_eq!(o.to_vec(), hex("3fb735061abc519dfe979e54c1ee5bfad0a9d858b3315bad34bde999efd724dd"));
        // Split updates give the same digest.
        let mut s = State::new(32, &key);
        for c in msg.chunks(13) { s.update(c); }
        let mut p = [0u8; 32];
        s.finish(&mut p);
        assert_eq!(o, p);
    }
}
