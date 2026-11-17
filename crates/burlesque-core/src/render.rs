// SPDX-License-Identifier: GPL-3.0-only
//! Software rasterizer for slides, vector shapes, typography, and images.

use alloc::vec;
use sex_canvas::{Canvas, Color, Rect};
use sex_font::raster::scratch_len;
use sex_font::{Font, GEIST_MONO, GEIST_SANS};
use crate::animation::BuildEffect;
use crate::deck::AspectRatio;
use crate::elements::{
    Background, ShapeKind, SlideElement, StrokeStyle, TextBox, Transition,
};
use crate::theme::Theme;
use crate::Slide;

/// Render a single slide onto a `0xAARRGGBB` pixel buffer.
pub fn render_slide(
    slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    render_slide_at_build(
        slide,
        theme,
        aspect_ratio,
        usize::MAX,
        None,
        buffer,
        target_w,
        target_h,
    );
}

/// Render a slide at a specific intra-slide build animation stage.
pub fn render_slide_at_build(
    slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    completed_builds: usize,
    active_build_progress: Option<f32>,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    render_slide_at_build_with_options(
        slide,
        theme,
        aspect_ratio,
        completed_builds,
        active_build_progress,
        false,
        buffer,
        target_w,
        target_h,
    );
}

/// Render a slide at a build stage with optional transparent background (for live overlays).
#[allow(clippy::too_many_arguments)]
pub fn render_slide_at_build_with_options(
    slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    completed_builds: usize,
    active_build_progress: Option<f32>,
    transparent_background: bool,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    let needed = match (target_w as usize).checked_mul(target_h as usize) {
        Some(n) => n,
        None => return,
    };
    if target_w == 0 || target_h == 0 || buffer.len() < needed {
        return;
    }

    let mut canvas = match Canvas::new(buffer, target_w, target_h) {
        Some(c) => c,
        None => return,
    };

    let deck_w = aspect_ratio.width();
    let deck_h = aspect_ratio.height();
    let sx = target_w as f32 / deck_w;
    let sy = target_h as f32 / deck_h;

    // 1. Background
    if transparent_background {
        canvas.clear(Color::TRANSPARENT);
    } else {
        match slide.background_override {
            Some(Background::Solid(c)) => canvas.clear(c),
            Some(Background::Gradient(top, bottom)) => {
                canvas.fill_round_rect_gradient(
                    Rect::new(0.0, 0.0, target_w as f32, target_h as f32),
                    0.0,
                    top,
                    bottom,
                );
            }
            None => canvas.clear(theme.palette.background),
        }

        // 2. Master chrome (top accent ribbon, subtle card accents, styled slide number)
        if theme.show_master_chrome {
            render_master_chrome(&mut canvas, theme, slide.id, target_w, target_h, sx, sy);
        }
    }

    let draw_elem_with_meta = |elem_idx: usize, elem: &SlideElement, dst: &mut [u32]| {
        let meta = slide.element_meta(elem_idx);
        if libm::fabsf(meta.rotation_deg) > 0.01 || meta.opacity < 0.999 {
            crate::morph::render_element_transformed(
                elem,
                None,
                meta.rotation_deg,
                meta.opacity,
                theme,
                sx,
                sy,
                dst,
                target_w,
                target_h,
                needed,
            );
        } else if let Some(mut c) = Canvas::new(dst, target_w, target_h) {
            render_single_element(elem, None, None, theme, sx, sy, &mut c, target_w, target_h);
        }
    };

    // 3. Elements with intra-slide builds
    if slide.builds.is_empty() {
        for (elem_idx, elem) in slide.elements.iter().enumerate() {
            draw_elem_with_meta(elem_idx, elem, canvas.pixels_mut());
        }
        return;
    }

    // Determine which build steps are simultaneously animating when WithPrevious chains exist.
    let active_range_end = if active_build_progress.is_some() && completed_builds < slide.builds.len() {
        let mut end = completed_builds + 1;
        while end < slide.builds.len()
            && matches!(slide.builds[end].trigger, crate::animation::BuildTrigger::WithPrevious)
        {
            end += 1;
        }
        end
    } else {
        completed_builds
    };

    for (elem_idx, elem) in slide.elements.iter().enumerate() {
        let mut has_whole = false;
        let mut visible_after_completed = false;
        let mut active_whole = None;
        for (step_k, step) in slide.builds.iter().enumerate() {
            if step.element_index != elem_idx || step.paragraph_index.is_some() {
                continue;
            }
            if !has_whole {
                has_whole = true;
                // Elements whose first whole build is a build-out start visible on slide entry.
                visible_after_completed = step.effect.is_build_out();
            }
            if step_k < completed_builds {
                visible_after_completed = !step.effect.is_build_out();
            } else if active_build_progress.is_some() && step_k < active_range_end {
                active_whole = Some(step);
            }
        }

        if has_whole {
            if let (Some(step), Some(progress)) = (active_whole, active_build_progress) {
                render_element_build_animation(
                    elem,
                    None,
                    step.effect,
                    progress,
                    theme,
                    sx,
                    sy,
                    canvas.pixels_mut(),
                    target_w,
                    target_h,
                );
            } else if visible_after_completed {
                draw_elem_with_meta(elem_idx, elem, canvas.pixels_mut());
            }
        } else if let SlideElement::TextBox(tb) = elem {
            let has_any_para_step = slide
                .builds
                .iter()
                .any(|b| b.element_index == elem_idx && b.paragraph_index.is_some());

            if has_any_para_step {
                let para_filter = |p_idx: usize| -> bool {
                    let para_step = slide
                        .builds
                        .iter()
                        .enumerate()
                        .find(|(_, b)| b.element_index == elem_idx && b.paragraph_index == Some(p_idx));
                    match para_step {
                        None => true,
                        Some((k, s)) => {
                            if k < completed_builds {
                                !s.effect.is_build_out()
                            } else if k >= completed_builds && k < active_range_end {
                                active_build_progress.is_none() && s.effect.is_build_out()
                            } else {
                                s.effect.is_build_out()
                            }
                        }
                    }
                };

                let rect = scale_rect(tb.bounds, sx, sy);
                if let Some(bg) = tb.background {
                    canvas.fill_round_rect(rect, 8.0 * sx, bg);
                }
                render_text_box(tb, rect, theme, sx, sy, canvas.pixels_mut(), target_w, target_h, Some(&para_filter));

                if let Some(progress) = active_build_progress {
                    for k in completed_builds..active_range_end {
                        if let Some(active_step) = slide.builds.get(k) {
                            if active_step.element_index == elem_idx {
                                if let Some(active_p_idx) = active_step.paragraph_index {
                                    let single_p_filter = move |p_idx: usize| p_idx == active_p_idx;
                                    render_element_build_animation(
                                        elem,
                                        Some(&single_p_filter),
                                        active_step.effect,
                                        progress,
                                        theme,
                                        sx,
                                        sy,
                                        canvas.pixels_mut(),
                                        target_w,
                                        target_h,
                                    );
                                }
                            }
                        }
                    }
                }
            } else {
                draw_elem_with_meta(elem_idx, elem, canvas.pixels_mut());
            }
        } else {
            draw_elem_with_meta(elem_idx, elem, canvas.pixels_mut());
        }
    }
}

