// SPDX-License-Identifier: GPL-3.0-only
//! Display's text, drawn from the Geist fonts at run time and cached.
//!
//! Grid text is Geist Mono fitted to whatever cell the text scale gives; UI chrome,
//! menus, SilkBar labels, buttons, and window titles use Geist SemiBold (600) while
//! secondary/body labels and hints use Geist Medium (500) with Apple CoreText-style
//! subtle stem darkening so text feels warm and effortlessly readable on Noir dark
//! surfaces. Glyphs are drawn on first use into a fixed cache in Display's own
//! memory, so a new size costs one draw per character and nothing after. Display is
//! one task: these statics have one user.
use sex_font::cache::{GlyphCache, Key};
use sex_font::raster::scratch_len;
use sex_font::{
    Font, Synthetic, GEIST_MONO, GEIST_MONO_MEDIUM, GEIST_SANS, GEIST_SANS_MEDIUM,
    GEIST_SANS_SEMIBOLD,
};

const MONO: u8 = 1;
const SANS: u8 = 2;
const SANS_MEDIUM: u8 = 32;
const SANS_SEMIBOLD: u8 = 64;
const MONO_MEDIUM: u8 = 96;
/// Cache keys of fallback outline fonts in proportional text are SANS + their id;
/// emoji drawn in proportional text use this one.
const EMOJI_FONT: u8 = 200;

const WEIGHT_REGULAR: u8 = 0;
const WEIGHT_MEDIUM: u8 = 1;
const WEIGHT_SEMIBOLD: u8 = 2;

/// Largest bitmap drawn; a Key holds sizes up to 255.
// A wide (CJK, emoji) character's two cells at the 90x135 maximum: 180.
const MAX_W: usize = 192;
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

struct Fonts {
    mono: Font<'static>,
    sans: Font<'static>,
    sans_medium: Font<'static>,
    sans_semibold: Font<'static>,
    mono_medium: Font<'static>,
}

static mut FONTS: Option<Fonts> = None;

/// All Geist faces parsed once.
fn fonts() -> Option<&'static Fonts> {
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(FONTS) };
    if slot.is_none() {
        *slot = Some(Fonts {
            mono: Font::new(GEIST_MONO)?,
            sans: Font::new(GEIST_SANS)?,
            sans_medium: Font::new(GEIST_SANS_MEDIUM)?,
            sans_semibold: Font::new(GEIST_SANS_SEMIBOLD)?,
            mono_medium: Font::new(GEIST_MONO_MEDIUM)?,
        });
    }
    slot.as_ref()
}
fn mono() -> Option<&'static Font<'static>> { fonts().map(|f| &f.mono) }
fn mono_medium() -> Option<&'static Font<'static>> { fonts().map(|f| &f.mono_medium) }
fn sans_for_weight(weight: u8) -> Option<&'static Font<'static>> {
    let f = fonts()?;
    Some(match weight {
        WEIGHT_MEDIUM => &f.sans_medium,
        WEIGHT_SEMIBOLD => &f.sans_semibold,
        _ => &f.sans,
    })
}

static mut FALLBACK: core::mem::MaybeUninit<sex_font::fallback::Fallback<'static>> = core::mem::MaybeUninit::uninit();
/// 0: not tried yet, 1: parsed, 2: a font failed to parse.
static mut FALLBACK_STATE: u8 = 0;
/// The Noto fallbacks and the emoji, parsed on the first character Geist lacks.
/// They are parsed straight into the static: built on Display's 64 KB stack and
/// copied, deep inside text drawing, they overflowed it.
fn fallback() -> Option<&'static sex_font::fallback::Fallback<'static>> {
    // SAFETY: Display is one task (see the module note); FALLBACK is read only once
    // FALLBACK_STATE says it was fully written.
    unsafe {
        if FALLBACK_STATE == 0 { load_fallback(); }
        (FALLBACK_STATE == 1).then(|| (*core::ptr::addr_of!(FALLBACK)).assume_init_ref())
    }
}
/// Parse every font now, while the stack is shallow, rather than on the first
/// character that needs one, deep inside drawing.
pub fn warm() { let _ = fonts(); let _ = fallback(); }
#[cold]
#[inline(never)]
unsafe fn load_fallback() {
    let ok = sex_font::fallback::Fallback::embedded_into(&mut *core::ptr::addr_of_mut!(FALLBACK));
    FALLBACK_STATE = if ok { 1 } else { 2 };
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

/// Coverage for one `w` x `h` grid cell rendered in `GeistMono-Medium` (500) for
/// warm Apple-terminal density.
#[allow(dead_code)]
pub fn mono_medium_cell(scalar: u32, w: usize, h: usize) -> Option<&'static [u8]> {
    if w == 0 || h == 0 || w > MAX_W || h > MAX_H || scalar == 9 || scalar == 10 { return None; }
    let key = Key { font: MONO_MEDIUM, scalar, size: 0, phase: 0, w: w as u8, h: h as u8 };
    cache().get(key, |out| {
        mono_medium().is_some_and(|font| sex_font::draw_cell(font, scalar, w, h, acc(), out))
            || fallback().is_some_and(|f| f.draw_cell(scalar, w, h, acc(), out))
    })
}

