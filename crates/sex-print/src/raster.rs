// SPDX-License-Identifier: GPL-3.0-only
//! Raster pages for driverless printers: PWG Raster (PWG 5102.4, `image/pwg-raster`,
//! sync word `RaS2`) and Apple URF (`image/urf`, `UNIRAST`).
//!
//! Both compress a page the same way: each run of identical lines starts with a
//! line-repeat byte (count - 1), then the line as pixel runs. A control byte 0..=127
//! repeats the next pixel 1..=128 times; 129..=255 is followed by 257 - n literal
//! pixels (2..=128). (128, "white to the end of the line", is read but never written.)
use crate::caps;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// sgray_8: one byte per pixel, 255 is white.
    Gray8,
    /// srgb_8: three bytes per pixel.
    Rgb8,
}
impl Color {
    pub fn bytes_per_pixel(self) -> usize { match self { Color::Gray8 => 1, Color::Rgb8 => 3 } }
}

/// One page of pixels, rows top to bottom, at `dpi` in both directions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub color: Color,
    pub pixels: Vec<u8>,
}

impl Page {
    /// A white page.
    pub fn blank(width: u32, height: u32, dpi: u32, color: Color) -> Self {
        Page { width, height, dpi, color, pixels: alloc::vec![255; width as usize * height as usize * color.bytes_per_pixel()] }
    }
    /// A white page the size of a PWG media name, at `dpi`.
    pub fn for_media(media: &str, dpi: u32, color: Color) -> Option<Self> {
        let (w, h) = caps::media_size(media)?;
        Some(Self::blank(caps::hmm_to_pixels(w, dpi), caps::hmm_to_pixels(h, dpi), dpi, color))
    }
    pub fn bytes_per_line(&self) -> usize { self.width as usize * self.color.bytes_per_pixel() }
    pub fn line(&self, y: usize) -> &[u8] { let n = self.bytes_per_line(); &self.pixels[y * n..(y + 1) * n] }
}

/// The page's lines in the shared run-length form.
pub fn compress(page: &Page) -> Vec<u8> {
    let mut out = Vec::new();
    let bpp = page.color.bytes_per_pixel();
    let mut y = 0usize;
    let h = page.height as usize;
    while y < h {
        let line = page.line(y);
        let mut repeat = 1;
        while y + repeat < h && repeat < 256 && page.line(y + repeat) == line { repeat += 1; }
        out.push((repeat - 1) as u8);
        compress_line(line, bpp, &mut out);
        y += repeat;
    }
    out
}

fn compress_line(line: &[u8], bpp: usize, out: &mut Vec<u8>) {
    let px = |i: usize| &line[i * bpp..(i + 1) * bpp];
    let n = line.len() / bpp;
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < 128 && px(i + run) == px(i) { run += 1; }
        if run > 1 || i + 1 == n {
            out.push((run - 1) as u8);
            out.extend_from_slice(px(i));
            i += run;
            continue;
        }
        // Literal pixels until two equal neighbours start a run (or 128 are gathered).
        let start = i;
        while i < n && i - start < 128 && !(i + 1 < n && px(i) == px(i + 1)) { i += 1; }
        let count = i - start;
        if count == 1 {
            out.push(0);
        } else {
            out.push((257 - count) as u8);
        }
        out.extend_from_slice(&line[start * bpp..i * bpp]);
    }
}

/// Reverses [`compress`]: `height` lines of `bytes_per_line` (white is `white`).
pub fn decompress(mut data: &[u8], bytes_per_line: usize, bpp: usize, height: usize, white: u8) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes_per_line * height);
    while out.len() < bytes_per_line * height {
        let repeat = *data.first()? as usize + 1;
        data = &data[1..];
        let mut line = Vec::with_capacity(bytes_per_line);
        while line.len() < bytes_per_line {
            let c = *data.first()?;
            data = &data[1..];
            if c == 128 {
                line.resize(bytes_per_line, white);
            } else if c > 128 {
                let n = (257 - c as usize) * bpp;
                line.extend_from_slice(data.get(..n)?);
                data = &data[n..];
            } else {
                let p = data.get(..bpp)?;
                for _ in 0..=c { line.extend_from_slice(p); }
                data = &data[bpp..];
            }
        }
        if line.len() != bytes_per_line { return None; }
        for _ in 0..repeat { out.extend_from_slice(&line); }
    }
    out.truncate(bytes_per_line * height);
    Some(out)
}

