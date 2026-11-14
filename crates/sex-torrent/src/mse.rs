// SPDX-License-Identifier: GPL-3.0-only
//! Message Stream Encryption (MSE, also called Protocol Encryption or PE), the Vuze and
//! libtorrent scheme that hides BitTorrent traffic from simple traffic shapers.
//!
//! Five messages, A the side that connects, B the side that accepts:
//! 1. A->B: Ya (A's 768-bit Diffie-Hellman public key), PadA (0..512 random bytes)
//! 2. B->A: Yb, PadB
//! 3. A->B: HASH('req1', S), HASH('req2', SKEY) xor HASH('req3', S),
//!    ENCRYPT(VC, crypto_provide, len(PadC), PadC, len(IA)), ENCRYPT(IA)
//! 4. B->A: ENCRYPT(VC, crypto_select, len(PadD), PadD), ENCRYPT2(payload...)
//! 5. A->B: ENCRYPT2(payload...)
//!
//! S is the shared DH secret, SKEY the torrent's info-hash (which B looks up among all
//! its torrents with a caller callback), HASH is SHA-1, VC eight zero bytes, ENCRYPT is
//! RC4 with the first 1024 bytes of keystream dropped, keyed by HASH('keyA', S, SKEY)
//! for A->B and HASH('keyB', S, SKEY) for B->A. ENCRYPT2 is the method B selected: RC4
//! (continuing the same keystream) or plaintext. IA, A's initial payload (normally the
//! BitTorrent handshake), is always RC4-encrypted.
//!
//! Because the pads have random lengths, each side has to search for the start of the
//! next part: B searches for HASH('req1', S) within the first 628 bytes (96 + 512 + 20),
//! A for the encrypted VC within 616 bytes after Yb.
//!
//! [`Handshake`] does either side, sans-IO: feed it what arrives, send what
//! [`Handshake::take_output`] gives, until it returns [`Established`], whose [`Cipher`]
//! then encrypts and decrypts the rest of the connection. The responder also recognises
//! a plain (unencrypted) BitTorrent handshake and passes it through when policy allows.

use alloc::vec::Vec;
use crypto_bigint::modular::runtime_mod::{DynResidue, DynResidueParams};
use crypto_bigint::{Encoding, U192, U768};
use sha1::{Digest, Sha1};

/// The 768-bit MSE prime (the Oakley group 1 prime).
pub const PRIME_HEX: &str = "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A3620FFFFFFFFFFFFFFFF";
/// crypto_provide / crypto_select bits.
pub const CRYPTO_PLAINTEXT: u32 = 0x01;
pub const CRYPTO_RC4: u32 = 0x02;
/// Longest pad (PadA..PadD).
pub const MAX_PAD: usize = 512;
/// How far B searches for HASH('req1', S): Ya + PadA + the hash.
pub const REQ1_SYNC_BOUND: usize = 96 + MAX_PAD + 20;
/// How far A searches for the encrypted VC after Yb: PadB + VC.
pub const VC_SYNC_BOUND: usize = MAX_PAD + 8;
/// Size of a DH public key on the wire.
pub const KEY_BYTES: usize = 96;
/// The private exponent size we use (the spec asks for at least 128 bits; 160 is what
/// the reference clients use).
pub const PRIVATE_BYTES: usize = 20;
const BT_PROTOCOL: &[u8] = b"\x13BitTorrent protocol";
const MAX_BUFFER: usize = 96 + MAX_PAD + 20 + 20 + 14 + MAX_PAD + 2 + 65_535 + 4096;

/// RC4, the stream cipher MSE uses (with the first 1024 bytes of keystream discarded).
#[derive(Clone)]
pub struct Rc4 { s: [u8; 256], i: u8, j: u8 }

impl Rc4 {
    /// Plain RC4 keyed with `key` (1..=256 bytes; an empty key acts as one zero byte).
    pub fn new(key: &[u8]) -> Rc4 {
        let mut s = [0u8; 256];
        for (i, b) in s.iter_mut().enumerate() { *b = i as u8; }
        let mut j: u8 = 0;
        for i in 0..256 {
            let k = if key.is_empty() { 0 } else { key[i % key.len()] };
            j = j.wrapping_add(s[i]).wrapping_add(k);
            s.swap(i, j as usize);
        }
        Rc4 { s, i: 0, j: 0 }
    }
    /// RC4 keyed with `key` after dropping 1024 keystream bytes, as MSE uses it.
    pub fn drop1024(key: &[u8]) -> Rc4 {
        let mut r = Rc4::new(key);
        r.apply(&mut [0u8; 1024]);
        r
    }
    /// XORs the keystream into `data` (encrypts or decrypts).
    pub fn apply(&mut self, data: &mut [u8]) {
        for b in data {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.s[self.i as usize]);
            self.s.swap(self.i as usize, self.j as usize);
            let k = self.s[(self.s[self.i as usize].wrapping_add(self.s[self.j as usize])) as usize];
            *b ^= k;
        }
    }
}

