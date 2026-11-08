//! Anti-aliased drawing for SexOS window pixel buffers.
//!
//! A thin, SexOS-shaped layer over [tiny-skia](https://github.com/linebender/tiny-skia)
//! (BSD-3-Clause): rounded rectangles, circles, lines, arbitrary paths,
//! gradients, analytic soft shadows and a complete window frame.
//!
//! # Pixel format
//!
//! SexOS pixels are `u32` words `0xAARRGGBB`, so a little-endian buffer holds
//! the bytes `B, G, R, A`. tiny-skia writes premultiplied `R, G, B, A`. Every
//! colour handed to tiny-skia here has red and blue swapped, so tiny-skia
//! draws straight into SexOS words with no copy and no conversion pass.
//! Words in a [`Canvas`] are premultiplied; an opaque window (alpha `0xFF`)
//! is identical either way.
//!
//! # Memory
//!
//! tiny-skia needs `alloc` (path building and rasterising use `Vec`). A
//! program that links this crate must provide a `#[global_allocator]`.
//! [`Canvas`] borrows a caller-owned buffer; [`Layer`] owns one and can be
//! composited into any strided buffer.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

pub use tiny_skia;
use tiny_skia::{
    FillRule, GradientStop, LinearGradient, Paint, PathBuilder, PixmapMut, Point, Shader, SpreadMode,
    Stroke, Transform,
};
pub use tiny_skia::{LineCap, Path};

/// A straight-alpha SexOS colour, `0xAARRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const TRANSPARENT: Color = Color(0);
    pub const BLACK: Color = Color(0xFF00_0000);
    pub const WHITE: Color = Color(0xFFFF_FFFF);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color((a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }
    pub const fn a(self) -> u8 { (self.0 >> 24) as u8 }
    pub const fn r(self) -> u8 { (self.0 >> 16) as u8 }
    pub const fn g(self) -> u8 { (self.0 >> 8) as u8 }
    pub const fn b(self) -> u8 { self.0 as u8 }

    /// The same colour with its alpha replaced.
    pub const fn with_alpha(self, a: u8) -> Self {
        Color(self.0 & 0x00FF_FFFF | (a as u32) << 24)
    }

    /// The premultiplied word as it lands in a [`Canvas`].
    pub const fn premultiplied(self) -> u32 {
        const fn m(c: u8, a: u32) -> u32 { (c as u32 * a + 127) / 255 }
        let a = self.a() as u32;
        a << 24 | m(self.r(), a) << 16 | m(self.g(), a) << 8 | m(self.b(), a)
    }

    /// tiny-skia colour with red and blue swapped (see the module docs).
    fn skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.b(), self.g(), self.r(), self.a())
    }
}

/// An axis-aligned rectangle in pixels. Fractional edges are anti-aliased.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }
    /// Grown by `d` on every side (shrunk when negative).
    pub fn inset(&self, d: f32) -> Self {
        Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// Corner radii, clockwise from top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Radii {
    pub const fn all(r: f32) -> Self {
        Radii { top_left: r, top_right: r, bottom_right: r, bottom_left: r }
    }
    pub const fn top(r: f32) -> Self {
        Radii { top_left: r, top_right: r, bottom_right: 0.0, bottom_left: 0.0 }
    }
    pub const fn bottom(r: f32) -> Self {
        Radii { top_left: 0.0, top_right: 0.0, bottom_right: r, bottom_left: r }
    }
}

impl From<f32> for Radii {
    fn from(r: f32) -> Self { Radii::all(r) }
}

/// Cubic Bézier control distance for a quarter circle.
const KAPPA: f32 = 0.552_284_8;

