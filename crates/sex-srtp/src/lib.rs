// SPDX-License-Identifier: GPL-3.0-only
//! SRTP and SRTCP for WebRTC media (RFC 3711, RFC 7714). No I/O: a packet goes in, the
//! protected or opened packet comes out.
//!
//! Profiles are the ones DTLS-SRTP negotiates (RFC 5764 use_srtp): AES-128 counter mode with
//! an 80- or 32-bit HMAC-SHA1 tag, and AEAD AES-128-GCM. One `Context` holds one direction's
//! master key and salt; a WebRTC session has two, one to send and one to receive.
//!
//! Each SSRC has its own rollover counter (RFC 3711 3.3.1, the index guess of Appendix A) and
//! a 64-packet replay window. Tags are compared in constant time.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::{Aes128, Block};
use aes_gcm::aead::{AeadInPlace, KeyInit as AeadKeyInit};
use aes_gcm::{Aes128Gcm, Nonce, Tag};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;

/// DTLS-SRTP protection profiles (RFC 5764 4.1.2, RFC 7714 14.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Aes128CmHmacSha1_80,
    Aes128CmHmacSha1_32,
    AeadAes128Gcm,
}

impl Profile {
    /// The use_srtp profile number.
    pub fn id(self) -> u16 {
        match self {
            Profile::Aes128CmHmacSha1_80 => 0x0001,
            Profile::Aes128CmHmacSha1_32 => 0x0002,
            Profile::AeadAes128Gcm => 0x0007,
        }
    }
    pub fn from_id(id: u16) -> Option<Profile> {
        match id {
            0x0001 => Some(Profile::Aes128CmHmacSha1_80),
            0x0002 => Some(Profile::Aes128CmHmacSha1_32),
            0x0007 => Some(Profile::AeadAes128Gcm),
            _ => None,
        }
    }
    pub fn key_len(self) -> usize {
        16
    }
    pub fn salt_len(self) -> usize {
        match self {
            Profile::AeadAes128Gcm => 12,
            _ => 14,
        }
    }
    /// Bytes added to an RTP packet.
    pub fn rtp_overhead(self) -> usize {
        match self {
            Profile::Aes128CmHmacSha1_80 => 10,
            Profile::Aes128CmHmacSha1_32 => 4,
            Profile::AeadAes128Gcm => 16,
        }
    }
    /// Bytes added to an RTCP packet (tag and the E|index word).
    pub fn rtcp_overhead(self) -> usize {
        match self {
            Profile::AeadAes128Gcm => 16 + 4,
            _ => 10 + 4,
        }
    }
    fn gcm(self) -> bool {
        self == Profile::AeadAes128Gcm
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Too short or not version 2.
    Malformed,
    /// Tag did not match.
    Auth,
    /// Seen before or older than the window.
    Replay,
    /// Master key or salt of the wrong length.
    KeyLength,
}

/// A 64-packet sliding window over a stream's packet index.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayWindow {
    top: u64,
    bits: u64,
    any: bool,
}

impl ReplayWindow {
    pub fn check(&self, index: u64) -> bool {
        if !self.any || index > self.top {
            return true;
        }
        let back = self.top - index;
        back < 64 && self.bits & (1u64 << back) == 0
    }
    /// Mark an index seen. Call only after the tag checked out.
    pub fn accept(&mut self, index: u64) {
        if !self.any {
            self.any = true;
            self.top = index;
            self.bits = 1;
        } else if index > self.top {
            let shift = index - self.top;
            self.bits = if shift >= 64 { 0 } else { self.bits << shift };
            self.bits |= 1;
            self.top = index;
        } else {
            let back = self.top - index;
            if back < 64 {
                self.bits |= 1u64 << back;
            }
        }
    }
}

/// Session keys for one of SRTP or SRTCP.
struct Keys {
    aes: Aes128,
    gcm: Option<Aes128Gcm>,
    salt: [u8; 14],
    auth: Option<Hmac<Sha1>>,
}