pub(crate) fn render_single_element(
    elem: &SlideElement,
    override_bounds: Option<Rect>,
    para_filter: Option<&dyn Fn(usize) -> bool>,
    theme: &Theme,
    sx: f32,
    sy: f32,
    canvas: &mut Canvas<'_>,
    target_w: u32,
    target_h: u32,
) {
    let rect = override_bounds.unwrap_or_else(|| scale_rect(elem.bounds(), sx, sy));
    match elem {
        SlideElement::Shape(s) => match s.kind {
            ShapeKind::Rect => {
                if let Some(fill) = s.fill {
                    canvas.fill_rect(rect, fill);
                }
                if let Some(st) = s.stroke {
                    canvas.stroke_round_rect(rect, 0.0, st.width * sx, st.color);
                }
            }
            ShapeKind::RoundedRect { radius } => {
                let r = radius * sx;
                if let Some(fill) = s.fill {
                    canvas.fill_round_rect(rect, r, fill);
                }
                if let Some(st) = s.stroke {
                    canvas.stroke_round_rect(rect, r, st.width * sx, st.color);
                }
            }
            ShapeKind::Ellipse => {
                let cx = rect.x + rect.w / 2.0;
                let cy = rect.y + rect.h / 2.0;
                let r = (rect.w.min(rect.h) / 2.0).max(1.0);
                if let Some(fill) = s.fill {
                    canvas.fill_circle(cx, cy, r, fill);
                }
                if let Some(st) = s.stroke {
                    canvas.stroke_circle(cx, cy, r, st.width * sx, st.color);
                }
            }
            ShapeKind::Line => {
                if let Some(st) = s.stroke {
                    let x0 = rect.x;
                    let y0 = rect.y;
                    let x1 = rect.right();
                    let y1 = rect.bottom();
                    canvas.line(x0, y0, x1, y1, st.width * sx, st.color);
                }
            }
            ShapeKind::Arrow => {
                if let Some(st) = s.stroke {
                    let x0 = rect.x;
                    let y0 = rect.y + rect.h / 2.0;
                    let x1 = rect.right();
                    let y1 = rect.y + rect.h / 2.0;
                    canvas.line(x0, y0, x1, y1, st.width * sx, st.color);
                    let head_len = (rect.h * 0.4).max(6.0);
                    canvas.line(x1, y1, x1 - head_len, y1 - head_len, st.width * sx, st.color);
                    canvas.line(x1, y1, x1 - head_len, y1 + head_len, st.width * sx, st.color);
                }
            }
        },
        SlideElement::TextBox(tb) => {
            if let Some(bg) = tb.background {
                canvas.fill_round_rect(rect, 8.0 * sx, bg);
            }
            render_text_box(tb, rect, theme, sx, sy, canvas.pixels_mut(), target_w, target_h, para_filter);
        }
        SlideElement::Image(img) => {
            render_image(img, rect, canvas.pixels_mut(), target_w, target_h);
        }
        SlideElement::Table(t) => {
            render_table(t, rect, theme, sx, sy, canvas.pixels_mut(), target_w, target_h);
        }
        SlideElement::Code(c) => {
            render_code_block(c, rect, theme, sx, sy, canvas.pixels_mut(), target_w, target_h);
        }
    }
}

