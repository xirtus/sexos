// SPDX-License-Identifier: GPL-3.0-only
//! Tapestry Channel Rack, Multi-Channel Pattern Matrix, Linked Timeline Instances,
//! Scene Launcher, Retrospective Note Recovery, and Audio Baking Integration.

extern crate alloc;

use crate::MAX_TRACKS;
use alloc::{format, string::String, sync::Arc, vec, vec::Vec};
use moan_core::{
    compose::ExpressiveNote,
    instruments::{self, InstrumentParams},
    kits::{self, ChannelTemplate, KitSpec, SongStarterSpec, ALL_KITS, ALL_SONG_STARTERS},
    midi::{self, Instrument},
    pattern::{
        ChannelId, ChannelPreviewMode, PatternId, RealizedChannelNote, SceneMatrix,
        TapestryBank, TapestryPattern, TransportContext,
    },
    time::Time,
    Clip, Frame, Track,
};
use weft::{Id, Layout, TextStyle, Ui};

pub const RACK_WIDTH: usize = 640;
pub const RACK_HEIGHT: usize = 210;
pub const HEADER_H: usize = 22;
pub const ROW_H: usize = 23;
pub const GRID_X: usize = 192;
pub const GRID_W: usize = RACK_WIDTH - GRID_X;
pub const MAX_VISIBLE_ROWS: usize = 8;

const DEFAULT_CHANNEL_COLORS: [u32; 8] = [
    0xe05d44, 0xe69f38, 0xd8b848, 0x48b5a3, 0x66c7b4, 0x9b5de5, 0x5c80d6, 0xc76b8f,
];

/// Map a kit `ChannelTemplate` to Moan's built-in engine table (`0..=11`).
pub fn template_instrument(ch: &ChannelTemplate) -> u8 {
    if ch.legato && ch.instrument == kits::INSTR_SATIN_PULSE {
        Instrument::Sub808 as u8
    } else if ch.instrument == kits::INSTR_VELVET_SAMPLER {
        Instrument::DrumKit as u8
    } else if ch.name.contains("Tine") {
        Instrument::TineKeys as u8
    } else if ch.name.contains("Guitar") {
        Instrument::PluckGuitar as u8
    } else if ch.name.contains("Mallet") {
        Instrument::ModalMallet as u8
    } else if ch.name.contains("Cross") {
        Instrument::CrossSynth as u8
    } else {
        ch.instrument
    }
}

/// Weft UI handles and view state for `Workspace::Tapestry`.
pub struct TapestryUi {
    pub panel: Id,
    pub mode_btn: Id,
    pub pattern_btn: Id,
    pub new_pat_btn: Id,
    pub clone_pat_btn: Id,
    pub instance_btn: Id,
    pub make_unique_btn: Id,
    pub split_btn: Id,
    pub swing_label: Id,
    pub swing_slider: Id,
    pub kit_btn: Id,
    pub add_kit_btn: Id,
    pub starter_btn: Id,
    pub load_starter_btn: Id,
    pub paint_btn: Id,
    pub canvas: Id,
    pub scene_btn: Id,
    pub queue_scene_btn: Id,
    pub advance_bar_btn: Id,
    pub recover_riff_btn: Id,
    pub import_midi_btn: Id,
    pub export_midi_btn: Id,
    pub info_label: Id,

    pub selected_kit_idx: usize,
    pub selected_starter_idx: usize,
    pub selected_instance_idx: usize,
    pub selected_scene_idx: usize,
    pub selected_channel_idx: usize,
    pub scene_clock_frame: u64,
    pub channel_colors: Vec<u32>,
}