impl core::fmt::Debug for Rc4 { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("Rc4(..)") } }

/// Which connections to allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Only RC4-encrypted streams; refuse plaintext and legacy handshakes.
    RequireEncryption,
    /// Offer and pick RC4, accept plaintext when the peer only does that.
    PreferEncryption,
    /// Obfuscated handshake but plaintext payload when the peer offers it (saves CPU);
    /// RC4 otherwise.
    PreferPlaintext,
}

impl Policy {
    /// The crypto_provide bits this policy offers.
    pub fn provide(self) -> u32 {
        match self { Policy::RequireEncryption => CRYPTO_RC4, _ => CRYPTO_RC4 | CRYPTO_PLAINTEXT }
    }
    /// What this policy selects from a peer's crypto_provide.
    pub fn select(self, provided: u32) -> Option<u32> {
        let rc4 = provided & CRYPTO_RC4 != 0;
        let plain = provided & CRYPTO_PLAINTEXT != 0;
        match self {
            Policy::RequireEncryption => rc4.then_some(CRYPTO_RC4),
            Policy::PreferEncryption => if rc4 { Some(CRYPTO_RC4) } else if plain { Some(CRYPTO_PLAINTEXT) } else { None },
            Policy::PreferPlaintext => if plain { Some(CRYPTO_PLAINTEXT) } else if rc4 { Some(CRYPTO_RC4) } else { None },
        }
    }
    /// Whether an unencrypted BitTorrent handshake (no MSE at all) is accepted.
    pub fn allows_legacy(self) -> bool { self != Policy::RequireEncryption }
}

/// How the established stream is carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// MSE handshake done, payload in the clear.
    Plaintext,
    /// MSE handshake done, payload RC4-encrypted.
    Rc4,
    /// The peer sent a plain BitTorrent handshake: no MSE at all.
    Legacy,
}

/// Why a handshake failed. The connection should be dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A length field out of range, a select bit that was not offered, or input after
    /// the handshake finished.
    Malformed,
    /// The DH public key was 0, 1, P-1 or not below P.
    BadKey,
    /// HASH('req1', S) or the encrypted VC did not show up within the bound.
    NoSync,
    /// The decrypted VC was not eight zeros.
    BadVc,
    /// The responder has no torrent with that SKEY.
    UnknownTorrent,
    /// No crypto method both sides accept.
    NoCommonMethod,
    /// A plaintext BitTorrent handshake while encryption is required.
    PlaintextRefused,
    /// More buffered than any valid handshake can need.
    TooLong,
}

/// The ciphers for the rest of the connection (both `None` for plaintext).
#[derive(Clone, Debug)]
pub struct Cipher { enc: Option<Rc4>, dec: Option<Rc4> }

impl Cipher {
    /// A cipher that does nothing.
    pub fn plaintext() -> Cipher { Cipher { enc: None, dec: None } }
    /// Encrypts outgoing bytes in place.
    pub fn encrypt(&mut self, data: &mut [u8]) { if let Some(r) = &mut self.enc { r.apply(data); } }
    /// Decrypts incoming bytes in place.
    pub fn decrypt(&mut self, data: &mut [u8]) { if let Some(r) = &mut self.dec { r.apply(data); } }
    pub fn is_encrypted(&self) -> bool { self.enc.is_some() }
}

/// A finished handshake.
#[derive(Clone, Debug)]
pub struct Established {
    pub method: Method,
    pub cipher: Cipher,
    /// SKEY: the info-hash the stream is for (`None` for a legacy plaintext handshake,
    /// where the BitTorrent handshake in `payload` names it).
    pub info_hash: Option<[u8; 20]>,
    /// Received bytes past the handshake, already decrypted: for the responder this starts
    /// with A's initial payload (IA); for a legacy handshake it is everything received.
    pub payload: Vec<u8>,
}

/// Result of feeding bytes.
#[derive(Debug)]
pub enum Progress {
    /// Keep feeding (and send any output).
    NeedMore,
    /// Done. Send any remaining output first (it holds message 4 for the responder).
    Done(Established),
}

