// SPDX-License-Identifier: GPL-3.0-only
//! Time-based one-time passwords (RFC 6238, over HOTP, RFC 4226).
//!
//! An entry keeps its TOTP setup in the `otp` field as an `otpauth://totp/...` URI, as
//! KeePassXC does, so codes set up in either program work in the other. A bare Base32
//! secret is read too (SHA-1, 6 digits, 30 seconds).
use hmac::{Hmac, KeyInit, Mac};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm { Sha1, Sha256, Sha512 }

/// A TOTP setup. The secret is zeroized when it is dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Totp {
    pub secret: Zeroizing<Vec<u8>>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
}

impl core::fmt::Debug for Totp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Totp").field("algorithm", &self.algorithm).field("digits", &self.digits).field("period", &self.period).finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TotpError { NotTotp, BadSecret, BadParameter }

impl Totp {
    pub fn new(secret: Vec<u8>, algorithm: Algorithm, digits: u32, period: u64) -> Result<Self, TotpError> {
        if secret.is_empty() { return Err(TotpError::BadSecret); }
        if !(6..=10).contains(&digits) || period == 0 { return Err(TotpError::BadParameter); }
        Ok(Self { secret: Zeroizing::new(secret), algorithm, digits, period })
    }

    /// Reads an `otpauth://totp/...` URI or a bare Base32 secret.
    pub fn parse(text: &str) -> Result<Self, TotpError> {
        let text = text.trim();
        let Some(rest) = strip_prefix_ignore_case(text, "otpauth://") else {
            let secret = base32_decode(text).ok_or(TotpError::BadSecret)?;
            return Self::new(secret, Algorithm::Sha1, 6, 30);
        };
        let rest = strip_prefix_ignore_case(rest, "totp").filter(|r| r.starts_with('/') || r.starts_with('?')).ok_or(TotpError::NotTotp)?;
        let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
        let (mut secret, mut algorithm, mut digits, mut period) = (None, Algorithm::Sha1, 6, 30);
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(base32_decode(&v.replace("%3D", "=").replace("%3d", "=")).ok_or(TotpError::BadSecret)?),
                "algorithm" => algorithm = match v.to_ascii_uppercase().as_str() {
                    "SHA1" => Algorithm::Sha1, "SHA256" => Algorithm::Sha256, "SHA512" => Algorithm::Sha512,
                    _ => return Err(TotpError::BadParameter),
                },
                "digits" => digits = v.parse().map_err(|_| TotpError::BadParameter)?,
                "period" => period = v.parse().map_err(|_| TotpError::BadParameter)?,
                _ => {}
            }
        }
        Self::new(secret.ok_or(TotpError::BadSecret)?, algorithm, digits, period)
    }

    /// The code for Unix time `seconds`, zero-padded to `digits`.
    pub fn code(&self, seconds: u64) -> String {
        let value = hotp(&self.secret, seconds / self.period, self.algorithm, self.digits);
        format!("{value:0width$}", width = self.digits as usize)
    }

    /// Seconds until the code at `seconds` changes.
    pub fn remaining(&self, seconds: u64) -> u64 { self.period - seconds % self.period }

    /// An `otpauth://totp/` URI for this setup, labelled `label` (as stored in `otp`).
    pub fn to_uri(&self, label: &str) -> String {
        let alg = match self.algorithm { Algorithm::Sha1 => "SHA1", Algorithm::Sha256 => "SHA256", Algorithm::Sha512 => "SHA512" };
        let label: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || "-._~".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32 & 0xff) }).collect();
        format!("otpauth://totp/{label}?secret={}&algorithm={alg}&digits={}&period={}", base32_encode(&self.secret), self.digits, self.period)
    }
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix)).then(|| &s[prefix.len()..])
}

