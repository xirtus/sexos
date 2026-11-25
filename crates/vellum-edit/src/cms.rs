//! Detached CMS SignedData for a Vellum PDF `/Contents` slot.
//!
//! This module holds no private key. A caller supplies the covered PDF byte slices,
//! an X.509 DER certificate, the public point from Shibari, and a callback that
//! signs DER encoded CMS attributes. Certificate trust and PDF permissions are
//! separate decisions; constructing a valid container does not make a signer trusted.
use sha2::{Digest, Sha256};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};

const ID_DATA: &[u8] = &[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01,
];
const ID_SIGNED_DATA: &[u8] = &[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02,
];
const ID_CONTENT_TYPE: &[u8] = &[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x03,
];
const ID_MESSAGE_DIGEST: &[u8] = &[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04,
];
const ID_SHA256: &[u8] = &[
    0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
];
const ID_ECDSA_SHA256: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const ID_EC_PUBLIC_KEY: &[u8] = &[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const ID_PRIME256V1: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmsError {
    BadCertificate,
    KeyMismatch,
    BadSignature,
    SignerRefused,
    TooLarge,
}

#[derive(Clone, Copy)]
struct Item<'a> {
    tag: u8,
    full: &'a [u8],
    value: &'a [u8],
}

/// Read one bounded, definite-length DER item. Reject non-minimal lengths.
fn read(input: &[u8]) -> Result<(Item<'_>, &[u8]), CmsError> {
    if input.len() < 2 {
        return Err(CmsError::BadCertificate);
    }
    let (tag, head) = (input[0], input[1]);
    let (len, offset) = if head < 0x80 {
        (head as usize, 2)
    } else {
        let count = (head & 0x7f) as usize;
        if count == 0 || count > 3 || input.len() < 2 + count || input[2] == 0 {
            return Err(CmsError::BadCertificate);
        }
        let mut len = 0usize;
        for byte in &input[2..2 + count] {
            len = (len << 8) | *byte as usize;
        }
        if len < 128 {
            return Err(CmsError::BadCertificate);
        }
        (len, 2 + count)
    };
    let end = offset
        .checked_add(len)
        .filter(|end| *end <= input.len())
        .ok_or(CmsError::BadCertificate)?;
    Ok((
        Item {
            tag,
            full: &input[..end],
            value: &input[offset..end],
        },
        &input[end..],
    ))
}

fn take<'a>(input: &mut &'a [u8], tag: u8) -> Result<Item<'a>, CmsError> {
    let (item, rest) = read(input)?;
    if item.tag != tag {
        return Err(CmsError::BadCertificate);
    }
    *input = rest;
    Ok(item)
}

/// Return issuer, serial and P-256 public point from an X.509 DER certificate.
/// Full certificate-chain validation remains the caller's policy decision.
fn cert_parts(cert: &[u8]) -> Result<(&[u8], &[u8], &[u8]), CmsError> {
    if cert.len() > 32 * 1024 {
        return Err(CmsError::TooLarge);
    }
    let mut input = cert;
    let outer = take(&mut input, 0x30)?;
    if !input.is_empty() {
        return Err(CmsError::BadCertificate);
    }
    let mut cert_fields = outer.value;
    let tbs = take(&mut cert_fields, 0x30)?;
    take(&mut cert_fields, 0x30)?; // signatureAlgorithm
    take(&mut cert_fields, 0x03)?; // signatureValue
    if !cert_fields.is_empty() {
        return Err(CmsError::BadCertificate);
    }
    let mut fields = tbs.value;
    if fields.first() == Some(&0xa0) {
        take(&mut fields, 0xa0)?;
    }
    let serial = take(&mut fields, 0x02)?;
    if serial.value.is_empty() || serial.value[0] & 0x80 != 0 {
        return Err(CmsError::BadCertificate);
    }
    take(&mut fields, 0x30)?; // TBSCertificate signature algorithm
    let issuer = take(&mut fields, 0x30)?;
    take(&mut fields, 0x30)?; // validity
    take(&mut fields, 0x30)?; // subject
    let spki = take(&mut fields, 0x30)?;
    let mut key_fields = spki.value;
    let algorithm = take(&mut key_fields, 0x30)?;
    let point = take(&mut key_fields, 0x03)?;
    if !key_fields.is_empty()
        || point.value.len() != 66
        || point.value[0] != 0
        || point.value[1] != 4
    {
        return Err(CmsError::BadCertificate);
    }
    let mut alg = algorithm.value;
    if take(&mut alg, 0x06)?.full != ID_EC_PUBLIC_KEY
        || take(&mut alg, 0x06)?.full != ID_PRIME256V1
        || !alg.is_empty()
    {
        return Err(CmsError::BadCertificate);
    }
    Ok((issuer.full, serial.full, &point.value[1..]))
}

