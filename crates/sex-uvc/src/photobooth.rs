// SPDX-License-Identifier: GPL-3.0-only
//! Photo Booth Filters & Compositor for Flash.
//!
//! Pure `#![no_std]`, `#![forbid(unsafe_code)]`.
//! Provides 9 live filters, 3x3 Live Effects Grid compositor,
//! and 2x2 Photo Booth Contact Sheet composition for burst shots.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// The 9 classic Photo Booth live filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Normal,
    Sepia,
    Noir,
    Thermal,
    XRay,
    PopArt,
    Mirror,
    Bulge,
    Comic,
}

impl Filter {
    pub const ALL: [Filter; 9] = [
        Filter::Normal,
        Filter::Sepia,
        Filter::Noir,
        Filter::Thermal,
        Filter::XRay,
        Filter::PopArt,
        Filter::Mirror,
        Filter::Bulge,
        Filter::Comic,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Filter::Normal => "Normal",
            Filter::Sepia => "Sepia",
            Filter::Noir => "Noir",
            Filter::Thermal => "Thermal",
            Filter::XRay => "XRay",
            Filter::PopArt => "PopArt",
            Filter::Mirror => "Mirror",
            Filter::Bulge => "Bulge",
            Filter::Comic => "Comic",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let b = name.as_bytes();
        if eq_ignore_ascii(b, b"normal") {
            Some(Filter::Normal)
        } else if eq_ignore_ascii(b, b"sepia") {
            Some(Filter::Sepia)
        } else if eq_ignore_ascii(b, b"noir") {
            Some(Filter::Noir)
        } else if eq_ignore_ascii(b, b"thermal") {
            Some(Filter::Thermal)
        } else if eq_ignore_ascii(b, b"xray") || eq_ignore_ascii(b, b"x-ray") || eq_ignore_ascii(b, b"x_ray") {
            Some(Filter::XRay)
        } else if eq_ignore_ascii(b, b"popart") || eq_ignore_ascii(b, b"pop_art") || eq_ignore_ascii(b, b"pop-art") || eq_ignore_ascii(b, b"pop") {
            Some(Filter::PopArt)
        } else if eq_ignore_ascii(b, b"mirror") {
            Some(Filter::Mirror)
        } else if eq_ignore_ascii(b, b"bulge") || eq_ignore_ascii(b, b"fisheye") {
            Some(Filter::Bulge)
        } else if eq_ignore_ascii(b, b"comic") {
            Some(Filter::Comic)
        } else {
            None
        }
    }

    pub fn next(self) -> Self {
        let idx = match self {
            Filter::Normal => 0,
            Filter::Sepia => 1,
            Filter::Noir => 2,
            Filter::Thermal => 3,
            Filter::XRay => 4,
            Filter::PopArt => 5,
            Filter::Mirror => 6,
            Filter::Bulge => 7,
            Filter::Comic => 8,
        };
        Filter::ALL[(idx + 1) % Filter::ALL.len()]
    }
}

fn eq_ignore_ascii(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        if a[i].to_ascii_lowercase() != b[i].to_ascii_lowercase() {
            return false;
        }
    }
    true
}

/// Precomputed 256-entry false-color thermal heatmap LUT:
/// Blue (cold) -> Purple -> Red -> Yellow -> White (hot).
const fn make_thermal_lut() -> [u32; 256] {
    let mut lut = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let (r, g, b) = if i < 64 {
            let t = i;
            let r = (128 * t) / 63;
            let g = 0;
            let b = 128 + (72 * t) / 63;
            (r, g, b)
        } else if i < 128 {
            let t = i - 64;
            let r = 128 + (127 * t) / 63;
            let g = (64 * t) / 63;
            let b = 200 - (200 * t) / 63;
            (r, g, b)
        } else if i < 192 {
            let t = i - 128;
            let r = 255;
            let g = 64 + (166 * t) / 63;
            let b = 0;
            (r, g, b)
        } else {
            let t = i - 192;
            let r = 255;
            let g = 230 + (25 * t) / 63;
            let b = (255 * t) / 63;
            (r, g, b)
        };
        lut[i] = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        i += 1;
    }
    lut
}

pub static THERMAL_LUT: [u32; 256] = make_thermal_lut();

/// Integer square root (deterministic, no floating point).
pub fn isqrt(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x0 = n / 2;
    if x0 == 0 {
        x0 = 1;
    }
    let mut x1 = (x0 + n / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + n / x0) / 2;
    }
    x0
}

