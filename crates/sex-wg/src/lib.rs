// SPDX-License-Identifier: GPL-3.0-only
//! WireGuard for SexOS: the Noise_IKpsk2 handshake, transport data, cookies, replay
//! protection and the protocol's timers, written from the WireGuard paper (no code
//! taken from other implementations). No I/O: the network service hands this the
//! UDP payloads that came from the peer and the IP packets to tunnel, and sends what
//! comes back. Randomness and wall-clock time are the caller's inputs.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod blake2s;
mod x25519;

use alloc::vec::Vec;
use blake2s::{hash, kdf, mac};
use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, XChaCha20Poly1305};

const CONSTRUCTION: &[u8] = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
const IDENTIFIER: &[u8] = b"WireGuard v1 zx2c4 Jason@zx2c4.com";
const LABEL_MAC1: &[u8] = b"mac1----";
const LABEL_COOKIE: &[u8] = b"cookie--";
pub const INITIATION_BYTES: usize = 148;
pub const RESPONSE_BYTES: usize = 92;
pub const COOKIE_BYTES: usize = 64;
pub const DATA_HEADER: usize = 16;
/// Timers (section 6 of the paper), in milliseconds.
pub const REKEY_AFTER_TIME: u64 = 120_000;
pub const REJECT_AFTER_TIME: u64 = 180_000;
pub const REKEY_TIMEOUT: u64 = 5_000;
pub const KEEPALIVE_TIMEOUT: u64 = 10_000;
pub const REKEY_ATTEMPT_TIME: u64 = 90_000;
pub const COOKIE_LIFETIME: u64 = 120_000;
pub const REKEY_AFTER_MESSAGES: u64 = 1 << 60;
pub const REJECT_AFTER_MESSAGES: u64 = u64::MAX - (1 << 13);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a WireGuard message, or one of the wrong size.
    Malformed,
    /// A MAC, an AEAD tag or a key didn't check out.
    Authentication,
    /// A replayed handshake (old timestamp) or data counter.
    Replay,
    /// No session to send or receive with: a handshake is needed.
    NoSession,
    /// Not addressed to a session of ours.
    UnknownIndex,
    /// The peer's static key isn't the one this tunnel trusts.
    WrongPeer,
}

/// What a message from the peer produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// An IP packet that came through the tunnel.
    Packet(Vec<u8>),
    /// A keepalive (nothing to deliver).
    Keepalive,
    /// Send these bytes to the peer (a handshake response or a keepalive confirming one).
    Reply(Vec<u8>),
    /// A cookie came: the next initiation carries it (the peer is under load).
    Cookie,
}

/// Something the timers say to do now.
#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    /// Start (or retry) a handshake: call `initiate`.
    Handshake,
    /// Send this keepalive.
    Keepalive(Vec<u8>),
    /// Every attempt for `REKEY_ATTEMPT_TIME` failed: the tunnel is down until traffic asks again.
    GiveUp,
}

pub fn public_key(private: &[u8; 32]) -> [u8; 32] { x25519::x25519(private, &x25519::BASE) }

fn dh(private: &[u8; 32], public: &[u8; 32]) -> Result<[u8; 32], Error> {
    let s = x25519::x25519(private, public);
    // An all-zero result means a low-order point: refuse it.
    if s == [0; 32] { Err(Error::Authentication) } else { Ok(s) }
}
fn nonce(counter: u64) -> [u8; 12] { let mut n = [0u8; 12]; n[4..].copy_from_slice(&counter.to_le_bytes()); n }
fn seal(key: &[u8; 32], counter: u64, plain: &[u8], ad: &[u8]) -> Vec<u8> {
    let mut buf = plain.to_vec();
    let tag = ChaCha20Poly1305::new(key.into()).encrypt_in_place_detached(&nonce(counter).into(), ad, &mut buf).expect("length");
    buf.extend_from_slice(&tag);
    buf
}
fn open(key: &[u8; 32], counter: u64, sealed: &[u8], ad: &[u8]) -> Result<Vec<u8>, Error> {
    if sealed.len() < 16 { return Err(Error::Malformed); }
    let (c, tag) = sealed.split_at(sealed.len() - 16);
    let mut buf = c.to_vec();
    ChaCha20Poly1305::new(key.into()).decrypt_in_place_detached(&nonce(counter).into(), ad, &mut buf, tag.into()).map_err(|_| Error::Authentication)?;
    Ok(buf)
}
fn mix(h: &[u8; 32], data: &[u8]) -> [u8; 32] { hash(&[h, data]) }

