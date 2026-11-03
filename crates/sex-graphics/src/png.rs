//! Allocation-free PNG decoder for small owned assets.
//!
//! Independently implemented from the PNG specification (ISO/IEC 15948,
//! W3C PNG 2nd edition) and RFC 1950/1951 (zlib/DEFLATE). Supported: every
//! standard colour type and bit depth (grayscale 1/2/4/8/16, palette 1/2/4/8
//! with tRNS alpha, RGB/gray+alpha/RGBA 8/16), non-interlaced or Adam7, up
//! to the same `MAX_DIMENSION` as the QOI decoder. 16-bit samples keep their
//! high byte. Every
//! chunk CRC and the zlib Adler-32 are checked; anything else is refused.
//! Pixels are unpremultiplied `0xAARRGGBB`, like `qoi`.
//!
//! The caller supplies the scratch buffer for the inflated scanlines, so the
//! decoder itself holds no allocation and no large stack array. DEFLATE back
//! references read from that same buffer: the whole image fits in it, so no
//! separate window is needed.

use crate::qoi::{MAX_DIMENSION, MAX_INPUT_BYTES, MAX_PIXELS};

/// Inflated bytes for the largest accepted image: one filter byte per row.
/// 16-bit RGBA rows plus one filter byte per row for each of the seven
/// Adam7 passes.
pub const SCRATCH_BYTES: usize = MAX_DIMENSION * (MAX_DIMENSION * 8 + 1) + 7 * MAX_DIMENSION;
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Signature,
    InputTooLarge,
    Truncated,
    Crc,
    Header,
    Unsupported,
    Palette,
    Zlib,
    Deflate,
    Adler,
    Size,
    Filter,
}

pub fn is_png(input: &[u8]) -> bool { input.starts_with(&SIGNATURE) }

fn be32(b: &[u8]) -> u32 { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) }
fn be16(b: &[u8]) -> u32 { u16::from_be_bytes([b[0], b[1]]) as u32 }

fn crc32(parts: [&[u8]; 2]) -> u32 {
    let mut c = !0u32;
    for part in parts {
        for &b in part {
            c ^= b as u32;
            for _ in 0..8 { c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 }; }
        }
    }
    !c
}

/// Decode `input` into `out` (row-major, `width * height` pixels used).
/// Returns `(width, height)`. On error `out` and `scratch` are erased.
pub fn decode(input: &[u8], scratch: &mut [u8; SCRATCH_BYTES], out: &mut [u32; MAX_PIXELS]) -> Result<(usize, usize), Error> {
    let result = decode_inner(input, scratch, out);
    if result.is_err() {
        for b in scratch.iter_mut() { unsafe { core::ptr::write_volatile(b, 0); } }
        for p in out.iter_mut() { unsafe { core::ptr::write_volatile(p, 0); } }
    }
    result
}

