// SPDX-License-Identifier: GPL-3.0-only
//! Under-Slide Build Timeline & Light-Table multi-slide operations.
//!
//! Provides:
//! - Ordered `BuildTimeline` per slide (`Trigger::{OnClick, WithPrevious, AfterPrevious}`,
//!   `BuildEffect::{Appear, Fade, FlyIn, Scale, Wipe, Disappear}`, `delay_ms`, `duration_ms`)
//! - Build step add / remove / reorder / timing mutation helpers
//! - Light-Table multi-slide reorder, duplicate, skip (`hidden`), and delete operations
//! - Software renderers for the Light-Table grid and Under-Slide Build Timeline strip

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::{Canvas, Color, Rect};
use sex_font::{Font, GEIST_MONO, GEIST_SANS};

pub use crate::animation::{BuildEffect, BuildStep, BuildTrigger};
use crate::deck::{Deck, Slide};
use crate::elements::SlideElement;
use crate::render::{draw_text_run, render_slide};

/// Normalized trigger kind for the under-slide build timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    OnClick,
    WithPrevious,
    AfterPrevious,
}

impl Default for Trigger {
    fn default() -> Self {
        Self::OnClick
    }
}

impl Trigger {
    pub fn name(self) -> &'static str {
        match self {
            Self::OnClick => "On Click",
            Self::WithPrevious => "With Previous",
            Self::AfterPrevious => "After Previous",
        }
    }

    pub fn kind_str(self) -> &'static str {
        match self {
            Self::OnClick => "on_click",
            Self::WithPrevious => "with_previous",
            Self::AfterPrevious => "after_previous",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        let clean = s.trim();
        if clean.eq_ignore_ascii_case("with_previous") || clean.eq_ignore_ascii_case("withprevious") || clean.eq_ignore_ascii_case("with") {
            Self::WithPrevious
        } else if clean.eq_ignore_ascii_case("after_previous") || clean.eq_ignore_ascii_case("afterprevious") || clean.eq_ignore_ascii_case("after") {
            Self::AfterPrevious
        } else {
            Self::OnClick
        }
    }

    pub fn to_build_trigger(self, delay_ms: u32) -> BuildTrigger {
        match self {
            Self::OnClick => BuildTrigger::OnClick,
            Self::WithPrevious => BuildTrigger::WithPrevious,
            Self::AfterPrevious => BuildTrigger::AfterPrevious { delay_ms },
        }
    }
}

impl From<BuildTrigger> for Trigger {
    fn from(value: BuildTrigger) -> Self {
        match value {
            BuildTrigger::OnClick => Self::OnClick,
            BuildTrigger::WithPrevious => Self::WithPrevious,
            BuildTrigger::AfterPrevious { .. } => Self::AfterPrevious,
        }
    }
}

/// Single ordered build entry in a slide's under-slide build timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildTimelineItem {
    pub step_index: usize,
    pub element_index: usize,
    pub paragraph_index: Option<usize>,
    pub effect: BuildEffect,
    pub trigger: Trigger,
    pub delay_ms: u32,
    pub duration_ms: u32,
    /// Start offset in ms relative to the start of the active click group.
    pub group_start_ms: u32,
    /// End offset in ms relative to the start of the active click group.
    pub group_end_ms: u32,
    /// Cumulative start offset across the entire slide timeline.
    pub start_ms: u32,
    /// Cumulative end offset across the entire slide timeline.
    pub end_ms: u32,
    /// 0-based click group index (`OnClick` starts a new group).
    pub click_group: usize,
    pub label: String,
}

/// Computed ordered build timeline for a slide.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildTimeline {
    pub items: Vec<BuildTimelineItem>,
    pub click_group_count: usize,
    pub total_duration_ms: u32,
}

