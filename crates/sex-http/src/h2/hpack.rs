// SPDX-License-Identifier: GPL-3.0-only
//! HPACK (RFC 7541): header compression for HTTP/2. The decoder keeps the dynamic
//! table the peer builds; the encoder never adds to ours (every literal is "without
//! indexing"), which is always allowed and keeps the peer's memory use at zero.

use super::huffman_table::CODES;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    Integer,
    Huffman,
    Index,
    TableSize,
    TooLarge,
}

/// RFC 7541 Appendix A.
pub(crate) const STATIC: [(&[u8], &[u8]); 61] = [
    (b":authority", b""), (b":method", b"GET"), (b":method", b"POST"), (b":path", b"/"),
    (b":path", b"/index.html"), (b":scheme", b"http"), (b":scheme", b"https"), (b":status", b"200"),
    (b":status", b"204"), (b":status", b"206"), (b":status", b"304"), (b":status", b"400"),
    (b":status", b"404"), (b":status", b"500"), (b"accept-charset", b""), (b"accept-encoding", b"gzip, deflate"),
    (b"accept-language", b""), (b"accept-ranges", b""), (b"accept", b""), (b"access-control-allow-origin", b""),
    (b"age", b""), (b"allow", b""), (b"authorization", b""), (b"cache-control", b""),
    (b"content-disposition", b""), (b"content-encoding", b""), (b"content-language", b""), (b"content-length", b""),
    (b"content-location", b""), (b"content-range", b""), (b"content-type", b""), (b"cookie", b""),
    (b"date", b""), (b"etag", b""), (b"expect", b""), (b"expires", b""),
    (b"from", b""), (b"host", b""), (b"if-match", b""), (b"if-modified-since", b""),
    (b"if-none-match", b""), (b"if-range", b""), (b"if-unmodified-since", b""), (b"last-modified", b""),
    (b"link", b""), (b"location", b""), (b"max-forwards", b""), (b"proxy-authenticate", b""),
    (b"proxy-authorization", b""), (b"range", b""), (b"referer", b""), (b"refresh", b""),
    (b"retry-after", b""), (b"server", b""), (b"set-cookie", b""), (b"strict-transport-security", b""),
    (b"transfer-encoding", b""), (b"user-agent", b""), (b"vary", b""), (b"via", b""),
    (b"www-authenticate", b""),
];

/// A header: lower-case name and value bytes.
pub type Header = (Vec<u8>, Vec<u8>);

pub fn encode_integer(out: &mut Vec<u8>, first: u8, prefix: u8, mut value: usize) {
    let max = (1usize << prefix) - 1;
    if value < max { out.push(first | value as u8); return; }
    out.push(first | max as u8);
    value -= max;
    while value >= 128 { out.push((value % 128) as u8 | 0x80); value /= 128; }
    out.push(value as u8);
}

fn decode_integer(input: &[u8], at: &mut usize, prefix: u8) -> Result<usize, Error> {
    let max = (1usize << prefix) - 1;
    let mut value = (*input.get(*at).ok_or(Error::Truncated)? as usize) & max;
    *at += 1;
    if value < max { return Ok(value); }
    let mut shift = 0u32;
    loop {
        let b = *input.get(*at).ok_or(Error::Truncated)?;
        *at += 1;
        if shift > 28 { return Err(Error::Integer); }
        value = value.checked_add(((b & 0x7f) as usize) << shift).ok_or(Error::Integer)?;
        shift += 7;
        if b & 0x80 == 0 { return Ok(value); }
    }
}

pub fn huffman_encode(out: &mut Vec<u8>, bytes: &[u8]) {
    let (mut acc, mut bits) = (0u64, 0u32);
    for b in bytes {
        let (code, len) = CODES[*b as usize];
        acc = (acc << len) | code as u64;
        bits += len as u32;
        while bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); }
    }
    // Padding is the high bits of EOS: all ones.
    if bits > 0 { out.push(((acc << (8 - bits)) | (0xff >> bits)) as u8); }
}

fn huffman_len(bytes: &[u8]) -> usize { (bytes.iter().map(|b| CODES[*b as usize].1 as usize).sum::<usize>() + 7) / 8 }