/// A sliding window of the last 2048 counters seen (RFC 6479's idea, as WireGuard uses).
struct Window { top: u64, bits: [u64; 32], any: bool }
impl Window {
    const SIZE: u64 = 2048;
    fn new() -> Self { Self { top: 0, bits: [0; 32], any: false } }
    fn check(&self, n: u64) -> bool {
        if !self.any || n > self.top { return n < REJECT_AFTER_MESSAGES; }
        if self.top - n >= Self::SIZE { return false; }
        self.bits[((n / 64) % 32) as usize] & (1 << (n % 64)) == 0
    }
    fn set(&mut self, n: u64) {
        if !self.any || n > self.top {
            let from = if self.any { self.top / 64 + 1 } else { n / 64 };
            let to = n / 64;
            // Clear the words the window slid over.
            if to >= from { for w in from..=to.min(from + 32) { self.bits[(w % 32) as usize] = 0; } }
            self.top = n;
            self.any = true;
        }
        self.bits[((n / 64) % 32) as usize] |= 1 << (n % 64);
    }
}

struct Session { ours: u32, theirs: u32, send: [u8; 32], recv: [u8; 32], counter: u64, window: Window, born: u64, initiator: bool, confirmed: bool }

/// A handshake we started and await the answer to.
struct Pending { index: u32, ephemeral: [u8; 32], hash: [u8; 32], chain: [u8; 32], sent_at: u64 }

pub struct Tunnel {
    private: [u8; 32],
    public: [u8; 32],
    peer: [u8; 32],
    psk: [u8; 32],
    /// The peer's MAC keys: mac1 key for messages to it, cookie key for its cookies.
    peer_mac1: [u8; 32],
    peer_cookie_key: [u8; 32],
    our_mac1: [u8; 32],
    our_cookie_key: [u8; 32],
    pending: Option<Pending>,
    current: Option<Session>,
    previous: Option<Session>,
    /// The newest initiation timestamp taken from the peer (replay protection).
    last_timestamp: [u8; 12],
    cookie: Option<([u8; 16], u64)>,
    last_mac1: [u8; 16],
    /// Timers: first attempt of the current handshake run, last data sent and received.
    attempt_began: Option<u64>,
    last_sent: u64,
    last_received: u64,
    persistent_keepalive: Option<u64>,
}

impl Tunnel {
    /// A tunnel between `private` (ours) and the peer's `peer` public key, with an
    /// optional preshared key.
    pub fn new(private: [u8; 32], peer: [u8; 32], psk: Option<[u8; 32]>) -> Self {
        let public = public_key(&private);
        Self {
            private, public, peer, psk: psk.unwrap_or([0; 32]),
            peer_mac1: hash(&[LABEL_MAC1, &peer]), peer_cookie_key: hash(&[LABEL_COOKIE, &peer]),
            our_mac1: hash(&[LABEL_MAC1, &public]), our_cookie_key: hash(&[LABEL_COOKIE, &public]),
            pending: None, current: None, previous: None, last_timestamp: [0; 12], cookie: None, last_mac1: [0; 16],
            attempt_began: None, last_sent: 0, last_received: 0, persistent_keepalive: None,
        }
    }
    pub fn public(&self) -> [u8; 32] { self.public }
    /// Sends a keepalive at least this often while idle (for NAT), in seconds.
    pub fn set_persistent_keepalive(&mut self, seconds: Option<u16>) { self.persistent_keepalive = seconds.map(|s| s as u64 * 1000); }
    /// A session is up and may carry packets.
    pub fn up(&self, now: u64) -> bool { self.current.as_ref().is_some_and(|s| now.saturating_sub(s.born) < REJECT_AFTER_TIME && (s.initiator || s.confirmed)) }

    fn macs(&mut self, msg: &mut [u8], now: u64) {
        let n = msg.len();
        let m1 = mac(&self.peer_mac1, &msg[..n - 32]);
        msg[n - 32..n - 16].copy_from_slice(&m1);
        self.last_mac1 = m1;
        let m2 = match self.cookie { Some((c, at)) if now.saturating_sub(at) < COOKIE_LIFETIME => mac(&c, &msg[..n - 16]), _ => [0; 16] };
        msg[n - 16..].copy_from_slice(&m2);
    }

