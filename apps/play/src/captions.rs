// SPDX-License-Identifier: GPL-3.0-only
//! Subtitles drawn over the picture: white Geist Sans with a dark outline, centred
//! near the bottom, as large as the picture allows. The text comes from a SubRip or
//! SubStation file beside the video (`film.srt` beside `film.mp4`), parsed by
//! `sex_media::subtitles`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, Window};
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_media::subtitles::Track;

/// The largest subtitle file read.
const MAX_BYTES: u64 = 4 << 20;

/// The subtitle file beside `video`, trying .srt, .ass and .ssa in place of its
/// extension; or `given` when the person named one.
pub fn load(window: &mut Window, video: &[u8], given: Option<&[u8]>) -> Option<(Vec<u8>, Track)> {
    let stem = match video.iter().rposition(|b| *b == b'.') { Some(i) => &video[..i], None => video };
    let mut tries: Vec<Vec<u8>> = Vec::new();
    if let Some(g) = given { tries.push(g.to_vec()); }
    for ext in [&b".srt"[..], b".ass", b".ssa"] { let mut p = stem.to_vec(); p.extend_from_slice(ext); tries.push(p); }
    for path in tries {
        let len = match files::size(window, &path) {
            Ok(len) => len,
            Err(e) => {
                sex_pdx::serial_record!("[play.subtitles.try] file={} err={:?}", core::str::from_utf8(&path).unwrap_or("?"), e);
                continue;
            }
        };
        sex_pdx::serial_record!("[play.subtitles.try] file={} bytes={}", core::str::from_utf8(&path).unwrap_or("?"), len);
        if len == 0 || len > MAX_BYTES { continue; }
        let mut bytes = vec![0u8; len as usize];
        let mut at = 0usize;
        while at < bytes.len() {
            match files::read_at(window, &path, at as u64, &mut bytes[at..]) { Ok(n) if n > 0 => at += n, _ => break }
        }
        bytes.truncate(at);
        let track = Track::parse(&bytes);
        if !track.cues.is_empty() { return Some((path, track)); }
    }
    None
}

pub struct Painter {
    font: Font<'static>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
    line: Vec<u8>,
    line_w: usize,
    /// The text drawn last, to skip work when it did not change (and for logs).
    pub shown: String,
}

impl Painter {
    pub fn new() -> Option<Painter> {
        Some(Painter { font: Font::new(sex_font::GEIST_SANS)?, acc: Vec::new(), glyph: Vec::new(), line: Vec::new(), line_w: 0, shown: String::new() })
    }

    /// Draws `text` (lines split by '\n') over `px`, a `w` x `h` picture.
    pub fn draw(&mut self, px: &mut [u32], w: usize, h: usize, text: &str) {
        if text.is_empty() || w < 32 || h < 32 { return; }
        // Text height about 1/16 of the picture, at least 12 px.
        let px_size = (h / 16).max(12);
        let line_h = self.line_height(px_size);
        let lines: Vec<&str> = text.split('\n').collect();
        let bottom = h - h / 20;
        let top = bottom.saturating_sub(lines.len() * line_h);
        for (k, l) in lines.iter().enumerate() {
            self.render(l, px_size);
            let lw = self.line_w;
            composite(px, w, h, &self.line, lw, line_h, (w.saturating_sub(lw)) / 2, top + k * line_h, true);
        }
    }

    /// The controls band over the bottom of the picture: a darkened strip, a progress
    /// line (`progress` in thousandths) and `text` centred in it.
    pub fn controls(&mut self, px: &mut [u32], w: usize, h: usize, text: &str, progress: u32) {
        if w < 32 || h < 32 { return; }
        let px_size = (h / 22).max(11);
        let line_h = self.line_height(px_size);
        let band = line_h + 10;
        let top = h.saturating_sub(band);
        for y in top..h {
            for p in &mut px[y * w..(y + 1) * w] {
                let c = |s: u32| ((*p >> s) & 0xFF) * 2 / 5;
                *p = (*p & 0xFF00_0000) | c(16) << 16 | c(8) << 8 | c(0);
            }
        }
        let done = w * progress.min(1000) as usize / 1000;
        for y in top..(top + 3).min(h) {
            for x in 0..w { px[y * w + x] = if x < done { 0x00E8_A33D } else { 0x0050_5050 }; }
        }
        self.render(text, px_size);
        let lw = self.line_w;
        composite(px, w, h, &self.line, lw, line_h, (w.saturating_sub(lw)) / 2, top + 6, false);
    }