fn render_element_build_animation(
    elem: &SlideElement,
    para_filter: Option<&dyn Fn(usize) -> bool>,
    effect: BuildEffect,
    progress: f32,
    theme: &Theme,
    sx: f32,
    sy: f32,
    dst_buf: &mut [u32],
    w: u32,
    h: u32,
) {
    let t = progress.clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);

    let len = (w * h) as usize;
    let mut scratch = vec![0u32; len];
    let mut canvas = match Canvas::new(&mut scratch, w, h) {
        Some(c) => c,
        None => return,
    };

    let orig_bounds = elem.bounds();
    let scaled_bounds = scale_rect(orig_bounds, sx, sy);

    match effect {
        BuildEffect::Appear => {
            if t > 0.0 {
                render_single_element(elem, None, para_filter, theme, sx, sy, &mut canvas, w, h);
                blend_buffer_over_alpha(&scratch, dst_buf, 1.0, None, w, h);
            }
        }
        BuildEffect::Fade | BuildEffect::DissolveIn => {
            render_single_element(elem, None, para_filter, theme, sx, sy, &mut canvas, w, h);
            blend_buffer_over_alpha(&scratch, dst_buf, ease, None, w, h);
        }
        BuildEffect::FlyIn | BuildEffect::FlyInLeft => {
            let offset_x = (scaled_bounds.right().max(250.0 * sx)) * (1.0 - ease);
            let translated = Rect::new(
                scaled_bounds.x - offset_x,
                scaled_bounds.y,
                scaled_bounds.w,
                scaled_bounds.h,
            );
            render_single_element(elem, Some(translated), para_filter, theme, sx, sy, &mut canvas, w, h);
            blend_buffer_over_alpha(&scratch, dst_buf, ease, None, w, h);
        }
        BuildEffect::FlyInBottom => {
            let offset_y = ((h as f32 - scaled_bounds.y).max(180.0 * sy)) * (1.0 - ease);
            let translated = Rect::new(
                scaled_bounds.x,
                scaled_bounds.y + offset_y,
                scaled_bounds.w,
                scaled_bounds.h,
            );
            render_single_element(elem, Some(translated), para_filter, theme, sx, sy, &mut canvas, w, h);
            blend_buffer_over_alpha(&scratch, dst_buf, ease, None, w, h);
        }
        BuildEffect::Scale | BuildEffect::PopScale => {
            let s = 0.35 + 0.65 * ease;
            let cx = scaled_bounds.x + scaled_bounds.w * 0.5;
            let cy = scaled_bounds.y + scaled_bounds.h * 0.5;
            let new_w = scaled_bounds.w * s;
            let new_h = scaled_bounds.h * s;
            let scaled_rect = Rect::new(cx - new_w * 0.5, cy - new_h * 0.5, new_w, new_h);
            render_single_element(elem, Some(scaled_rect), para_filter, theme, sx * s, sy * s, &mut canvas, w, h);
            blend_buffer_over_alpha(&scratch, dst_buf, ease, None, w, h);
        }
        BuildEffect::Wipe | BuildEffect::WipeRight => {
            render_single_element(elem, None, para_filter, theme, sx, sy, &mut canvas, w, h);
            let clip_x = scaled_bounds.x + scaled_bounds.w * ease;
            blend_buffer_over_alpha(&scratch, dst_buf, 1.0, Some(clip_x), w, h);
        }
        BuildEffect::Disappear | BuildEffect::FadeOut => {
            render_single_element(elem, None, para_filter, theme, sx, sy, &mut canvas, w, h);
            let alpha = if effect == BuildEffect::Disappear && t >= 1.0 {
                0.0
            } else {
                1.0 - ease
            };
            blend_buffer_over_alpha(&scratch, dst_buf, alpha, None, w, h);
        }
    }
}

