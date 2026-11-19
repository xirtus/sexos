// SPDX-License-Identifier: GPL-3.0-only
//! Studio 2.0 Visual Lower Device Chain & Clip Settings.
//!
//! Provides the visual multi-card curve strip (Prism Wavetable, Distortion Transfer,
//! and Equalizer Frequency Response), Clip Warp (Transpose, Speed, On Beat),
//! offline Extract MIDI, and the Piano Roll Arpeggiator.

extern crate alloc;

use alloc::{vec, vec::Vec};
use moan_core::{
    arp::{generate_arp, ArpMode, StepDivision},
    effects::{Distortion, DistortionMode, Equalizer},
    midi::{Note, Pattern},
    wavetable::{preview_curve, PrismParams},
};
use weft::{Id, Layout, TextStyle, Ui};

pub const CURVES_WIDTH: usize = 480;
pub const CURVES_HEIGHT: usize = 90;

/// UI widget IDs and parameters for the visual device chain & clip warp.
pub struct StudioRackUi {
    pub panel: Id,
    pub curves_image: Id,

    // Clip Warp controls
    pub transpose_slider: Id,
    pub speed_slider: Id,
    pub on_beat_checkbox: Id,
    pub apply_warp_btn: Id,
    pub extract_midi_btn: Id,

    // Distortion controls
    pub distortion_checkbox: Id,
    pub dist_mode_btn: Id,
    pub dist_drive_slider: Id,

    // Arp command button
    pub arp_btn: Id,

    pub transpose: i8,
    pub speed_percent: u16,
    pub on_beat: bool,
    pub distortion_enabled: bool,
    pub distortion_mode_idx: usize,
    pub distortion_drive: u16,
    pub prism_params: PrismParams,
}

impl StudioRackUi {
    pub fn build(ui: &mut Ui, parent: Id) -> Self {
        let panel = ui.panel(
            parent,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );

        ui.text(panel, "Studio 2.0 Visual Device Chain & Clip Warp", TextStyle::Heading);

        // Visual curve display strip (Wavetable + Distortion + EQ)
        let (_, _, pixels) = Self::render_device_curves(
            &PrismParams::default(),
            None,
            None,
        );
        let curves_image = ui.image(panel, CURVES_WIDTH, CURVES_HEIGHT, pixels);
        ui.set_label(
            curves_image,
            "Realtime DSP curve strip: Prism Wavetable, Distortion Transfer Curve, and 3-Band EQ Response",
        );

        // Clip Settings: Warp, Transpose, Speed, Extract MIDI
        let warp_row = ui.group(panel, Layout::row().gap(6.0).wrap().fill_width());
        ui.text(warp_row, "Transpose", TextStyle::Caption);
        let transpose_slider = ui.slider(warp_row, -12.0, 12.0, 0.0);
        ui.set_layout(transpose_slider, Layout::default().width(90.0));
        ui.set_step(transpose_slider, 1.0);

        ui.text(warp_row, "Speed %", TextStyle::Caption);
        let speed_slider = ui.slider(warp_row, 50.0, 200.0, 100.0);
        ui.set_layout(speed_slider, Layout::default().width(90.0));
        ui.set_step(speed_slider, 1.0);

        let on_beat_checkbox = ui.checkbox(warp_row, "On Beat", false);
        let apply_warp_btn = ui.button(warp_row, "Apply Warp");
        let extract_midi_btn = ui.button(warp_row, "Extract MIDI");

        // Lower Distortion Chain controls
        let dist_row = ui.group(panel, Layout::row().gap(6.0).wrap().fill_width());
        let distortion_checkbox = ui.checkbox(dist_row, "Distortion", false);
        let dist_mode_btn = ui.button(dist_row, "Mode: Sym");

        ui.text(dist_row, "Drive", TextStyle::Caption);
        let dist_drive_slider = ui.slider(dist_row, 0.0, 400.0, 100.0);
        ui.set_layout(dist_drive_slider, Layout::default().width(90.0));

        let arp_btn = ui.button(dist_row, "Arpeggiate");

        Self {
            panel,
            curves_image,
            transpose_slider,
            speed_slider,
            on_beat_checkbox,
            apply_warp_btn,
            extract_midi_btn,
            distortion_checkbox,
            dist_mode_btn,
            dist_drive_slider,
            arp_btn,

            transpose: 0,
            speed_percent: 100,
            on_beat: false,
            distortion_enabled: false,
            distortion_mode_idx: 0,
            distortion_drive: 100,
            prism_params: PrismParams::default(),
        }
    }