/// Window titles: Geist SemiBold at 17 px on an 18-pixel line, baseline 14 px down.
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

/// Window titles are at most 16 bytes; panel notices (`sex_pdx::notify`) up to 128.
const TITLE_MAX: usize = 128;
/// `font` of a byte inside a UTF-8 sequence: drawn by the sequence's first byte.
const SKIP: u8 = 255;
/// `font` of an emoji (its scalar in `scalar`), drawn from the emoji coverage.
const EMOJI: u8 = 254;

/// A title laid out once: per byte its glyph (0 = none), the font that has it (0
/// Geist Sans, 1.. a fallback outline font, [`EMOJI`], [`SKIP`]), pen position in
/// 1/256 pixel and bitmap width in pixels. Recent titles are kept, as titles are
/// drawn a pixel at a time.
#[derive(Clone, Copy)]
struct TitleLayout { size: u32, weight: u8, len: usize, text: [u8; TITLE_MAX], glyph: [u16; TITLE_MAX], font: [u8; TITLE_MAX], scalar: [u32; TITLE_MAX], pen: [i32; TITLE_MAX], width: [u8; TITLE_MAX], end: i32 }
const NO_LAYOUT: TitleLayout = TitleLayout { size: 0, weight: 0, len: usize::MAX, text: [0; TITLE_MAX], glyph: [0; TITLE_MAX], font: [0; TITLE_MAX], scalar: [0; TITLE_MAX], pen: [0; TITLE_MAX], width: [0; TITLE_MAX], end: 0 };
static mut LAYOUTS: [TitleLayout; 8] = [NO_LAYOUT; 8];
static mut NEXT_LAYOUT: usize = 0;

/// The scalar starting at byte `at` and its end; a malformed byte stands for itself.
fn utf8_at(text: &[u8], at: usize) -> (u32, usize) {
    let n = match text[at] { 0xC2..=0xDF => 2, 0xE0..=0xEF => 3, 0xF0..=0xF4 => 4, _ => 1 };
    match text.get(at..at + n).and_then(|b| core::str::from_utf8(b).ok()).and_then(|s| s.chars().next()) {
        Some(c) if n > 1 => (c as u32, at + n),
        _ => (text[at] as u32, at + 1),
    }
}