impl BuildTimeline {
    /// Compute the ordered build timeline for `slide`.
    pub fn from_slide(slide: &Slide) -> Self {
        let mut items = Vec::with_capacity(slide.builds.len());
        let mut click_group = 0usize;
        let mut group_base_ms = 0u32;
        let mut group_max_end = 0u32;
        let mut prev_group_start = 0u32;
        let mut prev_group_end = 0u32;

        for (step_idx, step) in slide.builds.iter().enumerate() {
            let trig: Trigger = step.trigger.into();
            let delay_ms = slide.build_delay_ms(step_idx);
            let duration_ms = step.duration_ms;

            let (g_start, g_end) = if step_idx == 0 {
                let s = delay_ms;
                let e = s.saturating_add(duration_ms);
                (s, e)
            } else {
                match trig {
                    Trigger::OnClick => {
                        click_group += 1;
                        group_base_ms = group_base_ms.saturating_add(group_max_end);
                        group_max_end = 0;
                        let s = delay_ms;
                        let e = s.saturating_add(duration_ms);
                        (s, e)
                    }
                    Trigger::WithPrevious => {
                        let s = prev_group_start.saturating_add(delay_ms);
                        let e = s.saturating_add(duration_ms);
                        (s, e)
                    }
                    Trigger::AfterPrevious => {
                        let s = prev_group_end.saturating_add(delay_ms);
                        let e = s.saturating_add(duration_ms);
                        (s, e)
                    }
                }
            };

            prev_group_start = g_start;
            prev_group_end = g_end;
            group_max_end = group_max_end.max(g_end);

            let start_ms = group_base_ms.saturating_add(g_start);
            let end_ms = group_base_ms.saturating_add(g_end);

            let label = describe_element_target(slide, step.element_index, step.paragraph_index);

            items.push(BuildTimelineItem {
                step_index: step_idx,
                element_index: step.element_index,
                paragraph_index: step.paragraph_index,
                effect: step.effect,
                trigger: trig,
                delay_ms,
                duration_ms,
                group_start_ms: g_start,
                group_end_ms: g_end,
                start_ms,
                end_ms,
                click_group,
                label,
            });
        }

        let click_group_count = if items.is_empty() { 0 } else { click_group + 1 };
        let total_duration_ms = group_base_ms.saturating_add(group_max_end);

        Self {
            items,
            click_group_count,
            total_duration_ms,
        }
    }

    pub fn items(&self) -> &[BuildTimelineItem] {
        &self.items
    }

    pub fn click_groups(&self) -> usize {
        self.click_group_count
    }

    pub fn total_duration_ms(&self) -> u32 {
        self.total_duration_ms
    }

    pub fn steps_in_group(&self, group: usize) -> Vec<&BuildTimelineItem> {
        self.items.iter().filter(|item| item.click_group == group).collect()
    }

    pub fn format_strip_summary(&self) -> String {
        if self.items.is_empty() {
            return String::from("Timeline: no builds");
        }
        let mut out = format!(
            "Timeline [{} steps, {} clicks, {}ms]: ",
            self.items.len(),
            self.click_group_count,
            self.total_duration_ms
        );
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                out.push_str(" | ");
            }
            out.push_str(&format!(
                "#{} {} ({}, +{}ms, {}ms)",
                item.step_index + 1,
                item.effect.name(),
                item.trigger.kind_str(),
                item.delay_ms,
                item.duration_ms
            ));
        }
        out
    }
}

fn describe_element_target(slide: &Slide, element_index: usize, paragraph_index: Option<usize>) -> String {
    let base = match slide.elements.get(element_index) {
        Some(SlideElement::TextBox(tb)) => {
            let txt = tb.plain_text();
            let trimmed = txt.trim();
            if trimmed.is_empty() {
                format!("TextBox {}", element_index)
            } else {
                let preview: String = trimmed.chars().take(18).collect();
                preview
            }
        }
        Some(SlideElement::Shape(_)) => format!("Shape {}", element_index),
        Some(SlideElement::Image(_)) => format!("Image {}", element_index),
        Some(SlideElement::Table(_)) => format!("Table {}", element_index),
        Some(SlideElement::Code(_)) => format!("Code {}", element_index),
        None => format!("Missing {}", element_index),
    };
    if let Some(p) = paragraph_index {
        format!("{} [p{}]", base, p)
    } else {
        base
    }
}

/// Append a build step with explicit trigger, delay, and duration to `slide`.
pub fn add_build_step(
    slide: &mut Slide,
    element_index: usize,
    effect: BuildEffect,
    trigger: Trigger,
    delay_ms: u32,
    duration_ms: u32,
) -> usize {
    let step = BuildStep::new(element_index, effect)
        .with_trigger(trigger.to_build_trigger(delay_ms))
        .with_duration(duration_ms);
    let idx = slide.builds.len();
    slide.builds.push(step);
    if delay_ms > 0 {
        slide.set_build_delay_ms(idx, delay_ms);
    }
    idx
}

