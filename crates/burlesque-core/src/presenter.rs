// SPDX-License-Identifier: GPL-3.0-only
//! Presenter view session state machine and multi-pane presenter screen renderer.
//!
//! Provides a Keynote-grade dual-card Presenter Display:
//! - Left: Current slide preview, current slide badge `Slide M of N`, transition indicator
//! - Top-Right: Next slide preview (or "End of Presentation" card)
//! - Bottom-Right: Speaker Notes card with word-wrapped Geist Sans rendering
//! - Header: Presentation title, elapsed timer with Running/Paused status, progress bar, screen blanking

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::{Canvas, Color, Rect};
use sex_font::{Font, GEIST_MONO, GEIST_SANS};
use crate::deck::Deck;
use crate::render::{draw_text_run, render_slide};
use crate::Slide;

/// Screen blanking state for the presentation display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlankMode {
    None,
    Black,
    White,
}

/// Presenter view state machine.
#[derive(Clone, Debug)]
pub struct PresenterSession {
    pub deck: Deck,
    pub current_slide: usize,
    pub blank_mode: BlankMode,
    pub elapsed_ms: u64,
    pub timer_running: bool,
}

impl PresenterSession {
    pub fn new(deck: Deck) -> Self {
        Self {
            deck,
            current_slide: 0,
            blank_mode: BlankMode::None,
            elapsed_ms: 0,
            timer_running: true,
        }
    }

    pub fn current_slide(&self) -> Option<&Slide> {
        self.deck.get_slide(self.current_slide)
    }

    pub fn next_slide_preview(&self) -> Option<&Slide> {
        self.deck.get_slide(self.current_slide + 1)
    }

    pub fn next_slide(&mut self) -> bool {
        if self.current_slide + 1 < self.deck.slide_count() {
            self.current_slide += 1;
            true
        } else {
            false
        }
    }

    pub fn prev_slide(&mut self) -> bool {
        if self.current_slide > 0 {
            self.current_slide -= 1;
            true
        } else {
            false
        }
    }

    pub fn first_slide(&mut self) {
        self.current_slide = 0;
    }

    pub fn last_slide(&mut self) {
        if self.deck.slide_count() > 0 {
            self.current_slide = self.deck.slide_count() - 1;
        }
    }

    pub fn goto_slide(&mut self, index: usize) -> bool {
        if index < self.deck.slide_count() {
            self.current_slide = index;
            true
        } else {
            false
        }
    }

    pub fn step(&mut self) -> bool {
        self.next_slide()
    }

    pub fn toggle_timer(&mut self) {
        self.timer_running = !self.timer_running;
    }

    pub fn toggle_black_screen(&mut self) {
        self.blank_mode = match self.blank_mode {
            BlankMode::Black => BlankMode::None,
            _ => BlankMode::Black,
        };
    }

    pub fn toggle_white_screen(&mut self) {
        self.blank_mode = match self.blank_mode {
            BlankMode::White => BlankMode::None,
            _ => BlankMode::White,
        };
    }

    pub fn clear_blank(&mut self) {
        self.blank_mode = BlankMode::None;
    }

    pub fn tick(&mut self, delta_ms: u64) {
        if self.timer_running {
            self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        }
    }

    pub fn start_timer(&mut self) {
        self.timer_running = true;
    }

    pub fn stop_timer(&mut self) {
        self.timer_running = false;
    }

    pub fn reset_timer(&mut self) {
        self.elapsed_ms = 0;
    }

    pub fn slide_counter_text(&self) -> String {
        format!("{} / {}", self.current_slide + 1, self.deck.slide_count().max(1))
    }

