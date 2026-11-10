// SPDX-License-Identifier: GPL-3.0-only
//! Key derivation and key wrapping used by WPA2 and WPA3 (IEEE 802.11-2020 12.7).
//! Everything is on fixed buffers; no allocation.
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Block};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::Sha256;

pub fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> [u8; 20] {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("hmac takes any key");
    for part in parts { mac.update(part); }
    mac.finalize().into_bytes().into()
}

pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac takes any key");
    for part in parts { mac.update(part); }
    mac.finalize().into_bytes().into()
}

/// The WPA2 passphrase hash: PBKDF2-HMAC-SHA1, 4096 rounds, 256 bits.
pub fn psk_from_passphrase(passphrase: &[u8], ssid: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (block, chunk) in out.chunks_mut(20).enumerate() {
        let index = (block as u32 + 1).to_be_bytes();
        let mut u = hmac_sha1(passphrase, &[ssid, &index]);
        let mut t = u;
        for _ in 1..4096 {
            u = hmac_sha1(passphrase, &[&u]);
            for (a, b) in t.iter_mut().zip(u.iter()) { *a ^= b; }
        }
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
    out
}

/// PRF-n from 802.11 12.7.1.2: HMAC-SHA1(K, A || 0 || B || i), i = 0, 1, ...
pub fn prf_sha1(key: &[u8], label: &[u8], data: &[u8], out: &mut [u8]) {
    for (i, chunk) in out.chunks_mut(20).enumerate() {
        let block = hmac_sha1(key, &[label, &[0], data, &[i as u8]]);
        chunk.copy_from_slice(&block[..chunk.len()]);
    }
}

/// KDF-Hash-Length from 802.11 12.7.1.7.2 with SHA-256. `out.len()` sets Length.
pub fn kdf_sha256(key: &[u8], label: &[u8], context: &[u8], out: &mut [u8]) {
    let bits = (out.len() * 8) as u16;
    for (i, chunk) in out.chunks_mut(32).enumerate() {
        let counter = (i as u16 + 1).to_le_bytes();
        let block = hmac_sha256(key, &[&counter, label, context, &bits.to_le_bytes()]);
        chunk.copy_from_slice(&block[..chunk.len()]);
    }
}

/// Like `kdf_sha256` but with a Length in bits that is not a whole number of
/// bytes' worth of output (SAE hunting and pecking uses the prime's bit length).
pub fn kdf_sha256_bits(key: &[u8], label: &[u8], context: &[u8], bits: u16, out: &mut [u8]) {
    for (i, chunk) in out.chunks_mut(32).enumerate() {
        let counter = (i as u16 + 1).to_le_bytes();
        let block = hmac_sha256(key, &[&counter, label, context, &bits.to_le_bytes()]);
        chunk.copy_from_slice(&block[..chunk.len()]);
    }
}

/// HKDF-Extract and HKDF-Expand with SHA-256 (RFC 5869), used by SAE hash-to-element.
pub fn hkdf_extract(salt: &[u8], ikm: &[&[u8]]) -> [u8; 32] { hmac_sha256(salt, ikm) }
pub fn hkdf_expand(prk: &[u8; 32], info: &[u8], out: &mut [u8]) {
    let mut previous = [0u8; 32];
    let mut have_previous = false;
    for (i, chunk) in out.chunks_mut(32).enumerate() {
        let counter = [i as u8 + 1];
        let t = if have_previous { hmac_sha256(prk, &[&previous, info, &counter]) }
            else { hmac_sha256(prk, &[info, &counter]) };
        chunk.copy_from_slice(&t[..chunk.len()]);
        previous = t;
        have_previous = true;
    }
}

/// AES-128-CMAC (RFC 4493), the EAPOL-Key MIC for SAE and other AKMs with key descriptor 3/0.
pub fn aes_cmac(key: &[u8; 16], parts: &[&[u8]]) -> [u8; 16] {
    let cipher = Aes128::new(key.into());
    let mut l = Block::default();
    cipher.encrypt_block(&mut l);
    let k1 = cmac_double(l.into());
    let k2 = cmac_double(k1);
    let total: usize = parts.iter().map(|p| p.len()).sum();
    let blocks = if total == 0 { 1 } else { total.div_ceil(16) };
    let mut state = [0u8; 16];
    let mut block = [0u8; 16];
    let mut fill;
    let mut done = 0usize;
    let mut seen = 0usize;
    let mut iter = parts.iter().flat_map(|p| p.iter());
    loop {
        // Gather the next block.
        fill = 0;
        while fill < 16 && seen < total { block[fill] = *iter.next().unwrap(); fill += 1; seen += 1; }
        done += 1;
        if done == blocks {
            if fill == 16 { for i in 0..16 { block[i] ^= k1[i]; } }
            else {
                block[fill] = 0x80;
                for b in block.iter_mut().skip(fill + 1) { *b = 0; }
                for i in 0..16 { block[i] ^= k2[i]; }
            }
        }
        for i in 0..16 { state[i] ^= block[i]; }
        let mut b = Block::from(state);
        cipher.encrypt_block(&mut b);
        state = b.into();
        if done == blocks { break; }
    }
    state
}
fn cmac_double(input: [u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut carry = 0u8;
    for i in (0..16).rev() {
        out[i] = (input[i] << 1) | carry;
        carry = input[i] >> 7;
    }
    if input[0] & 0x80 != 0 { out[15] ^= 0x87; }
    out
}

const WRAP_IV: [u8; 8] = [0xa6; 8];

/// AES key wrap (RFC 3394). `out` must be `plain.len() + 8` bytes; `plain` a multiple of 8.
pub fn aes_wrap(kek: &[u8; 16], plain: &[u8], out: &mut [u8]) -> Result<(), ()> {
    if plain.len() % 8 != 0 || plain.len() < 16 || out.len() != plain.len() + 8 { return Err(()); }
    let cipher = Aes128::new(kek.into());
    let n = plain.len() / 8;
    let mut a = WRAP_IV;
    out[8..].copy_from_slice(plain);
    for j in 0..6 {
        for i in 1..=n {
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&a);
            b[8..].copy_from_slice(&out[i * 8..i * 8 + 8]);
            let mut block = Block::from(b);
            cipher.encrypt_block(&mut block);
            let t = (n * j + i) as u64;
            a.copy_from_slice(&block[..8]);
            for (x, y) in a.iter_mut().zip(t.to_be_bytes()) { *x ^= y; }
            out[i * 8..i * 8 + 8].copy_from_slice(&block[8..]);
        }
    }
    out[..8].copy_from_slice(&a);
    Ok(())
}

/// AES key unwrap (RFC 3394). `out` must be `wrapped.len() - 8` bytes. Fails on a bad integrity check.
pub fn aes_unwrap(kek: &[u8; 16], wrapped: &[u8], out: &mut [u8]) -> Result<(), ()> {
    if wrapped.len() % 8 != 0 || wrapped.len() < 24 || out.len() + 8 != wrapped.len() { return Err(()); }
    let cipher = Aes128::new(kek.into());
    let n = out.len() / 8;
    let mut a = [0u8; 8];
    a.copy_from_slice(&wrapped[..8]);
    out.copy_from_slice(&wrapped[8..]);
    for j in (0..6).rev() {
        for i in (1..=n).rev() {
            let t = (n * j + i) as u64;
            let mut b = [0u8; 16];
            for (k, (x, y)) in a.iter().zip(t.to_be_bytes()).enumerate() { b[k] = x ^ y; }
            b[8..].copy_from_slice(&out[(i - 1) * 8..i * 8]);
            let mut block = Block::from(b);
            cipher.decrypt_block(&mut block);
            a.copy_from_slice(&block[..8]);
            out[(i - 1) * 8..i * 8].copy_from_slice(&block[8..]);
        }
    }
    if !ct_eq(&a, &WRAP_IV) { for b in out.iter_mut() { *b = 0; } return Err(()); }
    Ok(())
}

/// Constant-time equality for MICs and tags.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) { diff |= x ^ y; }
    diff == 0
}

/// Lexicographic min/max of two byte strings, as 802.11 key derivations order addresses and nonces.
pub fn min_max<'a>(a: &'a [u8], b: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    if a < b { (a, b) } else { (b, a) }
}
