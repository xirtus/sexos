// SPDX-License-Identifier: GPL-3.0-only
//! PKCE (RFC 7636, S256) and opaque state values. Randomness comes from the caller (the
//! service reads the entropy source), so these are pure and testable.
use crate::b64;
use alloc::string::String;
use sha2::{Digest, Sha256};

pub struct Pkce { pub verifier: String, pub challenge: String }

pub fn pkce(random32: &[u8; 32]) -> Pkce {
    let verifier = b64::encode(random32);
    let challenge = b64::encode(&Sha256::digest(verifier.as_bytes()));
    Pkce { verifier, challenge }
}

/// A state or nonce value: 18 random bytes, as leeloo-multipass uses.
pub fn state(random18: &[u8; 18]) -> String { b64::encode(random18) }

/// The bio-proof code a person pastes into their profile.
pub fn profile_code(random18: &[u8; 18]) -> String {
    let mut s = String::from("multipass-");
    for b in random18 { s.push_str(&alloc::format!("{b:02x}")); }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc7636_appendix_b() {
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = super::b64::encode(&<super::Sha256 as super::Digest>::digest(v.as_bytes()));
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let raw = super::b64::decode(v).unwrap();
        let p = super::pkce(&raw.try_into().unwrap());
        assert_eq!((p.verifier.as_str(), p.challenge.as_str()), (v, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert_eq!(super::profile_code(&[0xab; 18]).len(), 10 + 36);
    }
}
