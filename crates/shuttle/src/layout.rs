// SPDX-License-Identifier: GPL-3.0-only
//! Multi-line paragraph layout, line breaking, bidirectional cursor mapping, and software blitting.
//!
//! Converts itemized shaped runs into wrapped visual lines respecting word and CJK
//! ideograph break opportunities, handles text alignment (`Start`, `End`, `Left`,
//! `Right`, `Center`), computes cursor coordinates and mouse hit-testing, and
//! renders glyphs into 32-bit `0xAARRGGBB` pixel buffers.

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

use crate::bidi::{BaseDirection, Direction, Paragraph};
use crate::emoji::{blit_color_argb, blit_coverage, rasterize_emoji};
use crate::itemize::itemize_paragraph;
use crate::shaper::Shaper;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Start,
    End,
    Left,
    Right,
    Center,
}

#[derive(Clone, Debug)]
pub struct LayoutOptions {
    pub max_width: Option<f32>,
    pub align: Alignment,
    pub line_height: Option<f32>,
    pub font_size: f32,
    pub base_direction: BaseDirection,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            align: Alignment::Start,
            line_height: None,
            font_size: 16.0,
            base_direction: BaseDirection::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub line_index: usize,
    pub is_rtl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitResult {
    pub byte_offset: usize,
    pub is_trailing: bool,
    pub line_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualGlyph {
    pub font_id: usize,
    pub glyph_id: u16,
    pub scalar: u32,
    pub is_emoji: bool,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    pub cluster: u32,
    pub is_rtl: bool,
    pub x_offset: f32,
    pub run_id: usize,
    pub bidi_level: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualLine {
    pub y: f32,
    pub height: f32,
    pub baseline: f32,
    pub width: f32,
    pub glyphs: Vec<VisualGlyph>,
    pub byte_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphLayout {
    pub lines: Vec<VisualLine>,
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub is_paragraph_rtl: bool,
}

impl ParagraphLayout {
    /// Maps a byte offset in `text` to cursor geometric bounds `{ x, y, width, height }`.
    pub fn byte_to_cursor(&self, byte_offset: usize) -> CursorRect {
        if self.lines.is_empty() {
            return CursorRect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 16.0,
                line_index: 0,
                is_rtl: self.is_paragraph_rtl,
            };
        }

        // Find the line containing `byte_offset`
        let (line_idx, line) = self
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| byte_offset >= l.byte_range.start && byte_offset <= l.byte_range.end)
            .unwrap_or_else(|| (self.lines.len() - 1, self.lines.last().unwrap()));

        if line.glyphs.is_empty() {
            return CursorRect {
                x: 0.0,
                y: line.y,
                width: 2.0,
                height: line.height,
                line_index: line_idx,
                is_rtl: self.is_paragraph_rtl,
            };
        }

        // Look for the visual glyph matching `byte_offset`
        for g in &line.glyphs {
            if g.cluster as usize == byte_offset {
                let x = if g.is_rtl { g.x + g.advance } else { g.x };
                return CursorRect {
                    x,
                    y: line.y,
                    width: 2.0,
                    height: line.height,
                    line_index: line_idx,
                    is_rtl: g.is_rtl,
                };
            }
        }

        // If at the end of the line
        let last = line.glyphs.last().unwrap();
        let x = if self.is_paragraph_rtl || last.is_rtl {
            line.glyphs.first().map_or(0.0, |g| g.x)
        } else {
            last.x + last.advance
        };

        CursorRect {
            x,
            y: line.y,
            width: 2.0,
            height: line.height,
            line_index: line_idx,
            is_rtl: self.is_paragraph_rtl,
        }
    }

    /// Hit-tests a visual point `(x, y)` to the nearest character byte offset.
    pub fn hit_test(&self, x: f32, y: f32) -> HitResult {
        if self.lines.is_empty() {
            return HitResult {
                byte_offset: 0,
                is_trailing: false,
                line_index: 0,
            };
        }

        let line_idx = self
            .lines
            .iter()
            .position(|l| y >= l.y && y < l.y + l.height)
            .unwrap_or_else(|| {
                if y < self.lines[0].y {
                    0
                } else {
                    self.lines.len() - 1
                }
            });

        let line = &self.lines[line_idx];
        if line.glyphs.is_empty() {
            return HitResult {
                byte_offset: line.byte_range.start,
                is_trailing: false,
                line_index: line_idx,
            };
        }

        // Find nearest glyph on the line
        for g in &line.glyphs {
            let left = g.x;
            let right = g.x + g.advance;
            if x >= left && x <= right {
                let mid = left + g.advance * 0.5;
                let is_trailing = if g.is_rtl { x < mid } else { x >= mid };
                return HitResult {
                    byte_offset: g.cluster as usize,
                    is_trailing,
                    line_index: line_idx,
                };
            }
        }

        // Before first glyph or after last glyph
        let first = line.glyphs.first().unwrap();
        let last = line.glyphs.last().unwrap();
        if x < first.x {
            HitResult {
                byte_offset: if first.is_rtl {
                    last.cluster as usize
                } else {
                    first.cluster as usize
                },
                is_trailing: false,
                line_index: line_idx,
            }
        } else {
            HitResult {
                byte_offset: if last.is_rtl {
                    first.cluster as usize
                } else {
                    last.cluster as usize
                },
                is_trailing: true,
                line_index: line_idx,
            }
        }
    }

    /// Renders this paragraph into a 32-bit `0xAARRGGBB` surface.
    pub fn draw(
        &self,
        shaper: &Shaper,
        target: &mut [u32],
        stride: usize,
        width: usize,
        height: usize,
        origin_x: i32,
        origin_y: i32,
        color_argb: u32,
    ) {
        if target.is_empty() || width == 0 || height == 0 {
            return;
        }

        for line in &self.lines {
            let line_y = origin_y + line.y as i32;
            let baseline_y = line_y + line.baseline as i32;

            for glyph in &line.glyphs {
                let glyph_x = origin_x + (glyph.x + glyph.x_offset) as i32;
                let glyph_y = baseline_y + glyph.y as i32;

                if glyph.is_emoji {
                    let side = (line.height * 0.9).max(12.0).min(512.0) as usize;
                    if let Some(raster) = rasterize_emoji(glyph.scalar, side) {
                        let emoji_y = line_y + ((line.height - side as f32) * 0.5) as i32;
                        if let Some(ref color_px) = raster.color_pixels {
                            blit_color_argb(target, stride, width, height, glyph_x, emoji_y, color_px, side, side);
                        } else {
                            blit_coverage(
                                target,
                                stride,
                                width,
                                height,
                                glyph_x,
                                emoji_y,
                                &raster.coverage,
                                side,
                                side,
                                color_argb,
                            );
                        }
                    }
                } else {
                    let font_opt = shaper.font_by_id(glyph.font_id);
                    if let Some(font) = font_opt {
                        let size_units = (line.height * 64.0 + 0.5) as u32;
                        let gid = sex_font::GlyphId(glyph.glyph_id);
                        let asc = font.ascender(size_units) / 256 + 2;
                        let desc = -font.descender(size_units) / 256 + 2;
                        let adv = font.advance(gid, size_units) / 256;
                        let pad = (line.height as i32) / 3 + 2;
                        let w = (adv + 2 * pad).max(1) as usize;
                        let h = (asc + desc).max(1) as usize;

                        let need = sex_font::raster::scratch_len(w, h);
                        let mut acc = vec![0i32; need];
                        let mut coverage = vec![0u8; w * h];

                        let draw_x = pad * 256;
                        let draw_baseline = asc * 256;

                        if font.draw(
                            gid,
                            size_units,
                            draw_x,
                            draw_baseline,
                            w,
                            h,
                            &mut acc,
                            &mut coverage,
                        ) {
                            blit_coverage(
                                target,
                                stride,
                                width,
                                height,
                                glyph_x - pad,
                                glyph_y - asc,
                                &coverage,
                                w,
                                h,
                                color_argb,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Builds a multi-line paragraph layout from `text`.
pub fn layout_paragraph(shaper: &Shaper, text: &str, options: &LayoutOptions) -> ParagraphLayout {
    let paras = Paragraph::split_and_resolve(text, options.base_direction);
    let is_para_rtl = paras.first().map_or(false, |p| p.direction() == Direction::Rtl);

    let font_size = if options.font_size.is_finite() && options.font_size > 0.0 {
        options.font_size.clamp(1.0, 512.0)
    } else {
        16.0
    };
    let line_height = options.line_height.unwrap_or(font_size * 1.25);
    let max_width = if let Some(w) = options.max_width {
        if w.is_finite() && w > 0.0 {
            w
        } else {
            0.0
        }
    } else {
        f32::INFINITY
    };

    let mut all_lines: Vec<VisualLine> = Vec::new();
    let mut current_y = 0.0;

    for para in &paras {
        let items = itemize_paragraph(para);
        let mut para_glyphs: Vec<VisualGlyph> = Vec::new();

        // Shape all items in this paragraph
        for (item_idx, item) in items.iter().enumerate() {
            let shaped_glyphs = shaper.shape_run(item, font_size);
            let mut run_glyphs: Vec<VisualGlyph> = Vec::with_capacity(shaped_glyphs.len());
            for sg in shaped_glyphs {
                run_glyphs.push(VisualGlyph {
                    font_id: sg.font_id,
                    glyph_id: sg.glyph_id,
                    scalar: sg.scalar,
                    is_emoji: sg.is_emoji,
                    x: 0.0,
                    y: sg.y_offset,
                    advance: sg.x_advance,
                    cluster: sg.cluster,
                    is_rtl: item.is_rtl(),
                    x_offset: sg.x_offset,
                    run_id: item_idx,
                    bidi_level: item.level,
                });
            }

            if item.is_rtl() && run_glyphs.len() > 1 {
                reverse_cluster_groups(&mut run_glyphs);
            }

            para_glyphs.extend(run_glyphs);
        }

        // Line-breaking into visual lines
        let mut line_start = 0;
        let mut current_width = 0.0;
        let mut last_break_idx = None;

        let n = para_glyphs.len();
        let mut i = 0;

        while i < n {
            let adv = para_glyphs[i].advance;
            let c = char::from_u32(para_glyphs[i].scalar).unwrap_or(' ');

            if current_width + adv > max_width && i > line_start {
                let break_at = last_break_idx.filter(|&b| b > line_start).unwrap_or(i);
                all_lines.push(build_visual_line(
                    &para_glyphs[line_start..break_at],
                    current_y,
                    line_height,
                    font_size * 0.8,
                    max_width,
                    options.align,
                    is_para_rtl,
                    text.len(),
                ));
                current_y += line_height;
                line_start = break_at;
                current_width = 0.0;
                last_break_idx = None;
                i = break_at;
                continue;
            }

            current_width += adv;

            // Break opportunities: whitespace, hyphens, or CJK ideographs
            if c == ' ' || c == '-' || is_cjk_break_boundary(c) {
                last_break_idx = Some(i + 1);
            }

            i += 1;
        }

        if line_start < n || all_lines.is_empty() {
            all_lines.push(build_visual_line(
                &para_glyphs[line_start..n],
                current_y,
                line_height,
                font_size * 0.8,
                max_width,
                options.align,
                is_para_rtl,
                text.len(),
            ));
            current_y += line_height;
        }
    }

    let total_width = all_lines
        .iter()
        .map(|l| l.width)
        .fold(0.0f32, |a, b| if a > b { a } else { b });

    ParagraphLayout {
        lines: all_lines,
        width: total_width,
        height: current_y,
        text: String::from(text),
        is_paragraph_rtl: is_para_rtl,
    }
}

#[derive(Clone, Copy, Debug)]
struct LineRunSlice {
    level: u8,
    start: usize,
    end: usize,
}

/// Constructs a `VisualLine` with proper bidirectional run placement and alignment offset.
fn build_visual_line(
    glyphs_slice: &[VisualGlyph],
    y: f32,
    height: f32,
    baseline: f32,
    max_width: f32,
    align: Alignment,
    is_para_rtl: bool,
    text_len: usize,
) -> VisualLine {
    if glyphs_slice.is_empty() {
        return VisualLine {
            y,
            height,
            baseline,
            width: 0.0,
            glyphs: Vec::new(),
            byte_range: 0..0,
        };
    }

    // 1. Group glyphs into contiguous run slices
    let n = glyphs_slice.len();
    let mut run_slices: Vec<LineRunSlice> = Vec::new();
    let mut g_start = 0;
    while g_start < n {
        let run_id = glyphs_slice[g_start].run_id;
        let level = glyphs_slice[g_start].bidi_level;
        let mut g_end = g_start + 1;
        while g_end < n
            && glyphs_slice[g_end].run_id == run_id
            && glyphs_slice[g_end].bidi_level == level
        {
            g_end += 1;
        }
        run_slices.push(LineRunSlice {
            level,
            start: g_start,
            end: g_end,
        });
        g_start = g_end;
    }

    // 2. Reorder run slices according to UAX #9 Rule L2
    if run_slices.len() > 1 {
        let high = run_slices.iter().map(|r| r.level).max().unwrap_or(0);
        let low_odd = run_slices
            .iter()
            .map(|r| r.level)
            .filter(|&l| l % 2 == 1)
            .min()
            .unwrap_or(high + 1);

        let mut level = high;
        while level >= low_odd && level > 0 {
            let mut i = 0;
            while i < run_slices.len() {
                if run_slices[i].level >= level {
                    let s = i;
                    while i < run_slices.len() && run_slices[i].level >= level {
                        i += 1;
                    }
                    run_slices[s..i].reverse();
                } else {
                    i += 1;
                }
            }
            level -= 1;
        }
    }

    // 3. Assemble visual glyphs in display order
    let mut visual_glyphs = Vec::with_capacity(n);
    for slice in &run_slices {
        let mut run_slice_glyphs = glyphs_slice[slice.start..slice.end].to_vec();
        if slice.level % 2 == 1 && run_slice_glyphs.len() > 1 {
            reverse_cluster_groups(&mut run_slice_glyphs);
        }
        visual_glyphs.extend(run_slice_glyphs);
    }

    let line_width: f32 = visual_glyphs.iter().map(|g| g.advance).sum();

    // 4. Horizontal alignment offset
    let offset_x = match align {
        Alignment::Left => 0.0,
        Alignment::Right => {
            if max_width.is_finite() {
                (max_width - line_width).max(0.0)
            } else {
                0.0
            }
        }
        Alignment::Center => {
            if max_width.is_finite() {
                ((max_width - line_width) * 0.5).max(0.0)
            } else {
                0.0
            }
        }
        Alignment::Start => {
            if is_para_rtl && max_width.is_finite() {
                (max_width - line_width).max(0.0)
            } else {
                0.0
            }
        }
        Alignment::End => {
            if !is_para_rtl && max_width.is_finite() {
                (max_width - line_width).max(0.0)
            } else {
                0.0
            }
        }
    };

    // 5. Position glyphs visually from left to right
    let mut cur_x = offset_x;
    for g in visual_glyphs.iter_mut() {
        g.x = cur_x;
        cur_x += g.advance;
    }

    let start_byte = visual_glyphs.iter().map(|g| g.cluster as usize).min().unwrap_or(0);
    let end_byte = visual_glyphs
        .iter()
        .map(|g| {
            let char_len = char::from_u32(g.scalar).map_or(1, |c| c.len_utf8());
            (g.cluster as usize).saturating_add(char_len)
        })
        .max()
        .unwrap_or(start_byte)
        .min(text_len);

    VisualLine {
        y,
        height,
        baseline,
        width: line_width,
        glyphs: visual_glyphs,
        byte_range: start_byte..end_byte,
    }
}

/// Reverses the sequence of contiguous cluster groups within `glyphs`, while preserving
/// the internal glyph order inside each cluster (keeping base glyphs and combining marks intact).
fn reverse_cluster_groups(glyphs: &mut [VisualGlyph]) {
    if glyphs.len() <= 1 {
        return;
    }
    let n = glyphs.len();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start < n {
        let cl = glyphs[start].cluster;
        let mut end = start + 1;
        while end < n && glyphs[end].cluster == cl {
            end += 1;
        }
        groups.push((start, end));
        start = end;
    }

    if groups.len() <= 1 {
        return;
    }

    let mut out: Vec<VisualGlyph> = Vec::with_capacity(n);
    for &(s, e) in groups.iter().rev() {
        out.extend_from_slice(&glyphs[s..e]);
    }
    glyphs.clone_from_slice(&out);
}

/// Identifies if character is a CJK ideograph permitting line breaking before/after it.
fn is_cjk_break_boundary(c: char) -> bool {
    let s = c as u32;
    matches!(
        s,
        0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x3040..=0x309F // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0xAC00..=0xD7AF // Hangul Syllables
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_cjk_wrapping() {
        let shaper = Shaper::default();
        let text = "天地玄黄宇宙洪荒";
        let opts = LayoutOptions {
            max_width: Some(50.0), // Force line break across CJK characters
            font_size: 16.0,
            ..Default::default()
        };
        let layout = layout_paragraph(&shaper, text, &opts);
        assert!(layout.lines.len() >= 2, "CJK text must wrap at character boundaries");
    }

    #[test]
    fn test_layout_mixed_bidi() {
        let shaper = Shaper::default();
        let text = "Order #42: السلام عليكم";
        let opts = LayoutOptions {
            max_width: Some(400.0),
            font_size: 16.0,
            ..Default::default()
        };
        let layout = layout_paragraph(&shaper, text, &opts);
        assert!(!layout.lines.is_empty());
        let hit = layout.hit_test(10.0, 5.0);
        assert!(hit.byte_offset <= text.len());
    }
}
