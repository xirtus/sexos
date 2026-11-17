// SPDX-License-Identifier: GPL-3.0-only
//! Live presentation source feed for **Cabaret** (`apps/cabaret`).
//!
//! Provides [`LivePresentationSource`], a stateful step-through presenter with:
//! - Build-step and slide navigation (`next_step`, `prev_step`, `goto_slide`, `tick`)
//! - Smooth intra-slide build playback (`OnClick`, `WithPrevious`, `AfterPrevious`)
//!   and slide-to-slide Morph / MagicMove / standard transitions
//! - Three output modes via [`PresentationOverlayMode`]:
//!   - `FullSlide`: full opaque presentation canvas
//!   - `TransparentSlide`: transparent-background element overlay (`alpha = 0` backdrop)
//!   - `LowerThird`: transparent-background broadcast lower-third chyron banner
//! - Direct rendering to straight 8-bit `[R, G, B, A]` byte buffers (matching
//!   `cabaret_core::compositor::FrameBuffer::from_srgb_rgba8`) and SexOS `0xAARRGGBB` `u32` words
//! - Accessors `presenter_notes()` and `state()` -> `(slide_index, total_slides, step_index, total_steps)`

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::{Canvas, Color, Rect};
use sex_font::{Font, GEIST_SANS};

use crate::animation::BuildTrigger;
use crate::deck::{Deck, Slide};
use crate::elements::{SlideElement, Transition};
use crate::morph::{render_morph_frame_with_options, MorphEasing};
use crate::ops::{apply_ops_json, ApplyOpsResult, OpsError};
use crate::render::{draw_text_run, render_deck_transition, render_slide_at_build_with_options};
use crate::Result;

/// Output presentation compositing mode for Cabaret scene ingestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationOverlayMode {
    /// Full-frame slide rendering with opaque slide/theme background.
    FullSlide,
    /// Slide elements rendered onto a transparent (`alpha = 0`) canvas.
    TransparentSlide,
    /// Transparent-background broadcast lower-third overlay banner showing slide title and active point.
    LowerThird,
}

