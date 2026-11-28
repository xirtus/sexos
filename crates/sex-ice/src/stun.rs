// SPDX-License-Identifier: GPL-3.0-only
//! STUN messages (RFC 8489) with the attributes ICE (RFC 8445) and TURN (RFC 8656) use.
//! A message is decoded once; its integrity and fingerprint are checked on the raw bytes.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const MAGIC: u32 = 0x2112_A442;
pub const HEADER: usize = 20;

pub mod method {
    pub const BINDING: u16 = 0x001;
    pub const ALLOCATE: u16 = 0x003;
    pub const REFRESH: u16 = 0x004;
    pub const SEND: u16 = 0x006;
    pub const DATA: u16 = 0x007;
    pub const CREATE_PERMISSION: u16 = 0x008;
    pub const CHANNEL_BIND: u16 = 0x009;
}

pub mod attr {
    pub const MAPPED_ADDRESS: u16 = 0x0001;
    pub const USERNAME: u16 = 0x0006;
    pub const MESSAGE_INTEGRITY: u16 = 0x0008;
    pub const ERROR_CODE: u16 = 0x0009;
    pub const UNKNOWN_ATTRIBUTES: u16 = 0x000A;
    pub const CHANNEL_NUMBER: u16 = 0x000C;
    pub const LIFETIME: u16 = 0x000D;
    pub const XOR_PEER_ADDRESS: u16 = 0x0012;
    pub const DATA: u16 = 0x0013;
    pub const REALM: u16 = 0x0014;
    pub const NONCE: u16 = 0x0015;
    pub const XOR_RELAYED_ADDRESS: u16 = 0x0016;
    pub const REQUESTED_ADDRESS_FAMILY: u16 = 0x0017;
    pub const REQUESTED_TRANSPORT: u16 = 0x0019;
    pub const DONT_FRAGMENT: u16 = 0x001A;
    pub const XOR_MAPPED_ADDRESS: u16 = 0x0020;
    pub const PRIORITY: u16 = 0x0024;
    pub const USE_CANDIDATE: u16 = 0x0025;
    pub const SOFTWARE: u16 = 0x8022;
    pub const ALTERNATE_SERVER: u16 = 0x8023;
    pub const FINGERPRINT: u16 = 0x8028;
    pub const ICE_CONTROLLED: u16 = 0x8029;
    pub const ICE_CONTROLLING: u16 = 0x802A;
}

/// Error codes this crate sends or reacts to.
pub mod code {
    pub const TRY_ALTERNATE: u16 = 300;
    pub const BAD_REQUEST: u16 = 400;
    pub const UNAUTHORIZED: u16 = 401;
    pub const FORBIDDEN: u16 = 403;
    pub const UNKNOWN_ATTRIBUTE: u16 = 420;
    pub const ALLOCATION_MISMATCH: u16 = 437;
    pub const STALE_NONCE: u16 = 438;
    pub const WRONG_CREDENTIALS: u16 = 441;
    pub const UNSUPPORTED_TRANSPORT: u16 = 442;
    pub const ALLOCATION_QUOTA: u16 = 486;
    pub const ROLE_CONFLICT: u16 = 487;
    pub const SERVER_ERROR: u16 = 500;
    pub const INSUFFICIENT_CAPACITY: u16 = 508;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class { Request, Indication, Success, Error }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Short, NotStun, BadLength, BadAttribute }

/// A first look: two zero bits, the magic cookie, a length that fits.
pub fn is_stun(b: &[u8]) -> bool {
    b.len() >= HEADER && b[0] & 0xC0 == 0 && b[4..8] == MAGIC.to_be_bytes()
}

/// The 16-byte long-term key: MD5(username ":" realm ":" password) (RFC 8489 9.2.2).
/// The strings are taken as given (no SASLprep: ASCII credentials, as TURN services issue).
pub fn long_term_key(username: &str, realm: &str, password: &str) -> [u8; 16] {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(username.as_bytes()); h.update(b":"); h.update(realm.as_bytes()); h.update(b":"); h.update(password.as_bytes());
    h.finalize().into()
}

pub fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> [u8; 20] {
    let mut m = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("HMAC takes any key");
    for p in parts { m.update(p); }
    m.finalize().into_bytes().into()
}

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    for p in parts { h.update(p); }
    h.finalize()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub class: Class,
    pub method: u16,
    pub transaction: [u8; 12],
    /// In order, values unpadded. MESSAGE-INTEGRITY and FINGERPRINT are added by `encode`.
    pub attributes: Vec<(u16, Vec<u8>)>,
}

