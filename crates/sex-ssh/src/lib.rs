// SPDX-License-Identifier: GPL-3.0-only
//! An SSH client (RFC 4253, 4252, 4254) without I/O, for SexOS's `ssh` program.
//! Key exchange curve25519-sha256 (RFC 8731) with strict KEX, AES-GCM packets
//! (RFC 5647, OpenSSH's names), host keys ssh-ed25519, ecdsa-sha2-nistp256 and
//! rsa-sha2-256/512, user keys ssh-ed25519 (OpenSSH private key files) or a password,
//! and session channels with exec, a PTY and a shell. The caller moves bytes
//! (`output`, `receive`), supplies randomness, and decides whether to trust a host key
//! (`Event::HostKey`, then `accept_host_key`).
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod wire;
pub mod keys;

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes128Gcm, Aes256Gcm};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};
use wire::{Reader, Writer};

pub const VERSION: &[u8] = b"SSH-2.0-SexOS_1.0";
const KEX: &[&str] = &["curve25519-sha256", "curve25519-sha256@libssh.org"];
const HOST_KEYS: &[&str] = &["ssh-ed25519", "ecdsa-sha2-nistp256", "rsa-sha2-512", "rsa-sha2-256"];
const CIPHERS: &[&str] = &["aes256-gcm@openssh.com", "aes128-gcm@openssh.com"];
const WINDOW: u32 = 2 << 20;
const MAX_PACKET: u32 = 32768;
const MAX_INCOMING: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The server speaks something else, or broke the protocol.
    Protocol,
    /// No algorithm in common.
    NoAlgorithm,
    /// The host key's signature over the exchange is wrong.
    HostKeySignature,
    /// A packet failed authentication (corrupted or tampered with).
    Integrity,
    /// The server closed the conversation (code from DISCONNECT).
    Disconnected(u32),
    /// Every way of logging in we have was refused.
    AuthFailed,
    /// The host key was not accepted.
    HostKeyRejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The server's host key: check it (known hosts) and call `accept_host_key` or `reject_host_key`.
    HostKey { algorithm: String, blob: Vec<u8>, fingerprint: String },
    /// A banner the server shows before login.
    Banner(String),
    /// Logged in; a session channel is being opened.
    Authenticated,
    /// The session is ready for `exec` or `shell` (already sent if asked in `Client::new`).
    SessionOpen,
    /// Output of the command or shell (stderr when `stderr`).
    Data { bytes: Vec<u8>, stderr: bool },
    ExitStatus(u32),
    /// The session ended; the connection can be dropped.
    Closed,
}

pub enum Auth { Ed25519(keys::Ed25519), Password(String) }

/// What to run once the session is open.
#[derive(Clone, Debug)]
pub enum Run { Exec(String), Shell { term: String, cols: u32, rows: u32 } }

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Stage { Version, Kex, HostKey, Auth, Open, Session, Done }

struct Keys { cipher: Cipher, iv: [u8; 12] }
enum Cipher { A128(Aes128Gcm), A256(Aes256Gcm) }
impl Keys {
    fn seal(&mut self, len: &[u8; 4], body: &mut Vec<u8>) {
        let tag = match &self.cipher { Cipher::A128(c) => c.encrypt_in_place_detached((&self.iv).into(), len, body), Cipher::A256(c) => c.encrypt_in_place_detached((&self.iv).into(), len, body) }.expect("size");
        body.extend_from_slice(&tag);
        self.bump();
    }
    fn open(&mut self, len: &[u8], body: &mut [u8], tag: &[u8]) -> Result<(), Error> {
        let r = match &self.cipher { Cipher::A128(c) => c.decrypt_in_place_detached((&self.iv).into(), len, body, tag.into()), Cipher::A256(c) => c.decrypt_in_place_detached((&self.iv).into(), len, body, tag.into()) };
        r.map_err(|_| Error::Integrity)?;
        self.bump();
        Ok(())
    }
    /// RFC 5647: the last 8 bytes of the nonce count packets.
    fn bump(&mut self) { let c = u64::from_be_bytes(self.iv[4..].try_into().unwrap()).wrapping_add(1); self.iv[4..].copy_from_slice(&c.to_be_bytes()); }
}

struct Kex { ours: Vec<u8>, theirs: Vec<u8>, secret: [u8; 32], cipher: &'static str, strict: bool, host_alg: &'static str }