/// A rounded-rectangle path, or `None` for an empty rectangle.
pub fn round_rect_path(rect: Rect, radii: impl Into<Radii>) -> Option<Path> {
    if !(rect.w > 0.0 && rect.h > 0.0) {
        return None;
    }
    let max = rect.w.min(rect.h) / 2.0;
    let r = radii.into();
    let (tl, tr, br, bl) = (
        r.top_left.clamp(0.0, max),
        r.top_right.clamp(0.0, max),
        r.bottom_right.clamp(0.0, max),
        r.bottom_left.clamp(0.0, max),
    );
    let (l, t, rt, b) = (rect.x, rect.y, rect.right(), rect.bottom());
    let k = KAPPA;
    let mut p = PathBuilder::new();
    p.move_to(l + tl, t);
    p.line_to(rt - tr, t);
    if tr > 0.0 {
        p.cubic_to(rt - tr + tr * k, t, rt, t + tr - tr * k, rt, t + tr);
    }
    p.line_to(rt, b - br);
    if br > 0.0 {
        p.cubic_to(rt, b - br + br * k, rt - br + br * k, b, rt - br, b);
    }
    p.line_to(l + bl, b);
    if bl > 0.0 {
        p.cubic_to(l + bl - bl * k, b, l, b - bl + bl * k, l, b - bl);
    }
    p.line_to(l, t + tl);
    if tl > 0.0 {
        p.cubic_to(l, t + tl - tl * k, l + tl - tl * k, t, l + tl, t);
    }
    p.close();
    p.finish()
}

fn solid(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.skia());
    paint.anti_alias = true;
    paint
}

/// Reinterpret SexOS words as the byte slice tiny-skia draws into.
fn as_bytes(words: &mut [u32]) -> &mut [u8] {
    // SAFETY: u8 has no alignment or validity requirements and the length
    // covers exactly the same memory.
    unsafe { core::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), words.len() * 4) }
}

/// Premultiplied source-over of one word onto another.
#[inline]
fn over(src: u32, dst: u32) -> u32 {
    let sa = src >> 24;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return dst;
    }
    let inv = 255 - sa;
    let mut out = 0;
    for shift in [0, 8, 16, 24] {
        let s = (src >> shift) & 255;
        let d = (dst >> shift) & 255;
        out |= (s + (d * inv + 127) / 255).min(255) << shift;
    }
    out
}

/// Drawing surface over a borrowed, contiguous (`stride == width`) buffer
/// of SexOS words.
pub struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: u32,
    height: u32,
}

impl<'a> Canvas<'a> {
    /// `None` when the buffer is too small or a side is zero.
    pub fn new(pixels: &'a mut [u32], width: u32, height: u32) -> Option<Self> {
        let len = (width as usize).checked_mul(height as usize)?;
        if width == 0 || height == 0 || pixels.len() < len {
            return None;
        }
        Some(Canvas { pixels: &mut pixels[..len], width, height })
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn pixels(&self) -> &[u32] { self.pixels }
    pub fn pixels_mut(&mut self) -> &mut [u32] { self.pixels }

    /// The premultiplied word at `(x, y)`, or `None` outside the canvas.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        (x < self.width && y < self.height).then(|| self.pixels[(y * self.width + x) as usize])
    }

