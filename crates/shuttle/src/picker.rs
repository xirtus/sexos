// SPDX-License-Identifier: GPL-3.0-only
//! Shared font picker state machine and self-contained software UI renderer.
//!
//! Provides a complete, toolkit-agnostic font selection widget shared by SexOS apps
//! (Weft, egui, Quil, Jot, or raw windows). Handles keyboard and mouse navigation,
//! search filtering, font family selection, size presets, live multilingual specimen
//! preview (`AaBbCc 0123 · 天地玄黄 · السلام عليكم · שלום עולם · 🚀✨`), and blits
//! directly into any 32-bit `0xAARRGGBB` pixel buffer.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::catalog::{FontCatalog, FontEntry, FontFilter};
use crate::layout::{layout_paragraph, LayoutOptions};
use crate::shaper::Shaper;

pub const PRESET_SIZES: [f32; 11] = [9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 24.0, 32.0, 48.0, 72.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Backspace,
    Char(char),
    Enter,
    Escape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontSelection {
    pub family: String,
    pub subfamily: String,
    pub weight: u16,
    pub size: f32,
    pub font_data: &'static [u8],
}

pub struct FontPicker {
    catalog: FontCatalog,
    filter_query: String,
    selected_index: usize,
    selected_size: f32,
    scroll_offset: usize,
    specimen: String,
    is_confirmed: bool,
    is_cancelled: bool,
    monospace_only: bool,
}

impl Default for FontPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl FontPicker {
    pub fn new() -> Self {
        Self {
            catalog: FontCatalog::new(),
            filter_query: String::new(),
            selected_index: 0,
            selected_size: 16.0,
            scroll_offset: 0,
            specimen: String::from("AaBbCc 0123 · 天地玄黄 · السلام عليكم · שלום עולם · 🚀✨"),
            is_confirmed: false,
            is_cancelled: false,
            monospace_only: false,
        }
    }

    pub fn set_monospace_only(&mut self, mono: bool) {
        self.monospace_only = mono;
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    pub fn set_selected_size(&mut self, size: f32) {
        self.selected_size = size.clamp(6.0, 120.0);
    }

    pub fn selected_size(&self) -> f32 {
        self.selected_size
    }

    pub fn set_specimen(&mut self, text: &str) {
        self.specimen = String::from(text);
    }

    pub fn is_confirmed(&self) -> bool {
        self.is_confirmed
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled
    }

    pub fn reset_actions(&mut self) {
        self.is_confirmed = false;
        self.is_cancelled = false;
    }

    pub fn filtered_entries(&self) -> Vec<&FontEntry> {
        let filter = FontFilter {
            query: if self.filter_query.is_empty() {
                None
            } else {
                Some(self.filter_query.clone())
            },
            monospace_only: self.monospace_only,
            ..Default::default()
        };
        self.catalog.query(&filter)
    }

    pub fn selected_entry(&self) -> Option<&FontEntry> {
        let entries = self.filtered_entries();
        entries.get(self.selected_index).copied()
    }

    pub fn selection(&self) -> Option<FontSelection> {
        let entry = self.selected_entry()?;
        Some(FontSelection {
            family: entry.family.clone(),
            subfamily: entry.subfamily.clone(),
            weight: entry.weight,
            size: self.selected_size,
            font_data: entry.data,
        })
    }

    pub fn handle_key(&mut self, key: PickerKey) -> bool {
        let entries_count = self.filtered_entries().len();

        match key {
            PickerKey::Up => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    if self.selected_index < self.scroll_offset {
                        self.scroll_offset = self.selected_index;
                    }
                    return true;
                }
            }
            PickerKey::Down => {
                if entries_count > 0 && self.selected_index + 1 < entries_count {
                    self.selected_index += 1;
                    if self.selected_index >= self.scroll_offset + 8 {
                        self.scroll_offset = self.selected_index - 7;
                    }
                    return true;
                }
            }
            PickerKey::PageUp => {
                self.selected_index = self.selected_index.saturating_sub(6);
                self.scroll_offset = self.scroll_offset.saturating_sub(6);
                return true;
            }
            PickerKey::PageDown => {
                if entries_count > 0 {
                    self.selected_index = (self.selected_index + 6).min(entries_count - 1);
                    if self.selected_index >= self.scroll_offset + 8 {
                        self.scroll_offset = self.selected_index - 7;
                    }
                    return true;
                }
            }
            PickerKey::Home => {
                self.selected_index = 0;
                self.scroll_offset = 0;
                return true;
            }
            PickerKey::End => {
                if entries_count > 0 {
                    self.selected_index = entries_count - 1;
                    self.scroll_offset = self.selected_index.saturating_sub(7);
                    return true;
                }
            }
            PickerKey::Backspace => {
                if !self.filter_query.is_empty() {
                    self.filter_query.pop();
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                    return true;
                }
            }
            PickerKey::Char(c) => {
                if !c.is_control() {
                    self.filter_query.push(c);
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                    return true;
                }
            }
            PickerKey::Enter => {
                self.is_confirmed = true;
                return true;
            }
            PickerKey::Escape => {
                self.is_cancelled = true;
                return true;
            }
        }

        false
    }

    pub fn handle_click(&mut self, x: i32, y: i32) -> bool {
        // UI Layout bounds:
        // Window: 480 x 360
        // Search bar: (16, 16, 448, 32)
        // Family list: (16, 60, 280, 160) - 8 items of height 20
        // Size presets: (312, 60, 152, 160)
        // Specimen box: (16, 232, 448, 72)
        // Buttons: Cancel (288, 316, 80, 32), OK (384, 316, 80, 32)

        // 1. Check family list click
        if (16..=296).contains(&x) && (60..=220).contains(&y) {
            let row = ((y - 60) / 20) as usize;
            let target_idx = self.scroll_offset + row;
            let entries_count = self.filtered_entries().len();
            if target_idx < entries_count {
                self.selected_index = target_idx;
                return true;
            }
        }

        // 2. Check size preset click
        if (312..=464).contains(&x) && (60..=220).contains(&y) {
            let col = if x < 388 { 0 } else { 1 };
            let row = ((y - 60) / 26) as usize;
            let preset_idx = row * 2 + col;
            if preset_idx < PRESET_SIZES.len() {
                self.selected_size = PRESET_SIZES[preset_idx];
                return true;
            }
        }

        // 3. Check OK button click
        if (384..=464).contains(&x) && (316..=348).contains(&y) {
            self.is_confirmed = true;
            return true;
        }

        // 4. Check Cancel button click
        if (288..=368).contains(&x) && (316..=348).contains(&y) {
            self.is_cancelled = true;
            return true;
        }

        false
    }

    pub fn handle_scroll(&mut self, delta: i32) -> bool {
        let entries_count = self.filtered_entries().len();
        if entries_count <= 8 {
            return false;
        }

        if delta > 0 && self.scroll_offset > 0 {
            self.scroll_offset -= 1;
            return true;
        } else if delta < 0 && self.scroll_offset + 8 < entries_count {
            self.scroll_offset += 1;
            return true;
        }

        false
    }

    /// Renders the complete font picker dialog into a 32-bit pixel buffer `target`.
    pub fn render(&self, target: &mut [u32], width: usize, height: usize) {
        let Some(need) = width.checked_mul(height) else { return; };
        if width < 100 || height < 100 || target.len() < need {
            return;
        }

        // Theme colors
        let bg_color = 0xFF181A20;
        let card_color = 0xFF222630;
        let border_color = 0xFF353C4D;
        let select_color = 0xFF2F4F7F;
        let text_color = 0xFFEDEDED;
        let text_dim = 0xFF9E9E9E;
        let ok_btn_color = 0xFF2563EB;
        let cancel_btn_color = 0xFF374151;

        // Clear background
        target.fill(bg_color);

        // Fill Search Bar rect (16, 16, width - 32, 32)
        fill_rect(target, width, height, 16, 16, width.saturating_sub(32), 32, card_color);
        stroke_rect(target, width, height, 16, 16, width.saturating_sub(32), 32, border_color);

        // Fill List Pane (16, 60, width * 6 / 10, 160)
        let list_w = (width * 6 / 10).saturating_sub(20);
        fill_rect(target, width, height, 16, 60, list_w, 160, card_color);
        stroke_rect(target, width, height, 16, 60, list_w, 160, border_color);

        // Fill Size Presets Pane (16 + list_w + 12, 60, width - (32 + list_w + 12), 160)
        let size_x = 16 + list_w + 12;
        let size_w = width.saturating_sub(size_x + 16);
        fill_rect(target, width, height, size_x, 60, size_w, 160, card_color);
        stroke_rect(target, width, height, size_x, 60, size_w, 160, border_color);

        // Fill Specimen Box (16, 230, width - 32, height.saturating_sub(280))
        let spec_y = 230;
        let spec_h = height.saturating_sub(spec_y + 48);
        fill_rect(target, width, height, 16, spec_y, width.saturating_sub(32), spec_h, 0xFF12141A);
        stroke_rect(target, width, height, 16, spec_y, width.saturating_sub(32), spec_h, border_color);

        // Fill OK & Cancel Buttons
        let btn_y = height.saturating_sub(40);
        let ok_x = width.saturating_sub(96);
        let cancel_x = width.saturating_sub(184);
        fill_rect(target, width, height, cancel_x, btn_y, 80, 30, cancel_btn_color);
        fill_rect(target, width, height, ok_x, btn_y, 80, 30, ok_btn_color);

        // Draw font rows
        let entries = self.filtered_entries();
        let visible_count = 8.min(entries.len().saturating_sub(self.scroll_offset));

        let ui_shaper = Shaper::default();

        for i in 0..visible_count {
            let entry_idx = self.scroll_offset + i;
            let entry = entries[entry_idx];
            let row_y = 62 + i * 19;
            let is_selected = entry_idx == self.selected_index;

            if is_selected {
                fill_rect(target, width, height, 18, row_y, list_w.saturating_sub(4), 18, select_color);
            }

            // Draw entry name
            let entry_layout = layout_paragraph(
                &ui_shaper,
                &entry.name,
                &LayoutOptions {
                    font_size: 13.0,
                    ..Default::default()
                },
            );
            entry_layout.draw(
                &ui_shaper,
                target,
                width,
                width,
                height,
                24,
                row_y as i32 + 2,
                if is_selected { 0xFFFFFFFF } else { text_color },
            );
        }

        // Draw active search text or placeholder
        let search_text = if self.filter_query.is_empty() {
            "Search fonts... (type to filter)"
        } else {
            &self.filter_query
        };
        let search_layout = layout_paragraph(
            &ui_shaper,
            search_text,
            &LayoutOptions {
                font_size: 14.0,
                ..Default::default()
            },
        );
        search_layout.draw(
            &ui_shaper,
            target,
            width,
            width,
            height,
            24,
            24,
            if self.filter_query.is_empty() { text_dim } else { text_color },
        );

        // Draw live specimen using chosen font
        if let Some(entry) = self.selected_entry() {
            let specimen_shaper = Shaper::new(entry.data);
            let specimen_layout = layout_paragraph(
                &specimen_shaper,
                &self.specimen,
                &LayoutOptions {
                    font_size: self.selected_size,
                    max_width: Some(width.saturating_sub(48) as f32),
                    ..Default::default()
                },
            );
            specimen_layout.draw(
                &specimen_shaper,
                target,
                width,
                width,
                height,
                24,
                spec_y as i32 + 8,
                text_color,
            );
        }

        // Button labels
        let cancel_layout = layout_paragraph(&ui_shaper, "Cancel", &LayoutOptions { font_size: 13.0, ..Default::default() });
        cancel_layout.draw(&ui_shaper, target, width, width, height, cancel_x as i32 + 18, btn_y as i32 + 6, text_color);

        let ok_layout = layout_paragraph(&ui_shaper, "OK", &LayoutOptions { font_size: 13.0, ..Default::default() });
        ok_layout.draw(&ui_shaper, target, width, width, height, ok_x as i32 + 28, btn_y as i32 + 6, 0xFFFFFFFF);
    }
}