fn decode_inner(input: &[u8], scratch: &mut [u8; SCRATCH_BYTES], out: &mut [u32; MAX_PIXELS]) -> Result<(usize, usize), Error> {
    if input.len() > MAX_INPUT_BYTES { return Err(Error::InputTooLarge); }
    if !is_png(input) { return Err(Error::Signature); }
    let mut at = 8;
    let mut header: Option<(usize, usize, u8, u8, bool)> = None;
    let mut palette = [0u32; 256];
    let mut palette_len = 0usize;
    let mut key: Option<[u32; 3]> = None;
    // IDAT data may be split across chunks; they are read as one stream.
    let mut idat = Chunks { input, at: 0, left: 0, started: false };
    let mut ended = false;
    let mut last_idat = false;
    while at < input.len() {
        if input.len() - at < 12 { return Err(Error::Truncated); }
        let len = be32(&input[at..]) as usize;
        let kind = &input[at + 4..at + 8];
        let end = at.checked_add(12).and_then(|v| v.checked_add(len)).ok_or(Error::Truncated)?;
        if end > input.len() { return Err(Error::Truncated); }
        let data = &input[at + 8..at + 8 + len];
        if crc32([kind, data]) != be32(&input[at + 8 + len..]) { return Err(Error::Crc); }
        match kind {
            b"IHDR" => {
                if header.is_some() || at != 8 || len != 13 { return Err(Error::Header); }
                let (w, h) = (be32(data) as usize, be32(&data[4..]) as usize);
                let (depth, color, method, filter, interlace) = (data[8], data[9], data[10], data[11], data[12]);
                if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION { return Err(Error::Size); }
                let depth_ok = match color { 0 => matches!(depth, 1 | 2 | 4 | 8 | 16), 3 => matches!(depth, 1 | 2 | 4 | 8), 2 | 4 | 6 => matches!(depth, 8 | 16), _ => false };
                if !depth_ok || method != 0 || filter != 0 || interlace > 1 { return Err(Error::Unsupported); }
                header = Some((w, h, color, depth, interlace == 1));
            }
            b"PLTE" => {
                if header.is_none() || len % 3 != 0 || len == 0 || len > 768 || idat.started { return Err(Error::Palette); }
                palette_len = len / 3;
                for i in 0..palette_len {
                    palette[i] = 0xff00_0000 | (data[i * 3] as u32) << 16 | (data[i * 3 + 1] as u32) << 8 | data[i * 3 + 2] as u32;
                }
            }
            b"tRNS" => {
                let Some((_, _, color, _, _)) = header else { return Err(Error::Header) };
                if color == 3 {
                    if len > palette_len { return Err(Error::Palette); }
                    for i in 0..len { palette[i] = (palette[i] & 0x00ff_ffff) | (data[i] as u32) << 24; }
                }
                // Gray/RGB colour key: that exact sample value is transparent.
                if color == 0 && len == 2 { key = Some([be16(data), 0, 0]); }
                if color == 2 && len == 6 { key = Some([be16(data), be16(&data[2..]), be16(&data[4..])]); }
            }
            b"IDAT" => {
                if header.is_none() { return Err(Error::Header); }
                // IDAT chunks must be consecutive; the reader follows them.
                if idat.started && !last_idat { return Err(Error::Zlib); }
                if !idat.started { idat.started = true; idat.at = at + 8; idat.left = len; }
            }
            b"IEND" => { ended = true; at = end; break; }
            _ => {
                // Ancillary chunks (lowercase first letter) are skipped;
                // unknown critical chunks are refused.
                if kind[0] & 0x20 == 0 { return Err(Error::Unsupported); }
            }
        }
        last_idat = kind == b"IDAT";
        at = end;
    }
    if !ended || at != input.len() { return Err(Error::Truncated); }
    let (w, h, color, depth, interlaced) = header.ok_or(Error::Header)?;
    if color == 3 && palette_len == 0 { return Err(Error::Palette); }
    let channels = match color { 0 => 1, 2 => 3, 3 => 1, 4 => 2, _ => 4 };
    let bits = channels * depth as usize;
    // Bytes back to the corresponding byte of the previous pixel (filters).
    let bpp = (bits + 7) / 8;
    let row_bytes = |pw: usize| (pw * bits + 7) / 8;
    // (x0, y0, dx, dy) per pass: Adam7 or one full pass.
    const ADAM7: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
    let passes: &[(usize, usize, usize, usize)] = if interlaced { &ADAM7 } else { &[(0, 0, 1, 1)] };
    let size = |&(x0, y0, dx, dy): &(usize, usize, usize, usize)| ((w + dx - 1 - x0.min(w)) / dx, (h + dy - 1 - y0.min(h)) / dy);
    let mut need = 0;
    for p in passes { let (pw, ph) = size(p); if pw > 0 && ph > 0 { need += ph * (row_bytes(pw) + 1); } }
    if need > SCRATCH_BYTES { return Err(Error::Size); }
    let mut z = Bits { src: idat, bit: 0, nbits: 0 };
    // zlib header: CM=8, window <= 32K, no preset dictionary, check bits.
    let (cmf, flg) = (z.byte()?, z.byte()?);
    if cmf & 0x0f != 8 || cmf >> 4 > 7 || flg & 0x20 != 0 || ((cmf as u16) << 8 | flg as u16) % 31 != 0 { return Err(Error::Zlib); }
    let n = inflate(&mut z, &mut scratch[..need])?;
    if n != need { return Err(Error::Size); }
    z.align();
    let adler = (z.byte()? as u32) << 24 | (z.byte()? as u32) << 16 | (z.byte()? as u32) << 8 | z.byte()? as u32;
    if adler != adler32(&scratch[..need]) { return Err(Error::Adler); }
    if z.src.next().is_some() { return Err(Error::Zlib); }
    let max = (1u32 << depth.min(8)) - 1;
    let mut base = 0;
    for p in passes {
        let (pw, ph) = size(p);
        if pw == 0 || ph == 0 { continue; }
        let stride = row_bytes(pw);
        let region = &mut scratch[base..base + ph * (stride + 1)];
        unfilter(region, stride, bpp, ph)?;
        for py in 0..ph {
            let row = &region[py * (stride + 1) + 1..(py + 1) * (stride + 1)];
            for px in 0..pw {
                // Sample `c` of pixel `px`: sub-byte depths pack MSB first;
                // 16-bit samples use their high byte; low depths scale to 8 bits.
                let sample = |c: usize| -> u32 {
                    let i = px * channels + c;
                    match depth {
                        16 => row[i * 2] as u32,
                        8 => row[i] as u32,
                        d => { let bit = i * d as usize; ((row[bit / 8] >> (8 - d as usize - bit % 8)) as u32) & max }
                    }
                };
                let gray = |v: u32| if depth < 8 { v * 255 / max } else { v };
                // Full-precision sample, for the colour key.
                let raw = |c: usize| -> u32 { let i = px * channels + c; if depth == 16 { (row[i * 2] as u32) << 8 | row[i * 2 + 1] as u32 } else { sample(c) } };
                let keyed = match (color, key) { (0, Some(k)) => raw(0) == k[0], (2, Some(k)) => raw(0) == k[0] && raw(1) == k[1] && raw(2) == k[2], _ => false };
                let (x, y) = (p.0 + px * p.2, p.1 + py * p.3);
                out[y * w + x] = match color {
                    0 => (if keyed { 0 } else { 0xff00_0000 }) | gray(sample(0)) * 0x0001_0101,
                    2 => (if keyed { 0 } else { 0xff00_0000 }) | sample(0) << 16 | sample(1) << 8 | sample(2),
                    3 => { let i = sample(0) as usize; if i >= palette_len { return Err(Error::Palette); } palette[i] }
                    4 => sample(1) << 24 | sample(0) * 0x0001_0101,
                    _ => sample(3) << 24 | sample(0) << 16 | sample(1) << 8 | sample(2),
                };
            }
        }
        base += ph * (stride + 1);
    }
    Ok((w, h))
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &d in data { a = (a + d as u32) % 65521; b = (b + a) % 65521; }
    b << 16 | a
}