    fn skia(&mut self) -> PixmapMut<'_> {
        let (w, h) = (self.width, self.height);
        PixmapMut::from_bytes(as_bytes(self.pixels), w, h).expect("size checked in Canvas::new")
    }

    /// Replace every pixel (no blending).
    pub fn clear(&mut self, color: Color) {
        self.pixels.fill(color.premultiplied());
    }

    pub fn fill_path(&mut self, path: &Path, color: Color) {
        self.skia().fill_path(path, &solid(color), FillRule::Winding, Transform::identity(), None);
    }

    pub fn stroke_path(&mut self, path: &Path, width: f32, color: Color) {
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Stroke::default() };
        self.skia().stroke_path(path, &solid(color), &stroke, Transform::identity(), None);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(r) = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.w, rect.h) {
            self.skia().fill_rect(r, &solid(color), Transform::identity(), None);
        }
    }

    pub fn fill_round_rect(&mut self, rect: Rect, radii: impl Into<Radii>, color: Color) {
        if let Some(path) = round_rect_path(rect, radii) {
            self.fill_path(&path, color);
        }
    }

    /// Outline centred on the rectangle's edge.
    pub fn stroke_round_rect(&mut self, rect: Rect, radii: impl Into<Radii>, width: f32, color: Color) {
        if let Some(path) = round_rect_path(rect, radii) {
            self.stroke_path(&path, width, color);
        }
    }

    /// Fill with a vertical gradient from `top` to `bottom`.
    pub fn fill_round_rect_gradient(&mut self, rect: Rect, radii: impl Into<Radii>, top: Color, bottom: Color) {
        let Some(path) = round_rect_path(rect, radii) else { return };
        let shader = LinearGradient::new(
            Point::from_xy(rect.x, rect.y),
            Point::from_xy(rect.x, rect.bottom()),
            vec![GradientStop::new(0.0, top.skia()), GradientStop::new(1.0, bottom.skia())],
            SpreadMode::Pad,
            Transform::identity(),
        );
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.shader = match shader {
            Some(s) => s,
            // Degenerate gradient (zero height or equal colours): solid top.
            None => Shader::SolidColor(top.skia()),
        };
        self.skia().fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
        PathBuilder::from_circle(cx, cy, r)
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        if let Some(path) = Self::circle(cx, cy, r) {
            self.fill_path(&path, color);
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, width: f32, color: Color) {
        if let Some(path) = Self::circle(cx, cy, r) {
            self.stroke_path(&path, width, color);
        }
    }

    /// A round-capped line.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Color) {
        let mut p = PathBuilder::new();
        p.move_to(x0, y0);
        p.line_to(x1, y1);
        if let Some(path) = p.finish() {
            self.stroke_path(&path, width, color);
        }
    }

    /// A soft drop shadow under a rounded rectangle.
    ///
    /// Analytic (no blur pass, no scratch buffer): each pixel's coverage is
    /// a Gaussian falloff of its distance to the rounded rectangle, with
    /// `blur` the distance over which it fades (about two standard
    /// deviations). Draws only outside-and-under; paint the body afterwards.
    pub fn shadow(&mut self, rect: Rect, radius: f32, blur: f32, color: Color) {
        if !(rect.w > 0.0 && rect.h > 0.0) || color.a() == 0 {
            return;
        }
        let sigma = (blur / 2.0).max(0.5);
        let reach = 3.0 * sigma;
        let x0 = libm::floorf(rect.x - reach).max(0.0) as u32;
        let y0 = libm::floorf(rect.y - reach).max(0.0) as u32;
        let x1 = (libm::ceilf(rect.right() + reach).max(0.0) as u32).min(self.width);
        let y1 = (libm::ceilf(rect.bottom() + reach).max(0.0) as u32).min(self.height);
        let r = radius.clamp(0.0, rect.w.min(rect.h) / 2.0);
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let (hx, hy) = (rect.w / 2.0 - r, rect.h / 2.0 - r);
        let scale = 1.0 / (sigma * core::f32::consts::SQRT_2);
        let alpha = color.a() as f32;
        for y in y0..y1 {
            let qy = libm::fabsf(y as f32 + 0.5 - cy) - hy;
            let row = (y * self.width) as usize;
            for x in x0..x1 {
                let qx = libm::fabsf(x as f32 + 0.5 - cx) - hx;
                let outside = libm::sqrtf(qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0));
                let d = outside + qx.max(qy).min(0.0) - r;
                let a = (alpha * 0.5 * libm::erfcf(d * scale) + 0.5) as u32;
                if a == 0 {
                    continue;
                }
                let src = color.with_alpha(a.min(255) as u8).premultiplied();
                let px = &mut self.pixels[row + x as usize];
                *px = over(src, *px);
            }
        }
    }

    /// Draw a complete window frame (shadow, body, title bar, hairline and
    /// the three lights) and return the content rectangle below the title
    /// bar. `rect` is the window's outer edge; the shadow spills outside it.
    pub fn window(&mut self, rect: Rect, style: &WindowStyle, active: bool) -> Rect {
        let s = style;
        let shadow = if active { s.shadow } else { s.shadow.with_alpha(s.shadow.a() / 2) };
        self.shadow(rect.offset(0.0, s.shadow_offset), s.radius, s.shadow_blur, shadow);
        self.fill_round_rect(rect, s.radius, s.body);
        let bar = Rect::new(rect.x, rect.y, rect.w, s.title_height);
        let (top, bottom) = if active { (s.title_top, s.title_bottom) } else { (s.title_bottom, s.title_bottom) };
        self.fill_round_rect_gradient(bar, Radii::top(s.radius), top, bottom);
        self.fill_rect(Rect::new(rect.x, bar.bottom() - 1.0, rect.w, 1.0), s.separator);
        self.stroke_round_rect(rect.inset(0.5), s.radius - 0.5, 1.0, s.border);
        let cy = rect.y + s.title_height / 2.0;
        let mut cx = rect.x + s.light_inset + s.light_radius;
        for light in s.lights {
            let light = if active { light } else { s.light_idle };
            self.fill_circle(cx, cy, s.light_radius, light);
            self.stroke_circle(cx, cy, s.light_radius - 0.25, 0.5, Color::rgba(0, 0, 0, 40));
            cx += s.light_radius * 2.0 + s.light_gap;
        }
        Rect::new(rect.x, bar.bottom(), rect.w, rect.h - s.title_height)
    }
}