    pub fn timer_text(&self) -> String {
        let total_secs = self.elapsed_ms / 1000;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    pub fn current_notes(&self) -> &str {
        if let Some(slide) = self.current_slide() {
            slide.notes()
        } else {
            ""
        }
    }
}

/// Render the complete presenter view UI into a target buffer.
pub fn render_presenter_view(
    session: &PresenterSession,
    buffer: &mut [u32],
    width: u32,
    height: u32,
) {
    let needed = match (width as usize).checked_mul(height as usize) {
        Some(n) => n,
        None => return,
    };
    if width == 0 || height == 0 || buffer.len() < needed {
        return;
    }
    if width < 320 || height < 200 {
        return;
    }

    let mut canvas = match Canvas::new(buffer, width, height) {
        Some(c) => c,
        None => return,
    };

    let bg_color = Color::rgb(0x0B, 0x11, 0x20); // Deep Midnight Slate
    let card_bg = Color::rgb(0x1E, 0x29, 0x3B);
    let border_color = Color::rgb(0x33, 0x41, 0x55);
    let text_white = Color::rgb(0xF8, 0xFA, 0xFC);
    let text_muted = Color::rgb(0x94, 0xA3, 0xB8);
    let accent_pink = Color::rgb(0xF4, 0x3F, 0x5E);
    let accent_gold = Color::rgb(0xF5, 0x9E, 0x0B);
    let accent_cyan = Color::rgb(0x38, 0xBD, 0xF8);

    canvas.clear(bg_color);

    let sans_font = match Font::new(GEIST_SANS) {
        Some(f) => f,
        None => return,
    };
    let mono_font = match Font::new(GEIST_MONO) {
        Some(f) => f,
        None => return,
    };

    let w = width as f32;
    let h = height as f32;

    // Top Header Bar (height 56)
    let header_h = 56.0;
    canvas.fill_rect(Rect::new(0.0, 0.0, w, header_h), card_bg);
    canvas.line(0.0, header_h, w, header_h, 1.5, border_color);

    // Progress bar across bottom of header
    let total_slides = session.deck.slide_count().max(1);
    let progress = ((session.current_slide + 1) as f32 / total_slides as f32).clamp(0.0, 1.0);
    canvas.fill_rect(Rect::new(0.0, header_h - 3.0, w * progress, 3.0), accent_gold);

    // Title / App name
    let title = if session.deck.metadata.title.is_empty() {
        "BURLESQUE PRESENTER"
    } else {
        session.deck.metadata.title.as_str()
    };
    draw_text_run(
        &sans_font,
        title,
        20.0,
        20.0,
        34.0,
        text_white,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Slide Counter badge
    let counter_str = session.slide_counter_text();
    let counter_text = format!("Slide {}", counter_str);
    draw_text_run(
        &sans_font,
        &counter_text,
        18.0,
        w * 0.40,
        34.0,
        accent_gold,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Timer display badge with Running/Paused indicator
    let timer_str = session.timer_text();
    let status_str = if session.timer_running { "RUN" } else { "PAUSE" };
    let timer_display = format!("{} [{}]", timer_str, status_str);
    draw_text_run(
        &mono_font,
        &timer_display,
        20.0,
        w - 200.0,
        34.0,
        if session.timer_running { accent_pink } else { text_muted },
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Blank screen warning if active
    if session.blank_mode != BlankMode::None {
        let blank_msg = match session.blank_mode {
            BlankMode::Black => "[ SCREEN BLANK (BLACK) ]",
            BlankMode::White => "[ SCREEN BLANK (WHITE) ]",
            BlankMode::None => "",
        };
        draw_text_run(
            &sans_font,
            blank_msg,
            16.0,
            w * 0.58,
            34.0,
            accent_pink,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    // Main layout dimensions
    let margin = 16.0;
    let main_top = header_h + margin;
    let left_w = (w - margin * 3.0) * 0.58;
    let bottom_bar_h = 40.0;
    let main_h = h - main_top - margin - bottom_bar_h;

    // Aspect-ratio-correct sub-canvas for current slide
    let deck_ratio = session.deck.aspect_ratio.ratio();
    let mut slide_preview_w = left_w;
    let mut slide_preview_h = left_w / deck_ratio;
    if slide_preview_h > main_h - 54.0 {
        slide_preview_h = main_h - 54.0;
        slide_preview_w = slide_preview_h * deck_ratio;
    }
    let slide_x = margin + (left_w - slide_preview_w) / 2.0;
    let slide_y = main_top + 30.0;

    // Current Slide Header & Transition badge
    let current_slide_ref = session.current_slide();
    let trans_name = current_slide_ref
        .map(|s| s.transition.name())
        .unwrap_or("None");
    let current_label = format!("CURRENT SLIDE  ·  Slide {}", session.slide_counter_text());
    draw_text_run(
        &sans_font,
        &current_label,
        15.0,
        margin + 4.0,
        main_top + 18.0,
        accent_cyan,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Frame current slide
    let frame_rect = Rect::new(slide_x - 2.0, slide_y - 2.0, slide_preview_w + 4.0, slide_preview_h + 4.0);
    canvas.stroke_round_rect(frame_rect, 4.0, 2.0, border_color);

    // Render current slide
    let pw = slide_preview_w as u32;
    let ph = slide_preview_h as u32;
    if let Some(pw_len) = (pw as usize).checked_mul(ph as usize) {
        if pw_len > 0 && pw_len <= 3840 * 2160 {
            let mut slide_buf = vec![0u32; pw_len];
            if let Some(slide) = current_slide_ref {
                render_slide(
                    slide,
                    &session.deck.theme,
                    session.deck.aspect_ratio,
                    &mut slide_buf,
                    pw,
                    ph,
                );
            } else {
                slide_buf.fill(card_bg.premultiplied());
            }
            blit_sub_buffer(canvas.pixels_mut(), width, height, slide_x as u32, slide_y as u32, &slide_buf, pw, ph);
        }
    }

    // Transition badge under current slide
    let trans_badge = format!("Transition: {} ({}ms)", trans_name, current_slide_ref.map(|s| s.transition_duration_ms()).unwrap_or(0));
    draw_text_run(
        &sans_font,
        &trans_badge,
        14.0,
        slide_x + 4.0,
        slide_y + slide_preview_h + 18.0,
        text_muted,
        false,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Right side:
    // Top Right: Next slide preview (42% width)
    let right_x = margin + left_w + margin;
    let right_w = w - right_x - margin;
    let next_h = main_h * 0.42;

    draw_text_run(
        &sans_font,
        "NEXT SLIDE",
        15.0,
        right_x + 4.0,
        main_top + 18.0,
        accent_gold,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    let mut next_w = right_w;
    let mut next_h_fit = right_w / deck_ratio;
    if next_h_fit > next_h - 26.0 {
        next_h_fit = next_h - 26.0;
        next_w = next_h_fit * deck_ratio;
    }
    let next_x_pos = right_x + (right_w - next_w) / 2.0;
    let next_y_pos = main_top + 30.0;

    let next_frame = Rect::new(next_x_pos - 2.0, next_y_pos - 2.0, next_w + 4.0, next_h_fit + 4.0);
    canvas.stroke_round_rect(next_frame, 4.0, 1.5, border_color);

    let npw = next_w as u32;
    let nph = next_h_fit as u32;
    if let Some(next_slide) = session.next_slide_preview() {
        if let Some(npw_len) = (npw as usize).checked_mul(nph as usize) {
            if npw_len > 0 && npw_len <= 3840 * 2160 {
                let mut next_buf = vec![0u32; npw_len];
                render_slide(
                    next_slide,
                    &session.deck.theme,
                    session.deck.aspect_ratio,
                    &mut next_buf,
                    npw,
                    nph,
                );
                blit_sub_buffer(canvas.pixels_mut(), width, height, next_x_pos as u32, next_y_pos as u32, &next_buf, npw, nph);
            }
        }
    } else {
        // End of Presentation card
        canvas.fill_round_rect(next_frame, 4.0, card_bg);
        draw_text_run(
            &sans_font,
            "End of Presentation",
            18.0,
            next_x_pos + (next_w - 180.0) / 2.0,
            next_y_pos + next_h_fit / 2.0 + 6.0,
            text_muted,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    // Bottom Right: Speaker Notes panel
    let notes_top = next_y_pos + next_h_fit + 18.0;
    let notes_h = main_top + main_h - notes_top;
    let notes_rect = Rect::new(right_x, notes_top, right_w, notes_h);
    canvas.fill_round_rect(notes_rect, 6.0, card_bg);
    canvas.stroke_round_rect(notes_rect, 6.0, 1.5, border_color);

    draw_text_run(
        &sans_font,
        "SPEAKER NOTES",
        15.0,
        right_x + 12.0,
        notes_top + 24.0,
        accent_gold,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Word-wrapped speaker notes text
    let notes_content = if let Some(slide) = current_slide_ref {
        let n = slide.notes();
        if n.is_empty() {
            "No speaker notes for this slide"
        } else {
            n
        }
    } else {
        "No speaker notes for this slide"
    };

    let max_note_chars = ((right_w - 32.0) / 9.5).max(20.0) as usize;
    let mut note_y = notes_top + 48.0;
    let line_spacing = 20.0;

    for raw_line in notes_content.lines() {
        if note_y > notes_rect.bottom() - 14.0 {
            break;
        }
        let wrapped = wrap_text_line(raw_line, max_note_chars);
        for line in wrapped {
            if note_y > notes_rect.bottom() - 14.0 {
                break;
            }
            draw_text_run(
                &sans_font,
                &line,
                15.0,
                right_x + 16.0,
                note_y,
                text_white,
                false,
                false,
                canvas.pixels_mut(),
                width,
                height,
            );
            note_y += line_spacing;
        }
    }

    // Bottom Navigation Bar
    let bottom_y = h - bottom_bar_h;
    canvas.fill_rect(Rect::new(0.0, bottom_y, w, bottom_bar_h), card_bg);
    canvas.line(0.0, bottom_y, w, bottom_bar_h, 1.0, border_color);

    let nav_hint = "[←/→] Prev/Next   [Space] Advance   [T] Timer   [R] Reset   [B] Black Screen   [W] White Screen";
    draw_text_run(
        &sans_font,
        nav_hint,
        14.0,
        24.0,
        bottom_y + 24.0,
        text_muted,
        false,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );
}

fn wrap_text_line(line: &str, max_chars: usize) -> Vec<String> {
    if line.len() <= max_chars {
        return vec![String::from(line)];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in line.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.len() + 1 + word.len() <= max_chars {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(cur);
            cur = String::from(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn blit_sub_buffer(
    target: &mut [u32],
    target_w: u32,
    target_h: u32,
    dst_x: u32,
    dst_y: u32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    for y in 0..src_h {
        let ty = dst_y + y;
        if ty >= target_h {
            break;
        }
        let target_row = ty as usize * target_w as usize;
        let src_row = y as usize * src_w as usize;
        for x in 0..src_w {
            let tx = dst_x + x;
            if tx >= target_w {
                break;
            }
            target[target_row + tx as usize] = src[src_row + x as usize];
        }
    }
}