/// Applies one of the 9 Photo Booth filters to `src` writing into `dst`.
/// Both slices must have at least `width * height` elements.
pub fn apply_filter(filter: Filter, src: &[u32], dst: &mut [u32], width: usize, height: usize) {
    let total = width.saturating_mul(height);
    if total == 0 || src.len() < total || dst.len() < total {
        return;
    }

    match filter {
        Filter::Normal => {
            dst[..total].copy_from_slice(&src[..total]);
        }
        Filter::Sepia => {
            for (s, d) in src[..total].iter().zip(dst[..total].iter_mut()) {
                let p = *s;
                let r = (p >> 16) & 0xFF;
                let g = (p >> 8) & 0xFF;
                let b = p & 0xFF;
                let nr = ((402 * r + 787 * g + 194 * b) >> 10).min(255);
                let ng = ((357 * r + 702 * g + 172 * b) >> 10).min(255);
                let nb = ((279 * r + 547 * g + 134 * b) >> 10).min(255);
                *d = (nr << 16) | (ng << 8) | nb;
            }
        }
        Filter::Noir => {
            let cx = (width / 2) as i32;
            let cy = (height / 2) as i32;
            let max_dist_sq = (cx * cx + cy * cy).max(1);

            for y in 0..height {
                let dy = y as i32 - cy;
                let row = y * width;
                for x in 0..width {
                    let p = src[row + x];
                    let r = (p >> 16) & 0xFF;
                    let g = (p >> 8) & 0xFF;
                    let b = p & 0xFF;
                    let lum = (77 * r + 150 * g + 29 * b) >> 8;
                    // S-curve contrast
                    let contrast = if lum < 128 {
                        (lum * lum) / 128
                    } else {
                        255 - ((255 - lum) * (255 - lum)) / 128
                    };
                    // Vignette falloff
                    let dx = x as i32 - cx;
                    let dist_sq = dx * dx + dy * dy;
                    let vig = (256 - ((dist_sq * 96) / max_dist_sq).min(96)) as u32;
                    let val = ((contrast * vig) >> 8).min(255);
                    dst[row + x] = (val << 16) | (val << 8) | val;
                }
            }
        }
        Filter::Thermal => {
            for (s, d) in src[..total].iter().zip(dst[..total].iter_mut()) {
                let p = *s;
                let r = (p >> 16) & 0xFF;
                let g = (p >> 8) & 0xFF;
                let b = p & 0xFF;
                let lum = ((77 * r + 150 * g + 29 * b) >> 8).min(255) as usize;
                *d = THERMAL_LUT[lum];
            }
        }
        Filter::XRay => {
            for (s, d) in src[..total].iter().zip(dst[..total].iter_mut()) {
                let p = *s;
                let r = (p >> 16) & 0xFF;
                let g = (p >> 8) & 0xFF;
                let b = p & 0xFF;
                let lum = (77 * r + 150 * g + 29 * b) >> 8;
                let inv = 255 - lum;
                let nr = (inv * 180) >> 8;
                let ng = (inv * 240) >> 8;
                let nb = inv;
                *d = (nr << 16) | (ng << 8) | nb;
            }
        }
        Filter::PopArt => {
            let mid_x = width / 2;
            let mid_y = height / 2;

            for y in 0..height {
                let row = y * width;
                let top = y < mid_y;
                for x in 0..width {
                    let left = x < mid_x;
                    let p = src[row + x];
                    let r = (p >> 16) & 0xFF;
                    let g = (p >> 8) & 0xFF;
                    let b = p & 0xFF;
                    let lum = (77 * r + 150 * g + 29 * b) >> 8;

                    let color = match (top, left) {
                        // Quadrant 0: Pink / Cyan / Yellow
                        (true, true) => {
                            if lum < 85 {
                                0x00E02070
                            } else if lum < 170 {
                                0x0000C8D8
                            } else {
                                0x00FFF020
                            }
                        }
                        // Quadrant 1: Deep Purple / Magenta / Gold
                        (true, false) => {
                            if lum < 85 {
                                0x00600070
                            } else if lum < 170 {
                                0x00FF007F
                            } else {
                                0x00FFD700
                            }
                        }
                        // Quadrant 2: Dark Violet / Lime / Orange
                        (false, true) => {
                            if lum < 85 {
                                0x00300860
                            } else if lum < 170 {
                                0x0032CD32
                            } else {
                                0x00FF8C00
                            }
                        }
                        // Quadrant 3: Electric Blue / Crimson / Mint
                        (false, false) => {
                            if lum < 85 {
                                0x000020A0
                            } else if lum < 170 {
                                0x00FF2040
                            } else {
                                0x0040FFB0
                            }
                        }
                    };
                    dst[row + x] = color;
                }
            }
        }
        Filter::Mirror => {
            let mid_x = width / 2;
            for y in 0..height {
                let row = y * width;
                for x in 0..mid_x {
                    dst[row + x] = src[row + x];
                }
                for x in mid_x..width {
                    let src_x = width.saturating_sub(1).saturating_sub(x);
                    dst[row + x] = src[row + src_x];
                }
            }
        }
        Filter::Bulge => {
            let cx = (width / 2) as i32;
            let cy = (height / 2) as i32;
            let max_r = cx.min(cy).max(1);
            let max_r_sq = max_r * max_r;

            for y in 0..height {
                let dy = y as i32 - cy;
                let row = y * width;
                for x in 0..width {
                    let dx = x as i32 - cx;
                    let r_sq = dx * dx + dy * dy;
                    if r_sq < max_r_sq && r_sq > 0 {
                        let r = isqrt(r_sq as u32) as i32;
                        let scale = (r * r) / max_r;
                        let sx = cx + (dx * scale) / r;
                        let sy = cy + (dy * scale) / r;
                        let sx = sx.clamp(0, width as i32 - 1) as usize;
                        let sy = sy.clamp(0, height as i32 - 1) as usize;
                        dst[row + x] = src[sy * width + sx];
                    } else {
                        dst[row + x] = src[row + x];
                    }
                }
            }
        }
        Filter::Comic => {
            // Step 1: Precompute luminance row-by-row and Sobel edges
            let w = width;
            let h = height;

            for y in 0..h {
                let row = y * w;
                let is_edge_y = y == 0 || y == h - 1;
                for x in 0..w {
                    let p = src[row + x];
                    let r = (p >> 16) & 0xFF;
                    let g = (p >> 8) & 0xFF;
                    let b = p & 0xFF;

                    let pr = ((r & 0xC0) | 0x20).min(255);
                    let pg = ((g & 0xC0) | 0x20).min(255);
                    let pb = ((b & 0xC0) | 0x20).min(255);

                    if is_edge_y || x == 0 || x == w - 1 {
                        dst[row + x] = (pr << 16) | (pg << 8) | pb;
                        continue;
                    }

                    let lum = |ox: usize, oy: usize| -> i32 {
                        let pix = src[oy * w + ox];
                        let pr = ((pix >> 16) & 0xFF) as i32;
                        let pg = ((pix >> 8) & 0xFF) as i32;
                        let pb = (pix & 0xFF) as i32;
                        (77 * pr + 150 * pg + 29 * pb) >> 8
                    };

                    let p00 = lum(x - 1, y - 1);
                    let p01 = lum(x, y - 1);
                    let p02 = lum(x + 1, y - 1);
                    let p10 = lum(x - 1, y);
                    let p12 = lum(x + 1, y);
                    let p20 = lum(x - 1, y + 1);
                    let p21 = lum(x, y + 1);
                    let p22 = lum(x + 1, y + 1);

                    let gx = -p00 + p02 - 2 * p10 + 2 * p12 - p20 + p22;
                    let gy = -p00 - 2 * p01 - p02 + p20 + 2 * p21 + p22;
                    let mag = gx.abs() + gy.abs();

                    if mag > 110 {
                        dst[row + x] = 0x00050810; // Ink outline
                    } else {
                        dst[row + x] = (pr << 16) | (pg << 8) | pb;
                    }
                }
            }
        }
    }
}