    /// A handshake initiation. `ephemeral` is 32 fresh random bytes, `index` a fresh
    /// random session number, `timestamp` TAI64N of now (see `tai64n`).
    pub fn initiate(&mut self, now: u64, ephemeral: [u8; 32], index: u32, timestamp: [u8; 12]) -> [u8; INITIATION_BYTES] {
        let c = hash(&[CONSTRUCTION]);
        let h = mix(&mix(&c, IDENTIFIER), &self.peer);
        let e_pub = public_key(&ephemeral);
        let [c] = kdf::<1>(&c, &e_pub);
        let h = mix(&h, &e_pub);
        let mut msg = [0u8; INITIATION_BYTES];
        msg[0] = 1;
        msg[4..8].copy_from_slice(&index.to_le_bytes());
        msg[8..40].copy_from_slice(&e_pub);
        // A peer key that is a low-order point makes the DH zero: the message then
        // carries garbage the peer refuses, which is all such a misconfiguration deserves.
        let [c, k] = kdf::<2>(&c, &dh(&ephemeral, &self.peer).unwrap_or([0; 32]));
        let s = seal(&k, 0, &self.public, &h);
        msg[40..88].copy_from_slice(&s);
        let h = mix(&h, &s);
        let [c, k] = kdf::<2>(&c, &dh(&self.private, &self.peer).unwrap_or([0; 32]));
        let t = seal(&k, 0, &timestamp, &h);
        msg[88..116].copy_from_slice(&t);
        let h = mix(&h, &t);
        self.macs(&mut msg, now);
        self.pending = Some(Pending { index, ephemeral, hash: h, chain: c, sent_at: now });
        if self.attempt_began.is_none() { self.attempt_began = Some(now); }
        self.last_sent = now;
        msg
    }

    /// Wraps an IP packet for the peer. `Err(NoSession)` means a handshake is needed
    /// first (`initiate`), and the packet should be held until it completes.
    pub fn send(&mut self, now: u64, packet: &[u8]) -> Result<Vec<u8>, Error> {
        let s = self.current.as_mut().filter(|s| now.saturating_sub(s.born) < REJECT_AFTER_TIME && s.counter < REJECT_AFTER_MESSAGES).ok_or(Error::NoSession)?;
        // The responder may not send until the initiator's first data confirms the keys.
        if !s.initiator && !s.confirmed { return Err(Error::NoSession); }
        let mut msg = Vec::with_capacity(DATA_HEADER + packet.len() + 32);
        msg.extend_from_slice(&[4, 0, 0, 0]);
        msg.extend_from_slice(&s.theirs.to_le_bytes());
        msg.extend_from_slice(&s.counter.to_le_bytes());
        let mut padded = packet.to_vec();
        padded.resize(packet.len().div_ceil(16) * 16, 0);
        msg.extend(seal(&s.send, s.counter, &padded, &[]));
        s.counter += 1;
        self.last_sent = now;
        Ok(msg)
    }

    /// Takes one UDP payload from the peer. `fresh` gives random bytes when a
    /// response must be made (32 for its ephemeral key, then 4 for its index).
    pub fn receive(&mut self, now: u64, msg: &[u8], fresh: &mut dyn FnMut(&mut [u8])) -> Result<Received, Error> {
        match (msg.first(), msg.len()) {
            (Some(1), INITIATION_BYTES) => self.respond(now, msg, fresh),
            (Some(2), RESPONSE_BYTES) => self.complete(now, msg),
            (Some(3), COOKIE_BYTES) => self.take_cookie(now, msg),
            (Some(4), n) if n >= DATA_HEADER + 16 => self.data(now, msg),
            _ => Err(Error::Malformed),
        }
    }

    fn check_mac1(&self, msg: &[u8]) -> Result<(), Error> {
        let n = msg.len();
        if mac(&self.our_mac1, &msg[..n - 32]) != msg[n - 32..n - 16] { return Err(Error::Authentication); }
        Ok(())
    }