/// HASH(parts...) with SHA-1.
fn hash(parts: &[&[u8]]) -> [u8; 20] {
    let mut h = Sha1::new();
    for p in parts { h.update(p); }
    h.finalize().into()
}

/// HASH('req2', SKEY): what a responder indexes its torrents by, so that the lookup
/// callback is a map lookup.
pub fn req2_hash(skey: &[u8; 20]) -> [u8; 20] { hash(&[b"req2", skey]) }

fn prime() -> U768 { U768::from_be_hex(PRIME_HEX) }

/// G^x mod P for a private exponent (big-endian bytes, at most 24).
fn dh_public(x: &[u8; PRIVATE_BYTES]) -> [u8; KEY_BYTES] {
    dh_pow(&U768::from_u8(2), x)
}

fn dh_pow(base: &U768, x: &[u8; PRIVATE_BYTES]) -> [u8; KEY_BYTES] {
    let mut e = [0u8; 24];
    e[24 - PRIVATE_BYTES..].copy_from_slice(x);
    let params = DynResidueParams::new(&prime());
    DynResidue::new(base, params).pow(&U192::from_be_slice(&e)).retrieve().to_be_bytes()
}

/// 1 < y < P-1, compared as big-endian byte strings.
fn valid_public(y: &[u8; KEY_BYTES]) -> bool {
    let p = prime().to_be_bytes();
    let pm1 = prime().wrapping_sub(&U768::ONE).to_be_bytes();
    let mut two = [0u8; KEY_BYTES];
    two[KEY_BYTES - 1] = 2;
    y[..] >= two[..] && y[..] < pm1[..] && y[..] < p[..]
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w == needle) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    // Initiator
    AwaitYb,
    AwaitVc,
    AwaitPadD(u16),
    // Responder
    AwaitYa,
    AwaitReq1,
    AwaitReq23,
    AwaitVcProvide,
    AwaitPadC(u16),
    AwaitIa(u16),
    Done,
}

/// One side of an MSE handshake.
pub struct Handshake {
    initiator: bool,
    policy: Policy,
    x: [u8; PRIVATE_BYTES],
    s: [u8; KEY_BYTES],
    skey: [u8; 20],
    state: State,
    buf: Vec<u8>,
    out: Vec<u8>,
    enc: Option<Rc4>,
    dec: Option<Rc4>,
    ia: Vec<u8>,
    provided: u32,
    pad_seed: [u8; 2],
    pad: Vec<u8>,
}

impl core::fmt::Debug for Handshake {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handshake").field("initiator", &self.initiator).field("state", &self.state).finish()
    }
}

fn pad_len(random: &mut dyn FnMut(&mut [u8])) -> usize {
    let mut b = [0u8; 2];
    random(&mut b);
    u16::from_be_bytes(b) as usize % (MAX_PAD + 1)
}

impl Handshake {
    /// The connecting side (A), for the torrent `skey` (its info-hash as used on the wire).
    /// `initial_payload` (IA, at most 65535 bytes; longer is cut) goes out encrypted in
    /// message 3, normally the BitTorrent handshake. Output (Ya, PadA) is ready at once.
    pub fn initiator(skey: [u8; 20], policy: Policy, initial_payload: &[u8], random: &mut dyn FnMut(&mut [u8])) -> Handshake {
        let mut h = Handshake::blank(true, policy, random);
        h.skey = skey;
        h.ia = initial_payload[..initial_payload.len().min(65_535)].to_vec();
        let y = dh_public(&h.x);
        h.out.extend_from_slice(&y);
        let n = pad_len(random);
        let mut pad = alloc::vec![0u8; n];
        random(&mut pad);
        h.out.extend_from_slice(&pad);
        h.state = State::AwaitYb;
        h
    }

    /// The accepting side (B). It sends Yb once Ya has arrived.
    pub fn responder(policy: Policy, random: &mut dyn FnMut(&mut [u8])) -> Handshake {
        let mut h = Handshake::blank(false, policy, random);
        h.state = State::AwaitYa;
        h
    }

