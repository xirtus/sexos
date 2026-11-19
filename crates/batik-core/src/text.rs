// SPDX-License-Identifier: GPL-3.0-only
//! Scalable, editable non-destructive typography layer for SexOS Weft / Batik.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_font::raster::scratch_len;
use sex_font::{Font, GEIST_SANS};

use crate::blend::{pack_argb, split_argb};
use crate::error::BatikError;
use crate::layer::RasterLayer;

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayer {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub font_px: f32,
    pub color: u32,
    pub bold: bool,
    pub letter_spacing: i16,
    pub line_height_pct: u16,
    pub drop_shadow: bool,
    pub mask: Option<Vec<u8>>,
}

impl TextLayer {
    pub fn new(text: &str, x: i32, y: i32, font_px: f32, color: u32) -> Self {
        Self {
            text: String::from(text),
            x,
            y,
            font_px: if font_px.is_finite() && font_px > 0.0 { font_px } else { 24.0 },
            color,
            bold: false,
            letter_spacing: 0,
            line_height_pct: 120,
            drop_shadow: false,
            mask: None,
        }
    }

    pub fn with_shadow(mut self, shadow: bool) -> Self {
        self.drop_shadow = shadow;
        self
    }

    pub fn with_bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    pub fn with_spacing(mut self, spacing: i16) -> Self {
        self.letter_spacing = spacing;
        self
    }

    pub fn render_to_buffer(&self, width: usize, height: usize) -> Vec<u32> {
        let total = match width.checked_mul(height) {
            Some(t) => t,
            None => return Vec::new(),
        };
        let mut buffer = vec![0u32; total];
        self.render_into_slice(&mut buffer, width, height);
        buffer
    }

    pub fn render_into_slice(&self, buffer: &mut [u32], width: usize, height: usize) {
        if width == 0 || height == 0 || buffer.len() < width * height {
            return;
        }

        if self.drop_shadow {
            let shadow_color = 0x80000000;
            self.draw_text_lines(buffer, width, height, self.x + 2, self.y + 2, shadow_color);
        }

        self.draw_text_lines(buffer, width, height, self.x, self.y, self.color);
    }

    fn draw_text_lines(
        &self,
        buffer: &mut [u32],
        width: usize,
        height: usize,
        start_x: i32,
        start_y: i32,
        color: u32,
    ) {
        let font = match Font::new(GEIST_SANS) {
            Some(f) => f,
            None => {
                self.draw_fallback_bitmap(buffer, width, height, start_x, start_y, color);
                return;
            }
        };

        let font_sz = self.font_px.clamp(6.0, 256.0);
        let size_arg = (font_sz * 64.0) as u32;
        let line_height = (font_sz * (self.line_height_pct as f32 / 100.0)).max(font_sz);
        let asc_256 = font.ascender(size_arg);

        let glyph_box_w = (font_sz * 1.5).max(12.0) as usize;
        let glyph_box_h = (font_sz * 1.5).max(12.0) as usize;
        let mut acc = vec![0i32; scratch_len(glyph_box_w, glyph_box_h)];
        let mut out = vec![0u8; glyph_box_w * glyph_box_h];

        let (ca, cr, cg, cb) = split_argb(color);

        for (line_idx, line) in self.text.split('\n').enumerate() {
            let baseline_y = start_y as f32 + font_sz * 0.85 + (line_idx as f32 * line_height);
            let mut cur_x = start_x as f32;
            let mut prev_glyph = None;

            for ch in line.chars() {
                let scalar = ch as u32;
                let glyph = match font.glyph(scalar) {
                    Some(g) => g,
                    None => {
                        cur_x += font_sz * 0.5 + self.letter_spacing as f32;
                        continue;
                    }
                };

                if let Some(prev) = prev_glyph {
                    let kern = font.kerning(prev, glyph, size_arg);
                    cur_x += kern as f32 / 256.0;
                }
                prev_glyph = Some(glyph);

                out.fill(0);
                let baseline_local = asc_256;
                if font.draw(glyph, size_arg, 0, baseline_local, glyph_box_w, glyph_box_h, &mut acc, &mut out) {
                    let dst_origin_x = cur_x as i32;
                    let dst_origin_y = (baseline_y - (asc_256 as f32 / 256.0)) as i32;

                    for gy in 0..glyph_box_h {
                        let dy = dst_origin_y + gy as i32;
                        if dy < 0 || dy >= height as i32 {
                            continue;
                        }
                        let row_offset = dy as usize * width;

                        for gx in 0..glyph_box_w {
                            let cov = out[gy * glyph_box_w + gx];
                            if cov > 0 {
                                let dx = dst_origin_x + gx as i32;
                                blend_glyph_pixel(buffer, row_offset, dx, width, ca, cr, cg, cb, cov);
                                if self.bold {
                                    blend_glyph_pixel(buffer, row_offset, dx + 1, width, ca, cr, cg, cb, cov);
                                }
                            }
                        }
                    }
                }

                let adv = font.advance(glyph, size_arg);
                cur_x += adv as f32 / 256.0 + self.letter_spacing as f32;
            }
        }
    }

