// SPDX-License-Identifier: GPL-3.0-only
//! Presenter view session state machine and multi-pane presenter screen renderer.

use alloc::string::String;
use alloc::vec;
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
        alloc::format!("{} / {}", self.current_slide + 1, self.deck.slide_count().max(1))
    }

    pub fn timer_text(&self) -> String {
        let total_secs = self.elapsed_ms / 1000;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        alloc::format!("{:02}:{:02}", mins, secs)
    }
}

/// Render the complete presenter view UI into a target buffer.
pub fn render_presenter_view(
    session: &PresenterSession,
    buffer: &mut [u32],
    width: u32,
    height: u32,
) {
    let mut canvas = match Canvas::new(buffer, width, height) {
        Some(c) => c,
        None => return,
    };

    let bg_color = Color::rgb(0x0F, 0x17, 0x2A); // Dark slate
    let card_bg = Color::rgb(0x1E, 0x29, 0x3B);
    let border_color = Color::rgb(0x33, 0x41, 0x55);
    let text_white = Color::rgb(0xF8, 0xFA, 0xFC);
    let text_muted = Color::rgb(0x94, 0xA3, 0xB8);
    let accent_pink = Color::rgb(0xF4, 0x3F, 0x5E);
    let accent_gold = Color::rgb(0xF5, 0x9E, 0x0B);

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

    // Title / App name
    let title = if session.deck.metadata.title.is_empty() {
        "BURLESQUE PRESENTER"
    } else {
        session.deck.metadata.title.as_str()
    };
    draw_text_run(
        &sans_font,
        title,
        22.0,
        20.0,
        36.0,
        text_white,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Slide Counter badge
    let counter_str = session.slide_counter_text();
    let counter_text = alloc::format!("Slide {}", counter_str);
    draw_text_run(
        &sans_font,
        &counter_text,
        20.0,
        w * 0.45,
        35.0,
        accent_gold,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Timer display badge
    let timer_str = session.timer_text();
    let timer_display = alloc::format!("Time: {}", timer_str);
    draw_text_run(
        &mono_font,
        &timer_display,
        22.0,
        w - 180.0,
        36.0,
        accent_pink,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Blank screen warning if active
    if session.blank_mode != BlankMode::None {
        let blank_msg = match session.blank_mode {
            BlankMode::Black => "[ SCREEN BLANKED (BLACK) ]",
            BlankMode::White => "[ SCREEN BLANKED (WHITE) ]",
            BlankMode::None => "",
        };
        draw_text_run(
            &sans_font,
            blank_msg,
            18.0,
            w * 0.65,
            35.0,
            accent_pink,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    // Main layout:
    // Left: Current Slide preview (58% width)
    let margin = 16.0;
    let main_top = header_h + margin;
    let left_w = (w - margin * 3.0) * 0.58;
    let bottom_bar_h = 40.0;
    let main_h = h - main_top - margin - bottom_bar_h;

    // Aspect-ratio-correct sub-canvas for current slide
    let deck_ratio = session.deck.aspect_ratio.ratio();
    let mut slide_preview_w = left_w;
    let mut slide_preview_h = left_w / deck_ratio;
    if slide_preview_h > main_h - 40.0 {
        slide_preview_h = main_h - 40.0;
        slide_preview_w = slide_preview_h * deck_ratio;
    }
    let slide_x = margin + (left_w - slide_preview_w) / 2.0;
    let slide_y = main_top + 32.0;

    // Label: "CURRENT SLIDE"
    draw_text_run(
        &sans_font,
        "CURRENT SLIDE",
        16.0,
        margin + 4.0,
        main_top + 20.0,
        text_muted,
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
    if pw > 0 && ph > 0 {
        let mut slide_buf = vec![0u32; pw as usize * ph as usize];
        if let Some(slide) = session.current_slide() {
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

        // Blit current slide preview
        blit_sub_buffer(canvas.pixels_mut(), width, height, slide_x as u32, slide_y as u32, &slide_buf, pw, ph);
    }

    // Right side:
    // Top Right: Next slide preview (42% width)
    let right_x = margin + left_w + margin;
    let right_w = w - right_x - margin;
    let next_h = main_h * 0.42;

    draw_text_run(
        &sans_font,
        "NEXT SLIDE",
        16.0,
        right_x + 4.0,
        main_top + 20.0,
        text_muted,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    let mut next_w = right_w;
    let mut next_h_fit = right_w / deck_ratio;
    if next_h_fit > next_h - 30.0 {
        next_h_fit = next_h - 30.0;
        next_w = next_h_fit * deck_ratio;
    }
    let next_x_pos = right_x + (right_w - next_w) / 2.0;
    let next_y_pos = main_top + 32.0;

    let next_frame = Rect::new(next_x_pos - 2.0, next_y_pos - 2.0, next_w + 4.0, next_h_fit + 4.0);
    canvas.stroke_round_rect(next_frame, 4.0, 1.5, border_color);

    let npw = next_w as u32;
    let nph = next_h_fit as u32;
    if npw > 0 && nph > 0 {
        let mut next_buf = vec![0u32; npw as usize * nph as usize];
        if let Some(next_slide) = session.next_slide_preview() {
            render_slide(
                next_slide,
                &session.deck.theme,
                session.deck.aspect_ratio,
                &mut next_buf,
                npw,
                nph,
            );
        } else {
            next_buf.fill(card_bg.premultiplied());
        }
        blit_sub_buffer(canvas.pixels_mut(), width, height, next_x_pos as u32, next_y_pos as u32, &next_buf, npw, nph);
    }

    // Bottom Right: Speaker Notes panel
    let notes_top = next_y_pos + next_h_fit + 20.0;
    let notes_h = main_top + main_h - notes_top;
    let notes_rect = Rect::new(right_x, notes_top, right_w, notes_h);
    canvas.fill_round_rect(notes_rect, 6.0, card_bg);
    canvas.stroke_round_rect(notes_rect, 6.0, 1.5, border_color);

    draw_text_run(
        &sans_font,
        "SPEAKER NOTES",
        16.0,
        right_x + 12.0,
        notes_top + 26.0,
        accent_gold,
        true,
        false,
        canvas.pixels_mut(),
        width,
        height,
    );

    // Wrapped speaker notes text
    if let Some(slide) = session.current_slide() {
        let notes_content = if slide.speaker_notes.is_empty() {
            "(No speaker notes for this slide)"
        } else {
            slide.speaker_notes.as_str()
        };

        let mut note_y = notes_top + 52.0;
        let line_spacing = 22.0;
        for line in notes_content.lines() {
            if note_y > notes_rect.bottom() - 16.0 {
                break;
            }
            draw_text_run(
                &sans_font,
                line,
                16.0,
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
    canvas.line(0.0, bottom_y, w, bottom_y, 1.0, border_color);

    let nav_hint = "[←/→] Prev/Next   [Home/End] First/Last   [B] Black Screen   [W] White Screen   [Space] Advance";
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
