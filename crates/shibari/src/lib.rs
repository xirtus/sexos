// SPDX-License-Identifier: GPL-3.0-only
//! Shibari keeps secrets (tokens, passwords, keys) in RAM only. A secret is tied to the
//! domain that stored it. Callers get an opaque [`Knot`] back and can ask Shibari to *use*
//! a secret inside a closure; there is no call that returns the bytes. Removing a secret,
//! or dropping the keeper, wipes it.
//!
//! This is the library the Shibari service and its clients share. Multipass stores account
//! tokens here, Chastity will store vault keys here, Quil's assistant its API key.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use zeroize::Zeroizing;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};

/// Which protection domain owns a secret. On SexOS this is the caller's PDX domain id,
/// filled in by the service from the call, never by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Owner(pub u32);

/// An opaque name for a stored secret. Knowing a knot is not authority: the owner must match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Knot(pub u64);

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such secret, or it belongs to another owner. The two look the same on purpose.
    Unknown,
    /// The keeper is full.
    Full,
    /// This knot does not contain a valid P-256 signing key.
    BadKey,
    /// A signing request exceeded the bounded input size.
    TooLarge,
}

struct Entry {
    owner: Owner,
    label: String,
    bytes: Zeroizing<Vec<u8>>,
}

/// The RAM-only keeper.
pub struct Keeper {
    entries: BTreeMap<Knot, Entry>,
    next: u64,
    limit: usize,
    salt: u64,
}

