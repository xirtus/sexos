// SPDX-License-Identifier: GPL-3.0-only
//! Inline images: the per-screen store that kitty graphics ([`crate::kitty`]) and sixel
//! ([`crate::sixel`]) fill, the placements that anchor images to cells, and the
//! compositor that draws them over the text.
//!
//! A placement is anchored to an absolute line, counted since the screen was made
//! (history trims do not renumber it), so it scrolls with the text into the history
//! and is dropped once its last row leaves the scrollback. The primary and alternate
//! screens keep separate placements; switching screens drops the alternate screen's,
//! as kitty does, and `ED 2`/`ED 3` clear them. Decoded pixels are 0xAARRGGBB
//! (unpremultiplied), at most [`MEMORY_CAP`] bytes across all images, oldest evicted.

use crate::Screen;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::Cell;

/// Bytes of decoded pixels the store keeps at most; older images are evicted first.
pub const MEMORY_CAP: usize = 64 * 1024 * 1024;
/// Placements kept at most per screen; the oldest go first.
const MAX_PLACEMENTS: usize = 4096;
/// The cell size assumed until a painter says otherwise.
pub const DEFAULT_CELL: (usize, usize) = (10, 20);
/// kitty: z-indexes below this are drawn under non-default cell backgrounds too.
const UNDER_BACKGROUNDS: i32 = -1_073_741_824;

/// A decoded image.
#[derive(Clone, Debug)]
pub struct Image {
    /// The kitty image id, or 0 for an anonymous image (sixel, or `a=T` without `i`).
    pub id: u32,
    /// The kitty image number (`I`) it was transmitted with, or 0.
    pub number: u32,
    pub width: usize,
    pub height: usize,
    /// 0xAARRGGBB, row-major, unpremultiplied.
    pub pixels: Vec<u32>,
}

/// Where and how an image is shown.
#[derive(Clone, Debug)]
pub struct Placement {
    pub(crate) key: u64,
    /// The kitty image id (0 for anonymous images) and placement id (`p`, or 0).
    pub image_id: u32,
    pub id: u32,
    /// Absolute line since the screen was made (not renumbered by history trims).
    pub(crate) line: i64,
    pub col: usize,
    /// The cells the placement covers.
    pub cols: usize,
    pub rows: usize,
    /// Source rectangle in image pixels: x, y, width, height.
    pub src: (usize, usize, usize, usize),
    /// Pixel offset inside the first cell, at the reference cell size.
    pub offset: (usize, usize),
    /// Drawn size in pixels at the reference cell size; scaled when the cell size changes.
    pub(crate) px: (usize, usize),
    pub(crate) cell_ref: (usize, usize),
    /// Negative: under the text (see kitty's rules); otherwise over it.
    pub z: i32,
    pub(crate) alt: bool,
    seq: u64,
}

/// A placement as tests and callers see it: `row` is on the whole history, like
/// [`crate::Point::row`] (negative once its top has been trimmed away).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementInfo {
    pub image_id: u32,
    pub placement_id: u32,
    pub row: isize,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    pub z: i32,
    pub width: usize,
    pub height: usize,
}

/// What a new placement asks for; `None` sizes come from the image and cell size.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PlaceSpec {
    pub image_id: u32,
    pub placement_id: u32,
    pub src: (usize, usize, usize, usize),
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub offset: (usize, usize),
    pub z: i32,
}

pub(crate) enum CursorMove {
    /// kitty: right of the image, on its last row.
    Kitty,
    /// xterm sixel scrolling: the line below the image, same column.
    Sixel,
    None,
}

pub struct ImageStore {
    images: BTreeMap<u64, Image>,
    placements: Vec<Placement>,
    /// Lines trimmed from the front of the history since the screen was made.
    base: i64,
    next_key: u64,
    seq: u64,
    bytes: usize,
    cap: usize,
    next_id: u32,
    /// The painter's cell size, which sizes placements given in pixels.
    cell: Cell<(u16, u16)>,
    pub(crate) kitty: Option<crate::kitty::Pending>,
    pub(crate) sixel: Option<crate::sixel::Decoder>,
}

impl Default for ImageStore { fn default() -> Self { Self::new() } }

