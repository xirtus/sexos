// SPDX-License-Identifier: GPL-3.0-only
//! RFC 9204 QPACK static-table header encoder and decoder (zero dynamic table,
//! Required Insert Count = 0, Delta Base = 0) for HTTP/3.

use crate::h2::hpack::{self, Header};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QpackError {
    Truncated,
    Integer,
    Huffman,
    Index,
    DynamicTableUnsupported,
    TooLarge,
}

/// Maximum total decoded header bytes per field section.
pub const MAX_FIELD_SECTION_BYTES: usize = 256 * 1024;

/// RFC 9204 Appendix A: all 99 QPACK static table entries (indices 0..=98).
pub const STATIC_TABLE: [(&[u8], &[u8]); 99] = [
    (b":authority", b""),                                                    // 0
    (b":path", b"/"),                                                        // 1
    (b"age", b"0"),                                                          // 2
    (b"content-disposition", b""),                                           // 3
    (b"content-length", b"0"),                                               // 4
    (b"cookie", b""),                                                        // 5
    (b"date", b""),                                                          // 6
    (b"etag", b""),                                                          // 7
    (b"if-modified-since", b""),                                             // 8
    (b"if-none-match", b""),                                                 // 9
    (b"last-modified", b""),                                                 // 10
    (b"link", b""),                                                          // 11
    (b"location", b""),                                                      // 12
    (b"referer", b""),                                                       // 13
    (b"set-cookie", b""),                                                    // 14
    (b":method", b"CONNECT"),                                                // 15
    (b":method", b"DELETE"),                                                 // 16
    (b":method", b"GET"),                                                    // 17
    (b":method", b"HEAD"),                                                   // 18
    (b":method", b"OPTIONS"),                                                // 19
    (b":method", b"POST"),                                                   // 20
    (b":method", b"PUT"),                                                    // 21
    (b":scheme", b"http"),                                                   // 22
    (b":scheme", b"https"),                                                  // 23
    (b":status", b"103"),                                                    // 24
    (b":status", b"200"),                                                    // 25
    (b":status", b"304"),                                                    // 26
    (b":status", b"404"),                                                    // 27
    (b":status", b"503"),                                                    // 28
    (b"accept", b"*/*"),                                                     // 29
    (b"accept", b"application/dns-message"),                                 // 30
    (b"accept-encoding", b"gzip, deflate, br"),                              // 31
    (b"accept-ranges", b"bytes"),                                            // 32
    (b"access-control-allow-headers", b"cache-control"),                     // 33
    (b"access-control-allow-headers", b"content-type"),                      // 34
    (b"access-control-allow-origin", b"*"),                                  // 35
    (b"cache-control", b"max-age=0"),                                        // 36
    (b"cache-control", b"max-age=2592000"),                                  // 37
    (b"cache-control", b"max-age=604800"),                                   // 38
    (b"cache-control", b"no-cache"),                                         // 39
    (b"cache-control", b"no-store"),                                         // 40
    (b"cache-control", b"public, max-age=31536000"),                         // 41
    (b"content-encoding", b"br"),                                            // 42
    (b"content-encoding", b"gzip"),                                          // 43
    (b"content-type", b"application/dns-message"),                           // 44
    (b"content-type", b"application/javascript"),                            // 45
    (b"content-type", b"application/json"),                                  // 46
    (b"content-type", b"application/x-www-form-urlencoded"),                 // 47
    (b"content-type", b"image/gif"),                                         // 48
    (b"content-type", b"image/jpeg"),                                        // 49
    (b"content-type", b"image/png"),                                         // 50
    (b"content-type", b"text/css"),                                          // 51
    (b"content-type", b"text/html; charset=utf-8"),                          // 52
    (b"content-type", b"text/plain"),                                        // 53
    (b"content-type", b"text/plain;charset=utf-8"),                          // 54
    (b"range", b"bytes=0-"),                                                 // 55
    (b"strict-transport-security", b"max-age=31536000"),                     // 56
    (b"strict-transport-security", b"max-age=31536000; includesubdomains"),  // 57
    (b"strict-transport-security", b"max-age=31536000; includesubdomains; preload"), // 58
    (b"vary", b"accept-encoding"),                                           // 59
    (b"vary", b"origin"),                                                    // 60
    (b"x-content-type-options", b"nosniff"),                                 // 61
    (b"x-xss-protection", b"1; mode=block"),                                 // 62
    (b":status", b"100"),                                                    // 63
    (b":status", b"204"),                                                    // 64
    (b":status", b"206"),                                                    // 65
    (b":status", b"302"),                                                    // 66
    (b":status", b"400"),                                                    // 67
    (b":status", b"403"),                                                    // 68
    (b":status", b"421"),                                                    // 69
    (b":status", b"425"),                                                    // 70
    (b":status", b"500"),                                                    // 71
    (b"accept-language", b""),                                               // 72
    (b"access-control-allow-credentials", b"FALSE"),                         // 73
    (b"access-control-allow-credentials", b"TRUE"),                          // 74
    (b"access-control-allow-headers", b"*"),                                 // 75
    (b"access-control-allow-methods", b"get"),                               // 76
    (b"access-control-allow-methods", b"get, post, options"),                // 77
    (b"access-control-allow-methods", b"options"),                           // 78
    (b"access-control-expose-headers", b"content-length"),                   // 79
    (b"access-control-request-headers", b"content-type"),                    // 80
    (b"access-control-request-method", b"get"),                              // 81
    (b"access-control-request-method", b"post"),                             // 82
    (b"alt-svc", b"clear"),                                                  // 83
    (b"authorization", b""),                                                 // 84
    (b"content-security-policy", b"script-src 'none'; object-src 'none'; base-uri 'none'"), // 85
    (b"early-data", b"1"),                                                   // 86
    (b"expect-ct", b""),                                                     // 87
    (b"forwarded", b""),                                                     // 88
    (b"if-range", b""),                                                      // 89
    (b"origin", b""),                                                        // 90
    (b"purpose", b"prefetch"),                                               // 91
    (b"server", b""),                                                        // 92
    (b"timing-allow-origin", b"*"),                                          // 93
    (b"upgrade-insecure-requests", b"1"),                                    // 94
    (b"user-agent", b""),                                                    // 95
    (b"x-forwarded-for", b""),                                               // 96
    (b"x-frame-options", b"deny"),                                           // 97
    (b"x-frame-options", b"sameorigin"),                                     // 98
];

