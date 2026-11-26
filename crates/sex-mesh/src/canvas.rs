// SPDX-License-Identifier: GPL-3.0-only
//! A window's pixels (0x00RRGGBB, row-major) and the few things Mesh draws on them:
//! boxes, lines and monospaced text from Geist Mono. Everything is clipped.
use sex_font::cache::{GlyphCache, Key};
use sex_font::raster::scratch_len;
use sex_font::Font;

/// One text cell, in pixels.
pub const CELL_W: usize = 9;
pub const CELL_H: usize = 18;
const SLOTS: usize = 256;

/// `a` blended toward `b` by `alpha` of 255.
pub fn mix(a: u32, b: u32, alpha: u8) -> u32 {
    let k = alpha as u32;
    let ch = |s: u32| -> u32 { (((a >> s) & 0xFF) * (255 - k) + ((b >> s) & 0xFF) * k + 127) / 255 };
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

pub struct Canvas<'a> {
    px: &'a mut [u32],
    pub w: usize,
    pub h: usize,
}

impl<'a> Canvas<'a> {
    /// `None` when the buffer holds fewer than `w * h` pixels.
    pub fn new(px: &'a mut [u32], w: usize, h: usize) -> Option<Self> {
        (px.len() >= w.checked_mul(h)?).then_some(Self { px, w, h })
    }
    pub fn pixel(&self, x: i32, y: i32) -> Option<u32> {
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h).then(|| self.px[y as usize * self.w + x as usize])
    }
    pub fn put(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h { self.px[y as usize * self.w + x as usize] = color; }
    }
    fn blend(&mut self, x: i32, y: i32, color: u32, alpha: u8) {
        if let Some(old) = self.pixel(x, y) { self.put(x, y, mix(old, color, alpha)); }
    }
    /// A filled box, clipped to the canvas.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let x1 = (x.saturating_add(w)).clamp(0, self.w as i32) as usize;
        let y1 = (y.saturating_add(h)).clamp(0, self.h as i32) as usize;
        if x0 >= x1 { return; }
        for row in y0..y1 { self.px[row * self.w + x0..row * self.w + x1].fill(color); }
    }
    /// A one-pixel outline.
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.fill(x, y, w, 1, color);
        self.fill(x, y + h - 1, w, 1, color);
        self.fill(x, y, 1, h, color);
        self.fill(x + w - 1, y, 1, h, color);
    }
    /// A one-pixel line from (x0, y0) to (x1, y1), both ends included.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.put(x, y, color);
            if x == x1 && y == y1 { break; }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; x += sx; }
            if e2 <= dx { err += dx; y += sy; }
        }
    }
    /// Text in cells from (x, y), the cell's top left; returns the x after it. Bytes
    /// outside printable ASCII show as `?`.
    pub fn text(&mut self, glyphs: &mut Glyphs, x: i32, y: i32, text: &[u8], color: u32) -> i32 {
        let mut at = x;
        for &b in text {
            if at >= self.w as i32 { break; }
            let c = if (0x21..0x7F).contains(&b) { b } else if b == b' ' { at += CELL_W as i32; continue } else { b'?' };
            if let Some(cover) = glyphs.cell(c) {
                for (i, &a) in cover.iter().enumerate() {
                    if a != 0 { self.blend(at + (i % CELL_W) as i32, y + (i / CELL_W) as i32, color, a); }
                }
            }
            at += CELL_W as i32;
        }
        at
    }
    /// Smooth anti-aliased rounded rectangle.
    pub fn rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32) {
        sex_theme::candy_ui::fill_rounded_rect(self.px, self.w, self.h, x, y, w, h, radius, color);
    }
    /// Frosted Candy Lavender gradient toolbar bar with specular highlight and orchid hairline.
    pub fn candy_toolbar(&mut self, x: i32, y: i32, w: i32, h: i32, pal: &sex_theme::candy_ui::CandyPalette) {
        sex_theme::candy_ui::draw_candy_toolbar(self.px, self.w, self.h, x, y, w, h, pal);
    }
    /// Smooth anti-aliased rounded Candy card/panel.
    pub fn candy_panel(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, pal: &sex_theme::candy_ui::CandyPalette) {
        sex_theme::candy_ui::draw_candy_panel(self.px, self.w, self.h, x, y, w, h, radius, pal);
    }
    /// Smooth anti-aliased rounded Candy button with hover/pressed visual states.
    #[allow(clippy::too_many_arguments)]
    pub fn candy_button(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
        style: sex_theme::candy_ui::ButtonStyle,
        state: sex_theme::candy_ui::ButtonState,
        pal: &sex_theme::candy_ui::CandyPalette,
    ) {
        sex_theme::candy_ui::draw_candy_button(self.px, self.w, self.h, x, y, w, h, radius, style, state, pal);
    }
    /// Smooth anti-aliased rounded Candy tab pill.
    #[allow(clippy::too_many_arguments)]
    pub fn candy_tab(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        active: bool,
        state: sex_theme::candy_ui::ButtonState,
        pal: &sex_theme::candy_ui::CandyPalette,
    ) {
        sex_theme::candy_ui::draw_candy_tab(self.px, self.w, self.h, x, y, w, h, active, state, pal);
    }
    /// Rounded Candy selection pill with left orchid indicator.
    pub fn candy_selection_row(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, pal: &sex_theme::candy_ui::CandyPalette) {
        sex_theme::candy_ui::draw_candy_selection_row(self.px, self.w, self.h, x, y, w, h, radius, pal);
    }
}