impl Default for PresentationOverlayMode {
    fn default() -> Self {
        Self::FullSlide
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ActiveTransition {
    from_slide: usize,
    to_slide: usize,
    transition: Transition,
    elapsed_ms: u32,
    duration_ms: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct ActiveBuild {
    slide_index: usize,
    completed_before: usize,
    target_completed: usize,
    delay_ms: u32,
    elapsed_ms: u32,
    duration_ms: u32,
}

/// Stateful live presentation source feed designed for Cabaret broadcast compositing.
#[derive(Clone, Debug)]
pub struct LivePresentationSource {
    deck: Deck,
    slide_index: usize,
    step_index: usize,
    overlay_mode: PresentationOverlayMode,
    skip_hidden: bool,
    auto_play_after_previous: bool,
    animate_steps: bool,
    active_transition: Option<ActiveTransition>,
    active_build: Option<ActiveBuild>,
}

impl LivePresentationSource {
    /// Create a new live presentation source positioned at the first non-skipped slide (or 0).
    pub fn new(deck: Deck) -> Self {
        let first = deck
            .slides
            .iter()
            .position(|s| !s.hidden)
            .unwrap_or(0);
        Self {
            deck,
            slide_index: first,
            step_index: 0,
            overlay_mode: PresentationOverlayMode::FullSlide,
            skip_hidden: true,
            auto_play_after_previous: true,
            animate_steps: true,
            active_transition: None,
            active_build: None,
        }
    }

    /// Open a `.pptx` deck directly into a live presentation source.
    pub fn from_pptx(bytes: &[u8]) -> Result<Self> {
        Ok(Self::new(Deck::open_pptx(bytes)?))
    }

    /// Open an `.odp` deck directly into a live presentation source.
    pub fn from_odp(bytes: &[u8]) -> Result<Self> {
        Ok(Self::new(Deck::open_odp(bytes)?))
    }

    /// Open an Apple Keynote `.key` bundle directly into a live presentation source.
    pub fn from_keynote(bytes: &[u8]) -> Result<Self> {
        Ok(Self::new(Deck::open_keynote(bytes)?))
    }

    /// Open a Markdown presentation directly into a live presentation source.
    pub fn from_markdown(md: &str) -> Result<Self> {
        Ok(Self::new(Deck::from_markdown(md)?))
    }

    pub fn with_overlay_mode(mut self, mode: PresentationOverlayMode) -> Self {
        self.overlay_mode = mode;
        self
    }

    pub fn with_skip_hidden(mut self, skip_hidden: bool) -> Self {
        self.skip_hidden = skip_hidden;
        self
    }

    pub fn overlay_mode(&self) -> PresentationOverlayMode {
        self.overlay_mode
    }

    pub fn set_overlay_mode(&mut self, mode: PresentationOverlayMode) {
        self.overlay_mode = mode;
    }

    pub fn skip_hidden(&self) -> bool {
        self.skip_hidden
    }

    pub fn set_skip_hidden(&mut self, skip_hidden: bool) {
        self.skip_hidden = skip_hidden;
    }

    pub fn set_auto_play_after_previous(&mut self, enabled: bool) {
        self.auto_play_after_previous = enabled;
    }

    pub fn set_animate_steps(&mut self, animate: bool) {
        self.animate_steps = animate;
    }

    pub fn deck(&self) -> &Deck {
        &self.deck
    }

    pub fn deck_mut(&mut self) -> &mut Deck {
        &mut self.deck
    }

    /// Atomically apply OmaShow-style JSON ops to the live deck, clamping cursor bounds safely.
    pub fn apply_ops_json(&mut self, ops_json: &str) -> core::result::Result<ApplyOpsResult, OpsError> {
        let res = apply_ops_json(&mut self.deck, ops_json)?;
        let total = self.deck.slide_count();
        if total == 0 {
            self.slide_index = 0;
            self.step_index = 0;
        } else {
            self.slide_index = self.slide_index.min(total - 1);
            let max_steps = self.total_steps();
            self.step_index = self.step_index.min(max_steps);
        }
        self.active_transition = None;
        self.active_build = None;
        Ok(res)
    }

    /// Returns `(slide_index, total_slides, step_index, total_steps)` for Cabaret HUD & RPC sync.
    pub fn state(&self) -> (usize, usize, usize, usize) {
        (
            self.slide_index,
            self.deck.slide_count(),
            self.step_index,
            self.total_steps(),
        )
    }

    /// Alias for [`Self::state`].
    pub fn progress(&self) -> (usize, usize, usize, usize) {
        self.state()
    }

    pub fn slide_index(&self) -> usize {
        self.slide_index
    }

    pub fn total_slides(&self) -> usize {
        self.deck.slide_count()
    }

    pub fn step_index(&self) -> usize {
        self.step_index
    }

    pub fn total_steps(&self) -> usize {
        self.deck
            .get_slide(self.slide_index)
            .map(|s| s.builds.len())
            .unwrap_or(0)
    }

    /// Speaker / presenter notes for the active slide (used by Cabaret teleprompter/notes pane).
    pub fn presenter_notes(&self) -> &str {
        self.deck
            .get_slide(self.slide_index)
            .map(|s| s.notes())
            .unwrap_or("")
    }

    /// Title of the active slide.
    pub fn slide_title(&self) -> &str {
        self.deck
            .get_slide(self.slide_index)
            .map(|s| s.title.as_str())
            .unwrap_or("")
    }

    pub fn is_animating(&self) -> bool {
        self.active_transition.is_some() || self.active_build.is_some()
    }

    /// Immediately finalize any in-progress transition or build animation.
    pub fn flush_animations(&mut self) {
        self.active_transition = None;
        if let Some(ab) = self.active_build.take() {
            if self.slide_index == ab.slide_index {
                self.step_index = ab.target_completed.min(self.total_steps());
            }
        }
    }

    fn find_next_slide(&self, from: usize) -> Option<usize> {
        let total = self.deck.slide_count();
        for idx in (from + 1)..total {
            if !self.skip_hidden || !self.deck.slides[idx].hidden {
                return Some(idx);
            }
        }
        None
    }

    fn find_prev_slide(&self, from: usize) -> Option<usize> {
        for idx in (0..from).rev() {
            if !self.skip_hidden || !self.deck.slides[idx].hidden {
                return Some(idx);
            }
        }
        None
    }

    fn group_end_from(&self, slide: &Slide, start_step: usize) -> usize {
        if start_step >= slide.builds.len() {
            return slide.builds.len();
        }
        let mut end = start_step + 1;
        while end < slide.builds.len()
            && matches!(slide.builds[end].trigger, BuildTrigger::WithPrevious)
        {
            end += 1;
        }
        end
    }

    fn group_start_before(&self, slide: &Slide, cur_step: usize) -> usize {
        if cur_step == 0 {
            return 0;
        }
        let mut start = cur_step.min(slide.builds.len()).saturating_sub(1);
        while start > 0 && matches!(slide.builds[start].trigger, BuildTrigger::WithPrevious) {
            start -= 1;
        }
        start
    }

    fn start_build_step(&mut self, completed_before: usize, target_completed: usize, initial_delay_ms: u32) {
        let Some(slide) = self.deck.get_slide(self.slide_index) else {
            return;
        };
        if completed_before >= target_completed || target_completed > slide.builds.len() {
            return;
        }
        let max_dur = slide.builds[completed_before..target_completed]
            .iter()
            .map(|b| b.duration_ms)
            .max()
            .unwrap_or(0);

        self.step_index = target_completed;
        if self.animate_steps && (max_dur > 0 || initial_delay_ms > 0) {
            self.active_build = Some(ActiveBuild {
                slide_index: self.slide_index,
                completed_before,
                target_completed,
                delay_ms: initial_delay_ms,
                elapsed_ms: 0,
                duration_ms: max_dur.max(1),
            });
        } else {
            self.active_build = None;
        }
    }

    /// Advance to the next build step (including `WithPrevious` companions) or next visible slide.
    pub fn next_step(&mut self) -> bool {
        self.flush_animations();

        let total_slides = self.deck.slide_count();
        if total_slides == 0 || self.slide_index >= total_slides {
            return false;
        }

        let current_slide = &self.deck.slides[self.slide_index];
        let total_builds = current_slide.builds.len();

        if self.step_index < total_builds {
            let next_end = self.group_end_from(current_slide, self.step_index);
            self.start_build_step(self.step_index, next_end, 0);
            return true;
        }

        // Advance to next visible slide
        let Some(next_idx) = self.find_next_slide(self.slide_index) else {
            return false;
        };
        let from_idx = self.slide_index;
        let trans = self.deck.slides[from_idx].transition;
        let trans_dur = self.deck.slides[from_idx].transition_duration_ms();

        self.slide_index = next_idx;
        self.step_index = 0;

        if self.animate_steps && trans != Transition::None && trans_dur > 0 {
            self.active_transition = Some(ActiveTransition {
                from_slide: from_idx,
                to_slide: next_idx,
                transition: trans,
                elapsed_ms: 0,
                duration_ms: trans_dur,
            });
        } else {
            self.maybe_trigger_after_previous();
        }
        true
    }

    /// Step backward to the previous build step or previous visible slide (at its completed builds).
    pub fn prev_step(&mut self) -> bool {
        self.flush_animations();

        let total_slides = self.deck.slide_count();
        if total_slides == 0 || self.slide_index >= total_slides {
            return false;
        }

        if self.step_index > 0 {
            let slide = &self.deck.slides[self.slide_index];
            self.step_index = self.group_start_before(slide, self.step_index);
            return true;
        }

        let Some(prev_idx) = self.find_prev_slide(self.slide_index) else {
            return false;
        };
        self.slide_index = prev_idx;
        self.step_index = self.deck.slides[prev_idx].builds.len();
        true
    }

    /// Jump directly to slide `idx` (resetting step index to 0).
    pub fn goto_slide(&mut self, idx: usize) -> bool {
        if idx >= self.deck.slide_count() {
            return false;
        }
        self.active_transition = None;
        self.active_build = None;
        self.slide_index = idx;
        self.step_index = 0;
        true
    }

    /// Advance internal transition and build animations by `dt_ms` milliseconds.
    ///
    /// Returns `true` if any active animation progressed or triggered a visual update.
    pub fn tick(&mut self, dt_ms: u32) -> bool {
        if dt_ms == 0 {
            return self.is_animating();
        }

        let mut progressed = false;

        if let Some(mut tr) = self.active_transition.take() {
            tr.elapsed_ms = tr.elapsed_ms.saturating_add(dt_ms);
            if tr.elapsed_ms < tr.duration_ms {
                self.active_transition = Some(tr);
                return true;
            }
            // Transition finished -> check if slide starts with AfterPrevious
            self.maybe_trigger_after_previous();
            return true;
        }

        if let Some(mut ab) = self.active_build.take() {
            progressed = true;
            if ab.delay_ms > 0 {
                if dt_ms < ab.delay_ms {
                    ab.delay_ms -= dt_ms;
                    self.active_build = Some(ab);
                    return true;
                }
                let rem = dt_ms - ab.delay_ms;
                ab.delay_ms = 0;
                ab.elapsed_ms = ab.elapsed_ms.saturating_add(rem);
            } else {
                ab.elapsed_ms = ab.elapsed_ms.saturating_add(dt_ms);
            }

            if ab.elapsed_ms < ab.duration_ms {
                self.active_build = Some(ab);
                return true;
            }

            // Build completed
            self.maybe_trigger_after_previous();
        }

        progressed
    }

    fn maybe_trigger_after_previous(&mut self) {
        if !self.auto_play_after_previous {
            return;
        }
        let Some(slide) = self.deck.get_slide(self.slide_index) else {
            return;
        };
        if self.step_index >= slide.builds.len() {
            return;
        }
        if let BuildTrigger::AfterPrevious { delay_ms } = slide.builds[self.step_index].trigger {
            let next_end = self.group_end_from(slide, self.step_index);
            let eff_delay = delay_ms.max(slide.build_delay_ms(self.step_index));
            self.start_build_step(self.step_index, next_end, eff_delay);
        }
    }

    /// Render current presentation state into a SexOS `0xAARRGGBB` `u32` buffer.
    pub fn render_argb_u32(&self, buffer: &mut [u32], width: u32, height: u32) -> bool {
        let Some(needed) = (width as usize).checked_mul(height as usize) else {
            return false;
        };
        if width == 0 || height == 0 || buffer.len() < needed {
            return false;
        }

        match self.overlay_mode {
            PresentationOverlayMode::LowerThird => {
                self.render_lower_third_argb(buffer, width, height);
                true
            }
            PresentationOverlayMode::FullSlide | PresentationOverlayMode::TransparentSlide => {
                let transparent = self.overlay_mode == PresentationOverlayMode::TransparentSlide;
                self.render_slide_or_transition_argb(buffer, width, height, transparent)
            }
        }
    }

    /// Render current presentation state into a caller-provided straight 8-bit `[R, G, B, A]` buffer.
    ///
    /// Directly compatible with `cabaret_core::compositor::FrameBuffer::from_srgb_rgba8`.
    pub fn render_rgba_into(&self, out: &mut [u8], width: u32, height: u32) -> bool {
        let Some(px_count) = (width as usize).checked_mul(height as usize) else {
            return false;
        };
        let Some(byte_len) = px_count.checked_mul(4) else {
            return false;
        };
        if width == 0 || height == 0 || out.len() < byte_len {
            return false;
        }
        let mut argb = vec![0u32; px_count];
        if !self.render_argb_u32(&mut argb, width, height) {
            return false;
        }
        argb_premul_to_rgba8(&argb, &mut out[..byte_len]);
        true
    }

    /// Allocate and render current presentation state as straight 8-bit `[R, G, B, A]` bytes.
    pub fn render_rgba(&self, width: u32, height: u32) -> Vec<u8> {
        let Some(px_count) = (width as usize).checked_mul(height as usize) else {
            return Vec::new();
        };
        let Some(byte_len) = px_count.checked_mul(4) else {
            return Vec::new();
        };
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let mut out = vec![0u8; byte_len];
        let _ = self.render_rgba_into(&mut out, width, height);
        out
    }

    fn render_slide_or_transition_argb(
        &self,
        buffer: &mut [u32],
        width: u32,
        height: u32,
        transparent_bg: bool,
    ) -> bool {
        if let Some(ref tr) = self.active_transition {
            let t = if tr.duration_ms > 0 {
                (tr.elapsed_ms as f32 / tr.duration_ms as f32).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if let (Some(from_s), Some(to_s)) = (
                self.deck.get_slide(tr.from_slide),
                self.deck.get_slide(tr.to_slide),
            ) {
                if tr.transition == Transition::MagicMove || tr.transition == Transition::Morph {
                    render_morph_frame_with_options(
                        from_s,
                        to_s,
                        &self.deck.theme,
                        self.deck.aspect_ratio,
                        t,
                        MorphEasing::EaseInOutCubic,
                        transparent_bg,
                        buffer,
                        width,
                        height,
                    );
                    return true;
                }
                if !transparent_bg {
                    render_deck_transition(
                        from_s,
                        to_s,
                        &self.deck.theme,
                        self.deck.aspect_ratio,
                        tr.transition,
                        t,
                        buffer,
                        width,
                        height,
                    );
                    return true;
                }
                // Transparent non-morph transition: render both transparent & dissolve
                let len = (width * height) as usize;
                let mut fbuf = vec![0u32; len];
                let mut tbuf = vec![0u32; len];
                render_slide_at_build_with_options(
                    from_s,
                    &self.deck.theme,
                    self.deck.aspect_ratio,
                    from_s.builds.len(),
                    None,
                    true,
                    &mut fbuf,
                    width,
                    height,
                );
                render_slide_at_build_with_options(
                    to_s,
                    &self.deck.theme,
                    self.deck.aspect_ratio,
                    0,
                    None,
                    true,
                    &mut tbuf,
                    width,
                    height,
                );
                crate::render::render_transition_frame(&fbuf, &tbuf, buffer, width, height, tr.transition, t);
                return true;
            }
        }

        let Some(slide) = self.deck.get_slide(self.slide_index) else {
            buffer[..(width * height) as usize].fill(0);
            return true;
        };

        let (completed, active_progress) = if let Some(ref ab) = self.active_build {
            if ab.slide_index == self.slide_index {
                if ab.delay_ms > 0 {
                    (ab.completed_before, None)
                } else {
                    let p = (ab.elapsed_ms as f32 / ab.duration_ms.max(1) as f32).clamp(0.0, 1.0);
                    (ab.completed_before, Some(p))
                }
            } else {
                (self.step_index, None)
            }
        } else {
            (self.step_index, None)
        };

        render_slide_at_build_with_options(
            slide,
            &self.deck.theme,
            self.deck.aspect_ratio,
            completed,
            active_progress,
            transparent_bg,
            buffer,
            width,
            height,
        );
        true
    }

    fn render_lower_third_argb(&self, buffer: &mut [u32], width: u32, height: u32) {
        let len = (width * height) as usize;
        buffer[..len].fill(0);

        let Some(mut canvas) = Canvas::new(buffer, width, height) else {
            return;
        };
        canvas.clear(Color::TRANSPARENT);

        let Some(slide) = self.deck.get_slide(self.slide_index) else {
            return;
        };

        // Transition / fade progress factor
        let opacity = if let Some(ref tr) = self.active_transition {
            let t = (tr.elapsed_ms as f32 / tr.duration_ms.max(1) as f32).clamp(0.0, 1.0);
            t
        } else {
            1.0
        };
        if opacity <= 0.01 {
            return;
        }

        let w = width as f32;
        let h = height as f32;
        let sx = w / 1280.0;
        let sy = h / 720.0;

        let bar_x = w * 0.05;
        let bar_w = w * 0.90;
        let bar_h = (h * 0.17).clamp(48.0, 180.0);
        let bar_y = h - bar_h - (h * 0.05);

        let pal = &self.deck.theme.palette;
        let alpha_u8 = (232.0 * opacity).clamp(0.0, 255.0) as u8;
        let card_bg = pal.surface.with_alpha(alpha_u8);
        let accent = pal.primary_accent.with_alpha((255.0 * opacity) as u8);
        let accent2 = pal.secondary_accent.with_alpha((255.0 * opacity) as u8);
        let fg = pal.foreground.with_alpha((255.0 * opacity) as u8);
        let muted = pal.muted.with_alpha((240.0 * opacity) as u8);

        let card_rect = Rect::new(bar_x, bar_y, bar_w, bar_h);
        canvas.fill_round_rect(card_rect, 10.0 * sx, card_bg);
        canvas.stroke_round_rect(card_rect, 10.0 * sx, 1.5, accent2);

        // Left vertical accent pillar
        let pillar_w = (8.0 * sx).max(4.0);
        canvas.fill_round_rect(
            Rect::new(bar_x, bar_y, pillar_w, bar_h),
            4.0 * sx,
            accent,
        );

        let Some(font) = Font::new(GEIST_SANS) else {
            return;
        };

        let title_str = if slide.title.is_empty() {
            format!("Slide {}", self.slide_index + 1)
        } else {
            slide.title.clone()
        };

        let title_sz = (28.0 * sy).clamp(11.0, 46.0);
        let sub_sz = (18.0 * sy).clamp(9.0, 30.0);
        let text_x = bar_x + pillar_w + 16.0 * sx;
        let title_y = bar_y + bar_h * 0.44;
        let sub_y = bar_y + bar_h * 0.78;

        draw_text_run(
            &font,
            &title_str,
            title_sz,
            text_x,
            title_y,
            fg,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );

        // Subtitle / active visible bullet line
        let subtitle = lower_third_subtitle(slide, self.step_index);
        if !subtitle.is_empty() {
            draw_text_run(
                &font,
                &subtitle,
                sub_sz,
                text_x,
                sub_y,
                muted,
                false,
                false,
                canvas.pixels_mut(),
                width,
                height,
            );
        }

        // Slide / build progress pill on right
        let badge = if slide.builds.is_empty() {
            format!("{}/{}", self.slide_index + 1, self.deck.slide_count().max(1))
        } else {
            format!(
                "{}/{} · {}/{}",
                self.slide_index + 1,
                self.deck.slide_count().max(1),
                self.step_index,
                slide.builds.len()
            )
        };
        let badge_x = (bar_x + bar_w - 110.0 * sx).max(text_x + 40.0);
        draw_text_run(
            &font,
            &badge,
            (15.0 * sy).clamp(9.0, 22.0),
            badge_x,
            bar_y + bar_h * 0.36,
            accent2,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }
}

fn lower_third_subtitle(slide: &Slide, completed_steps: usize) -> String {
    // Pick the most recently revealed non-title text line
    let mut candidate = String::new();
    for (e_idx, elem) in slide.elements.iter().enumerate() {
        let is_hidden_by_build = slide
            .builds
            .iter()
            .enumerate()
            .find(|(_, b)| b.element_index == e_idx && b.paragraph_index.is_none())
            .map(|(k, step)| {
                if step.effect.is_build_out() {
                    k < completed_steps
                } else {
                    k >= completed_steps
                }
            })
            .unwrap_or(false);
        if is_hidden_by_build {
            continue;
        }
        if let SlideElement::TextBox(tb) = elem {
            for (p_idx, p) in tb.paragraphs.iter().enumerate() {
                let para_hidden = slide
                    .builds
                    .iter()
                    .enumerate()
                    .find(|(_, b)| b.element_index == e_idx && b.paragraph_index == Some(p_idx))
                    .map(|(k, step)| {
                        if step.effect.is_build_out() {
                            k < completed_steps
                        } else {
                            k >= completed_steps
                        }
                    })
                    .unwrap_or(false);
                if para_hidden {
                    continue;
                }
                let pt = p.plain_text();
                let trimmed = pt.trim();
                if !trimmed.is_empty() && trimmed != slide.title.trim() {
                    candidate = trimmed.to_string();
                }
            }
        }
    }
    candidate
}

/// Convert premultiplied `0xAARRGGBB` words into unpremultiplied `[R, G, B, A]` bytes.
fn argb_premul_to_rgba8(src: &[u32], dst: &mut [u8]) {
    for (&word, out) in src.iter().zip(dst.chunks_exact_mut(4)) {
        let a = ((word >> 24) & 0xFF) as u32;
        if a == 0 {
            out[0] = 0;
            out[1] = 0;
            out[2] = 0;
            out[3] = 0;
        } else if a == 255 {
            out[0] = ((word >> 16) & 0xFF) as u8;
            out[1] = ((word >> 8) & 0xFF) as u8;
            out[2] = (word & 0xFF) as u8;
            out[3] = 255;
        } else {
            let pr = (word >> 16) & 0xFF;
            let pg = (word >> 8) & 0xFF;
            let pb = word & 0xFF;
            let half = a / 2;
            out[0] = ((pr * 255 + half) / a).min(255) as u8;
            out[1] = ((pg * 255 + half) / a).min(255) as u8;
            out[2] = ((pb * 255 + half) / a).min(255) as u8;
            out[3] = a as u8;
        }
    }
}
