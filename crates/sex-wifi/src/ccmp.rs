// SPDX-License-Identifier: GPL-3.0-only
//! CCMP-128 (IEEE 802.11-2020 12.5.3): AES-CCM with an 8-byte MIC and a 13-byte nonce.
//! Intel firmware normally does this in hardware once the key is installed; this
//! software path covers frames sent before that and is the reference the tests check.
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::{Aes128, Block};

pub const MIC_LEN: usize = 8;
pub const HEADER_LEN: usize = 8;
const NONCE_LEN: usize = 13;

fn encrypt(cipher: &Aes128, block: [u8; 16]) -> [u8; 16] {
    let mut b = Block::from(block);
    cipher.encrypt_block(&mut b);
    b.into()
}

fn cbc_mac(cipher: &Aes128, nonce: &[u8; NONCE_LEN], aad: &[u8], plain: &[u8]) -> [u8; 16] {
    let mut b0 = [0u8; 16];
    b0[0] = if aad.is_empty() { 0 } else { 0x40 } | (((MIC_LEN as u8 - 2) / 2) << 3) | 1;
    b0[1..14].copy_from_slice(nonce);
    b0[14..].copy_from_slice(&(plain.len() as u16).to_be_bytes());
    let mut x = encrypt(cipher, b0);
    if !aad.is_empty() {
        // The AAD is prefixed with its 2-byte length, then padded to whole blocks.
        let mut first = true;
        let mut rest = aad;
        while first || !rest.is_empty() {
            let mut block = [0u8; 16];
            let start = if first { block[..2].copy_from_slice(&(aad.len() as u16).to_be_bytes()); 2 } else { 0 };
            let take = rest.len().min(16 - start);
            block[start..start + take].copy_from_slice(&rest[..take]);
            rest = &rest[take..];
            first = false;
            for i in 0..16 { x[i] ^= block[i]; }
            x = encrypt(cipher, x);
        }
    }
    for chunk in plain.chunks(16) {
        for (i, b) in chunk.iter().enumerate() { x[i] ^= b; }
        x = encrypt(cipher, x);
    }
    x
}

fn counter_block(nonce: &[u8; NONCE_LEN], counter: u16) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0] = 1; // L - 1
    a[1..14].copy_from_slice(nonce);
    a[14..].copy_from_slice(&counter.to_be_bytes());
    a
}

fn ctr(cipher: &Aes128, nonce: &[u8; NONCE_LEN], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(16).enumerate() {
        let s = encrypt(cipher, counter_block(nonce, i as u16 + 1));
        for (b, k) in chunk.iter_mut().zip(s.iter()) { *b ^= k; }
    }
}

/// AES-CCM seal in place; returns the MIC.
pub fn ccm_seal(key: &[u8; 16], nonce: &[u8; NONCE_LEN], aad: &[u8], data: &mut [u8]) -> [u8; MIC_LEN] {
    let cipher = Aes128::new(key.into());
    let t = cbc_mac(&cipher, nonce, aad, data);
    ctr(&cipher, nonce, data);
    let s0 = encrypt(&cipher, counter_block(nonce, 0));
    let mut mic = [0u8; MIC_LEN];
    for i in 0..MIC_LEN { mic[i] = t[i] ^ s0[i]; }
    mic
}

/// AES-CCM open in place. On a bad MIC the data is zeroed and `Err` returned.
pub fn ccm_open(key: &[u8; 16], nonce: &[u8; NONCE_LEN], aad: &[u8], data: &mut [u8], mic: &[u8]) -> Result<(), ()> {
    let cipher = Aes128::new(key.into());
    ctr(&cipher, nonce, data);
    let t = cbc_mac(&cipher, nonce, aad, data);
    let s0 = encrypt(&cipher, counter_block(nonce, 0));
    let mut expect = [0u8; MIC_LEN];
    for i in 0..MIC_LEN { expect[i] = t[i] ^ s0[i]; }
    if mic.len() != MIC_LEN || !crate::crypto::ct_eq(&expect, mic) {
        for b in data.iter_mut() { *b = 0; }
        return Err(());
    }
    Ok(())
}