impl Message {
    pub fn new(class: Class, method: u16, transaction: [u8; 12]) -> Self { Message { class, method, transaction, attributes: Vec::new() } }
    pub fn request(method: u16, transaction: [u8; 12]) -> Self { Self::new(Class::Request, method, transaction) }
    /// A success or error answer to `self`, with its transaction.
    pub fn reply(&self, success: bool) -> Self { Self::new(if success { Class::Success } else { Class::Error }, self.method, self.transaction) }

    pub fn add(&mut self, t: u16, v: &[u8]) -> &mut Self { self.attributes.push((t, v.into())); self }
    pub fn add_str(&mut self, t: u16, v: &str) -> &mut Self { self.add(t, v.as_bytes()) }
    pub fn add_u32(&mut self, t: u16, v: u32) -> &mut Self { self.add(t, &v.to_be_bytes()) }
    pub fn add_u64(&mut self, t: u16, v: u64) -> &mut Self { self.add(t, &v.to_be_bytes()) }
    pub fn add_flag(&mut self, t: u16) -> &mut Self { self.add(t, &[]) }
    pub fn add_xor_address(&mut self, t: u16, a: SocketAddr) -> &mut Self { let v = self.xor_address_bytes(a); self.add(t, &v) }
    pub fn add_address(&mut self, t: u16, a: SocketAddr) -> &mut Self { self.add(t, &address_bytes(a)) }
    pub fn add_error(&mut self, code: u16, reason: &str) -> &mut Self {
        let mut v = alloc::vec![0, 0, (code / 100) as u8, (code % 100) as u8];
        v.extend_from_slice(reason.as_bytes());
        self.add(attr::ERROR_CODE, &v)
    }

    pub fn get(&self, t: u16) -> Option<&[u8]> { self.attributes.iter().find(|a| a.0 == t).map(|a| a.1.as_slice()) }
    pub fn has(&self, t: u16) -> bool { self.get(t).is_some() }
    pub fn get_str(&self, t: u16) -> Option<&str> { core::str::from_utf8(self.get(t)?).ok() }
    pub fn get_u32(&self, t: u16) -> Option<u32> { Some(u32::from_be_bytes(self.get(t)?.get(..4)?.try_into().ok()?)) }
    pub fn get_u64(&self, t: u16) -> Option<u64> { Some(u64::from_be_bytes(self.get(t)?.get(..8)?.try_into().ok()?)) }
    pub fn xor_address(&self, t: u16) -> Option<SocketAddr> { self.unxor(self.get(t)?) }
    pub fn address(&self, t: u16) -> Option<SocketAddr> { plain_address(self.get(t)?) }
    /// The XOR-MAPPED-ADDRESS, or the old MAPPED-ADDRESS from a RFC 3489 server.
    pub fn mapped(&self) -> Option<SocketAddr> { self.xor_address(attr::XOR_MAPPED_ADDRESS).or_else(|| self.address(attr::MAPPED_ADDRESS)) }
    pub fn error_code(&self) -> Option<(u16, String)> {
        let v = self.get(attr::ERROR_CODE)?;
        if v.len() < 4 { return None; }
        let code = (v[2] & 7) as u16 * 100 + v[3] as u16;
        Some((code, String::from_utf8_lossy(&v[4..]).into()))
    }

    fn type_word(&self) -> u16 {
        let m = self.method & 0xFFF;
        let c: u16 = match self.class { Class::Request => 0, Class::Indication => 1, Class::Success => 2, Class::Error => 3 };
        (m & 0xF) | (c & 1) << 4 | (m & 0x70) << 1 | (c & 2) << 7 | (m & 0xF80) << 2
    }

    fn xor_address_bytes(&self, a: SocketAddr) -> Vec<u8> {
        let mut v = address_bytes(a);
        let port = a.port() ^ (MAGIC >> 16) as u16;
        v[2..4].copy_from_slice(&port.to_be_bytes());
        let mask = self.mask();
        for (i, b) in v[4..].iter_mut().enumerate() { *b ^= mask[i]; }
        v
    }
    fn unxor(&self, v: &[u8]) -> Option<SocketAddr> {
        let mut v: Vec<u8> = v.into();
        if v.len() < 8 { return None; }
        let port = u16::from_be_bytes([v[2], v[3]]) ^ (MAGIC >> 16) as u16;
        v[2..4].copy_from_slice(&port.to_be_bytes());
        let mask = self.mask();
        for (i, b) in v[4..].iter_mut().enumerate().take(16) { *b ^= mask[i]; }
        plain_address(&v)
    }
    fn mask(&self) -> [u8; 16] {
        let mut m = [0; 16];
        m[..4].copy_from_slice(&MAGIC.to_be_bytes());
        m[4..].copy_from_slice(&self.transaction);
        m
    }