impl ImageStore {
    pub fn new() -> Self {
        ImageStore {
            images: BTreeMap::new(), placements: Vec::new(), base: 0, next_key: 1, seq: 0, bytes: 0, cap: MEMORY_CAP,
            next_id: 0x4000_0000, cell: Cell::new((DEFAULT_CELL.0 as u16, DEFAULT_CELL.1 as u16)), kitty: None, sixel: None,
        }
    }
    pub(crate) fn cell(&self) -> (usize, usize) { let (w, h) = self.cell.get(); (w.max(1) as usize, h.max(1) as usize) }
    /// Bytes of decoded pixels held.
    pub fn bytes(&self) -> usize { self.bytes }
    pub fn len(&self) -> usize { self.images.len() }
    pub fn is_empty(&self) -> bool { self.images.is_empty() }
    pub(crate) fn key_of(&self, id: u32) -> Option<u64> {
        if id == 0 { return None; }
        self.images.iter().rev().find(|(_, i)| i.id == id).map(|(k, _)| *k)
    }
    pub(crate) fn key_of_number(&self, number: u32) -> Option<u64> {
        if number == 0 { return None; }
        self.images.iter().rev().find(|(_, i)| i.number == number).map(|(k, _)| *k)
    }
    pub(crate) fn keys_in_range(&self, lo: u32, hi: u32) -> Vec<u64> {
        self.images.iter().filter(|(_, i)| i.id != 0 && i.id >= lo && i.id <= hi).map(|(k, _)| *k).collect()
    }
    pub(crate) fn get(&self, key: u64) -> Option<&Image> { self.images.get(&key) }
    /// An id no stored image has, for images sent with only a number.
    pub(crate) fn fresh_id(&mut self) -> u32 {
        loop {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if self.key_of(self.next_id).is_none() { return self.next_id; }
        }
    }
}

impl Screen {
    /// The cell size the painter draws with; it sizes images given in pixels. The
    /// painter sets it on every draw, so callers rarely need to.
    pub fn set_cell_pixels(&self, w: usize, h: usize) { self.images.cell.set((w.clamp(1, 1024) as u16, h.clamp(1, 1024) as u16)); }
    /// How many bytes of decoded pixels the screen keeps (default [`MEMORY_CAP`]).
    pub fn set_image_memory_cap(&mut self, bytes: usize) {
        self.images.cap = bytes;
        while self.images.bytes > bytes {
            let Some(&oldest) = self.images.images.keys().next() else { break };
            self.remove_image(oldest);
        }
    }
    /// The images this screen holds.
    pub fn images(&self) -> &ImageStore { &self.images }
    /// A kitty image by id.
    pub fn image(&self, id: u32) -> Option<&Image> { self.images.key_of(id).and_then(|k| self.images.get(k)) }
    /// The placements of the screen shown now (primary or alternate), oldest first.
    pub fn placements(&self) -> Vec<PlacementInfo> {
        let alt = self.alternate();
        self.images.placements.iter().filter(|p| p.alt == alt).map(|p| PlacementInfo {
            image_id: p.image_id, placement_id: p.id, row: (p.line - self.images.base) as isize, col: p.col,
            cols: p.cols, rows: p.rows, z: p.z, width: p.src.2, height: p.src.3,
        }).collect()
    }
    /// The absolute line of screen row 0 (the bottom of the history).
    pub(crate) fn screen_line(&self) -> i64 { self.images.base + self.history.len() as i64 }

