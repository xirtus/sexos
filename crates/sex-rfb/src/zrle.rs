// SPDX-License-Identifier: GPL-3.0-only
//! ZRLE's tiles (RFC 6143 section 7.7.6), after and before zlib. With
//! [`PixelFormat::RGB32`](crate::PixelFormat::RGB32) a CPIXEL is the low three bytes:
//! blue, green, red.
use crate::{Error, Framebuffer, Reader};
use alloc::vec::Vec;

const TILE: usize = 64;
const BAD: Error = Error::Protocol("ZRLE tile");

fn cpixel(r: &mut Reader) -> Result<u32, Error> {
    let b = r.bytes(3).ok_or(BAD)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
}

fn run_length(r: &mut Reader) -> Result<usize, Error> {
    let mut n = 1;
    loop {
        let b = r.u8().ok_or(BAD)?;
        n += b as usize;
        if b != 255 { return Ok(n); }
        if n > TILE * TILE { return Err(BAD); }
    }
}

fn tiles(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize, usize, usize)> {
    (0..h).step_by(TILE).flat_map(move |ty| (0..w).step_by(TILE).map(move |tx| (x + tx, y + ty, TILE.min(w - tx), TILE.min(h - ty))))
}

/// Paints the decompressed tile data of a `w` by `h` rectangle at `(x, y)`.
pub fn decode(data: &[u8], fb: &mut Framebuffer, x: usize, y: usize, w: usize, h: usize) -> Result<(), Error> {
    let mut r = Reader::new(data);
    let mut palette = [0u32; 128];
    for (tx, ty, tw, th) in tiles(x, y, w, h) {
        let total = tw * th;
        let put = |i: usize, c: u32, fb: &mut Framebuffer| fb.pixels[(ty + i / tw) * fb.width + tx + i % tw] = c;
        let sub = r.u8().ok_or(BAD)?;
        match sub {
            0 => for i in 0..total { let c = cpixel(&mut r)?; put(i, c, fb) },
            1 => { let c = cpixel(&mut r)?; fb.fill(tx, ty, tw, th, c) }
            2..=16 => {
                let n = sub as usize;
                for p in palette.iter_mut().take(n) { *p = cpixel(&mut r)?; }
                let bits = match n { 2 => 1, 3..=4 => 2, _ => 4 };
                for row in 0..th {
                    let packed = r.bytes((tw * bits).div_ceil(8)).ok_or(BAD)?;
                    for col in 0..tw {
                        let bit = col * bits;
                        let index = (packed[bit / 8] >> (8 - bits - bit % 8)) as usize & ((1 << bits) - 1);
                        if index >= n { return Err(BAD); }
                        put(row * tw + col, palette[index], fb);
                    }
                }
            }
            128 | 130..=255 => {
                let n = if sub == 128 { 0 } else { sub as usize - 128 };
                for p in palette.iter_mut().take(n) { *p = cpixel(&mut r)?; }
                let mut i = 0;
                while i < total {
                    let (c, len) = if n == 0 {
                        (cpixel(&mut r)?, run_length(&mut r)?)
                    } else {
                        let index = r.u8().ok_or(BAD)?;
                        let c = *palette[..n].get(index as usize & 127).ok_or(BAD)?;
                        (c, if index & 128 != 0 { run_length(&mut r)? } else { 1 })
                    };
                    if i + len > total { return Err(BAD); }
                    for k in i..i + len { put(k, c, fb); }
                    i += len;
                }
            }
            _ => return Err(BAD),
        }
    }
    if r.at != data.len() { return Err(Error::Protocol("ZRLE data after the last tile")); }
    Ok(())
}

fn push_cpixel(out: &mut Vec<u8>, c: u32) { out.extend_from_slice(&c.to_le_bytes()[..3]); }

/// The tile data (before zlib) for the `w` by `h` block of `src` (rows `stride` pixels
/// apart) at `(x, y)`: solid tiles as solid, up to 16 colours as a packed palette,
/// the rest raw.
pub fn encode(src: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize, out: &mut Vec<u8>) {
    for (tx, ty, tw, th) in tiles(x, y, w, h) {
        let pixel = |i: usize| src[(ty + i / tw) * stride + tx + i % tw] & 0x00FF_FFFF;
        let mut palette: Vec<u32> = Vec::new();
        for i in 0..tw * th {
            let c = pixel(i);
            if !palette.contains(&c) {
                palette.push(c);
                if palette.len() > 16 { break; }
            }
        }
        match palette.len() {
            1 => { out.push(1); push_cpixel(out, palette[0]) }
            n @ 2..=16 => {
                out.push(n as u8);
                for &c in &palette { push_cpixel(out, c); }
                let bits = match n { 2 => 1, 3..=4 => 2, _ => 4 };
                for row in 0..th {
                    let mut packed = alloc::vec![0u8; (tw * bits).div_ceil(8)];
                    for col in 0..tw {
                        let index = palette.iter().position(|&c| c == pixel(row * tw + col)).unwrap() as u8;
                        let bit = col * bits;
                        packed[bit / 8] |= index << (8 - bits - bit % 8);
                    }
                    out.extend_from_slice(&packed);
                }
            }
            _ => { out.push(0); for i in 0..tw * th { push_cpixel(out, pixel(i)); } }
        }
    }
}
