// SPDX-License-Identifier: GPL-3.0-only
//! Rehearse Slide Timings, dual-clock HUD, and deterministic video timeline generator (Slice T6 / G-28).

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::{Canvas, Color, Rect};
use sex_font::{Font, GEIST_MONO, GEIST_SANS};
use crate::deck::Deck;
use crate::elements::Transition;
use crate::render::{draw_text_run, render_deck_transition, render_magic_move_frame, render_slide_at_build};

/// Recorded slide timing data from a rehearsal run.
#[derive(Clone, Debug, PartialEq)]
pub struct SlideTiming {
    pub slide_index: usize,
    pub dwell_ms: u32,
    pub build_delays_ms: Vec<u32>,
}

/// Interactive rehearsal session state machine.
#[derive(Clone, Debug)]
pub struct RehearsalSession {
    pub deck: Deck,
    pub current_slide: usize,
    pub current_build: usize,
    pub slide_elapsed_ms: u64,
    pub total_elapsed_ms: u64,
    pub recorded_timings: Vec<SlideTiming>,
    pub is_recording: bool,
    current_build_elapsed_ms: u64,
}

impl RehearsalSession {
    pub fn new(deck: Deck) -> Self {
        Self {
            deck,
            current_slide: 0,
            current_build: 0,
            slide_elapsed_ms: 0,
            total_elapsed_ms: 0,
            recorded_timings: Vec::new(),
            is_recording: true,
            current_build_elapsed_ms: 0,
        }
    }

    pub fn tick(&mut self, delta_ms: u64) {
        if self.is_recording {
            self.slide_elapsed_ms = self.slide_elapsed_ms.saturating_add(delta_ms);
            self.total_elapsed_ms = self.total_elapsed_ms.saturating_add(delta_ms);
            self.current_build_elapsed_ms = self.current_build_elapsed_ms.saturating_add(delta_ms);
        }
    }

    pub fn record_advance(&mut self) -> bool {
        let num_builds = self.deck.get_slide(self.current_slide).map(|s| s.builds.len()).unwrap_or(0);

        while self.recorded_timings.len() <= self.current_slide {
            let idx = self.recorded_timings.len();
            self.recorded_timings.push(SlideTiming {
                slide_index: idx,
                dwell_ms: 0,
                build_delays_ms: Vec::new(),
            });
        }

        if self.current_build < num_builds {
            let delay = self.current_build_elapsed_ms.min(u32::MAX as u64) as u32;
            self.recorded_timings[self.current_slide].build_delays_ms.push(delay.max(100));
            self.current_build += 1;
            self.current_build_elapsed_ms = 0;
            true
        } else {
            let dwell = self.slide_elapsed_ms.min(u32::MAX as u64) as u32;
            self.recorded_timings[self.current_slide].dwell_ms = dwell.max(500);

            if self.current_slide + 1 < self.deck.slide_count() {
                self.current_slide += 1;
                self.current_build = 0;
                self.slide_elapsed_ms = 0;
                self.current_build_elapsed_ms = 0;
                true
            } else {
                self.is_recording = false;
                false
            }
        }
    }

    pub fn apply_to_deck(&self, deck: &mut Deck) {
        for timing in &self.recorded_timings {
            if let Some(slide) = deck.get_slide_mut(timing.slide_index) {
                slide.rehearsed_dwell_ms = Some(timing.dwell_ms);
                for (b_idx, &delay) in timing.build_delays_ms.iter().enumerate() {
                    if let Some(b) = slide.builds.get_mut(b_idx) {
                        b.rehearsed_delay_ms = Some(delay);
                    }
                }
            }
        }
    }
}

/// A planned frame in a deterministic video timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineFrame {
    SlideHold {
        slide_idx: usize,
        completed_builds: usize,
    },
    SlideBuild {
        slide_idx: usize,
        completed_builds: usize,
        progress: f32,
    },
    SlideTransition {
        from_slide_idx: usize,
        to_slide_idx: usize,
        transition: Transition,
        progress: f32,
    },
}

/// Deterministic video timeline planner for slide decks.
#[derive(Clone, Debug)]
pub struct VideoTimeline {
    pub frames: Vec<TimelineFrame>,
    pub fps: u32,
}

