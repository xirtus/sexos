// SPDX-License-Identifier: GPL-3.0-only
//! DTLS 1.2 (RFC 6347) the way WebRTC uses it: both sides hold self-signed certificates
//! checked against the SDP fingerprint, the handshake negotiates SRTP keys (RFC 5764), and
//! the same association carries SCTP data channels as application data.
//!
//! No I/O. Datagrams go in with `handle_datagram`, come out of `poll_transmit`; the caller
//! keeps time and calls `handle_timeout` at `poll_timeout`. Randomness comes from a seed.
//!
//! Cipher suite TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 on P-256, extended master secret
//! (RFC 7627) required. Flights are retransmitted with a doubling timer and fragmented to
//! the MTU; fragments are reassembled in any order; epoch 1 records pass a 64-record replay
//! window. RSA certificates are not supported (browsers send ECDSA ones for WebRTC).
#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod cert;

pub use cert::{public_key, Certificate, Fingerprint};
pub use sex_srtp::Profile as SrtpProfile;

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce, Tag};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use hmac::{Hmac, Mac};
use p256::ecdh::EphemeralSecret;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, VerifyingKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::PublicKey;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const DTLS12: u16 = 0xfefd;
const DTLS10: u16 = 0xfeff;
pub const TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256: u16 = 0xc02b;
const SECP256R1: u16 = 23;
const ECDSA_SHA256: u16 = 0x0403;

const CT_CCS: u8 = 20;
const CT_ALERT: u8 = 21;
const CT_HANDSHAKE: u8 = 22;
const CT_APP: u8 = 23;

const HT_CLIENT_HELLO: u8 = 1;
const HT_SERVER_HELLO: u8 = 2;
const HT_HELLO_VERIFY: u8 = 3;
const HT_CERTIFICATE: u8 = 11;
const HT_SERVER_KEY_EXCHANGE: u8 = 12;
const HT_CERTIFICATE_REQUEST: u8 = 13;
const HT_SERVER_HELLO_DONE: u8 = 14;
const HT_CERTIFICATE_VERIFY: u8 = 15;
const HT_CLIENT_KEY_EXCHANGE: u8 = 16;
const HT_FINISHED: u8 = 20;

const EXT_SUPPORTED_GROUPS: u16 = 10;
const EXT_POINT_FORMATS: u16 = 11;
const EXT_SIGNATURE_ALGORITHMS: u16 = 13;
const EXT_USE_SRTP: u16 = 14;
const EXT_EMS: u16 = 23;
const EXT_RENEGOTIATION_INFO: u16 = 0xff01;

/// Alert descriptions we send or name.
pub mod alert {
    pub const CLOSE_NOTIFY: u8 = 0;
    pub const UNEXPECTED_MESSAGE: u8 = 10;
    pub const HANDSHAKE_FAILURE: u8 = 40;
    pub const BAD_CERTIFICATE: u8 = 42;
    pub const UNSUPPORTED_CERTIFICATE: u8 = 43;
    pub const ILLEGAL_PARAMETER: u8 = 47;
    pub const DECODE_ERROR: u8 = 50;
    pub const DECRYPT_ERROR: u8 = 51;
    pub const INSUFFICIENT_SECURITY: u8 = 71;
}

const RECORD_HEADER: usize = 13;
const HS_HEADER: usize = 12;
const GCM_OVERHEAD: usize = 8 + 16;
const MAX_MESSAGE: usize = 1 << 16;
const MAX_RETRANSMITS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A handshake message did not parse.
    Decode,
    /// A message arrived that the handshake does not allow here.
    Unexpected,
    /// The peer's certificate does not match the SDP fingerprint.
    Fingerprint,
    /// The peer's certificate is not P-256 ECDSA.
    UnsupportedCertificate,
    /// A signature or Finished did not verify.
    Verify,
    /// No cipher suite, group, signature or extended master secret in common.
    Negotiation,
    /// No SRTP profile in common.
    NoSrtpProfile,
    /// The peer sent a fatal alert.
    Alert(u8),
    /// Flights went unanswered.
    Timeout,
    /// Not connected, or closed.
    NotConnected,
    /// Application data over the record size.
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Handshake done; keys can be exported.
    Connected,
    /// Application data (SCTP for data channels).
    Data(Vec<u8>),
    /// The peer sent close_notify.
    Closed,
}

#[derive(Clone)]
pub struct Config {
    pub role: Role,
    pub certificate: Certificate,
    /// The peer's fingerprint from the remote SDP.
    pub remote_fingerprint: Fingerprint,
    /// SRTP profiles we accept, most wanted first. Empty: no use_srtp (data channels only).
    pub srtp_profiles: Vec<SrtpProfile>,
    /// Largest datagram we send.
    pub mtu: usize,
    /// Server: answer a first ClientHello with a HelloVerifyRequest cookie.
    pub hello_verify: bool,
    /// First retransmission timeout; it doubles up to `max_rto_ms`.
    pub initial_rto_ms: u64,
    pub max_rto_ms: u64,
}

impl Config {
    pub fn new(role: Role, certificate: Certificate, remote_fingerprint: Fingerprint) -> Config {
        Config {
            role,
            certificate,
            remote_fingerprint,
            srtp_profiles: Vec::from([SrtpProfile::AeadAes128Gcm, SrtpProfile::Aes128CmHmacSha1_80]),
            mtu: 1200,
            hello_verify: false,
            initial_rto_ms: 1000,
            max_rto_ms: 60_000,
        }
    }
}

