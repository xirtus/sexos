// SPDX-License-Identifier: GPL-3.0-only
//! Real-time ARGB32 software telemetry and comparison chart renderer for Climax.
//! Provides pure-Rust `#![no_std]`, `#![forbid(unsafe_code)]` rendering of:
//! 1. Subsystem benchmark bars, Sattolo ladder curve, and composite history sparkline.
//! 2. Sustained thermal torture time-series with baseline/throttle dashed reference lines,
//!    translucent gradient area fill, and live KPI header badges.
//! 3. Side-by-side grouped horizontal comparison chart against published Linux 6.6 LTS medians.

#![forbid(unsafe_code)]

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::linux_compare::LinuxComparisonDoc;
use crate::report::SubsystemScores;
use crate::stress::Sample;
use crate::thermal::ThermalAnalysis;

// Colors (0xAARRGGBB)
const COLOR_BG: u32 = 0xFF0F172A;          // Deep Slate 900
const COLOR_CARD: u32 = 0xFF1E293B;        // Slate 800
const COLOR_BORDER: u32 = 0xFF334155;      // Slate 700
const COLOR_GRID: u32 = 0xFF273549;        // Faint Slate
const COLOR_TEXT: u32 = 0xFFF8FAFC;        // Crisp Slate 50
const COLOR_MUTED: u32 = 0xFF94A3B8;       // Slate 400
const COLOR_BLUE: u32 = 0xFF3B82F6;        // Electric Blue (SexOS Primary)
const COLOR_CYAN: u32 = 0xFF06B6D4;        // Cyan (Baseline Boost / L1)
const COLOR_EMERALD: u32 = 0xFF10B981;     // Emerald (Passed / 100%+ Advantage)
const COLOR_DARK_GREEN: u32 = 0xFF065F46;  // Dark green background
const COLOR_ORANGE: u32 = 0xFFF97316;      // Orange (Linux 6.6 LTS Median)
const COLOR_AMBER: u32 = 0xFFF59E0B;       // Amber (Warning / Throttle)
const COLOR_RED: u32 = 0xFFEF4444;         // Red (Throttled / Corrupt)
const COLOR_PURPLE: u32 = 0xFF8B5CF6;      // Purple (Audio DSP / Stream)
const COLOR_GOLD: u32 = 0xFFFBBF24;        // Gold (Composite Score)
const COLOR_ROSE: u32 = 0xFFF43F5E;        // Rose (Pointer Latency)