/// Job-wide settings that go into each PWG page header.
#[derive(Clone, Debug, Default)]
pub struct PwgOptions {
    /// PWG media name, e.g. `iso_a4_210x297mm` (also sets PageSize).
    pub media: String,
    pub duplex: bool,
    pub tumble: bool,
    /// print-quality: 3 draft, 4 normal, 5 high; 0 leaves it to the printer.
    pub quality: u32,
}

pub const PWG_HEADER_LEN: usize = 1796;

fn put_str(h: &mut [u8], at: usize, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(63);
    h[at..at + n].copy_from_slice(&b[..n]);
}
fn put_u32(h: &mut [u8], at: usize, v: u32) { h[at..at + 4].copy_from_slice(&v.to_be_bytes()); }

/// The 1796-byte PWG page header (field offsets from PWG 5102.4 section 4.3).
pub fn pwg_header(page: &Page, o: &PwgOptions, total_pages: u32) -> [u8; PWG_HEADER_LEN] {
    let mut h = [0u8; PWG_HEADER_LEN];
    put_str(&mut h, 0, "PwgRaster");
    put_u32(&mut h, 272, o.duplex as u32);
    put_u32(&mut h, 276, page.dpi);
    put_u32(&mut h, 280, page.dpi);
    put_u32(&mut h, 340, 1); // NumCopies: copies are a job attribute, not repeated pages.
    let (wpt, hpt) = caps::media_size(&o.media)
        .map(|(w, hh)| (caps::hmm_to_points(w), caps::hmm_to_points(hh)))
        .unwrap_or(((page.width * 72 + page.dpi / 2) / page.dpi, (page.height * 72 + page.dpi / 2) / page.dpi));
    put_u32(&mut h, 352, wpt);
    put_u32(&mut h, 356, hpt);
    put_u32(&mut h, 368, o.tumble as u32);
    put_u32(&mut h, 372, page.width);
    put_u32(&mut h, 376, page.height);
    let bpp = page.color.bytes_per_pixel() as u32;
    put_u32(&mut h, 384, 8); // BitsPerColor
    put_u32(&mut h, 388, 8 * bpp); // BitsPerPixel
    put_u32(&mut h, 392, page.width * bpp); // BytesPerLine
    put_u32(&mut h, 396, 0); // ColorOrder: chunky
    put_u32(&mut h, 400, match page.color { Color::Gray8 => 18, Color::Rgb8 => 19 }); // sGray / sRGB
    put_u32(&mut h, 420, bpp); // NumColors
    put_u32(&mut h, 452, total_pages);
    put_u32(&mut h, 456, 1); // CrossFeedTransform
    put_u32(&mut h, 460, 1); // FeedTransform
    put_u32(&mut h, 484, o.quality);
    put_str(&mut h, 1732, &o.media);
    h
}

/// A whole `image/pwg-raster` document.
pub fn pwg_document(pages: &[Page], o: &PwgOptions) -> Vec<u8> {
    let mut out = Vec::from(&b"RaS2"[..]);
    for p in pages {
        out.extend_from_slice(&pwg_header(p, o, pages.len() as u32));
        out.extend_from_slice(&compress(p));
    }
    out
}

/// A whole `image/urf` document (Apple raster, for AirPrint printers without PWG).
pub fn urf_document(pages: &[Page], duplex: bool, quality: u8) -> Vec<u8> {
    let mut out = Vec::from(&b"UNIRAST\0"[..]);
    out.extend_from_slice(&(pages.len() as u32).to_be_bytes());
    for p in pages {
        let mut h = [0u8; 32];
        h[0] = (8 * p.color.bytes_per_pixel()) as u8;
        h[1] = match p.color { Color::Gray8 => 0, Color::Rgb8 => 1 };
        h[2] = if duplex { 2 } else { 1 };
        h[3] = if quality == 0 { 4 } else { quality };
        h[12..16].copy_from_slice(&p.width.to_be_bytes());
        h[16..20].copy_from_slice(&p.height.to_be_bytes());
        h[20..24].copy_from_slice(&p.dpi.to_be_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(&compress(p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_noise() {
        let mut p = Page::blank(97, 31, 72, Color::Rgb8);
        let mut s = 0x1234_5678u32;
        for (i, b) in p.pixels.iter_mut().enumerate() {
            s ^= s << 13; s ^= s >> 17; s ^= s << 5;
            if (i / 291) % 3 != 0 { *b = (s % 4) as u8 * 60; }
        }
        let c = compress(&p);
        assert_eq!(decompress(&c, p.bytes_per_line(), 3, 31, 255).unwrap(), p.pixels);
    }
}
