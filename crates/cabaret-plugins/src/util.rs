//! Internal `no_std` math, linear-light color, 2D rasterization, and bitmap text helpers
//! shared across `cabaret-plugins` modules.

use cabaret_core::compositor::{blend_pixel, BlendMode, FrameBuffer, LinearRgba, MixColorSpace};
use core::f32::consts::PI;

#[inline]
pub(crate) fn absf(x: f32) -> f32 {
    libm::fabsf(x)
}

#[inline]
pub(crate) fn floorf(x: f32) -> f32 {
    libm::floorf(x)
}

#[inline]
pub(crate) fn ceilf(x: f32) -> f32 {
    libm::ceilf(x)
}

#[inline]
pub(crate) fn roundf(x: f32) -> f32 {
    libm::roundf(x)
}

#[inline]
pub(crate) fn sqrtf(x: f32) -> f32 {
    libm::sqrtf(x.max(0.0))
}

#[inline]
pub(crate) fn sinf(x: f32) -> f32 {
    libm::sinf(x)
}

#[inline]
pub(crate) fn cosf(x: f32) -> f32 {
    libm::cosf(x)
}

#[inline]
pub(crate) fn tanf(x: f32) -> f32 {
    libm::tanf(x)
}

#[inline]
pub(crate) fn atan2f(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

#[inline]
pub(crate) fn powf(base: f32, exp: f32) -> f32 {
    libm::powf(base, exp)
}

#[inline]
pub(crate) fn expf(x: f32) -> f32 {
    libm::expf(x)
}

#[inline]
pub(crate) fn log10f(x: f32) -> f32 {
    libm::log10f(x)
}

#[inline]
pub(crate) fn clamp01(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub(crate) fn fract(x: f32) -> f32 {
    x - floorf(x)
}

#[inline]
pub(crate) fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let span = edge1 - edge0;
    if absf(span) < 1e-7 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = clamp01((x - edge0) / span);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub(crate) fn rec709_luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

#[inline]
pub(crate) fn hash2d(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16_777_215.0
}

#[inline]
pub(crate) fn deg_to_rad(deg: f32) -> f32 {
    deg * (PI / 180.0)
}

/// Blends `src` over pixel `(x, y)` in `fb` using Porter-Duff Normal linear blending.
#[inline]
pub(crate) fn blend_over(fb: &mut FrameBuffer, x: i32, y: i32, src: LinearRgba) {
    if x < 0 || y < 0 {
        return;
    }
    let ux = x as u32;
    let uy = y as u32;
    if let Some(dst) = fb.get_pixel(ux, uy) {
        let out = blend_pixel(dst, src, BlendMode::Normal, 1.0, MixColorSpace::Linear);
        fb.set_pixel(ux, uy, out);
    }
}

/// Fills an axis-aligned rectangle `(x, y, w, h)` with `color` blended over `fb`.
pub(crate) fn fill_rect(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, color: LinearRgba) {
    if w <= 0 || h <= 0 || color[3] <= 0.0 {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(fb.width as i32);
    let y1 = (y + h).min(fb.height as i32);
    for py in y0..y1 {
        for px in x0..x1 {
            blend_over(fb, px, py, color);
        }
    }
}

/// Strokes a 1px+ outline rectangle `(x, y, w, h)`.
pub(crate) fn stroke_rect(
    fb: &mut FrameBuffer,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    thickness: i32,
    color: LinearRgba,
) {
    let t = thickness.max(1);
    fill_rect(fb, x, y, w, t, color);
    fill_rect(fb, x, y + h - t, w, t, color);
    fill_rect(fb, x, y + t, t, (h - 2 * t).max(0), color);
    fill_rect(fb, x + w - t, y + t, t, (h - 2 * t).max(0), color);
}

/// Fills a rounded rectangle with smooth anti-aliased corner coverage.
pub(crate) fn fill_rounded_rect(
    fb: &mut FrameBuffer,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    color: LinearRgba,
) {
    if w <= 0.0 || h <= 0.0 || color[3] <= 0.0 {
        return;
    }
    let r = radius.clamp(0.0, 0.5 * w.min(h));
    let x0 = (floorf(x) as i32).max(0);
    let y0 = (floorf(y) as i32).max(0);
    let x1 = (ceilf(x + w) as i32).min(fb.width as i32);
    let y1 = (ceilf(y + h) as i32).min(fb.height as i32);
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let hx = w * 0.5 - r;
    let hy = h * 0.5 - r;

    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        let qy = (absf(fy - cy) - hy).max(0.0);
        for px in x0..x1 {
            let fx = px as f32 + 0.5;
            let qx = (absf(fx - cx) - hx).max(0.0);
            let dist = sqrtf(qx * qx + qy * qy) - r;
            let cov = clamp01(0.5 - dist);
            if cov > 0.0 {
                let mut c = color;
                c[3] *= cov;
                blend_over(fb, px, py, c);
            }
        }
    }
}

/// Fills an anti-aliased disc centered at `(cx, cy)` with `radius`.
pub(crate) fn fill_circle(
    fb: &mut FrameBuffer,
    cx: f32,
    cy: f32,
    radius: f32,
    color: LinearRgba,
) {
    if radius <= 0.0 || color[3] <= 0.0 {
        return;
    }
    let x0 = (floorf(cx - radius - 1.0) as i32).max(0);
    let y0 = (floorf(cy - radius - 1.0) as i32).max(0);
    let x1 = (ceilf(cx + radius + 1.0) as i32).min(fb.width as i32);
    let y1 = (ceilf(cy + radius + 1.0) as i32).min(fb.height as i32);
    for py in y0..y1 {
        let dy = (py as f32 + 0.5) - cy;
        for px in x0..x1 {
            let dx = (px as f32 + 0.5) - cx;
            let dist = sqrtf(dx * dx + dy * dy) - radius;
            let cov = clamp01(0.5 - dist);
            if cov > 0.0 {
                let mut c = color;
                c[3] *= cov;
                blend_over(fb, px, py, c);
            }
        }
    }
}

/// Draws a thick anti-aliased line segment from `(x0, y0)` to `(x1, y1)`.
pub(crate) fn draw_line(
    fb: &mut FrameBuffer,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    color: LinearRgba,
) {
    if color[3] <= 0.0 {
        return;
    }
    let half_t = (thickness * 0.5).max(0.5);
    let min_x = (floorf(x0.min(x1) - half_t - 1.0) as i32).max(0);
    let min_y = (floorf(y0.min(y1) - half_t - 1.0) as i32).max(0);
    let max_x = (ceilf(x0.max(x1) + half_t + 1.0) as i32).min(fb.width as i32);
    let max_y = (ceilf(y0.max(y1) + half_t + 1.0) as i32).min(fb.height as i32);

    let vx = x1 - x0;
    let vy = y1 - y0;
    let len2 = vx * vx + vy * vy;

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let dist = if len2 <= 1e-6 {
                let dx = fx - x0;
                let dy = fy - y0;
                sqrtf(dx * dx + dy * dy)
            } else {
                let t = clamp01(((fx - x0) * vx + (fy - y0) * vy) / len2);
                let proj_x = x0 + t * vx;
                let proj_y = y0 + t * vy;
                let dx = fx - proj_x;
                let dy = fy - proj_y;
                sqrtf(dx * dx + dy * dy)
            };
            let cov = clamp01(half_t + 0.5 - dist);
            if cov > 0.0 {
                let mut c = color;
                c[3] *= cov;
                blend_over(fb, px, py, c);
            }
        }
    }
}

