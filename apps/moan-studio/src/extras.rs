// SPDX-License-Identifier: GPL-3.0-only
//! Geist font loading, Logic Noir Musical Typing (`K`), Quick Sampler (`Q`),
//! Event List (`D`), and macOS-style Menu Bar for Moan Studio on SexOS.

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
};
use std::sync::Arc;

const GEIST_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Regular.ttf");
const GEIST_MONO: &[u8] = include_bytes!("../../../assets/fonts/geist/GeistMono-Regular.ttf");

/// Install Geist (proportional) and GeistMono (monospace) as the primary egui fonts
/// so Moan Studio matches macOS / Logic Noir typography cleanly.
pub fn install_geist_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Geist".to_owned(),
        Arc::new(FontData::from_static(GEIST_REGULAR)),
    );
    fonts.font_data.insert(
        "GeistMono".to_owned(),
        Arc::new(FontData::from_static(GEIST_MONO)),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Geist".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "GeistMono".to_owned());
    ctx.set_fonts(fonts);
}

/// QWERTY -> semitone offset from current octave root (Logic Pro Musical Typing layout).
pub const TYPING_KEYS: [(Key, u8, &str, bool); 17] = [
    (Key::A, 0, "A", false),
    (Key::W, 1, "W", true),
    (Key::S, 2, "S", false),
    (Key::E, 3, "E", true),
    (Key::D, 4, "D", false),
    (Key::F, 5, "F", false),
    (Key::T, 6, "T", true),
    (Key::G, 7, "G", false),
    (Key::Y, 8, "Y", true),
    (Key::H, 9, "H", false),
    (Key::U, 10, "U", true),
    (Key::J, 11, "J", false),
    (Key::K, 12, "K", false),
    (Key::O, 13, "O", true),
    (Key::L, 14, "L", false),
    (Key::P, 15, "P", true),
    (Key::Semicolon, 16, ";", false),
];

pub struct LiveVoice {
    pub pitch: u8,
    pub phase: f32,
    pub env: f32,
    pub vel: f32,
    pub held: bool,
}

pub struct MusicalTypingState {
    pub open: bool,
    pub octave: i8,   // C3 = 3 (MIDI 48)
    pub velocity: u8, // 1..=127
    pub sustain: bool,
    pub voices: Vec<LiveVoice>,
}

impl Default for MusicalTypingState {
    fn default() -> Self {
        Self {
            open: false,
            octave: 4, // C4 = MIDI 60
            velocity: 100,
            sustain: false,
            voices: Vec::new(),
        }
    }
}

impl MusicalTypingState {
    pub fn root_midi(&self) -> u8 {
        ((self.octave as i32 + 1) * 12).clamp(24, 96) as u8
    }

