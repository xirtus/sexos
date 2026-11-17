// SPDX-License-Identifier: GPL-3.0-only
//! Pure `no_std` software broadcast frame, Multiview, DSK, Vertical 9:16,
//! wide-gamut `sex-color` scope, and audio visualizer renderer for Cabaret.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use cabaret_core::compositor::{
    blend_pixel, BlendMode, Compositor, CompositorLayer, FrameBuffer, LinearRgba, MixColorSpace,
    Scaler, Transform2D,
};
use cabaret_core::scene::{render_studio_frame, SourceId, StudioId};
use cabaret_core::transitions::{render_transition_frame, MultiviewRole, TransitionKind};
use cabaret_plugins::local_captions::{render_lower_third_captions, CaptionBurnInStyle};
use cabaret_plugins::move_transition::MoveTransitionPlan;
use cabaret_plugins::shader_filter::apply_shader_preset;
use cabaret_plugins::spectralizer::{
    render_bar_spectrum, render_goniometer, BarSpectrumConfig, GoniometerConfig,
};
use cabaret_plugins::streamfx_3d::{apply_perspective_3d, warp_corner_pin_normalized};
use sex_color::{
    convert_rgb, tone_map_rgb, ColorPrimaries, ColorSpace, LinearFrame, RgbaF32, ScopeSurface,
};

use crate::studio::{ActiveTransitionPreset, CabaretStudio, ScopeKind};

/// Fast UI preview dimensions for 16:9 Program/Preview canvases.
pub const UI_PREVIEW_W: u32 = 320;
pub const UI_PREVIEW_H: u32 = 180;

/// Fast UI preview dimensions for 9:16 Vertical Shorts/TikTok canvas.
pub const UI_VERTICAL_W: u32 = 135;
pub const UI_VERTICAL_H: u32 = 240;