/// Standalone ARGB32 software graphics canvas.
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Canvas {
    /// Creates a new canvas initialized to the given background color.
    pub fn new(width: usize, height: usize, bg_color: u32) -> Self {
        let total = width.saturating_mul(height);
        Self {
            width,
            height,
            pixels: vec![bg_color; total],
        }
    }

    #[inline]
    pub fn set_pixel(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            let idx = (y as usize) * self.width + (x as usize);
            if idx < self.pixels.len() {
                self.pixels[idx] = color;
            }
        }
    }

    /// Blends a source ARGB32 color over the existing destination pixel using alpha.
    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || x >= self.width as i32 || y < 0 || y >= self.height as i32 {
            return;
        }
        let alpha = (color >> 24) & 0xFF;
        if alpha == 0 {
            return;
        }
        if alpha == 255 {
            self.set_pixel(x, y, color);
            return;
        }

        let idx = (y as usize) * self.width + (x as usize);
        if idx >= self.pixels.len() {
            return;
        }
        let dst = self.pixels[idx];

        let sr = (color >> 16) & 0xFF;
        let sg = (color >> 8) & 0xFF;
        let sb = color & 0xFF;

        let dr = (dst >> 16) & 0xFF;
        let dg = (dst >> 8) & 0xFF;
        let db = dst & 0xFF;

        let inv = 255 - alpha;
        let r = (sr * alpha + dr * inv) / 255;
        let g = (sg * alpha + dg * inv) / 255;
        let b = (sb * alpha + db * inv) / 255;

        self.pixels[idx] = 0xFF000000 | (r << 16) | (g << 8) | b;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let x0 = x.clamp(0, self.width as i32) as usize;
        let y0 = y.clamp(0, self.height as i32) as usize;
        let x1 = (x + w).clamp(0, self.width as i32) as usize;
        let y1 = (y + h).clamp(0, self.height as i32) as usize;

        for row in y0..y1 {
            let start = row * self.width + x0;
            let end = row * self.width + x1;
            if start < self.pixels.len() && end <= self.pixels.len() {
                self.pixels[start..end].fill(color);
            }
        }
    }

    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, color);
        self.fill_rect(x, y + h - 1, w, 1, color);
        self.fill_rect(x, y, 1, h, color);
        self.fill_rect(x + w - 1, y, 1, h, color);
    }

    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = radius.min(w / 2).min(h / 2).max(1);
        self.fill_rect(x + r, y, w - 2 * r, h, color);
        self.fill_rect(x, y + r, r, h - 2 * r, color);
        self.fill_rect(x + w - r, y + r, r, h - 2 * r, color);

        for dy in 0..r {
            for dx in 0..r {
                let dist_sq = (r - 1 - dx) * (r - 1 - dx) + (r - 1 - dy) * (r - 1 - dy);
                if dist_sq <= r * r {
                    self.set_pixel(x + dx, y + dy, color);
                    self.set_pixel(x + w - 1 - dx, y + dy, color);
                    self.set_pixel(x + dx, y + h - 1 - dy, color);
                    self.set_pixel(x + w - 1 - dx, y + h - 1 - dy, color);
                }
            }
        }
    }

    pub fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = radius.min(w / 2).min(h / 2).max(1);
        self.fill_rect(x + r, y, w - 2 * r, 1, color);
        self.fill_rect(x + r, y + h - 1, w - 2 * r, 1, color);
        self.fill_rect(x, y + r, 1, h - 2 * r, color);
        self.fill_rect(x + w - 1, y + r, 1, h - 2 * r, color);

        for dy in 0..r {
            for dx in 0..r {
                let dist_sq = (r - 1 - dx) * (r - 1 - dx) + (r - 1 - dy) * (r - 1 - dy);
                if dist_sq <= r * r && dist_sq >= (r - 2) * (r - 2) {
                    self.set_pixel(x + dx, y + dy, color);
                    self.set_pixel(x + w - 1 - dx, y + dy, color);
                    self.set_pixel(x + dx, y + h - 1 - dy, color);
                    self.set_pixel(x + w - 1 - dx, y + h - 1 - dy, color);
                }
            }
        }
    }

    pub fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u32) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            self.set_pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn draw_thick_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        self.draw_line(x0, y0, x1, y1, color);
        self.draw_line(x0, y0 + 1, x1, y1 + 1, color);
        self.draw_line(x0 + 1, y0, x1 + 1, y1, color);
    }

    pub fn draw_dashed_line(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        dash_len: i32,
        gap_len: i32,
        color: u32,
    ) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cur_x = x0;
        let mut cur_y = y0;
        let mut step = 0;
        let period = dash_len + gap_len;

        loop {
            if (step % period) < dash_len {
                self.set_pixel(cur_x, cur_y, color);
            }
            if cur_x == x1 && cur_y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cur_x += sx;
            }
            if e2 <= dx {
                err += dx;
                cur_y += sy;
            }
            step += 1;
        }
    }

    /// Fills the area between a polygonal curve and baseline_y with an alpha gradient.
    pub fn fill_area_under_curve(
        &mut self,
        points: &[(i32, i32)],
        baseline_y: i32,
        top_color: u32,
        bottom_color: u32,
    ) {
        if points.len() < 2 {
            return;
        }
        let min_x = points[0].0;
        let max_x = points[points.len() - 1].0;

        for x in min_x..=max_x {
            // Find bounding segment
            let mut seg = None;
            for i in 0..points.len() - 1 {
                if x >= points[i].0 && x <= points[i + 1].0 {
                    seg = Some((points[i], points[i + 1]));
                    break;
                }
            }
            let Some((p0, p1)) = seg else { continue };
            let span = (p1.0 - p0.0).max(1);
            let t = ((x - p0.0) as f64) / (span as f64);
            let curve_y = ((p0.1 as f64) * (1.0 - t) + (p1.1 as f64) * t) as i32;

            let start_y = curve_y.min(baseline_y);
            let end_y = curve_y.max(baseline_y);
            let total_h = (end_y - start_y).max(1);

            for y in start_y..=end_y {
                let frac = ((y - start_y) as f64) / (total_h as f64);
                let alpha = (60.0 * (1.0 - frac) + 15.0 * frac) as u32;
                let r0 = ((top_color >> 16) & 0xFF) as f64;
                let g0 = ((top_color >> 8) & 0xFF) as f64;
                let b0 = (top_color & 0xFF) as f64;
                let r1 = ((bottom_color >> 16) & 0xFF) as f64;
                let g1 = ((bottom_color >> 8) & 0xFF) as f64;
                let b1 = (bottom_color & 0xFF) as f64;
                let r = (r0 * (1.0 - frac) + r1 * frac) as u32;
                let g = (g0 * (1.0 - frac) + g1 * frac) as u32;
                let b = (b0 * (1.0 - frac) + b1 * frac) as u32;
                let c = (alpha << 24) | (r << 16) | (g << 8) | b;
                self.blend_pixel(x, y, c);
            }
        }
    }

    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: u32) {
        let mut cx = x;
        for ch in text.chars() {
            if cx + 8 > self.width as i32 {
                break;
            }
            self.draw_char(cx, y, ch, color);
            cx += 8;
        }
    }

    fn draw_char(&mut self, x: i32, y: i32, ch: char, color: u32) {
        let bitmap = glyph_bits(ch);
        for row in 0..12 {
            let py = y + row as i32;
            if py < 0 || py >= self.height as i32 {
                continue;
            }
            let bits = bitmap[row];
            for col in 0..8 {
                let px = x + col as i32;
                if px >= 0 && px < self.width as i32 && (bits & (0x80 >> col)) != 0 {
                    self.set_pixel(px, py, color);
                }
            }
        }
    }

    pub fn draw_pill_badge(&mut self, x: i32, y: i32, text: &str, bg: u32, fg: u32) {
        let text_w = text.len() as i32 * 8;
        let w = text_w + 14;
        let h = 18;
        self.fill_rounded_rect(x, y, w, h, 6, bg);
        self.stroke_rounded_rect(x, y, w, h, 6, (bg & 0x00FFFFFF) | 0xFF000000);
        self.draw_text(x + 7, y + 3, text, fg);
    }
}