pub(crate) fn blend_buffer_over_alpha(
    src_buf: &[u32],
    dst_buf: &mut [u32],
    opacity: f32,
    clip_x_max: Option<f32>,
    w: u32,
    _h: u32,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 || w == 0 {
        return;
    }
    let alpha_mul = (opacity * 255.0 + 0.5) as u32;

    let len = src_buf.len().min(dst_buf.len());
    let w_usize = w as usize;

    for i in 0..len {
        let src = src_buf[i];
        let sa = (src >> 24) & 0xFF;
        if sa == 0 {
            continue;
        }
        if let Some(clip_x) = clip_x_max {
            let x = (i % w_usize) as f32;
            if x >= clip_x {
                continue;
            }
        }
        let eff_a = ((sa * alpha_mul + 127) / 255).min(255);
        if eff_a == 0 {
            continue;
        }

        // src is premultiplied in Canvas; scale channels by alpha_mul
        let sr = (((src >> 16) & 0xFF) * alpha_mul + 127) / 255;
        let sg = (((src >> 8) & 0xFF) * alpha_mul + 127) / 255;
        let sb = ((src & 0xFF) * alpha_mul + 127) / 255;

        let dst = dst_buf[i];
        let da = (dst >> 24) & 0xFF;
        let dr = (dst >> 16) & 0xFF;
        let dg = (dst >> 8) & 0xFF;
        let db = dst & 0xFF;

        let inv_a = 255 - eff_a;
        let out_a = (eff_a + (da * inv_a + 127) / 255).min(255);
        let out_r = (sr + (dr * inv_a + 127) / 255).min(255);
        let out_g = (sg + (dg * inv_a + 127) / 255).min(255);
        let out_b = (sb + (db * inv_a + 127) / 255).min(255);

        dst_buf[i] = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
    }
}

pub(crate) fn scale_rect(r: Rect, sx: f32, sy: f32) -> Rect {
    Rect::new(r.x * sx, r.y * sy, r.w * sx, r.h * sy)
}

pub(crate) fn render_text_box(
    tb: &TextBox,
    bounds: Rect,
    theme: &Theme,
    sx: f32,
    sy: f32,
    pixels: &mut [u32],
    target_w: u32,
    target_h: u32,
    para_filter: Option<&dyn Fn(usize) -> bool>,
) {
    let sans_font = match Font::new(GEIST_SANS) {
        Some(f) => f,
        None => return,
    };
    let mono_font = match Font::new(GEIST_MONO) {
        Some(f) => f,
        None => return,
    };

    let mut cursor_y = bounds.y + tb.padding * sy;

    for (p_idx, paragraph) in tb.paragraphs.iter().enumerate() {
        if cursor_y >= bounds.bottom() {
            break;
        }

        let first_fs = paragraph.runs.first().map(|r| r.font_size).unwrap_or(24.0);
        let line_max_h = (first_fs * sy * paragraph.line_spacing).max(24.0 * sy);

        if let Some(ref filter) = para_filter {
            if !filter(p_idx) {
                cursor_y += line_max_h;
                continue;
            }
        }

        let default_color = theme.palette.foreground;
        let indent = (paragraph.bullet_level as f32 * 28.0) * sx;
        let start_x = bounds.x + tb.padding * sx + indent;

        // Render bullet symbol if level > 0
        if paragraph.bullet_level > 0 {
            let bullet_r = 3.5 * sx;
            let bullet_cx = start_x - 14.0 * sx;
            let bullet_cy = cursor_y + 12.0 * sy;
            fill_small_circle(pixels, target_w, target_h, bullet_cx, bullet_cy, bullet_r, default_color);
        }

        // Measure & draw runs
        let mut line_x = start_x;
        let mut line_max_h = 24.0 * sy;

        for run in &paragraph.runs {
            let font = if run.mono { &mono_font } else { &sans_font };
            let run_color = run.color.unwrap_or(default_color);
            let scaled_font_sz = (run.font_size * sy).max(8.0);
            line_max_h = line_max_h.max(scaled_font_sz * paragraph.line_spacing);

            let advance = draw_text_run(
                font,
                &run.text,
                scaled_font_sz,
                line_x,
                cursor_y + scaled_font_sz * 0.85,
                run_color,
                run.bold,
                run.italic,
                pixels,
                target_w,
                target_h,
            );
            line_x += advance;
        }

        cursor_y += line_max_h;
    }
}