/// Compact 8x8 font table for ASCII characters 32..=126.
#[rustfmt::skip]
static FONT_8X8: [[u8; 8]; 95] = [
    // 32 ' '
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00],
    // 33 '!'
    [0x18,0x3C,0x3C,0x18,0x18,0x00,0x18,0x00],
    // 34 '"'
    [0x66,0x66,0x24,0x00,0x00,0x00,0x00,0x00],
    // 35 '#'
    [0x6C,0xFE,0x6C,0x6C,0xFE,0x6C,0x00,0x00],
    // 36 '$'
    [0x18,0x7C,0x60,0x38,0x06,0x7C,0x18,0x00],
    // 37 '%'
    [0x62,0x66,0x0C,0x18,0x30,0x66,0x46,0x00],
    // 38 '&'
    [0x38,0x6C,0x38,0x76,0xDC,0xCC,0x76,0x00],
    // 39 '\''
    [0x18,0x18,0x30,0x00,0x00,0x00,0x00,0x00],
    // 40 '('
    [0x0C,0x18,0x30,0x30,0x30,0x18,0x0C,0x00],
    // 41 ')'
    [0x30,0x18,0x0C,0x0C,0x0C,0x18,0x30,0x00],
    // 42 '*'
    [0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00],
    // 43 '+'
    [0x00,0x18,0x18,0x7E,0x18,0x18,0x00,0x00],
    // 44 ','
    [0x00,0x00,0x00,0x00,0x18,0x18,0x30,0x00],
    // 45 '-'
    [0x00,0x00,0x00,0x7E,0x00,0x00,0x00,0x00],
    // 46 '.'
    [0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x00],
    // 47 '/'
    [0x06,0x0C,0x18,0x30,0x60,0xC0,0x80,0x00],
    // 48-57 '0'-'9'
    [0x3C,0x66,0x6E,0x76,0x66,0x66,0x3C,0x00], // 0
    [0x18,0x38,0x18,0x18,0x18,0x18,0x7E,0x00], // 1
    [0x3C,0x66,0x06,0x0C,0x18,0x30,0x7E,0x00], // 2
    [0x3C,0x66,0x06,0x1C,0x06,0x66,0x3C,0x00], // 3
    [0x0C,0x1C,0x3C,0x6C,0x7E,0x0C,0x0C,0x00], // 4
    [0x7E,0x60,0x7C,0x06,0x06,0x66,0x3C,0x00], // 5
    [0x1C,0x30,0x60,0x7C,0x66,0x66,0x3C,0x00], // 6
    [0x7E,0x06,0x0C,0x18,0x30,0x30,0x30,0x00], // 7
    [0x3C,0x66,0x66,0x3C,0x66,0x66,0x3C,0x00], // 8
    [0x3C,0x66,0x66,0x3E,0x06,0x0C,0x38,0x00], // 9
    // 58 ':'
    [0x00,0x18,0x18,0x00,0x18,0x18,0x00,0x00],
    // 59 ';'
    [0x00,0x18,0x18,0x00,0x18,0x18,0x30,0x00],
    // 60 '<'
    [0x0C,0x18,0x30,0x60,0x30,0x18,0x0C,0x00],
    // 61 '='
    [0x00,0x00,0x7E,0x00,0x7E,0x00,0x00,0x00],
    // 62 '>'
    [0x60,0x30,0x18,0x0C,0x18,0x30,0x60,0x00],
    // 63 '?'
    [0x3C,0x66,0x06,0x0C,0x18,0x00,0x18,0x00],
    // 64 '@'
    [0x3C,0x66,0x6E,0x6A,0x6E,0x60,0x3C,0x00],
    // 65-90 'A'-'Z'
    [0x18,0x3C,0x66,0x7E,0x66,0x66,0x66,0x00], // A
    [0x7C,0x66,0x66,0x7C,0x66,0x66,0x7C,0x00], // B
    [0x3C,0x66,0x60,0x60,0x60,0x66,0x3C,0x00], // C
    [0x78,0x6C,0x66,0x66,0x66,0x6C,0x78,0x00], // D
    [0x7E,0x60,0x60,0x7C,0x60,0x60,0x7E,0x00], // E
    [0x7E,0x60,0x60,0x7C,0x60,0x60,0x60,0x00], // F
    [0x3C,0x66,0x60,0x6E,0x66,0x66,0x3C,0x00], // G
    [0x66,0x66,0x66,0x7E,0x66,0x66,0x66,0x00], // H
    [0x3C,0x18,0x18,0x18,0x18,0x18,0x3C,0x00], // I
    [0x1E,0x0C,0x0C,0x0C,0x6C,0x6C,0x38,0x00], // J
    [0x66,0x6C,0x78,0x70,0x78,0x6C,0x66,0x00], // K
    [0x60,0x60,0x60,0x60,0x60,0x60,0x7E,0x00], // L
    [0xC6,0xEE,0xFE,0xD6,0xC6,0xC6,0xC6,0x00], // M
    [0x66,0x76,0x7E,0x6E,0x66,0x66,0x66,0x00], // N
    [0x3C,0x66,0x66,0x66,0x66,0x66,0x3C,0x00], // O
    [0x7C,0x66,0x66,0x7C,0x60,0x60,0x60,0x00], // P
    [0x3C,0x66,0x66,0x66,0x6A,0x6C,0x36,0x00], // Q
    [0x7C,0x66,0x66,0x7C,0x78,0x6C,0x66,0x00], // R
    [0x3C,0x66,0x60,0x3C,0x06,0x66,0x3C,0x00], // S
    [0x7E,0x18,0x18,0x18,0x18,0x18,0x18,0x00], // T
    [0x66,0x66,0x66,0x66,0x66,0x66,0x3C,0x00], // U
    [0x66,0x66,0x66,0x66,0x66,0x3C,0x18,0x00], // V
    [0xC6,0xC6,0xC6,0xD6,0xFE,0xEE,0xC6,0x00], // W
    [0x66,0x66,0x3C,0x18,0x3C,0x66,0x66,0x00], // X
    [0x66,0x66,0x66,0x3C,0x18,0x18,0x18,0x00], // Y
    [0x7E,0x06,0x0C,0x18,0x30,0x60,0x7E,0x00], // Z
    // 91 '['
    [0x3C,0x30,0x30,0x30,0x30,0x30,0x3C,0x00],
    // 92 '\\'
    [0x80,0xC0,0x60,0x30,0x18,0x0C,0x06,0x00],
    // 93 ']'
    [0x3C,0x0C,0x0C,0x0C,0x0C,0x0C,0x3C,0x00],
    // 94 '^'
    [0x18,0x3C,0x66,0x00,0x00,0x00,0x00,0x00],
    // 95 '_'
    [0x00,0x00,0x00,0x00,0x00,0x00,0xFF,0x00],
    // 96 '`'
    [0x30,0x18,0x0C,0x00,0x00,0x00,0x00,0x00],
    // 97-122 'a'-'z'
    [0x00,0x00,0x3C,0x06,0x3E,0x66,0x3E,0x00], // a
    [0x60,0x60,0x7C,0x66,0x66,0x66,0x7C,0x00], // b
    [0x00,0x00,0x3C,0x66,0x60,0x66,0x3C,0x00], // c
    [0x06,0x06,0x3E,0x66,0x66,0x66,0x3E,0x00], // d
    [0x00,0x00,0x3C,0x66,0x7E,0x60,0x3C,0x00], // e
    [0x1C,0x30,0x30,0x7C,0x30,0x30,0x30,0x00], // f
    [0x00,0x00,0x3E,0x66,0x66,0x3E,0x06,0x3C], // g
    [0x60,0x60,0x7C,0x66,0x66,0x66,0x66,0x00], // h
    [0x18,0x00,0x38,0x18,0x18,0x18,0x3C,0x00], // i
    [0x06,0x00,0x06,0x06,0x06,0x66,0x66,0x3C], // j
    [0x60,0x60,0x66,0x6C,0x78,0x6C,0x66,0x00], // k
    [0x38,0x18,0x18,0x18,0x18,0x18,0x3C,0x00], // l
    [0x00,0x00,0xCC,0xFE,0xD6,0xC6,0xC6,0x00], // m
    [0x00,0x00,0x7C,0x66,0x66,0x66,0x66,0x00], // n
    [0x00,0x00,0x3C,0x66,0x66,0x66,0x3C,0x00], // o
    [0x00,0x00,0x7C,0x66,0x66,0x7C,0x60,0x60], // p
    [0x00,0x00,0x3E,0x66,0x66,0x3E,0x06,0x06], // q
    [0x00,0x00,0x6C,0x76,0x60,0x60,0x60,0x00], // r
    [0x00,0x00,0x3C,0x60,0x3C,0x06,0x7C,0x00], // s
    [0x30,0x30,0x7E,0x30,0x30,0x36,0x1C,0x00], // t
    [0x00,0x00,0x66,0x66,0x66,0x66,0x3E,0x00], // u
    [0x00,0x00,0x66,0x66,0x66,0x3C,0x18,0x00], // v
    [0x00,0x00,0xC6,0xC6,0xD6,0xFE,0x6C,0x00], // w
    [0x00,0x00,0x66,0x3C,0x18,0x3C,0x66,0x00], // x
    [0x00,0x00,0x66,0x66,0x66,0x3E,0x06,0x3C], // y
    [0x00,0x00,0x7E,0x0C,0x18,0x30,0x7E,0x00], // z
    // 123-126 '{', '|', '}', '~'
    [0x0C,0x18,0x18,0x70,0x18,0x18,0x0C,0x00],
    [0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x00],
    [0x30,0x18,0x18,0x0E,0x18,0x18,0x30,0x00],
    [0x76,0xDC,0x00,0x00,0x00,0x00,0x00,0x00],
];