    /// Stores an image; frees space first by evicting the oldest. `None` if it cannot fit.
    pub(crate) fn add_image(&mut self, image: Image) -> Option<u64> {
        let size = image.pixels.len() * 4;
        if size > self.images.cap { return None; }
        // A new image with an existing id replaces it, and its placements go too.
        if let Some(old) = self.images.key_of(image.id) { self.remove_image(old); }
        while self.images.bytes + size > self.images.cap {
            let Some(&oldest) = self.images.images.keys().next() else { break };
            self.remove_image(oldest);
        }
        let key = self.images.next_key;
        self.images.next_key += 1;
        self.images.bytes += size;
        self.images.images.insert(key, image);
        Some(key)
    }
    pub(crate) fn remove_image(&mut self, key: u64) {
        if let Some(img) = self.images.images.remove(&key) { self.images.bytes -= img.pixels.len() * 4; }
        self.drop_placements(|p| p.key == key);
    }
    /// Removes placements matching `f`, redraws their rows, then frees anonymous
    /// images nothing shows any more (they cannot be referred to again).
    pub(crate) fn drop_placements(&mut self, mut f: impl FnMut(&Placement) -> bool) -> usize {
        let mut gone = Vec::new();
        self.images.placements.retain(|p| if f(p) { gone.push(p.clone()); false } else { true });
        for p in &gone { self.touch_placement(p); }
        self.free_orphans();
        gone.len()
    }
    pub(crate) fn free_orphans(&mut self) {
        let orphans: Vec<u64> = self.images.images.iter()
            .filter(|(k, i)| i.id == 0 && !self.images.placements.iter().any(|p| p.key == **k))
            .map(|(k, _)| *k).collect();
        for k in orphans { if let Some(img) = self.images.images.remove(&k) { self.images.bytes -= img.pixels.len() * 4; } }
    }
    /// Frees images (with an id) no placement shows: kitty's upper-case deletes.
    pub(crate) fn free_unplaced(&mut self, keys: &[u64]) {
        for &k in keys {
            if !self.images.placements.iter().any(|p| p.key == k) {
                if let Some(img) = self.images.images.remove(&k) { self.images.bytes -= img.pixels.len() * 4; }
            }
        }
    }
    fn touch_placement(&mut self, p: &Placement) {
        if p.alt != self.alternate() { return; }
        let top = p.line - self.screen_line() + self.view as i64;
        for r in top.max(0)..(top + p.rows as i64).min(self.rows as i64) { self.touch(r as usize); }
    }
    pub(crate) fn placements_iter(&self) -> impl Iterator<Item = &Placement> { self.images.placements.iter() }

    /// Shows an image at the cursor and moves the cursor as `mv` says.
    pub(crate) fn place_at_cursor(&mut self, key: u64, spec: PlaceSpec, mv: CursorMove) -> Option<(usize, usize)> {
        let img = self.images.get(key)?;
        let (iw, ih) = (img.width, img.height);
        let (cw, ch) = self.images.cell();
        let (sx, sy) = (spec.src.0.min(iw.saturating_sub(1)), spec.src.1.min(ih.saturating_sub(1)));
        let sw = if spec.src.2 == 0 { iw - sx } else { spec.src.2.min(iw - sx) }.max(1);
        let sh = if spec.src.3 == 0 { ih - sy } else { spec.src.3.min(ih - sy) }.max(1);
        let (ox, oy) = (spec.offset.0.min(cw - 1), spec.offset.1.min(ch - 1));
        let (pw, ph) = match (spec.cols, spec.rows) {
            (Some(c), Some(r)) => (c * cw, r * ch),
            (Some(c), None) => (c * cw, (c * cw * sh).div_ceil(sw)),
            (None, Some(r)) => ((r * ch * sw).div_ceil(sh), r * ch),
            (None, None) => (sw, sh),
        };
        let cols = spec.cols.unwrap_or_else(|| (ox + pw).div_ceil(cw)).max(1);
        let rows = spec.rows.unwrap_or_else(|| (oy + ph).div_ceil(ch)).max(1);
        let (x, y) = (self.x.min(self.cols - 1), self.y);
        let alt = self.alternate();
        if spec.placement_id != 0 {
            let (id, pid) = (spec.image_id, spec.placement_id);
            self.drop_placements(|p| p.alt == alt && p.image_id == id && p.id == pid);
        }
        let mut excess = self.images.placements.iter().filter(|p| p.alt == alt).count().saturating_sub(MAX_PLACEMENTS - 1);
        if excess > 0 { self.drop_placements(|p| { let hit = excess > 0 && p.alt == alt; if hit { excess -= 1; } hit }); }
        self.images.seq += 1;
        let p = Placement {
            key, image_id: spec.image_id, id: spec.placement_id, line: self.screen_line() + y as i64, col: x, cols, rows,
            src: (sx, sy, sw, sh), offset: (ox, oy), px: (pw, ph), cell_ref: (cw, ch), z: spec.z, alt, seq: self.images.seq,
        };
        self.touch_placement(&p);
        self.images.placements.push(p);
        match mv {
            CursorMove::None => {}
            CursorMove::Kitty => {
                for _ in 1..rows { self.linefeed(); }
                let nx = x + cols;
                if nx >= self.cols { self.x = self.cols - 1; self.pending_wrap = true; } else { self.x = nx; }
            }
            CursorMove::Sixel => { for _ in 0..rows { self.linefeed(); } self.x = x; self.pending_wrap = false; }
        }
        Some((cols, rows))
    }