    fn line_height(&self, px_size: usize) -> usize {
        let size = (px_size * 64) as u32;
        ((self.font.ascender(size) - self.font.descender(size)) / FX + 2) as usize
    }

    /// One line of coverage into `self.line` (`self.line_w` wide).
    fn render(&mut self, l: &str, px_size: usize) {
        let size = (px_size * 64) as u32;
        let asc = self.font.ascender(size);
        let line_h = self.line_height(px_size);
        let pad = 2usize;
        let width = self.layout_width(l, size);
        let lw = (width / FX as i64) as usize + 2 * pad + 2;
        self.line_w = lw;
        self.line.clear();
        self.line.resize(lw * line_h, 0);
        let mut pen = (pad as i32) * FX;
        let mut prev = None;
        for ch in l.chars() {
            let Some(g) = self.font.glyph(ch as u32) else { continue };
            if let Some(p) = prev { pen += self.font.kerning(p, g, size); }
            let adv = self.font.advance(g, size);
            let gx = (pen / FX) as i64 - pad as i64;
            let gw = (adv / FX) as usize + 2 * pad + 2;
            self.acc.resize(scratch_len(gw, line_h), 0);
            self.glyph.resize(gw * line_h, 0);
            let sub = pen - (pen / FX) * FX + pad as i32 * FX;
            self.font.draw(g, size, sub, asc + FX, gw, line_h, &mut self.acc, &mut self.glyph);
            for y in 0..line_h {
                for x in 0..gw {
                    let lx = gx + x as i64;
                    if lx < 0 || lx as usize >= lw { continue; }
                    let d = &mut self.line[y * lw + lx as usize];
                    *d = (*d).max(self.glyph[y * gw + x]);
                }
            }
            pen += adv;
            prev = Some(g);
        }
    }

    fn layout_width(&self, l: &str, size: u32) -> i64 {
        let mut prev = None;
        let mut width = 0i64;
        for ch in l.chars() {
            let Some(g) = self.font.glyph(ch as u32) else { continue };
            if let Some(p) = prev { width += self.font.kerning(p, g, size) as i64; }
            width += self.font.advance(g, size) as i64;
            prev = Some(g);
        }
        width
    }
}

/// White text, over a dark outline (the largest coverage within 2 px) when `outline`.
fn composite(px: &mut [u32], w: usize, h: usize, cov: &[u8], lw: usize, lh: usize, x0: usize, y0: usize, outline: bool) {
    for y in 0..lh {
        let py = y0 + y;
        if py >= h { break; }
        for x in 0..lw {
            let pxx = x0 + x;
            if pxx >= w { break; }
            let c = cov[y * lw + x] as u32;
            let mut o = if outline { 0u32 } else { c };
            if outline { for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                    if sx < 0 || sy < 0 || sx as usize >= lw || sy as usize >= lh { continue; }
                    o = o.max(cov[sy as usize * lw + sx as usize] as u32);
                }
            } }
            if o == 0 { continue; }
            let p = &mut px[py * w + pxx];
            let blend = |v: u32, to: u32, a: u32| (v * (255 - a) + to * a) / 255;
            let ch = |p: u32, s: u32| (p >> s) & 0xFF;
            // Outline 85% black, then the glyph in white over it.
            let oa = if outline { o * 217 / 255 } else { 0 };
            let mut rgb = [ch(*p, 16), ch(*p, 8), ch(*p, 0)];
            for v in rgb.iter_mut() { *v = blend(blend(*v, 0, oa), 255, c); }
            *p = (*p & 0xFF00_0000) | rgb[0] << 16 | rgb[1] << 8 | rgb[2];
        }
    }
}