    fn respond(&mut self, now: u64, msg: &[u8], fresh: &mut dyn FnMut(&mut [u8])) -> Result<Received, Error> {
        self.check_mac1(msg)?;
        if msg[1..4] != [0, 0, 0] { return Err(Error::Malformed); }
        let c = hash(&[CONSTRUCTION]);
        let h = mix(&mix(&c, IDENTIFIER), &self.public);
        let e_i: [u8; 32] = msg[8..40].try_into().unwrap();
        let [c] = kdf::<1>(&c, &e_i);
        let h = mix(&h, &e_i);
        let [c, k] = kdf::<2>(&c, &dh(&self.private, &e_i)?);
        let s = open(&k, 0, &msg[40..88], &h)?;
        if s != self.peer { return Err(Error::WrongPeer); }
        let h = mix(&h, &msg[40..88]);
        let [c, k] = kdf::<2>(&c, &dh(&self.private, &self.peer)?);
        let t = open(&k, 0, &msg[88..116], &h)?;
        let t: [u8; 12] = t.try_into().map_err(|_| Error::Malformed)?;
        // TAI64N compares as big-endian bytes; an old or repeated one is a replay.
        if t <= self.last_timestamp { return Err(Error::Replay); }
        self.last_timestamp = t;
        let h = mix(&h, &msg[88..116]);
        let theirs = u32::from_le_bytes(msg[4..8].try_into().unwrap());
        // Our half.
        let mut eph = [0u8; 32];
        fresh(&mut eph);
        let mut idx = [0u8; 4];
        fresh(&mut idx);
        let ours = u32::from_le_bytes(idx);
        let e_r = public_key(&eph);
        let mut out = [0u8; RESPONSE_BYTES];
        out[0] = 2;
        out[4..8].copy_from_slice(&ours.to_le_bytes());
        out[8..12].copy_from_slice(&theirs.to_le_bytes());
        out[12..44].copy_from_slice(&e_r);
        let [c] = kdf::<1>(&c, &e_r);
        let h = mix(&h, &e_r);
        let [c] = kdf::<1>(&c, &dh(&eph, &e_i)?);
        let [c] = kdf::<1>(&c, &dh(&eph, &self.peer)?);
        let [c, tau, k] = kdf::<3>(&c, &self.psk);
        let h = mix(&h, &tau);
        let empty = seal(&k, 0, &[], &h);
        out[44..60].copy_from_slice(&empty);
        self.macs(&mut out, now);
        let [recv, send] = kdf::<2>(&c, &[]);
        self.previous = self.current.take();
        self.current = Some(Session { ours, theirs, send, recv, counter: 0, window: Window::new(), born: now, initiator: false, confirmed: false });
        self.last_sent = now;
        self.last_received = now;
        Ok(Received::Reply(out.to_vec()))
    }

    fn complete(&mut self, now: u64, msg: &[u8]) -> Result<Received, Error> {
        self.check_mac1(msg)?;
        let p = self.pending.as_ref().ok_or(Error::UnknownIndex)?;
        if u32::from_le_bytes(msg[8..12].try_into().unwrap()) != p.index { return Err(Error::UnknownIndex); }
        let e_r: [u8; 32] = msg[12..44].try_into().unwrap();
        let [c] = kdf::<1>(&p.chain, &e_r);
        let h = mix(&p.hash, &e_r);
        let [c] = kdf::<1>(&c, &dh(&p.ephemeral, &e_r)?);
        let [c] = kdf::<1>(&c, &dh(&self.private, &e_r)?);
        let [c, tau, k] = kdf::<3>(&c, &self.psk);
        let h = mix(&h, &tau);
        open(&k, 0, &msg[44..60], &h)?;
        let [send, recv] = kdf::<2>(&c, &[]);
        let theirs = u32::from_le_bytes(msg[4..8].try_into().unwrap());
        let ours = p.index;
        self.pending = None;
        self.attempt_began = None;
        self.previous = self.current.take();
        self.current = Some(Session { ours, theirs, send, recv, counter: 0, window: Window::new(), born: now, initiator: true, confirmed: true });
        self.last_received = now;
        // A keepalive confirms the session to the responder, which may not send before.
        let k = self.send(now, &[])?;
        Ok(Received::Reply(k))
    }

    fn take_cookie(&mut self, now: u64, msg: &[u8]) -> Result<Received, Error> {
        let p = self.pending.as_ref().ok_or(Error::UnknownIndex)?;
        if u32::from_le_bytes(msg[4..8].try_into().unwrap()) != p.index { return Err(Error::UnknownIndex); }
        let mut c = msg[32..48].to_vec();
        let tag: [u8; 16] = msg[48..64].try_into().unwrap();
        XChaCha20Poly1305::new((&self.peer_cookie_key).into())
            .decrypt_in_place_detached(msg[8..32].into(), &self.last_mac1, &mut c, (&tag).into()).map_err(|_| Error::Authentication)?;
        self.cookie = Some((c.try_into().unwrap(), now));
        Ok(Received::Cookie)
    }