fn unfilter(buf: &mut [u8], stride: usize, bpp: usize, h: usize) -> Result<(), Error> {
    for y in 0..h {
        let start = y * (stride + 1);
        let kind = buf[start];
        for x in 0..stride {
            let i = start + 1 + x;
            let a = if x >= bpp { buf[i - bpp] as i16 } else { 0 };
            let b = if y > 0 { buf[i - stride - 1] as i16 } else { 0 };
            let c = if y > 0 && x >= bpp { buf[i - stride - 1 - bpp] as i16 } else { 0 };
            let add = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                }
                _ => return Err(Error::Filter),
            };
            buf[i] = buf[i].wrapping_add(add as u8);
        }
    }
    Ok(())
}

/// The IDAT payload as one byte stream across consecutive chunks.
#[derive(Clone, Copy)]
struct Chunks<'a> { input: &'a [u8], at: usize, left: usize, started: bool }
impl Chunks<'_> {
    fn next(&mut self) -> Option<u8> {
        loop {
            if self.left > 0 {
                let b = self.input[self.at];
                self.at += 1;
                self.left -= 1;
                return Some(b);
            }
            // At the end of one IDAT, continue only into a directly following IDAT.
            let crc_end = self.at + 4;
            if crc_end + 8 > self.input.len() || &self.input[crc_end + 4..crc_end + 8] != b"IDAT" { return None; }
            self.left = be32(&self.input[crc_end..]) as usize;
            self.at = crc_end + 8;
        }
    }
}

struct Bits<'a> { src: Chunks<'a>, bit: u32, nbits: u32 }
impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<(), Error> {
        while self.nbits < n {
            let b = self.src.next().ok_or(Error::Deflate)?;
            self.bit |= (b as u32) << self.nbits;
            self.nbits += 8;
        }
        Ok(())
    }
    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 { return Ok(0); }
        self.need(n)?;
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }
    fn align(&mut self) { let r = self.nbits % 8; self.bit >>= r; self.nbits -= r; }
    fn byte(&mut self) -> Result<u8, Error> { self.align(); self.bits(8).map(|v| v as u8) }
}

