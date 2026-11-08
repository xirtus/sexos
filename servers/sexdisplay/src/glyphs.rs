// SPDX-License-Identifier: GPL-3.0-only
//! Display's text, drawn from the Geist fonts at run time and cached.
//!
//! Grid text is Geist Mono fitted to whatever cell the text scale gives; titles are
//! Geist Sans, kerned and placed to a quarter pixel. Glyphs are drawn on first use
//! into a fixed cache in Display's own memory, so a new size costs one draw per
//! character and nothing after. Display is one task: these statics have one user.
use sex_font::cache::{GlyphCache, Key};
use sex_font::raster::scratch_len;
use sex_font::{Font, GEIST_MONO, GEIST_SANS};

const MONO: u8 = 1;
const SANS: u8 = 2;
/// Largest bitmap drawn; a Key holds sizes up to 255.
const MAX_W: usize = 128;
const MAX_H: usize = 96;

static mut CACHE: GlyphCache<2048, { 512 * 1024 }> = GlyphCache::new();
static mut ACC: [i32; scratch_len(MAX_W, MAX_H)] = [0; scratch_len(MAX_W, MAX_H)];

fn cache() -> &'static mut GlyphCache<2048, { 512 * 1024 }> { unsafe { &mut *core::ptr::addr_of_mut!(CACHE) } }
fn acc() -> &'static mut [i32] { unsafe { &mut *core::ptr::addr_of_mut!(ACC) } }

static mut FONTS: Option<(Font<'static>, Font<'static>)> = None;

/// Both fonts, parsed once.
fn fonts() -> Option<&'static (Font<'static>, Font<'static>)> {
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(FONTS) };
    if slot.is_none() { *slot = Some((Font::new(GEIST_MONO)?, Font::new(GEIST_SANS)?)); }
    slot.as_ref()
}
fn mono() -> Option<&'static Font<'static>> { fonts().map(|f| &f.0) }
fn sans() -> Option<&'static Font<'static>> { fonts().map(|f| &f.1) }

/// Geist Mono coverage for one `w` x `h` grid cell, or `None` if Geist lacks the
/// scalar (callers fall back to the bitmap font) or the cell is beyond the cache.
pub fn mono_cell(scalar: u32, w: usize, h: usize) -> Option<&'static [u8]> {
    if w == 0 || h == 0 || w > MAX_W || h > MAX_H || scalar == 9 || scalar == 10 { return None; }
    let key = Key { font: MONO, scalar, size: 0, phase: 0, w: w as u8, h: h as u8 };
    cache().get(key, |out| mono().is_some_and(|font| sex_font::draw_cell(font, scalar, w, h, acc(), out)))
}

/// Window titles: Geist Sans at 17 px on an 18-pixel line, baseline 14 px down.
pub const TITLE_SIZE: u32 = 17 * 64;
pub const TITLE_H: usize = 18;
const TITLE_BASELINE: i32 = 14 * 256;
/// Pixels a glyph's bitmap reaches left of its pen position, for overhangs.
const TITLE_LEFT: i32 = 2;

const TITLE_MAX: usize = 16;

/// A title laid out once: per character its glyph (0 = none), pen position in
/// 1/256 pixel and bitmap width in pixels. Recent titles are kept, as titles are
/// drawn a pixel at a time.
#[derive(Clone, Copy)]
struct TitleLayout { len: usize, text: [u8; TITLE_MAX], glyph: [u16; TITLE_MAX], pen: [i32; TITLE_MAX], width: [u8; TITLE_MAX] }
const NO_LAYOUT: TitleLayout = TitleLayout { len: usize::MAX, text: [0; TITLE_MAX], glyph: [0; TITLE_MAX], pen: [0; TITLE_MAX], width: [0; TITLE_MAX] };
static mut LAYOUTS: [TitleLayout; 8] = [NO_LAYOUT; 8];
static mut NEXT_LAYOUT: usize = 0;

fn title_layout(title: &[u8]) -> Option<&'static TitleLayout> {
    let title = &title[..title.len().min(TITLE_MAX)];
    let layouts = unsafe { &mut *core::ptr::addr_of_mut!(LAYOUTS) };
    if let Some(i) = layouts.iter().position(|l| l.len == title.len() && &l.text[..l.len] == title) { return Some(&layouts[i]); }
    let font = sans()?;
    let mut l = NO_LAYOUT;
    l.len = title.len();
    l.text[..title.len()].copy_from_slice(title);
    let (mut pen, mut prev) = (0, None);
    for (i, &byte) in title.iter().enumerate() {
        let glyph = font.glyph(byte as u32);
        if let (Some(p), Some(g)) = (prev, glyph) { pen += font.kerning(p, g, TITLE_SIZE); }
        l.pen[i] = pen;
        if let Some(g) = glyph {
            let width = ((font.advance(g, TITLE_SIZE) + 255) >> 8) + 2 * TITLE_LEFT;
            if width as usize <= MAX_W { l.glyph[i] = g.0; l.width[i] = width as u8; }
            pen += font.advance(g, TITLE_SIZE);
            prev = Some(g);
        }
    }
    let next = unsafe { &mut *core::ptr::addr_of_mut!(NEXT_LAYOUT) };
    let i = *next % layouts.len();
    *next = next.wrapping_add(1);
    layouts[i] = l;
    Some(&layouts[i])
}

/// Where each character of `title` begins, in whole pixels from the title's left.
pub fn title_starts(title: &[u8], out: &mut [usize]) -> usize {
    let Some(l) = title_layout(title) else { return 0 };
    for (slot, pen) in out.iter_mut().zip(&l.pen[..l.len]) { *slot = (*pen >> 8) as usize; }
    l.len
}

/// Title coverage at (`x`, `y`) pixels from the title's top left.
pub fn title_coverage(title: &[u8], x: i32, y: usize) -> u8 {
    if y >= TITLE_H { return 0; }
    let (Some(l), Some(font)) = (title_layout(title), sans()) else { return 0 };
    let mut cover = 0u32;
    for i in 0..l.len {
        let left = (l.pen[i] >> 8) - TITLE_LEFT;
        if x < left { break; }
        let width = l.width[i] as i32;
        if l.glyph[i] == 0 || x >= left + width { continue; }
        // Quarter-pixel phases keep spacing true without a bitmap per position.
        let phase = ((l.pen[i] & 255) >> 6) as u8;
        let glyph = sex_font::GlyphId(l.glyph[i]);
        let key = Key { font: SANS, scalar: glyph.0 as u32, size: (TITLE_SIZE / 64) as u16, phase, w: width as u8, h: TITLE_H as u8 };
        let origin = TITLE_LEFT * 256 + phase as i32 * 64;
        if let Some(bitmap) = cache().get(key, |out| font.draw(glyph, TITLE_SIZE, origin, TITLE_BASELINE, width as usize, TITLE_H, acc(), out)) {
            cover += bitmap[y * width as usize + (x - left) as usize] as u32;
        }
    }
    cover.min(255) as u8
}