pub(crate) fn draw_text_run(
    font: &Font<'_>,
    text: &str,
    font_size_px: f32,
    start_x: f32,
    baseline_y: f32,
    color: Color,
    bold: bool,
    italic: bool,
    target: &mut [u32],
    target_w: u32,
    target_h: u32,
) -> f32 {
    let font_size_px = if font_size_px.is_finite() {
        font_size_px.clamp(4.0, 256.0)
    } else {
        16.0
    };
    let size_arg = (font_size_px * 64.0) as u32;
    let mut cur_x = start_x;
    let mut prev_glyph = None;

    let glyph_box_w = (font_size_px * 1.5).max(12.0) as usize;
    let glyph_box_h = (font_size_px * 1.5).max(12.0) as usize;
    let mut acc = vec![0i32; scratch_len(glyph_box_w, glyph_box_h)];
    let mut out = vec![0u8; glyph_box_w * glyph_box_h];

    let asc_256 = font.ascender(size_arg);

    for ch in text.chars() {
        let scalar = ch as u32;
        let glyph = match font.glyph(scalar) {
            Some(g) => g,
            None => {
                cur_x += font_size_px * 0.5;
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
                if dy < 0 || dy >= target_h as i32 {
                    continue;
                }
                let row_offset = dy as usize * target_w as usize;

                for gx in 0..glyph_box_w {
                    let cov = out[gy * glyph_box_w + gx];
                    if cov > 0 {
                        let shear = if italic { (glyph_box_h as i32 - gy as i32) / 4 } else { 0 };
                        let dx = dst_origin_x + gx as i32 + shear;
                        blend_pixel_coverage(target, row_offset, dx, target_w, color, cov);
                        if bold {
                            blend_pixel_coverage(target, row_offset, dx + 1, target_w, color, cov);
                        }
                    }
                }
            }
        }

        let adv = font.advance(glyph, size_arg);
        cur_x += adv as f32 / 256.0;
    }

    cur_x - start_x
}

#[inline]
fn blend_pixel_coverage(target: &mut [u32], row_offset: usize, dx: i32, target_w: u32, color: Color, cov: u8) {
    if dx >= 0 && dx < target_w as i32 {
        let idx = row_offset + dx as usize;
        let effective_a = ((color.a() as u32 * cov as u32 + 127) / 255) as u8;
        if effective_a > 0 {
            let src_word = color.with_alpha(effective_a).premultiplied();
            let dst_word = target[idx];
            target[idx] = blend_over(src_word, dst_word);
        }
    }
}

#[inline]
fn blend_over(src: u32, dst: u32) -> u32 {
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

fn fill_small_circle(target: &mut [u32], target_w: u32, target_h: u32, cx: f32, cy: f32, r: f32, color: Color) {
    let x0 = (cx - r).max(0.0) as i32;
    let x1 = ((cx + r + 1.0).min(target_w as f32)) as i32;
    let y0 = (cy - r).max(0.0) as i32;
    let y1 = ((cy + r + 1.0).min(target_h as f32)) as i32;
    let r2 = r * r;
    let word = color.premultiplied();

    for y in y0..y1 {
        let dy = y as f32 + 0.5 - cy;
        let row = y as usize * target_w as usize;
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - cx;
            if dx * dx + dy * dy <= r2 {
                target[row + x as usize] = word;
            }
        }
    }
}

fn render_image(
    img: &crate::elements::ImageElement,
    bounds: Rect,
    pixels: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    let Some(ref decoded) = img.decoded else {
        return;
    };
    if decoded.width == 0 || decoded.height == 0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return;
    }

    let dst_x = bounds.x.max(0.0) as u32;
    let dst_y = bounds.y.max(0.0) as u32;
    let dst_w = (bounds.w as u32).min(target_w.saturating_sub(dst_x));
    let dst_h = (bounds.h as u32).min(target_h.saturating_sub(dst_y));

    for ty in 0..dst_h {
        let sy = (ty as usize * decoded.height) / dst_h as usize;
        let py = dst_y + ty;
        if py >= target_h {
            break;
        }
        let target_row = py as usize * target_w as usize;
        let src_row = sy * decoded.width;

        for tx in 0..dst_w {
            let sx = (tx as usize * decoded.width) / dst_w as usize;
            let px = dst_x + tx;
            if px >= target_w {
                break;
            }
            let src_px = Color(decoded.pixels[src_row + sx]).premultiplied();
            let dst_px = pixels[target_row + px as usize];
            pixels[target_row + px as usize] = blend_over(src_px, dst_px);
        }
    }
}