/// Look of [`Canvas::window`].
#[derive(Clone, Copy, Debug)]
pub struct WindowStyle {
    pub radius: f32,
    pub title_height: f32,
    pub body: Color,
    pub title_top: Color,
    pub title_bottom: Color,
    pub separator: Color,
    pub border: Color,
    pub shadow: Color,
    pub shadow_blur: f32,
    pub shadow_offset: f32,
    /// Close, minimise, zoom.
    pub lights: [Color; 3],
    pub light_idle: Color,
    pub light_radius: f32,
    pub light_gap: f32,
    pub light_inset: f32,
}

impl Default for WindowStyle {
    fn default() -> Self {
        WindowStyle {
            radius: 10.0,
            title_height: 30.0,
            body: Color::rgb(0xFA, 0xFA, 0xFB),
            title_top: Color::rgb(0xEE, 0xEE, 0xF1),
            title_bottom: Color::rgb(0xE2, 0xE2, 0xE6),
            separator: Color::rgba(0, 0, 0, 30),
            border: Color::rgba(0, 0, 0, 60),
            shadow: Color::rgba(0, 0, 0, 110),
            shadow_blur: 24.0,
            shadow_offset: 8.0,
            lights: [Color::rgb(0xFF, 0x5F, 0x57), Color::rgb(0xFE, 0xBC, 0x2E), Color::rgb(0x28, 0xC8, 0x40)],
            light_idle: Color::rgb(0xCC, 0xCC, 0xD0),
            light_radius: 6.0,
            light_gap: 8.0,
            light_inset: 12.0,
        }
    }
}

/// An owned, premultiplied drawing buffer that can be composited into any
/// strided SexOS buffer (a window surface, the framebuffer).
pub struct Layer {
    pixels: Vec<u32>,
    width: u32,
    height: u32,
}

impl Layer {
    /// A fully transparent layer, or `None` for a zero or overflowing size.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let len = (width as usize).checked_mul(height as usize)?;
        if len == 0 {
            return None;
        }
        Some(Layer { pixels: vec![0; len], width, height })
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn pixels(&self) -> &[u32] { &self.pixels }

