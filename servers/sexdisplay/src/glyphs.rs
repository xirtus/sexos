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
// UI scale 3 times maximum text zoom 5 needs a 90x135 cell.
const MAX_H: usize = 144;

// Holds all 95 printable glyphs at the supported 90x135 maximum cell, plus
// ordinary UI text. Fixed service-private backing, never a thread-stack value.
const CACHE_BYTES: usize = 2 * 1024 * 1024;
static mut CACHE: GlyphCache<2048, CACHE_BYTES> = GlyphCache::new();
static mut ACC: [i32; scratch_len(MAX_W, MAX_H)] = [0; scratch_len(MAX_W, MAX_H)];

fn cache() -> &'static mut GlyphCache<2048, CACHE_BYTES> { unsafe { &mut *core::ptr::addr_of_mut!(CACHE) } }
#[cfg(test)]
pub fn cache_resets() -> u32 { cache().resets }
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

static mut FALLBACK: Option<sex_font::fallback::Fallback<'static>> = None;
/// Noto Sans Mono, Symbols 2, CJK and the emoji, parsed on the first character Geist lacks.
fn fallback() -> Option<&'static sex_font::fallback::Fallback<'static>> {
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(FALLBACK) };
    if slot.is_none() { *slot = sex_font::fallback::Fallback::embedded(); }
    slot.as_ref()
}

/// Coverage for one `w` x `h` grid cell: Geist Mono, else the first fallback font
/// that has the scalar (Cyrillic, Greek, CJK, emoji, symbols). `None` if none has it
/// (callers fall back to the bitmap font) or the cell is beyond the cache.
pub fn mono_cell(scalar: u32, w: usize, h: usize) -> Option<&'static [u8]> {
    if w == 0 || h == 0 || w > MAX_W || h > MAX_H || scalar == 9 || scalar == 10 { return None; }
    let key = Key { font: MONO, scalar, size: 0, phase: 0, w: w as u8, h: h as u8 };
    cache().get(key, |out| {
        mono().is_some_and(|font| sex_font::draw_cell(font, scalar, w, h, acc(), out))
            || fallback().is_some_and(|f| f.draw_cell(scalar, w, h, acc(), out))
    })
}

/// Window titles: Geist Sans at 17 px on an 18-pixel line, baseline 14 px down.
pub const TITLE_SIZE: u32 = 17 * 64;
pub const TITLE_H: usize = 18;
const TITLE_BASELINE: i32 = 14 * 256;
/// Pixels a glyph's bitmap reaches left of its pen position, for overhangs.
const TITLE_LEFT: i32 = 2;
fn sans_padding(size:u32)->i32{
    let base=sex_pdx::desktop_style::chrome::UI_FONT*64;
    let scale=size/base;
    if size%base==0&&scale>=1&&scale<=3{TITLE_LEFT*scale as i32}else{TITLE_LEFT}
}

/// Window titles are at most 16 bytes; panel notices use up to 48.
const TITLE_MAX: usize = 48;

/// A title laid out once: per character its glyph (0 = none), pen position in
/// 1/256 pixel and bitmap width in pixels. Recent titles are kept, as titles are
/// drawn a pixel at a time.
#[derive(Clone, Copy)]
struct TitleLayout { size: u32, len: usize, text: [u8; TITLE_MAX], glyph: [u16; TITLE_MAX], pen: [i32; TITLE_MAX], width: [u8; TITLE_MAX] }
const NO_LAYOUT: TitleLayout = TitleLayout { size: 0, len: usize::MAX, text: [0; TITLE_MAX], glyph: [0; TITLE_MAX], pen: [0; TITLE_MAX], width: [0; TITLE_MAX] };
static mut LAYOUTS: [TitleLayout; 8] = [NO_LAYOUT; 8];
static mut NEXT_LAYOUT: usize = 0;

