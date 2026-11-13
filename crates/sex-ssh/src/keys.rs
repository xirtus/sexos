// SPDX-License-Identifier: GPL-3.0-only
//! Host key signatures and user keys.
use crate::wire::{Reader, Writer};
use crate::Error;
use alloc::string::String;
use alloc::vec::Vec;
use sha2::{Digest, Sha256, Sha512};

/// Checks the server's signature over the exchange hash `h` with its host key `blob`.
pub fn verify_host(algorithm: &str, blob: &[u8], sig: &[u8], h: &[u8]) -> Result<(), Error> {
    let bad = Error::HostKeySignature;
    let mut k = Reader::new(blob);
    let mut s = Reader::new(sig);
    let key_type = k.string()?;
    let sig_type = s.string()?;
    if sig_type != algorithm.as_bytes() { return Err(bad); }
    let sig = s.string()?;
    match algorithm {
        "ssh-ed25519" => {
            if key_type != b"ssh-ed25519" { return Err(bad); }
            let pk = ed25519_compact::PublicKey::from_slice(k.string()?).map_err(|_| bad)?;
            let sig = ed25519_compact::Signature::from_slice(sig).map_err(|_| bad)?;
            pk.verify(h, &sig).map_err(|_| bad)
        }
        "ecdsa-sha2-nistp256" => {
            use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
            if key_type != algorithm.as_bytes() || k.string()? != b"nistp256" { return Err(bad); }
            let key = VerifyingKey::from_sec1_bytes(k.string()?).map_err(|_| bad)?;
            let mut rs = Reader::new(sig);
            let (r, s) = (rs.mpint()?, rs.mpint()?);
            if r.len() > 32 || s.len() > 32 { return Err(bad); }
            let mut raw = [0u8; 64];
            raw[32 - r.len()..32].copy_from_slice(r);
            raw[64 - s.len()..].copy_from_slice(s);
            let sig = Signature::from_slice(&raw).map_err(|_| bad)?;
            key.verify(h, &sig).map_err(|_| bad)
        }
        "rsa-sha2-256" | "rsa-sha2-512" => {
            use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
            if key_type != b"ssh-rsa" { return Err(bad); }
            let e = BigUint::from_bytes_be(k.mpint()?);
            let n = BigUint::from_bytes_be(k.mpint()?);
            if n.bits() < 2048 { return Err(bad); }
            let key = RsaPublicKey::new(n, e).map_err(|_| bad)?;
            if algorithm == "rsa-sha2-256" { key.verify(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(h), sig) } else { key.verify(Pkcs1v15Sign::new::<Sha512>(), &Sha512::digest(h), sig) }.map_err(|_| bad)
        }
        _ => Err(bad),
    }
}

/// "SHA256:" and the unpadded base64 of the key's SHA-256, as OpenSSH shows it.
pub fn fingerprint(blob: &[u8]) -> String {
    let mut s = String::from("SHA256:");
    s.push_str(&base64(&Sha256::digest(blob)));
    s.trim_end_matches('=').into()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub fn base64(b: &[u8]) -> String {
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { if i <= c.len() { s.push(B64[(n >> (18 - 6 * i)) as usize & 63] as char); } else { s.push('='); } }
    }
    s
}
pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' { break; }
        let v = B64.iter().position(|x| *x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); }
    }
    Some(out)
}

/// An Ed25519 user key.
pub struct Ed25519 { key: ed25519_compact::KeyPair, pub comment: String }

impl Ed25519 {
    pub fn from_seed(seed: [u8; 32]) -> Self { Self { key: ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(seed)), comment: String::new() } }
    /// Reads an unencrypted OpenSSH private key file ("-----BEGIN OPENSSH PRIVATE KEY-----").
    pub fn from_openssh(text: &str) -> Result<Self, KeyError> {
        let body: String = text.lines().filter(|l| !l.starts_with("-----")).collect();
        let raw = unbase64(&body).ok_or(KeyError::Format)?;
        let rest = raw.strip_prefix(b"openssh-key-v1\0".as_slice()).ok_or(KeyError::Format)?;
        let mut r = Reader::new(rest);
        let (cipher, kdf, _opts) = (r.string().map_err(|_| KeyError::Format)?, r.string().map_err(|_| KeyError::Format)?, r.string().map_err(|_| KeyError::Format)?);
        if cipher != b"none" || kdf != b"none" { return Err(KeyError::Encrypted); }
        if r.u32().map_err(|_| KeyError::Format)? != 1 { return Err(KeyError::Format); }
        let _public = r.string().map_err(|_| KeyError::Format)?;
        let mut p = Reader::new(r.string().map_err(|_| KeyError::Format)?);
        let f = |_| KeyError::Format;
        if p.u32().map_err(f)? != p.u32().map_err(f)? { return Err(KeyError::Format); }
        if p.string().map_err(f)? != b"ssh-ed25519" { return Err(KeyError::Unsupported); }
        let _pk = p.string().map_err(f)?;
        let sk = p.string().map_err(f)?;
        let comment = String::from_utf8_lossy(p.string().map_err(f)?).into();
        let key = ed25519_compact::KeyPair::from_slice(sk).map_err(|_| KeyError::Format)?;
        Ok(Self { key, comment })
    }
    /// The key as an unencrypted OpenSSH private key file, readable by OpenSSH too.
    pub fn to_openssh(&self, check: u32) -> String {
        let mut private = Writer::new();
        private.u32(check).u32(check).string(b"ssh-ed25519").string(&self.key.pk[..]).string(&self.key.sk[..]).string(self.comment.as_bytes());
        let mut p = private.finish();
        let mut i = 1u8;
        while p.len() % 8 != 0 { p.push(i); i += 1; }
        let mut w = Writer::new();
        w.raw(b"openssh-key-v1\0").string(b"none").string(b"none").string(b"").u32(1).string(&self.public_blob()).string(&p);
        let b = base64(&w.finish());
        let mut out = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
        for line in b.as_bytes().chunks(70) { out.push_str(core::str::from_utf8(line).unwrap()); out.push('\n'); }
        out.push_str("-----END OPENSSH PRIVATE KEY-----\n");
        out
    }
    pub fn public_blob(&self) -> Vec<u8> { let mut w = Writer::new(); w.string(b"ssh-ed25519").string(&self.key.pk[..]); w.finish() }
    /// The line for a server's authorized_keys.
    pub fn authorized_line(&self) -> String { let mut s = String::from("ssh-ed25519 "); s.push_str(&base64(&self.public_blob())); s }
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let sig = self.key.sk.sign(data, None);
        let mut w = Writer::new();
        w.string(b"ssh-ed25519").string(&sig[..]);
        w.finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError { Format, Encrypted, Unsupported }