    // ---- Hooks the screen calls as text moves ----

    /// One line left the front of the history: drop placements now wholly gone.
    pub(crate) fn images_trimmed(&mut self) {
        self.images.base += 1;
        let base = self.images.base;
        if self.images.placements.iter().any(|p| !p.alt && p.line + (p.rows as i64) <= base) {
            self.drop_placements(|p| !p.alt && p.line + (p.rows as i64) <= base);
        }
    }
    /// Rows `top..=bottom` moved `n` lines up (or down); `into_history` when the lines
    /// leaving the top went into the history (whose lines keep their absolute numbers).
    pub(crate) fn images_scrolled(&mut self, top: usize, bottom: usize, n: usize, up: bool, into_history: bool) {
        if self.images.placements.is_empty() || n == 0 { return; }
        let alt = self.alternate();
        let n = n as i64;
        let (top, bottom) = (top as i64, bottom as i64);
        // The absolute line of screen row 0 before the scroll.
        let origin = self.screen_line() - if into_history { n } else { 0 };
        let mut drop_any = false;
        for p in self.images.placements.iter_mut().filter(|p| p.alt == alt) {
            let row = p.line - origin;
            if into_history {
                // Rows 0..=bottom moved into place without renumbering; rows below shifted.
                if row > bottom { p.line += n; }
            } else if row >= top && row <= bottom {
                if up { p.line -= n; if row - n + p.rows as i64 <= top { drop_any = true; } }
                else { p.line += n; if row + n > bottom { drop_any = true; } }
            }
        }
        if drop_any {
            let origin = self.screen_line();
            self.drop_placements(|p| {
                let row = p.line - origin;
                p.alt == alt && (if up { row + p.rows as i64 <= top && row + n >= top } else { row > bottom && row - n <= bottom })
            });
        }
    }
    /// `ED 2` drops the placements on the screen, `ED 3` all of this screen's.
    pub(crate) fn images_erased(&mut self, mode: u16) {
        if self.images.placements.is_empty() { return; }
        let (alt, origin, rows) = (self.alternate(), self.screen_line(), self.rows as i64);
        match mode {
            2 => { self.drop_placements(|p| { let r = p.line - origin; p.alt == alt && r + p.rows as i64 > 0 && r < rows }); }
            3 => { self.drop_placements(|p| p.alt == alt); }
            _ => {}
        }
    }
    /// Entering or leaving the alternate screen: its placements are dropped.
    pub(crate) fn images_alt_switched(&mut self) {
        if self.images.placements.iter().any(|p| p.alt) { self.drop_placements(|p| p.alt); }
        self.touch_all();
    }

    /// `CSI 14 t` and `CSI 16 t`: the text area and one cell in pixels.
    pub(crate) fn report_pixels(&mut self, what: u16) {
        let (cw, ch) = self.images.cell();
        let r = match what {
            14 => alloc::format!("\x1b[4;{};{}t", self.rows * ch, self.cols * cw),
            16 => alloc::format!("\x1b[6;{};{}t", ch, cw),
            _ => return,
        };
        self.replies.extend_from_slice(r.as_bytes());
    }
    /// XTSMGRAPHICS (`CSI ? Pi ; Pa ; Pv S`): colour registers and sixel geometry.
    pub(crate) fn xtsmgraphics(&mut self, params: &vte::Params) {
        let mut it = params.iter().map(|p| p[0]);
        let (item, _action) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        let (cw, ch) = self.images.cell();
        let r = match item {
            1 => alloc::format!("\x1b[?1;0;{}S", crate::sixel::REGISTERS),
            2 => alloc::format!("\x1b[?2;0;{};{}S", (self.cols * cw).min(crate::sixel::MAX_SIDE), (self.rows * ch).min(crate::sixel::MAX_SIDE)),
            n => alloc::format!("\x1b[?{};1;0S", n),
        };
        self.replies.extend_from_slice(r.as_bytes());
    }
}