impl Keeper {
    /// `salt` comes from the entropy source so knots are not guessable sequences.
    pub fn new(salt: u64, limit: usize) -> Self {
        Self { entries: BTreeMap::new(), next: 1, limit, salt }
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Store a secret. The caller's copy should be dropped (zeroized) right after.
    pub fn tie(&mut self, owner: Owner, label: &str, secret: &[u8]) -> Result<Knot, Refused> {
        if self.entries.len() >= self.limit { return Err(Refused::Full); }
        let knot = Knot(self.next.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.salt);
        self.next += 1;
        self.entries.insert(knot, Entry { owner, label: label.into(), bytes: Zeroizing::new(secret.to_vec()) });
        Ok(knot)
    }

    /// Replace a secret in place (a refreshed token keeps its knot).
    pub fn retie(&mut self, owner: Owner, knot: Knot, secret: &[u8]) -> Result<(), Refused> {
        let entry = self.entry_mut(owner, knot)?;
        entry.bytes = Zeroizing::new(secret.to_vec());
        Ok(())
    }

    /// Use a secret in place. The closure sees the bytes; nothing it returns can borrow them.
    pub fn with<R>(&self, owner: Owner, knot: Knot, f: impl FnOnce(&[u8]) -> R) -> Result<R, Refused> {
        match self.entries.get(&knot) {
            Some(e) if e.owner == owner => Ok(f(&e.bytes)),
            _ => Err(Refused::Unknown),
        }
    }

    pub fn label(&self, owner: Owner, knot: Knot) -> Result<&str, Refused> {
        match self.entries.get(&knot) {
            Some(e) if e.owner == owner => Ok(&e.label),
            _ => Err(Refused::Unknown),
        }
    }

    /// Sign bounded CMS signed-attribute bytes with ECDSA P-256 and SHA-256.
    /// Only the DER signature leaves the keeper; the private scalar stays inside it.
    /// The caller must independently check that its X.509 certificate names this key.
    pub fn sign_p256_sha256(&self, owner: Owner, knot: Knot, signed_attrs: &[u8]) -> Result<Vec<u8>, Refused> {
        if signed_attrs.is_empty() || signed_attrs.len() > 8192 { return Err(Refused::TooLarge); }
        let secret = self.entries.get(&knot).filter(|entry| entry.owner == owner).ok_or(Refused::Unknown)?;
        let key = SigningKey::from_slice(&secret.bytes).map_err(|_| Refused::BadKey)?;
        let signature: Signature = key.sign(signed_attrs);
        Ok(signature.to_der().as_bytes().to_vec())
    }

    /// The uncompressed SEC1 public point for comparing with a certificate's key.
    /// It contains no private key material.
    pub fn p256_public_key(&self, owner: Owner, knot: Knot) -> Result<Vec<u8>, Refused> {
        let secret = self.entries.get(&knot).filter(|entry| entry.owner == owner).ok_or(Refused::Unknown)?;
        let key = SigningKey::from_slice(&secret.bytes).map_err(|_| Refused::BadKey)?;
        Ok(key.verifying_key().to_encoded_point(false).as_bytes().to_vec())
    }

    /// Wipe and forget a secret.
    pub fn untie(&mut self, owner: Owner, knot: Knot) -> Result<(), Refused> {
        self.entry_mut(owner, knot)?;
        self.entries.remove(&knot); // Zeroizing wipes on drop
        Ok(())
    }

    /// Wipe everything an owner stored (its program ended or its account was removed).
    pub fn untie_all(&mut self, owner: Owner) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, e| e.owner != owner);
        before - self.entries.len()
    }

    fn entry_mut(&mut self, owner: Owner, knot: Knot) -> Result<&mut Entry, Refused> {
        match self.entries.get_mut(&knot) {
            Some(e) if e.owner == owner => Ok(e),
            _ => Err(Refused::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{format, vec};

    #[test]
    fn owner_uses_secret_in_place_and_others_are_refused() {
        let mut k = Keeper::new(0x5eed, 8);
        let a = Owner(7);
        let knot = k.tie(a, "mastodon token", b"tok-123").unwrap();
        assert_eq!(k.with(a, knot, |s| s == b"tok-123"), Ok(true));
        assert_eq!(k.with(Owner(8), knot, |_| ()), Err(Refused::Unknown));
        assert_eq!(k.untie(Owner(8), knot), Err(Refused::Unknown));
        assert_eq!(k.label(a, knot), Ok("mastodon token"));
    }

    #[test]
    fn untie_forgets_and_limit_holds() {
        let mut k = Keeper::new(1, 2);
        let o = Owner(1);
        let x = k.tie(o, "x", b"1").unwrap();
        k.tie(o, "y", b"2").unwrap();
        assert_eq!(k.tie(o, "z", b"3"), Err(Refused::Full));
        k.retie(o, x, b"new").unwrap();
        assert_eq!(k.with(o, x, |s| s.to_vec()), Ok(b"new".to_vec()));
        k.untie(o, x).unwrap();
        assert_eq!(k.with(o, x, |_| ()), Err(Refused::Unknown));
        k.tie(Owner(2), "other", b"o").unwrap();
        assert_eq!(k.untie_all(o), 1);
        assert_eq!(k.len(), 1);
    }

    #[test]
    fn knots_are_not_small_sequential_numbers() {
        let mut k = Keeper::new(0xA5A5_0000_1234, 4);
        let a = k.tie(Owner(1), "a", b"a").unwrap();
        let b = k.tie(Owner(1), "b", b"b").unwrap();
        assert!(a.0 > 1 << 32 && b.0 > 1 << 32 && b.0 != a.0 + 1);
    }

    #[test]
    fn p256_signature_verifies_with_openssl_and_tampering_fails() {
        use std::process::Command;
        let mut keeper = Keeper::new(0x51a7, 2);
        let owner = Owner(17);
        let knot = keeper.tie(owner, "PDF signing key", &[7u8; 32]).unwrap();
        let message = b"DER CMS signed attributes for a PDF";
        assert_eq!(keeper.sign_p256_sha256(Owner(18), knot, message), Err(Refused::Unknown));
        assert_eq!(keeper.p256_public_key(Owner(18), knot), Err(Refused::Unknown));
        assert_eq!(keeper.sign_p256_sha256(owner, knot, &[]), Err(Refused::TooLarge));
        assert_eq!(keeper.sign_p256_sha256(owner, knot, &[0; 8193]), Err(Refused::TooLarge));
        let invalid = keeper.tie(owner, "invalid signing key", b"not a scalar").unwrap();
        assert_eq!(keeper.sign_p256_sha256(owner, invalid, message), Err(Refused::BadKey));
        let signature = keeper.sign_p256_sha256(owner, knot, message).unwrap();
        let point = keeper.p256_public_key(owner, knot).unwrap();
        assert_eq!(point.len(), 65);
        // SubjectPublicKeyInfo: id-ecPublicKey + prime256v1 + uncompressed point.
        let mut spki = vec![0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86,
            0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48,
            0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00];
        spki.extend_from_slice(&point);
        let dir = std::env::temp_dir().join(format!("shibari-sign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("public.der");
        let sig_path = dir.join("signature.der");
        let message_path = dir.join("message.bin");
        std::fs::write(&key_path, spki).unwrap();
        std::fs::write(&sig_path, signature).unwrap();
        std::fs::write(&message_path, message).unwrap();
        let verify = || Command::new("openssl").args(["dgst", "-sha256", "-keyform", "DER", "-verify",
            key_path.to_str().unwrap(), "-signature", sig_path.to_str().unwrap(), message_path.to_str().unwrap()])
            .output().expect("OpenSSL is required for independent signature verification");
        let good = verify();
        assert!(good.status.success(), "{}", String::from_utf8_lossy(&good.stderr));
        std::fs::write(&message_path, b"tampered signed attributes").unwrap();
        assert!(!verify().status.success());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