    fn blank(initiator: bool, policy: Policy, random: &mut dyn FnMut(&mut [u8])) -> Handshake {
        let mut x = [0u8; PRIVATE_BYTES];
        random(&mut x);
        if x.iter().all(|&b| b == 0) { x[PRIVATE_BYTES - 1] = 1; }
        let mut pad_seed = [0u8; 2];
        random(&mut pad_seed);
        // PadB / PadC / PadD: decided now so the handshake needs no randomness later.
        let n = pad_len(random);
        let mut pad = alloc::vec![0u8; n];
        if !initiator { random(&mut pad); }
        Handshake {
            initiator, policy, x, s: [0; KEY_BYTES], skey: [0; 20], state: State::Done, buf: Vec::new(),
            out: Vec::new(), enc: None, dec: None, ia: Vec::new(), provided: 0, pad_seed, pad,
        }
    }

    /// Bytes to send to the peer now.
    pub fn take_output(&mut self) -> Vec<u8> { core::mem::take(&mut self.out) }

    /// Feeds received bytes. `lookup` (responder only) maps HASH('req2', SKEY) to the
    /// torrent's SKEY, or `None` when we do not have that torrent; see [`req2_hash`].
    pub fn feed(&mut self, data: &[u8], lookup: &mut dyn FnMut(&[u8; 20]) -> Option<[u8; 20]>) -> Result<Progress, Error> {
        if self.state == State::Done { return Err(Error::Malformed); }
        if self.buf.len() + data.len() > MAX_BUFFER { return Err(Error::TooLong); }
        self.buf.extend_from_slice(data);
        loop {
            match self.step(lookup) {
                Ok(Some(true)) => continue,
                Ok(Some(false)) | Ok(None) => {}
                Err(e) => { self.state = State::Done; return Err(e); }
            }
            if self.state == State::Done { return Ok(Progress::Done(self.finish())); }
            return Ok(Progress::NeedMore);
        }
    }

    /// Takes `n` buffered bytes, decrypted with the handshake cipher.
    fn take_dec(&mut self, n: usize) -> Vec<u8> {
        let mut v: Vec<u8> = self.buf.drain(..n).collect();
        if let Some(d) = &mut self.dec { d.apply(&mut v); }
        v
    }

    fn compute_s(&mut self, y: &[u8; KEY_BYTES]) -> Result<(), Error> {
        if !valid_public(y) { return Err(Error::BadKey); }
        self.s = dh_pow(&U768::from_be_slice(y), &self.x);
        Ok(())
    }

    fn keys(&mut self) {
        let a = Rc4::drop1024(&hash(&[b"keyA", &self.s, &self.skey]));
        let b = Rc4::drop1024(&hash(&[b"keyB", &self.s, &self.skey]));
        if self.initiator { self.enc = Some(a); self.dec = Some(b); } else { self.enc = Some(b); self.dec = Some(a); }
    }