fn render_table(
    t: &crate::elements::TableElement,
    bounds: Rect,
    theme: &Theme,
    sx: f32,
    sy: f32,
    pixels: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    if t.rows == 0 || t.cols == 0 {
        return;
    }
    let col_w = bounds.w / t.cols as f32;
    let row_h = bounds.h / t.rows as f32;

    let font = match Font::new(GEIST_SANS) {
        Some(f) => f,
        None => return,
    };

    // Header background
    if t.has_header_row && t.rows > 0 {
        if let Some(header_bg) = t.header_background {
            let header_rect = Rect::new(bounds.x, bounds.y, bounds.w, row_h);
            fill_solid_rect(pixels, target_w, target_h, header_rect, header_bg);
        }
    }

    // Grid borders
    let border_color = t.border_color.premultiplied();
    for r in 0..=t.rows {
        let y = bounds.y + r as f32 * row_h;
        draw_h_line(pixels, target_w, target_h, bounds.x, bounds.right(), y, border_color);
    }
    for c in 0..=t.cols {
        let x = bounds.x + c as f32 * col_w;
        draw_v_line(pixels, target_w, target_h, bounds.y, bounds.bottom(), x, border_color);
    }

    // Cell text
    for (r, row) in t.cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let cell_x = bounds.x + c as f32 * col_w + 8.0 * sx;
            let cell_y = bounds.y + r as f32 * row_h + row_h * 0.7;
            let cell_color = cell.text_color.unwrap_or(if r == 0 && t.has_header_row {
                Color::WHITE
            } else {
                theme.palette.foreground
            });

            draw_text_run(
                &font,
                &cell.text,
                (20.0 * sy).max(8.0),
                cell_x,
                cell_y,
                cell_color,
                cell.bold || (r == 0 && t.has_header_row),
                false,
                pixels,
                target_w,
                target_h,
            );
        }
    }
}

fn render_code_block(
    c: &crate::elements::CodeBlock,
    bounds: Rect,
    _theme: &Theme,
    sx: f32,
    sy: f32,
    pixels: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    fill_solid_rect(pixels, target_w, target_h, bounds, c.background);

    let font = match Font::new(GEIST_MONO) {
        Some(f) => f,
        None => return,
    };

    let font_sz = (c.font_size * sy).max(8.0);
    let line_h = font_sz * 1.4;
    let mut cur_y = bounds.y + 12.0 * sy + font_sz * 0.85;

    for (line_idx, line) in c.code.lines().enumerate() {
        if cur_y >= bounds.bottom() - 8.0 {
            break;
        }

        let mut start_x = bounds.x + 16.0 * sx;
        if c.show_line_numbers {
            let num_str = alloc::format!("{:2} ", line_idx + 1);
            let num_w = draw_text_run(
                &font,
                &num_str,
                font_sz,
                start_x,
                cur_y,
                Color::rgb(0x64, 0x74, 0x8B),
                false,
                false,
                pixels,
                target_w,
                target_h,
            );
            start_x += num_w + 8.0 * sx;
        }

        draw_text_run(
            &font,
            line,
            font_sz,
            start_x,
            cur_y,
            c.text_color,
            false,
            false,
            pixels,
            target_w,
            target_h,
        );

        cur_y += line_h;
    }
}

fn fill_solid_rect(target: &mut [u32], target_w: u32, target_h: u32, r: Rect, color: Color) {
    let x0 = r.x.max(0.0) as u32;
    let y0 = r.y.max(0.0) as u32;
    let x1 = (r.right().min(target_w as f32)) as u32;
    let y1 = (r.bottom().min(target_h as f32)) as u32;
    let word = color.premultiplied();

    for y in y0..y1 {
        let row = y as usize * target_w as usize;
        for x in x0..x1 {
            target[row + x as usize] = word;
        }
    }
}

fn draw_h_line(target: &mut [u32], target_w: u32, target_h: u32, x0: f32, x1: f32, y: f32, color_word: u32) {
    let iy = y as i32;
    if iy >= 0 && iy < target_h as i32 {
        let left = (x0.max(0.0) as usize).min(target_w as usize);
        let right = (x1.max(0.0) as usize).min(target_w as usize);
        let row = iy as usize * target_w as usize;
        for x in left..right {
            target[row + x] = color_word;
        }
    }
}

fn draw_v_line(target: &mut [u32], target_w: u32, target_h: u32, y0: f32, y1: f32, x: f32, color_word: u32) {
    let ix = x as i32;
    if ix >= 0 && ix < target_w as i32 {
        let top = (y0.max(0.0) as usize).min(target_h as usize);
        let bottom = (y1.max(0.0) as usize).min(target_h as usize);
        for y in top..bottom {
            target[y * target_w as usize + ix as usize] = color_word;
        }
    }
}

pub(crate) fn render_master_chrome(
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    slide_id: u64,
    target_w: u32,
    target_h: u32,
    sx: f32,
    sy: f32,
) {
    let w = target_w as f32;
    let h = target_h as f32;
    let ribbon_h = (4.0 * sy).max(2.0);

    // Top accent ribbon (65% primary accent, 35% secondary accent)
    let p_w = w * 0.65;
    canvas.fill_rect(Rect::new(0.0, 0.0, p_w, ribbon_h), theme.palette.primary_accent);
    canvas.fill_rect(Rect::new(p_w, 0.0, w - p_w, ribbon_h), theme.palette.secondary_accent);

    // Slide number at bottom-right
    if let Some(font) = Font::new(GEIST_SANS) {
        let num_str = alloc::format!("{:02}", slide_id);
        let font_sz = (14.0 * sy).max(9.0);
        let num_w = font_sz * 1.5;
        let num_x = w - (24.0 * sx) - num_w;
        let num_y = h - (16.0 * sy);
        draw_text_run(
            &font,
            &num_str,
            font_sz,
            num_x,
            num_y,
            theme.palette.muted,
            false,
            false,
            canvas.pixels_mut(),
            target_w,
            target_h,
        );
    }
}