    pub fn trigger(&mut self, pitch: u8) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.pitch == pitch && v.held) {
            v.env = 1.0;
            return;
        }
        self.voices.push(LiveVoice {
            pitch,
            phase: 0.0,
            env: 1.0,
            vel: (self.velocity as f32 / 127.0).clamp(0.15, 1.0),
            held: true,
        });
        if self.voices.len() > 16 {
            self.voices.remove(0);
        }
    }

    pub fn release(&mut self, pitch: u8) {
        for v in &mut self.voices {
            if v.pitch == pitch {
                v.held = false;
            }
        }
    }

    pub fn is_active(&self, pitch: u8) -> bool {
        self.voices.iter().any(|v| v.pitch == pitch && v.held)
    }

    /// Render polyphonic electric-piano voices into a stereo i32 mix buffer.
    pub fn mix_into(&mut self, mix: &mut [[i32; 2]], rate: f32) {
        let tau = std::f32::consts::TAU;
        for v in &mut self.voices {
            let freq = 440.0 * 2.0f32.powf((v.pitch as f32 - 69.0) / 12.0);
            let inc = freq / rate;
            let decay = if v.held || self.sustain { 0.99985 } else { 0.9982 };
            for sample in mix.iter_mut() {
                v.phase = (v.phase + inc) % 1.0;
                // Warm Rhodes/E-Piano harmonic blend
                let s = (v.phase * tau).sin() * 0.72
                    + (v.phase * tau * 2.0).sin() * 0.20 * v.env
                    + (v.phase * tau * 3.0).sin() * 0.08 * v.env;
                let out = (s * v.env * v.vel * 7500.0) as i32;
                sample[0] += out;
                sample[1] += out;
                v.env *= decay;
            }
        }
        self.voices.retain(|v| v.env > 0.004);
    }

    pub fn has_sound(&self) -> bool {
        !self.voices.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SamplerMode {
    Classic,
    OneShot,
    Slice,
    Recorder,
}

pub struct QuickSamplerState {
    pub open: bool,
    pub mode: SamplerMode,
    pub start: f32,
    pub end: f32,
    pub cutoff: f32,
    pub reso: f32,
    pub drive: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub pitch_semi: f32,
    pub lfo_rate: f32,
}

impl Default for QuickSamplerState {
    fn default() -> Self {
        Self {
            open: false,
            mode: SamplerMode::Classic,
            start: 0.08,
            end: 0.88,
            cutoff: 0.72,
            reso: 0.25,
            drive: 0.15,
            attack: 0.02,
            decay: 0.35,
            sustain: 0.65,
            release: 0.40,
            pitch_semi: 0.0,
            lfo_rate: 3.2,
        }
    }
}

pub fn draw_musical_typing_window(
    ctx: &egui::Context,
    mt: &mut MusicalTypingState,
    on_note_on: &mut dyn FnMut(u8, u8),
) {
    if !mt.open {
        return;
    }
    let mut open = mt.open;
    egui::Window::new(RichText::new("Musical Typing  (K)").color(Color32::WHITE).strong())
        .id(egui::Id::unique("sexos_musical_typing"))
        .open(&mut open)
        .resizable(false)
        .default_size([510.0, 178.0])
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(Color32::from_rgb(0x1f, 0x1f, 0x24))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3a, 0x44)))
                .corner_radius(CornerRadius::same(8)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Octave: C{} (Z/X)", mt.octave)).size(11.0));
                if ui.small_button("-").clicked() {
                    mt.octave = (mt.octave - 1).clamp(1, 6);
                }
                if ui.small_button("+").clicked() {
                    mt.octave = (mt.octave + 1).clamp(1, 6);
                }
                ui.separator();
                ui.label(RichText::new(format!("Vel: {} (C/V)", mt.velocity)).size(11.0));
                ui.add(egui::Slider::new(&mut mt.velocity, 1..=127).show_value(false));
                ui.checkbox(&mut mt.sustain, "Sustain (Tab)");
            });
            ui.add_space(6.0);

            // Piano keyboard canvas (10 white keys + 7 black keys)
            let (rect, _) = ui.allocate_exact_size(vec2(486.0, 112.0), Sense::hover());
            let g = ui.painter();
            let root = mt.root_midi();
            let white_offsets: [(u8, &str); 10] = [
                (0, "A"),
                (2, "S"),
                (4, "D"),
                (5, "F"),
                (7, "G"),
                (9, "H"),
                (11, "J"),
                (12, "K"),
                (14, "L"),
                (16, ";"),
            ];
            let kw = rect.width() / white_offsets.len() as f32;
            for (i, (semi, cap)) in white_offsets.iter().enumerate() {
                let pitch = root + semi;
                let kr = Rect::from_min_size(
                    pos2(rect.left() + i as f32 * kw, rect.top()),
                    vec2(kw - 1.5, rect.height()),
                );
                let resp = ui.interact(kr, egui::Id::unique(("mt_w", i)), Sense::click_and_drag());
                if resp.is_pointer_button_down_on() {
                    if !mt.is_active(pitch) {
                        mt.trigger(pitch);
                        on_note_on(pitch, mt.velocity);
                    }
                } else if resp.drag_stopped() || resp.clicked() {
                    mt.release(pitch);
                }
                let active = mt.is_active(pitch);
                g.rect(
                    kr,
                    CornerRadius::same(3),
                    if active {
                        Color32::from_rgb(0x2d, 0x7a, 0xff)
                    } else {
                        Color32::from_rgb(0xe6, 0xe6, 0xec)
                    },
                    Stroke::new(1.0, Color32::from_rgb(0x14, 0x14, 0x18)),
                    StrokeKind::Inside,
                );
                g.text(
                    pos2(kr.center().x, kr.bottom() - 14.0),
                    Align2::CENTER_CENTER,
                    *cap,
                    FontId::proportional(11.5),
                    if active { Color32::WHITE } else { Color32::from_rgb(0x22, 0x22, 0x26) },
                );
            }

            // Black keys overlay
            let black_pos: [(f32, u8, &str); 7] = [
                (0.66, 1, "W"),
                (1.66, 3, "E"),
                (3.66, 6, "T"),
                (4.66, 8, "Y"),
                (5.66, 10, "U"),
                (7.66, 13, "O"),
                (8.66, 15, "P"),
            ];
            for (i, (slot, semi, cap)) in black_pos.iter().enumerate() {
                let pitch = root + semi;
                let kr = Rect::from_min_size(
                    pos2(rect.left() + slot * kw, rect.top()),
                    vec2(kw * 0.66, rect.height() * 0.62),
                );
                let resp = ui.interact(kr, egui::Id::unique(("mt_b", i)), Sense::click_and_drag());
                if resp.is_pointer_button_down_on() {
                    if !mt.is_active(pitch) {
                        mt.trigger(pitch);
                        on_note_on(pitch, mt.velocity);
                    }
                } else if resp.drag_stopped() || resp.clicked() {
                    mt.release(pitch);
                }
                let active = mt.is_active(pitch);
                g.rect(
                    kr,
                    CornerRadius::same(3),
                    if active {
                        Color32::from_rgb(0x2d, 0x7a, 0xff)
                    } else {
                        Color32::from_rgb(0x16, 0x16, 0x1b)
                    },
                    Stroke::new(1.0, Color32::from_rgb(0x0c, 0x0c, 0x0f)),
                    StrokeKind::Inside,
                );
                g.text(
                    pos2(kr.center().x, kr.bottom() - 12.0),
                    Align2::CENTER_CENTER,
                    *cap,
                    FontId::proportional(10.0),
                    Color32::from_rgb(0xdc, 0xdc, 0xe4),
                );
            }
        });
    mt.open = open;
}