/// Draws text onto an ARGB pixel buffer with bounds checking.
pub fn draw_text_8x8(dst: &mut [u32], dst_w: usize, dst_h: usize, x: usize, y: usize, text: &[u8], fg: u32, bg: Option<u32>) {
    let mut cx = x;
    for &b in text {
        if b < 32 || b > 126 {
            cx += 8;
            continue;
        }
        let glyph = &FONT_8X8[(b - 32) as usize];
        for row in 0..8usize {
            let py = y + row;
            if py >= dst_h {
                break;
            }
            let byte = glyph[row];
            for col in 0..8usize {
                let px = cx + col;
                if px >= dst_w {
                    break;
                }
                let bit = (byte >> (7 - col)) & 1;
                let offset = py * dst_w + px;
                if bit != 0 {
                    dst[offset] = fg;
                } else if let Some(b_col) = bg {
                    dst[offset] = b_col;
                }
            }
        }
        cx += 8;
    }
}

/// Renders all 9 live filters in a 3x3 Photo Booth preview matrix with labels and borders.
pub fn render_effects_grid(src: &[u32], src_w: usize, src_h: usize, dst: &mut [u32], dst_w: usize, dst_h: usize) {
    if dst_w < 30 || dst_h < 30 || src_w == 0 || src_h == 0 {
        return;
    }
    let cell_w = dst_w / 3;
    let cell_h = dst_h / 3;

    let mut cell_buf = vec![0u32; cell_w * cell_h];
    let mut filtered_cell = vec![0u32; cell_w * cell_h];

    for row_idx in 0..3 {
        for col_idx in 0..3 {
            let filter_idx = row_idx * 3 + col_idx;
            let filter = Filter::ALL[filter_idx];

            let x0 = col_idx * cell_w;
            let y0 = row_idx * cell_h;
            let x1 = if col_idx == 2 { dst_w } else { (col_idx + 1) * cell_w };
            let y1 = if row_idx == 2 { dst_h } else { (row_idx + 1) * cell_h };
            let cur_w = x1 - x0;
            let cur_h = y1 - y0;

            if cur_w == 0 || cur_h == 0 {
                continue;
            }

            if cell_buf.len() < cur_w * cur_h {
                cell_buf.resize(cur_w * cur_h, 0);
                filtered_cell.resize(cur_w * cur_h, 0);
            }

            // Downsample src into cell_buf
            for cy in 0..cur_h {
                let sy = (cy * src_h) / cur_h;
                let sy_off = sy * src_w;
                let cy_off = cy * cur_w;
                for cx in 0..cur_w {
                    let sx = (cx * src_w) / cur_w;
                    cell_buf[cy_off + cx] = src[sy_off + sx];
                }
            }

            // Apply filter to downsampled cell
            apply_filter(filter, &cell_buf[..cur_w * cur_h], &mut filtered_cell[..cur_w * cur_h], cur_w, cur_h);

            // Copy filtered cell to destination
            for cy in 0..cur_h {
                let dst_off = (y0 + cy) * dst_w + x0;
                let cell_off = cy * cur_w;
                dst[dst_off..dst_off + cur_w].copy_from_slice(&filtered_cell[cell_off..cell_off + cur_w]);
            }

            // Draw tile border (2px dark line)
            let border_col = 0x0010141C;
            for cy in 0..cur_h {
                dst[(y0 + cy) * dst_w + x0] = border_col;
                if x0 + 1 < dst_w {
                    dst[(y0 + cy) * dst_w + x0 + 1] = border_col;
                }
                dst[(y0 + cy) * dst_w + x1 - 1] = border_col;
            }
            for cx in 0..cur_w {
                dst[y0 * dst_w + x0 + cx] = border_col;
                if y0 + 1 < dst_h {
                    dst[(y0 + 1) * dst_w + x0 + cx] = border_col;
                }
                dst[(y1 - 1) * dst_w + x0 + cx] = border_col;
            }

            // Label pill badge at bottom center of tile
            let label: &[u8] = match filter {
                Filter::Normal => b"1 Normal",
                Filter::Sepia => b"2 Sepia",
                Filter::Noir => b"3 Noir",
                Filter::Thermal => b"4 Thermal",
                Filter::XRay => b"5 XRay",
                Filter::PopArt => b"6 PopArt",
                Filter::Mirror => b"7 Mirror",
                Filter::Bulge => b"8 Bulge",
                Filter::Comic => b"9 Comic",
            };
            let text_w = label.len() * 8;
            let badge_w = text_w + 12;
            let badge_h = 16usize;
            if cur_w > badge_w + 8 && cur_h > badge_h + 8 {
                let bx = x0 + (cur_w - badge_w) / 2;
                let by = y1 - badge_h - 6;
                // Translucent dark pill background
                for py in by..by + badge_h {
                    for px in bx..bx + badge_w {
                        if px < dst_w && py < dst_h {
                            let old = dst[py * dst_w + px];
                            let r = ((old >> 16) & 0xFF) / 4;
                            let g = ((old >> 8) & 0xFF) / 4;
                            let b = (old & 0xFF) / 4;
                            dst[py * dst_w + px] = (r << 16) | (g << 8) | b;
                        }
                    }
                }
                // Text
                draw_text_8x8(dst, dst_w, dst_h, bx + 6, by + 4, label, 0x00FFFFFF, None);
            }
        }
    }
}