/// AES-CM keystream XOR (RFC 3711 4.1.1): counter block is IV, incremented as one 128-bit
/// big-endian number.
fn aes_cm(aes: &Aes128, iv: [u8; 16], data: &mut [u8]) {
    let mut ctr = u128::from_be_bytes(iv);
    for chunk in data.chunks_mut(16) {
        let mut b = Block::from(ctr.to_be_bytes());
        aes.encrypt_block(&mut b);
        for (d, k) in chunk.iter_mut().zip(b.iter()) {
            *d ^= k;
        }
        ctr = ctr.wrapping_add(1);
    }
}

/// The RFC 3711 4.3 key derivation with the AES-CM PRF and key_derivation_rate 0. A 12-byte
/// (GCM) master salt is padded with zeros at the end, as RFC 7714 and libsrtp do.
fn derive(master_key: &[u8; 16], salt: &[u8; 14], label: u8, out: &mut [u8]) {
    let aes = Aes128::new(master_key.into());
    let mut iv = [0u8; 16];
    iv[..14].copy_from_slice(salt);
    iv[7] ^= label;
    out.fill(0);
    aes_cm(&aes, iv, out);
}

impl Keys {
    fn new(profile: Profile, mk: &[u8; 16], ms: &[u8; 14], base: u8) -> Keys {
        let mut k = [0u8; 16];
        derive(mk, ms, base, &mut k);
        let mut salt = [0u8; 14];
        derive(mk, ms, base + 2, &mut salt[..profile.salt_len()]);
        let auth = if profile.gcm() {
            None
        } else {
            let mut a = [0u8; 20];
            derive(mk, ms, base + 1, &mut a);
            Some(<Hmac<Sha1> as Mac>::new_from_slice(&a).unwrap())
        };
        Keys {
            aes: Aes128::new((&k).into()),
            gcm: profile.gcm().then(|| <Aes128Gcm as AeadKeyInit>::new((&k).into())),
            salt,
            auth,
        }
    }
    fn mac(&self, parts: &[&[u8]]) -> [u8; 20] {
        let mut m = self.auth.clone().unwrap();
        for p in parts {
            m.update(p);
        }
        m.finalize().into_bytes().into()
    }
}

#[derive(Clone, Copy, Default)]
struct Stream {
    roc: u32,
    s_l: u16,
    started: bool,
    replay: ReplayWindow,
    /// SRTCP: next index to send.
    rtcp_next: u32,
    rtcp_replay: ReplayWindow,
}

impl Stream {
    /// RFC 3711 Appendix A: the ROC this sequence number most likely belongs to.
    fn guess(&self, seq: u16) -> u32 {
        if !self.started {
            return self.roc;
        }
        let (s_l, seq) = (self.s_l as i32, seq as i32);
        if s_l < 32768 {
            if seq - s_l > 32768 {
                self.roc.wrapping_sub(1)
            } else {
                self.roc
            }
        } else if s_l - 32768 > seq {
            self.roc.wrapping_add(1)
        } else {
            self.roc
        }
    }
    fn update(&mut self, roc: u32, seq: u16) {
        let idx = ((roc as u64) << 16) | seq as u64;
        let top = ((self.roc as u64) << 16) | self.s_l as u64;
        if !self.started || idx > top {
            self.roc = roc;
            self.s_l = seq;
            self.started = true;
        }
    }
}

/// One direction of SRTP and SRTCP under one master key.
pub struct Context {
    profile: Profile,
    rtp: Keys,
    rtcp: Keys,
    streams: BTreeMap<u32, Stream>,
    /// Index the first SRTCP packet of a new stream gets.
    pub first_srtcp_index: u32,
}