fn decode_prefixed_int(input: &[u8], at: &mut usize, prefix: u8) -> Result<usize, QpackError> {
    let max = (1usize << prefix) - 1;
    let mut value = (*input.get(*at).ok_or(QpackError::Truncated)? as usize) & max;
    *at += 1;
    if value < max {
        return Ok(value);
    }
    let mut shift = 0u32;
    loop {
        let b = *input.get(*at).ok_or(QpackError::Truncated)?;
        *at += 1;
        if shift > 28 {
            return Err(QpackError::Integer);
        }
        value = value
            .checked_add(((b & 0x7f) as usize) << shift)
            .ok_or(QpackError::Integer)?;
        shift += 7;
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
}

fn encode_string(out: &mut Vec<u8>, bytes: &[u8]) {
    // Plain literal (H=0) with 7-bit prefix length.
    hpack::encode_integer(out, 0x00, 7, bytes.len());
    out.extend_from_slice(bytes);
}

fn decode_string(input: &[u8], at: &mut usize, prefix_bits: u8, huffman_mask: u8, limit: usize) -> Result<Vec<u8>, QpackError> {
    let first = *input.get(*at).ok_or(QpackError::Truncated)?;
    let huffman = (first & huffman_mask) != 0;
    let len = decode_prefixed_int(input, at, prefix_bits)?;
    let raw = input.get(*at..*at + len).ok_or(QpackError::Truncated)?;
    *at += len;
    if huffman {
        hpack::huffman_decode(raw, limit).map_err(|e| match e {
            hpack::Error::TooLarge => QpackError::TooLarge,
            _ => QpackError::Huffman,
        })
    } else {
        if raw.len() > limit {
            return Err(QpackError::TooLarge);
        }
        Ok(raw.to_vec())
    }
}

/// Encodes `headers` as a static-only QPACK field section (RFC 9204 Section 4.5).
pub fn encode_field_section(headers: &[(&[u8], &[u8])], out: &mut Vec<u8>) {
    // Encoded Field Section Prefix: Required Insert Count = 0, S = 0, Delta Base = 0.
    out.push(0x00);
    out.push(0x00);
    for &(name, value) in headers {
        if let Some(idx) = STATIC_TABLE.iter().position(|(n, v)| *n == name && *v == value) {
            // Indexed Field Line (Static, T=1): 1 1 index(6+)
            hpack::encode_integer(out, 0xC0, 6, idx);
        } else if let Some(idx) = STATIC_TABLE.iter().position(|(n, _)| *n == name) {
            // Literal Field Line With Name Reference (Static, N=0, T=1): 0 1 0 1 index(4+)
            hpack::encode_integer(out, 0x50, 4, idx);
            encode_string(out, value);
        } else {
            // Literal Field Line With Literal Name (N=0, H=0): 0 0 1 0 0 name_len(3+)
            hpack::encode_integer(out, 0x20, 3, name.len());
            out.extend_from_slice(name);
            encode_string(out, value);
        }
    }
}

/// Decodes a static-only QPACK field section (Required Insert Count = 0).
pub fn decode_field_section(input: &[u8]) -> Result<Vec<Header>, QpackError> {
    let mut at = 0usize;
    let req_insert_count = decode_prefixed_int(input, &mut at, 8)?;
    let _delta_base = decode_prefixed_int(input, &mut at, 7)?;
    if req_insert_count != 0 {
        return Err(QpackError::DynamicTableUnsupported);
    }
    let mut headers = Vec::new();
    let mut total_bytes = 0usize;
    while at < input.len() {
        let b = input[at];
        let (name, value) = if (b & 0x80) != 0 {
            // Indexed Field Line: 1 T index(6+)
            let is_static = (b & 0x40) != 0;
            if !is_static {
                return Err(QpackError::DynamicTableUnsupported);
            }
            let idx = decode_prefixed_int(input, &mut at, 6)?;
            let &(n, v) = STATIC_TABLE.get(idx).ok_or(QpackError::Index)?;
            (n.to_vec(), v.to_vec())
        } else if (b & 0x40) != 0 {
            // Literal Field Line With Name Reference: 0 1 N T index(4+)
            let is_static = (b & 0x10) != 0;
            if !is_static {
                return Err(QpackError::DynamicTableUnsupported);
            }
            let idx = decode_prefixed_int(input, &mut at, 4)?;
            let &(n, _) = STATIC_TABLE.get(idx).ok_or(QpackError::Index)?;
            let v = decode_string(input, &mut at, 7, 0x80, MAX_FIELD_SECTION_BYTES)?;
            (n.to_vec(), v)
        } else if (b & 0x20) != 0 {
            // Literal Field Line With Literal Name: 0 0 1 N H name_len(3+)
            let n = decode_string(input, &mut at, 3, 0x08, MAX_FIELD_SECTION_BYTES)?;
            let v = decode_string(input, &mut at, 7, 0x80, MAX_FIELD_SECTION_BYTES)?;
            (n, v)
        } else {
            // Post-Base Indexed (0001xxxx) or Post-Base Literal (0000xxxx) require dynamic table.
            return Err(QpackError::DynamicTableUnsupported);
        };
        total_bytes = total_bytes
            .checked_add(name.len() + value.len() + 32)
            .ok_or(QpackError::TooLarge)?;
        if total_bytes > MAX_FIELD_SECTION_BYTES {
            return Err(QpackError::TooLarge);
        }
        headers.push((name, value));
    }
    Ok(headers)
}
