// SPDX-License-Identifier: GPL-3.0-only
//! Narrow allocation-free HTTP message codec for a controlled diagnostic peer.
//! The fixed request grants no network authority. Parse only after complete
//! transport receipt: a successful parse publishes a body range, never a prefix.
//! No redirects, cookies, HTML interpretation, TLS, caching or online-web claim.
//! Content-Type is required and restricted to text/plain, optionally with the
//! single unquoted charset=us-ascii parameter. Body bytes remain raw data.

pub const MAX_REQUEST: usize = 128;
pub const MAX_RESPONSE: usize = 4096;
pub const MAX_HEADERS: usize = 1024;
pub const MAX_BODY: usize = 2048;
// Exact supported-profile fixture for host/native wire oracles. Runtime Host
// bytes come from the shared BOOT profile below; the actual request test binds it.
const REQUEST: &[u8] = b"GET /probe HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub body: core::ops::Range<usize>,
    pub content_length: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    Malformed,
    ResponseLimit,
    HeaderLimit,
    BodyLimit,
    UnsupportedVersion,
    UnsupportedStatus,
    MissingContentLength,
    InvalidContentLength,
    DuplicateContentLength,
    MissingContentType,
    UnsupportedContentType,
    UnsupportedEncoding,
    TransferEncoding,
    TrailingData,
}

/// Emit only the fixed controlled request; the returned length is the extent
/// to send. Bytes beyond it are not part of the request and are left untouched.
pub fn request(out: &mut [u8; MAX_REQUEST]) -> usize {
    let prefix=b"GET /probe HTTP/1.1\r\nHost: ";
    let host=sex_pdx::network_config::HOST_NAME;
    let suffix=b"\r\nConnection: close\r\nAccept: text/plain\r\n\r\n";
    let length=prefix.len()+host.len()+suffix.len();
    assert!(length<=MAX_REQUEST,"BOOT_HTTP_PROFILE_BOUND");
    out[..prefix.len()].copy_from_slice(prefix);
    out[prefix.len()..prefix.len()+host.len()].copy_from_slice(host);
    out[prefix.len()+host.len()..length].copy_from_slice(suffix);
    length
}

