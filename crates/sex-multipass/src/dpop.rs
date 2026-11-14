// SPDX-License-Identifier: GPL-3.0-only
//! DPoP proofs (RFC 9449) for Bluesky/AT Protocol OAuth, which binds every token to a key
//! the client holds. The key lives in Multipass's RAM next to the token in Shibari.
use crate::b64;
use alloc::format;
use alloc::string::String;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use sha2::{Digest, Sha256};

pub struct DpopKey { key: SigningKey }

impl DpopKey {
    /// From 32 bytes of entropy. Fails only for the (vanishingly unlikely) invalid scalar.
    pub fn from_random(bytes: &[u8; 32]) -> Option<Self> {
        SigningKey::from_bytes(bytes.into()).ok().map(|key| Self { key })
    }

    /// The public key as a JWK (`kty`, `crv`, `x`, `y`), members in lexicographic order so
    /// it doubles as the RFC 7638 thumbprint input.
    pub fn jwk(&self) -> String {
        let point = self.key.verifying_key().to_encoded_point(false);
        format!(r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#, b64::encode(point.x().unwrap()), b64::encode(point.y().unwrap()))
    }

    pub fn thumbprint(&self) -> String { b64::encode(&Sha256::digest(self.jwk().as_bytes())) }

    /// A proof for one request. `htu` loses its query and fragment; `ath` binds the access
    /// token when one is sent; `nonce` is the server's last `DPoP-Nonce`.
    pub fn proof(&self, method: &str, url: &str, iat: u64, jti: &str, nonce: Option<&str>, access_token: Option<&str>) -> String {
        let htu = url.split(['?', '#']).next().unwrap_or(url);
        let header = format!(r#"{{"typ":"dpop+jwt","alg":"ES256","jwk":{}}}"#, self.jwk());
        let mut claims = format!(r#"{{"jti":{},"htm":{},"htu":{},"iat":{iat}"#, json_str(jti), json_str(method), json_str(htu));
        if let Some(n) = nonce { claims.push_str(&format!(r#","nonce":{}"#, json_str(n))); }
        if let Some(t) = access_token { claims.push_str(&format!(r#","ath":"{}""#, b64::encode(&Sha256::digest(t.as_bytes())))); }
        claims.push('}');
        let input = format!("{}.{}", b64::encode(header.as_bytes()), b64::encode(claims.as_bytes()));
        let sig: Signature = self.key.sign(input.as_bytes());
        format!("{input}.{}", b64::encode(&sig.to_bytes()))
    }
}

pub fn json_str(s: &str) -> String { serde_json::to_string(s).unwrap_or_default() }

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, VerifyingKey};

    #[test]
    fn proof_verifies_and_binds_method_url_nonce_and_token() {
        let k = DpopKey::from_random(&[7u8; 32]).unwrap();
        let jwt = k.proof("POST", "https://bsky.social/oauth/token?x=1", 1_700_000_000, "jti-1", Some("srv-nonce"), Some("at"));
        let parts: alloc::vec::Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value = serde_json::from_slice(&b64::decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["htu"], "https://bsky.social/oauth/token");
        assert_eq!(claims["htm"], "POST");
        assert_eq!(claims["nonce"], "srv-nonce");
        assert_eq!(claims["ath"], b64::encode(&Sha256::digest(b"at")).as_str());
        let header: serde_json::Value = serde_json::from_slice(&b64::decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["jwk"]["crv"], "P-256");
        let sig = Signature::from_slice(&b64::decode(parts[2]).unwrap()).unwrap();
        let vk = VerifyingKey::from(&k.key);
        assert!(vk.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).is_ok());
        assert_eq!(k.thumbprint().len(), 43);
    }
}