    /// The wire form. With `integrity`, a MESSAGE-INTEGRITY (HMAC-SHA1) under that key
    /// (the password for ICE, the long-term key for TURN); with `fingerprint`, a
    /// FINGERPRINT last.
    pub fn encode(&self, integrity: Option<&[u8]>, fingerprint: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(128);
        out.extend_from_slice(&self.type_word().to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&MAGIC.to_be_bytes());
        out.extend_from_slice(&self.transaction);
        for (t, v) in &self.attributes {
            if *t == attr::MESSAGE_INTEGRITY || *t == attr::FINGERPRINT { continue; }
            out.extend_from_slice(&t.to_be_bytes());
            out.extend_from_slice(&(v.len() as u16).to_be_bytes());
            out.extend_from_slice(v);
            while out.len() % 4 != 0 { out.push(0); }
        }
        let set_len = |out: &mut Vec<u8>, extra: usize| {
            let l = (out.len() - HEADER + extra) as u16;
            out[2..4].copy_from_slice(&l.to_be_bytes());
        };
        if let Some(key) = integrity {
            set_len(&mut out, 24);
            let mac = hmac_sha1(key, &[&out]);
            out.extend_from_slice(&attr::MESSAGE_INTEGRITY.to_be_bytes());
            out.extend_from_slice(&20u16.to_be_bytes());
            out.extend_from_slice(&mac);
        }
        if fingerprint {
            set_len(&mut out, 8);
            let crc = crc32(&[&out]) ^ 0x5354_554E;
            out.extend_from_slice(&attr::FINGERPRINT.to_be_bytes());
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&crc.to_be_bytes());
        }
        set_len(&mut out, 0);
        out
    }

    /// Reads a message. Attributes after MESSAGE-INTEGRITY other than FINGERPRINT are
    /// dropped, as RFC 8489 says.
    pub fn decode(b: &[u8]) -> Result<Message, Error> {
        if b.len() < HEADER { return Err(Error::Short); }
        if !is_stun(b) { return Err(Error::NotStun); }
        let len = u16::from_be_bytes([b[2], b[3]]) as usize;
        if len % 4 != 0 || HEADER + len > b.len() { return Err(Error::BadLength); }
        let b = &b[..HEADER + len];
        let t = u16::from_be_bytes([b[0], b[1]]);
        let method = (t & 0xF) | (t >> 1 & 0x70) | (t >> 2 & 0xF80);
        let class = match (t >> 4 & 1) | (t >> 7 & 2) { 0 => Class::Request, 1 => Class::Indication, 2 => Class::Success, _ => Class::Error };
        let mut m = Message { class, method, transaction: b[8..20].try_into().unwrap(), attributes: Vec::new() };
        let mut i = HEADER;
        let mut after_integrity = false;
        while i + 4 <= b.len() {
            let at = u16::from_be_bytes([b[i], b[i + 1]]);
            let al = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            let v = b.get(i + 4..i + 4 + al).ok_or(Error::BadAttribute)?;
            if !after_integrity || at == attr::FINGERPRINT { m.attributes.push((at, v.into())); }
            if at == attr::MESSAGE_INTEGRITY { after_integrity = true; }
            i += 4 + (al + 3) / 4 * 4;
        }
        Ok(m)
    }

    /// The attribute types at or above 0x8000 are optional; below are required. Those
    /// this crate doesn't know, for a 420 answer.
    pub fn unknown_required(&self) -> Vec<u16> {
        const KNOWN: &[u16] = &[attr::MAPPED_ADDRESS, attr::USERNAME, attr::MESSAGE_INTEGRITY, attr::ERROR_CODE, attr::UNKNOWN_ATTRIBUTES,
            attr::CHANNEL_NUMBER, attr::LIFETIME, attr::XOR_PEER_ADDRESS, attr::DATA, attr::REALM, attr::NONCE, attr::XOR_RELAYED_ADDRESS,
            attr::REQUESTED_ADDRESS_FAMILY, attr::REQUESTED_TRANSPORT, attr::DONT_FRAGMENT, attr::XOR_MAPPED_ADDRESS, attr::PRIORITY, attr::USE_CANDIDATE];
        self.attributes.iter().map(|a| a.0).filter(|t| *t < 0x8000 && !KNOWN.contains(t)).collect()
    }
}