    /// Advances one state. `Some(true)`: progress made, try again; `None`: need bytes.
    fn step(&mut self, lookup: &mut dyn FnMut(&[u8; 20]) -> Option<[u8; 20]>) -> Result<Option<bool>, Error> {
        match self.state {
            State::AwaitYb => {
                if self.buf.len() < KEY_BYTES { return Ok(None); }
                let mut y = [0u8; KEY_BYTES];
                y.copy_from_slice(&self.buf[..KEY_BYTES]);
                self.buf.drain(..KEY_BYTES);
                self.compute_s(&y)?;
                self.keys();
                // Message 3.
                let req1 = hash(&[b"req1", &self.s]);
                let req2 = req2_hash(&self.skey);
                let req3 = hash(&[b"req3", &self.s]);
                self.out.extend_from_slice(&req1);
                for i in 0..20 { self.out.push(req2[i] ^ req3[i]); }
                let pad_c = self.pad.len();
                let mut m = Vec::with_capacity(16 + pad_c + self.ia.len());
                m.extend_from_slice(&[0u8; 8]);
                m.extend_from_slice(&self.policy.provide().to_be_bytes());
                m.extend_from_slice(&(pad_c as u16).to_be_bytes());
                m.extend_from_slice(&self.pad);
                m.extend_from_slice(&(self.ia.len() as u16).to_be_bytes());
                m.extend_from_slice(&self.ia);
                if let Some(e) = &mut self.enc { e.apply(&mut m); }
                self.out.extend_from_slice(&m);
                self.state = State::AwaitVc;
                Ok(Some(true))
            }
            State::AwaitVc => {
                let mut vc = [0u8; 8];
                if let Some(d) = &self.dec { d.clone().apply(&mut vc); }
                match find(&self.buf, &vc) {
                    Some(k) => {
                        if k > MAX_PAD { return Err(Error::NoSync); }
                        self.buf.drain(..k);
                        if self.buf.len() < 14 { return Ok(None); }
                        let h = self.take_dec(14);
                        let select = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
                        let pad_d = u16::from_be_bytes([h[12], h[13]]);
                        let provided = self.policy.provide();
                        if (select != CRYPTO_RC4 && select != CRYPTO_PLAINTEXT) || select & provided == 0 { return Err(Error::Malformed); }
                        if pad_d as usize > MAX_PAD { return Err(Error::Malformed); }
                        self.provided = select;
                        self.state = State::AwaitPadD(pad_d);
                        Ok(Some(true))
                    }
                    None if self.buf.len() >= VC_SYNC_BOUND => Err(Error::NoSync),
                    None => Ok(None),
                }
            }
            State::AwaitPadD(n) => {
                if self.buf.len() < n as usize { return Ok(None); }
                self.take_dec(n as usize);
                self.state = State::Done;
                Ok(Some(false))
            }
            State::AwaitYa => {
                // A plain BitTorrent handshake instead of MSE?
                let k = self.buf.len().min(BT_PROTOCOL.len());
                if k > 0 && self.buf[..k] == BT_PROTOCOL[..k] {
                    if k < BT_PROTOCOL.len() { return Ok(None); }
                    if !self.policy.allows_legacy() { return Err(Error::PlaintextRefused); }
                    self.provided = 0;
                    self.state = State::Done;
                    return Ok(Some(false));
                }
                if self.buf.len() < KEY_BYTES { return Ok(None); }
                let mut y = [0u8; KEY_BYTES];
                y.copy_from_slice(&self.buf[..KEY_BYTES]);
                self.buf.drain(..KEY_BYTES);
                self.compute_s(&y)?;
                // Message 2.
                self.out.extend_from_slice(&dh_public(&self.x));
                let pad = core::mem::take(&mut self.pad);
                self.out.extend_from_slice(&pad);
                self.state = State::AwaitReq1;
                Ok(Some(true))
            }
            State::AwaitReq1 => {
                let req1 = hash(&[b"req1", &self.s]);
                match find(&self.buf, &req1) {
                    Some(k) if k <= MAX_PAD => { self.buf.drain(..k + 20); self.state = State::AwaitReq23; Ok(Some(true)) }
                    Some(_) => Err(Error::NoSync),
                    None if self.buf.len() >= REQ1_SYNC_BOUND - KEY_BYTES => Err(Error::NoSync),
                    None => Ok(None),
                }
            }
            State::AwaitReq23 => {
                if self.buf.len() < 20 { return Ok(None); }
                let req3 = hash(&[b"req3", &self.s]);
                let mut req2 = [0u8; 20];
                for i in 0..20 { req2[i] = self.buf[i] ^ req3[i]; }
                self.buf.drain(..20);
                self.skey = lookup(&req2).ok_or(Error::UnknownTorrent)?;
                self.keys();
                self.state = State::AwaitVcProvide;
                Ok(Some(true))
            }
            State::AwaitVcProvide => {
                if self.buf.len() < 14 { return Ok(None); }
                let h = self.take_dec(14);
                if h[..8] != [0u8; 8] { return Err(Error::BadVc); }
                self.provided = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
                let pad_c = u16::from_be_bytes([h[12], h[13]]);
                if pad_c as usize > MAX_PAD { return Err(Error::Malformed); }
                self.state = State::AwaitPadC(pad_c);
                Ok(Some(true))
            }
            State::AwaitPadC(n) => {
                if self.buf.len() < n as usize + 2 { return Ok(None); }
                let v = self.take_dec(n as usize + 2);
                let ia = u16::from_be_bytes([v[v.len() - 2], v[v.len() - 1]]);
                self.state = State::AwaitIa(ia);
                Ok(Some(true))
            }
            State::AwaitIa(n) => {
                if self.buf.len() < n as usize { return Ok(None); }
                self.ia = self.take_dec(n as usize);
                let select = self.policy.select(self.provided).ok_or(Error::NoCommonMethod)?;
                self.provided = select;
                // Message 4: PadD is zeros, its length random.
                let pad_d = (u16::from_be_bytes(self.pad_seed) as usize) % (MAX_PAD + 1);
                let mut m = Vec::with_capacity(14 + pad_d);
                m.extend_from_slice(&[0u8; 8]);
                m.extend_from_slice(&select.to_be_bytes());
                m.extend_from_slice(&(pad_d as u16).to_be_bytes());
                m.resize(14 + pad_d, 0);
                if let Some(e) = &mut self.enc { e.apply(&mut m); }
                self.out.extend_from_slice(&m);
                self.state = State::Done;
                Ok(Some(false))
            }
            State::Done => Ok(None),
        }
    }