fn fill_rect(target: &mut [u32], stride: usize, height: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    if w == 0 || h == 0 || stride == 0 || height == 0 {
        return;
    }
    let x1 = x.saturating_add(w).min(stride);
    let y1 = y.saturating_add(h).min(height);
    for row in y..y1 {
        let Some(r_start) = row.checked_mul(stride) else { continue; };
        for col in x..x1 {
            if let Some(d_idx) = r_start.checked_add(col) {
                if let Some(px) = target.get_mut(d_idx) {
                    *px = color;
                }
            }
        }
    }
}

fn stroke_rect(target: &mut [u32], stride: usize, height: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    if w == 0 || h == 0 || stride == 0 || height == 0 {
        return;
    }
    let x1 = x.saturating_add(w).saturating_sub(1).min(stride.saturating_sub(1));
    let y1 = y.saturating_add(h).saturating_sub(1).min(height.saturating_sub(1));
    if x > x1 || y > y1 {
        return;
    }

    for col in x..=x1 {
        if let Some(idx) = y.checked_mul(stride).and_then(|r| r.checked_add(col)) {
            if let Some(px) = target.get_mut(idx) {
                *px = color;
            }
        }
        if let Some(idx) = y1.checked_mul(stride).and_then(|r| r.checked_add(col)) {
            if let Some(px) = target.get_mut(idx) {
                *px = color;
            }
        }
    }
    for row in y..=y1 {
        if let Some(idx) = row.checked_mul(stride).and_then(|r| r.checked_add(x)) {
            if let Some(px) = target.get_mut(idx) {
                *px = color;
            }
        }
        if let Some(idx) = row.checked_mul(stride).and_then(|r| r.checked_add(x1)) {
            if let Some(px) = target.get_mut(idx) {
                *px = color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_picker_navigation_and_selection() {
        let mut picker = FontPicker::new();
        assert!(!picker.filtered_entries().is_empty());
        let initial_fam = picker.selected_entry().unwrap().name.clone();

        picker.handle_key(PickerKey::Down);
        let next_fam = picker.selected_entry().unwrap().name.clone();
        assert_ne!(initial_fam, next_fam);

        picker.handle_key(PickerKey::Enter);
        assert!(picker.is_confirmed());
    }

    #[test]
    fn test_picker_render() {
        let picker = FontPicker::new();
        let mut target = vec![0u32; 480 * 360];
        picker.render(&mut target, 480, 360);
        let non_zero = target.iter().filter(|&&px| px != 0).count();
        assert!(non_zero > 1000, "Picker rendered pixels must be populated");
    }
}