/// Measures width in pixels of `text` drawn at integer `scale`.
pub(crate) fn measure_text_width(text: &str, scale: u32) -> u32 {
    let s = scale.max(1);
    let count = text.chars().count() as u32;
    if count == 0 {
        0
    } else {
        count * 6 * s - s
    }
}

/// Renders crisp embedded 5x7 ASCII bitmap text onto `fb`.
pub(crate) fn draw_text(
    fb: &mut FrameBuffer,
    x: i32,
    y: i32,
    text: &str,
    scale: u32,
    color: LinearRgba,
) {
    let s = scale.max(1) as i32;
    let mut cx = x;
    for ch in text.chars() {
        let glyph = glyph_5x7(ch);
        for (col_idx, &col_bits) in glyph.iter().enumerate() {
            for row_idx in 0..7 {
                if (col_bits >> row_idx) & 1 != 0 {
                    fill_rect(
                        fb,
                        cx + (col_idx as i32) * s,
                        y + row_idx * s,
                        s,
                        s,
                        color,
                    );
                }
            }
        }
        cx += 6 * s;
    }
}

/// 5x7 column-major bitmap font for printable ASCII `0x20..=0x7E`.
/// Bit 0 is the top row, bit 6 is the bottom row.
fn glyph_5x7(ch: char) -> [u8; 5] {
    let idx = match ch as u32 {
        0x20..=0x7E => (ch as u32 - 0x20) as usize,
        _ => (b'?' - 0x20) as usize,
    };
    FONT_5X7[idx]
}