fn text_layout(title: &[u8], size: u32) -> Option<&'static TitleLayout> {
    let title = &title[..title.len().min(TITLE_MAX)];
    let layouts = unsafe { &mut *core::ptr::addr_of_mut!(LAYOUTS) };
    if let Some(i) = layouts.iter().position(|l| l.size == size && l.len == title.len() && &l.text[..l.len] == title) { return Some(&layouts[i]); }
    let font = sans()?;
    let mut l = NO_LAYOUT;
    l.size = size;
    l.len = title.len();
    l.text[..title.len()].copy_from_slice(title);
    let (mut pen, mut prev) = (0, None);
    for (i, &byte) in title.iter().enumerate() {
        let glyph = font.glyph(byte as u32);
        if let (Some(p), Some(g)) = (prev, glyph) { pen += font.kerning(p, g, size); }
        l.pen[i] = pen;
        if let Some(g) = glyph {
            let width = ((font.advance(g, size) + 255) >> 8) + 2 * sans_padding(size);
            if width as usize <= MAX_W { l.glyph[i] = g.0; l.width[i] = width as u8; }
            pen += font.advance(g, size);
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
    let Some(l) = text_layout(title, TITLE_SIZE) else { return 0 };
    for (slot, pen) in out.iter_mut().zip(&l.pen[..l.len]) { *slot = (*pen >> 8) as usize; }
    l.len
}

/// Compact UI captions: proportional Geist Sans, 9 px on a 9 px row.
pub const CAPTION_SIZE: u32 = 9 * 64;
pub const CAPTION_H: usize = 9;
const CAPTION_BASELINE: i32 = 7 * 256;

/// Natural advance for a compact caption, used to bound the existing status boxes.
pub fn caption_width(text: &[u8]) -> Option<usize> {
    let layout = text_layout(text, CAPTION_SIZE)?;
    if layout.len == 0 { return Some(0); }
    let last = layout.len - 1;
    let font = sans()?;
    let advance = font.advance(sex_font::GlyphId(layout.glyph[last]), CAPTION_SIZE);
    Some(((layout.pen[last] + advance + 255).max(0) >> 8) as usize)
}

/// Default UI role: natural Geist Sans at 13 logical pixels on a 16px line.
/// Device scale changes size and baseline together; Mono document grids remain
/// separate. The fixed glyph/layout caches retain the same bounded backing.
pub const UI_H:usize=sex_pdx::desktop_style::chrome::UI_LINE as usize;
pub fn ui_width(text:&[u8],scale:u32)->Option<usize>{
    let size=sex_pdx::desktop_style::chrome::UI_FONT*64*scale.clamp(1,3);
    let l=text_layout(text,size)?;
    if l.len==0{return Some(0);}
    let last=l.len-1;
    Some(((l.pen[last]+sans()?.advance(sex_font::GlyphId(l.glyph[last]),size)+255).max(0)>>8)as usize)
}
pub fn ui_coverage(text:&[u8],x:i32,y:usize,scale:u32)->Option<u8>{
    let scale=scale.clamp(1,3);
    sans_coverage(text,x,y,sex_pdx::desktop_style::chrome::UI_FONT*64*scale,UI_H*scale as usize,sex_pdx::desktop_style::chrome::UI_FONT as i32*256*scale as i32)
}

/// Title coverage at (`x`, `y`) pixels from the title's top left.
pub fn title_coverage(title: &[u8], x: i32, y: usize) -> u8 {
    sans_coverage(title, x, y, TITLE_SIZE, TITLE_H, TITLE_BASELINE).unwrap_or(0)
}

/// Coverage of a compact caption, or None on unavailable font/glyph/cache backing.
pub fn caption_coverage(text: &[u8], x: i32, y: usize) -> Option<u8> {
    sans_coverage(text, x, y, CAPTION_SIZE, CAPTION_H, CAPTION_BASELINE)
}

fn sans_coverage(title: &[u8], x: i32, y: usize, size: u32, height: usize, baseline: i32) -> Option<u8> {
    if y >= height { return Some(0); }
    let l = text_layout(title, size)?;
    let font = sans()?;
    let mut cover = 0u32;
    for i in 0..l.len {
        let left = (l.pen[i] >> 8) - sans_padding(size);
        if x < left { break; }
        let width = l.width[i] as i32;
        if l.glyph[i] == 0 { return None; }
        if x >= left + width { continue; }
        let phase = ((l.pen[i] & 255) >> 6) as u8;
        let glyph = sex_font::GlyphId(l.glyph[i]);
        let key = Key { font: SANS, scalar: glyph.0 as u32, size: (size / 64) as u16, phase, w: width as u8, h: height as u8 };
        let origin = sans_padding(size) * 256 + phase as i32 * 64;
        let bitmap = cache().get(key, |out| font.draw(glyph, size, origin, baseline, width as usize, height, acc(), out))?;
        cover += bitmap[y * width as usize + (x - left) as usize] as u32;
    }
    Some(cover.min(255) as u8)
}
