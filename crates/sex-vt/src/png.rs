// SPDX-License-Identifier: GPL-3.0-only
//! A small PNG decoder for kitty graphics `f=100`: the container, filters, Adam7 and
//! every colour type and depth, with `miniz_oxide` doing the inflating. Pixels come
//! out as unpremultiplied 0xAARRGGBB, the layout the image store keeps.
//!
//! Hostile input is bounded before allocation: the header's size is checked against
//! [`MAX_PIXELS`] and the inflated stream is limited to exactly what the header needs.

use alloc::vec;
use alloc::vec::Vec;

/// 16 Mpx (64 MiB of pixels), the image store's whole budget.
pub const MAX_PIXELS: usize = 16 * 1024 * 1024;
const MAX_SIDE: usize = 16384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngError { Signature, Header, TooLarge, Inflate, Data, Unsupported }

struct Header { w: usize, h: usize, depth: u8, ctype: u8, interlace: bool }

impl Header {
    fn channels(&self) -> usize { match self.ctype { 0 | 3 => 1, 4 => 2, 2 => 3, _ => 4 } }
    fn bits_pp(&self) -> usize { self.channels() * self.depth as usize }
    /// Bytes of one filtered row `w` pixels wide, filter byte excluded.
    fn row_bytes(&self, w: usize) -> usize { (w * self.bits_pp()).div_ceil(8) }
    fn valid(&self) -> bool {
        matches!((self.ctype, self.depth), (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) | (2 | 4 | 6, 8 | 16))
    }
}

/// Adam7 passes: (x start, y start, x step, y step).
const ADAM7: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

fn be32(b: &[u8]) -> usize { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize }

/// Decodes a PNG into (width, height, 0xAARRGGBB pixels).
pub fn decode(bytes: &[u8]) -> Result<(usize, usize, Vec<u32>), PngError> {
    if bytes.len() < 8 || bytes[..8] != *b"\x89PNG\r\n\x1a\n" { return Err(PngError::Signature); }
    let mut at = 8;
    let mut hdr: Option<Header> = None;
    let mut palette: Vec<u32> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    while at + 8 <= bytes.len() {
        let len = be32(&bytes[at..]);
        let kind = &bytes[at + 4..at + 8];
        let body = bytes.get(at + 8..at + 8 + len).ok_or(PngError::Data)?;
        at += 12 + len; // length, type, body, CRC (not checked: the stream is local)
        match kind {
            b"IHDR" => {
                if body.len() < 13 { return Err(PngError::Header); }
                let h = Header { w: be32(body), h: be32(&body[4..]), depth: body[8], ctype: body[9], interlace: body[12] == 1 };
                if !h.valid() || body[10] != 0 || body[11] != 0 || body[12] > 1 { return Err(PngError::Unsupported); }
                if h.w == 0 || h.h == 0 { return Err(PngError::Header); }
                if h.w > MAX_SIDE || h.h > MAX_SIDE || h.w * h.h > MAX_PIXELS { return Err(PngError::TooLarge); }
                hdr = Some(h);
            }
            b"PLTE" => palette = body.chunks_exact(3).map(|c| 0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => { if idat.len() + body.len() > 256 * 1024 * 1024 { return Err(PngError::TooLarge); } idat.extend_from_slice(body) }
            b"IEND" => break,
            _ => {}
        }
    }
    let h = hdr.ok_or(PngError::Header)?;
    // Exactly the bytes the passes need, so a lying stream cannot inflate further.
    let passes: Vec<(usize, usize, usize, usize, usize, usize)> = if h.interlace {
        ADAM7.iter().map(|&(x0, y0, dx, dy)| (x0, y0, dx, dy, (h.w + dx - 1 - x0) / dx, (h.h + dy - 1 - y0) / dy)).filter(|p| p.4 > 0 && p.5 > 0).collect()
    } else { vec![(0, 0, 1, 1, h.w, h.h)] };
    let need: usize = passes.iter().map(|p| (h.row_bytes(p.4) + 1) * p.5).sum();
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, need).map_err(|_| PngError::Inflate)?;
    if raw.len() < need { return Err(PngError::Data); }
    drop(idat);

    let mut out: Vec<u32> = Vec::new();
    out.try_reserve_exact(h.w * h.h).map_err(|_| PngError::TooLarge)?;
    out.resize(h.w * h.h, 0);
    let bpp = h.bits_pp().div_ceil(8).max(1);
    let mut at = 0;
    for &(x0, y0, dx, dy, pw, ph) in &passes {
        let rb = h.row_bytes(pw);
        let mut prev = vec![0u8; rb];
        let mut cur = vec![0u8; rb];
        for py in 0..ph {
            let filter = raw[at];
            cur.copy_from_slice(&raw[at + 1..at + 1 + rb]);
            at += rb + 1;
            unfilter(filter, bpp, &mut cur, &prev)?;
            let y = y0 + py * dy;
            for px in 0..pw {
                out[y * h.w + x0 + px * dx] = pixel(&h, &cur, px, &palette, &trns);
            }
            core::mem::swap(&mut prev, &mut cur);
        }
    }
    Ok((h.w, h.h, out))
}

