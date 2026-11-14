// SPDX-License-Identifier: GPL-3.0-only
//! Files as they are on disk versus text as the editor holds it. The editor always
//! holds UTF-8 with `\n` line ends; a file can be UTF-8 (with or without a byte-order
//! mark), UTF-16 (either byte order, with its mark), or Windows-1252/Latin-1 when it
//! is not valid UTF-8, and its lines can end in `\n` or `\r\n`. Saving writes it back
//! the way it came, unless the person chose otherwise.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding { Utf8, Utf8Bom, Utf16Le, Utf16Be, Windows1252 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol { Lf, CrLf }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format { pub encoding: Encoding, pub eol: Eol }

impl Default for Format { fn default() -> Self { Format { encoding: Encoding::Utf8, eol: Eol::Lf } } }

impl Encoding {
    pub fn name(self) -> &'static str {
        match self { Encoding::Utf8 => "UTF-8", Encoding::Utf8Bom => "UTF-8 BOM", Encoding::Utf16Le => "UTF-16 LE", Encoding::Utf16Be => "UTF-16 BE", Encoding::Windows1252 => "Windows-1252" }
    }
    /// The next choice when the person cycles through encodings.
    pub fn next(self) -> Encoding {
        match self { Encoding::Utf8 => Encoding::Utf8Bom, Encoding::Utf8Bom => Encoding::Utf16Le, Encoding::Utf16Le => Encoding::Utf16Be, Encoding::Utf16Be => Encoding::Windows1252, Encoding::Windows1252 => Encoding::Utf8 }
    }
    pub fn to_byte(self) -> u8 { self as u8 }
    pub fn from_byte(b: u8) -> Encoding {
        match b { 1 => Encoding::Utf8Bom, 2 => Encoding::Utf16Le, 3 => Encoding::Utf16Be, 4 => Encoding::Windows1252, _ => Encoding::Utf8 }
    }
}
impl Eol {
    pub fn name(self) -> &'static str { match self { Eol::Lf => "LF", Eol::CrLf => "CRLF" } }
}

/// Windows-1252's 0x80..0x9F; the rest of the byte range is Latin-1.
const CP1252: [u16; 32] = [
    0x20AC, 0xFFFD, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0xFFFD, 0x017D, 0xFFFD,
    0xFFFD, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0xFFFD, 0x017E, 0x0178,
];

fn push_char(out: &mut Vec<u8>, c: char) { let mut b = [0u8; 4]; out.extend_from_slice(c.encode_utf8(&mut b).as_bytes()); }

/// Reads a file's bytes into editor text, saying how it was stored.
pub fn decode(bytes: &[u8]) -> (Vec<u8>, Format) {
    let (text, encoding) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (rest.to_vec(), Encoding::Utf8Bom)
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units = bytes[2..].chunks_exact(2).map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) });
        let mut out = Vec::with_capacity(bytes.len());
        for c in char::decode_utf16(units) { push_char(&mut out, c.unwrap_or(char::REPLACEMENT_CHARACTER)); }
        (out, if le { Encoding::Utf16Le } else { Encoding::Utf16Be })
    } else if core::str::from_utf8(bytes).is_ok() {
        (bytes.to_vec(), Encoding::Utf8)
    } else {
        let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 8);
        for &b in bytes {
            let c = match b { 0x80..=0x9F => char::from_u32(CP1252[(b - 0x80) as usize] as u32).unwrap_or('\u{FFFD}'), _ => b as char };
            push_char(&mut out, c);
        }
        (out, Encoding::Windows1252)
    };
    // The file's line ends are whatever most of its lines use.
    let crlf = text.windows(2).filter(|w| w == b"\r\n").count();
    let lf = text.iter().filter(|b| **b == b'\n').count();
    let eol = if crlf > 0 && crlf * 2 >= lf { Eol::CrLf } else { Eol::Lf };
    let text = if crlf > 0 { strip_cr(&text) } else { text };
    (text, Format { encoding, eol })
}

fn strip_cr(t: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if t[i] == b'\r' && t.get(i + 1) == Some(&b'\n') { i += 1; continue; }
        out.push(t[i]);
        i += 1;
    }
    out
}

/// Editor text to file bytes in `f`. Characters Windows-1252 cannot hold become `?`;
/// the count of those comes back so the editor can warn.
pub fn encode(text: &[u8], f: Format) -> (Vec<u8>, usize) {
    let mut lined = Vec::with_capacity(text.len() + text.len() / 32);
    for &b in text { if b == b'\n' && f.eol == Eol::CrLf { lined.push(b'\r'); } lined.push(b); }
    let s = core::str::from_utf8(&lined).ok();
    let mut lost = 0;
    let out = match f.encoding {
        Encoding::Utf8 => lined,
        Encoding::Utf8Bom => { let mut v = alloc::vec![0xEF, 0xBB, 0xBF]; v.extend_from_slice(&lined); v }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let le = f.encoding == Encoding::Utf16Le;
            let mut v = if le { alloc::vec![0xFF, 0xFE] } else { alloc::vec![0xFE, 0xFF] };
            for u in s.unwrap_or("").encode_utf16() { v.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() }); }
            v
        }
        Encoding::Windows1252 => {
            let mut v = Vec::with_capacity(lined.len());
            for c in s.unwrap_or("").chars() {
                let u = c as u32;
                let b = if u < 0x80 || (0xA0..=0xFF).contains(&u) { Some(u as u8) } else { CP1252.iter().position(|x| *x as u32 == u && u != 0xFFFD).map(|i| 0x80 + i as u8) };
                match b { Some(b) => v.push(b), None => { v.push(b'?'); lost += 1; } }
            }
            v
        }
    };
    (out, lost)
}