fn text_layout(title: &[u8], size: u32, weight: u8) -> Option<&'static TitleLayout> {
    let title = &title[..title.len().min(TITLE_MAX)];
    let layouts = unsafe { &mut *core::ptr::addr_of_mut!(LAYOUTS) };
    if let Some(i) = layouts.iter().position(|l| l.size == size && l.weight == weight && l.len == title.len() && &l.text[..l.len] == title) { return Some(&layouts[i]); }
    let font = sans_for_weight(weight)?;
    // Laid out in its slot, not on the stack (Display's stack is 64 KB).
    let next = unsafe { &mut *core::ptr::addr_of_mut!(NEXT_LAYOUT) };
    let slot = *next % layouts.len();
    *next = next.wrapping_add(1);
    let l = &mut layouts[slot];
    *l = NO_LAYOUT;
    l.size = size;
    l.weight = weight;
    l.len = title.len();
    l.text[..title.len()].copy_from_slice(title);
    let (mut pen, mut prev) = (0, None);
    let mut i = 0;
    while i < title.len() {
        // One scalar (a malformed byte stands for itself); its other bytes are skipped.
        let (scalar, end) = utf8_at(title, i);
        for k in i + 1..end.min(title.len()) { l.font[k] = SKIP; l.pen[k] = pen; }
        l.pen[i] = pen;
        l.scalar[i] = scalar;
        if let Some(g) = font.glyph(scalar) {
            if let Some(p) = prev { pen += font.kerning(p, g, size); l.pen[i] = pen; }
            let width = ((font.advance(g, size) + 255) >> 8) + 2 * sans_padding(size);
            if width as usize <= MAX_W { l.glyph[i] = g.0; l.width[i] = width as u8; }
            pen += font.advance(g, size);
            prev = Some(g);
        } else if let Some((id, f, g)) = fallback().and_then(|fb| fb.outline(scalar)).and_then(|(id, f)| Some((id, f, f.glyph(scalar)?))) {
            // CJK, Cyrillic, Arabic...: the fallback font at the same size.
            let width = ((f.advance(g, size) + 255) >> 8) + 2 * sans_padding(size);
            if width as usize <= MAX_W { l.glyph[i] = g.0; l.font[i] = id; l.width[i] = width as u8; }
            pen += f.advance(g, size);
            prev = None;
        } else if fallback().is_some_and(|fb| fb.emoji().has(scalar)) {
            let side = (size / 64) as i32 + 2;
            if side as usize <= MAX_W { l.glyph[i] = 1; l.font[i] = EMOJI; l.width[i] = side as u8; }
            pen += side * 256;
            prev = None;
        }
        i = end.max(i + 1);
    }
    l.end = pen;
    Some(&layouts[slot])
}

/// Where each character of `title` begins, in whole pixels from the title's left.
pub fn title_starts(title: &[u8], out: &mut [usize]) -> usize {
    let Some(l) = text_layout(title, TITLE_SIZE, WEIGHT_SEMIBOLD) else { return 0 };
    for (slot, pen) in out.iter_mut().zip(&l.pen[..l.len]) { *slot = (*pen >> 8) as usize; }
    l.len
}

/// Compact UI captions: proportional Geist Medium, 9 px on a 9 px row.
pub const CAPTION_SIZE: u32 = 9 * 64;
pub const CAPTION_H: usize = 9;
const CAPTION_BASELINE: i32 = 7 * 256;

/// Natural advance for a compact caption, used to bound the existing status boxes.
pub fn caption_width(text: &[u8]) -> Option<usize> {
    let layout = text_layout(text, CAPTION_SIZE, WEIGHT_MEDIUM)?;
    Some(((layout.end + 255).max(0) >> 8) as usize)
}

/// Primary UI chrome role (SilkBar labels, window titles, dropdown items, Spotlight
/// tile labels, active tabs, buttons): Geist SemiBold (600) at 13 logical pixels on
/// a 16px line with Apple CoreText-style subtle stem darkening.
pub const UI_H:usize=sex_pdx::desktop_style::chrome::UI_LINE as usize;
pub fn ui_width(text:&[u8],scale:u32)->Option<usize>{
    let size=sex_pdx::desktop_style::chrome::UI_FONT*64*scale.clamp(1,3);
    let l=text_layout(text,size,WEIGHT_SEMIBOLD)?;
    Some(((l.end+255).max(0)>>8)as usize)
}
pub fn ui_coverage(text:&[u8],x:i32,y:usize,scale:u32)->Option<u8>{
    let scale=scale.clamp(1,3);
    sans_coverage(text,x,y,sex_pdx::desktop_style::chrome::UI_FONT*64*scale,UI_H*scale as usize,sex_pdx::desktop_style::chrome::UI_FONT as i32*256*scale as i32,WEIGHT_SEMIBOLD)
}

/// Where `ui_coverage`'s ink ends: past this x every pixel of the text has none.
/// None when the text cannot be laid out (then ask pixel by pixel).
pub fn ui_ink_end(text:&[u8],scale:u32)->Option<i32>{
    let scale=scale.clamp(1,3);
    let size=sex_pdx::desktop_style::chrome::UI_FONT*64*scale;
    let l=text_layout(text,size,WEIGHT_SEMIBOLD)?;
    let mut end=0i32;
    for i in 0..l.len{
        if l.font[i]==SKIP{continue;}
        end=end.max((l.pen[i]>>8)-sans_padding(size)+l.width[i] as i32);
    }
    Some(end)
}