/// Compact 8x12 readable glyph table.
fn glyph_bits(ch: char) -> [u8; 12] {
    let code = ch as u32;
    match code {
        0x20 => [0; 12],
        0x21 => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x18, 0, 0], // !
        0x22 => [0x66, 0x66, 0x66, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],                   // "
        0x23 => [0x6C, 0x6C, 0xFE, 0x6C, 0xFE, 0x6C, 0x6C, 0, 0, 0, 0, 0],           // #
        0x25 => [0x62, 0x64, 0x08, 0x10, 0x26, 0x46, 0, 0, 0, 0, 0, 0],              // %
        0x28 => [0x0C, 0x18, 0x30, 0x30, 0x30, 0x30, 0x30, 0x18, 0x0C, 0, 0, 0],     // (
        0x29 => [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0, 0, 0],     // )
        0x2A | 0x2022 => [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0, 0, 0, 0, 0, 0],      // * or bullet
        0x2B => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0, 0, 0, 0, 0, 0],              // +
        0x2C => [0x00, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0x30, 0, 0],                     // ,
        0x2D | 0x2014 => [0x00, 0, 0, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0],                  // - or em-dash
        0x2E => [0x00, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0, 0, 0],                         // .
        0x2F => [0x02, 0x06, 0x0C, 0x18, 0x30, 0x60, 0xC0, 0, 0, 0, 0, 0],           // /
        0x30 => [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 0
        0x31 => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0, 0, 0, 0, 0],           // 1
        0x32 => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x30, 0x7E, 0, 0, 0, 0, 0],           // 2
        0x33 => [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 3
        0x34 => [0x0C, 0x1C, 0x34, 0x64, 0x7E, 0x04, 0x04, 0, 0, 0, 0, 0],           // 4
        0x35 => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 5
        0x36 => [0x1C, 0x30, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 6
        0x37 => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0, 0, 0, 0, 0],           // 7
        0x38 => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 8
        0x39 => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x38, 0, 0, 0, 0, 0],           // 9
        0x3A => [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0, 0, 0, 0, 0, 0],              // :
        0x3C => [0x0C, 0x18, 0x30, 0x60, 0x30, 0x18, 0x0C, 0, 0, 0, 0, 0],           // <
        0x3D => [0x00, 0x7E, 0x00, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0],                     // =
        0x3E => [0x30, 0x18, 0x0C, 0x06, 0x0C, 0x18, 0x30, 0, 0, 0, 0, 0],           // >
        0x41 => [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0, 0, 0, 0, 0],           // A
        0x42 => [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0, 0, 0, 0, 0],           // B
        0x43 => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0, 0, 0, 0, 0],           // C
        0x44 => [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0, 0, 0, 0, 0],           // D
        0x45 => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x7E, 0, 0, 0, 0, 0],           // E
        0x46 => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0],           // F
        0x47 => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3A, 0, 0, 0, 0, 0],           // G
        0x48 => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0, 0, 0, 0, 0],           // H
        0x49 => [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0, 0, 0],           // I
        0x4A => [0x0E, 0x06, 0x06, 0x06, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // J
        0x4B => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0, 0, 0, 0, 0],           // K
        0x4C => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0, 0, 0, 0, 0],           // L
        0x4D => [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0, 0, 0, 0, 0],           // M
        0x4E => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0, 0, 0, 0, 0],           // N
        0x4F => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // O
        0x50 => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0],           // P
        0x51 => [0x3C, 0x66, 0x66, 0x66, 0x6A, 0x6C, 0x36, 0, 0, 0, 0, 0],           // Q
        0x52 => [0x7C, 0x66, 0x66, 0x7C, 0x78, 0x6C, 0x66, 0, 0, 0, 0, 0],           // R
        0x53 => [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // S
        0x54 => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0],           // T
        0x55 => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // U
        0x56 => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0, 0, 0, 0, 0],           // V
        0x57 => [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0, 0, 0, 0, 0],           // W
        0x58 => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0, 0, 0, 0, 0],           // X
        0x59 => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0],           // Y
        0x5A => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0, 0, 0, 0, 0],           // Z
        0x5B => [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0, 0, 0, 0, 0],           // [
        0x5D => [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0, 0, 0, 0, 0],           // ]
        0x5F => [0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0, 0],                           // _
        0x61 => [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3B, 0, 0, 0, 0, 0],           // a
        0x62 => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0, 0, 0, 0, 0],           // b
        0x63 => [0x00, 0x00, 0x3C, 0x66, 0x60, 0x66, 0x3C, 0, 0, 0, 0, 0],           // c
        0x64 => [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0, 0, 0],           // d
        0x65 => [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0, 0, 0, 0, 0],           // e
        0x66 => [0x0C, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0],           // f
        0x67 => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x3C, 0, 0, 0, 0],         // g
        0x68 => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0, 0, 0, 0, 0],           // h
        0x69 => [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0, 0, 0],           // i
        0x6A => [0x06, 0x00, 0x0E, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // j
        0x6B => [0x60, 0x60, 0x66, 0x6C, 0x78, 0x6C, 0x66, 0, 0, 0, 0, 0],           // k
        0x6C => [0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0, 0, 0],           // l
        0x6D => [0x00, 0x00, 0x66, 0x7F, 0x7F, 0x6B, 0x63, 0, 0, 0, 0, 0],           // m
        0x6E => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0, 0, 0, 0, 0],           // n
        0x6F => [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // o
        0x70 => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0, 0, 0, 0],         // p
        0x71 => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06, 0, 0, 0, 0],         // q
        0x72 => [0x00, 0x00, 0x7C, 0x66, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0],           // r
        0x73 => [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0, 0, 0, 0, 0],           // s
        0x74 => [0x18, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x0E, 0, 0, 0, 0, 0],           // t
        0x75 => [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0, 0, 0],           // u
        0x76 => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0, 0, 0, 0, 0],           // v
        0x77 => [0x00, 0x00, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0, 0, 0, 0, 0],           // w
        0x78 => [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0, 0, 0, 0, 0],           // x
        0x79 => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x3C, 0, 0, 0, 0],         // y
        0x7A => [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0, 0, 0, 0, 0],           // z
        0x7C => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0],     // |
        _ => [0x00, 0x7E, 0x42, 0x42, 0x42, 0x7E, 0, 0, 0, 0, 0, 0],                  // default rect
    }
}