pub fn huffman_decode(input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u64, 0u32);
    for b in input {
        acc = (acc << 8) | *b as u64;
        bits += 8;
        // Codes are 5 to 30 bits: match the shortest code that fits the next bits.
        'next: while bits >= 5 {
            for len in 5..=30u32.min(bits) {
                let code = ((acc >> (bits - len)) & ((1u64 << len) - 1)) as u32;
                if let Some(sym) = lookup(code, len as u8) {
                    if sym == 256 { return Err(Error::Huffman); }
                    if out.len() >= limit { return Err(Error::TooLarge); }
                    out.push(sym as u8);
                    bits -= len;
                    continue 'next;
                }
            }
            if bits >= 30 { return Err(Error::Huffman); }
            break;
        }
        acc &= (1u64 << bits) - 1;
    }
    // At most 7 bits of padding, all ones (section 5.2).
    if bits > 7 || acc != (1u64 << bits) - 1 { return Err(Error::Huffman); }
    Ok(out)
}

fn lookup(code: u32, len: u8) -> Option<u16> {
    // Codes of one length are consecutive in canonical order; a small table per call
    // would cost more than this scan of 257 entries for short header values.
    CODES.iter().position(|(c, l)| *l == len && *c == code).map(|i| i as u16)
}

fn read_string(input: &[u8], at: &mut usize, limit: usize) -> Result<Vec<u8>, Error> {
    let huffman = *input.get(*at).ok_or(Error::Truncated)? & 0x80 != 0;
    let len = decode_integer(input, at, 7)?;
    let end = at.checked_add(len).ok_or(Error::Truncated)?;
    let raw = input.get(*at..end).ok_or(Error::Truncated)?;
    *at = end;
    if huffman { huffman_decode(raw, limit) } else if raw.len() > limit { Err(Error::TooLarge) } else { Ok(raw.to_vec()) }
}

fn write_string(out: &mut Vec<u8>, bytes: &[u8]) {
    let h = huffman_len(bytes);
    if h < bytes.len() { encode_integer(out, 0x80, 7, h); huffman_encode(out, bytes); } else { encode_integer(out, 0, 7, bytes.len()); out.extend_from_slice(bytes); }
}

/// Entry overhead in the size accounting (section 4.1).
const ENTRY_OVERHEAD: usize = 32;

pub struct Decoder {
    table: VecDeque<Header>,
    size: usize,
    /// The most the peer may use: what our SETTINGS_HEADER_TABLE_SIZE allows.
    max: usize,
    current_max: usize,
    /// The largest decoded header list (names + values + 32 each).
    pub list_limit: usize,
}

impl Decoder {
    pub fn new(max: usize, list_limit: usize) -> Self { Self { table: VecDeque::new(), size: 0, max, current_max: max, list_limit } }
    fn evict(&mut self) {
        while self.size > self.current_max {
            let Some((n, v)) = self.table.pop_back() else { break };
            self.size -= n.len() + v.len() + ENTRY_OVERHEAD;
        }
    }
    fn get(&self, index: usize) -> Result<(&[u8], &[u8]), Error> {
        if index == 0 { return Err(Error::Index); }
        if index <= STATIC.len() { let (n, v) = STATIC[index - 1]; return Ok((n, v)); }
        let (n, v) = self.table.get(index - STATIC.len() - 1).ok_or(Error::Index)?;
        Ok((n, v))
    }
    /// Decodes one header block.
    pub fn decode(&mut self, input: &[u8]) -> Result<Vec<Header>, Error> {
        let mut out = Vec::new();
        let (mut at, mut total, mut first) = (0, 0usize, true);
        while at < input.len() {
            let b = input[at];
            let header = if b & 0x80 != 0 {
                let i = decode_integer(input, &mut at, 7)?;
                let (n, v) = self.get(i)?;
                (n.to_vec(), v.to_vec())
            } else if b & 0xe0 == 0x20 {
                // Size updates come only at the start of a block (section 4.2).
                if !first { return Err(Error::TableSize); }
                let size = decode_integer(input, &mut at, 5)?;
                if size > self.max { return Err(Error::TableSize); }
                self.current_max = size;
                self.evict();
                continue;
            } else {
                let (prefix, index) = if b & 0x40 != 0 { (6, true) } else { (4, false) };
                let i = decode_integer(input, &mut at, prefix)?;
                let name = if i == 0 { read_string(input, &mut at, self.list_limit)? } else { self.get(i)?.0.to_vec() };
                let value = read_string(input, &mut at, self.list_limit)?;
                if index {
                    let size = name.len() + value.len() + ENTRY_OVERHEAD;
                    self.table.push_front((name.clone(), value.clone()));
                    self.size += size;
                    self.evict();
                }
                (name, value)
            };
            first = false;
            total += header.0.len() + header.1.len() + ENTRY_OVERHEAD;
            if total > self.list_limit { return Err(Error::TooLarge); }
            out.push(header);
        }
        Ok(out)
    }
}