fn line<'a>(input: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], Error> {
    let start = *cursor;
    let mut at = start;
    while let Some(&byte) = input.get(at) {
        if at >= MAX_HEADERS { return Err(Error::HeaderLimit); }
        if byte == b'\r' {
            let next = at.checked_add(1).ok_or(Error::HeaderLimit)?;
            match input.get(next) {
                None => return Err(Error::Truncated),
                Some(b'\n') => {},
                _ => return Err(Error::Malformed),
            }
            let end = next.checked_add(1).ok_or(Error::HeaderLimit)?;
            if end > MAX_HEADERS { return Err(Error::HeaderLimit); }
            *cursor = end;
            return Ok(&input[start..at]);
        }
        // HTAB is allowed only as field-value whitespace. Name/status checks
        // below restrict its position; obs-fold is rejected independently.
        if (byte < 0x20 && byte != b'\t') || byte >= 0x7F { return Err(Error::Malformed); }
        at += 1;
    }
    Err(if at >= MAX_HEADERS { Error::HeaderLimit } else { Error::Truncated })
}
fn trim(mut bytes: &[u8]) -> &[u8] {
    while matches!(bytes.first(), Some(b' ' | b'\t')) { bytes = &bytes[1..]; }
    while matches!(bytes.last(), Some(b' ' | b'\t')) { bytes = &bytes[..bytes.len()-1]; }
    bytes
}
fn token(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|&b| b.is_ascii_alphanumeric()
        || matches!(b, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+'
            | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'))
}
fn length(bytes: &[u8]) -> Result<usize, Error> {
    if bytes.is_empty() { return Err(Error::InvalidContentLength); }
    let mut value = 0usize;
    for &byte in bytes {
        if !byte.is_ascii_digit() { return Err(Error::InvalidContentLength); }
        value = value.checked_mul(10).and_then(|v| v.checked_add((byte-b'0') as usize))
            .ok_or(Error::InvalidContentLength)?;
    }
    if value > MAX_BODY { return Err(Error::BodyLimit); }
    Ok(value)
}
fn content_type(bytes: &[u8]) -> Result<(), Error> {
    let mut parts = bytes.split(|&b| b == b';');
    if !trim(parts.next().unwrap_or(&[])).eq_ignore_ascii_case(b"text/plain") {
        return Err(Error::UnsupportedContentType);
    }
    if let Some(parameter) = parts.next() {
        let equals = parameter.iter().position(|&b| b == b'=').ok_or(Error::UnsupportedContentType)?;
        if !trim(&parameter[..equals]).eq_ignore_ascii_case(b"charset")
            || !trim(&parameter[equals+1..]).eq_ignore_ascii_case(b"us-ascii") {
            return Err(Error::UnsupportedContentType);
        }
    }
    if parts.next().is_some() { return Err(Error::UnsupportedContentType); }
    Ok(())
}

/// Require one complete supported response with an exact Content-Length.
/// Header extent includes status line and the final CRLFCRLF. Every header and
/// the entire body extent are validated before returning any publishable range.
pub fn parse(input: &[u8]) -> Result<Response, Error> {
    if input.len() > MAX_RESPONSE { return Err(Error::ResponseLimit); }
    let mut cursor = 0;
    let status = line(input, &mut cursor)?;
    if status.len() < 13 || &status[..9] != b"HTTP/1.1 " {
        return Err(if status.starts_with(b"HTTP/") { Error::UnsupportedVersion } else { Error::Malformed });
    }
    if !status[9..12].iter().all(u8::is_ascii_digit) || status[12] != b' '
        || status[13..].iter().any(|&b| b < 0x20) { return Err(Error::Malformed); }
    if &status[9..12] != b"200" { return Err(Error::UnsupportedStatus); }
    let mut body_length = None;
    let mut has_type = false;
    loop {
        let header = line(input, &mut cursor)?;
        if header.is_empty() { break; }
        if matches!(header.first(), Some(b' ' | b'\t')) { return Err(Error::Malformed); }
        let colon = header.iter().position(|&b| b == b':').ok_or(Error::Malformed)?;
        let name = &header[..colon];
        if !token(name) { return Err(Error::Malformed); }
        let value = trim(&header[colon+1..]);
        if name.eq_ignore_ascii_case(b"transfer-encoding") { return Err(Error::TransferEncoding); }
        if name.eq_ignore_ascii_case(b"content-encoding") { return Err(Error::UnsupportedEncoding); }
        if name.eq_ignore_ascii_case(b"content-length") {
            if body_length.is_some() { return Err(Error::DuplicateContentLength); }
            body_length = Some(length(value)?);
        } else if name.eq_ignore_ascii_case(b"content-type") {
            if has_type { return Err(Error::UnsupportedContentType); }
            content_type(value)?;has_type = true;
        }
    }
    let content_length = body_length.ok_or(Error::MissingContentLength)?;
    if !has_type { return Err(Error::MissingContentType); }
    let end = cursor.checked_add(content_length).ok_or(Error::InvalidContentLength)?;
    if input.len() < end { return Err(Error::Truncated); }
    if input.len() > end { return Err(Error::TrailingData); }
    Ok(Response { body: cursor..end, content_length })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{format, vec, vec::Vec};
    fn response(body: &[u8]) -> Vec<u8> {
        let mut bytes = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);bytes
    }
    fn message(headers: &str, body: &[u8]) -> Vec<u8> {
        let mut bytes = format!("HTTP/1.1 200 OK\r\n{headers}\r\n").into_bytes();bytes.extend_from_slice(body);bytes
    }
    #[test] fn actual_fixed_request_bytes() {
        let mut out = [0xA5; MAX_REQUEST];let len = request(&mut out);
        assert_eq!(&out[..len], b"GET /probe HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n");
        assert!(out[len..].iter().all(|b| *b == 0xA5));
    }
    #[test] fn complete_200_exposes_exact_body_only_after_full_receipt() {
        let bytes = response(b"controlled peer body\n");let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.content_length, 21);assert_eq!(&bytes[parsed.body], b"controlled peer body\n");
    }
    #[test] fn every_truncation_cut_fails_without_body_publication() {
        for body in [b"real response".as_slice(), b"".as_slice()] {
            let bytes = response(body);
            for cut in 0..bytes.len() { assert!(parse(&bytes[..cut]).is_err(), "cut={cut}"); }
        }
    }
    #[test] fn empty_body_has_explicit_zero_length_range() {
        let bytes = response(b"");let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.content_length, 0);assert_eq!(parsed.body, bytes.len()..bytes.len());
    }
    #[test] fn header_names_mime_and_ascii_charset_are_case_insensitive() {
        let bytes = message("cOnTeNt-LeNgTh:\t 0004 \t\r\ncOnTeNt-TyPe: TeXt/PlAiN ; Charset = US-ASCII\r\nX-Test: valid\tvalue\r\n", b"body");
        let parsed = parse(&bytes).unwrap();assert_eq!(&bytes[parsed.body], b"body");
    }
    #[test] fn duplicates_conflicting_lengths_and_any_transfer_encoding_rejected() {
        for duplicate in ["4", "5"] {
            assert_eq!(parse(&message(&format!("Content-Length: 4\r\ncontent-length: {duplicate}\r\nContent-Type: text/plain\r\n"), b"body")), Err(Error::DuplicateContentLength));
        }
        for encoding in ["", "identity", "chunked", "gzip, chunked"] {
            let bytes = message(&format!("Content-Length: 4\r\nContent-Type: text/plain\r\nTransfer-Encoding: {encoding}\r\n"), b"body");
            assert_eq!(parse(&bytes), Err(Error::TransferEncoding));
        }
    }
    #[test] fn invalid_decimal_or_overflow_never_becomes_a_body_range() {
        for value in ["", "-1", "+4", "4,4", "4 4", "0x4", "4a", "999999999999999999999999999999999999999999999999"] {
            assert_eq!(parse(&message(&format!("Content-Length: {value}\r\nContent-Type: text/plain\r\n"), b"body")), Err(Error::InvalidContentLength), "{value}");
        }
        assert_eq!(parse(&message("Content-Length: 2049\r\nContent-Type: text/plain\r\n", b"")), Err(Error::BodyLimit));
        assert_eq!(parse(&message("Content-Type: text/plain\r\n", b"")), Err(Error::MissingContentLength));
    }
    #[test] fn malformed_crlf_controls_names_and_obs_fold_rejected() {
        for bytes in [
            b"HTTP/1.1 200 OK\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\rX".as_slice(),
            b"HTTP/1.1 200 OK\r\n Bad: fold\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\n\tBad: fold\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length : 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad@Name: value\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad: x\0y\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad: x\x7Fy\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nBad: x\x80y\r\n\r\n".as_slice(),
        ] { assert_eq!(parse(bytes), Err(Error::Malformed)); }
    }
    #[test] fn unsupported_status_version_and_content_types_rejected() {
        for status in ["301 Moved", "302 Found", "204 Empty", "404 Missing", "500 Error"] {
            let bytes = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nContent-Type: text/plain\r\n\r\n");
            assert_eq!(parse(bytes.as_bytes()), Err(Error::UnsupportedStatus));
        }
        assert_eq!(parse(b"HTTP/1.0 200 OK\r\n\r\n"), Err(Error::UnsupportedVersion));
        for mime in ["text/html", "application/json", "text/plain; charset=utf-8", "text/plain; charset=\"us-ascii\"", "text/plain; charset=us-ascii; x=y", "text/plain;"] {
            assert_eq!(parse(&message(&format!("Content-Length: 0\r\nContent-Type: {mime}\r\n"), b"")), Err(Error::UnsupportedContentType));
        }
        assert_eq!(parse(&message("Content-Length: 0\r\n", b"")), Err(Error::MissingContentType));
        assert_eq!(parse(&message("Content-Length: 0\r\nContent-Type: text/plain\r\nContent-Type: text/plain\r\n", b"")), Err(Error::UnsupportedContentType));
        assert_eq!(parse(&message("Content-Length: 0\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\n", b"")), Err(Error::UnsupportedEncoding));
    }
    #[test] fn maximum_body_and_exact_header_limit_are_accepted_but_excess_is_rejected() {
        let body = vec![0xA5; MAX_BODY];let bytes = response(&body);let parsed = parse(&bytes).unwrap();assert_eq!(&bytes[parsed.body], body);
        let base = "Content-Length: 0\r\nContent-Type: text/plain\r\nX-Pad: ";
        let overhead = message(base, b"").len()+2;
        let headers = format!("{base}{}\r\n", "a".repeat(MAX_HEADERS-overhead));
        let bytes = message(&headers, b"");assert_eq!(bytes.len(), MAX_HEADERS);assert!(parse(&bytes).is_ok());
        let bytes = message(&format!("{base}{}\r\n", "a".repeat(MAX_HEADERS-overhead+1)), b"");assert_eq!(parse(&bytes), Err(Error::HeaderLimit));
        assert_eq!(parse(&vec![b'A'; MAX_RESPONSE+1]), Err(Error::ResponseLimit));
    }
    #[test] fn trailing_data_and_body_length_mismatch_rejected() {
        let mut bytes = response(b"body");bytes.push(b'!');assert_eq!(parse(&bytes), Err(Error::TrailingData));
        let mut bytes = response(b"body");bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\n\r\n");assert_eq!(parse(&bytes), Err(Error::TrailingData));
        assert_eq!(parse(&message("Content-Length: 5\r\nContent-Type: text/plain\r\n", b"body")), Err(Error::Truncated));
        assert_eq!(parse(&message("Content-Length: 0\r\nContent-Type: text/plain\r\n", b"x")), Err(Error::TrailingData));
    }
    #[test] fn malformed_later_header_cannot_publish_an_earlier_valid_body_extent() {
        assert_eq!(parse(&message("Content-Length: 4\r\nContent-Type: text/plain\r\nBroken later header\r\n", b"body")), Err(Error::Malformed));
        let bytes = response(&[0, 0xFF, b'\r', b'\n']);let parsed = parse(&bytes).unwrap();assert_eq!(&bytes[parsed.body], &[0, 0xFF, b'\r', b'\n']);
    }
}