fn unfilter(filter: u8, bpp: usize, cur: &mut [u8], prev: &[u8]) -> Result<(), PngError> {
    match filter {
        0 => {}
        1 => for i in bpp..cur.len() { cur[i] = cur[i].wrapping_add(cur[i - bpp]); },
        2 => for i in 0..cur.len() { cur[i] = cur[i].wrapping_add(prev[i]); },
        3 => for i in 0..cur.len() {
            let a = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
            cur[i] = cur[i].wrapping_add(((a + prev[i] as u16) / 2) as u8);
        },
        4 => for i in 0..cur.len() {
            let (a, b, c) = if i >= bpp { (cur[i - bpp], prev[i], prev[i - bpp]) } else { (0, prev[i], 0) };
            cur[i] = cur[i].wrapping_add(paeth(a, b, c));
        },
        _ => return Err(PngError::Data),
    }
    Ok(())
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Sample `i` of a row (in units of the bit depth), scaled to 8 bits when `scale`.
fn sample(depth: u8, row: &[u8], i: usize) -> u16 {
    match depth {
        16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]),
        8 => row[i] as u16,
        d => {
            let bit = i * d as usize;
            let byte = row[bit / 8];
            let shift = 8 - d as usize - bit % 8;
            ((byte >> shift) & ((1u8 << d) - 1)) as u16
        }
    }
}

fn pixel(h: &Header, row: &[u8], x: usize, palette: &[u32], trns: &[u8]) -> u32 {
    let n = h.channels();
    let to8 = |v: u16| -> u32 {
        match h.depth { 16 => (v >> 8) as u32, 8 => v as u32, 4 => v as u32 * 17, 2 => v as u32 * 85, _ => v as u32 * 255 }
    };
    let s = |c: usize| sample(h.depth, row, x * n + c);
    let rgb = |r: u32, g: u32, b: u32, a: u32| a << 24 | r << 16 | g << 8 | b;
    match h.ctype {
        0 => {
            let v = s(0);
            let transparent = trns.len() >= 2 && u16::from_be_bytes([trns[0], trns[1]]) == v;
            let g = to8(v);
            rgb(g, g, g, if transparent { 0 } else { 255 })
        }
        2 => {
            let (r, g, b) = (s(0), s(1), s(2));
            let key = |i: usize| u16::from_be_bytes([trns[2 * i], trns[2 * i + 1]]);
            let transparent = trns.len() >= 6 && key(0) == r && key(1) == g && key(2) == b;
            rgb(to8(r), to8(g), to8(b), if transparent { 0 } else { 255 })
        }
        3 => {
            let i = s(0) as usize;
            let c = palette.get(i).copied().unwrap_or(0xFF00_0000);
            let a = trns.get(i).copied().unwrap_or(255) as u32;
            (c & 0x00FF_FFFF) | a << 24
        }
        4 => { let g = to8(s(0)); rgb(g, g, g, to8(s(1))) }
        _ => rgb(to8(s(0)), to8(s(1)), to8(s(2)), to8(s(3))),
    }
}

/// Encodes 0xAARRGGBB pixels as an RGBA PNG with stored (uncompressed) deflate blocks:
/// enough for fixtures, and it links no compressor.
pub fn encode_rgba(w: usize, h: usize, pixels: &[u32]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for y in 0..h {
        raw.push(0);
        for &p in &pixels[y * w..(y + 1) * w] { raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]); }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A zlib stream of stored deflate blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(65535).peekable();
    if blocks.peek().is_none() { out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]); }
    while let Some(b) = blocks.next() {
        out.push(if blocks.peek().is_none() { 1 } else { 0 });
        let n = b.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(!n).to_le_bytes());
        out.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data { a = (a + x as u32) % 65521; b = (b + a) % 65521; }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 { crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 }; }
    }
    !crc
}