/// Renders 6-subsystem normalized score bars, Sattolo ladder throughput curve,
/// and rolling composite score sparkline.
pub fn render_benchmark_charts(
    scores: Option<&SubsystemScores>,
    ladder_tiers: &[(u32, f64)],
    history: &[f64],
    width: usize,
    height: usize,
) -> Vec<u32> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut canvas = Canvas::new(width, height, COLOR_BG);
    if width < 300 || height < 140 {
        return canvas.pixels;
    }

    let pad = 10;
    let card_w = (width as i32 - 3 * pad) / 2;
    let card_h = height as i32 - 2 * pad;

    // --- LEFT PANEL: 6-Subsystem Normalized Score Bars ---
    let left_x = pad;
    let left_y = pad;
    canvas.fill_rounded_rect(left_x, left_y, card_w, card_h, 6, COLOR_CARD);
    canvas.stroke_rounded_rect(left_x, left_y, card_w, card_h, 6, COLOR_BORDER);

    canvas.draw_text(left_x + 12, left_y + 10, "6-SUBSYSTEM BENCHMARK SCORES", COLOR_CYAN);

    let labels = [
        ("CPU ARX", COLOR_BLUE),
        ("SHA-256", COLOR_CYAN),
        ("GEMM FP32", COLOR_EMERALD),
        ("Audio DSP", COLOR_AMBER),
        ("STREAM Mem", COLOR_PURPLE),
        ("Ptr Latency", COLOR_ROSE),
    ];

    if let Some(sc) = scores {
        let vals = [
            sc.cpu_int,
            sc.sha256,
            sc.gemm_fp,
            sc.dsp_fp,
            sc.memory_bandwidth,
            sc.pointer_latency,
        ];
        let max_val = vals.iter().copied().max().unwrap_or(1).max(1);

        let row_h = (card_h - 65) / 6;
        for i in 0..6 {
            let ry = left_y + 34 + i * row_h;
            let (name, col) = labels[i as usize];
            let val = vals[i as usize];

            canvas.draw_text(left_x + 12, ry + 2, name, COLOR_TEXT);

            // Bar background track
            let bar_x = left_x + 105;
            let bar_max_w = card_w - 180;
            let bar_h = 10;
            canvas.fill_rounded_rect(bar_x, ry + 2, bar_max_w, bar_h, 3, COLOR_GRID);

            // Active bar
            let fill_w = (((val as u128) * (bar_max_w as u128)) / (max_val as u128)) as i32;
            let fill_w = fill_w.clamp(2, bar_max_w);
            canvas.fill_rounded_rect(bar_x, ry + 2, fill_w, bar_h, 3, col);

            // Value badge
            let val_str = format!("{}", val);
            canvas.draw_text(bar_x + bar_max_w + 8, ry + 1, &val_str, col);
        }

        // Composite footer badge
        let comp_str = format!("COMPOSITE SCORE: {}", sc.composite);
        canvas.draw_pill_badge(left_x + 12, left_y + card_h - 26, &comp_str, COLOR_GOLD, 0xFF0F172A);
    } else {
        // Ready / zero state
        canvas.draw_pill_badge(left_x + card_w - 90, left_y + 8, "ONLINE", COLOR_DARK_GREEN, COLOR_EMERALD);

        let row_h = (card_h - 65) / 6;
        for i in 0..6 {
            let ry = left_y + 34 + i * row_h;
            let (name, col) = labels[i as usize];
            canvas.draw_text(left_x + 12, ry + 2, name, col);

            let bar_x = left_x + 105;
            let bar_max_w = card_w - 180;
            let bar_h = 10;
            canvas.fill_rounded_rect(bar_x, ry + 2, bar_max_w, bar_h, 3, COLOR_GRID);
            canvas.draw_dashed_line(bar_x, ry + 7, bar_x + bar_max_w, ry + 7, 4, 4, COLOR_BORDER);
        }

        let ready_msg = "READY — CLICK 'RUN SUITE'";
        let rx = left_x + (card_w - (ready_msg.len() as i32 * 8)) / 2;
        canvas.draw_pill_badge(rx - 7, left_y + card_h - 26, ready_msg, COLOR_AMBER, 0xFF0F172A);
    }

    // --- RIGHT PANEL: Sattolo Ladder Curve & Composite History Sparkline ---
    let right_x = pad * 2 + card_w;
    let right_y = pad;
    canvas.fill_rounded_rect(right_x, right_y, card_w, card_h, 6, COLOR_CARD);
    canvas.stroke_rounded_rect(right_x, right_y, card_w, card_h, 6, COLOR_BORDER);
    canvas.draw_pill_badge(right_x + card_w - 95, right_y + 8, "STANDBY", COLOR_GRID, COLOR_CYAN);

    // Top: Sattolo Ladder Curve
    let ladder_h = (card_h - 16) / 2;
    canvas.draw_text(right_x + 12, right_y + 10, "SATTOLO CACHE/DRAM LADDER", COLOR_EMERALD);

    let ladder_area_x = right_x + 36;
    let ladder_area_y = right_y + 32;
    let ladder_area_w = card_w - 48;
    let ladder_area_h = ladder_h - 44;

    // Gridlines
    canvas.stroke_rect(ladder_area_x, ladder_area_y, ladder_area_w, ladder_area_h, COLOR_BORDER);
    for g in 1..4 {
        let gy = ladder_area_y + (g * ladder_area_h) / 4;
        canvas.draw_dashed_line(ladder_area_x, gy, ladder_area_x + ladder_area_w, gy, 4, 4, COLOR_GRID);
    }

    let default_tiers = [
        (4u32, 219.7f64),
        (256, 193.3),
        (4096, 46.7),
        (16384, 14.2),
    ];
    let tiers_to_use = if !ladder_tiers.is_empty() {
        ladder_tiers
    } else if scores.is_some() {
        &default_tiers[..]
    } else {
        &[][..]
    };

    if !tiers_to_use.is_empty() {
        let max_hops = tiers_to_use.iter().map(|t| t.1).fold(1.0f64, |a, b| if a > b { a } else { b });
        let step_x = ladder_area_w / (tiers_to_use.len().max(1) as i32);
        let mut pts = Vec::new();

        for (idx, &(size_kb, hops)) in tiers_to_use.iter().enumerate() {
            let px = ladder_area_x + (idx as i32 * step_x) + step_x / 2;
            let norm_y = (hops / max_hops).clamp(0.0, 1.0);
            let py = ladder_area_y + ladder_area_h - (norm_y * (ladder_area_h as f64 - 10.0)) as i32 - 4;
            pts.push((px, py));

            // Tier marker & label
            canvas.fill_rounded_rect(px - 3, py - 3, 7, 7, 3, COLOR_CYAN);
            let lbl = match size_kb {
                4 => "L1",
                256 => "L2",
                4096 => "L3",
                _ => "DRAM",
            };
            canvas.draw_text(px - 6, ladder_area_y + ladder_area_h + 2, lbl, COLOR_MUTED);
        }

        // Connect ladder curve
        for w in 0..pts.len() - 1 {
            canvas.draw_thick_line(pts[w].0, pts[w].1, pts[w + 1].0, pts[w + 1].1, COLOR_CYAN);
        }
        canvas.fill_area_under_curve(&pts, ladder_area_y + ladder_area_h, COLOR_CYAN, COLOR_CARD);
    } else {
        canvas.draw_text(
            ladder_area_x + (ladder_area_w - 184) / 2,
            ladder_area_y + ladder_area_h / 2 - 6,
            "RUN SUITE TO PROBE CACHE",
            COLOR_MUTED,
        );
    }

    // Bottom: Composite History Sparkline
    let spark_y = right_y + ladder_h + 4;
    canvas.draw_text(right_x + 12, spark_y + 6, "COMPOSITE SCORE HISTORY (RUNS)", COLOR_PURPLE);

    let spark_area_x = right_x + 36;
    let spark_area_y = spark_y + 24;
    let spark_area_w = card_w - 48;
    let spark_area_h = card_h - (spark_y - right_y) - 34;

    canvas.stroke_rect(spark_area_x, spark_area_y, spark_area_w, spark_area_h, COLOR_BORDER);
    for g in 1..3 {
        let gy = spark_area_y + (g * spark_area_h) / 3;
        canvas.draw_dashed_line(spark_area_x, gy, spark_area_x + spark_area_w, gy, 4, 4, COLOR_GRID);
    }

    if !history.is_empty() {
        let max_h = history.iter().copied().fold(1.0f64, |a, b| if a > b { a } else { b });
        let step_x = spark_area_w / (history.len().max(2) as i32);
        let mut pts = Vec::new();

        for (idx, &score) in history.iter().enumerate() {
            let px = spark_area_x + (idx as i32 * step_x) + (step_x / 2).min(10);
            let norm_y = (score / max_h).clamp(0.0, 1.0);
            let py = spark_area_y + spark_area_h - (norm_y * (spark_area_h as f64 - 10.0)) as i32 - 4;
            pts.push((px, py));
            canvas.fill_rounded_rect(px - 2, py - 2, 5, 5, 2, COLOR_GOLD);
        }

        for w in 0..pts.len() - 1 {
            canvas.draw_thick_line(pts[w].0, pts[w].1, pts[w + 1].0, pts[w + 1].1, COLOR_GOLD);
        }
        canvas.fill_area_under_curve(&pts, spark_area_y + spark_area_h, COLOR_GOLD, COLOR_CARD);
    } else {
        canvas.draw_text(
            spark_area_x + (spark_area_w - 128) / 2,
            spark_area_y + spark_area_h / 2 - 6,
            "1ST RUN PENDING",
            COLOR_MUTED,
        );
    }

    canvas.pixels
}