/// The P-256 public point in a bounded X.509 certificate. Shibari compares
/// this with its private key's public point before it offers the certificate.
pub fn certificate_public_point(cert: &[u8]) -> Result<&[u8], CmsError> {
    cert_parts(cert).map(|(_, _, point)| point)
}

fn der(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = value.len();
    if len < 128 {
        out.push(len as u8);
    } else {
        let n = if len <= 0xff {
            1
        } else if len <= 0xffff {
            2
        } else {
            3
        };
        out.push(0x80 | n);
        for shift in (0..n).rev() {
            out.push((len >> (shift * 8)) as u8);
        }
    }
    out.extend_from_slice(value);
    out
}

fn joined(tag: u8, parts: &[&[u8]]) -> Vec<u8> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    let mut value = Vec::with_capacity(len);
    for part in parts {
        value.extend_from_slice(part);
    }
    der(tag, &value)
}

fn signed_attrs(digest: &[u8; 32]) -> Vec<u8> {
    let content = joined(0x30, &[ID_CONTENT_TYPE, &joined(0x31, &[ID_DATA])]);
    let message = joined(
        0x30,
        &[ID_MESSAGE_DIGEST, &joined(0x31, &[&der(0x04, digest)])],
    );
    let mut attrs = [content, message];
    attrs.sort(); // DER SET OF canonical order, independent of attribute names.
    joined(0x31, &[&attrs[0], &attrs[1]])
}

/// Construct detached CMS SignedData for exact PDF `/ByteRange` slices.
/// `sign` receives the DER SET OF attributes (tag `0x31`), while CMS stores the
/// same content as `[0] IMPLICIT` inside SignerInfo. It should call Shibari's
/// `sign_p256_sha256` with these bytes and return a DER ECDSA signature.
pub fn detached_p256_sha256(
    first: &[u8],
    second: &[u8],
    certificate_der: &[u8],
    public_point: &[u8],
    sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, CmsError>,
) -> Result<Vec<u8>, CmsError> {
    let (issuer, serial, cert_point) = cert_parts(certificate_der)?;
    if public_point != cert_point {
        return Err(CmsError::KeyMismatch);
    }
    let mut hash = Sha256::new();
    hash.update(first);
    hash.update(second);
    let attrs = signed_attrs(&hash.finalize().into());
    let signature = sign(&attrs)?;
    if signature.len() > 80 {
        return Err(CmsError::BadSignature);
    }
    let mut signature_input = signature.as_slice();
    let sequence = take(&mut signature_input, 0x30).map_err(|_| CmsError::BadSignature)?;
    if !signature_input.is_empty() {
        return Err(CmsError::BadSignature);
    }
    let mut components = sequence.value;
    for _ in 0..2 {
        let integer = take(&mut components, 0x02).map_err(|_| CmsError::BadSignature)?;
        if integer.value.is_empty() || integer.value[0] & 0x80 != 0 {
            return Err(CmsError::BadSignature);
        }
    }
    if !components.is_empty() {
        return Err(CmsError::BadSignature);
    }

    let digest_algorithm = joined(0x30, &[ID_SHA256]);
    let signature_algorithm = joined(0x30, &[ID_ECDSA_SHA256]);
    let sid = joined(0x30, &[issuer, serial]);
    let signer = joined(
        0x30,
        &[
            &der(0x02, &[1]),
            &sid,
            &digest_algorithm,
            &der(0xa0, &attrs[2..]),
            &signature_algorithm,
            &der(0x04, &signature),
        ],
    );
    let signed_data = joined(
        0x30,
        &[
            &der(0x02, &[1]),
            &joined(0x31, &[&digest_algorithm]),
            &joined(0x30, &[ID_DATA]),
            &der(0xa0, certificate_der),
            &joined(0x31, &[&signer]),
        ],
    );
    let output = joined(0x30, &[ID_SIGNED_DATA, &der(0xa0, &signed_data)]);
    if output.len() > 64 * 1024 {
        return Err(CmsError::TooLarge);
    }
    Ok(output)
}