    fn finish(&mut self) -> Established {
        let mut rest = core::mem::take(&mut self.buf);
        let (method, cipher, info_hash) = match self.provided {
            0 => (Method::Legacy, Cipher::plaintext(), None),
            CRYPTO_RC4 => (Method::Rc4, Cipher { enc: self.enc.take(), dec: self.dec.take() }, Some(self.skey)),
            _ => (Method::Plaintext, Cipher::plaintext(), Some(self.skey)),
        };
        let mut cipher = cipher;
        cipher.decrypt(&mut rest);
        let mut payload = core::mem::take(&mut self.ia);
        if self.initiator { payload.clear(); }
        payload.extend_from_slice(&rest);
        self.x = [0; PRIVATE_BYTES];
        self.s = [0; KEY_BYTES];
        Established { method, cipher, info_hash, payload }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn rng(seed: u64) -> impl FnMut(&mut [u8]) {
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        move |b: &mut [u8]| for x in b { s ^= s << 13; s ^= s >> 7; s ^= s << 17; *x = s as u8; }
    }

    #[test]
    fn rc4_vectors() {
        let cases: [(&[u8], &[u8], &[u8]); 3] = [
            (b"Key", b"Plaintext", &[0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]),
            (b"Wiki", b"pedia", &[0x10, 0x21, 0xBF, 0x04, 0x20]),
            (b"Secret", b"Attack at dawn", &[0x45, 0xA0, 0x1F, 0x64, 0x5F, 0xC3, 0x5B, 0x38, 0x35, 0x52, 0x54, 0x4B, 0x9B, 0xF5]),
        ];
        for (k, p, c) in cases {
            let mut d = p.to_vec();
            Rc4::new(k).apply(&mut d);
            assert_eq!(&d[..], c);
        }
        // drop1024 = keystream from byte 1024 on.
        let mut long = vec![0u8; 1040];
        Rc4::new(b"k").apply(&mut long);
        let mut tail = vec![0u8; 16];
        Rc4::drop1024(b"k").apply(&mut tail);
        assert_eq!(&long[1024..], &tail[..]);
    }

    #[test]
    fn dh_agrees_and_rejects_weak_keys() {
        let mut one = [0u8; PRIVATE_BYTES];
        one[PRIVATE_BYTES - 1] = 1;
        let g = dh_public(&one);
        assert_eq!(g[KEY_BYTES - 1], 2);
        assert!(g[..KEY_BYTES - 1].iter().all(|&b| b == 0));
        let (mut a, mut b) = ([0u8; PRIVATE_BYTES], [0u8; PRIVATE_BYTES]);
        rng(1)(&mut a);
        rng(2)(&mut b);
        let (ya, yb) = (dh_public(&a), dh_public(&b));
        assert_eq!(dh_pow(&U768::from_be_slice(&yb), &a), dh_pow(&U768::from_be_slice(&ya), &b));
        assert!(valid_public(&ya));
        let mut zero = [0u8; KEY_BYTES];
        assert!(!valid_public(&zero));
        zero[KEY_BYTES - 1] = 1;
        assert!(!valid_public(&zero));
        assert!(!valid_public(&prime().to_be_bytes()));
        assert!(!valid_public(&prime().wrapping_sub(&U768::ONE).to_be_bytes()));
        assert!(!valid_public(&[0xff; KEY_BYTES]));
    }

    fn bt_handshake(ih: &[u8; 20]) -> Vec<u8> {
        let mut v = BT_PROTOCOL.to_vec();
        v.extend_from_slice(&[0u8; 8]);
        v.extend_from_slice(ih);
        v.extend_from_slice(b"-SX0100-abcdefghijkl");
        v
    }

    /// Runs a full handshake, delivering bytes in chunks of `chunk`.
    fn run(pa: Policy, pb: Policy, seed: u64, chunk: usize, torrents: &[[u8; 20]], skey: [u8; 20])
        -> Result<(Established, Established), Error>
    {
        let mut ra = rng(seed);
        let mut rb = rng(seed + 1000);
        let ia = bt_handshake(&skey);
        let mut a = Handshake::initiator(skey, pa, &ia, &mut ra);
        let mut b = Handshake::responder(pb, &mut rb);
        let mut lookup = |h: &[u8; 20]| torrents.iter().find(|t| &req2_hash(t) == h).copied();
        let (mut ea, mut eb) = (None, None);
        for _ in 0..10_000 {
            let to_b = a.take_output();
            let to_a = b.take_output();
            if to_b.is_empty() && to_a.is_empty() { break; }
            for c in to_b.chunks(chunk) {
                if eb.is_none() { if let Progress::Done(e) = b.feed(c, &mut lookup)? { eb = Some(e); } }
            }
            for c in to_a.chunks(chunk) {
                if ea.is_none() { if let Progress::Done(e) = a.feed(c, &mut |_| None)? { ea = Some(e); } }
            }
        }
        Ok((ea.expect("initiator finished"), eb.expect("responder finished")))
    }

    #[test]
    fn end_to_end_rc4_both_ways() {
        let t1 = [7u8; 20];
        let t2 = [9u8; 20];
        for (seed, chunk) in [(1, 1), (2, 7), (3, 1000), (4, 61), (5, 3)] {
            let (mut ea, mut eb) = run(Policy::PreferEncryption, Policy::PreferEncryption, seed, chunk, &[t1, t2], t2).unwrap();
            assert_eq!(ea.method, Method::Rc4);
            assert_eq!(eb.method, Method::Rc4);
            assert_eq!(eb.info_hash, Some(t2));
            assert_eq!(eb.payload, bt_handshake(&t2));
            assert!(ea.payload.is_empty());
            // B replies with its BitTorrent handshake, A sends a message; both through RC4.
            let mut m = bt_handshake(&t2);
            eb.cipher.encrypt(&mut m);
            assert_ne!(m, bt_handshake(&t2));
            ea.cipher.decrypt(&mut m);
            assert_eq!(m, bt_handshake(&t2));
            let mut m = b"\x00\x00\x00\x01\x02interested".to_vec();
            ea.cipher.encrypt(&mut m);
            eb.cipher.decrypt(&mut m);
            assert_eq!(m, b"\x00\x00\x00\x01\x02interested");
        }
    }

    #[test]
    fn plaintext_select() {
        let t = [3u8; 20];
        let (mut ea, mut eb) = run(Policy::PreferPlaintext, Policy::PreferPlaintext, 9, 13, &[t], t).unwrap();
        assert_eq!((ea.method, eb.method), (Method::Plaintext, Method::Plaintext));
        // IA still came through RC4 and arrives decrypted.
        assert_eq!(eb.payload, bt_handshake(&t));
        let mut m = b"hello".to_vec();
        eb.cipher.encrypt(&mut m);
        assert_eq!(m, b"hello");
        ea.cipher.decrypt(&mut m);
        assert_eq!(m, b"hello");
        // Prefer-encryption responder still picks RC4 when offered both.
        let (ea, _) = run(Policy::PreferPlaintext, Policy::PreferEncryption, 10, 5, &[t], t).unwrap();
        assert_eq!(ea.method, Method::Rc4);
        // Require on one side forces RC4.
        let (ea, _) = run(Policy::RequireEncryption, Policy::PreferPlaintext, 11, 5, &[t], t).unwrap();
        assert_eq!(ea.method, Method::Rc4);
    }

    #[test]
    fn unknown_torrent_fails() {
        let r = run(Policy::PreferEncryption, Policy::PreferEncryption, 4, 50, &[[1; 20]], [2; 20]);
        assert_eq!(r.unwrap_err(), Error::UnknownTorrent);
    }

    #[test]
    fn no_common_method() {
        assert_eq!(Policy::RequireEncryption.select(CRYPTO_PLAINTEXT), None);
        assert_eq!(Policy::PreferEncryption.select(0), None);
        assert_eq!(Policy::PreferEncryption.select(CRYPTO_PLAINTEXT), Some(CRYPTO_PLAINTEXT));
        assert_eq!(Policy::PreferPlaintext.select(CRYPTO_RC4), Some(CRYPTO_RC4));
        assert_eq!(Policy::PreferPlaintext.select(3), Some(CRYPTO_PLAINTEXT));
        assert_eq!(Policy::RequireEncryption.provide(), CRYPTO_RC4);
    }

    #[test]
    fn legacy_plaintext_handshake() {
        let t = [8u8; 20];
        let hs = bt_handshake(&t);
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(3));
        assert!(matches!(b.feed(&hs[..5], &mut |_| None), Ok(Progress::NeedMore)));
        match b.feed(&hs[5..], &mut |_| None).unwrap() {
            Progress::Done(e) => { assert_eq!(e.method, Method::Legacy); assert_eq!(e.payload, hs); }
            _ => panic!(),
        }
        assert!(b.take_output().is_empty());
        let mut b = Handshake::responder(Policy::RequireEncryption, &mut rng(3));
        assert_eq!(b.feed(&hs, &mut |_| None).unwrap_err(), Error::PlaintextRefused);
    }