/// Standard UI body role (descriptions, hints, shortcuts, placeholders, subtext):
/// Geist Medium (500) at 13 logical pixels on a 16px line with Apple CoreText-style
/// subtle stem darkening so dark-mode body text reads warmly without feeling thin.
pub fn ui_body_width(text:&[u8],scale:u32)->Option<usize>{
    let size=sex_pdx::desktop_style::chrome::UI_FONT*64*scale.clamp(1,3);
    let l=text_layout(text,size,WEIGHT_MEDIUM)?;
    Some(((l.end+255).max(0)>>8)as usize)
}
pub fn ui_body_coverage(text:&[u8],x:i32,y:usize,scale:u32)->Option<u8>{
    let scale=scale.clamp(1,3);
    sans_coverage(text,x,y,sex_pdx::desktop_style::chrome::UI_FONT*64*scale,UI_H*scale as usize,sex_pdx::desktop_style::chrome::UI_FONT as i32*256*scale as i32,WEIGHT_MEDIUM)
}

/// Title coverage at (`x`, `y`) pixels from the title's top left.
pub fn title_coverage(title: &[u8], x: i32, y: usize) -> u8 {
    sans_coverage(title, x, y, TITLE_SIZE, TITLE_H, TITLE_BASELINE, WEIGHT_SEMIBOLD).unwrap_or(0)
}

/// Coverage of a compact caption, or None on unavailable font/glyph/cache backing.
pub fn caption_coverage(text: &[u8], x: i32, y: usize) -> Option<u8> {
    sans_coverage(text, x, y, CAPTION_SIZE, CAPTION_H, CAPTION_BASELINE, WEIGHT_MEDIUM)
}

fn sans_cache_base(weight: u8) -> u8 {
    match weight {
        WEIGHT_MEDIUM => SANS_MEDIUM,
        WEIGHT_SEMIBOLD => SANS_SEMIBOLD,
        _ => SANS,
    }
}

fn sans_coverage(title: &[u8], x: i32, y: usize, size: u32, height: usize, baseline: i32, weight: u8) -> Option<u8> {
    if y >= height { return Some(0); }
    let l = text_layout(title, size, weight)?;
    let mut cover = 0u32;
    let base_id = sans_cache_base(weight);
    for i in 0..l.len {
        if l.font[i] == SKIP { continue; }
        let left = (l.pen[i] >> 8) - sans_padding(size);
        if x < left { break; }
        let width = l.width[i] as i32;
        if l.glyph[i] == 0 { return None; }
        if x >= left + width { continue; }
        let phase = ((l.pen[i] & 255) >> 6) as u8;
        let bitmap = if l.font[i] == EMOJI {
            let key = Key { font: EMOJI_FONT, scalar: l.scalar[i], size: (size / 64) as u16, phase: 0, w: width as u8, h: height as u8 };
            cache().get(key, |out| fallback().is_some_and(|f| f.emoji().draw_cell(l.scalar[i], width as usize, height, out)))?
        } else if l.font[i] == 0 {
            let font = sans_for_weight(weight)?;
            let glyph = sex_font::GlyphId(l.glyph[i]);
            let key = Key { font: base_id, scalar: glyph.0 as u32, size: (size / 64) as u16, phase, w: width as u8, h: height as u8 };
            let origin = sans_padding(size) * 256 + phase as i32 * 64;
            cache().get(key, |out| font.draw_humanized(glyph, size, origin, baseline, width as usize, height, acc(), out, Synthetic::NONE, 32 * 64))?
        } else {
            let font = fallback()?.font(l.font[i])?;
            let glyph = sex_font::GlyphId(l.glyph[i]);
            let key = Key { font: SANS + l.font[i], scalar: glyph.0 as u32, size: (size / 64) as u16, phase, w: width as u8, h: height as u8 };
            let origin = sans_padding(size) * 256 + phase as i32 * 64;
            cache().get(key, |out| font.draw(glyph, size, origin, baseline, width as usize, height, acc(), out))?
        };
        cover += bitmap[y * width as usize + (x - left) as usize] as u32;
    }
    Some(cover.min(255) as u8)
}