#[inline]
fn srgb_encode_u8(v: f32) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let enc = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * libm::powf(v, 1.0 / 2.4) - 0.055
    };
    (enc * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// Converts a linear [`FrameBuffer`] directly into Weft's `0xFFRRGGBB` pixel array.
pub fn fb_to_weft_argb(fb: &FrameBuffer) -> Vec<u32> {
    let mut out = Vec::with_capacity(fb.pixels.len());
    for &p in &fb.pixels {
        let a = if p[3].is_nan() { 0.0 } else { p[3].clamp(0.0, 1.0) };
        let r = srgb_encode_u8(p[0] * a);
        let g = srgb_encode_u8(p[1] * a);
        let b = srgb_encode_u8(p[2] * a);
        out.push(0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
    }
    out
}

/// Resizes/pads an arbitrary `0xAARRGGBB` pixel slice into exact `(dst_w, dst_h)` words.
pub fn argb_fit(src: &[u32], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u32> {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 || src.is_empty() {
        return vec![0xFF0E_1014; dst_w * dst_h];
    }
    if src_w == dst_w && src_h == dst_h && src.len() >= src_w * src_h {
        return src[..src_w * src_h].to_vec();
    }
    let mut out = vec![0xFF0E_1014; dst_w * dst_h];
    for y in 0..dst_h {
        let sy = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let sx = (x * src_w) / dst_w;
            out[y * dst_w + x] = src[sy * src_w + sx] | 0xFF00_0000;
        }
    }
    out
}

/// Synthesizes deterministic, high-contrast test/live feeds for all seeded sources.
pub fn build_raw_sources(
    studio: &mut CabaretStudio,
    width: u32,
    height: u32,
) -> BTreeMap<SourceId, FrameBuffer> {
    let mut map = BTreeMap::new();
    let w = width.max(32);
    let h = height.max(32);
    let phase = (studio.frame_counter % 120) as f32 / 120.0;

    // 1. UVC Camera (warm keylight + geometric stage grid + motion indicator)
    if let Ok(mut cam) = FrameBuffer::new(w, h) {
        let wf = w as f32;
        let hf = h as f32;
        for y in 0..h {
            let v = y as f32 / hf;
            for x in 0..w {
                let u = x as f32 / wf;
                let dx = u - 0.5;
                let dy = v - 0.42;
                let dist = libm::sqrtf(dx * dx + dy * dy);
                let halo = (1.0 - dist * 1.6).max(0.0);
                let skin = if dist < 0.22 { 0.42 } else { 0.0 };
                let r = (0.14 + 0.46 * halo + skin * 0.55).clamp(0.0, 1.0);
                let g = (0.10 + 0.26 * halo + skin * 0.30).clamp(0.0, 1.0);
                let b = (0.22 + 0.48 * (1.0 - v) + skin * 0.16).clamp(0.0, 1.0);
                cam.set_pixel(x, y, [r, g, b, 1.0]);
            }
        }
        // Moving vertical sync bar so Replay / Freeze / TimeWarp changes are observable frame-to-frame
        let bar_x = ((phase * wf) as i32).clamp(2, w as i32 - 6);
        fill_rect_fb(&mut cam, bar_x, 6, 4, (h as i32) - 12, [0.95, 0.28, 0.55, 0.90]);
        draw_mini_text(&mut cam, 8, 8, "UVC 1080P60", [0.96, 0.96, 1.0, 1.0]);

        // Pre-composite Source Record tap + Freeze filter on the clean camera feed
        if studio.source_tap.recording.state
            == cabaret_core::outputs::RecordingState::Recording
        {
            let pts_us = (studio.clock_ms as i64) * 1000;
            let _ = studio.source_tap.tap_video_frame(
                pts_us,
                &cam,
                studio.frame_counter % 30 == 0,
                studio.available_disk_bytes,
            );
        }
        let cam_frozen = studio.freeze_filter.process(&cam, 16.666);
        map.insert(studio.sources.uvc_camera, cam_frozen);
    }

    // 2. Silk Window (focused desktop editor/terminal capture frame)
    if let Ok(mut silk) = FrameBuffer::filled(w, h, [0.06, 0.08, 0.11, 1.0]) {
        fill_rect_fb(&mut silk, 0, 0, w as i32, 18, [0.14, 0.17, 0.24, 1.0]);
        stroke_rect_fb(&mut silk, 0, 0, w as i32, h as i32, 1, [0.28, 0.36, 0.52, 1.0]);
        draw_mini_text(
            &mut silk,
            6,
            6,
            &studio.focused_silk_window,
            [0.82, 0.92, 1.0, 1.0],
        );
        // Code/terminal accent lines
        for line in 0..5 {
            let ly = 28 + line * 16;
            let lw = 60 + ((line * 37 + (studio.frame_counter as i32 * 3)) % (w as i32 - 80)).abs();
            let col = if line % 2 == 0 {
                [0.20, 0.65, 0.88, 0.85]
            } else {
                [0.40, 0.82, 0.55, 0.80]
            };
            fill_rect_fb(&mut silk, 12, ly, lw.min(w as i32 - 24), 7, col);
        }
        map.insert(studio.sources.silk_window, silk);
    }

    // 3. Burlesque Live Presentation Deck (actual rendered slide buffer)
    let mut slide_rgba = vec![0u8; (w * h * 4) as usize];
    if studio.presentation.render_rgba_into(&mut slide_rgba, w, h) {
        if let Ok(slide_fb) = FrameBuffer::from_srgb_rgba8(w, h, &slide_rgba) {
            map.insert(studio.sources.presentation_deck, slide_fb);
        }
    }

    // 4. Kaleidoscope Browser Alert Overlay (transparent pill)
    if let Ok(mut alerts) = FrameBuffer::new(w, h) {
        fill_rect_fb(&mut alerts, 8, 8, (w as i32 - 16).max(32), 46, [0.08, 0.03, 0.14, 0.88]);
        stroke_rect_fb(&mut alerts, 8, 8, (w as i32 - 16).max(32), 46, 2, [0.95, 0.25, 0.65, 0.95]);
        draw_mini_text(&mut alerts, 16, 18, "ALERT OVERLAY", [1.0, 0.85, 0.35, 1.0]);
        draw_mini_text(&mut alerts, 16, 32, "OBSSTUDIO BRIDGE OK", [0.75, 0.95, 1.0, 1.0]);
        map.insert(studio.sources.browser_alerts, alerts);
    }

    // 5. Phone SRT Camera
    if let Ok(mut phone) = FrameBuffer::filled(w, h, [0.09, 0.06, 0.12, 1.0]) {
        stroke_rect_fb(&mut phone, 2, 2, w as i32 - 4, h as i32 - 4, 2, [0.35, 0.85, 0.95, 1.0]);
        draw_mini_text(&mut phone, 10, 12, "SRT PHONE 9000", [0.45, 0.95, 1.0, 1.0]);
        fill_rect_fb(&mut phone, 16, 36, 48, 36, [0.85, 0.30, 0.45, 0.9]);
        map.insert(studio.sources.phone_srt, phone);
    }

    // 6. Now Playing Card (`overlays::NowPlayingCard`)
    if let Ok(np) = studio.now_playing.render() {
        map.insert(studio.sources.now_playing_card, np);
    }

    // 7. Input Overlay HUD (`overlays::InputOverlayHud`)
    if let Ok(hud) = studio.input_overlay.render() {
        map.insert(studio.sources.input_overlay, hud);
    }

    map
}

fn build_dsk_overlays(studio: &CabaretStudio, w: u32, h: u32) -> BTreeMap<u64, FrameBuffer> {
    let mut map = BTreeMap::new();

    // Channel 1: Persistent CABARET LIVE Logo Bug (top-right)
    if let Ok(mut bug) = FrameBuffer::new(w, h) {
        let bx = (w as i32 - 86).max(4);
        fill_rect_fb(&mut bug, bx, 6, 80, 18, [0.88, 0.12, 0.44, 0.94]);
        stroke_rect_fb(&mut bug, bx, 6, 80, 18, 1, [1.0, 0.82, 0.92, 1.0]);
        draw_mini_text(&mut bug, bx + 6, 12, "CABARET", [1.0, 1.0, 1.0, 1.0]);
        map.insert(1, bug);
    }

    // Channel 2: Lower-Third chyron above Program
    if let Ok(mut lt) = FrameBuffer::new(w, h) {
        let lw = (w as i32 - 28).max(80);
        let ly = (h as i32 - 34).max(12);
        fill_rect_fb(&mut lt, 14, ly, lw, 24, [0.05, 0.08, 0.16, 0.92]);
        fill_rect_fb(&mut lt, 14, ly, 5, 24, [0.15, 0.85, 0.98, 1.0]);
        let title = studio.presentation.slide_title();
        let txt = if title.is_empty() {
            "DSK LOWER THIRD"
        } else {
            title
        };
        draw_mini_text(&mut lt, 24, ly + 8, txt, [0.98, 0.98, 1.0, 1.0]);
        map.insert(2, lt);
    }

    map
}

fn render_studio_with_sources(
    studio: &mut CabaretStudio,
    studio_id: StudioId,
    w: u32,
    h: u32,
    raw_sources: &BTreeMap<SourceId, FrameBuffer>,
) -> FrameBuffer {
    let ai_empty = BTreeMap::new();
    render_studio_frame(
        &studio.collection,
        studio_id,
        w,
        h,
        raw_sources,
        &mut studio.filter_contexts,
        &ai_empty,
    )
    .unwrap_or_else(|_| FrameBuffer::filled(w, h, [0.04, 0.04, 0.06, 1.0]).expect("fallback fb"))
}

/// Renders Preview bus for UI or inspection.
pub fn render_preview_frame(studio: &mut CabaretStudio, w: u32, h: u32) -> FrameBuffer {
    let raw = build_raw_sources(studio, w, h);
    let preview_id = studio.studio_mode.preview_studio;
    let mut fb = if let Some(ref staged) = studio.studio_mode.staged_preview {
        if staged.id == preview_id {
            let mut tmp_col = studio.collection.clone();
            tmp_col.replace_studio_in_place(staged.clone());
            let ai = BTreeMap::new();
            render_studio_frame(&tmp_col, preview_id, w, h, &raw, &mut studio.filter_contexts, &ai)
                .unwrap_or_else(|_| render_studio_with_sources(studio, preview_id, w, h, &raw))
        } else {
            render_studio_with_sources(studio, preview_id, w, h, &raw)
        }
    } else {
        render_studio_with_sources(studio, preview_id, w, h, &raw)
    };
    stroke_rect_fb(&mut fb, 0, 0, w as i32, h as i32, 2, [0.08, 0.88, 0.30, 1.0]);
    fb
}

/// Renders Program output through transition engine, active video plugins (TimeWarp, ShaderFilter,
/// StreamFX 3D, Captions, CabaretLink), Downstream Keyer bank, and `sex-color` wide-gamut pipeline.
pub fn render_program_frame(studio: &mut CabaretStudio, w: u32, h: u32) -> FrameBuffer {
    let raw = build_raw_sources(studio, w, h);

    // Determine base transition composite
    let mut fb = if let Some(ref running) = studio.studio_mode.running.clone() {
        let t = (running.elapsed_ms / (running.spec.duration_ms.max(1) as f32)).clamp(0.0, 1.0);
        composite_transition_pair(studio, running.from_studio, running.to_studio, &running.spec.kind, t, w, h, &raw)
    } else if studio.studio_mode.t_bar > 0.001 && studio.studio_mode.t_bar < 0.999 {
        let t = studio.studio_mode.t_bar;
        let kind = studio.studio_mode.active_transition.kind.clone();
        let from_id = studio.studio_mode.program_studio;
        let to_id = studio.studio_mode.preview_studio;
        composite_transition_pair(studio, from_id, to_id, &kind, t, w, h, &raw)
    } else {
        let pgm = studio.studio_mode.program_studio;
        render_studio_with_sources(studio, pgm, w, h, &raw)
    };

    // Push clean frame into DynamicDelayBuffer & handle slow-mo / reverse replay if active
    let ts_us = (studio.clock_ms as i64) * 1000;
    studio.delay_buffer.push_frame(ts_us, fb.clone());
    if !studio.delay_buffer.live_follow {
        if let Some(delayed) = studio.delay_buffer.step_and_sample(1.0) {
            fb = delayed;
        }
    }

    // TimeWarpScan slit-scan plugin
    if studio.time_warp_enabled {
        fb = studio.time_warp.process(&fb, 33.3);
    }

    // ShaderFilter preset plugin
    if let Some(preset) = studio.active_shader_preset() {
        let time_s = studio.clock_ms as f32 * 0.001;
        let rms = libm::powf(10.0, (studio.strip_meters(studio.strips.mic_serenade).sample_peak_db / 20.0).clamp(-3.0, 0.0));
        if let Ok(shaded) = apply_shader_preset(&fb, &preset, time_s, rms) {
            fb = shaded;
        }
    }

    // StreamFX 3D Corner-Pin / Perspective warp
    if studio.warp_3d_enabled {
        if studio.corner_pin != cabaret_plugins::streamfx_3d::CornerPinQuad::unit() {
            if let Ok(warped) = warp_corner_pin_normalized(&fb, &studio.corner_pin, w, h) {
                fb = warped;
            }
        } else if let Ok(p3d) = apply_perspective_3d(&fb, &studio.perspective_3d, w, h) {
            fb = p3d;
        }
    }

    // Burned-in Local Captions lower third
    if studio.burn_in_captions {
        let lines = studio.captions.current_lines().to_vec();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        if !refs.is_empty() {
            let style = CaptionBurnInStyle {
                font_scale: 1,
                margin_bottom_px: 8,
                padding_x: 10,
                padding_y: 6,
                ..CaptionBurnInStyle::default()
            };
            render_lower_third_captions(&mut fb, &refs, &style);
        }
    }

    // Downstream Keyer Bank pinned ABOVE studio transitions!
    let dsk_frames = build_dsk_overlays(studio, w, h);
    fb = studio.dsk.composite_overlays(&fb, &dsk_frames);

    // Wide-gamut sex-color LUT / HDR tone-mapping pass
    apply_color_pipeline(&mut fb, &studio.color_pipeline);

    // Update CabaretLink LAN telemetry size
    let _ = studio.emit_cabaret_link_packet(&fb);

    fb
}

fn composite_transition_pair(
    studio: &mut CabaretStudio,
    from_id: StudioId,
    to_id: StudioId,
    kind: &TransitionKind,
    t: f32,
    w: u32,
    h: u32,
    raw: &BTreeMap<SourceId, FrameBuffer>,
) -> FrameBuffer {
    if studio.transition_preset == ActiveTransitionPreset::Move {
        if let (Some(from_st), Some(to_st)) = (
            studio.collection.studio(from_id).cloned(),
            studio.collection.studio(to_id).cloned(),
        ) {
            let plan = MoveTransitionPlan::build(&from_st, &to_st, studio.move_config.clone());
            if let Ok(moved) = plan.render(
                t,
                w,
                h,
                &studio.collection,
                raw,
                &mut studio.filter_contexts,
            ) {
                return moved;
            }
        }
    }

    let a = render_studio_with_sources(studio, from_id, w, h, raw);
    let b = render_studio_with_sources(studio, to_id, w, h, raw);

    let stinger_frame = if matches!(kind, TransitionKind::Stinger { .. }) {
        build_stinger_overlay_frame(w, h, t)
    } else {
        None
    };
    render_transition_frame(&a, &b, kind, t, stinger_frame.as_ref(), None)
}

fn build_stinger_overlay_frame(w: u32, h: u32, t: f32) -> Option<FrameBuffer> {
    let mut fb = FrameBuffer::new(w, h).ok()?;
    let sweep = ((t * 1.4 - 0.2).clamp(0.0, 1.0) * (w as f32)) as i32;
    let band_w = (w as i32 / 5).max(16);
    fill_rect_fb(
        &mut fb,
        sweep - band_w / 2,
        0,
        band_w,
        h as i32,
        [0.95, 0.20, 0.55, 0.92],
    );
    Some(fb)
}

/// Converts a `FrameBuffer` into `sex_color::LinearFrame`, applying active FilmStock LUT and
/// HDR/wide-gamut `ToneMapConfig` + gamut transform when configured.
pub fn framebuffer_to_linear_color(
    fb: &FrameBuffer,
    pipeline: &crate::studio::CanvasColorPipeline,
) -> LinearFrame {
    let primaries = pipeline
        .canvas_space
        .primaries()
        .unwrap_or(ColorPrimaries::Rec709);
    let mut lf = LinearFrame {
        width: fb.width as usize,
        height: fb.height as usize,
        primaries,
        pixels: Vec::with_capacity(fb.pixels.len()),
    };
    for &p in &fb.pixels {
        let lin_rgb = if primaries == ColorPrimaries::Rec2020 {
            convert_rgb([p[0], p[1], p[2]], ColorSpace::LinearRec709, ColorSpace::LinearRec2020)
        } else {
            [p[0], p[1], p[2]]
        };
        let mapped = if pipeline.canvas_space.is_hdr() {
            tone_map_rgb(lin_rgb, pipeline.tone_map)
        } else {
            lin_rgb
        };
        lf.pixels.push(RgbaF32::from_rgb(mapped, p[3]));
    }
    if let Some(stock) = pipeline.film_stock {
        stock.apply_frame(&mut lf);
    }
    lf
}

fn apply_color_pipeline(fb: &mut FrameBuffer, pipeline: &crate::studio::CanvasColorPipeline) {
    if pipeline.film_stock.is_none() && pipeline.canvas_space == ColorSpace::Rec709 {
        return;
    }
    let lf = framebuffer_to_linear_color(fb, pipeline);
    for (dst, src) in fb.pixels.iter_mut().zip(lf.pixels.iter()) {
        let back = if lf.primaries == ColorPrimaries::Rec2020 {
            convert_rgb(src.rgb(), ColorSpace::LinearRec2020, ColorSpace::LinearRec709)
        } else {
            src.rgb()
        };
        *dst = [back[0], back[1], back[2], src.0[3]];
    }
}

/// Renders the live Multiview composite with tally borders and 90%/80% safe-area guides.
pub fn render_multiview_frame(studio: &mut CabaretStudio, w: u32, h: u32) -> FrameBuffer {
    let mv = studio.multiview_layout(w as f32, h as f32);
    let mut canvas = FrameBuffer::filled(w, h, [0.03, 0.03, 0.05, 1.0]).expect("mv fb");
    let compositor = Compositor::new([0.03, 0.03, 0.05, 1.0]);

    let raw = build_raw_sources(studio, 160, 90);
    let mut studio_thumbs: BTreeMap<StudioId, FrameBuffer> = BTreeMap::new();
    let studio_ids: Vec<StudioId> = studio.collection.studios.iter().map(|s| s.id).collect();
    for sid in studio_ids {
        let full = render_studio_with_sources(studio, sid, 320, 180, &raw);
        let scaled = cabaret_core::compositor::scale_frame(&full, 160, 90, Scaler::Bilinear)
            .unwrap_or(full);
        studio_thumbs.insert(sid, scaled);
    }

    for slot in &mv.slots {
        let rx = slot.rect.x as i32;
        let ry = slot.rect.y as i32;
        let rw = (slot.rect.w as i32).max(4);
        let rh = (slot.rect.h as i32).max(4);

        fill_rect_fb(&mut canvas, rx, ry, rw, rh, [0.06, 0.07, 0.10, 1.0]);
        if let Some(sid) = slot.studio_id {
            if let Some(thumb) = studio_thumbs.get(&sid) {
                let sx = (rw as f32) / (thumb.width as f32).max(1.0);
                let sy = (rh as f32) / (thumb.height as f32).max(1.0);
                let tr = Transform2D {
                    pos: [slot.rect.x, slot.rect.y],
                    scale: [sx, sy],
                    ..Transform2D::default()
                };
                compositor.draw_layer(
                    &mut canvas,
                    &CompositorLayer {
                        frame: thumb,
                        transform: tr,
                        blend_mode: BlendMode::Normal,
                        opacity: 1.0,
                        mix_space: MixColorSpace::Linear,
                        scaler: Scaler::Bilinear,
                        visible: true,
                    },
                );
            }
        }

        // Safe-area overlays for Preview/Program slots
        if mv.show_safe_areas
            && matches!(slot.role, MultiviewRole::Preview | MultiviewRole::Program)
        {
            let a = slot.safe_areas.action_safe_90;
            let t = slot.safe_areas.title_safe_80;
            stroke_rect_fb(
                &mut canvas,
                a.x as i32,
                a.y as i32,
                a.w as i32,
                a.h as i32,
                1,
                [0.9, 0.9, 0.2, 0.55],
            );
            stroke_rect_fb(
                &mut canvas,
                t.x as i32,
                t.y as i32,
                t.w as i32,
                t.h as i32,
                1,
                [0.2, 0.9, 0.9, 0.55],
            );
        }

        // Tally border
        stroke_rect_fb(&mut canvas, rx, ry, rw, rh, 2, slot.border_color);
        let text = match slot.studio_id {
            Some(sid) => format!("{}: {}", slot.label, studio.studio_name(sid)),
            None => slot.label.clone(),
        };
        draw_mini_text(&mut canvas, rx + 4, ry + 4, &text, [1.0, 1.0, 1.0, 0.95]);
    }

    canvas
}

/// Renders fast scaled preview of the `9:16` Vertical Shorts/TikTok secondary canvas.
pub fn render_vertical_preview(studio: &mut CabaretStudio, w: u32, h: u32) -> FrameBuffer {
    let raw = build_raw_sources(studio, w, (w * 9) / 16);
    let mut canvas = FrameBuffer::filled(w, h, [0.03, 0.03, 0.06, 1.0]).expect("vert fb");
    let compositor = Compositor::new([0.03, 0.03, 0.06, 1.0]);

    if let Some(sec) = studio.secondary_canvases.canvas(studio.vertical_canvas_id) {
        let sx = (w as f32) / (sec.width as f32).max(1.0);
        let sy = (h as f32) / (sec.height as f32).max(1.0);
        for item in sec.active_items() {
            if !item.visible || item.opacity <= 0.0 {
                continue;
            }
            if let Some(src_fb) = raw.get(&item.source_id) {
                let mut tr = item.transform;
                tr.pos = [tr.pos[0] * sx, tr.pos[1] * sy];
                tr.scale = [
                    (w as f32 / (src_fb.width as f32).max(1.0)) * item.transform.scale[0],
                    (h as f32 / (src_fb.height as f32).max(1.0)) * item.transform.scale[1],
                ];
                compositor.draw_layer(
                    &mut canvas,
                    &CompositorLayer {
                        frame: src_fb,
                        transform: tr,
                        blend_mode: item.blend_mode,
                        opacity: item.opacity,
                        mix_space: item.opacity_space,
                        scaler: Scaler::Bilinear,
                        visible: true,
                    },
                );
            }
        }
    }
    stroke_rect_fb(&mut canvas, 0, 0, w as i32, h as i32, 2, [0.95, 0.25, 0.55, 1.0]);
    draw_mini_text(&mut canvas, 6, 8, "9:16 VERTICAL SHORTS", [1.0, 0.9, 0.95, 1.0]);
    canvas
}

/// Renders native-resolution secondary canvases using `SecondaryCanvasManager::render_canvas`.
pub fn render_secondary_canvas_native(
    studio: &mut CabaretStudio,
) -> Option<FrameBuffer> {
    let id = studio.vertical_canvas_id;
    let raw = build_raw_sources(studio, 320, 180);
    let ai = BTreeMap::new();
    studio
        .secondary_canvases
        .render_canvas(id, &studio.collection, &raw, &mut studio.filter_contexts, &ai)
        .ok()
}

/// Renders live `sex-color` broadcast scope (`Waveform`, `RgbParade`, `Vectorscope`, `Histogram`).
pub fn render_scope_argb(studio: &CabaretStudio, program_fb: &FrameBuffer, w: u32, h: u32) -> Vec<u32> {
    let lf = framebuffer_to_linear_color(program_fb, &studio.color_pipeline);
    let mut surface = ScopeSurface::new(w as usize, h as usize);
    let std_mode = studio.color_pipeline.scope_standard;
    match studio.color_pipeline.scope_kind {
        ScopeKind::Waveform => {
            surface.render_waveform(&lf, studio.color_pipeline.waveform_mode, std_mode);
        }
        ScopeKind::RgbParade => {
            surface.render_rgb_parade(&lf, std_mode);
        }
        ScopeKind::Vectorscope => {
            surface.render_vectorscope(&lf, std_mode);
        }
        ScopeKind::Histogram => {
            let _ = surface.render_histogram(&lf, std_mode);
        }
    }
    surface.bgra
}

/// Renders combined Audio Bar Spectrum + Stereo Correlation readout onto a [`FrameBuffer`].
pub fn render_audio_visualizer_fb(studio: &mut CabaretStudio, w: u32, h: u32) -> FrameBuffer {
    let bar_w = (w * 2 / 3).max(64);
    let gonio_w = w.saturating_sub(bar_w).max(48);
    let mut out = FrameBuffer::filled(w, h, [0.02, 0.02, 0.04, 1.0]).expect("vis fb");

    let cfg = BarSpectrumConfig {
        width: bar_w,
        height: h,
        stereo_mirror: false,
        ..BarSpectrumConfig::default()
    };
    if let Ok(bars) = render_bar_spectrum(&studio.spectrum, &cfg) {
        blit_fb(&mut out, &bars, 0, 0);
    }

    let empty_samples: Vec<[f32; 2]> = Vec::new();
    let samples: &[[f32; 2]] = studio
        .last_mixed_audio
        .as_ref()
        .map(|m| m.tracks[0].as_slice())
        .unwrap_or(&empty_samples);
    let sub = &samples[..samples.len().min(256)];
    let g_cfg = GoniometerConfig {
        width: gonio_w,
        height: h,
        ..GoniometerConfig::default()
    };
    if let Ok(gonio) = render_goniometer(sub, &g_cfg, studio.last_goniometer_frame.as_ref()) {
        blit_fb(&mut out, &gonio, bar_w, 0);
        studio.last_goniometer_frame = Some(gonio);
    }
    let corr_label = format!("CORR:{:+.2}", studio.stereo_correlation);
    draw_mini_text(&mut out, (bar_w + 4) as i32, 4, &corr_label, [0.85, 1.0, 0.85, 1.0]);
    out
}

fn blit_fb(dst: &mut FrameBuffer, src: &FrameBuffer, ox: u32, oy: u32) {
    for y in 0..src.height {
        if oy + y >= dst.height {
            break;
        }
        for x in 0..src.width {
            if ox + x >= dst.width {
                break;
            }
            if let Some(p) = src.get_pixel(x, y) {
                dst.set_pixel(ox + x, oy + y, p);
            }
        }
    }
}

fn fill_rect_fb(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, color: LinearRgba) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x + w).min(fb.width as i32)).max(0) as u32;
    let y1 = ((y + h).min(fb.height as i32)).max(0) as u32;
    for py in y0..y1 {
        for px in x0..x1 {
            if let Some(dst) = fb.get_pixel(px, py) {
                let blended = blend_pixel(dst, color, BlendMode::Normal, 1.0, MixColorSpace::Linear);
                fb.set_pixel(px, py, blended);
            }
        }
    }
}