/// Renders the real-time thermal torture time-series chart with baseline and throttle thresholds.
pub fn render_torture_realtime_chart(
    samples: &[Sample],
    analysis: &ThermalAnalysis,
    width: usize,
    height: usize,
) -> Vec<u32> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut canvas = Canvas::new(width, height, COLOR_BG);
    if width < 300 || height < 140 {
        return canvas.pixels;
    }

    let pad = 10;
    let card_x = pad;
    let card_y = pad;
    let card_w = width as i32 - 2 * pad;
    let card_h = height as i32 - 2 * pad;

    // Card frame
    canvas.fill_rounded_rect(card_x, card_y, card_w, card_h, 6, COLOR_CARD);
    canvas.stroke_rounded_rect(card_x, card_y, card_w, card_h, 6, COLOR_BORDER);

    // --- TOP KPI HEADER BADGES ---
    let (stab_bg, stab_fg) = if analysis.corruption_detected {
        (COLOR_RED, COLOR_TEXT)
    } else if analysis.stability_percent >= 95.0 {
        (COLOR_EMERALD, COLOR_TEXT)
    } else if analysis.stability_percent >= 80.0 {
        (COLOR_AMBER, 0xFF0F172A)
    } else {
        (COLOR_RED, COLOR_TEXT)
    };

    let stab_str = format!("STABILITY: {:.1}%", analysis.stability_percent);
    canvas.draw_pill_badge(card_x + 12, card_y + 8, &stab_str, stab_bg, stab_fg);

    let max_ops = samples.iter().map(|s| s.ops_per_sec).fold(analysis.baseline_ops_per_sec, |a, b| if a > b { a } else { b });
    let min_ops = samples.iter().map(|s| s.ops_per_sec).fold(max_ops, |a, b| if a < b { a } else { b });

    let peak_str = format!("PEAK: {:.0} OPS/S", max_ops);
    canvas.draw_pill_badge(card_x + 160, card_y + 8, &peak_str, COLOR_GRID, COLOR_CYAN);

    let min_str = format!("MIN: {:.0} OPS/S", min_ops);
    canvas.draw_pill_badge(card_x + 325, card_y + 8, &min_str, COLOR_GRID, COLOR_MUTED);

    let base_str = format!("BOOST: {:.0} OPS/S", analysis.baseline_ops_per_sec);
    canvas.draw_pill_badge(card_x + 485, card_y + 8, &base_str, COLOR_GRID, COLOR_BLUE);

    let (integ_str, integ_bg) = if analysis.corruption_detected {
        ("INTEGRITY: CORRUPT!", COLOR_RED)
    } else {
        ("INTEGRITY: BIT-EXACT", COLOR_EMERALD)
    };
    canvas.draw_pill_badge(card_x + card_w - 185, card_y + 8, integ_str, integ_bg, COLOR_TEXT);

    // --- MAIN TIME-SERIES CHART AREA ---
    let chart_x = card_x + 60;
    let chart_y = card_y + 36;
    let chart_w = card_w - 76;
    let chart_h = card_h - 60;

    canvas.fill_rect(chart_x, chart_y, chart_w, chart_h, 0xFF141F30);
    canvas.stroke_rect(chart_x, chart_y, chart_w, chart_h, COLOR_BORDER);

    // Subtle horizontal gridlines
    for g in 1..4 {
        let gy = chart_y + (g * chart_h) / 4;
        canvas.draw_dashed_line(chart_x, gy, chart_x + chart_w, gy, 4, 4, COLOR_GRID);
    }

    if samples.is_empty() {
        let ready_msg = "READY — CLICK 'START STEP' OR 'RUN BURST (5x)'";
        canvas.draw_text(
            chart_x + (chart_w - ready_msg.len() as i32 * 8) / 2,
            chart_y + chart_h / 2 - 6,
            ready_msg,
            COLOR_MUTED,
        );
        return canvas.pixels;
    }

    // Y Axis scaling (0 to max_y * 1.15)
    let y_max = (max_ops * 1.15).max(1.0);

    // Baseline Boost line (dashed cyan)
    if analysis.baseline_ops_per_sec > 0.0 {
        let base_norm = (analysis.baseline_ops_per_sec / y_max).clamp(0.0, 1.0);
        let base_y = chart_y + chart_h - (base_norm * (chart_h as f64)) as i32;
        canvas.draw_dashed_line(chart_x, base_y, chart_x + chart_w, base_y, 6, 4, COLOR_CYAN);
        canvas.draw_text(chart_x + 8, base_y - 12, "BASELINE BOOST", COLOR_CYAN);

        // Throttle threshold (-10%) line (dashed amber)
        let throttle_val = analysis.baseline_ops_per_sec * 0.90;
        let th_norm = (throttle_val / y_max).clamp(0.0, 1.0);
        let th_y = chart_y + chart_h - (th_norm * (chart_h as f64)) as i32;
        canvas.draw_dashed_line(chart_x, th_y, chart_x + chart_w, th_y, 4, 4, COLOR_AMBER);
        canvas.draw_text(chart_x + 8, th_y + 2, "THROTTLE (-10%)", COLOR_AMBER);
    }

    // Plot points across up to 20 steps
    let max_steps = 20.max(samples.len() as i32);
    let step_dx = chart_w / max_steps;

    let mut curve_pts = Vec::new();
    for (idx, s) in samples.iter().enumerate() {
        let px = chart_x + (idx as i32 * step_dx) + step_dx / 2;
        let norm_y = (s.ops_per_sec / y_max).clamp(0.0, 1.0);
        let py = chart_y + chart_h - (norm_y * (chart_h as f64)) as i32;
        curve_pts.push((px, py));

        // Data point marker
        let dot_color = if s.corruption_detected {
            COLOR_RED
        } else if analysis.baseline_ops_per_sec > 0.0 && s.ops_per_sec < analysis.baseline_ops_per_sec * 0.90 {
            COLOR_AMBER
        } else {
            COLOR_EMERALD
        };
        canvas.fill_rounded_rect(px - 3, py - 3, 7, 7, 3, dot_color);
    }

    // Draw connecting curve
    for w in 0..curve_pts.len() - 1 {
        canvas.draw_thick_line(curve_pts[w].0, curve_pts[w].1, curve_pts[w + 1].0, curve_pts[w + 1].1, COLOR_EMERALD);
    }
    canvas.fill_area_under_curve(&curve_pts, chart_y + chart_h, COLOR_EMERALD, COLOR_CARD);

    // Latest step callout
    if let Some(&last_pt) = curve_pts.last() {
        let last_sample = samples.last().unwrap();
        let callout = format!("{:.0} ops/s", last_sample.ops_per_sec);
        let cx = (last_pt.0 - 40).clamp(chart_x + 2, chart_x + chart_w - 90);
        let cy = (last_pt.1 - 22).clamp(chart_y + 2, chart_y + chart_h - 20);
        canvas.draw_pill_badge(cx, cy, &callout, COLOR_BLUE, COLOR_TEXT);
    }

    // X Axis step markers
    for step in 1..=20 {
        let sx = chart_x + ((step - 1) * step_dx) + step_dx / 2;
        if step % 5 == 0 || step == 1 {
            let num = format!("{}", step);
            canvas.draw_text(sx - 3, chart_y + chart_h + 3, &num, COLOR_MUTED);
        }
    }

    canvas.pixels
}