/// Reorder a build step on `slide`, keeping `build_delays` synchronized.
pub fn move_build_step(slide: &mut Slide, from: usize, to: usize) -> bool {
    if from >= slide.builds.len() || to >= slide.builds.len() || from == to {
        return false;
    }
    let mut delays: Vec<u32> = (0..slide.builds.len()).map(|i| slide.build_delay_ms(i)).collect();
    let step = slide.builds.remove(from);
    slide.builds.insert(to, step);
    let d = delays.remove(from);
    delays.insert(to, d);
    slide.build_delays.clear();
    for (i, val) in delays.into_iter().enumerate() {
        if val > 0 {
            slide.build_delays.insert(i, val);
        }
    }
    true
}

/// Remove a build step from `slide`, keeping `build_delays` synchronized.
pub fn remove_build_step(slide: &mut Slide, index: usize) -> Option<BuildStep> {
    if index >= slide.builds.len() {
        return None;
    }
    let mut delays: Vec<u32> = (0..slide.builds.len()).map(|i| slide.build_delay_ms(i)).collect();
    let removed = slide.builds.remove(index);
    delays.remove(index);
    slide.build_delays.clear();
    for (i, val) in delays.into_iter().enumerate() {
        if val > 0 {
            slide.build_delays.insert(i, val);
        }
    }
    Some(removed)
}

/// Update trigger & delay on an existing build step.
pub fn set_build_trigger(slide: &mut Slide, index: usize, trigger: Trigger, delay_ms: u32) -> bool {
    let Some(step) = slide.builds.get_mut(index) else {
        return false;
    };
    step.trigger = trigger.to_build_trigger(delay_ms);
    slide.set_build_delay_ms(index, delay_ms);
    true
}

/// Update delay & duration on an existing build step.
pub fn set_build_timing(slide: &mut Slide, index: usize, delay_ms: u32, duration_ms: u32) -> bool {
    let Some(step) = slide.builds.get_mut(index) else {
        return false;
    };
    step.duration_ms = duration_ms;
    if let BuildTrigger::AfterPrevious { .. } = step.trigger {
        step.trigger = BuildTrigger::AfterPrevious { delay_ms };
    }
    slide.set_build_delay_ms(index, delay_ms);
    true
}

/// Update the visual effect on an existing build step.
pub fn set_build_effect(slide: &mut Slide, index: usize, effect: BuildEffect) -> bool {
    let Some(step) = slide.builds.get_mut(index) else {
        return false;
    };
    step.effect = effect;
    true
}

// ---------- Light-Table Multi-Slide Operations ----------