/// Draws the placements visible on `screen` over the rows `drawn` marks (all rows when
/// `drawn` is empty), scaled to their cell boxes, alpha blended and clipped to the
/// grid (`cols * cell_w` by `rows * cell_h`, and the buffer). Pixels are 0x00RRGGBB.
pub fn composite(screen: &Screen, cell_w: usize, cell_h: usize, drawn: &[bool], pixels: &mut [u32], stride: usize) {
    let alt = screen.alternate();
    let mut order: Vec<&Placement> = screen.placements_iter().filter(|p| p.alt == alt).collect();
    if order.is_empty() || cell_w == 0 || cell_h == 0 || stride == 0 { return; }
    order.sort_by_key(|p| (p.z, p.seq));
    let pane_w = (screen.cols() * cell_w).min(stride) as i64;
    let pane_h = (screen.rows() * cell_h).min(pixels.len() / stride) as i64;
    let mut bgs: Vec<u32> = Vec::new();
    for p in order {
        let Some(img) = screen.images.get(p.key) else { continue };
        let (rcw, rch) = (p.cell_ref.0.max(1) as i64, p.cell_ref.1.max(1) as i64);
        let (cw, ch) = (cell_w as i64, cell_h as i64);
        let dw = (p.px.0 as i64 * cw / rcw).max(1);
        let dh = (p.px.1 as i64 * ch / rch).max(1);
        let srow = p.line - screen.screen_line() + screen.view_offset() as i64;
        let x0 = p.col as i64 * cw + p.offset.0 as i64 * cw / rcw;
        let y0 = srow * ch + p.offset.1 as i64 * ch / rch;
        let (sx, sy, sw, sh) = (p.src.0 as i64, p.src.1 as i64, p.src.2 as i64, p.src.3 as i64);
        let (xa, xb) = (x0.max(0), (x0 + dw).min(pane_w));
        if xa >= xb { continue; }
        for row in 0..screen.rows() {
            if !drawn.is_empty() && !drawn.get(row).copied().unwrap_or(false) { continue; }
            let (ya, yb) = ((row as i64 * ch).max(y0).max(0), ((row as i64 + 1) * ch).min(y0 + dh).min(pane_h));
            if ya >= yb { continue; }
            if p.z < 0 {
                bgs.clear();
                bgs.extend((0..screen.cols()).map(|c| screen.colors(&screen.visible(c, row)).1));
            }
            for y in ya..yb {
                let src_y = (sy + (y - y0) * sh / dh).min(img.height as i64 - 1) as usize;
                let src_row = &img.pixels[src_y * img.width..(src_y + 1) * img.width];
                let out = &mut pixels[y as usize * stride..y as usize * stride + pane_w as usize];
                for x in xa..xb {
                    let src_x = (sx + (x - x0) * sw / dw).min(img.width as i64 - 1) as usize;
                    let px = src_row[src_x];
                    let a = px >> 24;
                    if a == 0 { continue; }
                    let dst = &mut out[x as usize];
                    if p.z < 0 {
                        // Under the text: only where the cell's background shows.
                        let bg = bgs[x as usize / cell_w];
                        if *dst != bg || (p.z < UNDER_BACKGROUNDS && bg != screen.theme().bg) { continue; }
                    }
                    *dst = blend(*dst, px & 0x00FF_FFFF, a);
                }
            }
        }
    }
}

fn blend(bg: u32, fg: u32, a: u32) -> u32 {
    if a >= 255 { return fg; }
    let mix = |s: u32| (((fg >> s) & 0xFF) * a + ((bg >> s) & 0xFF) * (255 - a)) / 255;
    mix(16) << 16 | mix(8) << 8 | mix(0)
}