    /// Cycle through distortion modes (Symmetric, Asymmetric, HardClip, Foldback).
    pub fn cycle_distortion_mode(&mut self, ui: &mut Ui) -> DistortionMode {
        self.distortion_mode_idx = (self.distortion_mode_idx + 1) % 4;
        let mode = match self.distortion_mode_idx {
            0 => DistortionMode::Symmetric,
            1 => DistortionMode::Asymmetric,
            2 => DistortionMode::HardClip,
            _ => DistortionMode::Foldback,
        };
        let label = match mode {
            DistortionMode::Symmetric => "Mode: Sym",
            DistortionMode::Asymmetric => "Mode: Asym",
            DistortionMode::HardClip => "Mode: Clip",
            DistortionMode::Foldback => "Mode: Fold",
        };
        ui.set_text(self.dist_mode_btn, label);
        mode
    }

    /// Build distortion instance from current parameters.
    pub fn build_distortion(&self) -> Option<Distortion> {
        if !self.distortion_enabled {
            return None;
        }
        let mode = match self.distortion_mode_idx {
            0 => DistortionMode::Symmetric,
            1 => DistortionMode::Asymmetric,
            2 => DistortionMode::HardClip,
            _ => DistortionMode::Foldback,
        };
        Distortion::new(self.distortion_drive, 0, 0, 100, 8000, mode).ok()
    }

    /// Render 3-card DSP curves image (Wavetable + Distortion + EQ).
    pub fn render_device_curves(
        prism: &PrismParams,
        distortion: Option<&Distortion>,
        eq: Option<&Equalizer>,
    ) -> (usize, usize, Vec<u32>) {
        let w = CURVES_WIDTH;
        let h = CURVES_HEIGHT;
        let mut pixels = vec![0xff181c24u32; w * h];

        let card_w = 152usize;
        let card_h = h.saturating_sub(8);

        // Card 1: Prism Wavetable (x: 4..156)
        let mut wave_points = [0i16; 140];
        preview_curve(prism, &mut wave_points);
        Self::draw_card_frame(&mut pixels, w, 4, 4, card_w, card_h, 0xff242b38);
        for (i, p) in wave_points.iter().enumerate() {
            let cx = 10 + i;
            let norm = (*p as f32 / 32768.0).clamp(-1.0, 1.0);
            let cy = 4 + (card_h as f32 * (0.5 - norm * 0.45)) as usize;
            let cy = cy.min(h - 6);
            pixels[cy * w + cx] = 0xff4ade80; // Green waveform
        }

        // Card 2: Distortion Transfer Curve (x: 164..316)
        let mut dist_points = [0i16; 140];
        if let Some(dist) = distortion {
            dist.transfer_curve(&mut dist_points);
        } else {
            let dummy = Distortion::new(100, 0, 0, 100, 8000, DistortionMode::Symmetric).unwrap();
            dummy.transfer_curve(&mut dist_points);
        }
        Self::draw_card_frame(&mut pixels, w, 164, 4, card_w, card_h, 0xff242b38);
        for (i, p) in dist_points.iter().enumerate() {
            let cx = 170 + i;
            let norm = (*p as f32 / 32768.0).clamp(-1.0, 1.0);
            let cy = 4 + (card_h as f32 * (0.5 - norm * 0.45)) as usize;
            let cy = cy.min(h - 6);
            pixels[cy * w + cx] = 0xfffb923c; // Amber distortion curve
        }

        // Card 3: 3-Band Equalizer Frequency Response (x: 324..476)
        let mut eq_points = [0i16; 140];
        if let Some(eq) = eq {
            eq.response_curve(&mut eq_points);
        } else {
            let dummy = Equalizer::new(100, 100, 100).unwrap();
            dummy.response_curve(&mut eq_points);
        }
        Self::draw_card_frame(&mut pixels, w, 324, 4, card_w, card_h, 0xff242b38);
        for (i, p) in eq_points.iter().enumerate() {
            let cx = 330 + i;
            // Q8 dB (256 = 1 dB)
            let db = *p as f32 / 256.0;
            let norm = (db / 12.0).clamp(-1.0, 1.0);
            let cy = 4 + (card_h as f32 * (0.5 - norm * 0.45)) as usize;
            let cy = cy.min(h - 6);
            pixels[cy * w + cx] = 0xff38bdf8; // Cyan EQ curve
        }

        (w, h, pixels)
    }

    fn draw_card_frame(pixels: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
        for row in y..(y + h) {
            for col in x..(x + w) {
                if row == y || row == y + h - 1 || col == x || col == x + w - 1 {
                    pixels[row * stride + col] = color;
                }
            }
        }
    }

    /// Apply Arpeggiator to pattern notes.
    pub fn arpeggiate(&self, pattern: &mut Pattern, bpm: u16) {
        let held_keys: Vec<u8> = if pattern.notes.is_empty() {
            vec![60, 64, 67, 71] // Default Cmaj7 chord
        } else {
            pattern.notes.iter().map(|n| n.key).collect()
        };

        let generated = generate_arp(
            &held_keys,
            bpm as f32,
            StepDivision::Sixteenth,
            75.0,
            ArpMode::UpDown,
            16,
        );

        pattern.notes = generated
            .into_iter()
            .map(|(start, frames, key, velocity)| Note {
                start,
                frames,
                key,
                velocity,
            })
            .collect();
    }
}