/// Interpolate a single transition frame from `from` buffer to `to` buffer at progress `t` (0.0..=1.0).
pub fn render_transition_frame(
    from: &[u32],
    to: &[u32],
    out: &mut [u32],
    width: u32,
    height: u32,
    transition: Transition,
    t: f32,
) {
    let len = match (width as usize).checked_mul(height as usize) {
        Some(l) => l,
        None => return,
    };
    if width == 0 || height == 0 || from.len() < len || to.len() < len || out.len() < len {
        return;
    }

    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let w = width as usize;
    let h = height as usize;

    match transition {
        Transition::None => {
            if t < 1.0 {
                out[..len].copy_from_slice(&from[..len]);
            } else {
                out[..len].copy_from_slice(&to[..len]);
            }
        }
        Transition::Dissolve | Transition::Fade | Transition::MagicMove | Transition::Morph => {
            let t_u32 = (t * 256.0).min(256.0) as u32;
            let inv_u32 = 256 - t_u32;

            for i in 0..len {
                let f = from[i];
                let d = to[i];

                let fa = (f >> 24) & 0xFF;
                let fr = (f >> 16) & 0xFF;
                let fg = (f >> 8) & 0xFF;
                let fb = f & 0xFF;

                let da = (d >> 24) & 0xFF;
                let dr = (d >> 16) & 0xFF;
                let dg = (d >> 8) & 0xFF;
                let db = d & 0xFF;

                let a = ((fa * inv_u32 + da * t_u32) >> 8).min(255);
                let r = ((fr * inv_u32 + dr * t_u32) >> 8).min(255);
                let g = ((fg * inv_u32 + dg * t_u32) >> 8).min(255);
                let b = ((fb * inv_u32 + db * t_u32) >> 8).min(255);

                out[i] = (a << 24) | (r << 16) | (g << 8) | b;
            }
        }
        Transition::PushLeft | Transition::SlideLeft => {
            let shift_x = (((width as f32) * t) + 0.5) as usize;
            let shift_x = shift_x.min(w);

            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    if x + shift_x < w {
                        out[row + x] = from[row + x + shift_x];
                    } else {
                        out[row + x] = to[row + (x + shift_x - w)];
                    }
                }
            }
        }
        Transition::PushRight => {
            let shift_x = (((width as f32) * t) + 0.5) as usize;
            let shift_x = shift_x.min(w);

            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    if x < shift_x {
                        out[row + x] = to[row + (w - shift_x + x)];
                    } else {
                        out[row + x] = from[row + (x - shift_x)];
                    }
                }
            }
        }
        Transition::WipeRight => {
            let split_x = (((width as f32) * t) + 0.5) as usize;
            let split_x = split_x.min(w);

            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    if x < split_x {
                        out[row + x] = to[row + x];
                    } else {
                        out[row + x] = from[row + x];
                    }
                }
            }
        }
        Transition::SlideUp => {
            let shift_y = (((height as f32) * t) + 0.5) as usize;
            let shift_y = shift_y.min(h);

            for y in 0..h {
                let out_row = y * w;
                if y + shift_y < h {
                    let from_row = (y + shift_y) * w;
                    out[out_row..out_row + w].copy_from_slice(&from[from_row..from_row + w]);
                } else {
                    let to_row = (y + shift_y - h) * w;
                    out[out_row..out_row + w].copy_from_slice(&to[to_row..to_row + w]);
                }
            }
        }
        Transition::ZoomIn => {
            let scale = 0.2 + 0.8 * t;
            let sw = (((w as f32 * scale) + 0.5) as usize).max(1);
            let sh = (((h as f32 * scale) + 0.5) as usize).max(1);
            let ox = (w.saturating_sub(sw)) / 2;
            let oy = (h.saturating_sub(sh)) / 2;

            let t_u32 = (t * 256.0).min(256.0) as u32;
            let inv_u32 = 256 - t_u32;

            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    let f = from[row + x];
                    if x >= ox && x < ox + sw && y >= oy && y < oy + sh {
                        let sx = ((x - ox) * w) / sw;
                        let sy = ((y - oy) * h) / sh;
                        let d = to[sy.min(h - 1) * w + sx.min(w - 1)];

                        let fa = (f >> 24) & 0xFF;
                        let fr = (f >> 16) & 0xFF;
                        let fg = (f >> 8) & 0xFF;
                        let fb = f & 0xFF;

                        let da = (d >> 24) & 0xFF;
                        let dr = (d >> 16) & 0xFF;
                        let dg = (d >> 8) & 0xFF;
                        let db = d & 0xFF;

                        let a = ((fa * inv_u32 + da * t_u32) >> 8).min(255);
                        let r = ((fr * inv_u32 + dr * t_u32) >> 8).min(255);
                        let g = ((fg * inv_u32 + dg * t_u32) >> 8).min(255);
                        let b = ((fb * inv_u32 + db * t_u32) >> 8).min(255);

                        out[row + x] = (a << 24) | (r << 16) | (g << 8) | b;
                    } else {
                        out[row + x] = f;
                    }
                }
            }
        }
    }
}