// ---- Base64, for kitty payloads ----

/// A streaming base64 decoder that skips padding and whitespace, so chunks may split
/// anywhere.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Base64 { acc: u32, n: u8 }
impl Base64 {
    pub fn decode(&mut self, text: &[u8], out: &mut Vec<u8>) -> bool {
        for &c in text {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
                _ => return false,
            };
            self.acc = self.acc << 6 | v as u32;
            self.n += 1;
            if self.n == 4 {
                out.extend_from_slice(&[(self.acc >> 16) as u8, (self.acc >> 8) as u8, self.acc as u8]);
                self.acc = 0;
                self.n = 0;
            }
        }
        true
    }
    /// The bytes left in a final unpadded group.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        match self.n {
            2 => out.push((self.acc >> 4) as u8),
            3 => out.extend_from_slice(&[(self.acc >> 10) as u8, (self.acc >> 2) as u8]),
            _ => {}
        }
        *self = Base64::default();
    }
}

pub fn base64_encode(data: &[u8]) -> Vec<u8> {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63]);
        out.push(A[(n >> 12) as usize & 63]);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if c.len() > 2 { A[n as usize & 63] } else { b'=' });
    }
    out
}

// ---- Fixtures: known images a gate can print and check ----

/// The fixture's four quadrant colours, 0x00RRGGBB: top-left, top-right, bottom-left,
/// bottom-right.
pub const FIXTURE_QUADRANTS: [u32; 4] = [0xFF0000, 0x00FF00, 0x0000FF, 0xFFFFFF];
/// The fixture image's size in pixels.
pub const FIXTURE_SIZE: (usize, usize) = (32, 16);
/// The kitty fixtures' image ids and the cells they cover (columns, rows).
pub const FIXTURE_KITTY_PNG_ID: u32 = 4242;
pub const FIXTURE_KITTY_CELLS: (usize, usize) = (8, 2);

/// The fixture image: four solid quadrants of [`FIXTURE_QUADRANTS`], opaque.
pub fn fixture_image() -> (usize, usize, Vec<u32>) {
    let (w, h) = FIXTURE_SIZE;
    let px = (0..w * h).map(|i| {
        let (x, y) = (i % w, i / w);
        0xFF00_0000 | FIXTURE_QUADRANTS[(y >= h / 2) as usize * 2 + (x >= w / 2) as usize]
    }).collect();
    (w, h, px)
}

/// kitty graphics escapes that show the fixture image as a PNG (`f=100`), sent in
/// chunks (`m=1`), at the cursor over [`FIXTURE_KITTY_CELLS`], quietly (`q=2`).
pub fn fixture_kitty_png() -> Vec<u8> {
    let (w, h, px) = fixture_image();
    let b64 = base64_encode(&crate::png::encode_rgba(w, h, &px));
    let mut out = Vec::new();
    let mut chunks = b64.chunks(512).peekable();
    let mut first = true;
    while let Some(c) = chunks.next() {
        let more = chunks.peek().is_some() as u8;
        out.extend_from_slice(b"\x1b_G");
        if first {
            let (cols, rows) = FIXTURE_KITTY_CELLS;
            out.extend_from_slice(alloc::format!("a=T,f=100,i={},q=2,c={},r={},", FIXTURE_KITTY_PNG_ID, cols, rows).as_bytes());
            first = false;
        }
        out.extend_from_slice(alloc::format!("m={};", more).as_bytes());
        out.extend_from_slice(c);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

/// A sixel image 24 by 12 pixels: the left half red, the right half blue.
pub const FIXTURE_SIXEL: &[u8] = b"\x1bPq\"1;1;24;12#1;2;100;0;0#2;2;0;0;100#1!12~#2!12~-#1!12~#2!12~\x1b\\";
/// The sixel fixture's halves, 0x00RRGGBB, and its size in pixels.
pub const FIXTURE_SIXEL_COLORS: [u32; 2] = [0xFF0000, 0x0000FF];
pub const FIXTURE_SIXEL_SIZE: (usize, usize) = (24, 12);
pub fn fixture_sixel() -> Vec<u8> { FIXTURE_SIXEL.to_vec() }

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