    pub fn canvas(&mut self) -> Canvas<'_> {
        Canvas::new(&mut self.pixels, self.width, self.height).expect("layer size is valid")
    }

    /// Source-over this layer onto `dst` with its top-left at `(x, y)`.
    /// `dst` is `dst_height` rows of `dst_stride` words, `dst_width` used;
    /// anything outside is clipped. Destination alpha bytes come out as
    /// premultiplied alpha, so an opaque destination stays opaque.
    pub fn composite(&self, dst: &mut [u32], dst_width: u32, dst_height: u32, dst_stride: u32, x: i32, y: i32) {
        let rows = dst.len() / dst_stride.max(1) as usize;
        let dst_height = dst_height.min(rows as u32);
        let dst_width = dst_width.min(dst_stride);
        let sx0 = (-x).max(0) as u32;
        let sy0 = (-y).max(0) as u32;
        let dx0 = x.max(0) as u32;
        let dy0 = y.max(0) as u32;
        if sx0 >= self.width || sy0 >= self.height || dx0 >= dst_width || dy0 >= dst_height {
            return;
        }
        let w = (self.width - sx0).min(dst_width - dx0) as usize;
        let h = (self.height - sy0).min(dst_height - dy0);
        for row in 0..h {
            let s = ((sy0 + row) * self.width + sx0) as usize;
            let d = ((dy0 + row) * dst_stride + dx0) as usize;
            for (src, out) in self.pixels[s..s + w].iter().zip(&mut dst[d..d + w]) {
                *out = over(*src, *out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::rgb(255, 0, 0);

    fn canvas_buf(w: u32, h: u32) -> Vec<u32> {
        vec![0; (w * h) as usize]
    }

    #[test]
    fn colours_land_as_sexos_words() {
        let mut buf = canvas_buf(4, 4);
        let mut c = Canvas::new(&mut buf, 4, 4).unwrap();
        c.fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), RED);
        assert!(buf.iter().all(|&p| p == 0xFFFF_0000), "{:08x}", buf[0]);
        let mut c = Canvas::new(&mut buf, 4, 4).unwrap();
        c.fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(0x12, 0x34, 0x56));
        assert_eq!(buf[5], 0xFF12_3456);
    }

    #[test]
    fn too_small_buffer_is_refused() {
        let mut buf = canvas_buf(3, 3);
        assert!(Canvas::new(&mut buf, 4, 4).is_none());
        assert!(Canvas::new(&mut buf, 0, 3).is_none());
        assert!(Layer::new(0, 5).is_none());
    }

    #[test]
    fn rounded_corners_are_cut_and_anti_aliased() {
        let mut buf = canvas_buf(40, 40);
        let mut c = Canvas::new(&mut buf, 40, 40).unwrap();
        c.fill_round_rect(Rect::new(0.0, 0.0, 40.0, 40.0), 12.0, RED);
        // Corner pixels outside the arc stay empty, the middle is solid.
        assert_eq!(c.pixel(0, 0), Some(0));
        assert_eq!(c.pixel(39, 39), Some(0));
        assert_eq!(c.pixel(20, 20), Some(0xFFFF_0000));
        assert_eq!(c.pixel(20, 0), Some(0xFFFF_0000));
        // Some pixels on the arc have partial coverage (anti-aliasing).
        let partial = (0..12).flat_map(|y| (0..12).map(move |x| (x, y)))
            .filter_map(|(x, y)| c.pixel(x, y))
            .filter(|p| { let a = p >> 24; a > 0 && a < 255 })
            .count();
        assert!(partial >= 8, "only {partial} partially covered corner pixels");
    }

    #[test]
    fn per_corner_radii() {
        let mut buf = canvas_buf(30, 30);
        let mut c = Canvas::new(&mut buf, 30, 30).unwrap();
        c.fill_round_rect(Rect::new(0.0, 0.0, 30.0, 30.0), Radii::top(10.0), RED);
        assert_eq!(c.pixel(0, 0), Some(0));
        assert_eq!(c.pixel(29, 0), Some(0));
        assert_eq!(c.pixel(0, 29), Some(0xFFFF_0000));
        assert_eq!(c.pixel(29, 29), Some(0xFFFF_0000));
    }

    #[test]
    fn circle_edges_blend_over_background() {
        let mut buf = canvas_buf(32, 32);
        let mut c = Canvas::new(&mut buf, 32, 32).unwrap();
        c.clear(Color::WHITE);
        c.fill_circle(16.0, 16.0, 10.0, Color::BLACK);
        assert_eq!(c.pixel(16, 16), Some(0xFF00_0000));
        assert_eq!(c.pixel(0, 0), Some(0xFFFF_FFFF));
        // Edge pixels are grey and stay opaque.
        let greys = c.pixels().iter().filter(|&&p| p >> 24 == 255 && (p & 0xFF) > 0 && (p & 0xFF) < 255).count();
        assert!(greys > 20);
        assert!(c.pixels().iter().all(|&p| p >> 24 == 255));
    }

    #[test]
    fn line_is_drawn_between_endpoints() {
        let mut buf = canvas_buf(20, 20);
        let mut c = Canvas::new(&mut buf, 20, 20).unwrap();
        c.line(2.0, 10.0, 18.0, 10.0, 2.0, RED);
        assert_eq!(c.pixel(10, 9), Some(0xFFFF_0000));
        assert_eq!(c.pixel(10, 10), Some(0xFFFF_0000));
        assert_eq!(c.pixel(10, 2), Some(0));
    }

    #[test]
    fn gradient_runs_top_to_bottom() {
        let mut buf = canvas_buf(10, 100);
        let mut c = Canvas::new(&mut buf, 10, 100).unwrap();
        c.fill_round_rect_gradient(Rect::new(0.0, 0.0, 10.0, 100.0), 0.0, Color::BLACK, Color::WHITE);
        let top = c.pixel(5, 1).unwrap() & 0xFF;
        let mid = c.pixel(5, 50).unwrap() & 0xFF;
        let bottom = c.pixel(5, 98).unwrap() & 0xFF;
        assert!(top < 10 && bottom > 245 && (110..145).contains(&mid), "{top} {mid} {bottom}");
    }

    #[test]
    fn shadow_fades_with_distance() {
        let mut buf = canvas_buf(100, 100);
        let mut c = Canvas::new(&mut buf, 100, 100).unwrap();
        c.clear(Color::WHITE);
        c.shadow(Rect::new(30.0, 30.0, 40.0, 40.0), 6.0, 16.0, Color::rgba(0, 0, 0, 200));
        let at = |x| 255 - (c.pixel(x, 50).unwrap() & 0xFF);
        // Inside: full shadow alpha. At the edge: about half. Outward: falling to nothing.
        assert!((195..=201).contains(&at(50)), "{}", at(50));
        assert!((90..=110).contains(&at(30)), "{}", at(30));
        assert!(at(30) > at(25) && at(25) > at(20) && at(20) > at(16));
        assert_eq!(at(2), 0);
        assert!(c.pixels().iter().all(|&p| p >> 24 == 255));
        // Symmetric left/right.
        assert_eq!(at(25), at(74));
    }

    #[test]
    fn window_frame_has_lights_and_content_area() {
        let (w, h) = (300, 200);
        let mut buf = canvas_buf(w, h);
        let mut c = Canvas::new(&mut buf, w, h).unwrap();
        c.clear(Color::rgb(0x30, 0x40, 0x60));
        let style = WindowStyle::default();
        let content = c.window(Rect::new(40.0, 30.0, 220.0, 140.0), &style, true);
        assert_eq!(content, Rect::new(40.0, 60.0, 220.0, 110.0));
        // Close light centre is the close colour.
        let cx = (40.0 + style.light_inset + style.light_radius) as u32;
        assert_eq!(c.pixel(cx, 45), Some(style.lights[0].0));
        // Body is the body colour; the outer corner is cut away, showing
        // desktop (slightly shaded), not window.
        assert_eq!(c.pixel(150, 120), Some(style.body.0));
        let corner = c.pixel(40, 30).unwrap();
        assert!(corner & 0xFF <= 0x60 && corner >> 24 == 255, "{corner:08x}");
        // Shadow darkens the desktop below the window.
        assert!(c.pixel(150, 175).unwrap() & 0xFF < 0x60);
    }

    #[test]
    fn layer_composites_with_clipping_and_stride() {
        let mut layer = Layer::new(10, 10).unwrap();
        layer.canvas().fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgba(255, 0, 0, 128));
        // 8x8 visible area inside 12-word rows, pre-filled opaque blue.
        let mut dst = vec![0xFF00_00FFu32; 12 * 8];
        layer.composite(&mut dst, 8, 8, 12, 4, -3);
        // Clipped area untouched.
        assert_eq!(dst[0], 0xFF00_00FF);
        assert_eq!(dst[8], 0xFF00_00FF, "padding beyond dst_width is not drawn");
        // Covered: half red over blue, still opaque.
        let p = dst[4];
        assert_eq!(p >> 24, 255);
        assert!((126..=130).contains(&((p >> 16) & 255)) && (125..=129).contains(&(p & 255)), "{p:08x}");
        // Rows 0..7 covered (layer rows 3..10 => 7 rows), row 7 not.
        assert_ne!(dst[6 * 12 + 4], 0xFF00_00FF);
        assert_eq!(dst[7 * 12 + 4], 0xFF00_00FF);
        // Fully outside does nothing.
        layer.composite(&mut dst, 8, 8, 12, 100, 0);
        layer.composite(&mut dst, 8, 8, 12, -20, 0);
    }
}