/// Canonical Huffman decoding table (RFC 1951 3.2.2), counts per length.
struct Huffman { counts: [u16; 16], symbols: [u16; 288] }
impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, Error> {
        let mut h = Huffman { counts: [0; 16], symbols: [0; 288] };
        for &l in lengths { h.counts[l as usize] += 1; }
        h.counts[0] = 0;
        let mut left: i32 = 1;
        for len in 1..16 { left = (left << 1) - h.counts[len] as i32; if left < 0 { return Err(Error::Deflate); } }
        let mut offs = [0u16; 16];
        for len in 1..15 { offs[len + 1] = offs[len] + h.counts[len]; }
        for (sym, &l) in lengths.iter().enumerate() { if l != 0 { h.symbols[offs[l as usize] as usize] = sym as u16; offs[l as usize] += 1; } }
        Ok(h)
    }
    fn decode(&self, z: &mut Bits) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= z.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first { return Ok(self.symbols[(index + (code - first)) as usize]); }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::Deflate)
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Inflate into `out`, which must hold the complete result. Returns bytes written.
fn inflate(z: &mut Bits, out: &mut [u8]) -> Result<usize, Error> {
    let mut n = 0usize;
    loop {
        let last = z.bits(1)?;
        match z.bits(2)? {
            0 => {
                z.align();
                let len = z.bits(16)? as usize;
                let nlen = z.bits(16)? as usize;
                if len != !nlen & 0xffff { return Err(Error::Deflate); }
                if n + len > out.len() { return Err(Error::Size); }
                for _ in 0..len { out[n] = z.bits(8)? as u8; n += 1; }
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 {
                    let mut l = [0u8; 288];
                    for (i, v) in l.iter_mut().enumerate() { *v = match i { 0..=143 => 8, 144..=255 => 9, 256..=279 => 7, _ => 8 }; }
                    (Huffman::new(&l)?, Huffman::new(&[5u8; 30])?)
                } else { dynamic(z)? };
                loop {
                    let sym = lit.decode(z)? as usize;
                    if sym < 256 {
                        if n >= out.len() { return Err(Error::Size); }
                        out[n] = sym as u8; n += 1;
                    } else if sym == 256 { break; } else {
                        let i = sym - 257;
                        if i >= 29 { return Err(Error::Deflate); }
                        let len = LEN_BASE[i] as usize + z.bits(LEN_EXTRA[i] as u32)? as usize;
                        let d = dist.decode(z)? as usize;
                        if d >= 30 { return Err(Error::Deflate); }
                        let back = DIST_BASE[d] as usize + z.bits(DIST_EXTRA[d] as u32)? as usize;
                        if back > n { return Err(Error::Deflate); }
                        if n + len > out.len() { return Err(Error::Size); }
                        for _ in 0..len { out[n] = out[n - back]; n += 1; }
                    }
                }
            }
            _ => return Err(Error::Deflate),
        }
        if last == 1 { return Ok(n); }
    }
}

fn dynamic(z: &mut Bits) -> Result<(Huffman, Huffman), Error> {
    let hlit = z.bits(5)? as usize + 257;
    let hdist = z.bits(5)? as usize + 1;
    let hclen = z.bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 { return Err(Error::Deflate); }
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut cl = [0u8; 19];
    for &o in &ORDER[..hclen] { cl[o] = z.bits(3)? as u8; }
    let clh = Huffman::new(&cl)?;
    let mut lengths = [0u8; 316];
    let mut i = 0;
    while i < hlit + hdist {
        let sym = clh.decode(z)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => { if i == 0 { return Err(Error::Deflate); } (lengths[i - 1], 3 + z.bits(2)? as usize) }
            17 => (0, 3 + z.bits(3)? as usize),
            18 => (0, 11 + z.bits(7)? as usize),
            _ => return Err(Error::Deflate),
        };
        if i + repeat > hlit + hdist { return Err(Error::Deflate); }
        for _ in 0..repeat { lengths[i] = value; i += 1; }
    }
    if lengths[256] == 0 { return Err(Error::Deflate); }
    Ok((Huffman::new(&lengths[..hlit])?, Huffman::new(&lengths[hlit..hlit + hdist])?))
}