    fn draw_fallback_bitmap(
        &self,
        buffer: &mut [u32],
        width: usize,
        height: usize,
        start_x: i32,
        start_y: i32,
        color: u32,
    ) {
        let font_scale = (self.font_px / 10.0).max(1.0) as i32;
        let line_height = ((self.font_px * 1.2) as i32).max(12);

        for (line_idx, line) in self.text.split('\n').enumerate() {
            let ly = start_y + line_idx as i32 * line_height;
            for (char_idx, ch) in line.chars().enumerate() {
                let lx = start_x + char_idx as i32 * (8 * font_scale + self.letter_spacing as i32);
                let glyph_byte = (ch as u8).wrapping_mul(37);
                for py in 0..(8 * font_scale) {
                    let cy = ly + py;
                    if cy < 0 || cy >= height as i32 {
                        continue;
                    }
                    let row = cy as usize * width;
                    for px in 0..(8 * font_scale) {
                        let cx = lx + px;
                        if cx < 0 || cx >= width as i32 {
                            continue;
                        }
                        let bit = ((glyph_byte >> ((py / font_scale) % 8)) & 1) != 0;
                        if bit {
                            buffer[row + cx as usize] = color;
                        }
                    }
                }
            }
        }
    }

    pub fn to_raster_layer(&self, width: usize, height: usize) -> Result<RasterLayer, BatikError> {
        let pixels = self.render_to_buffer(width, height);
        Ok(RasterLayer {
            width,
            height,
            offset_x: 0,
            offset_y: 0,
            pixels,
            mask: self.mask.clone(),
            alpha_lock: false,
            pixel_lock: false,
        })
    }
}

#[inline]
fn blend_glyph_pixel(
    buffer: &mut [u32],
    row_offset: usize,
    dx: i32,
    width: usize,
    ca: u8,
    cr: u8,
    cg: u8,
    cb: u8,
    cov: u8,
) {
    if dx >= 0 && (dx as usize) < width {
        let idx = row_offset + dx as usize;
        let effective_a = ((ca as u32 * cov as u32 + 127) / 255) as u8;
        if effective_a > 0 {
            let bg = buffer[idx];
            let (ba, br, bg_c, bb) = split_argb(bg);
            if ba == 0 {
                buffer[idx] = pack_argb(effective_a, cr, cg, cb);
            } else {
                let sa = effective_a as u32;
                let da = ba as u32;
                let out_a = sa + da - (sa * da) / 255;
                if out_a > 0 {
                    let out_r = ((cr as u32 * sa + br as u32 * da * (255 - sa) / 255) / out_a).min(255) as u8;
                    let out_g = ((cg as u32 * sa + bg_c as u32 * da * (255 - sa) / 255) / out_a).min(255) as u8;
                    let out_b = ((cb as u32 * sa + bb as u32 * da * (255 - sa) / 255) / out_a).min(255) as u8;
                    buffer[idx] = pack_argb(out_a as u8, out_r, out_g, out_b);
                }
            }
        }
    }
}