impl VideoTimeline {
    /// Construct a deterministic video timeline from a presentation deck.
    /// Frame budget is bounded and scaled for smooth animation playback.
    pub fn from_deck(deck: &Deck, fps: u32) -> Self {
        let fps = fps.clamp(1, 60);
        let mut frames = Vec::new();
        let total_slides = deck.slide_count();

        if total_slides == 0 {
            return Self { frames, fps };
        }

        for slide_idx in 0..total_slides {
            let slide = &deck.slides[slide_idx];
            let num_builds = slide.builds.len();

            // 1. Initial hold before any builds
            let initial_hold_frames = ((fps as usize) * 4 / 10).clamp(3, 8);
            for _ in 0..initial_hold_frames {
                frames.push(TimelineFrame::SlideHold {
                    slide_idx,
                    completed_builds: 0,
                });
            }

            // 2. Intra-slide builds
            for b in 0..num_builds {
                let step = &slide.builds[b];
                // Delay hold before step if rehearsed or specified
                let delay_ms = step.rehearsed_delay_ms.unwrap_or(0);
                if delay_ms > 0 {
                    let delay_frames = ((delay_ms as usize * fps as usize) / 1000).clamp(2, 6);
                    for _ in 0..delay_frames {
                        frames.push(TimelineFrame::SlideHold {
                            slide_idx,
                            completed_builds: b,
                        });
                    }
                }

                // Build animation duration frames
                let anim_frames = ((step.duration_ms as usize * fps as usize) / 1000).clamp(3, 8);
                for f in 1..=anim_frames {
                    let progress = f as f32 / anim_frames as f32;
                    frames.push(TimelineFrame::SlideBuild {
                        slide_idx,
                        completed_builds: b,
                        progress,
                    });
                }

                // Post-build dwell hold
                let post_build_frames = ((fps as usize) * 3 / 10).clamp(2, 6);
                for _ in 0..post_build_frames {
                    frames.push(TimelineFrame::SlideHold {
                        slide_idx,
                        completed_builds: b + 1,
                    });
                }
            }

            // 3. Slide final dwell hold (after all builds)
            let dwell_ms = slide.rehearsed_dwell_ms.unwrap_or(1000);
            let dwell_frames = ((dwell_ms as usize * fps as usize) / 1000).clamp(4, 10);
            for _ in 0..dwell_frames {
                frames.push(TimelineFrame::SlideHold {
                    slide_idx,
                    completed_builds: num_builds,
                });
            }

            // 4. Slide-to-slide transition
            if slide_idx + 1 < total_slides {
                let next_slide_idx = slide_idx + 1;
                let transition = if slide.transition != Transition::None {
                    slide.transition
                } else {
                    Transition::Dissolve
                };

                let trans_duration_ms = slide.transition_duration_ms();
                let trans_frames = ((trans_duration_ms as usize * fps as usize) / 1000).clamp(4, 10);

                for f in 1..=trans_frames {
                    let progress = f as f32 / trans_frames as f32;
                    frames.push(TimelineFrame::SlideTransition {
                        from_slide_idx: slide_idx,
                        to_slide_idx: next_slide_idx,
                        transition,
                        progress,
                    });
                }
            }
        }

        Self { frames, fps }
    }

    pub fn total_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    pub fn duration_ms(&self) -> u64 {
        if self.fps == 0 {
            0
        } else {
            (self.frames.len() as u64 * 1000) / self.fps as u64
        }
    }

    pub fn render_frame(
        &self,
        deck: &Deck,
        frame_idx: usize,
        buf: &mut [u32],
        width: u32,
        height: u32,
    ) {
        if frame_idx >= self.frames.len() {
            return;
        }
        match &self.frames[frame_idx] {
            TimelineFrame::SlideHold {
                slide_idx,
                completed_builds,
            } => {
                if let Some(slide) = deck.get_slide(*slide_idx) {
                    render_slide_at_build(
                        slide,
                        &deck.theme,
                        deck.aspect_ratio,
                        *completed_builds,
                        None,
                        buf,
                        width,
                        height,
                    );
                }
            }
            TimelineFrame::SlideBuild {
                slide_idx,
                completed_builds,
                progress,
            } => {
                if let Some(slide) = deck.get_slide(*slide_idx) {
                    render_slide_at_build(
                        slide,
                        &deck.theme,
                        deck.aspect_ratio,
                        *completed_builds,
                        Some(*progress),
                        buf,
                        width,
                        height,
                    );
                }
            }
            TimelineFrame::SlideTransition {
                from_slide_idx,
                to_slide_idx,
                transition,
                progress,
            } => {
                if let (Some(from_s), Some(to_s)) = (deck.get_slide(*from_slide_idx), deck.get_slide(*to_slide_idx)) {
                    if *transition == Transition::MagicMove || *transition == Transition::Morph {
                        render_magic_move_frame(
                            from_s,
                            to_s,
                            &deck.theme,
                            deck.aspect_ratio,
                            *progress,
                            buf,
                            width,
                            height,
                        );
                    } else {
                        render_deck_transition(
                            from_s,
                            to_s,
                            &deck.theme,
                            deck.aspect_ratio,
                            *transition,
                            *progress,
                            buf,
                            width,
                            height,
                        );
                    }
                }
            }
        }
    }
}

