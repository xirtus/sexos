// SPDX-License-Identifier: GPL-3.0-only
//! Painting into the window's 0x00RRGGBB pixels: clipped rectangles, rounded
//! rectangles, anti-aliased lines, text and images.
use sex_office::{mix, Canvas, Pen};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
    pub fn shrink(&self, d: i32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2 * d).max(0), (self.h - 2 * d).max(0))
    }
    /// Split off the top `h` pixels: (top, rest).
    pub fn take_top(&self, h: i32) -> (Rect, Rect) {
        let h = h.min(self.h).max(0);
        (Rect::new(self.x, self.y, self.w, h), Rect::new(self.x, self.y + h, self.w, self.h - h))
    }
    pub fn take_bottom(&self, h: i32) -> (Rect, Rect) {
        let h = h.min(self.h).max(0);
        (Rect::new(self.x, self.bottom() - h, self.w, h), Rect::new(self.x, self.y, self.w, self.h - h))
    }
    pub fn take_left(&self, w: i32) -> (Rect, Rect) {
        let w = w.min(self.w).max(0);
        (Rect::new(self.x, self.y, w, self.h), Rect::new(self.x + w, self.y, self.w - w, self.h))
    }
    pub fn take_right(&self, w: i32) -> (Rect, Rect) {
        let w = w.min(self.w).max(0);
        (Rect::new(self.right() - w, self.y, w, self.h), Rect::new(self.x, self.y, self.w - w, self.h))
    }
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
}

/// Text alignment inside a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Painter {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
    clip: Rect,
    clips: Vec<Rect>,
    pub pen: Pen,
    pub mono: Pen,
}

impl Painter {
    pub fn new(width: usize, height: usize) -> Painter {
        Painter {
            width,
            height,
            pixels: vec![0; width * height],
            clip: Rect::new(0, 0, width as i32, height as i32),
            clips: Vec::new(),
            pen: Pen::sans(crate::theme::FONT_PX),
            mono: Pen::mono(crate::theme::FONT_PX),
        }
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.pixels.resize(width * height, 0);
        self.clip = Rect::new(0, 0, width as i32, height as i32);
        self.clips.clear();
    }
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    /// Restrict drawing to `r` (inside the current clip) until [`Painter::pop_clip`].
    pub fn push_clip(&mut self, r: Rect) {
        self.clips.push(self.clip);
        self.clip = self.clip.intersect(&r);
    }
    pub fn pop_clip(&mut self) {
        if let Some(c) = self.clips.pop() {
            self.clip = c;
        }
    }
    pub fn clip(&self) -> Rect {
        self.clip
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, color: u32, alpha: u32) {
        if !self.clip.contains(x, y) || alpha == 0 {
            return;
        }
        let i = y as usize * self.width + x as usize;
        self.pixels[i] = if alpha >= 255 { color } else { mix(self.pixels[i], color, alpha) };
    }