/// Composes a classic 2x2 Photo Booth Contact Sheet from 4 burst frames.
/// Returns (width, height, ARGB pixels).
pub fn compose_burst_contact_sheet(frames: [&[u32]; 4], w: usize, h: usize) -> (usize, usize, Vec<u32>) {
    let cell_w = w / 2;
    let cell_h = h / 2;

    let pad_x = 24usize;
    let pad_top = 24usize;
    let gap = 16usize;
    let pad_bottom = 60usize;

    let out_w = pad_x * 2 + cell_w * 2 + gap;
    let out_h = pad_top + cell_h * 2 + gap + pad_bottom;

    // Background: classic photo paper white
    let paper_color = 0x00FBFBFC;
    let mut out = vec![paper_color; out_w * out_h];

    let border_color = 0x00D0D2D8;

    let positions = [
        (pad_x, pad_top),
        (pad_x + cell_w + gap, pad_top),
        (pad_x, pad_top + cell_h + gap),
        (pad_x + cell_w + gap, pad_top + cell_h + gap),
    ];

    for (i, &(px, py)) in positions.iter().enumerate() {
        let frame = frames[i];
        // Downsample frame into cell
        for cy in 0..cell_h {
            let sy = (cy * h) / cell_h;
            let sy_off = sy * w;
            let out_y = py + cy;
            let out_row = out_y * out_w;
            for cx in 0..cell_w {
                let sx = (cx * w) / cell_w;
                out[out_row + px + cx] = frame[sy_off + sx];
            }
        }

        // Draw 1px photo frame border around each shot
        for cx in 0..cell_w {
            if py > 0 {
                out[(py - 1) * out_w + px + cx] = border_color;
            }
            if py + cell_h < out_h {
                out[(py + cell_h) * out_w + px + cx] = border_color;
            }
        }
        for cy in 0..cell_h {
            if px > 0 {
                out[(py + cy) * out_w + px - 1] = border_color;
            }
            if px + cell_w < out_w {
                out[(py + cy) * out_w + px + cell_w] = border_color;
            }
        }
    }

    // Bottom strip title
    let title = b"FLASH PHOTO BOOTH";
    let title_x = out_w.saturating_sub(title.len() * 8) / 2;
    let title_y = out_h.saturating_sub(pad_bottom) + 16;
    draw_text_8x8(&mut out, out_w, out_h, title_x, title_y, title, 0x002B303A, None);

    let subtitle = b"SEXOS 4-SHOT BURST * PURE RUST UVC";
    let sub_x = out_w.saturating_sub(subtitle.len() * 8) / 2;
    let sub_y = title_y + 16;
    draw_text_8x8(&mut out, out_w, out_h, sub_x, sub_y, subtitle, 0x00788090, None);

    (out_w, out_h, out)
}