    /// A cookie reply for an initiation we won't process now (under load). `secret`
    /// is the responder's rotating secret, `source` the initiator's address and port
    /// bytes, `nonce` 24 random bytes.
    pub fn cookie_reply(&self, initiation: &[u8], secret: &[u8; 32], source: &[u8], nonce: [u8; 24]) -> Result<[u8; COOKIE_BYTES], Error> {
        if initiation.len() != INITIATION_BYTES { return Err(Error::Malformed); }
        self.check_mac1(initiation)?;
        let cookie = mac(secret, source);
        let mut out = [0u8; COOKIE_BYTES];
        out[0] = 3;
        out[4..8].copy_from_slice(&initiation[4..8]);
        out[8..32].copy_from_slice(&nonce);
        let mut c = cookie.to_vec();
        let tag = XChaCha20Poly1305::new((&self.our_cookie_key).into())
            .encrypt_in_place_detached((&nonce).into(), &initiation[INITIATION_BYTES - 32..INITIATION_BYTES - 16], &mut c).map_err(|_| Error::Malformed)?;
        out[32..48].copy_from_slice(&c);
        out[48..64].copy_from_slice(&tag);
        Ok(out)
    }

    fn data(&mut self, now: u64, msg: &[u8]) -> Result<Received, Error> {
        let index = u32::from_le_bytes(msg[4..8].try_into().unwrap());
        let counter = u64::from_le_bytes(msg[8..16].try_into().unwrap());
        let s = match (&mut self.current, &mut self.previous) {
            (Some(s), _) if s.ours == index => s,
            (_, Some(s)) if s.ours == index => s,
            _ => return Err(Error::UnknownIndex),
        };
        if now.saturating_sub(s.born) >= REJECT_AFTER_TIME { return Err(Error::NoSession); }
        if !s.window.check(counter) { return Err(Error::Replay); }
        let plain = open(&s.recv, counter, &msg[16..], &[])?;
        s.window.set(counter);
        // The initiator's first data confirms the keys to us, the responder.
        s.confirmed = true;
        self.last_received = now;
        if plain.is_empty() { return Ok(Received::Keepalive); }
        // Cut the padding by the IP header's own length.
        let len = match plain[0] >> 4 {
            4 if plain.len() >= 20 => u16::from_be_bytes([plain[2], plain[3]]) as usize,
            6 if plain.len() >= 40 => 40 + u16::from_be_bytes([plain[4], plain[5]]) as usize,
            _ => return Err(Error::Malformed),
        };
        if len > plain.len() { return Err(Error::Malformed); }
        Ok(Received::Packet(plain[..len].to_vec()))
    }

    /// What the timers want now. Call every so often (each second is plenty).
    pub fn tick(&mut self, now: u64) -> Option<Due> {
        if let Some(began) = self.attempt_began {
            if now.saturating_sub(began) >= REKEY_ATTEMPT_TIME { self.attempt_began = None; self.pending = None; return Some(Due::GiveUp); }
            if self.pending.as_ref().is_some_and(|p| now.saturating_sub(p.sent_at) >= REKEY_TIMEOUT) { return Some(Due::Handshake); }
            if self.pending.is_some() { return None; }
        }
        let s = self.current.as_ref()?;
        let age = now.saturating_sub(s.born);
        // The initiator renews keys before they get old; nobody uses them past REJECT.
        if s.initiator && (age >= REKEY_AFTER_TIME || s.counter >= REKEY_AFTER_MESSAGES) { return Some(Due::Handshake); }
        if age >= REJECT_AFTER_TIME { self.current = None; return None; }
        let confirmed = s.initiator || s.confirmed;
        // Data came but nothing went back for a while: tell the peer we're alive.
        let idle_reply = self.last_received > self.last_sent && now.saturating_sub(self.last_received) >= KEEPALIVE_TIMEOUT;
        let persistent = self.persistent_keepalive.is_some_and(|p| now.saturating_sub(self.last_sent) >= p);
        if confirmed && (idle_reply || persistent) { return self.send(now, &[]).ok().map(Due::Keepalive); }
        None
    }
}

/// TAI64N for `unix_seconds` and `nanos`: what `initiate` wants as its timestamp.
pub fn tai64n(unix_seconds: u64, nanos: u32) -> [u8; 12] {
    let mut t = [0u8; 12];
    t[..8].copy_from_slice(&(0x4000_0000_0000_000a_u64 + unix_seconds).to_be_bytes());
    t[8..].copy_from_slice(&nanos.to_be_bytes());
    t
}

#[cfg(test)]
mod tests;
