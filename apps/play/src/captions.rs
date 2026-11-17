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

    /// The controls band over the bottom of the picture, in Weft's dark style: a
    /// translucent surface strip, a progress track with the accent fill and a knob
    /// (`progress` in thousandths), and `text` centred in it.
    pub fn controls(&mut self, px: &mut [u32], w: usize, h: usize, text: &str, progress: u32) {
        if w < 32 || h < 32 { return; }
        let px_size = (h / 22).max(11);
        let line_h = self.line_height(px_size);
        let band = line_h + 14;
        let top = h.saturating_sub(band);
        for y in top..h {
            for p in &mut px[y * w..(y + 1) * w] { *p = blend(*p, style::SURFACE, 210); }
        }
        // The track, inset from the sides, with the played part in the accent.
        let inset = (w / 40).max(6);
        let (x0, x1) = (inset, w.saturating_sub(inset));
        let done = x0 + (x1 - x0) * progress.min(1000) as usize / 1000;
        let ty = top + 4;
        for y in ty..(ty + 3).min(h) {
            for x in x0..x1 { px[y * w + x] = if x < done { style::ACCENT } else { style::TRACK }; }
        }
        // The knob: a small disc on the played end.
        let r = 4i64;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r { continue; }
                let (x, y) = (done as i64 + dx, ty as i64 + 1 + dy);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h { px[y as usize * w + x as usize] = style::ACCENT; }
            }
        }
        self.render(text, px_size);
        let lw = self.line_w;
        composite(px, w, h, &self.line, lw, line_h, (w.saturating_sub(lw)) / 2, top + 10, false);
    }

    /// A card in the lower right of the picture, above the controls band: `text`
    /// (lines split by '\n') on Weft's surface colour with rounded corners and an
    /// accent bar down its left side.
    pub fn card(&mut self, px: &mut [u32], w: usize, h: usize, text: &str) {
        if text.is_empty() || w < 64 || h < 48 { return; }
        let px_size = (h / 26).max(11);
        let line_h = self.line_height(px_size);
        let lines: Vec<&str> = text.split('\n').collect();
        let mut rendered: Vec<(Vec<u8>, usize)> = Vec::new();
        for l in &lines { self.render(l, px_size); rendered.push((core::mem::take(&mut self.line), self.line_w)); }
        let pad = (px_size / 2).max(6);
        let bar = 3usize;
        let box_w = (rendered.iter().map(|r| r.1).max().unwrap_or(0) + 2 * pad + bar).min(w);
        let box_h = (lines.len() * line_h + 2 * pad).min(h);
        let x0 = w.saturating_sub(box_w + w / 30);
        let bottom = h.saturating_sub(h / 7);
        let y0 = bottom.saturating_sub(box_h);
        let radius = style::RADIUS.min(box_h / 2) as i64;
        for y in y0..(y0 + box_h).min(h) {
            for x in x0..(x0 + box_w).min(w) {
                // Outside the rounded corners: leave the picture.
                let (lx, ly) = ((x - x0) as i64, (y - y0) as i64);
                let cx = if lx < radius { radius - lx } else if lx >= box_w as i64 - radius { lx - (box_w as i64 - radius - 1) } else { 0 };
                let cy = if ly < radius { radius - ly } else if ly >= box_h as i64 - radius { ly - (box_h as i64 - radius - 1) } else { 0 };
                if cx > 0 && cy > 0 && cx * cx + cy * cy > radius * radius { continue; }
                let p = &mut px[y * w + x];
                *p = if (x - x0) < bar { style::ACCENT } else { blend(*p, style::SURFACE, 235) };
            }
        }
        for (k, (line, lw)) in rendered.iter().enumerate() {
            composite(px, w, h, line, *lw, line_h, x0 + bar + pad, y0 + pad + k * line_h, false);
        }
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

/// Weft's dark theme (crates/weft/src/theme.rs): the default accent as dark grounds
/// show it, the surface and the track colour, and the corner radius.
pub mod style {
    pub const ACCENT: u32 = 0x00DB_4A7E;
    pub const SURFACE: u32 = 0x0026_232B;
    pub const TRACK: u32 = 0x0045_404C;
    pub const RADIUS: usize = 6;
}

/// `p` mixed toward `to` by `a` (0..255), keeping `p`'s top byte.
fn blend(p: u32, to: u32, a: u32) -> u32 {
    let ch = |v: u32, s: u32| (v >> s) & 0xFF;
    let mix = |s: u32| (ch(p, s) * (255 - a) + ch(to, s) * a) / 255;
    (p & 0xFF00_0000) | mix(16) << 16 | mix(8) << 8 | mix(0)
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