fn sanitize_unique_sorted(indices: &[usize], len: usize) -> Vec<usize> {
    let mut out: Vec<usize> = indices.iter().copied().filter(|&i| i < len).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Reorder one or more slides (`indices`) as a contiguous block inserted before `target_index`
/// (relative to the original slide array, clamped to `0..=deck.slides.len()`).
///
/// Returns the new slide indices of the moved slides.
pub fn reorder_slides(deck: &mut Deck, indices: &[usize], target_index: usize) -> Vec<usize> {
    let len = deck.slides.len();
    let selected = sanitize_unique_sorted(indices, len);
    if selected.is_empty() {
        return Vec::new();
    }
    let clamped_target = target_index.min(len);
    let removed_before = selected.iter().filter(|&&i| i < clamped_target).count();
    let insert_pos = clamped_target.saturating_sub(removed_before);

    let mut moved = Vec::with_capacity(selected.len());
    for &idx in selected.iter().rev() {
        moved.push(deck.slides.remove(idx));
    }
    moved.reverse();

    let count = moved.len();
    for (offset, slide) in moved.into_iter().enumerate() {
        deck.slides.insert(insert_pos + offset, slide);
    }

    (insert_pos..insert_pos + count).collect()
}

/// Duplicate one or more slides (`indices`) with fresh unique slide IDs.
/// Inserts the duplicated slides immediately after the last selected slide (preserving relative order).
///
/// Returns the indices of the newly created duplicate slides.
pub fn duplicate_slides(deck: &mut Deck, indices: &[usize]) -> Vec<usize> {
    let len = deck.slides.len();
    let selected = sanitize_unique_sorted(indices, len);
    if selected.is_empty() {
        return Vec::new();
    }

    let insert_at = selected.last().copied().unwrap_or(0) + 1;
    let copies: Vec<Slide> = selected.iter().map(|&idx| deck.slides[idx].clone()).collect();
    let mut new_indices = Vec::with_capacity(copies.len());

    for (offset, mut copy) in copies.into_iter().enumerate() {
        copy.id = 0; // force fresh ID assignment in insert_slide
        let pos = insert_at + offset;
        deck.insert_slide(pos, copy);
        new_indices.push(pos);
    }

    new_indices
}

/// Mark multiple slides as skipped (`hidden = true`) or active (`hidden = false`).
/// Returns the number of valid slide indices updated.
pub fn set_slides_skipped(deck: &mut Deck, indices: &[usize], skipped: bool) -> usize {
    let selected = sanitize_unique_sorted(indices, deck.slides.len());
    for &idx in &selected {
        deck.slides[idx].hidden = skipped;
    }
    selected.len()
}

/// Toggle skipped (`hidden`) state on multiple slides.
/// Returns the number of valid slide indices toggled.
pub fn toggle_slides_skipped(deck: &mut Deck, indices: &[usize]) -> usize {
    let selected = sanitize_unique_sorted(indices, deck.slides.len());
    for &idx in &selected {
        deck.slides[idx].hidden = !deck.slides[idx].hidden;
    }
    selected.len()
}

/// Delete multiple slides. When `keep_one` is true, guarantees at least 1 slide remains.
/// Returns the number of slides removed.
pub fn delete_slides(deck: &mut Deck, indices: &[usize], keep_one: bool) -> usize {
    let selected = sanitize_unique_sorted(indices, deck.slides.len());
    let mut removed = 0usize;
    for &idx in selected.iter().rev() {
        if keep_one && deck.slides.len() <= 1 {
            break;
        }
        deck.slides.remove(idx);
        removed += 1;
    }
    removed
}

/// Return indices of all non-skipped (`!slide.hidden`) slides.
pub fn visible_slide_indices(deck: &Deck) -> Vec<usize> {
    deck.slides
        .iter()
        .enumerate()
        .filter_map(|(i, s)| if !s.hidden { Some(i) } else { None })
        .collect()
}

/// Summary card for a slide in the Light-Table sorter view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightTableCard {
    pub slide_index: usize,
    pub slide_id: u64,
    pub title: String,
    pub skipped: bool,
    pub transition_name: &'static str,
    pub build_count: usize,
    pub element_count: usize,
    pub has_notes: bool,
    pub dwell_ms: Option<u32>,
}

/// Build Light-Table metadata cards for all slides in `deck`.
pub fn light_table_cards(deck: &Deck) -> Vec<LightTableCard> {
    deck.slides
        .iter()
        .enumerate()
        .map(|(i, s)| LightTableCard {
            slide_index: i,
            slide_id: s.id,
            title: if s.title.is_empty() {
                format!("Slide {}", i + 1)
            } else {
                s.title.clone()
            },
            skipped: s.hidden,
            transition_name: s.transition.name(),
            build_count: s.builds.len(),
            element_count: s.elements.len(),
            has_notes: !s.notes().is_empty(),
            dwell_ms: s.rehearsed_dwell_ms,
        })
        .collect()
}