/// Canvas-oriented transition helper.
pub fn render_transition_canvas<'a>(
    from: &Canvas<'_>,
    to: &Canvas<'_>,
    out: &'a mut [u32],
    transition: Transition,
    t: f32,
) -> Option<Canvas<'a>> {
    let w = from.width();
    let h = from.height();
    if to.width() != w || to.height() != h {
        return None;
    }
    render_transition_frame(from.pixels(), to.pixels(), out, w, h, transition, t);
    Canvas::new(out, w, h)
}

/// Render an animated slide transition between two slides into target buffer.
pub fn render_deck_transition(
    from_slide: &Slide,
    to_slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    transition: Transition,
    t: f32,
    out: &mut [u32],
    width: u32,
    height: u32,
) {
    if transition == Transition::MagicMove || transition == Transition::Morph {
        crate::morph::render_morph_frame(from_slide, to_slide, theme, aspect_ratio, t, out, width, height);
        return;
    }
    let len = match (width as usize).checked_mul(height as usize) {
        Some(l) => l,
        None => return,
    };
    if width == 0 || height == 0 || out.len() < len {
        return;
    }

    let mut buf_from = vec![0u32; len];
    let mut buf_to = vec![0u32; len];
    render_slide(from_slide, theme, aspect_ratio, &mut buf_from, width, height);
    render_slide(to_slide, theme, aspect_ratio, &mut buf_to, width, height);
    render_transition_frame(&buf_from, &buf_to, out, width, height, transition, t);
}

/// Render a Keynote-grade Magic Move frame with object interpolation.
pub fn render_magic_move_frame(
    from_slide: &Slide,
    to_slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    progress: f32,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    crate::morph::render_morph_frame(
        from_slide,
        to_slide,
        theme,
        aspect_ratio,
        progress,
        buffer,
        target_w,
        target_h,
    );
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(crate) fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let r = ((a.r() as f32) + ((b.r() as f32) - (a.r() as f32)) * t + 0.5) as u8;
    let g = ((a.g() as f32) + ((b.g() as f32) - (a.g() as f32)) * t + 0.5) as u8;
    let bl = ((a.b() as f32) + ((b.b() as f32) - (a.b() as f32)) * t + 0.5) as u8;
    let al = ((a.a() as f32) + ((b.a() as f32) - (a.a() as f32)) * t + 0.5) as u8;
    Color::rgba(r, g, bl, al)
}

pub(crate) fn lerp_color_opt(a: Option<Color>, b: Option<Color>, t: f32) -> Option<Color> {
    match (a, b) {
        (Some(ca), Some(cb)) => Some(lerp_color(ca, cb, t)),
        (Some(ca), None) => {
            let new_a = ((ca.a() as f32) * (1.0 - t) + 0.5) as u8;
            Some(ca.with_alpha(new_a))
        }
        (None, Some(cb)) => {
            let new_a = ((cb.a() as f32) * t + 0.5) as u8;
            Some(cb.with_alpha(new_a))
        }
        (None, None) => None,
    }
}

pub(crate) fn lerp_stroke_opt(a: Option<StrokeStyle>, b: Option<StrokeStyle>, t: f32) -> Option<StrokeStyle> {
    match (a, b) {
        (Some(sa), Some(sb)) => Some(StrokeStyle {
            color: lerp_color(sa.color, sb.color, t),
            width: lerp(sa.width, sb.width, t),
        }),
        (Some(sa), None) => {
            let new_a = ((sa.color.a() as f32) * (1.0 - t) + 0.5) as u8;
            Some(StrokeStyle {
                color: sa.color.with_alpha(new_a),
                width: sa.width,
            })
        }
        (None, Some(sb)) => {
            let new_a = ((sb.color.a() as f32) * t + 0.5) as u8;
            Some(StrokeStyle {
                color: sb.color.with_alpha(new_a),
                width: sb.width,
            })
        }
        (None, None) => None,
    }
}

