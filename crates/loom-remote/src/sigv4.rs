// SPDX-License-Identifier: GPL-3.0-only
//! AWS Signature Version 4 (header form), as S3 and S3-compatible stores check it.
use crate::text::{hex, percent_encode};
use alloc::{string::String, vec::Vec};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// An access key pair. `Debug` shows the key id only.
#[derive(Clone)]
pub struct Credentials { pub access_key: String, secret: String, pub session_token: Option<String> }
impl Credentials {
    pub fn new(access_key: &str, secret: &str) -> Self { Self { access_key: access_key.into(), secret: secret.into(), session_token: None } }
    /// Temporary credentials (STS) carry a session token too.
    pub fn with_token(mut self, token: &str) -> Self { self.session_token = Some(token.into()); self }
}
impl core::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.debug_struct("Credentials").field("access_key", &self.access_key).finish_non_exhaustive() }
}

pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
pub fn sha256_hex(b: &[u8]) -> String { hex(&Sha256::digest(b)) }

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    m.update(data);
    m.finalize().into_bytes().into()
}

/// The canonical query string: names and values encoded, sorted.
pub fn canonical_query(pairs: &[(&str, &str)]) -> String {
    let mut v: Vec<(String, String)> = pairs.iter().map(|(k, v)| (percent_encode(k, false), percent_encode(v, false))).collect();
    v.sort();
    let mut s = String::new();
    for (i, (k, v)) in v.iter().enumerate() { if i > 0 { s.push('&'); } s.push_str(k); s.push('='); s.push_str(v); }
    s
}

pub struct Scope<'a> { pub amz_date: &'a str, pub region: &'a str, pub service: &'a str }

/// The canonical request (section "Create a canonical request") and the signed header list.
pub fn canonical_request(method: &str, uri: &str, query: &str, headers: &[(&str, &str)], payload_hash: &str) -> (String, String) {
    let mut h: Vec<(String, String)> = Vec::new();
    for (n, v) in headers {
        let n = n.to_ascii_lowercase();
        let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
        match h.iter_mut().find(|(k, _)| *k == n) { Some((_, old)) => { old.push(','); old.push_str(&v); } None => h.push((n, v)) }
    }
    h.sort();
    let mut c = alloc::format!("{method}\n{uri}\n{query}\n");
    for (n, v) in &h { c.push_str(n); c.push(':'); c.push_str(v); c.push('\n'); }
    let signed = h.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(";");
    c.push('\n');
    c.push_str(&signed);
    c.push('\n');
    c.push_str(payload_hash);
    (c, signed)
}

/// The Authorization header value for a request whose `headers` (which must include
/// `host` and `x-amz-date`) are all signed.
pub fn authorization(creds: &Credentials, scope: &Scope<'_>, method: &str, uri: &str, query: &str, headers: &[(&str, &str)], payload_hash: &str) -> String {
    let (creq, signed) = canonical_request(method, uri, query, headers, payload_hash);
    let day = &scope.amz_date[..8];
    let cscope = alloc::format!("{day}/{}/{}/aws4_request", scope.region, scope.service);
    let to_sign = alloc::format!("AWS4-HMAC-SHA256\n{}\n{cscope}\n{}", scope.amz_date, sha256_hex(creq.as_bytes()));
    let mut k = hmac(alloc::format!("AWS4{}", creds.secret).as_bytes(), day.as_bytes());
    k = hmac(&k, scope.region.as_bytes());
    k = hmac(&k, scope.service.as_bytes());
    k = hmac(&k, b"aws4_request");
    let sig = hex(&hmac(&k, to_sign.as_bytes()));
    alloc::format!("AWS4-HMAC-SHA256 Credential={}/{cscope}, SignedHeaders={signed}, Signature={sig}", creds.access_key)
}