    pub fn fill(&mut self, r: Rect, color: u32) {
        let c = r.intersect(&self.clip);
        for y in c.y..c.bottom() {
            let row = y as usize * self.width;
            self.pixels[row + c.x as usize..row + c.right() as usize].fill(color);
        }
    }
    pub fn fill_alpha(&mut self, r: Rect, color: u32, alpha: u32) {
        let c = r.intersect(&self.clip);
        for y in c.y..c.bottom() {
            for x in c.x..c.right() {
                self.put(x, y, color, alpha);
            }
        }
    }
    pub fn frame(&mut self, r: Rect, color: u32) {
        self.fill(Rect::new(r.x, r.y, r.w, 1), color);
        self.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), color);
        self.fill(Rect::new(r.x, r.y, 1, r.h), color);
        self.fill(Rect::new(r.right() - 1, r.y, 1, r.h), color);
    }

    /// Rounded rectangle with anti-aliased corners of `radius` pixels.
    pub fn round_rect(&mut self, r: Rect, radius: i32, color: u32) {
        self.round_rect_alpha(r, radius, color, 255);
    }
    pub fn round_rect_alpha(&mut self, r: Rect, radius: i32, color: u32, alpha: u32) {
        let rad = radius.min(r.w / 2).min(r.h / 2).max(0);
        if rad == 0 {
            if alpha >= 255 {
                self.fill(r, color)
            } else {
                self.fill_alpha(r, color, alpha)
            }
            return;
        }
        let c = r.intersect(&self.clip);
        let rf = rad as f32;
        for y in c.y..c.bottom() {
            let dy = if y < r.y + rad {
                (r.y + rad) as f32 - (y as f32 + 0.5)
            } else if y >= r.bottom() - rad {
                (y as f32 + 0.5) - (r.bottom() - rad) as f32
            } else {
                -1.0
            };
            for x in c.x..c.right() {
                let dx = if x < r.x + rad {
                    (r.x + rad) as f32 - (x as f32 + 0.5)
                } else if x >= r.right() - rad {
                    (x as f32 + 0.5) - (r.right() - rad) as f32
                } else {
                    -1.0
                };
                let a = if dx > 0.0 && dy > 0.0 {
                    let d = (dx * dx + dy * dy).sqrt();
                    ((rf - d + 0.5).clamp(0.0, 1.0) * alpha as f32) as u32
                } else {
                    alpha
                };
                self.put(x, y, color, a);
            }
        }
    }
    /// Rounded outline one pixel wide.
    pub fn round_frame(&mut self, r: Rect, radius: i32, color: u32) {
        // Ring = outer minus inner, approximated by drawing the outer and refilling inside.
        let rad = radius.min(r.w / 2).min(r.h / 2).max(0) as f32;
        let c = r.intersect(&self.clip);
        for y in c.y..c.bottom() {
            for x in c.x..c.right() {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let qx = (px - (r.x as f32 + r.w as f32 / 2.0)).abs() - (r.w as f32 / 2.0 - rad);
                let qy = (py - (r.y as f32 + r.h as f32 / 2.0)).abs() - (r.h as f32 / 2.0 - rad);
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad;
                let a = (1.0 - (outside + 0.5).abs()).clamp(0.0, 1.0);
                if a > 0.0 {
                    self.put(x, y, color, (a * 255.0) as u32);
                }
            }
        }
    }

    /// Anti-aliased line (Xiaolin Wu) from (x0,y0) to (x1,y1).
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: u32) {
        self.line_alpha(x0, y0, x1, y1, color, 255);
    }
    pub fn line_alpha(&mut self, mut x0: f32, mut y0: f32, mut x1: f32, mut y1: f32, color: u32, alpha: u32) {
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return;
        }
        let steep = (y1 - y0).abs() > (x1 - x0).abs();
        if steep {
            core::mem::swap(&mut x0, &mut y0);
            core::mem::swap(&mut x1, &mut y1);
        }
        if x0 > x1 {
            core::mem::swap(&mut x0, &mut x1);
            core::mem::swap(&mut y0, &mut y1);
        }
        let dx = x1 - x0;
        let g = if dx.abs() < 1e-6 { 1.0 } else { (y1 - y0) / dx };
        // Clamp the walk to the clip to keep huge lines cheap.
        let (lo, hi) = if steep { (self.clip.y as f32 - 1.0, self.clip.bottom() as f32 + 1.0) } else { (self.clip.x as f32 - 1.0, self.clip.right() as f32 + 1.0) };
        let xs = x0.max(lo).floor() as i32;
        let xe = x1.min(hi).ceil() as i32;
        let a = alpha as f32;
        for x in xs..=xe {
            let y = y0 + g * (x as f32 - x0);
            let yi = y.floor();
            let f = y - yi;
            let (p, q) = (((1.0 - f) * a) as u32, (f * a) as u32);
            if steep {
                self.put(yi as i32, x, color, p);
                self.put(yi as i32 + 1, x, color, q);
            } else {
                self.put(x, yi as i32, color, p);
                self.put(x, yi as i32 + 1, color, q);
            }
        }
    }

    /// Filled anti-aliased circle.
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        let x0 = (cx - r - 1.0).floor() as i32;
        let y0 = (cy - r - 1.0).floor() as i32;
        let x1 = (cx + r + 1.0).ceil() as i32;
        let y1 = (cy + r + 1.0).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = (r - d + 0.5).clamp(0.0, 1.0);
                if a > 0.0 {
                    self.put(x, y, color, (a * 255.0) as u32);
                }
            }
        }
    }
    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        let x0 = (cx - r - 2.0).floor() as i32;
        let y0 = (cy - r - 2.0).floor() as i32;
        for y in y0..=(cy + r + 2.0).ceil() as i32 {
            for x in x0..=(cx + r + 2.0).ceil() as i32 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = (1.0 - (d - r).abs()).clamp(0.0, 1.0);
                if a > 0.0 {
                    self.put(x, y, color, (a * 255.0) as u32);
                }
            }
        }
    }

    /// A small filled triangle pointing down (`down`) or right, centred at (cx, cy).
    /// Used for dropdown arrows, panel disclosure and submenu markers.
    pub fn arrow(&mut self, cx: f32, cy: f32, size: f32, down: bool, color: u32) {
        let h = size * 0.5;
        let (a, b, c) = if down { ((cx - h, cy - h * 0.5), (cx + h, cy - h * 0.5), (cx, cy + h * 0.6)) } else { ((cx - h * 0.5, cy - h), (cx - h * 0.5, cy + h), (cx + h * 0.6, cy)) };
        let (x0, x1) = (a.0.min(b.0).min(c.0).floor() as i32, a.0.max(b.0).max(c.0).ceil() as i32);
        let (y0, y1) = (a.1.min(b.1).min(c.1).floor() as i32, a.1.max(b.1).max(c.1).ceil() as i32);
        let edge = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
        let area = edge(a, b, c.0, c.1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                // 4x4 supersampling for smooth edges.
                let mut cover = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (px, py) = (x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                        let (e0, e1, e2) = (edge(a, b, px, py), edge(b, c, px, py), edge(c, a, px, py));
                        if (e0 * area >= 0.0) && (e1 * area >= 0.0) && (e2 * area >= 0.0) {
                            cover += 1;
                        }
                    }
                }
                self.put(x, y, color, cover * 255 / 16);
            }
        }
    }
    /// A diamond (the app menu's mark).
    pub fn diamond(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                let d = (x as f32 + 0.5 - cx).abs() + (y as f32 + 0.5 - cy).abs();
                let a = (r - d + 0.5).clamp(0.0, 1.0);
                self.put(x, y, color, (a * 255.0) as u32);
            }
        }
    }

    pub fn text_width(&mut self, s: &str) -> i32 {
        self.pen.measure(s)
    }
    pub fn line_height(&self) -> i32 {
        self.pen.line_height()
    }

    /// Text with its top-left at (x, y). Returns the end x.
    pub fn text(&mut self, x: i32, y: i32, s: &str, color: u32) -> i32 {
        let clip = self.clip;
        if clip.is_empty() || y > clip.bottom() || y + self.pen.line_height() < clip.y {
            return x;
        }
        let (w, h) = (self.width, self.height);
        // Pen draws through a Canvas; vertical clipping is done by painting into the
        // full buffer then restoring outside rows would be costly, so we clip rows by
        // painting only when the line fits (UI text always sits inside its rows).
        let mut canvas = Canvas::new(&mut self.pixels, w, h);
        self.pen.text(&mut canvas, x, y, s, color, Some((clip.x, clip.w)))
    }
    pub fn mono_text(&mut self, x: i32, y: i32, s: &str, color: u32) -> i32 {
        let clip = self.clip;
        if clip.is_empty() || y > clip.bottom() || y + self.mono.line_height() < clip.y {
            return x;
        }
        let (w, h) = (self.width, self.height);
        let mut canvas = Canvas::new(&mut self.pixels, w, h);
        self.mono.text(&mut canvas, x, y, s, color, Some((clip.x, clip.w)))
    }
    /// Text aligned in `r`, vertically centred, cut with "…" when too wide.
    pub fn label(&mut self, r: Rect, s: &str, color: u32, align: Align) {
        let lh = self.pen.line_height();
        let y = r.y + (r.h - lh) / 2;
        let mut text = s.to_string();
        let mut w = self.pen.measure(&text);
        if w > r.w {
            while !text.is_empty() && self.pen.measure(&format!("{text}...")) > r.w {
                text.pop();
            }
            text.push_str("...");
            w = self.pen.measure(&text);
        }
        let x = match align {
            Align::Left => r.x,
            Align::Center => r.x + (r.w - w) / 2,
            Align::Right => r.right() - w,
        };
        self.push_clip(r);
        self.text(x, y, &text, color);
        self.pop_clip();
    }

    /// Copy an RGBA8 image (e.g. the 3D viewport) into `r`.
    pub fn blit_rgba(&mut self, r: Rect, rgba: &[u8], w: usize, h: usize) {
        let c = r.intersect(&self.clip);
        for y in c.y..c.bottom() {
            let sy = (y - r.y) as usize;
            if sy >= h {
                break;
            }
            let row = y as usize * self.width;
            for x in c.x..c.right() {
                let sx = (x - r.x) as usize;
                if sx >= w {
                    break;
                }
                let i = (sy * w + sx) * 4;
                self.pixels[row + x as usize] = (rgba[i] as u32) << 16 | (rgba[i + 1] as u32) << 8 | rgba[i + 2] as u32;
            }
        }
    }

    /// RGBA8 copy of the whole window, for screenshots and tests.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &p in &self.pixels {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]);
        }
        out
    }
}