impl TapestryUi {
    pub fn build(ui: &mut Ui, parent: Id) -> Self {
        let panel = ui.panel(
            parent,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );

        let header = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        ui.text(header, "Tapestry · Channel Rack", TextStyle::Heading);
        let mode_btn = ui.button(header, "Mode: Pattern");
        let pattern_btn = ui.button(header, "Pat: None");
        let new_pat_btn = ui.button(header, "New Pattern");
        let clone_pat_btn = ui.button(header, "Clone Pattern");
        let instance_btn = ui.button(header, "Inst: None");
        let make_unique_btn = ui.button(header, "Make Unique");
        let split_btn = ui.button(header, "Split Channels");
        let swing_label = ui.text(header, "Swing 0%", TextStyle::Caption);
        let swing_slider = ui.slider(header, 0.0, 75.0, 0.0);
        ui.set_layout(swing_slider, Layout::default().width(90.0));
        ui.set_step(swing_slider, 1.0);

        let starters_row = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        let kit_name = ALL_KITS.first().map_or("Kit", |k| k.name);
        let starter_name = ALL_SONG_STARTERS.first().map_or("Starter", |s| s.name);
        let kit_btn = ui.button(starters_row, &format!("Kit: {}", kit_name));
        let add_kit_btn = ui.button(starters_row, "Add Kit");
        let starter_btn = ui.button(starters_row, &format!("Starter: {}", starter_name));
        let load_starter_btn = ui.button(starters_row, "Load Starter");
        let paint_btn = ui.button(starters_row, "Paint Pattern to Arrange");

        let empty_bank = TapestryBank::new();
        let pixels = Self::render_rack(&empty_bank, 0, &[]);
        let canvas = ui.image(panel, RACK_WIDTH, RACK_HEIGHT, pixels);
        ui.set_label(
            canvas,
            "Tapestry interactive Channel Rack with 4-beat step colors and mini Piano Roll previews",
        );

        let footer = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        let scene_btn = ui.button(footer, "Scene 1");
        let queue_scene_btn = ui.button(footer, "Queue Scene");
        let advance_bar_btn = ui.button(footer, "Advance Bar");
        let recover_riff_btn = ui.button(footer, "Recover Riff");
        let import_midi_btn = ui.button(footer, "Import MIDI");
        let export_midi_btn = ui.button(footer, "Export MIDI");
        let info_label = ui.text(footer, "Ready · Load a kit or song starter", TextStyle::Caption);

        Self {
            panel,
            mode_btn,
            pattern_btn,
            new_pat_btn,
            clone_pat_btn,
            instance_btn,
            make_unique_btn,
            split_btn,
            swing_label,
            swing_slider,
            kit_btn,
            add_kit_btn,
            starter_btn,
            load_starter_btn,
            paint_btn,
            canvas,
            scene_btn,
            queue_scene_btn,
            advance_bar_btn,
            recover_riff_btn,
            import_midi_btn,
            export_midi_btn,
            info_label,

            selected_kit_idx: 0,
            selected_starter_idx: 0,
            selected_instance_idx: 0,
            selected_scene_idx: 0,
            selected_channel_idx: 0,
            scene_clock_frame: 0,
            channel_colors: Vec::new(),
        }
    }