/// Check the signed PDF bytes against a single-certificate detached CMS using
/// ECDSA P-256/SHA-256. This proves content integrity under the included public
/// key only. It does not establish the signer's identity or certificate trust.
/// Other CMS algorithms and layouts are left unsupported, never called valid.
pub fn verify_detached_p256_sha256(first: &[u8], second: &[u8], cms: &[u8]) -> Result<(), CmsError> {
    if cms.len() > 64 * 1024 { return Err(CmsError::TooLarge); }
    let mut input = cms;
    let outer = take(&mut input, 0x30)?;
    if input.iter().any(|byte| *byte != 0) { return Err(CmsError::BadCertificate); }
    let mut content = outer.value;
    if take(&mut content, 0x06)?.full != ID_SIGNED_DATA { return Err(CmsError::BadCertificate); }
    let signed_outer = take(&mut content, 0xa0)?;
    if !content.is_empty() { return Err(CmsError::BadCertificate); }
    let mut wrapper = signed_outer.value;
    let signed = take(&mut wrapper, 0x30)?;
    if !wrapper.is_empty() { return Err(CmsError::BadCertificate); }
    let mut fields = signed.value;
    if take(&mut fields, 0x02)?.full != [0x02, 0x01, 0x01] { return Err(CmsError::BadCertificate); }
    let algorithms = take(&mut fields, 0x31)?;
    if algorithms.value != joined(0x30, &[ID_SHA256]) { return Err(CmsError::BadCertificate); }
    let encapsulated = take(&mut fields, 0x30)?;
    if encapsulated.value != ID_DATA { return Err(CmsError::BadCertificate); }
    let certificates = take(&mut fields, 0xa0)?;
    let mut certs = certificates.value;
    let certificate = take(&mut certs, 0x30)?;
    if !certs.is_empty() { return Err(CmsError::BadCertificate); }
    let (issuer, serial, point) = cert_parts(certificate.full)?;
    let signers = take(&mut fields, 0x31)?;
    if !fields.is_empty() { return Err(CmsError::BadCertificate); }
    let mut signer_set = signers.value;
    let signer = take(&mut signer_set, 0x30)?;
    if !signer_set.is_empty() { return Err(CmsError::BadCertificate); }
    let mut signer_fields = signer.value;
    if take(&mut signer_fields, 0x02)?.full != [0x02, 0x01, 0x01]
        || take(&mut signer_fields, 0x30)?.full != joined(0x30, &[issuer, serial])
        || take(&mut signer_fields, 0x30)?.value != ID_SHA256
    { return Err(CmsError::BadCertificate); }
    let attrs_implicit = take(&mut signer_fields, 0xa0)?;
    if take(&mut signer_fields, 0x30)?.value != ID_ECDSA_SHA256 {
        return Err(CmsError::BadCertificate);
    }
    let signature = take(&mut signer_fields, 0x04)?;
    if !signer_fields.is_empty() { return Err(CmsError::BadCertificate); }
    let mut hash = Sha256::new();
    hash.update(first);
    hash.update(second);
    let expected_attrs = signed_attrs(&hash.finalize().into());
    let attrs = der(0x31, attrs_implicit.value);
    if attrs != expected_attrs { return Err(CmsError::BadSignature); }
    let key = VerifyingKey::from_sec1_bytes(point).map_err(|_| CmsError::BadCertificate)?;
    let signature = Signature::from_der(signature.value).map_err(|_| CmsError::BadSignature)?;
    key.verify(&attrs, &signature).map_err(|_| CmsError::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shibari::{Keeper, Owner};
    use std::{fs, process::Command};

    const CERT: &[u8] = include_bytes!("../tests/fixtures/cms-test-cert.der");
    const SCALAR: [u8; 32] = [
        0x15, 0xb6, 0xeb, 0x1d, 0x2f, 0xa7, 0xec, 0x84, 0xfc, 0x8d, 0x80, 0xef, 0x31, 0xbc, 0x5f,
        0x6c, 0x37, 0xdb, 0x2e, 0x88, 0x87, 0x94, 0x59, 0x32, 0xee, 0x00, 0xc7, 0x32, 0x2c, 0x42,
        0xa7, 0xce,
    ];

    #[test]
    fn openssl_verifies_detached_cms_over_exact_pdf_slices() {
        let owner = Owner(71);
        let mut keeper = Keeper::new(0x9811, 1);
        let knot = keeper.tie(owner, "Vellum fixture", &SCALAR).unwrap();
        let point = keeper.p256_public_key(owner, knot).unwrap();
        let (first, second) = (
            b"PDF bytes before /Contents".as_slice(),
            b"PDF bytes after /Contents".as_slice(),
        );
        let cms = detached_p256_sha256(first, second, CERT, &point, |attrs| {
            keeper
                .sign_p256_sha256(owner, knot, attrs)
                .map_err(|_| CmsError::SignerRefused)
        })
        .unwrap();
        assert_eq!(verify_detached_p256_sha256(first, second, &cms), Ok(()));
        assert_eq!(verify_detached_p256_sha256(b"changed", second, &cms), Err(CmsError::BadSignature));
        assert_eq!(verify_detached_p256_sha256(first, second, b"not cms"), Err(CmsError::BadCertificate));
        let path = std::env::temp_dir().join(format!("sexos-vellum-cms-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        let cms_path = path.join("signature.der");
        let content_path = path.join("content.bin");
        let output_path = path.join("verified.bin");
        fs::write(&cms_path, &cms).unwrap();
        fs::write(&content_path, [first, second].concat()).unwrap();
        let verify = |content: &std::path::Path| {
            Command::new("openssl")
                .args([
                    "cms",
                    "-verify",
                    "-inform",
                    "DER",
                    "-binary",
                    "-noverify",
                    "-in",
                ])
                .arg(&cms_path)
                .arg("-content")
                .arg(content)
                .arg("-out")
                .arg(&output_path)
                .output()
                .unwrap()
        };
        let good = verify(&content_path);
        assert!(
            good.status.success(),
            "{}",
            String::from_utf8_lossy(&good.stderr)
        );
        assert_eq!(fs::read(&output_path).unwrap(), [first, second].concat());
        fs::write(&content_path, b"tampered PDF bytes").unwrap();
        let bad = verify(&content_path);
        assert!(!bad.status.success(), "tampered content was accepted");
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn mismatched_key_and_malformed_inputs_are_refused_before_signing() {
        let mut called = false;
        let mut point = vec![4; 65];
        assert_eq!(
            detached_p256_sha256(b"a", b"b", CERT, &point, |_| {
                called = true;
                Ok(vec![])
            }),
            Err(CmsError::KeyMismatch)
        );
        assert!(!called);
        point.clear();
        assert_eq!(
            detached_p256_sha256(b"a", b"b", b"not a certificate", &point, |_| {
                called = true;
                Ok(vec![])
            }),
            Err(CmsError::BadCertificate)
        );
        assert!(!called);
    }
}