/// Render a sleek Keynote-style Rehearse Slideshow overlay.
pub fn render_rehearsal_hud(
    rehearsal: &RehearsalSession,
    buf: &mut [u32],
    width: u32,
    height: u32,
) {
    let needed = match (width as usize).checked_mul(height as usize) {
        Some(n) => n,
        None => return,
    };
    if width == 0 || height == 0 || buf.len() < needed {
        return;
    }

    let mut canvas = match Canvas::new(buf, width, height) {
        Some(c) => c,
        None => return,
    };

    // Dark sleek studio backdrop
    let bg_color = Color::rgb(0x0F, 0x17, 0x2A);
    canvas.clear(bg_color);

    let sans_font = Font::new(GEIST_SANS);
    let mono_font = Font::new(GEIST_MONO);

    let sx = width as f32 / 480.0;
    let sy = height as f32 / 270.0;

    // Header bar
    let header_rect = Rect::new(0.0, 0.0, width as f32, 38.0 * sy);
    canvas.fill_rect(header_rect, Color::rgb(0x1E, 0x29, 0x3B));
    canvas.line(0.0, 38.0 * sy, width as f32, 38.0 * sy, 1.0, Color::rgb(0x33, 0x41, 0x55));

    // Recording Badge: Red pill
    let badge_rect = Rect::new(12.0 * sx, 8.0 * sy, 138.0 * sx, 22.0 * sy);
    canvas.fill_round_rect(badge_rect, 4.0 * sx, Color::rgb(0xDC, 0x26, 0x26));
    canvas.fill_circle(21.0 * sx, 19.0 * sy, 3.8 * sx, Color::WHITE);

    if let Some(ref font) = sans_font {
        draw_text_run(
            font,
            "RECORDING TIMINGS",
            9.5 * sy,
            29.0 * sx,
            22.5 * sy,
            Color::WHITE,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    // Timers: Slide timer & Total timer
    let slide_secs = (rehearsal.slide_elapsed_ms / 1000) % 60;
    let slide_mins = (rehearsal.slide_elapsed_ms / 60000) % 60;
    let total_secs = (rehearsal.total_elapsed_ms / 1000) % 60;
    let total_mins = (rehearsal.total_elapsed_ms / 60000) % 60;

    let slide_timer_str = format!("SLIDE: {:02}:{:02}", slide_mins, slide_secs);
    let total_timer_str = format!("TOTAL: {:02}:{:02}", total_mins, total_secs);

    if let Some(ref font) = mono_font {
        draw_text_run(
            font,
            &slide_timer_str,
            13.0 * sy,
            160.0 * sx,
            22.0 * sy,
            Color::rgb(0x38, 0xBD, 0xF8), // electric cyan
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );

        draw_text_run(
            font,
            &total_timer_str,
            13.0 * sy,
            270.0 * sx,
            22.0 * sy,
            Color::rgb(0xFB, 0xBF, 0x24), // amber gold
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    // Card layouts:
    // Left card: Current Slide at current build state
    let pad = 12.0 * sx;
    let top_y = 44.0 * sy;
    let bot_h = 32.0 * sy;
    let avail_h = height as f32 - top_y - bot_h - pad;
    let card_w = (width as f32 - pad * 3.0) / 2.0;
    let card_h = avail_h;

    let left_rect = Rect::new(pad, top_y, card_w, card_h);
    let right_rect = Rect::new(pad * 2.0 + card_w, top_y, card_w, card_h);

    // Frame surrounding cards
    canvas.fill_round_rect(left_rect, 6.0 * sx, Color::rgb(0x1E, 0x29, 0x3B));
    canvas.stroke_round_rect(left_rect, 6.0 * sx, 1.5, Color::rgb(0x47, 0x55, 0x69));

    canvas.fill_round_rect(right_rect, 6.0 * sx, Color::rgb(0x1E, 0x29, 0x3B));
    canvas.stroke_round_rect(right_rect, 6.0 * sx, 1.5, Color::rgb(0x47, 0x55, 0x69));

    // Render current slide preview inside left card
    let inner_pad = 4.0 * sx;
    let inner_w = (card_w - inner_pad * 2.0) as u32;
    let inner_h = (card_h - inner_pad * 2.0) as u32;

    if inner_w > 16 && inner_h > 16 {
        let mut left_buf = vec![0u32; (inner_w * inner_h) as usize];
        if let Some(slide) = rehearsal.deck.get_slide(rehearsal.current_slide) {
            render_slide_at_build(
                slide,
                &rehearsal.deck.theme,
                rehearsal.deck.aspect_ratio,
                rehearsal.current_build,
                None,
                &mut left_buf,
                inner_w,
                inner_h,
            );
        }

        // Blit left_buf into canvas at (left_rect.x + inner_pad, left_rect.y + inner_pad)
        let ox = (left_rect.x + inner_pad) as usize;
        let oy = (left_rect.y + inner_pad) as usize;
        for y in 0..(inner_h as usize) {
            let src_row = y * (inner_w as usize);
            let dst_row = (oy + y) * (width as usize) + ox;
            if dst_row + (inner_w as usize) <= canvas.pixels_mut().len() {
                canvas.pixels_mut()[dst_row..dst_row + (inner_w as usize)]
                    .copy_from_slice(&left_buf[src_row..src_row + (inner_w as usize)]);
            }
        }

        // Render right card: next build OR next slide preview
        let mut right_buf = vec![0u32; (inner_w * inner_h) as usize];
        let num_builds = rehearsal.deck.get_slide(rehearsal.current_slide).map(|s| s.builds.len()).unwrap_or(0);

        if rehearsal.current_build < num_builds {
            // Next build on same slide
            if let Some(slide) = rehearsal.deck.get_slide(rehearsal.current_slide) {
                render_slide_at_build(
                    slide,
                    &rehearsal.deck.theme,
                    rehearsal.deck.aspect_ratio,
                    rehearsal.current_build + 1,
                    None,
                    &mut right_buf,
                    inner_w,
                    inner_h,
                );
            }
        } else if let Some(next_slide) = rehearsal.deck.get_slide(rehearsal.current_slide + 1) {
            render_slide_at_build(
                next_slide,
                &rehearsal.deck.theme,
                rehearsal.deck.aspect_ratio,
                0,
                None,
                &mut right_buf,
                inner_w,
                inner_h,
            );
        } else {
            // End of presentation
            right_buf.fill(0xFF1E293B);
        }

        let rox = (right_rect.x + inner_pad) as usize;
        let roy = (right_rect.y + inner_pad) as usize;
        for y in 0..(inner_h as usize) {
            let src_row = y * (inner_w as usize);
            let dst_row = (roy + y) * (width as usize) + rox;
            if dst_row + (inner_w as usize) <= canvas.pixels_mut().len() {
                canvas.pixels_mut()[dst_row..dst_row + (inner_w as usize)]
                    .copy_from_slice(&right_buf[src_row..src_row + (inner_w as usize)]);
            }
        }
    }

    // Bottom Bar: Slide counter & Build pips
    let num_builds = rehearsal.deck.get_slide(rehearsal.current_slide).map(|s| s.builds.len()).unwrap_or(0);
    let counter_text = format!("Slide {} of {}", rehearsal.current_slide + 1, rehearsal.deck.slide_count());
    let build_text = format!("Build: {} / {}", rehearsal.current_build, num_builds);

    if let Some(ref font) = sans_font {
        draw_text_run(
            font,
            &counter_text,
            12.0 * sy,
            pad + 4.0 * sx,
            height as f32 - 12.0 * sy,
            Color::rgb(0x94, 0xA3, 0xB8),
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );

        draw_text_run(
            font,
            &build_text,
            12.0 * sy,
            width as f32 / 2.0 + pad,
            height as f32 - 12.0 * sy,
            Color::rgb(0x94, 0xA3, 0xB8),
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }
}