pub fn draw_quick_sampler_window(ctx: &egui::Context, qs: &mut QuickSamplerState, peaks: &[f32]) {
    if !qs.open {
        return;
    }
    let mut open = qs.open;
    egui::Window::new(RichText::new("Quick Sampler  (Q)").color(Color32::WHITE).strong())
        .id(egui::Id::unique("sexos_quick_sampler"))
        .open(&mut open)
        .default_size([560.0, 340.0])
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(Color32::from_rgb(0x1b, 0x21, 0x29))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x2d, 0x7a, 0xff)))
                .corner_radius(CornerRadius::same(8)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (m, lbl) in [
                    (SamplerMode::Classic, "CLASSIC"),
                    (SamplerMode::OneShot, "ONE SHOT"),
                    (SamplerMode::Slice, "SLICE"),
                    (SamplerMode::Recorder, "RECORDER"),
                ] {
                    let on = qs.mode == m;
                    let btn = egui::Button::new(
                        RichText::new(lbl)
                            .color(if on { Color32::from_rgb(0x10, 0x14, 0x18) } else { Color32::from_rgb(0xc8, 0xd4, 0xe4) })
                            .size(10.5)
                            .strong(),
                    )
                    .fill(if on { Color32::from_rgb(0xe8, 0xa6, 0x2a) } else { Color32::from_rgb(0x24, 0x2c, 0x36) });
                    if ui.add(btn).clicked() {
                        qs.mode = m;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Root Key: C3  •  48 kHz").color(Color32::from_rgb(0x7c, 0xa8, 0xd4)).size(11.0));
                });
            });
            ui.add_space(4.0);

            // Logic Noir Blue Waveform Display with Start/End markers
            let (wf, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
            let g = ui.painter();
            g.rect(
                wf,
                CornerRadius::same(4),
                Color32::from_rgb(0x0b, 0x28, 0x42),
                Stroke::new(1.0, Color32::from_rgb(0x1d, 0x52, 0x7e)),
                StrokeKind::Inside,
            );
            let mid = wf.center().y;
            g.line_segment([pos2(wf.left(), mid), pos2(wf.right(), mid)], Stroke::new(1.0, Color32::from_rgb(0x1a, 0x48, 0x6e)));
            let sample_peaks: Vec<f32> = if peaks.is_empty() {
                (0..140).map(|i| {
                    let t = i as f32 / 140.0;
                    ((-t * 4.5).exp() * (t * 40.0).sin().abs()).max(0.04)
                }).collect()
            } else {
                peaks.to_vec()
            };
            let dx = wf.width() / sample_peaks.len().max(1) as f32;
            for (i, &pk) in sample_peaks.iter().enumerate() {
                let x = wf.left() + i as f32 * dx;
                let h = pk * wf.height() * 0.45;
                g.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(1.4, Color32::from_rgb(0x3a, 0xa8, 0xf2)));
            }
            let sx = wf.left() + qs.start * wf.width();
            let ex = wf.left() + qs.end * wf.width();
            g.line_segment([pos2(sx, wf.top()), pos2(sx, wf.bottom())], Stroke::new(2.0, Color32::from_rgb(0x52, 0xc4, 0xff)));
            g.line_segment([pos2(ex, wf.top()), pos2(ex, wf.bottom())], Stroke::new(2.0, Color32::from_rgb(0x52, 0xc4, 0xff)));

            ui.add_space(8.0);
            ui.columns(4, |cols| {
                cols[0].label(RichText::new("LFO 1").color(Color32::from_rgb(0xe8, 0xa6, 0x2a)).strong().size(11.0));
                cols[0].add(egui::Slider::new(&mut qs.lfo_rate, 0.1..=20.0).text("Rate"));
                cols[0].add(egui::Slider::new(&mut qs.start, 0.0..=0.5).text("Start"));

                cols[1].label(RichText::new("PITCH").color(Color32::from_rgb(0x7c, 0xc2, 0xf8)).strong().size(11.0));
                cols[1].add(egui::Slider::new(&mut qs.pitch_semi, -24.0..=24.0).text("Semi"));
                cols[1].add(egui::Slider::new(&mut qs.end, 0.5..=1.0).text("End"));

                cols[2].label(RichText::new("FILTER").color(Color32::from_rgb(0xe8, 0xa6, 0x2a)).strong().size(11.0));
                cols[2].add(egui::Slider::new(&mut qs.cutoff, 0.0..=1.0).text("Cutoff"));
                cols[2].add(egui::Slider::new(&mut qs.reso, 0.0..=1.0).text("Reso"));
                cols[2].add(egui::Slider::new(&mut qs.drive, 0.0..=1.0).text("Drive"));

                cols[3].label(RichText::new("AMP ADSR").color(Color32::from_rgb(0x52, 0xc4, 0xff)).strong().size(11.0));
                cols[3].add(egui::Slider::new(&mut qs.attack, 0.0..=1.0).text("A"));
                cols[3].add(egui::Slider::new(&mut qs.decay, 0.0..=1.0).text("D"));
                cols[3].add(egui::Slider::new(&mut qs.sustain, 0.0..=1.0).text("S"));
                cols[3].add(egui::Slider::new(&mut qs.release, 0.0..=1.0).text("R"));
            });
        });
    qs.open = open;
}

pub fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(pitch % 12) as usize], (pitch as i32 / 12) - 1)
}