    pub fn active_kit(&self) -> &'static KitSpec {
        &ALL_KITS[self.selected_kit_idx % ALL_KITS.len()]
    }

    pub fn cycle_kit(&mut self, ui: &mut Ui) -> &'static KitSpec {
        self.selected_kit_idx = (self.selected_kit_idx + 1) % ALL_KITS.len();
        let kit = self.active_kit();
        ui.set_text(self.kit_btn, &format!("Kit: {}", kit.name));
        kit
    }

    pub fn select_kit_by_name(&mut self, ui: &mut Ui, name: &str) -> bool {
        if let Some(pos) = ALL_KITS.iter().position(|k| k.name == name) {
            self.selected_kit_idx = pos;
            ui.set_text(self.kit_btn, &format!("Kit: {}", ALL_KITS[pos].name));
            true
        } else {
            false
        }
    }

    pub fn active_starter(&self) -> &'static SongStarterSpec {
        &ALL_SONG_STARTERS[self.selected_starter_idx % ALL_SONG_STARTERS.len()]
    }

    pub fn cycle_starter(&mut self, ui: &mut Ui) -> &'static SongStarterSpec {
        self.selected_starter_idx = (self.selected_starter_idx + 1) % ALL_SONG_STARTERS.len();
        let s = self.active_starter();
        ui.set_text(self.starter_btn, &format!("Starter: {}", s.name));
        s
    }

    pub fn select_starter_by_name(&mut self, ui: &mut Ui, name: &str) -> bool {
        if let Some(pos) = ALL_SONG_STARTERS.iter().position(|s| s.name == name) {
            self.selected_starter_idx = pos;
            ui.set_text(
                self.starter_btn,
                &format!("Starter: {}", ALL_SONG_STARTERS[pos].name),
            );
            true
        } else {
            false
        }
    }

    pub fn cycle_scene(&mut self, ui: &mut Ui) -> usize {
        self.selected_scene_idx = (self.selected_scene_idx + 1) % moan_core::pattern::SCENE_COUNT;
        ui.set_text(self.scene_btn, &format!("Scene {}", self.selected_scene_idx + 1));
        self.selected_scene_idx
    }

    pub fn cycle_pattern(&mut self, bank: &mut TapestryBank) {
        if bank.patterns.is_empty() {
            return;
        }
        let pos = bank
            .patterns
            .iter()
            .position(|p| p.id == bank.active_pattern)
            .unwrap_or(0);
        let next = (pos + 1) % bank.patterns.len();
        bank.active_pattern = bank.patterns[next].id;
    }

    pub fn cycle_instance(&mut self, bank: &TapestryBank) {
        if bank.instances.is_empty() {
            self.selected_instance_idx = 0;
        } else {
            self.selected_instance_idx = (self.selected_instance_idx + 1) % bank.instances.len();
        }
    }

    pub fn refresh(&mut self, ui: &mut Ui, bank: &TapestryBank, scenes: &SceneMatrix) {
        ui.set_text(
            self.mode_btn,
            match bank.transport_mode {
                TransportContext::PatternMode => "Mode: Pattern",
                TransportContext::SongMode => "Mode: Song",
            },
        );
        if let Some(pat) = bank.pattern(bank.active_pattern) {
            let pat_idx = bank
                .patterns
                .iter()
                .position(|p| p.id == pat.id)
                .map_or(0, |i| i + 1);
            ui.set_text(
                self.pattern_btn,
                &format!("Pat {}/{}: {}", pat_idx, bank.patterns.len(), pat.name),
            );
        } else {
            ui.set_text(self.pattern_btn, "Pat: None");
        }
        if bank.instances.is_empty() {
            self.selected_instance_idx = 0;
            ui.set_text(self.instance_btn, "Inst: None");
        } else {
            if self.selected_instance_idx >= bank.instances.len() {
                self.selected_instance_idx = bank.instances.len() - 1;
            }
            let inst = bank.instances[self.selected_instance_idx];
            ui.set_text(
                self.instance_btn,
                &format!(
                    "Inst {}/{} (P{})",
                    self.selected_instance_idx + 1,
                    bank.instances.len(),
                    inst.pattern.0 + 1
                ),
            );
        }
        ui.set_value(self.swing_slider, bank.swing_pct as f32);
        ui.set_text(self.swing_label, &format!("Swing {}%", bank.swing_pct));
        ui.set_text(self.scene_btn, &format!("Scene {}", self.selected_scene_idx + 1));

        let bpm = bank.tempo_map.bpm_x100_at(Time::ZERO) as f32 / 100.0;
        let scene_info = if let Some(q) = scenes.queued {
            format!("Queued S{}", q.scene_idx + 1)
        } else if let Some(a) = scenes.active_scene {
            format!("Active S{}", a + 1)
        } else {
            String::from("No Scene")
        };
        ui.set_text(
            self.info_label,
            &format!(
                "{} ch · {} pat · {} inst · {:.0} BPM · {}",
                bank.channels.len(),
                bank.patterns.len(),
                bank.instances.len(),
                bpm,
                scene_info
            ),
        );

        let pixels = Self::render_rack(bank, self.selected_channel_idx, &self.channel_colors);
        ui.set_image(self.canvas, RACK_WIDTH, RACK_HEIGHT, pixels);
    }

    /// Add all channels from `kit` into `bank` (and update `channel_colors`).
    pub fn apply_kit_to_bank(
        &mut self,
        kit: &KitSpec,
        bank: &mut TapestryBank,
    ) -> Result<(), &'static str> {
        let base_idx = bank.channels.len();
        for (i, tmpl) in kit.channels.iter().enumerate() {
            if bank.channels.len() >= moan_core::pattern::MAX_RACK_CHANNELS {
                break;
            }
            let inst = template_instrument(tmpl);
            let id = bank
                .add_channel(tmpl.name, inst)
                .map_err(|_| "Channel limit reached")?;
            if let Some(ch) = bank.channel_mut(id) {
                ch.root_key = tmpl.root_key;
                ch.volume_pct = tmpl.volume_pct as u16;
                ch.pan = tmpl.pan;
                ch.cut_group = tmpl.cut_group;
                ch.target_track = tmpl.mixer_strip.saturating_sub(1) % MAX_TRACKS as u8;
            }
            let idx = base_idx + i;
            if self.channel_colors.len() <= idx {
                self.channel_colors.resize(idx + 1, tmpl.color_rgb);
            } else {
                self.channel_colors[idx] = tmpl.color_rgb;
            }
        }
        if bank.patterns.is_empty() {
            let pid = bank
                .create_pattern(kit.name, 1, 4)
                .map_err(|_| "Pattern creation failed")?;
            bank.active_pattern = pid;
        }
        let bpm_x100 = (kit.default_bpm as u32 * 100).clamp(1000, 99900);
        if let Ok(tm) = moan_core::time::TempoMap::constant(bpm_x100) {
            bank.tempo_map = tm;
        }
        Ok(())
    }

    /// Populate `bank` and `scenes` from a `SongStarterSpec`.
    pub fn apply_starter_to_bank(
        &mut self,
        starter: &SongStarterSpec,
        bank: &mut TapestryBank,
        scenes: &mut SceneMatrix,
    ) -> Result<(), &'static str> {
        let mut next_bank = TapestryBank::new();
        let sig = moan_core::time::TimeSignature::new(
            starter.time_signature[0],
            starter.time_signature[1],
        )
        .unwrap_or_default();
        let bpm_x100 = (starter.bpm as u32 * 100).clamp(1000, 99900);
        let seg = moan_core::time::TempoSegment {
            start: Time::ZERO,
            bpm_x100,
            ramp_to_bpm_x100: None,
        };
        next_bank.tempo_map = moan_core::time::TempoMap::new(vec![seg], sig)
            .map_err(|_| "Invalid starter tempo")?;
        next_bank.swing_pct = starter.swing_pct.min(moan_core::time::MAX_SWING_PCT);

        self.channel_colors.clear();
        let mut channel_ids = Vec::new();
        for tmpl in starter.channels {
            let inst = template_instrument(tmpl);
            let ch_id = next_bank
                .add_channel(tmpl.name, inst)
                .map_err(|_| "Too many starter channels")?;
            if let Some(ch) = next_bank.channel_mut(ch_id) {
                ch.root_key = tmpl.root_key;
                ch.volume_pct = tmpl.volume_pct as u16;
                ch.pan = tmpl.pan;
                ch.cut_group = tmpl.cut_group;
                ch.target_track = tmpl.mixer_strip.saturating_sub(1) % MAX_TRACKS as u8;
            }
            self.channel_colors.push(tmpl.color_rgb);
            channel_ids.push(ch_id);
        }

        let pat_id = next_bank
            .create_pattern(starter.name, starter.bars, starter.steps_per_beat)
            .map_err(|_| "Invalid starter pattern")?;
        let step_len = Time::new(1, starter.steps_per_beat.max(1) as u32).unwrap_or(Time::SIXTEENTH);

        if let Some(pat) = next_bank.pattern_mut(pat_id) {
            for sn in starter.notes {
                let Some(&ch_id) = channel_ids.get(sn.channel_idx as usize) else {
                    continue;
                };
                let start = step_len
                    .mul_int(sn.step as i64)
                    .ok_or("Step timing overflow")?;
                let length = step_len
                    .mul_int(sn.length_steps.max(1) as i64)
                    .ok_or("Step length overflow")?;
                let mut note = ExpressiveNote::new(0, start, length, sn.key, sn.velocity);
                if sn.is_slide {
                    note.slide_to_key = Some(sn.key);
                }
                pat.add_note(ch_id, note)
                    .map_err(|_| "Failed adding starter note")?;
            }
        }
        next_bank.active_pattern = pat_id;
        next_bank.transport_mode = TransportContext::PatternMode;
        *bank = next_bank;

        *scenes = SceneMatrix::new();
        let _ = scenes.set_slot(0, 0, Some(pat_id));
        self.selected_instance_idx = 0;
        self.selected_channel_idx = 0;
        self.scene_clock_frame = 0;
        Ok(())
    }

    /// Map a canvas pixel `(x, y)` in `0..RACK_WIDTH, 0..RACK_HEIGHT` to `(channel_id, step_idx, root_key)`.
    pub fn hit_test_step(x: usize, y: usize, bank: &TapestryBank) -> Option<(ChannelId, usize, u8)> {
        if x < GRID_X || x >= RACK_WIDTH || y < HEADER_H || y >= RACK_HEIGHT {
            return None;
        }
        let row = (y - HEADER_H) / ROW_H;
        let ch = bank.channels.get(row)?;
        let pat = bank.pattern(bank.active_pattern)?;
        let sig = bank.tempo_map.time_signature_at(Time::ZERO);
        let total_steps = pat.total_steps(sig).max(1);
        let step_idx = ((x - GRID_X) * total_steps / GRID_W).min(total_steps - 1);
        Some((ch.id, step_idx, ch.root_key))
    }

    /// Map a canvas pixel `(x, y)` inside the mute/solo indicator dot to `ChannelId`.
    pub fn hit_test_mute(x: usize, y: usize, bank: &TapestryBank) -> Option<ChannelId> {
        if !(8..=22).contains(&x) || y < HEADER_H || y >= RACK_HEIGHT {
            return None;
        }
        let row = (y - HEADER_H) / ROW_H;
        bank.channels.get(row).map(|c| c.id)
    }

    /// Return exact layout-space click point for `(row_idx, step_idx)` inside `canvas_rect`.
    pub fn step_point(
        canvas_rect: weft::Rect,
        row_idx: usize,
        step_idx: usize,
        total_steps: usize,
    ) -> (f32, f32) {
        let steps = total_steps.max(1);
        let cx = GRID_X as f32 + (step_idx as f32 + 0.5) * GRID_W as f32 / steps as f32;
        let cy = HEADER_H as f32 + (row_idx as f32 + 0.5) * ROW_H as f32;
        (
            canvas_rect.x + cx * canvas_rect.w / RACK_WIDTH as f32,
            canvas_rect.y + cy * canvas_rect.h / RACK_HEIGHT as f32,
        )
    }

    /// Render the interactive `640x210` Tapestry Channel Rack image.
    pub fn render_rack(
        bank: &TapestryBank,
        selected_channel: usize,
        colors: &[u32],
    ) -> Vec<u32> {
        let w = RACK_WIDTH;
        let h = RACK_HEIGHT;
        let mut pixels = vec![0x141a24u32; w * h];

        let pat = bank.pattern(bank.active_pattern);
        let sig = bank.tempo_map.time_signature_at(Time::ZERO);
        let total_steps = pat.map_or(16, |p| p.total_steps(sig)).max(1);
        let spb = pat.map_or(4, |p| p.steps_per_beat);

        // Header bar background
        for y in 0..HEADER_H {
            for x in 0..w {
                pixels[y * w + x] = 0x1d2534;
            }
        }

        // Beat column divider ticks in header
        let total_beats = total_steps / spb.max(1) as usize;
        for b in 0..total_beats.max(1) {
            let bx = GRID_X + b * GRID_W / total_beats.max(1);
            if bx < w {
                for y in 2..HEADER_H {
                    pixels[y * w + bx] = 0x3a485f;
                }
            }
        }

        for (row, ch) in bank.channels.iter().take(MAX_VISIBLE_ROWS).enumerate() {
            let top = HEADER_H + row * ROW_H;
            let bot = (top + ROW_H - 1).min(h);
            let row_bg = if row == selected_channel {
                0x222c3d
            } else {
                0x19202d
            };
            for y in top..bot {
                for x in 0..GRID_X {
                    pixels[y * w + x] = row_bg;
                }
            }

            // Color tag
            let ch_color = colors
                .get(row)
                .copied()
                .unwrap_or_else(|| DEFAULT_CHANNEL_COLORS[row % DEFAULT_CHANNEL_COLORS.len()]);
            for y in (top + 2)..(bot.saturating_sub(2)) {
                for x in 2..7 {
                    pixels[y * w + x] = ch_color;
                }
            }

            // Mute / Solo indicator dot
            let dot_color = if ch.muted {
                0x6b2c2c
            } else if ch.solo {
                0xe6b838
            } else {
                0x4ade80
            };
            for y in (top + 7)..(top + 15).min(bot) {
                for x in 11..19 {
                    pixels[y * w + x] = dot_color;
                }
            }

            // Step Grid or Mini Piano Roll preview
            let preview_mode = pat.map_or(ChannelPreviewMode::StepGrid, |p| {
                p.preview_mode(ch.id, ch.root_key)
            });
            match (pat, preview_mode) {
                (Some(p), ChannelPreviewMode::MiniRoll) => {
                    // Subtle beat background strips
                    for s in 0..total_steps {
                        let x0 = GRID_X + s * GRID_W / total_steps;
                        let x1 = (GRID_X + (s + 1) * GRID_W / total_steps).min(w);
                        let beat_col = kits::step_group_color(s, spb);
                        for y in (top + 2)..(bot.saturating_sub(2)) {
                            for x in x0..x1 {
                                pixels[y * w + x] = if y == top + 2 || y + 3 == bot {
                                    beat_col
                                } else {
                                    0x182130
                                };
                            }
                        }
                    }
                    let notes = p.channel_notes(ch.id);
                    let pat_beats = p.total_beats(sig).to_f64().max(1.0);
                    let mut min_k = notes.iter().map(|n| n.key).min().unwrap_or(48);
                    let mut max_k = notes.iter().map(|n| n.key).max().unwrap_or(72);
                    if max_k <= min_k {
                        min_k = min_k.saturating_sub(6);
                        max_k = max_k.saturating_add(6).min(127);
                    }
                    let pitch_span = (max_k - min_k).max(1) as usize;
                    let inner_h = bot.saturating_sub(top + 6).max(4);
                    for n in notes {
                        let s_frac = (n.start.to_f64() / pat_beats).clamp(0.0, 1.0);
                        let e_frac = (n.end().unwrap_or(n.start).to_f64() / pat_beats).clamp(0.0, 1.0);
                        let nx0 = GRID_X + (s_frac * GRID_W as f64) as usize;
                        let nx1 = (GRID_X + (e_frac * GRID_W as f64) as usize)
                            .max(nx0 + 3)
                            .min(w);
                        let norm_k = (n.key.saturating_sub(min_k) as usize).min(pitch_span);
                        let ny = top + 3 + (pitch_span - norm_k) * (inner_h - 3) / pitch_span;
                        let bar_col = if n.slide_to_key.is_some() {
                            0xffcc66
                        } else {
                            0x67e8f9
                        };
                        for yy in ny..(ny + 3).min(bot.saturating_sub(2)) {
                            for xx in nx0..nx1 {
                                pixels[yy * w + xx] = bar_col;
                            }
                        }
                    }
                }
                _ => {
                    let cells = pat
                        .and_then(|p| p.step_grid(ch.id, ch.root_key, sig).ok())
                        .unwrap_or_default();
                    for s in 0..total_steps {
                        let x0 = GRID_X + s * GRID_W / total_steps;
                        let x1 = (GRID_X + (s + 1) * GRID_W / total_steps).min(w);
                        let bg = kits::step_group_color(s, spb);
                        let active = cells.get(s).is_some_and(|c| c.active);
                        let cell_x0 = x0 + 1;
                        let cell_x1 = x1.saturating_sub(1).max(cell_x0 + 1);
                        for y in (top + 2)..(bot.saturating_sub(2)) {
                            for x in cell_x0..cell_x1 {
                                pixels[y * w + x] = bg;
                            }
                        }
                        if active && cell_x1 > cell_x0 + 2 {
                            let pad_x0 = cell_x0 + 1;
                            let pad_x1 = (cell_x1 - 1).max(pad_x0 + 1);
                            let pad_y0 = top + 5;
                            let pad_y1 = bot.saturating_sub(5).max(pad_y0 + 1);
                            for y in pad_y0..pad_y1 {
                                for x in pad_x0..pad_x1 {
                                    pixels[y * w + x] = ch_color;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Draw crisp labels via Painter
        let mut fonts = weft::text::Fonts::new();
        let mut painter = weft::paint::Painter::new(&mut pixels, w, h, 1.0);
        let pat_title = pat.map_or("Empty Pattern", |p| p.name.as_str());
        let bpm = bank.tempo_map.bpm_x100_at(Time::ZERO) as f32 / 100.0;
        painter.text(
            &mut fonts,
            weft::text::Face::Mono,
            10.0,
            6.0,
            15.0,
            &format!("{} · {:.0} BPM · Swing {}%", pat_title, bpm, bank.swing_pct),
            weft::Color::hex(0xdbe4f0),
        );

        for (row, ch) in bank.channels.iter().take(MAX_VISIBLE_ROWS).enumerate() {
            let base_y = (HEADER_H + row * ROW_H + 15) as f32;
            let inst_name = Instrument::try_from(ch.instrument)
                .map_or("Patch", |i| i.name());
            let short_name: String = ch.name.chars().take(12).collect();
            let short_inst: String = inst_name.chars().take(8).collect();
            painter.text(
                &mut fonts,
                weft::text::Face::Sans,
                9.0,
                22.0,
                base_y,
                &format!("{} · {}", short_name, short_inst),
                weft::Color::hex(if ch.muted { 0x6c7a8e } else { 0xe2e8f0 }),
            );
            if ch.cut_group > 0 {
                painter.text(
                    &mut fonts,
                    weft::text::Face::Mono,
                    8.5,
                    136.0,
                    base_y,
                    &format!("Cut{}", ch.cut_group),
                    weft::Color::hex(0xfbbf24),
                );
            }
            painter.text(
                &mut fonts,
                weft::text::Face::Mono,
                8.5,
                168.0,
                base_y,
                &format!("T{}", ch.target_track + 1),
                weft::Color::hex(0x93c5fd),
            );
        }

        pixels
    }

    /// Bake `TapestryBank` onto `Session::tracks` preserving non-targeted tracks
    /// and existing track mixer/FX/routing settings.
    pub fn bake_bank_to_tracks(
        bank: &TapestryBank,
        tracks: &mut Vec<Track<'static>>,
    ) -> Result<(), &'static str> {
        let mut targeted = [false; MAX_TRACKS];
        let mut max_track = 0usize;
        let mut any_channel = false;
        for ch in &bank.channels {
            let t = ch.target_track as usize;
            if t < MAX_TRACKS {
                targeted[t] = true;
                max_track = max_track.max(t);
                any_channel = true;
            }
        }
        if any_channel {
            while tracks.len() <= max_track {
                tracks.push(Track::new());
            }
        }
        for (idx, track) in tracks.iter_mut().enumerate() {
            if idx < MAX_TRACKS && targeted[idx] {
                track.clips.clear();
                track.reset_effects();
            }
        }
        if bank.channels.is_empty() || bank.patterns.is_empty() {
            return Ok(());
        }

        let sig = bank.tempo_map.time_signature_at(Time::ZERO);
        match bank.transport_mode {
            TransportContext::PatternMode => {
                let Some(pat) = bank
                    .pattern(bank.active_pattern)
                    .or_else(|| bank.patterns.first().map(|a| a.as_ref()))
                else {
                    return Ok(());
                };
                let pat_beats = pat.total_beats(sig);
                let total_frames = (bank.tempo_map.beat_to_frame(pat_beats, bank.swing_pct)
                    as usize)
                    .clamp(1, crate::MAX_FRAMES);
                let realized = bank
                    .realize_events(TransportContext::PatternMode)
                    .map_err(|_| "Tapestry realization failed")?;
                let slice_buffers = bake_slice(
                    bank,
                    Some(pat),
                    &realized,
                    0,
                    total_frames,
                    &targeted,
                )?;
                for (t_idx, maybe_pcm) in slice_buffers.into_iter().enumerate() {
                    if let Some(pcm) = maybe_pcm {
                        let clip =
                            Clip::buffer(pcm, 0).map_err(|_| "Timeline overflow during bake")?;
                        tracks[t_idx].clips.push(clip);
                    }
                }
            }
            TransportContext::SongMode => {
                if bank.instances.is_empty() {
                    return Ok(());
                }
                let realized_all = bank
                    .realize_events(TransportContext::SongMode)
                    .map_err(|_| "Tapestry realization failed")?;
                let constant_tempo = bank.tempo_map.segments().len() == 1
                    && bank.tempo_map.segments()[0].ramp_to_bpm_x100.is_none();
                let mut cache: Vec<(PatternId, [Option<Arc<Vec<Frame>>>; MAX_TRACKS])> =
                    Vec::new();

                for inst in &bank.instances {
                    let Some(pat) = bank.pattern(inst.pattern) else {
                        continue;
                    };
                    let start_frame =
                        bank.tempo_map.beat_to_frame(inst.start_beat, bank.swing_pct) as usize;
                    let end_beat = inst
                        .start_beat
                        .saturating_quantized_add(inst.length_beats)
                        .unwrap_or(inst.start_beat);
                    let end_frame = bank.tempo_map.beat_to_frame(end_beat, bank.swing_pct) as usize;
                    let len_frames = end_frame
                        .saturating_sub(start_frame)
                        .clamp(1, crate::MAX_FRAMES);
                    if start_frame + len_frames > crate::project::MAX_TIMELINE {
                        return Err("Song arrangement exceeds timeline limit");
                    }

                    let pat_beats = pat.total_beats(sig);
                    let half_beat_aligned =
                        inst.start_beat.checked_mul_frac(2, 1).is_some_and(|t| t.den == 1);
                    let can_share = constant_tempo
                        && inst.offset_beats.is_zero()
                        && inst.length_beats == pat_beats
                        && (bank.swing_pct == 0 || half_beat_aligned);

                    let slice_buffers = if can_share {
                        if let Some((_, cached)) =
                            cache.iter().find(|(pid, _)| *pid == inst.pattern)
                        {
                            cached.clone()
                        } else {
                            let local_len = (bank.tempo_map.beat_to_frame(pat_beats, bank.swing_pct)
                                as usize)
                                .clamp(1, crate::MAX_FRAMES);
                            let mut local_bank = bank.clone();
                            local_bank.active_pattern = inst.pattern;
                            local_bank.transport_mode = TransportContext::PatternMode;
                            let local_realized = local_bank
                                .realize_events(TransportContext::PatternMode)
                                .map_err(|_| "Tapestry realization failed")?;
                            let built = bake_slice(
                                &local_bank,
                                Some(pat),
                                &local_realized,
                                0,
                                local_len,
                                &targeted,
                            )?;
                            cache.push((inst.pattern, built.clone()));
                            built
                        }
                    } else {
                        bake_slice(
                            bank,
                            Some(pat),
                            &realized_all,
                            start_frame as u64,
                            len_frames,
                            &targeted,
                        )?
                    };

                    for (t_idx, maybe_pcm) in slice_buffers.into_iter().enumerate() {
                        if let Some(pcm) = maybe_pcm {
                            let clip = Clip::buffer(pcm, start_frame)
                                .map_err(|_| "Timeline overflow during bake")?;
                            tracks[t_idx].clips.push(clip);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

fn bake_slice(
    bank: &TapestryBank,
    pat: Option<&TapestryPattern>,
    realized: &[RealizedChannelNote],
    slice_start: u64,
    slice_len: usize,
    targeted: &[bool; MAX_TRACKS],
) -> Result<[Option<Arc<Vec<Frame>>>; MAX_TRACKS], &'static str> {
    let slice_end = slice_start + slice_len as u64;
    let bpm = (bank.tempo_map.bpm_x100_at(Time::ZERO) / 100).clamp(30, 240) as u16;
    let mut track_accum: [Option<Vec<[f32; 2]>>; MAX_TRACKS] = Default::default();

    for ch in &bank.channels {
        let t_idx = ch.target_track as usize;
        if t_idx >= MAX_TRACKS || !targeted[t_idx] {
            continue;
        }
        let ch_events: Vec<&RealizedChannelNote> = realized
            .iter()
            .filter(|ev| {
                ev.channel == ch.id && ev.start_frame >= slice_start && ev.start_frame < slice_end
            })
            .collect();
        if ch_events.is_empty() {
            continue;
        }

        let inst = Instrument::try_from(ch.instrument).unwrap_or(Instrument::SilkTriangle);
        let raw_pat_notes = pat.map_or(&[][..], |p| p.channel_notes(ch.id));

        let mut ch_pcm = vec![[0.0f32; 2]; slice_len];
        if (inst as u8) >= 5 {
            let params = InstrumentParams::for_instrument(inst)
                .ok_or("Unsupported instrument index")?;
            let mut expr_notes = Vec::with_capacity(ch_events.len());
            for (k, ev) in ch_events.iter().enumerate() {
                let rel_start = (ev.start_frame - slice_start) as u32;
                let max_avail = (slice_len as u32).saturating_sub(rel_start).max(1);
                let mut len = ev
                    .end_frame
                    .saturating_sub(ev.start_frame)
                    .max(1)
                    .min(max_avail as u64) as u32;

                // Preserve phase-locked overlapping gate for Sub808 legato slides.
                if inst == Instrument::Sub808 {
                    if let Some(next_ev) = ch_events.get(k + 1) {
                        let next_rel = next_ev.start_frame.saturating_sub(slice_start) as u32;
                        if next_rel > rel_start && next_rel < slice_len as u32 {
                            let raw_overlap = raw_pat_notes.iter().any(|rn| {
                                let rn_end = rn.end().unwrap_or(rn.start);
                                let next_beat_overlap = next_ev.start_beat > rn.start
                                    && next_ev.start_beat < rn_end;
                                next_beat_overlap
                            });
                            if next_ev.slide_to_key.is_some()
                                || raw_overlap
                                || ev.end_frame > next_ev.start_frame
                            {
                                len = (next_rel - rel_start + 1).min(max_avail);
                            }
                        }
                    }
                }

                let note = midi::Note {
                    start: rel_start,
                    frames: len,
                    key: ev.key,
                    velocity: ev.velocity,
                };
                let expr = instruments::NoteExpr {
                    pitch_bend_semitones: ev.fine_cents as f32 / 100.0,
                    pressure: ev.pressure as f32 / 127.0,
                    timbre: ev.timbre as f32 / 127.0,
                    slide_from_key: None,
                    glide_frames: 0,
                };
                expr_notes.push(instruments::ExpressiveNote::new(note, expr));
            }
            for chunk in expr_notes.chunks(midi::MAX_NOTES) {
                let rendered =
                    instruments::render_notes_with_params(chunk, slice_len as u32, &params)
                        .map_err(|_| "Instrument synthesis failed")?;
                for (dst, src) in ch_pcm.iter_mut().zip(rendered.iter()) {
                    dst[0] += src[0] as f32 / 32768.0;
                    dst[1] += src[1] as f32 / 32768.0;
                }
            }
        } else {
            let mut midi_notes = Vec::with_capacity(ch_events.len());
            for ev in &ch_events {
                let rel_start = (ev.start_frame - slice_start) as u32;
                let max_avail = (slice_len as u32).saturating_sub(rel_start).max(1);
                let len = ev
                    .end_frame
                    .saturating_sub(ev.start_frame)
                    .max(1)
                    .min(max_avail as u64) as u32;
                midi_notes.push(midi::Note {
                    start: rel_start,
                    frames: len,
                    key: ev.key,
                    velocity: ev.velocity,
                });
            }
            for chunk in midi_notes.chunks(midi::MAX_NOTES) {
                let mpat = midi::Pattern {
                    frames: slice_len as u32,
                    bpm,
                    instrument: inst,
                    notes: chunk.to_vec(),
                };
                let rendered = mpat.bake().map_err(|_| "MIDI instrument bake failed")?;
                for (dst, src) in ch_pcm.iter_mut().zip(rendered.iter()) {
                    dst[0] += src[0] as f32 / 32768.0;
                    dst[1] += src[1] as f32 / 32768.0;
                }
            }
        }

        // Enforce cross-channel & intra-channel cut_group chokes with a crisp 3 ms (144-frame) fade.
        if ch.cut_group != 0 && inst != Instrument::Sub808 {
            const CHOKE_FADE: usize = 144;
            for (k, ev) in ch_events.iter().enumerate() {
                let Some(choke) = realized
                    .iter()
                    .find(|c| c.cut_group == ch.cut_group && c.start_frame > ev.start_frame)
                else {
                    continue;
                };
                let choke_rel = choke.start_frame.saturating_sub(slice_start) as usize;
                if choke_rel >= slice_len {
                    continue;
                }
                let next_own_rel = ch_events
                    .get(k + 1)
                    .map_or(slice_len, |n| {
                        (n.start_frame.saturating_sub(slice_start) as usize).min(slice_len)
                    });
                if choke_rel < next_own_rel {
                    let fade_end = (choke_rel + CHOKE_FADE).min(next_own_rel);
                    let fade_len = (fade_end - choke_rel).max(1);
                    for f in choke_rel..fade_end {
                        let g = 1.0 - (f - choke_rel) as f32 / fade_len as f32;
                        ch_pcm[f][0] *= g;
                        ch_pcm[f][1] *= g;
                    }
                    for f in fade_end..next_own_rel {
                        ch_pcm[f] = [0.0, 0.0];
                    }
                }
            }
        }

        let pan_norm = ch.pan as f32 / 100.0;
        let bal_l = 1.0 - pan_norm.max(0.0);
        let bal_r = 1.0 + pan_norm.min(0.0);
        let acc = track_accum[t_idx].get_or_insert_with(|| vec![[0.0f32; 2]; slice_len]);
        for (dst, src) in acc.iter_mut().zip(ch_pcm.iter()) {
            dst[0] += src[0] * bal_l;
            dst[1] += src[1] * bal_r;
        }
    }

    let mut out: [Option<Arc<Vec<Frame>>>; MAX_TRACKS] = Default::default();
    for (t_idx, maybe_acc) in track_accum.into_iter().enumerate() {
        if let Some(acc) = maybe_acc {
            let mut frames = Vec::new();
            frames
                .try_reserve_exact(acc.len())
                .map_err(|_| "Out of memory baking Tapestry track")?;
            for lr in acc {
                frames.push([
                    (lr[0] * 32767.0).clamp(-32768.0, 32767.0) as i16,
                    (lr[1] * 32767.0).clamp(-32768.0, 32767.0) as i16,
                ]);
            }
            out[t_idx] = Some(Arc::new(frames));
        }
    }
    Ok(out)
}