fn stroke_rect_fb(
    fb: &mut FrameBuffer,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    t: i32,
    color: LinearRgba,
) {
    let t = t.max(1);
    fill_rect_fb(fb, x, y, w, t, color);
    fill_rect_fb(fb, x, y + h - t, w, t, color);
    fill_rect_fb(fb, x, y + t, t, (h - 2 * t).max(0), color);
    fill_rect_fb(fb, x + w - t, y + t, t, (h - 2 * t).max(0), color);
}

/// Draws compact crisp 3x5 ASCII markers onto a [`FrameBuffer`].
fn draw_mini_text(fb: &mut FrameBuffer, mut x: i32, y: i32, text: &str, color: LinearRgba) {
    for ch in text.chars() {
        let bits = glyph_3x5(ch.to_ascii_uppercase());
        for row in 0..5 {
            for col in 0..3 {
                if ((bits[row] >> (2 - col)) & 1) != 0 {
                    let px = x + col as i32;
                    let py = y + row as i32;
                    if px >= 0 && py >= 0 {
                        if let Some(dst) = fb.get_pixel(px as u32, py as u32) {
                            fb.set_pixel(
                                px as u32,
                                py as u32,
                                blend_pixel(dst, color, BlendMode::Normal, 1.0, MixColorSpace::Linear),
                            );
                        }
                    }
                }
            }
        }
        x += 4;
    }
}

const fn glyph_3x5(c: char) -> [u8; 5] {
    match c {
        'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
        'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'N' => [0b101, 0b111, 0b111, 0b111, 0b101],
        'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'Q' => [0b010, 0b101, 0b101, 0b111, 0b011],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'V' => [0b101, 0b101, 0b101, 0b010, 0b010],
        'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b110, 0b001, 0b010, 0b100, 0b111],
        '3' => [0b110, 0b001, 0b010, 0b001, 0b110],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b110, 0b001, 0b110],
        '6' => [0b011, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b110],
        ':' => [0b000, 0b010, 0b000, 0b010, 0b000],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
        ' ' => [0b000, 0b000, 0b000, 0b000, 0b000],
        _ => [0b010, 0b000, 0b010, 0b000, 0b010],
    }
}
