// SPDX-License-Identifier: GPL-3.0-only
//! Self-signed ECDSA P-256 certificates and their SHA-256 fingerprints.
//!
//! WebRTC does not use a CA chain: each side makes its own certificate and the SDP carries
//! its fingerprint (RFC 8122 a=fingerprint). So we write the smallest X.509 v3 certificate
//! peers accept, and from a peer's certificate we read only its public key.
use alloc::string::String;
use alloc::vec::Vec;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_CN: &[u8] = &[0x55, 0x04, 0x03];

/// A SHA-256 certificate fingerprint, as SDP a=fingerprint:sha-256 carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of(der: &[u8]) -> Fingerprint {
        Fingerprint(Sha256::digest(der).into())
    }

    /// Parse "sha-256 AB:CD:..." or just the colon hex. Other hashes are refused.
    pub fn parse_sdp(s: &str) -> Option<Fingerprint> {
        let s = s.trim();
        let hex = match s.split_once(' ') {
            Some((alg, h)) if alg.eq_ignore_ascii_case("sha-256") => h.trim(),
            Some(_) => return None,
            None => s,
        };
        let mut out = [0u8; 32];
        let mut n = 0;
        for part in hex.split(':') {
            if n == 32 || part.len() != 2 {
                return None;
            }
            out[n] = u8::from_str_radix(part, 16).ok()?;
            n += 1;
        }
        (n == 32).then_some(Fingerprint(out))
    }

    /// "sha-256 AB:CD:..." for an SDP a=fingerprint line.
    pub fn to_sdp(&self) -> String {
        let mut s = String::from("sha-256 ");
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                s.push(':');
            }
            for d in [b >> 4, b & 15] {
                s.push(char::from_digit(d as u32, 16).unwrap().to_ascii_uppercase());
            }
        }
        s
    }

    pub fn ct_eq(&self, other: &Fingerprint) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

/// Our certificate and its key.
#[derive(Clone)]
pub struct Certificate {
    pub der: Vec<u8>,
    pub(crate) key: SigningKey,
}

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 4);
    v.push(tag);
    let n = body.len();
    if n < 0x80 {
        v.push(n as u8);
    } else if n < 0x100 {
        v.extend_from_slice(&[0x81, n as u8]);
    } else {
        v.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    }
    v.extend_from_slice(body);
    v
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

impl Certificate {
    /// A fresh key and a self-signed certificate for it (CN "sexos", valid 2024-2049).
    pub fn generate(rng: &mut impl CryptoRngCore) -> Certificate {
        let key = SigningKey::random(rng);
        let mut serial = [0u8; 8];
        rng.fill_bytes(&mut serial);
        serial[0] = (serial[0] & 0x7f) | 0x01;
        Certificate::build(key, &serial)
    }

    /// Rebuild from a saved 32-byte private key and DER certificate.
    pub fn from_parts(der: Vec<u8>, private_key: &[u8]) -> Option<Certificate> {
        let key = SigningKey::from_slice(private_key).ok()?;
        let pk = public_key(&der)?;
        (pk == *key.verifying_key()).then_some(Certificate { der, key })
    }

    pub fn private_key(&self) -> [u8; 32] {
        self.key.to_bytes().into()
    }

    fn build(key: SigningKey, serial: &[u8]) -> Certificate {
        let alg = seq(&[&tlv(0x06, OID_ECDSA_SHA256)]);
        let name = seq(&[&tlv(0x31, &seq(&[&tlv(0x06, OID_CN), &tlv(0x0c, b"sexos")]))]);
        let validity = seq(&[&tlv(0x17, b"240101000000Z"), &tlv(0x17, b"491231235959Z")]);
        let point = key.verifying_key().to_encoded_point(false);
        let mut bits = Vec::from([0u8]);
        bits.extend_from_slice(point.as_bytes());
        let spki = seq(&[&seq(&[&tlv(0x06, OID_EC_PUBLIC_KEY), &tlv(0x06, OID_P256)]), &tlv(0x03, &bits)]);
        let version = tlv(0xa0, &tlv(0x02, &[2]));
        let tbs = seq(&[&version, &tlv(0x02, serial), &alg, &name, &validity, &name, &spki]);
        let sig: Signature = key.sign(&tbs);
        let mut sbits = Vec::from([0u8]);
        sbits.extend_from_slice(sig.to_der().as_bytes());
        let der = seq(&[&tbs, &alg, &tlv(0x03, &sbits)]);
        Certificate { der, key }
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(&self.der)
    }
}

/// One DER element: (tag, body, rest).
fn next(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, d) = d.split_first()?;
    let (&l0, mut d) = d.split_first()?;
    let len = if l0 < 0x80 {
        l0 as usize
    } else {
        let k = (l0 & 0x7f) as usize;
        if k == 0 || k > 3 || d.len() < k {
            return None;
        }
        let mut n = 0usize;
        for &b in &d[..k] {
            n = n << 8 | b as usize;
        }
        d = &d[k..];
        n
    };
    (d.len() >= len).then(|| (tag, &d[..len], &d[len..]))
}

/// The P-256 public key of a certificate. Other key types are refused.
pub fn public_key(der: &[u8]) -> Option<VerifyingKey> {
    let (t, cert, _) = next(der)?;
    if t != 0x30 {
        return None;
    }
    let (t, tbs, _) = next(cert)?;
    if t != 0x30 {
        return None;
    }
    let mut d = tbs;
    let (t, _, rest) = next(d)?;
    if t == 0xa0 {
        d = rest;
    }
    // serial, signature, issuer, validity, subject, then the key.
    for _ in 0..5 {
        d = next(d)?.2;
    }
    let (t, spki, _) = next(d)?;
    if t != 0x30 {
        return None;
    }
    let (_, alg, rest) = next(spki)?;
    let (_, oid, alg_rest) = next(alg)?;
    let (_, curve, _) = next(alg_rest)?;
    if oid != OID_EC_PUBLIC_KEY || curve != OID_P256 {
        return None;
    }
    let (t, bits, _) = next(rest)?;
    if t != 0x03 || bits.first() != Some(&0) {
        return None;
    }
    VerifyingKey::from_sec1_bytes(&bits[1..]).ok()
}