/// SRTP master keys from the handshake (RFC 5764 4.2), as this side uses them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrtpKeys {
    pub profile: SrtpProfile,
    pub local_key: Vec<u8>,
    pub local_salt: Vec<u8>,
    pub remote_key: Vec<u8>,
    pub remote_salt: Vec<u8>,
}

impl SrtpKeys {
    /// (protect, unprotect) contexts.
    pub fn contexts(&self) -> (sex_srtp::Context, sex_srtp::Context) {
        let tx = sex_srtp::Context::new(self.profile, &self.local_key, &self.local_salt).unwrap();
        let rx = sex_srtp::Context::new(self.profile, &self.remote_key, &self.remote_salt).unwrap();
        (tx, rx)
    }
}

/// TLS 1.2 PRF, P_SHA256 (RFC 5246 5).
pub fn prf(secret: &[u8], label: &[u8], seed: &[u8], out: &mut [u8]) {
    let mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).unwrap();
    let mut a = {
        let mut m = mac.clone();
        m.update(label);
        m.update(seed);
        m.finalize().into_bytes()
    };
    for chunk in out.chunks_mut(32) {
        let mut m = mac.clone();
        m.update(&a);
        m.update(label);
        m.update(seed);
        let b = m.finalize().into_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
        let mut m = mac.clone();
        m.update(&a);
        a = m.finalize().into_bytes();
    }
}

/// AES-128-GCM record protection (RFC 5288): 4-byte implicit salt, 8-byte explicit nonce.
struct Cipher {
    aead: Aes128Gcm,
    iv: [u8; 4],
}

impl Cipher {
    fn new(key: &[u8], iv: &[u8]) -> Cipher {
        Cipher { aead: Aes128Gcm::new_from_slice(key).unwrap(), iv: iv.try_into().unwrap() }
    }
    fn aad(seq: u64, ct: u8, len: usize) -> [u8; 13] {
        let mut a = [0u8; 13];
        a[..8].copy_from_slice(&seq.to_be_bytes());
        a[8] = ct;
        a[9..11].copy_from_slice(&DTLS12.to_be_bytes());
        a[11..].copy_from_slice(&(len as u16).to_be_bytes());
        a
    }
    /// `seq` here is epoch << 48 | sequence number.
    fn seal(&self, seq: u64, ct: u8, plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.iv);
        nonce[4..].copy_from_slice(&seq.to_be_bytes());
        let mut out = Vec::with_capacity(plain.len() + GCM_OVERHEAD);
        out.extend_from_slice(&seq.to_be_bytes());
        out.extend_from_slice(plain);
        let tag = self
            .aead
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), &Self::aad(seq, ct, plain.len()), &mut out[8..])
            .unwrap();
        out.extend_from_slice(&tag);
        out
    }
    fn open(&self, seq: u64, ct: u8, payload: &[u8]) -> Option<Vec<u8>> {
        if payload.len() < GCM_OVERHEAD {
            return None;
        }
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.iv);
        nonce[4..].copy_from_slice(&payload[..8]);
        let n = payload.len() - GCM_OVERHEAD;
        let mut out = Vec::from(&payload[8..8 + n]);
        self.aead
            .decrypt_in_place_detached(
                Nonce::from_slice(&nonce),
                &Self::aad(seq, ct, n),
                &mut out,
                Tag::from_slice(&payload[8 + n..]),
            )
            .ok()?;
        Some(out)
    }
}

/// 64-record replay window over one epoch's sequence numbers (RFC 6347 4.1.2.6).
#[derive(Default, Clone, Copy)]
struct Window {
    top: u64,
    bits: u64,
    any: bool,
}

impl Window {
    fn check(&self, s: u64) -> bool {
        !self.any || s > self.top || (self.top - s < 64 && self.bits & (1 << (self.top - s)) == 0)
    }
    fn accept(&mut self, s: u64) {
        if !self.any {
            *self = Window { top: s, bits: 1, any: true };
        } else if s > self.top {
            let d = s - self.top;
            self.bits = if d >= 64 { 0 } else { self.bits << d } | 1;
            self.top = s;
        } else if self.top - s < 64 {
            self.bits |= 1 << (self.top - s);
        }
    }
}

/// A handshake message being reassembled from fragments.
struct Partial {
    ty: u8,
    body: Vec<u8>,
    have: Vec<(usize, usize)>,
}