/// True when the raw message's MESSAGE-INTEGRITY matches under `key`.
pub fn check_integrity(raw: &[u8], key: &[u8]) -> bool {
    match find(raw, attr::MESSAGE_INTEGRITY) {
        Some((at, v)) if v.len() == 20 => {
            let mut head = [0u8; HEADER];
            head.copy_from_slice(&raw[..HEADER]);
            head[2..4].copy_from_slice(&((at + 24 - HEADER) as u16).to_be_bytes());
            let mac = hmac_sha1(key, &[&head, &raw[HEADER..at]]);
            // Constant time, as a check of a secret should be.
            mac.iter().zip(v).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
        }
        _ => false,
    }
}

/// None when there is no FINGERPRINT; else whether it matches.
pub fn check_fingerprint(raw: &[u8]) -> Option<bool> {
    let (at, v) = find(raw, attr::FINGERPRINT)?;
    if v.len() != 4 { return Some(false); }
    let mut head = [0u8; HEADER];
    head.copy_from_slice(&raw[..HEADER]);
    head[2..4].copy_from_slice(&((at + 8 - HEADER) as u16).to_be_bytes());
    let crc = crc32(&[&head, &raw[HEADER..at]]) ^ 0x5354_554E;
    Some(crc.to_be_bytes() == v[..4])
}

/// The offset and value of the first attribute of type `t`.
fn find(raw: &[u8], t: u16) -> Option<(usize, &[u8])> {
    if raw.len() < HEADER { return None; }
    let end = (HEADER + u16::from_be_bytes([raw[2], raw[3]]) as usize).min(raw.len());
    let mut i = HEADER;
    while i + 4 <= end {
        let at = u16::from_be_bytes([raw[i], raw[i + 1]]);
        let al = u16::from_be_bytes([raw[i + 2], raw[i + 3]]) as usize;
        let v = raw.get(i + 4..i + 4 + al)?;
        if at == t { return Some((i, v)); }
        i += 4 + (al + 3) / 4 * 4;
    }
    None
}

fn address_bytes(a: SocketAddr) -> Vec<u8> {
    let mut v = alloc::vec![0, 0];
    v.extend_from_slice(&a.port().to_be_bytes());
    match a.ip() {
        IpAddr::V4(ip) => { v[1] = 1; v.extend_from_slice(&ip.octets()); }
        IpAddr::V6(ip) => { v[1] = 2; v.extend_from_slice(&ip.octets()); }
    }
    v
}
fn plain_address(v: &[u8]) -> Option<SocketAddr> {
    let port = u16::from_be_bytes([*v.get(2)?, *v.get(3)?]);
    match v[1] {
        1 => Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(v.get(4..8)?).ok()?)), port)),
        2 => Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(v.get(4..20)?).ok()?)), port)),
        _ => None,
    }
}

/// TURN ChannelData (RFC 8656 12.4): the channel, then the length, then the data. Over
/// TCP and TLS it is padded to four bytes.
pub fn channel_data(channel: u16, data: &[u8], pad: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + data.len() + 3);
    v.extend_from_slice(&channel.to_be_bytes());
    v.extend_from_slice(&(data.len() as u16).to_be_bytes());
    v.extend_from_slice(data);
    if pad { while v.len() % 4 != 0 { v.push(0); } }
    v
}
pub fn is_channel_data(b: &[u8]) -> bool { b.len() >= 4 && (0x40..=0x4F).contains(&b[0]) }
/// The channel and data of a ChannelData message.
pub fn read_channel_data(b: &[u8]) -> Option<(u16, &[u8])> {
    if !is_channel_data(b) { return None; }
    let n = u16::from_be_bytes([b[2], b[3]]) as usize;
    Some((u16::from_be_bytes([b[0], b[1]]), b.get(4..4 + n)?))
}

/// Cuts a TCP or TLS byte stream to a TURN server into whole STUN and ChannelData
/// messages (RFC 6544-style framing, as RFC 8656 uses on streams).
#[derive(Default, Debug)]
pub struct Framer { buf: Vec<u8> }
impl Framer {
    pub fn push(&mut self, bytes: &[u8]) { self.buf.extend_from_slice(bytes); }
    /// The next whole message, if the stream holds one.
    pub fn next(&mut self) -> Option<Vec<u8>> {
        if self.buf.len() < 4 { return None; }
        let n = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
        let (take, keep) = match self.buf[0] >> 6 {
            0 => (HEADER + n, HEADER + n),
            1 => ((4 + n + 3) / 4 * 4, 4 + n),
            // Not a TURN stream: drop it rather than read garbage forever.
            _ => { self.buf.clear(); return None; }
        };
        if self.buf.len() < take { return None; }
        let mut msg: Vec<u8> = self.buf.drain(..take).collect();
        msg.truncate(keep);
        Some(msg)
    }
}