/// Render a Keynote-grade Light-Table multi-slide grid onto a `0xAARRGGBB` pixel buffer.
pub fn render_light_table(
    deck: &Deck,
    selected_slides: &[usize],
    active_slide: usize,
    buffer: &mut [u32],
    width: u32,
    height: u32,
) {
    let needed = match (width as usize).checked_mul(height as usize) {
        Some(n) => n,
        None => return,
    };
    if width < 64 || height < 64 || buffer.len() < needed {
        return;
    }

    let mut canvas = match Canvas::new(buffer, width, height) {
        Some(c) => c,
        None => return,
    };

    let bg = Color::rgb(0x0F, 0x17, 0x2A);
    let card_bg = Color::rgb(0x1E, 0x29, 0x3B);
    let border_norm = Color::rgb(0x47, 0x55, 0x69);
    let border_active = Color::rgb(0xF5, 0x9E, 0x0B);
    let border_sel = Color::rgb(0x38, 0xBD, 0xF8);
    let skip_badge = Color::rgb(0xDC, 0x26, 0x26);
    let text_fg = Color::rgb(0xF8, 0xFA, 0xFC);
    let text_muted = Color::rgb(0x94, 0xA3, 0xB8);

    canvas.clear(bg);

    let sans = Font::new(GEIST_SANS);
    let w = width as f32;
    let h = height as f32;

    // Header strip
    let header_h = 28.0;
    canvas.fill_rect(Rect::new(0.0, 0.0, w, header_h), card_bg);
    if let Some(ref font) = sans {
        let header = format!(
            "LIGHT TABLE  ·  {} slides ({} active)",
            deck.slide_count(),
            visible_slide_indices(deck).len()
        );
        draw_text_run(
            font,
            &header,
            13.0,
            12.0,
            19.0,
            border_active,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    let n = deck.slides.len();
    if n == 0 {
        return;
    }

    let cols = if n <= 2 { 2 } else if n <= 6 { 3 } else { 4 };
    let rows = n.div_ceil(cols).max(1);
    let pad = 12.0;
    let avail_w = (w - pad * (cols as f32 + 1.0)).max(32.0);
    let avail_h = (h - header_h - pad * (rows as f32 + 1.0)).max(32.0);
    let cell_w = avail_w / cols as f32;
    let cell_h = avail_h / rows as f32;
    let ratio = deck.aspect_ratio.ratio();

    let label_h = 20.0;
    let mut thumb_w = (cell_w - 8.0).max(16.0);
    let mut thumb_h = thumb_w / ratio;
    if thumb_h > cell_h - label_h - 8.0 {
        thumb_h = (cell_h - label_h - 8.0).max(12.0);
        thumb_w = thumb_h * ratio;
    }

    for (idx, slide) in deck.slides.iter().enumerate() {
        let c = idx % cols;
        let r = idx / cols;
        let cx = pad + c as f32 * (cell_w + pad);
        let cy = header_h + pad + r as f32 * (cell_h + pad);
        if cy + cell_h > h + 1.0 {
            break;
        }

        let is_active = idx == active_slide;
        let is_selected = selected_slides.contains(&idx);
        let card_rect = Rect::new(cx, cy, cell_w, cell_h);
        canvas.fill_round_rect(card_rect, 6.0, card_bg);
        let bcol = if is_active {
            border_active
        } else if is_selected {
            border_sel
        } else {
            border_norm
        };
        canvas.stroke_round_rect(card_rect, 6.0, if is_active { 2.5 } else { 1.5 }, bcol);

        let tx = cx + (cell_w - thumb_w) * 0.5;
        let ty = cy + 4.0;
        let tw = (thumb_w as u32).max(8);
        let th = (thumb_h as u32).max(8);

        if let Some(tlen) = (tw as usize).checked_mul(th as usize) {
            let mut tbuf = vec![0u32; tlen];
            render_slide(slide, &deck.theme, deck.aspect_ratio, &mut tbuf, tw, th);
            if slide.hidden {
                // Dim skipped slides by 55%
                for px in &mut tbuf {
                    let a = (*px >> 24) & 0xFF;
                    let r_ch = ((*px >> 16) & 0xFF) * 45 / 100;
                    let g_ch = ((*px >> 8) & 0xFF) * 45 / 100;
                    let b_ch = (*px & 0xFF) * 45 / 100;
                    *px = (a << 24) | (r_ch << 16) | (g_ch << 8) | b_ch;
                }
            }
            blit_rect(canvas.pixels_mut(), width, height, tx as u32, ty as u32, &tbuf, tw, th);
        }

        if slide.hidden {
            let badge_r = Rect::new(tx + 4.0, ty + 4.0, (thumb_w - 8.0).min(64.0), 16.0);
            canvas.fill_round_rect(badge_r, 3.0, skip_badge);
            if let Some(ref font) = sans {
                draw_text_run(
                    font,
                    "SKIP",
                    10.0,
                    badge_r.x + 6.0,
                    badge_r.y + 12.0,
                    Color::WHITE,
                    true,
                    false,
                    canvas.pixels_mut(),
                    width,
                    height,
                );
            }
        }

        if let Some(ref font) = sans {
            let title_preview: String = if slide.title.is_empty() {
                format!("{}. Untitled", idx + 1)
            } else {
                let short: String = slide.title.chars().take(16).collect();
                format!("{}. {}", idx + 1, short)
            };
            draw_text_run(
                font,
                &title_preview,
                11.0,
                cx + 6.0,
                cy + cell_h - 6.0,
                if slide.hidden { text_muted } else { text_fg },
                is_active,
                false,
                canvas.pixels_mut(),
                width,
                height,
            );
        }
    }
}

/// Render an Under-Slide Build Timeline strip at the bottom of a `buffer` (occupying `strip_h` pixels).
pub fn render_build_timeline_strip(
    slide: &Slide,
    active_step: usize,
    buffer: &mut [u32],
    width: u32,
    height: u32,
    strip_h: u32,
) {
    let needed = match (width as usize).checked_mul(height as usize) {
        Some(n) => n,
        None => return,
    };
    if width < 120 || height < strip_h || strip_h < 24 || buffer.len() < needed {
        return;
    }

    let mut canvas = match Canvas::new(buffer, width, height) {
        Some(c) => c,
        None => return,
    };

    let y0 = (height - strip_h) as f32;
    let w = width as f32;
    let h = strip_h as f32;

    canvas.fill_rect(Rect::new(0.0, y0, w, h), Color::rgba(0x0F, 0x17, 0x2A, 240));
    canvas.line(0.0, y0, w, y0, 1.5, Color::rgb(0x38, 0xBD, 0xF8));

    let tl = BuildTimeline::from_slide(slide);
    let sans = Font::new(GEIST_SANS);
    let mono = Font::new(GEIST_MONO);

    if tl.items.is_empty() {
        if let Some(ref font) = sans {
            draw_text_run(
                font,
                "BUILD TIMELINE: No build steps on this slide (Click 'Add Build')",
                12.0,
                12.0,
                y0 + h * 0.62,
                Color::rgb(0x94, 0xA3, 0xB8),
                false,
                false,
                canvas.pixels_mut(),
                width,
                height,
            );
        }
        return;
    }

    let total_ms = tl.total_duration_ms.max(1) as f32;
    let label_col_w = 132.0;
    let track_x = label_col_w + 8.0;
    let track_w = (w - track_x - 12.0).max(40.0);

    if let Some(ref font) = sans {
        let hdr = format!("BUILDS ({}/{})", active_step.min(tl.items.len()), tl.items.len());
        draw_text_run(
            font,
            &hdr,
            11.5,
            10.0,
            y0 + 18.0,
            Color::rgb(0xF5, 0x9E, 0x0B),
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }

    let max_lanes = tl.items.len().min(3);
    let lane_h = ((h - 8.0) / max_lanes as f32).max(12.0);

    for (idx, item) in tl.items.iter().take(max_lanes).enumerate() {
        let ly = y0 + 4.0 + idx as f32 * lane_h;
        let x_start = track_x + (item.start_ms as f32 / total_ms) * track_w;
        let bw = ((item.duration_ms.max(40) as f32 / total_ms) * track_w).max(36.0);
        let bar_rect = Rect::new(x_start, ly + 2.0, bw.min(w - x_start - 4.0), (lane_h - 4.0).max(8.0));

        let bar_color = if idx < active_step {
            Color::rgb(0x05, 0x96, 0x69)
        } else if idx == active_step {
            Color::rgb(0x38, 0xBD, 0xF8)
        } else {
            Color::rgb(0x47, 0x55, 0x69)
        };
        canvas.fill_round_rect(bar_rect, 4.0, bar_color);

        if let Some(ref font) = mono {
            let tag = format!("{} {}ms", item.effect.name(), item.duration_ms);
            draw_text_run(
                font,
                &tag,
                10.0,
                bar_rect.x + 4.0,
                bar_rect.y + bar_rect.h * 0.78,
                Color::WHITE,
                false,
                false,
                canvas.pixels_mut(),
                width,
                height,
            );
        }
    }
}

fn blit_rect(
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    ox: u32,
    oy: u32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    for y in 0..src_h {
        let ty = oy + y;
        if ty >= dst_h {
            break;
        }
        let d_row = ty as usize * dst_w as usize;
        let s_row = y as usize * src_w as usize;
        for x in 0..src_w {
            let tx = ox + x;
            if tx >= dst_w {
                break;
            }
            dst[d_row + tx as usize] = src[s_row + x as usize];
        }
    }
}