impl Partial {
    fn add(&mut self, off: usize, data: &[u8]) {
        self.body[off..off + data.len()].copy_from_slice(data);
        self.have.push((off, off + data.len()));
        self.have.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for &(a, b) in &self.have {
            match merged.last_mut() {
                Some(l) if a <= l.1 => l.1 = l.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        self.have = merged;
    }
    fn done(&self) -> bool {
        self.body.is_empty() || self.have == [(0, self.body.len())]
    }
}

/// One message of a flight we may have to send again.
#[derive(Clone)]
enum Out {
    Hs { epoch: u16, ty: u8, seq: u16, body: Vec<u8> },
    Ccs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Idle,
    CHello,
    CCert,
    CKeyExchange,
    CCertReqOrDone,
    CDone,
    CCcs,
    CFinished,
    SHello,
    SCert,
    SKeyExchange,
    SVerify,
    SCcs,
    SFinished,
    Connected,
    Closed,
    Failed,
}

/// A DTLS 1.2 association with one peer.
pub struct Dtls {
    cfg: Config,
    rng: ChaCha20Rng,
    st: St,
    client_random: [u8; 32],
    server_random: [u8; 32],
    cookie: Vec<u8>,
    cookie_secret: [u8; 32],
    ecdh: Option<EphemeralSecret>,
    peer_ecdh: Option<PublicKey>,
    peer_key: Option<VerifyingKey>,
    cert_requested: bool,
    profile: Option<SrtpProfile>,
    master: Option<[u8; 48]>,
    transcript: Vec<u8>,
    send_seq: u16,
    recv_seq: u16,
    peer_flight_start: u16,
    partial: BTreeMap<u16, Partial>,
    write_epoch: u16,
    read_epoch: u16,
    write_rseq: [u64; 2],
    write_cipher: Option<Cipher>,
    read_cipher: Option<Cipher>,
    next_read_cipher: Option<Cipher>,
    replay: [Window; 2],
    future: Vec<Vec<u8>>,
    flight: Vec<Out>,
    timer: Option<u64>,
    rto: u64,
    retries: u32,
    out: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
    peer_reneg: bool,
    peer_point_formats: bool,
    resend: bool,
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err(Error::Decode);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u24(&mut self) -> Result<usize, Error> {
        let b = self.take(3)?;
        Ok((b[0] as usize) << 16 | (b[1] as usize) << 8 | b[2] as usize)
    }
    fn v8(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u8()? as usize;
        self.take(n)
    }
    fn v16(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn v24(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u24()?;
        self.take(n)
    }
    fn rest(&mut self) -> &'a [u8] {
        core::mem::take(&mut self.0)
    }
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn put24(v: &mut Vec<u8>, x: usize) {
    v.extend_from_slice(&(x as u32).to_be_bytes()[1..]);
}
fn ext(v: &mut Vec<u8>, ty: u16, body: &[u8]) {
    put16(v, ty);
    put16(v, body.len() as u16);
    v.extend_from_slice(body);
}

/// Extensions of a hello: (type, body) pairs.
fn parse_exts(d: &[u8]) -> Result<Vec<(u16, &[u8])>, Error> {
    let mut r = Reader(d);
    let mut v = Vec::new();
    while !r.0.is_empty() {
        let t = r.u16()?;
        v.push((t, r.v16()?));
    }
    Ok(v)
}

fn find<'a>(exts: &[(u16, &'a [u8])], t: u16) -> Option<&'a [u8]> {
    exts.iter().find(|e| e.0 == t).map(|e| e.1)
}

fn u16_list(d: &[u8]) -> Result<Vec<u16>, Error> {
    let mut r = Reader(d);
    let l = r.v16()?;
    if l.len() % 2 != 0 {
        return Err(Error::Decode);
    }
    Ok(l.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
}

/// use_srtp body: profile list and an empty MKI.
fn use_srtp_body(profiles: &[SrtpProfile]) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, profiles.len() as u16 * 2);
    for p in profiles {
        put16(&mut b, p.id());
    }
    b.push(0);
    b
}

/// Is this datagram DTLS? (RFC 7983 demultiplexing: first byte 20..=63.)
pub fn is_dtls(first_byte: u8) -> bool {
    (20..=63).contains(&first_byte)
}

impl Dtls {
    pub fn new(cfg: Config, seed: [u8; 32]) -> Dtls {
        let mut rng = ChaCha20Rng::from_seed(seed);
        let mut cookie_secret = [0u8; 32];
        rng.fill_bytes(&mut cookie_secret);
        let st = if cfg.role == Role::Server { St::SHello } else { St::Idle };
        let rto = cfg.initial_rto_ms;
        Dtls {
            cfg,
            rng,
            st,
            client_random: [0; 32],
            server_random: [0; 32],
            cookie: Vec::new(),
            cookie_secret,
            ecdh: None,
            peer_ecdh: None,
            peer_key: None,
            cert_requested: false,
            profile: None,
            master: None,
            transcript: Vec::new(),
            send_seq: 0,
            recv_seq: 0,
            peer_flight_start: 0,
            partial: BTreeMap::new(),
            write_epoch: 0,
            read_epoch: 0,
            write_rseq: [0; 2],
            write_cipher: None,
            read_cipher: None,
            next_read_cipher: None,
            replay: [Window::default(); 2],
            future: Vec::new(),
            flight: Vec::new(),
            timer: None,
            rto,
            retries: 0,
            out: VecDeque::new(),
            events: VecDeque::new(),
            peer_reneg: false,
            peer_point_formats: false,
            resend: false,
        }
    }

    pub fn role(&self) -> Role {
        self.cfg.role
    }
    pub fn is_connected(&self) -> bool {
        self.st == St::Connected
    }
    pub fn is_closed(&self) -> bool {
        matches!(self.st, St::Closed | St::Failed)
    }
    pub fn srtp_profile(&self) -> Option<SrtpProfile> {
        self.profile
    }