const FONT_5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // ' '
    [0x00, 0x00, 0x5F, 0x00, 0x00], // '!'
    [0x00, 0x07, 0x00, 0x07, 0x00], // '"'
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // '#'
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // '$'
    [0x23, 0x13, 0x08, 0x64, 0x62], // '%'
    [0x36, 0x49, 0x55, 0x22, 0x50], // '&'
    [0x00, 0x05, 0x03, 0x00, 0x00], // '''
    [0x00, 0x1C, 0x22, 0x41, 0x00], // '('
    [0x00, 0x41, 0x22, 0x1C, 0x00], // ')'
    [0x14, 0x08, 0x3E, 0x08, 0x14], // '*'
    [0x08, 0x08, 0x3E, 0x08, 0x08], // '+'
    [0x00, 0x50, 0x30, 0x00, 0x00], // ','
    [0x08, 0x08, 0x08, 0x08, 0x08], // '-'
    [0x00, 0x60, 0x60, 0x00, 0x00], // '.'
    [0x20, 0x10, 0x08, 0x04, 0x02], // '/'
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // '0'
    [0x00, 0x42, 0x7F, 0x40, 0x00], // '1'
    [0x42, 0x61, 0x51, 0x49, 0x46], // '2'
    [0x21, 0x41, 0x45, 0x4B, 0x31], // '3'
    [0x18, 0x14, 0x12, 0x7F, 0x10], // '4'
    [0x27, 0x45, 0x45, 0x45, 0x39], // '5'
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // '6'
    [0x01, 0x71, 0x09, 0x05, 0x03], // '7'
    [0x36, 0x49, 0x49, 0x49, 0x36], // '8'
    [0x06, 0x49, 0x49, 0x29, 0x1E], // '9'
    [0x00, 0x36, 0x36, 0x00, 0x00], // ':'
    [0x00, 0x56, 0x36, 0x00, 0x00], // ';'
    [0x08, 0x14, 0x22, 0x41, 0x00], // '<'
    [0x14, 0x14, 0x14, 0x14, 0x14], // '='
    [0x00, 0x41, 0x22, 0x14, 0x08], // '>'
    [0x02, 0x01, 0x51, 0x09, 0x06], // '?'
    [0x32, 0x49, 0x79, 0x41, 0x3E], // '@'
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // 'A'
    [0x7F, 0x49, 0x49, 0x49, 0x36], // 'B'
    [0x3E, 0x41, 0x41, 0x41, 0x22], // 'C'
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // 'D'
    [0x7F, 0x49, 0x49, 0x49, 0x41], // 'E'
    [0x7F, 0x09, 0x09, 0x09, 0x01], // 'F'
    [0x3E, 0x41, 0x49, 0x49, 0x7A], // 'G'
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // 'H'
    [0x00, 0x41, 0x7F, 0x41, 0x00], // 'I'
    [0x20, 0x40, 0x41, 0x3F, 0x01], // 'J'
    [0x7F, 0x08, 0x14, 0x22, 0x41], // 'K'
    [0x7F, 0x40, 0x40, 0x40, 0x40], // 'L'
    [0x7F, 0x02, 0x0C, 0x02, 0x7F], // 'M'
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // 'N'
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // 'O'
    [0x7F, 0x09, 0x09, 0x09, 0x06], // 'P'
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // 'Q'
    [0x7F, 0x09, 0x19, 0x29, 0x46], // 'R'
    [0x46, 0x49, 0x49, 0x49, 0x31], // 'S'
    [0x01, 0x01, 0x7F, 0x01, 0x01], // 'T'
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // 'U'
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // 'V'
    [0x3F, 0x40, 0x38, 0x40, 0x3F], // 'W'
    [0x63, 0x14, 0x08, 0x14, 0x63], // 'X'
    [0x07, 0x08, 0x70, 0x08, 0x07], // 'Y'
    [0x61, 0x51, 0x49, 0x45, 0x43], // 'Z'
    [0x00, 0x7F, 0x41, 0x41, 0x00], // '['
    [0x02, 0x04, 0x08, 0x10, 0x20], // '\'
    [0x00, 0x41, 0x41, 0x7F, 0x00], // ']'
    [0x04, 0x02, 0x01, 0x02, 0x04], // '^'
    [0x40, 0x40, 0x40, 0x40, 0x40], // '_'
    [0x00, 0x01, 0x02, 0x04, 0x00], // '`'
    [0x20, 0x54, 0x54, 0x54, 0x78], // 'a'
    [0x7F, 0x48, 0x44, 0x44, 0x38], // 'b'
    [0x38, 0x44, 0x44, 0x44, 0x20], // 'c'
    [0x38, 0x44, 0x44, 0x48, 0x7F], // 'd'
    [0x38, 0x54, 0x54, 0x54, 0x18], // 'e'
    [0x08, 0x7E, 0x09, 0x01, 0x02], // 'f'
    [0x0C, 0x52, 0x52, 0x52, 0x3E], // 'g'
    [0x7F, 0x08, 0x04, 0x04, 0x78], // 'h'
    [0x00, 0x44, 0x7D, 0x40, 0x00], // 'i'
    [0x20, 0x40, 0x44, 0x3D, 0x00], // 'j'
    [0x7F, 0x10, 0x28, 0x44, 0x00], // 'k'
    [0x00, 0x41, 0x7F, 0x40, 0x00], // 'l'
    [0x7C, 0x04, 0x18, 0x04, 0x78], // 'm'
    [0x7C, 0x08, 0x04, 0x04, 0x78], // 'n'
    [0x38, 0x44, 0x44, 0x44, 0x38], // 'o'
    [0x7C, 0x14, 0x14, 0x14, 0x08], // 'p'
    [0x08, 0x14, 0x14, 0x18, 0x7C], // 'q'
    [0x7C, 0x08, 0x04, 0x04, 0x08], // 'r'
    [0x48, 0x54, 0x54, 0x54, 0x20], // 's'
    [0x04, 0x3F, 0x44, 0x40, 0x20], // 't'
    [0x3C, 0x40, 0x40, 0x20, 0x7C], // 'u'
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // 'v'
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // 'w'
    [0x44, 0x28, 0x10, 0x28, 0x44], // 'x'
    [0x0C, 0x50, 0x50, 0x50, 0x3C], // 'y'
    [0x44, 0x64, 0x54, 0x4C, 0x44], // 'z'
    [0x00, 0x08, 0x36, 0x41, 0x00], // '{'
    [0x00, 0x00, 0x7F, 0x00, 0x00], // '|'
    [0x00, 0x41, 0x36, 0x08, 0x00], // '}'
    [0x08, 0x04, 0x08, 0x10, 0x08], // '~'
];