pub struct Client {
    stage: Stage,
    out: Vec<u8>,
    input: Vec<u8>,
    events: VecDeque<Event>,
    server_version: Vec<u8>,
    send_keys: Option<Keys>,
    recv_keys: Option<Keys>,
    pending_send: Option<Keys>,
    pending_recv: Option<Keys>,
    session_id: Option<[u8; 32]>,
    kex: Option<Kex>,
    strict: bool,
    first_kex: bool,
    user: String,
    auth: Vec<Auth>,
    run: Option<Run>,
    rng: alloc::boxed::Box<dyn FnMut(&mut [u8])>,
    /// Channel numbers (ours is 0), window we give, window we may use, their max packet.
    remote_channel: u32,
    our_window: u32,
    their_window: u32,
    their_max: u32,
    held: Vec<u8>,
    eof_sent: bool,
    host_key_ok: Option<bool>,
}

fn hash(parts: &[&[u8]]) -> [u8; 32] { let mut h = Sha256::new(); for p in parts { h.update(p); } h.finalize().into() }
fn pick(ours: &[&'static str], theirs: &str) -> Option<&'static str> { ours.iter().copied().find(|a| theirs.split(',').any(|t| t == *a)) }

impl Client {
    /// A connection for `user`, trying `auth` methods in order, then running `run`.
    /// `rng` fills buffers with random bytes.
    pub fn new(user: &str, auth: Vec<Auth>, run: Option<Run>, rng: alloc::boxed::Box<dyn FnMut(&mut [u8])>) -> Self {
        let mut c = Self {
            stage: Stage::Version, out: Vec::new(), input: Vec::new(), events: VecDeque::new(), server_version: Vec::new(),
            send_keys: None, recv_keys: None, pending_send: None, pending_recv: None, session_id: None, kex: None, strict: false, first_kex: true,
            user: user.into(), auth, run, rng, remote_channel: 0, our_window: WINDOW, their_window: 0, their_max: 0, held: Vec::new(), eof_sent: false, host_key_ok: None,
        };
        c.out.extend_from_slice(VERSION);
        c.out.extend_from_slice(b"\r\n");
        c.send_kexinit();
        c
    }
    pub fn output(&mut self) -> Vec<u8> { core::mem::take(&mut self.out) }
    pub fn event(&mut self) -> Option<Event> { self.events.pop_front() }
    pub fn server_version(&self) -> &[u8] { &self.server_version }

    fn send_kexinit(&mut self) {
        let mut cookie = [0u8; 16];
        (self.rng)(&mut cookie);
        let mut w = Writer::new();
        w.byte(20).raw(&cookie);
        let mut kex = KEX.join(",");
        if self.first_kex { kex.push_str(",ext-info-c,kex-strict-c-v00@openssh.com"); }
        w.string(kex.as_bytes()).string(HOST_KEYS.join(",").as_bytes()).string(CIPHERS.join(",").as_bytes()).string(CIPHERS.join(",").as_bytes())
            .string(b"hmac-sha2-256").string(b"hmac-sha2-256").string(b"none").string(b"none").string(b"").string(b"").byte(0).u32(0);
        let payload = w.finish();
        self.kex = Some(Kex { ours: payload.clone(), theirs: Vec::new(), secret: [0; 32], cipher: "", strict: false, host_alg: "" });
        self.packet(&payload);
    }

    /// Frames one payload: padded and, once keys are set, sealed with AES-GCM.
    fn packet(&mut self, payload: &[u8]) {
        let block = if self.send_keys.is_some() { 16 } else { 8 };
        let unpadded = if self.send_keys.is_some() { 1 + payload.len() } else { 5 + payload.len() };
        let mut pad = block - unpadded % block;
        if pad < 4 { pad += block; }
        let len = (1 + payload.len() + pad) as u32;
        let mut body = Vec::with_capacity(len as usize + 16);
        body.push(pad as u8);
        body.extend_from_slice(payload);
        let mut padding = alloc::vec![0u8; pad];
        (self.rng)(&mut padding);
        body.extend_from_slice(&padding);
        let lb = len.to_be_bytes();
        self.out.extend_from_slice(&lb);
        if let Some(k) = self.send_keys.as_mut() { k.seal(&lb, &mut body); }
        self.out.extend_from_slice(&body);
    }

    /// Takes bytes from the server.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.input.extend_from_slice(bytes);
        if self.stage == Stage::Version {
            // Lines before the version line are allowed (RFC 4253 section 4.2).
            loop {
                let Some(end) = self.input.windows(2).position(|w| w == b"\r\n").or_else(|| self.input.iter().position(|b| *b == b'\n')) else {
                    if self.input.len() > 8192 { return Err(Error::Protocol); }
                    return Ok(());
                };
                let line: Vec<u8> = self.input[..end].to_vec();
                let skip = if self.input.get(end) == Some(&b'\r') { end + 2 } else { end + 1 };
                self.input.drain(..skip);
                if line.starts_with(b"SSH-") {
                    if !(line.starts_with(b"SSH-2.0-") || line.starts_with(b"SSH-1.99-")) { return Err(Error::Protocol); }
                    self.server_version = line;
                    self.stage = Stage::Kex;
                    break;
                }
            }
        }
        while let Some(payload) = self.next_packet()? {
            self.handle(&payload)?;
            if self.stage == Stage::HostKey && self.host_key_ok.is_none() { break; }
        }
        Ok(())
    }

    fn next_packet(&mut self) -> Result<Option<Vec<u8>>, Error> {
        if self.stage == Stage::HostKey && self.host_key_ok.is_none() { return Ok(None); }
        if self.input.len() < 4 { return Ok(None); }
        let len = u32::from_be_bytes(self.input[..4].try_into().unwrap()) as usize;
        if !(5..=MAX_INCOMING).contains(&len) { return Err(Error::Protocol); }
        let tag = if self.recv_keys.is_some() { 16 } else { 0 };
        if self.input.len() < 4 + len + tag { return Ok(None); }
        let mut body = self.input[4..4 + len].to_vec();
        if let Some(k) = self.recv_keys.as_mut() {
            let (lb, t) = (self.input[..4].to_vec(), self.input[4 + len..4 + len + 16].to_vec());
            k.open(&lb, &mut body, &t)?;
        } else if (4 + len) % 8 != 0 { return Err(Error::Protocol); }
        self.input.drain(..4 + len + tag);
        let pad = body[0] as usize;
        if pad < 4 || pad + 1 > body.len() { return Err(Error::Protocol); }
        Ok(Some(body[1..body.len() - pad].to_vec()))
    }

    fn handle(&mut self, p: &[u8]) -> Result<(), Error> {
        let kind = *p.first().ok_or(Error::Protocol)?;
        // Strict KEX (Terrapin): nothing but KEX messages during the first exchange.
        if self.strict && self.session_id.is_none() && !matches!(kind, 20 | 21 | 31 | 1) { return Err(Error::Protocol); }
        let mut r = Reader::new(&p[1..]);
        match kind {
            1 => { let code = r.u32()?; return Err(Error::Disconnected(code)); }
            2 | 4 => {}
            3 => {}
            7 => {} // EXT_INFO: nothing we need (our user key is ed25519).
            20 => self.server_kexinit(p)?,
            31 => self.kex_reply(&mut r)?,
            21 => {
                self.recv_keys = self.pending_recv.take();
                if self.recv_keys.is_none() { return Err(Error::Protocol); }
                if self.stage == Stage::HostKey {
                    self.stage = Stage::Auth;
                    let mut w = Writer::new();
                    w.byte(5).string(b"ssh-userauth");
                    let m = w.finish();
                    self.packet(&m);
                }
            }
            6 => self.try_auth()?,
            51 => self.try_auth()?,
            52 => { self.stage = Stage::Open; self.events.push_back(Event::Authenticated); self.open_session(); }
            53 => { let msg = r.string()?; self.events.push_back(Event::Banner(String::from_utf8_lossy(msg).into())); }
            80 => {
                let _name = r.string()?;
                if r.bool()? { self.packet(&[82]); }
            }
            91 => {
                let _ours = r.u32()?;
                self.remote_channel = r.u32()?;
                self.their_window = r.u32()?;
                self.their_max = r.u32()?.max(1024);
                self.stage = Stage::Session;
                self.events.push_back(Event::SessionOpen);
                if let Some(run) = self.run.take() { self.start(run); }
            }
            92 => return Err(Error::Protocol),
            93 => { let _ = r.u32()?; self.their_window = self.their_window.saturating_add(r.u32()?); self.flush_held(); }
            94 | 95 => {
                let _ = r.u32()?;
                let stderr = kind == 95 && r.u32()? == 1;
                let data = r.string()?.to_vec();
                self.our_window = self.our_window.saturating_sub(data.len() as u32);
                if self.our_window < WINDOW / 2 {
                    let mut w = Writer::new();
                    w.byte(93).u32(self.remote_channel).u32(WINDOW - self.our_window);
                    let m = w.finish();
                    self.packet(&m);
                    self.our_window = WINDOW;
                }
                self.events.push_back(Event::Data { bytes: data, stderr });
            }
            96 => {}
            97 => {
                if self.stage != Stage::Done {
                    let mut w = Writer::new();
                    w.byte(97).u32(self.remote_channel);
                    let m = w.finish();
                    self.packet(&m);
                }
                self.stage = Stage::Done;
                self.events.push_back(Event::Closed);
            }
            98 => {
                let _ = r.u32()?;
                let name = r.string()?.to_vec();
                let want = r.bool()?;
                if name == b"exit-status" { let s = r.u32()?; self.events.push_back(Event::ExitStatus(s)); }
                if want { let mut w = Writer::new(); w.byte(100).u32(self.remote_channel); let m = w.finish(); self.packet(&m); }
            }
            99 | 100 | 81 | 82 => {}
            _ => {
                let mut w = Writer::new();
                w.byte(3).u32(0);
                let m = w.finish();
                self.packet(&m);
            }
        }
        Ok(())
    }

    fn server_kexinit(&mut self, p: &[u8]) -> Result<(), Error> {
        // A KEXINIT after login is the server renewing keys: answer with ours.
        if self.kex.is_none() { self.send_kexinit(); }
        let mut r = Reader::new(&p[17..]);
        let kex = core::str::from_utf8(r.string()?).map_err(|_| Error::Protocol)?;
        let hosts = core::str::from_utf8(r.string()?).map_err(|_| Error::Protocol)?;
        let c2s = core::str::from_utf8(r.string()?).map_err(|_| Error::Protocol)?;
        let s2c = core::str::from_utf8(r.string()?).map_err(|_| Error::Protocol)?;
        let _ = (r.string()?, r.string()?);
        let comp = (r.string()?, r.string()?);
        if !comp.0.split(|b| *b == b',').any(|a| a == b"none") || !comp.1.split(|b| *b == b',').any(|a| a == b"none") { return Err(Error::NoAlgorithm); }
        pick(KEX, kex).ok_or(Error::NoAlgorithm)?;
        let host_alg = pick(HOST_KEYS, hosts).ok_or(Error::NoAlgorithm)?;
        let cipher = pick(CIPHERS, c2s).filter(|c| s2c.split(',').any(|s| s == *c)).ok_or(Error::NoAlgorithm)?;
        if self.first_kex && kex.split(',').any(|k| k == "kex-strict-s-v00@openssh.com") { self.strict = true; }
        let mut secret = [0u8; 32];
        (self.rng)(&mut secret);
        let k = self.kex.as_mut().ok_or(Error::Protocol)?;
        k.theirs = p.to_vec();
        k.secret = secret;
        k.cipher = cipher;
        k.host_alg = host_alg;
        k.strict = self.strict;
        let public = ed25519_compact::x25519::SecretKey::new(secret).recover_public_key().map_err(|_| Error::Protocol)?;
        let mut w = Writer::new();
        w.byte(30).string(&public[..]);
        let m = w.finish();
        self.packet(&m);
        Ok(())
    }

    fn kex_reply(&mut self, r: &mut Reader) -> Result<(), Error> {
        let host = r.string()?.to_vec();
        let q_s = r.string()?.to_vec();
        let sig = r.string()?.to_vec();
        let k = self.kex.take().ok_or(Error::Protocol)?;
        let ours = ed25519_compact::x25519::SecretKey::new(k.secret);
        let q_c = ours.recover_public_key().map_err(|_| Error::Protocol)?;
        let shared = ed25519_compact::x25519::PublicKey::from_slice(&q_s).and_then(|p| p.dh(&ours)).map_err(|_| Error::Protocol)?;
        let mut kw = Writer::new();
        kw.mpint(&shared[..]);
        let kbytes = kw.finish();
        let mut hw = Writer::new();
        hw.string(VERSION).string(&self.server_version).string(&k.ours).string(&k.theirs).string(&host).string(&q_c[..]).string(&q_s).raw(&kbytes);
        let h = hash(&[&hw.finish()]);
        keys::verify_host(k.host_alg, &host, &sig, &h)?;
        let session_id = *self.session_id.get_or_insert(h);
        let derive = |x: u8, n: usize| -> Vec<u8> {
            let mut v = hash(&[&kbytes, &h, &[x], &session_id]).to_vec();
            while v.len() < n { let more = hash(&[&kbytes, &h, &v]); v.extend_from_slice(&more); }
            v.truncate(n);
            v
        };
        let n = if k.cipher.starts_with("aes256") { 32 } else { 16 };
        let make = |iv: Vec<u8>, key: Vec<u8>| Keys {
            cipher: if n == 32 { Cipher::A256(Aes256Gcm::new_from_slice(&key).unwrap()) } else { Cipher::A128(Aes128Gcm::new_from_slice(&key).unwrap()) },
            iv: iv.try_into().unwrap(),
        };
        self.pending_send = Some(make(derive(b'A', 12), derive(b'C', n)));
        self.pending_recv = Some(make(derive(b'B', 12), derive(b'D', n)));
        self.packet(&[21]);
        self.send_keys = self.pending_send.take();
        if self.first_kex {
            self.first_kex = false;
            self.stage = Stage::HostKey;
            self.events.push_back(Event::HostKey { algorithm: k.host_alg.into(), fingerprint: keys::fingerprint(&host), blob: host });
        }
        Ok(())
    }

    /// The host key is known and trusted (or the person accepted it): carry on.
    pub fn accept_host_key(&mut self) -> Result<(), Error> {
        self.host_key_ok = Some(true);
        self.receive(&[])
    }
    pub fn reject_host_key(&mut self) -> Error {
        self.host_key_ok = Some(false);
        let mut w = Writer::new();
        w.byte(1).u32(9).string(b"host key not accepted").string(b"");
        let m = w.finish();
        self.packet(&m);
        self.stage = Stage::Done;
        Error::HostKeyRejected
    }

    fn try_auth(&mut self) -> Result<(), Error> {
        let Some(sid) = self.session_id else { return Err(Error::Protocol) };
        if self.auth.is_empty() { return Err(Error::AuthFailed); }
        let method = self.auth.remove(0);
        let mut w = Writer::new();
        w.byte(50).string(self.user.as_bytes()).string(b"ssh-connection");
        match method {
            Auth::Password(p) => { w.string(b"password").byte(0).string(p.as_bytes()); }
            Auth::Ed25519(key) => {
                let blob = key.public_blob();
                w.string(b"publickey").byte(1).string(b"ssh-ed25519").string(&blob);
                let mut signed = Writer::new();
                signed.string(&sid).raw(&w.clone_bytes());
                let sig = key.sign(&signed.finish());
                w.string(&sig);
            }
        }
        let m = w.finish();
        self.packet(&m);
        Ok(())
    }

    fn open_session(&mut self) {
        let mut w = Writer::new();
        w.byte(90).string(b"session").u32(0).u32(WINDOW).u32(MAX_PACKET);
        let m = w.finish();
        self.packet(&m);
    }

    fn start(&mut self, run: Run) {
        let mut w = Writer::new();
        match run {
            Run::Exec(cmd) => { w.byte(98).u32(self.remote_channel).string(b"exec").byte(1).string(cmd.as_bytes()); }
            Run::Shell { term, cols, rows } => {
                let mut pty = Writer::new();
                pty.byte(98).u32(self.remote_channel).string(b"pty-req").byte(0).string(term.as_bytes()).u32(cols).u32(rows).u32(0).u32(0).string(&[0]);
                let m = pty.finish();
                self.packet(&m);
                w.byte(98).u32(self.remote_channel).string(b"shell").byte(0);
            }
        }
        let m = w.finish();
        self.packet(&m);
    }

    /// Runs a command or shell on the open session (when not given to `new`).
    pub fn run(&mut self, run: Run) { if self.stage == Stage::Session { self.start(run); } else { self.run = Some(run); } }

    /// Input for the command or shell (keystrokes). Held while the server's window is full.
    pub fn send(&mut self, bytes: &[u8]) { self.held.extend_from_slice(bytes); self.flush_held(); }

    fn flush_held(&mut self) {
        while self.stage == Stage::Session && !self.held.is_empty() && self.their_window > 0 {
            let n = self.held.len().min(self.their_window as usize).min(self.their_max as usize - 64);
            let chunk: Vec<u8> = self.held.drain(..n).collect();
            self.their_window -= n as u32;
            let mut w = Writer::new();
            w.byte(94).u32(self.remote_channel).string(&chunk);
            let m = w.finish();
            self.packet(&m);
        }
    }

    /// The terminal changed size.
    pub fn resize(&mut self, cols: u32, rows: u32) {
        let mut w = Writer::new();
        w.byte(98).u32(self.remote_channel).string(b"window-change").byte(0).u32(cols).u32(rows).u32(0).u32(0);
        let m = w.finish();
        self.packet(&m);
    }
    /// No more input (like Ctrl-D on a pipe).
    pub fn eof(&mut self) {
        if !self.eof_sent && self.stage == Stage::Session { self.eof_sent = true; let mut w = Writer::new(); w.byte(96).u32(self.remote_channel); let m = w.finish(); self.packet(&m); }
    }
    pub fn closed(&self) -> bool { self.stage == Stage::Done }
}

#[cfg(test)]
mod tests;