/// HOTP (RFC 4226) with the RFC 6238 hash choices.
pub fn hotp(secret: &[u8], counter: u64, algorithm: Algorithm, digits: u32) -> u64 {
    let msg = counter.to_be_bytes();
    let mac: Zeroizing<Vec<u8>> = Zeroizing::new(match algorithm {
        Algorithm::Sha1 => { let mut m = <Hmac<sha1::Sha1> as KeyInit>::new_from_slice(secret).expect("any key length"); m.update(&msg); m.finalize().into_bytes().to_vec() }
        Algorithm::Sha256 => { let mut m = <Hmac<sha2::Sha256> as KeyInit>::new_from_slice(secret).expect("any key length"); m.update(&msg); m.finalize().into_bytes().to_vec() }
        Algorithm::Sha512 => { let mut m = <Hmac<sha2::Sha512> as KeyInit>::new_from_slice(secret).expect("any key length"); m.update(&msg); m.finalize().into_bytes().to_vec() }
    });
    let offset = (mac[mac.len() - 1] & 0x0f) as usize;
    let binary = u32::from_be_bytes([mac[offset] & 0x7f, mac[offset + 1], mac[offset + 2], mac[offset + 3]]) as u64;
    binary % 10u64.pow(digits)
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 Base32, case-insensitive; spaces, dashes and `=` padding are ignored.
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let (mut out, mut buffer, mut bits) = (Vec::new(), 0u64, 0u32);
    for c in text.bytes() {
        if matches!(c, b' ' | b'-' | b'=') { continue; }
        let v = ALPHABET.iter().position(|a| *a == c.to_ascii_uppercase())? as u64;
        buffer = buffer << 5 | v;
        bits += 5;
        if bits >= 8 { bits -= 8; out.push((buffer >> bits) as u8); buffer &= (1 << bits) - 1; }
    }
    (!out.is_empty()).then_some(out)
}

pub fn base32_encode(bytes: &[u8]) -> String {
    let (mut out, mut buffer, mut bits) = (String::new(), 0u64, 0u32);
    for &b in bytes {
        buffer = buffer << 8 | b as u64;
        bits += 8;
        while bits >= 5 { bits -= 5; out.push(ALPHABET[(buffer >> bits) as usize & 31] as char); }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 { out.push(ALPHABET[(buffer << (5 - bits)) as usize & 31] as char); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B: 8 digits, 30 s, the ASCII seeds of 20, 32 and 64 bytes.
    #[test]
    fn rfc6238_vectors() {
        let seeds = [(Algorithm::Sha1, &b"12345678901234567890"[..]),
            (Algorithm::Sha256, &b"12345678901234567890123456789012"[..]),
            (Algorithm::Sha512, &b"1234567890123456789012345678901234567890123456789012345678901234"[..])];
        let table: [(u64, [&str; 3]); 6] = [
            (59, ["94287082", "46119246", "90693936"]),
            (1111111109, ["07081804", "68084774", "25091201"]),
            (1111111111, ["14050471", "67062674", "99943326"]),
            (1234567890, ["89005924", "91819424", "93441116"]),
            (2000000000, ["69279037", "90698825", "38618901"]),
            (20000000000, ["65353130", "77737706", "47863826"]),
        ];
        for (t, want) in table {
            for (i, (alg, seed)) in seeds.iter().enumerate() {
                let totp = Totp::new(seed.to_vec(), *alg, 8, 30).unwrap();
                assert_eq!(totp.code(t), want[i], "{alg:?} at {t}");
            }
        }
    }

    /// RFC 4226 Appendix D: HOTP, 6 digits.
    #[test]
    fn rfc4226_vectors() {
        let want = [755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489];
        for (c, w) in want.iter().enumerate() {
            assert_eq!(hotp(b"12345678901234567890", c as u64, Algorithm::Sha1, 6), *w);
        }
    }

    #[test]
    fn base32_round_trip() {
        assert_eq!(base32_encode(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(base32_decode("gezd gnbv-gy3tqojqgezdgnbvgy3tqojq").unwrap(), b"12345678901234567890");
        assert_eq!(base32_decode("MZXW6==="), Some(b"foo".to_vec()));
        assert_eq!(base32_encode(b"foo"), "MZXW6");
        assert_eq!(base32_decode("not base32!"), None);
    }

    #[test]
    fn otpauth_uris() {
        let t = Totp::parse("otpauth://totp/ACME:jdoe?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=ACME&algorithm=SHA256&digits=8&period=60").unwrap();
        assert_eq!((t.algorithm, t.digits, t.period), (Algorithm::Sha256, 8, 60));
        assert_eq!(&t.secret[..], b"12345678901234567890");
        let again = Totp::parse(&t.to_uri("ACME:jdoe")).unwrap();
        assert_eq!(again, t);
        let bare = Totp::parse("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        assert_eq!((bare.algorithm, bare.digits, bare.period), (Algorithm::Sha1, 6, 30));
        assert_eq!(bare.code(59), "287082");
        assert_eq!(bare.remaining(59), 1);
        assert_eq!(Totp::parse("otpauth://hotp/x?secret=GEZD&counter=1"), Err(TotpError::NotTotp));
        assert_eq!(Totp::parse("otpauth://totp/x?digits=6"), Err(TotpError::BadSecret));
        assert_eq!(Totp::parse("otpauth://totp/x?secret=GEZDGNBV&algorithm=MD5"), Err(TotpError::BadParameter));
        assert!(!format!("{t:?}").contains("secret:"), "Debug does not print the secret");
    }
}