/// Builds the CCMP AAD and nonce from an 802.11 MAC header and a 48-bit PN.
/// Returns the AAD length written into `aad` (at most 30 bytes).
pub fn aad_and_nonce(header: &[u8], pn: u64, aad: &mut [u8; 30], nonce: &mut [u8; NONCE_LEN]) -> Result<usize, ()> {
    if header.len() < 24 { return Err(()); }
    let mut fc = u16::from_le_bytes([header[0], header[1]]);
    let kind = (fc >> 2) & 3;
    let subtype = (fc >> 4) & 0xf;
    let four_address = fc & 0x0300 == 0x0300;
    let qos = kind == 2 && subtype & 0x8 != 0;
    let needed = 24 + if four_address { 6 } else { 0 } + if qos { 2 } else { 0 };
    if header.len() < needed { return Err(()); }
    nonce[0] = 0;
    if kind == 2 {
        fc &= !0x0070;
        if qos {
            fc &= !0x8000;
            nonce[0] = header[needed - 2] & 0x0f;
        }
    } else if kind == 0 {
        nonce[0] |= 0x10;
    }
    fc &= !(0x0800 | 0x1000 | 0x2000);
    fc |= 0x4000;
    aad[..2].copy_from_slice(&fc.to_le_bytes());
    aad[2..20].copy_from_slice(&header[4..22]);
    let seq = u16::from_le_bytes([header[22], header[23]]) & 0x000f;
    aad[20..22].copy_from_slice(&seq.to_le_bytes());
    let mut len = 22;
    if four_address { aad[len..len + 6].copy_from_slice(&header[24..30]); len += 6; }
    if qos {
        aad[len] = header[needed - 2] & !0x70 & !0x80;
        aad[len + 1] = 0;
        len += 2;
    }
    nonce[1..7].copy_from_slice(&header[10..16]);
    let pn_bytes = pn.to_be_bytes();
    nonce[7..13].copy_from_slice(&pn_bytes[2..8]);
    Ok(len)
}

/// The 8-byte CCMP header for a PN and key id (ExtIV always set).
pub fn ccmp_header(pn: u64, key_id: u8) -> [u8; HEADER_LEN] {
    let p = pn.to_le_bytes();
    [p[0], p[1], 0, 0x20 | (key_id << 6), p[2], p[3], p[4], p[5]]
}

/// PN and key id from a received CCMP header.
pub fn parse_ccmp_header(h: &[u8]) -> Option<(u64, u8)> {
    if h.len() < HEADER_LEN || h[3] & 0x20 == 0 { return None; }
    let pn = u64::from_le_bytes([h[0], h[1], h[4], h[5], h[6], h[7], 0, 0]);
    Some((pn, h[3] >> 6))
}

/// Encrypts an 802.11 frame: `frame` holds `header_len` header bytes then the
/// plaintext body. `out` receives header || CCMP header || ciphertext || MIC and
/// must be `frame.len() + 16` bytes. The Protected bit is set in the output header.
pub fn encrypt_frame(tk: &[u8; 16], frame: &[u8], header_len: usize, pn: u64, key_id: u8, out: &mut [u8]) -> Result<(), ()> {
    if frame.len() < header_len || out.len() != frame.len() + HEADER_LEN + MIC_LEN { return Err(()); }
    let mut aad = [0u8; 30];
    let mut nonce = [0u8; NONCE_LEN];
    let aad_len = aad_and_nonce(&frame[..header_len], pn, &mut aad, &mut nonce)?;
    out[..header_len].copy_from_slice(&frame[..header_len]);
    out[1] |= 0x40;
    out[header_len..header_len + HEADER_LEN].copy_from_slice(&ccmp_header(pn, key_id));
    let body = header_len + HEADER_LEN;
    let end = body + frame.len() - header_len;
    out[body..end].copy_from_slice(&frame[header_len..]);
    let mic = ccm_seal(tk, &nonce, &aad[..aad_len], &mut out[body..end]);
    out[end..].copy_from_slice(&mic);
    Ok(())
}

/// Decrypts a protected 802.11 frame in place. Returns (PN, plaintext range).
/// The caller enforces replay (PN strictly increasing per key and TID).
pub fn decrypt_frame(tk: &[u8; 16], frame: &mut [u8], header_len: usize) -> Result<(u64, core::ops::Range<usize>), ()> {
    if frame.len() < header_len + HEADER_LEN + MIC_LEN { return Err(()); }
    let (pn, _key) = parse_ccmp_header(&frame[header_len..]).ok_or(())?;
    let mut aad = [0u8; 30];
    let mut nonce = [0u8; NONCE_LEN];
    let aad_len = aad_and_nonce(&frame[..header_len], pn, &mut aad, &mut nonce)?;
    let body = header_len + HEADER_LEN;
    let mic_at = frame.len() - MIC_LEN;
    let mut mic = [0u8; MIC_LEN];
    mic.copy_from_slice(&frame[mic_at..]);
    ccm_open(tk, &nonce, &aad[..aad_len], &mut frame[body..mic_at], &mic)?;
    Ok((pn, body..mic_at))
}