    #[test]
    fn hostile_inputs() {
        // Garbage after a valid Ya: no req1 within 628 bytes.
        let mut ra = rng(5);
        let mut a = Handshake::initiator([1; 20], Policy::PreferEncryption, b"", &mut ra);
        let ya = a.take_output();
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(6));
        let mut junk = ya[..KEY_BYTES].to_vec();
        junk.extend(core::iter::repeat(0xAB).take(600));
        assert_eq!(b.feed(&junk, &mut |_| None).unwrap_err(), Error::NoSync);
        assert_eq!(b.feed(b"x", &mut |_| None).unwrap_err(), Error::Malformed);
        // A weak Ya.
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(6));
        let mut weak = [0u8; KEY_BYTES];
        weak[KEY_BYTES - 1] = 1;
        assert_eq!(b.feed(&weak, &mut |_| None).unwrap_err(), Error::BadKey);
        // A responder that sends a weak Yb.
        let mut a = Handshake::initiator([1; 20], Policy::PreferEncryption, b"", &mut rng(8));
        assert_eq!(a.feed(&[0u8; KEY_BYTES], &mut |_| None).unwrap_err(), Error::BadKey);
        // Initiator never sees VC.
        let mut a = Handshake::initiator([1; 20], Policy::PreferEncryption, b"", &mut rng(8));
        let yb = dh_public(&[3; PRIVATE_BYTES]);
        let mut m = yb.to_vec();
        m.extend(core::iter::repeat(0x55).take(VC_SYNC_BOUND));
        assert_eq!(a.feed(&m, &mut |_| None).unwrap_err(), Error::NoSync);
        // Truncated everywhere: feeding any prefix of a valid exchange never errors or panics.
        let t = [4u8; 20];
        let mut ra = rng(12);
        let mut a = Handshake::initiator(t, Policy::PreferEncryption, &bt_handshake(&t), &mut ra);
        let m1 = a.take_output();
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(13));
        let _ = b.feed(&m1, &mut |_| None).unwrap();
        let m2 = b.take_output();
        for cut in 0..m2.len() {
            let mut a2 = Handshake::initiator(t, Policy::PreferEncryption, &bt_handshake(&t), &mut rng(12));
            let _ = a2.take_output();
            assert!(matches!(a2.feed(&m2[..cut], &mut |_| None), Ok(Progress::NeedMore)));
        }
        // Too much input at once.
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(13));
        assert_eq!(b.feed(&vec![1u8; MAX_BUFFER + 1], &mut |_| None).unwrap_err(), Error::TooLong);
    }

    #[test]
    fn bad_vc_and_bad_pad_lengths() {
        // Build message 3 by hand with a non-zero VC.
        let t = [6u8; 20];
        let mut a = Handshake::initiator(t, Policy::PreferEncryption, b"", &mut rng(20));
        let m1 = a.take_output();
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(21));
        b.feed(&m1, &mut |_| None).unwrap();
        let m2 = b.take_output();
        let _ = a.feed(&m2, &mut |_| None).unwrap();
        let mut m3 = a.take_output();
        // Flip a bit in the encrypted VC (just after the two 20-byte hashes).
        m3[40] ^= 1;
        let mut lookup = |h: &[u8; 20]| (h == &req2_hash(&t)).then_some(t);
        assert_eq!(b.feed(&m3, &mut lookup).unwrap_err(), Error::BadVc);
        // Pad length over 512.
        let mut a = Handshake::initiator(t, Policy::PreferEncryption, b"", &mut rng(20));
        let m1 = a.take_output();
        let mut b = Handshake::responder(Policy::PreferEncryption, &mut rng(21));
        b.feed(&m1, &mut |_| None).unwrap();
        let m2 = b.take_output();
        let _ = a.feed(&m2, &mut |_| None).unwrap();
        let mut m3 = a.take_output();
        // len(PadC) sits at 40 + 12; its high byte is 0..=2, so this makes it > 512.
        m3[52] ^= 0xF0;
        assert_eq!(b.feed(&m3, &mut lookup).unwrap_err(), Error::Malformed);
    }
}