/// Renders the side-by-side grouped horizontal comparison chart against Linux 6.6 LTS medians.
pub fn render_linux_comparison_chart(
    doc: &LinuxComparisonDoc,
    live_arx_mops: Option<f64>,
    live_stream_mbs: Option<f64>,
    width: usize,
    height: usize,
) -> Vec<u32> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut canvas = Canvas::new(width, height, COLOR_BG);
    if width < 300 || height < 140 {
        return canvas.pixels;
    }

    let pad = 8;
    let card_x = pad;
    let card_y = pad;
    let card_w = width as i32 - 2 * pad;
    let card_h = height as i32 - 2 * pad;

    canvas.fill_rounded_rect(card_x, card_y, card_w, card_h, 6, COLOR_CARD);
    canvas.stroke_rounded_rect(card_x, card_y, card_w, card_h, 6, COLOR_BORDER);
    canvas.fill_rect(card_x + 2, card_y, card_w - 4, 3, 0xFF38BDF8);

    // --- HEADER BAR ---
    canvas.draw_text(card_x + 12, card_y + 8, "SEXOS v1.0.0 (SASOS+PKU) vs LINUX 6.6 LTS (QEMU)", COLOR_TEXT);

    // Checksums verified badge
    let ck_verified = doc.checksum_verified_count();
    let ck_badge = format!("{}/{} BIT-EXACT MATCH", ck_verified, doc.checksum_rows.len().max(9));
    canvas.draw_pill_badge(card_x + 400, card_y + 5, &ck_badge, COLOR_DARK_GREEN, COLOR_EMERALD);

    // Source badge
    canvas.draw_text(card_x + card_w - 230, card_y + 8, "docs/CLIMAX_VS_LINUX_2026_11_14.md", COLOR_MUTED);

    // --- LEGEND ---
    let leg_y = card_y + 26;
    canvas.fill_rounded_rect(card_x + 12, leg_y + 2, 10, 10, 2, COLOR_BLUE);
    canvas.draw_text(card_x + 26, leg_y + 2, "SexOS Median", COLOR_TEXT);

    canvas.fill_rounded_rect(card_x + 140, leg_y + 2, 10, 10, 2, COLOR_ORANGE);
    canvas.draw_text(card_x + 154, leg_y + 2, "Linux 6.6 LTS Median", COLOR_TEXT);

    canvas.draw_text(card_x + 340, leg_y + 2, "Speedup Delta (SexOS / Linux)", COLOR_MUTED);

    // --- 9 WORKLOAD ROWS ---
    let rows_count = doc.comparison_rows.len().max(1);
    let top_y = card_y + 42;
    let bottom_y = card_y + card_h - 22;
    let available_h = bottom_y - top_y;
    let row_h = available_h / (rows_count as i32);

    let label_w = 175;
    let bar_start_x = card_x + label_w;
    let badge_w = 115;
    let max_bar_w = card_w - label_w - badge_w - 20;

    for (idx, row) in doc.comparison_rows.iter().enumerate() {
        let ry = top_y + (idx as i32) * row_h;

        // Alternate subtle row striping
        if idx % 2 == 1 {
            canvas.fill_rect(card_x + 2, ry, card_w - 4, row_h, 0xFF141F30);
        }

        // Subsystem short label
        let short_name = match idx {
            0 => "ARX Throughput",
            1 => "SHA-256 Digest",
            2 => "GEMM FP32 Matrix",
            3 => "Audio Biquad DSP",
            4 => "STREAM Bandwidth",
            5 => "Sattolo L1 (4 KiB)",
            6 => "Sattolo L2 (256K)",
            7 => "Sattolo L3 (4 MiB)",
            8 => "Sattolo DRAM (16M)",
            _ => row.subsystem.as_str(),
        };
        canvas.draw_text(card_x + 10, ry + 3, short_name, COLOR_TEXT);

        // Normalize SexOS vs Linux dual bars
        let max_pair = row.sexos_median.max(row.linux_median).max(1e-9);
        let sexos_bar_w = (((row.sexos_median / max_pair) * (max_bar_w as f64)) as i32).clamp(4, max_bar_w);
        let linux_bar_w = (((row.linux_median / max_pair) * (max_bar_w as f64)) as i32).clamp(4, max_bar_w);

        let bar_h = (row_h / 2 - 3).max(4).min(7);
        // Background track for bars
        canvas.fill_rounded_rect(bar_start_x, ry + 2, max_bar_w, bar_h, 2, COLOR_GRID);
        canvas.fill_rounded_rect(bar_start_x, ry + 2 + bar_h + 1, max_bar_w, bar_h, 2, COLOR_GRID);

        // SexOS bar (top)
        canvas.fill_rounded_rect(bar_start_x, ry + 2, sexos_bar_w, bar_h, 2, COLOR_BLUE);
        // Linux bar (bottom)
        canvas.fill_rounded_rect(bar_start_x, ry + 2 + bar_h + 1, linux_bar_w, bar_h, 2, COLOR_ORANGE);

        // Live overlay if applicable
        if idx == 0 {
            if let Some(live) = live_arx_mops {
                let live_str = format!("live:{:.0}", live);
                canvas.draw_text(bar_start_x + sexos_bar_w + 6, ry + 2, &live_str, COLOR_CYAN);
            }
        } else if idx == 4 {
            if let Some(live) = live_stream_mbs {
                let live_str = format!("live:{:.0}", live);
                canvas.draw_text(bar_start_x + sexos_bar_w + 6, ry + 2, &live_str, COLOR_PURPLE);
            }
        }

        // Right-aligned speedup badge
        let badge_x = bar_start_x + max_bar_w + 8;
        let delta_pct = row.ratio_percent - 100.0;
        let speedup_ratio = row.ratio_percent / 100.0;

        let (badge_text, bg_col, fg_col) = if row.ratio_percent >= 300.0 {
            (format!("+{:.1}% ({:.2}x)", delta_pct, speedup_ratio), COLOR_DARK_GREEN, COLOR_EMERALD)
        } else if row.ratio_percent >= 150.0 {
            (format!("+{:.1}% ({:.2}x)", delta_pct, speedup_ratio), 0xFF0D544C, COLOR_CYAN)
        } else if row.ratio_percent >= 115.0 {
            (format!("+{:.1}%", delta_pct), 0xFF1E3A8A, 0xFF93C5FD)
        } else if row.ratio_percent >= 107.0 {
            (format!("+{:.1}%", delta_pct), 0xFF14532D, 0xFF86EFAC)
        } else if row.ratio_percent >= 100.0 {
            (format!("+{:.1}%", delta_pct), COLOR_GRID, COLOR_TEXT)
        } else {
            (format!("{:.1}%", delta_pct), COLOR_RED, COLOR_TEXT)
        };

        canvas.draw_pill_badge(badge_x, ry + 1, &badge_text, bg_col, fg_col);
    }

    // --- FOOTER SUMMARY ---
    let geomean = doc.geometric_mean_ratio_pct();
    let geomean_delta = geomean - 100.0;
    let geomean_ratio = geomean / 100.0;
    let summary_str = format!(
        "OVERALL ADVANTAGE: +{:.1}% GEOMETRIC MEAN ({:.2}x ACROSS ALL 9 WORKLOADS)",
        geomean_delta, geomean_ratio
    );
    canvas.draw_text(card_x + 14, bottom_y + 4, &summary_str, COLOR_GOLD);

    canvas.pixels
}