/// Encodes one header block. Uses the static table where it can, literals otherwise.
pub fn encode(out: &mut Vec<u8>, headers: &[(&[u8], &[u8])]) {
    for (name, value) in headers {
        if let Some(i) = STATIC.iter().position(|(n, v)| n == name && v == value) {
            encode_integer(out, 0x80, 7, i + 1);
        } else if let Some(i) = STATIC.iter().position(|(n, _)| n == name) {
            encode_integer(out, 0, 4, i + 1);
            write_string(out, value);
        } else {
            out.push(0);
            write_string(out, name);
            write_string(out, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> Vec<u8> { let s: alloc::string::String = s.split_whitespace().collect(); (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
    #[test]
    fn rfc7541_c4_requests_with_huffman_and_the_dynamic_table() {
        let mut d = Decoder::new(4096, 65536);
        let h = d.decode(&hex("8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff")).unwrap();
        assert_eq!(h[3], (b":authority".to_vec(), b"www.example.com".to_vec()));
        let h = d.decode(&hex("8286 84be 5886 a8eb 1064 9cbf")).unwrap();
        assert_eq!(h[3], (b":authority".to_vec(), b"www.example.com".to_vec()));
        assert_eq!(h[4], (b"cache-control".to_vec(), b"no-cache".to_vec()));
        let h = d.decode(&hex("8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf")).unwrap();
        assert_eq!(h[4], (b"custom-key".to_vec(), b"custom-value".to_vec()));
        assert_eq!(d.size, 164);
    }
    #[test]
    fn rfc7541_c6_responses_evict_from_a_small_table() {
        let mut d = Decoder::new(256, 65536);
        d.decode(&hex("4882 6402 5885 aec3 771a 4b61 96d0 7abe 9410 54d4 44a8 2005 9504 0b81 66e0 82a6 2d1b ff6e 919d 29ad 1718 63c7 8f0b 97c8 e9ae 82ae 43d3")).unwrap();
        d.decode(&hex("4883 640e ffc1 c0bf")).unwrap();
        let h = d.decode(&hex("88c1 6196 d07a be94 1054 d444 a820 0595 040b 8166 e084 a62d 1bff c05a 839b d9ab 77ad 94e7 821d d7f2 e6c7 b335 dfdf cd5b 3960 d5af 2708 7f36 72c1 ab27 0fb5 291f 9587 3160 65c0 03ed 4ee5 b106 3d50 07")).unwrap();
        assert_eq!(h[0], (b":status".to_vec(), b"200".to_vec()));
        assert_eq!(h.last().unwrap().1, b"foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1".to_vec());
        assert_eq!(d.size, 215);
    }
    #[test]
    fn huffman_round_trips_every_octet_and_rejects_bad_padding() {
        let all: Vec<u8> = (0..=255u8).collect();
        let mut e = Vec::new();
        huffman_encode(&mut e, &all);
        assert_eq!(huffman_decode(&e, 1024).unwrap(), all);
        assert_eq!(huffman_decode(&[0x00], 16), Err(Error::Huffman), "zero padding");
        assert_eq!(huffman_decode(&[0xff, 0xff, 0xff, 0xff], 16), Err(Error::Huffman), "EOS");
    }
    #[test]
    fn encoded_blocks_decode_to_the_same_headers_and_limits_hold() {
        let mut out = Vec::new();
        encode(&mut out, &[(b":method", b"GET"), (b":path", b"/a/b?c=d"), (b"user-agent", b"SexOS"), (b"x-thing", b"Value")]);
        let h = Decoder::new(4096, 65536).decode(&out).unwrap();
        assert_eq!(h.iter().map(|(n, v)| (n.as_slice(), v.as_slice())).collect::<Vec<_>>(),
            [(&b":method"[..], &b"GET"[..]), (b":path", b"/a/b?c=d"), (b"user-agent", b"SexOS"), (b"x-thing", b"Value")]);
        assert_eq!(Decoder::new(4096, 40).decode(&out), Err(Error::TooLarge));
        assert_eq!(Decoder::new(4096, 65536).decode(&[0x3f, 0xe2, 0x1f]), Err(Error::TableSize), "beyond our setting");
        assert_eq!(Decoder::new(4096, 65536).decode(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]), Err(Error::Integer));
    }
}