/// Length of the RTP header with CSRCs and the extension.
pub fn rtp_header_len(p: &[u8]) -> Result<usize, Error> {
    if p.len() < 12 || p[0] >> 6 != 2 {
        return Err(Error::Malformed);
    }
    let mut n = 12 + 4 * (p[0] & 0x0f) as usize;
    if p[0] & 0x10 != 0 {
        if p.len() < n + 4 {
            return Err(Error::Malformed);
        }
        n += 4 + 4 * u16::from_be_bytes([p[n + 2], p[n + 3]]) as usize;
    }
    if p.len() < n {
        return Err(Error::Malformed);
    }
    Ok(n)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl Context {
    pub fn new(profile: Profile, master_key: &[u8], master_salt: &[u8]) -> Result<Context, Error> {
        if master_key.len() != profile.key_len() || master_salt.len() != profile.salt_len() {
            return Err(Error::KeyLength);
        }
        let mut mk = [0u8; 16];
        mk.copy_from_slice(master_key);
        let mut ms = [0u8; 14];
        ms[..master_salt.len()].copy_from_slice(master_salt);
        Ok(Context {
            profile,
            rtp: Keys::new(profile, &mk, &ms, 0),
            rtcp: Keys::new(profile, &mk, &ms, 3),
            streams: BTreeMap::new(),
            first_srtcp_index: 0,
        })
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// The rollover counter this context holds for an SSRC.
    pub fn roc(&self, ssrc: u32) -> Option<u32> {
        self.streams.get(&ssrc).map(|s| s.roc)
    }

    /// Start a stream at a known ROC (from signalling, or to resume).
    pub fn set_roc(&mut self, ssrc: u32, roc: u32) {
        let s = self.streams.entry(ssrc).or_default();
        s.roc = roc;
    }

    fn stream(&mut self, ssrc: u32) -> &mut Stream {
        let first = self.first_srtcp_index;
        self.streams.entry(ssrc).or_insert_with(|| Stream { rtcp_next: first, ..Stream::default() })
    }

    fn gcm_rtp_iv(k: &Keys, ssrc: u32, roc: u32, seq: u16) -> [u8; 12] {
        let mut iv = [0u8; 12];
        iv[2..6].copy_from_slice(&ssrc.to_be_bytes());
        iv[6..10].copy_from_slice(&roc.to_be_bytes());
        iv[10..12].copy_from_slice(&seq.to_be_bytes());
        for i in 0..12 {
            iv[i] ^= k.salt[i];
        }
        iv
    }

    fn cm_iv(k: &Keys, ssrc: u32, index: u64) -> [u8; 16] {
        let mut iv = [0u8; 16];
        iv[..14].copy_from_slice(&k.salt);
        for (i, b) in ssrc.to_be_bytes().iter().enumerate() {
            iv[4 + i] ^= b;
        }
        for (i, b) in index.to_be_bytes()[2..].iter().enumerate() {
            iv[8 + i] ^= b;
        }
        iv
    }

    /// Protect one RTP packet.
    pub fn protect_rtp(&mut self, pkt: &[u8]) -> Result<Vec<u8>, Error> {
        let hl = rtp_header_len(pkt)?;
        let seq = u16::from_be_bytes([pkt[2], pkt[3]]);
        let ssrc = be32(&pkt[8..12]);
        let s = self.stream(ssrc);
        let roc = s.guess(seq);
        s.update(roc, seq);
        let mut out = Vec::with_capacity(pkt.len() + self.profile.rtp_overhead());
        out.extend_from_slice(pkt);
        if self.profile.gcm() {
            let iv = Self::gcm_rtp_iv(&self.rtp, ssrc, roc, seq);
            let (h, body) = out.split_at_mut(hl);
            let tag = self.rtp.gcm.as_ref().unwrap()
                .encrypt_in_place_detached(Nonce::from_slice(&iv), h, body)
                .map_err(|_| Error::Malformed)?;
            out.extend_from_slice(&tag);
        } else {
            let index = ((roc as u64) << 16) | seq as u64;
            aes_cm(&self.rtp.aes, Self::cm_iv(&self.rtp, ssrc, index), &mut out[hl..]);
            let tag = self.rtp.mac(&[&out, &roc.to_be_bytes()]);
            out.extend_from_slice(&tag[..self.profile.rtp_overhead()]);
        }
        Ok(out)
    }

    /// Check and open one SRTP packet.
    pub fn unprotect_rtp(&mut self, pkt: &[u8]) -> Result<Vec<u8>, Error> {
        let tl = self.profile.rtp_overhead();
        if pkt.len() < 12 + tl {
            return Err(Error::Malformed);
        }
        let body_end = pkt.len() - tl;
        let hl = rtp_header_len(&pkt[..body_end])?;
        let seq = u16::from_be_bytes([pkt[2], pkt[3]]);
        let ssrc = be32(&pkt[8..12]);
        let profile = self.profile;
        let s = *self.stream(ssrc);
        let roc = s.guess(seq);
        let index = ((roc as u64) << 16) | seq as u64;
        if !s.replay.check(index) {
            return Err(Error::Replay);
        }
        let mut out = Vec::from(&pkt[..body_end]);
        if profile.gcm() {
            let iv = Self::gcm_rtp_iv(&self.rtp, ssrc, roc, seq);
            let (h, body) = out.split_at_mut(hl);
            self.rtp.gcm.as_ref().unwrap()
                .decrypt_in_place_detached(Nonce::from_slice(&iv), h, body, Tag::from_slice(&pkt[body_end..]))
                .map_err(|_| Error::Auth)?;
        } else {
            let tag = self.rtp.mac(&[&out, &roc.to_be_bytes()]);
            if !bool::from(tag[..tl].ct_eq(&pkt[body_end..])) {
                return Err(Error::Auth);
            }
            aes_cm(&self.rtp.aes, Self::cm_iv(&self.rtp, ssrc, index), &mut out[hl..]);
        }
        let s = self.stream(ssrc);
        s.replay.accept(index);
        s.update(roc, seq);
        Ok(out)
    }

    /// Protect one compound RTCP packet (always encrypted, E set).
    pub fn protect_rtcp(&mut self, pkt: &[u8]) -> Result<Vec<u8>, Error> {
        if pkt.len() < 8 || pkt[0] >> 6 != 2 {
            return Err(Error::Malformed);
        }
        let ssrc = be32(&pkt[4..8]);
        let s = self.stream(ssrc);
        let index = s.rtcp_next & 0x7fff_ffff;
        s.rtcp_next = (index + 1) & 0x7fff_ffff;
        let word = (0x8000_0000 | index).to_be_bytes();
        let mut out = Vec::with_capacity(pkt.len() + self.profile.rtcp_overhead());
        out.extend_from_slice(pkt);
        if self.profile.gcm() {
            let iv = Self::gcm_rtcp_iv(&self.rtcp, ssrc, index);
            let mut aad = [0u8; 12];
            aad[..8].copy_from_slice(&pkt[..8]);
            aad[8..].copy_from_slice(&word);
            let tag = self.rtcp.gcm.as_ref().unwrap()
                .encrypt_in_place_detached(Nonce::from_slice(&iv), &aad, &mut out[8..])
                .map_err(|_| Error::Malformed)?;
            out.extend_from_slice(&tag);
            out.extend_from_slice(&word);
        } else {
            aes_cm(&self.rtcp.aes, Self::cm_iv(&self.rtcp, ssrc, index as u64), &mut out[8..]);
            out.extend_from_slice(&word);
            let tag = self.rtcp.mac(&[&out]);
            out.extend_from_slice(&tag[..10]);
        }
        Ok(out)
    }

    fn gcm_rtcp_iv(k: &Keys, ssrc: u32, index: u32) -> [u8; 12] {
        let mut iv = [0u8; 12];
        iv[2..6].copy_from_slice(&ssrc.to_be_bytes());
        iv[8..12].copy_from_slice(&index.to_be_bytes());
        for i in 0..12 {
            iv[i] ^= k.salt[i];
        }
        iv
    }

    /// Check and open one SRTCP packet.
    pub fn unprotect_rtcp(&mut self, pkt: &[u8]) -> Result<Vec<u8>, Error> {
        let ov = self.profile.rtcp_overhead();
        if pkt.len() < 8 + ov || pkt[0] >> 6 != 2 {
            return Err(Error::Malformed);
        }
        let ssrc = be32(&pkt[4..8]);
        let gcm = self.profile.gcm();
        let word_at = if gcm { pkt.len() - 4 } else { pkt.len() - 14 };
        let word = be32(&pkt[word_at..word_at + 4]);
        let (encrypted, index) = (word & 0x8000_0000 != 0, word & 0x7fff_ffff);
        if !self.stream(ssrc).rtcp_replay.check(index as u64) {
            return Err(Error::Replay);
        }
        let mut out;
        if gcm {
            let body_end = pkt.len() - 20;
            let iv = Self::gcm_rtcp_iv(&self.rtcp, ssrc, index);
            let g = self.rtcp.gcm.as_ref().unwrap();
            let tag = Tag::from_slice(&pkt[body_end..body_end + 16]);
            if encrypted {
                let mut aad = [0u8; 12];
                aad[..8].copy_from_slice(&pkt[..8]);
                aad[8..].copy_from_slice(&word.to_be_bytes());
                out = Vec::from(&pkt[..body_end]);
                g.decrypt_in_place_detached(Nonce::from_slice(&iv), &aad, &mut out[8..], tag)
                    .map_err(|_| Error::Auth)?;
            } else {
                // Not encrypted: the whole packet and the word are the AAD.
                let mut aad = Vec::from(&pkt[..body_end]);
                aad.extend_from_slice(&word.to_be_bytes());
                g.decrypt_in_place_detached(Nonce::from_slice(&iv), &aad, &mut [], tag)
                    .map_err(|_| Error::Auth)?;
                out = Vec::from(&pkt[..body_end]);
            }
        } else {
            let tag = self.rtcp.mac(&[&pkt[..word_at + 4]]);
            if !bool::from(tag[..10].ct_eq(&pkt[word_at + 4..])) {
                return Err(Error::Auth);
            }
            out = Vec::from(&pkt[..word_at]);
            if encrypted {
                aes_cm(&self.rtcp.aes, Self::cm_iv(&self.rtcp, ssrc, index as u64), &mut out[8..]);
            }
        }
        self.stream(ssrc).rtcp_replay.accept(index as u64);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3711_b2_keystream() {
        // RFC 3711 B.2: AES-CM keystream, session key and salt given.
        let key = hex("2B7E151628AED2A6ABF7158809CF4F3C");
        let aes = Aes128::new(key.as_slice().into());
        let mut iv = [0u8; 16];
        iv[..14].copy_from_slice(&hex("F0F1F2F3F4F5F6F7F8F9FAFBFCFD"));
        let mut ks = [0u8; 48];
        aes_cm(&aes, iv, &mut ks);
        assert_eq!(ks.to_vec(), hex("E03EAD0935C95E80E166B16DD92B4EB4D23513162B02D0F72A43A2FE4A5F97AB41E95B3BB0A2E8DD477901E4FCA894C0"));
    }

    #[test]
    fn rfc3711_b3_key_derivation() {
        let mk: [u8; 16] = hex("E1F97A0D3E018BE0D64FA32C06DE4139").try_into().unwrap();
        let mut ms = [0u8; 14];
        ms.copy_from_slice(&hex("0EC675AD498AFEEBB6960B3AABE6"));
        let mut k = [0u8; 16];
        derive(&mk, &ms, 0, &mut k);
        assert_eq!(k.to_vec(), hex("C61E7A93744F39EE10734AFE3FF7A087"));
        let mut s = [0u8; 14];
        derive(&mk, &ms, 2, &mut s);
        assert_eq!(s.to_vec(), hex("30CBBC08863D8C85D49DB34A9AE1"));
        let mut a = [0u8; 20];
        derive(&mk, &ms, 1, &mut a);
        assert_eq!(a.to_vec(), hex("CEBE321F6FF7716B6FD4AB49AF256A156D38BAA4"));
    }

    #[test]
    fn replay_window() {
        let mut w = ReplayWindow::default();
        assert!(w.check(100));
        w.accept(100);
        assert!(!w.check(100));
        assert!(w.check(99));
        w.accept(99);
        assert!(!w.check(99));
        w.accept(200);
        assert!(!w.check(100));
        assert!(w.check(150));
        assert!(!w.check(136));
    }

    #[test]
    fn roc_guess() {
        let mut s = Stream::default();
        s.update(0, 65530);
        assert_eq!(s.guess(3), 1);
        s.update(1, 3);
        assert_eq!(s.guess(65534), 0);
        assert_eq!(s.guess(10), 1);
    }

    pub fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
}