/// Geist Mono drawn once per character and kept. About 42 KiB: a program keeps it in
/// a static, not on its stack (`Glyphs::new` is `const` for that).
pub struct Glyphs {
    font: Option<Font<'static>>,
    loaded: bool,
    cache: GlyphCache<SLOTS, { SLOTS * CELL_W * CELL_H }>,
    acc: [i32; scratch_len(CELL_W, CELL_H)],
}

impl Default for Glyphs {
    fn default() -> Self { Self::new() }
}

impl Glyphs {
    pub const fn new() -> Self {
        Self { font: None, loaded: false, cache: GlyphCache::new(), acc: [0; scratch_len(CELL_W, CELL_H)] }
    }
    /// The coverage of one cell, `CELL_W * CELL_H` bytes.
    pub fn cell(&mut self, c: u8) -> Option<&[u8]> {
        if !self.loaded { self.font = Font::new(sex_font::GEIST_MONO); self.loaded = true; }
        let font = self.font.as_ref()?;
        let acc = &mut self.acc;
        let key = Key { font: 0, scalar: c as u32, size: 0, phase: 0, w: CELL_W as u8, h: CELL_H as u8 };
        self.cache.get(key, |out| sex_font::draw_cell(font, c as u32, CELL_W, CELL_H, acc, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_runs_from_one_color_to_the_other() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 255), 0xFFFFFF);
        assert_eq!(mix(0x000000, 0x00FF00, 128), 0x008000);
    }

    #[test]
    fn boxes_and_lines_are_clipped() {
        let mut px = vec![0u32; 10 * 6];
        let mut c = Canvas::new(&mut px, 10, 6).unwrap();
        c.fill(-5, -5, 8, 8, 1);
        assert_eq!((c.pixel(0, 0), c.pixel(2, 2), c.pixel(3, 3)), (Some(1), Some(1), Some(0)));
        c.fill(8, 4, 100, 100, 2);
        assert_eq!((c.pixel(9, 5), c.pixel(7, 5)), (Some(2), Some(0)));
        c.line(0, 5, 9, 0, 3);
        assert_eq!((c.pixel(0, 5), c.pixel(9, 0)), (Some(3), Some(3)));
        c.line(-20, 1, 30, 1, 4);
        assert_eq!((c.pixel(0, 1), c.pixel(9, 1)), (Some(4), Some(4)));
        assert!(c.pixel(10, 0).is_none() && c.pixel(-1, 0).is_none());
        assert!(Canvas::new(&mut [0u32; 3], 2, 2).is_none());
    }

    #[test]
    fn text_inks_its_cells_and_advances() {
        let mut px = vec![0u32; 40 * CELL_H];
        let mut c = Canvas::new(&mut px, 40, CELL_H).unwrap();
        let mut g = Glyphs::new();
        let end = c.text(&mut g, 0, 0, b"M ", 0xFFFFFF);
        assert_eq!(end, 2 * CELL_W as i32);
        let ink = |c: &Canvas, x0: usize, x1: usize| (x0..x1).flat_map(|x| (0..CELL_H).map(move |y| (x, y))).filter(|&(x, y)| c.pixel(x as i32, y as i32) != Some(0)).count();
        assert!(ink(&c, 0, CELL_W) > 10, "M has ink");
        assert_eq!(ink(&c, CELL_W, 2 * CELL_W), 0, "space has none");
        // Unknown bytes still take a cell.
        assert_eq!(c.text(&mut g, 0, 0, &[0xC3, 0xA9], 0xFFFFFF), 2 * CELL_W as i32);
    }
}