    /// Client: send the first ClientHello. Server: nothing to do.
    pub fn start(&mut self, now_ms: u64) {
        if self.st != St::Idle {
            return;
        }
        self.rng.fill_bytes(&mut self.client_random);
        self.send_client_hello(now_ms);
    }

    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }
    /// When `handle_timeout` should next be called.
    pub fn poll_timeout(&self) -> Option<u64> {
        self.timer
    }

    pub fn handle_timeout(&mut self, now_ms: u64) -> Result<(), Error> {
        match self.timer {
            Some(t) if now_ms >= t => {}
            _ => return Ok(()),
        }
        self.retries += 1;
        if self.retries > MAX_RETRANSMITS {
            self.timer = None;
            self.st = St::Failed;
            return Err(Error::Timeout);
        }
        self.rto = (self.rto * 2).min(self.cfg.max_rto_ms);
        self.timer = Some(now_ms + self.rto);
        self.transmit_flight();
        Ok(())
    }

    /// Send application data in one record.
    pub fn send(&mut self, data: &[u8]) -> Result<(), Error> {
        if self.st != St::Connected {
            return Err(Error::NotConnected);
        }
        if data.len() > 16384 {
            return Err(Error::TooLarge);
        }
        let r = self.record(CT_APP, 1, data);
        self.out.push_back(r);
        Ok(())
    }

    /// Send close_notify. Keys stay exportable.
    pub fn close(&mut self) {
        if !self.is_closed() {
            self.alert(1, alert::CLOSE_NOTIFY);
            self.st = St::Closed;
            self.timer = None;
        }
    }

    /// RFC 5705 exporter without context, after the handshake.
    pub fn export_keying_material(&self, label: &[u8], len: usize) -> Option<Vec<u8>> {
        if self.st != St::Connected && self.st != St::Closed {
            return None;
        }
        let master = self.master.as_ref()?;
        let mut seed = [0u8; 64];
        seed[..32].copy_from_slice(&self.client_random);
        seed[32..].copy_from_slice(&self.server_random);
        let mut out = alloc::vec![0u8; len];
        prf(master, label, &seed, &mut out);
        Some(out)
    }

    /// The SRTP master keys and salts (RFC 5764 4.2), local first.
    pub fn srtp_keys(&self) -> Option<SrtpKeys> {
        let p = self.profile?;
        let (k, s) = (p.key_len(), p.salt_len());
        let m = self.export_keying_material(b"EXTRACTOR-dtls_srtp", 2 * (k + s))?;
        let (ck, sk, cs, ss) = (&m[..k], &m[k..2 * k], &m[2 * k..2 * k + s], &m[2 * k + s..]);
        let (lk, ls, rk, rs) = match self.cfg.role {
            Role::Client => (ck, cs, sk, ss),
            Role::Server => (sk, ss, ck, cs),
        };
        Some(SrtpKeys {
            profile: p,
            local_key: lk.into(),
            local_salt: ls.into(),
            remote_key: rk.into(),
            remote_salt: rs.into(),
        })
    }

    // ---- records ----

    fn record(&mut self, ct: u8, epoch: u16, body: &[u8]) -> Vec<u8> {
        let e = epoch as usize;
        let seq = (epoch as u64) << 48 | self.write_rseq[e];
        self.write_rseq[e] += 1;
        let payload = if epoch == 0 { Vec::from(body) } else { self.write_cipher.as_ref().unwrap().seal(seq, ct, body) };
        let mut r = Vec::with_capacity(RECORD_HEADER + payload.len());
        r.push(ct);
        put16(&mut r, DTLS12);
        r.extend_from_slice(&seq.to_be_bytes());
        put16(&mut r, payload.len() as u16);
        r.extend_from_slice(&payload);
        r
    }

    fn alert(&mut self, level: u8, desc: u8) {
        let r = self.record(CT_ALERT, self.write_epoch, &[level, desc]);
        self.out.push_back(r);
    }

    fn fail(&mut self, e: Error) -> Error {
        let desc = match e {
            Error::Decode => alert::DECODE_ERROR,
            Error::Unexpected => alert::UNEXPECTED_MESSAGE,
            Error::Fingerprint => alert::BAD_CERTIFICATE,
            Error::UnsupportedCertificate => alert::UNSUPPORTED_CERTIFICATE,
            Error::Verify => alert::DECRYPT_ERROR,
            Error::NoSrtpProfile | Error::Negotiation => alert::HANDSHAKE_FAILURE,
            _ => 0,
        };
        if desc != 0 {
            self.alert(2, desc);
        }
        self.st = St::Failed;
        self.timer = None;
        e
    }

    /// Put the current flight on the wire: fragments packed into datagrams up to the MTU.
    fn transmit_flight(&mut self) {
        let mtu = self.cfg.mtu.max(256);
        let mut dgram: Vec<u8> = Vec::new();
        for m in self.flight.clone() {
            let recs: Vec<Vec<u8>> = match m {
                Out::Ccs => alloc::vec![self.record(CT_CCS, 0, &[1])],
                Out::Hs { epoch, ty, seq, body } => {
                    let room = mtu - RECORD_HEADER - HS_HEADER - if epoch > 0 { GCM_OVERHEAD } else { 0 };
                    let mut v = Vec::new();
                    let mut off = 0;
                    loop {
                        let n = (body.len() - off).min(room);
                        let mut f = Vec::with_capacity(HS_HEADER + n);
                        f.push(ty);
                        put24(&mut f, body.len());
                        put16(&mut f, seq);
                        put24(&mut f, off);
                        put24(&mut f, n);
                        f.extend_from_slice(&body[off..off + n]);
                        v.push(self.record(CT_HANDSHAKE, epoch, &f));
                        off += n;
                        if off >= body.len() {
                            break;
                        }
                    }
                    v
                }
            };
            for r in recs {
                if !dgram.is_empty() && dgram.len() + r.len() > mtu {
                    self.out.push_back(core::mem::take(&mut dgram));
                }
                dgram.extend_from_slice(&r);
            }
        }
        if !dgram.is_empty() {
            self.out.push_back(dgram);
        }
    }

    /// Start a new flight: it is sent now and again on the timer until answered.
    fn send_flight(&mut self, msgs: Vec<Out>, now_ms: u64, timed: bool) {
        self.flight = msgs;
        self.peer_flight_start = self.recv_seq;
        self.retries = 0;
        self.rto = self.cfg.initial_rto_ms;
        self.timer = timed.then(|| now_ms + self.rto);
        self.transmit_flight();
    }

    /// A handshake message of ours: numbered, and added to the transcript.
    fn hs(&mut self, ty: u8, body: Vec<u8>, epoch: u16) -> Out {
        let seq = self.send_seq;
        self.send_seq += 1;
        self.add_transcript(ty, seq, &body);
        Out::Hs { epoch, ty, seq, body }
    }

    fn add_transcript(&mut self, ty: u8, seq: u16, body: &[u8]) {
        self.transcript.push(ty);
        put24(&mut self.transcript, body.len());
        put16(&mut self.transcript, seq);
        put24(&mut self.transcript, 0);
        put24(&mut self.transcript, body.len());
        self.transcript.extend_from_slice(body);
    }

    fn transcript_hash(&self) -> [u8; 32] {
        Sha256::digest(&self.transcript).into()
    }

    fn finished(&self, label: &[u8]) -> [u8; 12] {
        let mut v = [0u8; 12];
        prf(self.master.as_ref().unwrap(), label, &self.transcript_hash(), &mut v);
        v
    }

    /// Feed one received datagram.
    pub fn handle_datagram(&mut self, now_ms: u64, data: &[u8]) -> Result<(), Error> {
        if matches!(self.st, St::Failed | St::Closed) {
            return Ok(());
        }
        self.resend = false;
        let mut d = data;
        while d.len() >= RECORD_HEADER {
            let len = u16::from_be_bytes([d[11], d[12]]) as usize;
            if d.len() < RECORD_HEADER + len {
                break;
            }
            let (rec, rest) = d.split_at(RECORD_HEADER + len);
            d = rest;
            self.handle_record(now_ms, rec)?;
        }
        if self.resend && !self.flight.is_empty() {
            self.transmit_flight();
        }
        Ok(())
    }

    fn handle_record(&mut self, now_ms: u64, rec: &[u8]) -> Result<(), Error> {
        let ct = rec[0];
        let ver = u16::from_be_bytes([rec[1], rec[2]]);
        if ver != DTLS12 && ver != DTLS10 {
            return Ok(());
        }
        let epoch = u16::from_be_bytes([rec[3], rec[4]]);
        let mut sb = [0u8; 8];
        sb[2..].copy_from_slice(&rec[5..11]);
        let rseq = u64::from_be_bytes(sb);
        let payload = &rec[RECORD_HEADER..];
        let body = match epoch {
            0 => {
                if ct == CT_APP {
                    return Ok(());
                }
                Vec::from(payload)
            }
            1 if self.read_epoch == 1 => {
                if !self.replay[1].check(rseq) {
                    return Ok(());
                }
                match self.read_cipher.as_ref().unwrap().open(1 << 48 | rseq, ct, payload) {
                    Some(b) => {
                        self.replay[1].accept(rseq);
                        b
                    }
                    None => return Ok(()),
                }
            }
            1 => {
                // Ahead of the peer's ChangeCipherSpec: keep a few for later.
                if self.future.len() < 16 {
                    self.future.push(Vec::from(rec));
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        match ct {
            CT_HANDSHAKE => self.handle_handshake(now_ms, epoch, &body),
            CT_CCS => {
                if body != [1] {
                    return Ok(());
                }
                if matches!(self.st, St::CCcs | St::SCcs) {
                    self.read_cipher = self.next_read_cipher.take();
                    self.read_epoch = 1;
                    self.st = if self.st == St::CCcs { St::CFinished } else { St::SFinished };
                    for r in core::mem::take(&mut self.future) {
                        self.handle_record(now_ms, &r)?;
                    }
                }
                Ok(())
            }
            CT_ALERT => {
                if body.len() < 2 {
                    return Ok(());
                }
                if body[1] == alert::CLOSE_NOTIFY {
                    if self.st != St::Closed {
                        self.alert(1, alert::CLOSE_NOTIFY);
                    }
                    self.st = St::Closed;
                    self.timer = None;
                    self.events.push_back(Event::Closed);
                    Ok(())
                } else if body[0] == 2 {
                    self.st = St::Failed;
                    self.timer = None;
                    Err(Error::Alert(body[1]))
                } else {
                    Ok(())
                }
            }
            CT_APP => {
                if self.st == St::Connected && epoch == 1 {
                    self.events.push_back(Event::Data(body));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn handle_handshake(&mut self, now_ms: u64, epoch: u16, body: &[u8]) -> Result<(), Error> {
        let mut r = Reader(body);
        while r.0.len() >= HS_HEADER {
            let ty = r.u8()?;
            let len = r.u24()?;
            let seq = r.u16()?;
            let off = r.u24()?;
            let flen = r.u24()?;
            let frag = match r.take(flen) {
                Ok(f) => f,
                Err(_) => return Ok(()),
            };
            if len > MAX_MESSAGE || off + flen > len {
                return Ok(());
            }
            if seq < self.recv_seq {
                // The peer sent an earlier flight again: ours was lost.
                if seq < self.peer_flight_start && !(self.st == St::Connected && self.cfg.role == Role::Client) {
                    self.resend = true;
                }
                continue;
            }
            if seq > self.recv_seq.saturating_add(16) {
                continue;
            }
            // A Finished must come under epoch 1, everything else under epoch 0.
            if (ty == HT_FINISHED) != (epoch == 1) {
                continue;
            }
            let p = self.partial.entry(seq).or_insert_with(|| Partial { ty, body: alloc::vec![0; len], have: Vec::new() });
            if p.ty != ty || p.body.len() != len {
                continue;
            }
            p.add(off, frag);
            while self.partial.get(&self.recv_seq).map_or(false, |p| p.done()) {
                let p = self.partial.remove(&self.recv_seq).unwrap();
                let s = self.recv_seq;
                self.recv_seq += 1;
                if let Err(e) = self.handle_message(now_ms, p.ty, s, &p.body) {
                    return Err(self.fail(e));
                }
                if matches!(self.st, St::Failed | St::Closed) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn handle_message(&mut self, now_ms: u64, ty: u8, seq: u16, body: &[u8]) -> Result<(), Error> {
        // Any new message from the peer answers our flight.
        self.timer = None;
        match (self.st, ty) {
            (St::CHello, HT_HELLO_VERIFY) => {
                let mut r = Reader(body);
                r.u16()?;
                self.cookie = r.v8()?.into();
                self.transcript.clear();
                self.send_client_hello(now_ms);
                Ok(())
            }
            (St::CHello, HT_SERVER_HELLO) => {
                self.add_transcript(ty, seq, body);
                self.on_server_hello(body)
            }
            (St::CCert, HT_CERTIFICATE) => {
                self.add_transcript(ty, seq, body);
                self.on_certificate(body)?;
                self.st = St::CKeyExchange;
                Ok(())
            }
            (St::CKeyExchange, HT_SERVER_KEY_EXCHANGE) => {
                self.add_transcript(ty, seq, body);
                self.on_server_key_exchange(body)
            }
            (St::CCertReqOrDone, HT_CERTIFICATE_REQUEST) => {
                self.add_transcript(ty, seq, body);
                let mut r = Reader(body);
                let types = r.v8()?;
                let algs = r.v16()?;
                if !types.contains(&64) || !algs.chunks(2).any(|c| c == ECDSA_SHA256.to_be_bytes()) {
                    return Err(Error::Negotiation);
                }
                self.cert_requested = true;
                self.st = St::CDone;
                Ok(())
            }
            (St::CCertReqOrDone | St::CDone, HT_SERVER_HELLO_DONE) => {
                self.add_transcript(ty, seq, body);
                self.send_client_flight(now_ms)
            }
            (St::CFinished, HT_FINISHED) => {
                let want = self.finished(b"server finished");
                if !bool::from(want.ct_eq(body)) {
                    return Err(Error::Verify);
                }
                self.add_transcript(ty, seq, body);
                self.flight.clear();
                self.st = St::Connected;
                self.events.push_back(Event::Connected);
                Ok(())
            }
            (St::SHello, HT_CLIENT_HELLO) => self.on_client_hello(now_ms, seq, body),
            (St::SCert, HT_CERTIFICATE) => {
                self.add_transcript(ty, seq, body);
                self.on_certificate(body)?;
                self.st = St::SKeyExchange;
                Ok(())
            }
            (St::SKeyExchange, HT_CLIENT_KEY_EXCHANGE) => {
                self.add_transcript(ty, seq, body);
                let mut r = Reader(body);
                self.peer_ecdh = Some(PublicKey::from_sec1_bytes(r.v8()?).map_err(|_| Error::Decode)?);
                self.derive_keys();
                self.st = St::SVerify;
                Ok(())
            }
            (St::SVerify, HT_CERTIFICATE_VERIFY) => {
                self.verify_signature(body, &self.transcript.clone())?;
                self.add_transcript(ty, seq, body);
                self.st = St::SCcs;
                Ok(())
            }
            (St::SFinished, HT_FINISHED) => {
                let want = self.finished(b"client finished");
                if !bool::from(want.ct_eq(body)) {
                    return Err(Error::Verify);
                }
                self.add_transcript(ty, seq, body);
                let fin = self.finished(b"server finished");
                self.write_epoch = 1;
                let f = self.hs(HT_FINISHED, fin.into(), 1);
                self.send_flight(alloc::vec![Out::Ccs, f], now_ms, false);
                self.st = St::Connected;
                self.events.push_back(Event::Connected);
                Ok(())
            }
            (St::Connected, _) => Ok(()),
            _ => Err(Error::Unexpected),
        }
    }

    fn send_client_hello(&mut self, now_ms: u64) {
        let mut b = Vec::new();
        put16(&mut b, DTLS12);
        b.extend_from_slice(&self.client_random);
        b.push(0);
        b.push(self.cookie.len() as u8);
        b.extend_from_slice(&self.cookie.clone());
        put16(&mut b, 2);
        put16(&mut b, TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256);
        b.extend_from_slice(&[1, 0]);
        let mut e = Vec::new();
        ext(&mut e, EXT_EMS, &[]);
        ext(&mut e, EXT_RENEGOTIATION_INFO, &[0]);
        ext(&mut e, EXT_SUPPORTED_GROUPS, &[0, 2, 0, SECP256R1 as u8]);
        ext(&mut e, EXT_POINT_FORMATS, &[1, 0]);
        ext(&mut e, EXT_SIGNATURE_ALGORITHMS, &[0, 2, 4, 3]);
        if !self.cfg.srtp_profiles.is_empty() {
            ext(&mut e, EXT_USE_SRTP, &use_srtp_body(&self.cfg.srtp_profiles));
        }
        put16(&mut b, e.len() as u16);
        b.extend_from_slice(&e);
        let m = self.hs(HT_CLIENT_HELLO, b, 0);
        self.st = St::CHello;
        self.send_flight(alloc::vec![m], now_ms, true);
    }

    fn on_server_hello(&mut self, body: &[u8]) -> Result<(), Error> {
        let mut r = Reader(body);
        if r.u16()? != DTLS12 {
            return Err(Error::Negotiation);
        }
        self.server_random.copy_from_slice(r.take(32)?);
        r.v8()?;
        if r.u16()? != TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 || r.u8()? != 0 {
            return Err(Error::Negotiation);
        }
        let exts = if r.0.is_empty() { Vec::new() } else { parse_exts(r.v16()?)? };
        if find(&exts, EXT_EMS).is_none() {
            return Err(Error::Negotiation);
        }
        if !self.cfg.srtp_profiles.is_empty() {
            let b = find(&exts, EXT_USE_SRTP).ok_or(Error::NoSrtpProfile)?;
            let ids = u16_list(b)?;
            let p = match ids.as_slice() {
                [id] => SrtpProfile::from_id(*id).filter(|p| self.cfg.srtp_profiles.contains(p)),
                _ => None,
            };
            self.profile = Some(p.ok_or(Error::NoSrtpProfile)?);
        }
        self.st = St::CCert;
        Ok(())
    }

    fn on_certificate(&mut self, body: &[u8]) -> Result<(), Error> {
        let mut r = Reader(body);
        let mut list = Reader(r.v24()?);
        let der = list.v24().map_err(|_| Error::Fingerprint)?;
        if !Fingerprint::of(der).ct_eq(&self.cfg.remote_fingerprint) {
            return Err(Error::Fingerprint);
        }
        self.peer_key = Some(cert::public_key(der).ok_or(Error::UnsupportedCertificate)?);
        Ok(())
    }

    fn verify_signature(&self, body: &[u8], signed: &[u8]) -> Result<(), Error> {
        let mut r = Reader(body);
        if r.u16()? != ECDSA_SHA256 {
            return Err(Error::Negotiation);
        }
        let sig = Signature::from_der(r.v16()?).map_err(|_| Error::Verify)?;
        self.peer_key.as_ref().ok_or(Error::Unexpected)?.verify(signed, &sig).map_err(|_| Error::Verify)
    }

    fn ecdh_params(point: &[u8]) -> Vec<u8> {
        let mut p = Vec::from([3u8, 0, SECP256R1 as u8, point.len() as u8]);
        p.extend_from_slice(point);
        p
    }

    fn on_server_key_exchange(&mut self, body: &[u8]) -> Result<(), Error> {
        let mut r = Reader(body);
        if r.u8()? != 3 || r.u16()? != SECP256R1 {
            return Err(Error::Negotiation);
        }
        let point = r.v8()?;
        let params = Self::ecdh_params(point);
        let mut signed = Vec::with_capacity(64 + params.len());
        signed.extend_from_slice(&self.client_random);
        signed.extend_from_slice(&self.server_random);
        signed.extend_from_slice(&params);
        self.verify_signature(r.rest(), &signed)?;
        self.peer_ecdh = Some(PublicKey::from_sec1_bytes(point).map_err(|_| Error::Decode)?);
        self.st = St::CCertReqOrDone;
        Ok(())
    }

    fn certificate_body(&self) -> Vec<u8> {
        let der = &self.cfg.certificate.der;
        let mut b = Vec::new();
        put24(&mut b, der.len() + 3);
        put24(&mut b, der.len());
        b.extend_from_slice(der);
        b
    }

    fn sign(&self, data: &[u8]) -> Vec<u8> {
        let sig: Signature = self.cfg.certificate.key.sign(data);
        let der = sig.to_der();
        let mut b = Vec::new();
        put16(&mut b, ECDSA_SHA256);
        put16(&mut b, der.as_bytes().len() as u16);
        b.extend_from_slice(der.as_bytes());
        b
    }

    fn our_point(&mut self) -> Vec<u8> {
        let s = EphemeralSecret::random(&mut self.rng);
        let p = Vec::from(s.public_key().to_encoded_point(false).as_bytes());
        self.ecdh = Some(s);
        p
    }

    /// Master secret (extended, RFC 7627: over the transcript through ClientKeyExchange)
    /// and the epoch 1 record keys.
    fn derive_keys(&mut self) {
        let shared = self.ecdh.take().unwrap().diffie_hellman(self.peer_ecdh.as_ref().unwrap());
        let mut master = [0u8; 48];
        prf(shared.raw_secret_bytes(), b"extended master secret", &self.transcript_hash(), &mut master);
        let mut seed = [0u8; 64];
        seed[..32].copy_from_slice(&self.server_random);
        seed[32..].copy_from_slice(&self.client_random);
        let mut kb = [0u8; 40];
        prf(&master, b"key expansion", &seed, &mut kb);
        let client = Cipher::new(&kb[..16], &kb[32..36]);
        let server = Cipher::new(&kb[16..32], &kb[36..40]);
        let (w, r) = match self.cfg.role {
            Role::Client => (client, server),
            Role::Server => (server, client),
        };
        self.write_cipher = Some(w);
        self.next_read_cipher = Some(r);
        self.master = Some(master);
    }

    fn send_client_flight(&mut self, now_ms: u64) -> Result<(), Error> {
        let mut msgs = Vec::new();
        if self.cert_requested {
            let c = self.certificate_body();
            msgs.push(self.hs(HT_CERTIFICATE, c, 0));
        }
        let point = self.our_point();
        let mut cke = Vec::from([point.len() as u8]);
        cke.extend_from_slice(&point);
        msgs.push(self.hs(HT_CLIENT_KEY_EXCHANGE, cke, 0));
        self.derive_keys();
        if self.cert_requested {
            let sig = self.sign(&self.transcript.clone());
            msgs.push(self.hs(HT_CERTIFICATE_VERIFY, sig, 0));
        }
        msgs.push(Out::Ccs);
        self.write_epoch = 1;
        let fin = self.finished(b"client finished");
        msgs.push(self.hs(HT_FINISHED, fin.into(), 1));
        self.st = St::CCcs;
        self.send_flight(msgs, now_ms, true);
        Ok(())
    }

    fn cookie_for(&self, random: &[u8]) -> Vec<u8> {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.cookie_secret).unwrap();
        m.update(random);
        Vec::from(&m.finalize().into_bytes()[..16])
    }

    fn on_client_hello(&mut self, now_ms: u64, seq: u16, body: &[u8]) -> Result<(), Error> {
        let mut r = Reader(body);
        r.u16()?;
        let random = r.take(32)?;
        r.v8()?;
        let cookie = r.v8()?;
        let suites = r.v16()?;
        r.v8()?;
        let exts = if r.0.is_empty() { Vec::new() } else { parse_exts(r.v16()?)? };
        if self.cfg.hello_verify {
            let want = self.cookie_for(random);
            if !bool::from(cookie.ct_eq(&want)) {
                // Stateless: no transcript, first message of our side is number 0.
                let mut b = Vec::new();
                put16(&mut b, DTLS12);
                b.push(want.len() as u8);
                b.extend_from_slice(&want);
                self.send_seq = 0;
                let m = Out::Hs { epoch: 0, ty: HT_HELLO_VERIFY, seq: 0, body: b };
                self.send_flight(alloc::vec![m], now_ms, false);
                return Ok(());
            }
        }
        self.client_random.copy_from_slice(random);
        self.send_seq = seq;
        self.add_transcript(HT_CLIENT_HELLO, seq, body);
        if !suites.chunks(2).any(|c| c == TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256.to_be_bytes()) {
            return Err(Error::Negotiation);
        }
        if find(&exts, EXT_EMS).is_none() {
            return Err(Error::Negotiation);
        }
        if let Some(g) = find(&exts, EXT_SUPPORTED_GROUPS) {
            if !u16_list(g)?.contains(&SECP256R1) {
                return Err(Error::Negotiation);
            }
        }
        if let Some(s) = find(&exts, EXT_SIGNATURE_ALGORITHMS) {
            if !u16_list(s)?.contains(&ECDSA_SHA256) {
                return Err(Error::Negotiation);
            }
        }
        self.peer_reneg = find(&exts, EXT_RENEGOTIATION_INFO).is_some() || suites.chunks(2).any(|c| c == [0, 0xff]);
        self.peer_point_formats = find(&exts, EXT_POINT_FORMATS).is_some();
        if !self.cfg.srtp_profiles.is_empty() {
            let offered = match find(&exts, EXT_USE_SRTP) {
                Some(b) => u16_list(b)?,
                None => return Err(Error::NoSrtpProfile),
            };
            let p = self.cfg.srtp_profiles.iter().copied().find(|p| offered.contains(&p.id()));
            self.profile = Some(p.ok_or(Error::NoSrtpProfile)?);
        }
        self.send_server_flight(now_ms);
        Ok(())
    }

    fn send_server_flight(&mut self, now_ms: u64) {
        self.rng.fill_bytes(&mut self.server_random);
        let mut sh = Vec::new();
        put16(&mut sh, DTLS12);
        sh.extend_from_slice(&self.server_random);
        sh.push(0);
        put16(&mut sh, TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256);
        sh.push(0);
        let mut e = Vec::new();
        ext(&mut e, EXT_EMS, &[]);
        if self.peer_reneg {
            ext(&mut e, EXT_RENEGOTIATION_INFO, &[0]);
        }
        if self.peer_point_formats {
            ext(&mut e, EXT_POINT_FORMATS, &[1, 0]);
        }
        if let Some(p) = self.profile {
            ext(&mut e, EXT_USE_SRTP, &use_srtp_body(&[p]));
        }
        put16(&mut sh, e.len() as u16);
        sh.extend_from_slice(&e);
        let mut msgs = alloc::vec![self.hs(HT_SERVER_HELLO, sh, 0)];
        let c = self.certificate_body();
        msgs.push(self.hs(HT_CERTIFICATE, c, 0));
        let point = self.our_point();
        let params = Self::ecdh_params(&point);
        let mut signed = Vec::new();
        signed.extend_from_slice(&self.client_random);
        signed.extend_from_slice(&self.server_random);
        signed.extend_from_slice(&params);
        let mut ske = params;
        ske.extend_from_slice(&self.sign(&signed));
        msgs.push(self.hs(HT_SERVER_KEY_EXCHANGE, ske, 0));
        msgs.push(self.hs(HT_CERTIFICATE_REQUEST, Vec::from([1, 64, 0, 2, 4, 3, 0, 0]), 0));
        msgs.push(self.hs(HT_SERVER_HELLO_DONE, Vec::new(), 0));
        self.st = St::SCert;
        self.send_flight(msgs, now_ms, true);
    }
}
