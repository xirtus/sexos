// SPDX-License-Identifier: GPL-3.0-only
//! Pro Console FX & Mastering Weft Controls:
//! 6-Band Parametric EQ + Spectrum Resonance Cut, Sidechain Compression/Gate + GR dB,
//! Post-Fader Sends/Buses, Analog Tape, Transient Shaper, BitCrusher, Stereo Chorus,
//! and Master Lookahead Limiting.

extern crate alloc;

use crate::{Session, MAX_TRACKS};
use alloc::{format, vec, vec::Vec};
use moan_core::{
    device_presets::{GATE_PRESETS, MASTERING_PRESETS, TAPE_PRESETS},
    dsp_math::{exp2_f32, log10_f32},
    mastering::MasteringProcessor,
    parametric_eq::{detect_resonances, ParametricEq, ResonanceCandidate, SpectrumAnalyzer},
    tape_fx::{BitCrusher, SidechainCompressor, SidechainGate, StereoChorus, TapeMachine, TransientShaper},
    Frame, Track, RATE,
};
use weft::{Id, Layout, TextStyle, Ui};

pub const EQ_WIDTH: usize = 440;
pub const EQ_HEIGHT: usize = 96;

/// UI widget identifiers and preset indices for Console FX & Mastering.
pub struct ConsoleFxUi {
    pub panel: Id,
    pub eq_image: Id,
    pub toggle_eq_btn: Id,
    pub hpf_btn: Id,
    pub auto_cut_btn: Id,
    pub reset_eq_btn: Id,

    pub sc_key_btn: Id,
    pub sc_duck_btn: Id,
    pub gate_preset_btn: Id,
    pub apply_gate_btn: Id,
    pub send_btn: Id,
    pub bus_btn: Id,
    pub gr_readout: Id,

    pub tape_preset_btn: Id,
    pub apply_tape_btn: Id,
    pub transient_btn: Id,
    pub crusher_btn: Id,
    pub chorus_btn: Id,
    pub master_preset_btn: Id,
    pub apply_master_btn: Id,
    pub disable_master_btn: Id,

    pub gate_preset_idx: usize,
    pub tape_preset_idx: usize,
    pub master_preset_idx: usize,
    pub last_spectrum_db: Vec<f32>,
    pub last_resonances: [Option<ResonanceCandidate>; 3],
}

impl ConsoleFxUi {
    pub fn build(ui: &mut Ui, parent: Id) -> Self {
        let panel = ui.panel(
            parent,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );

        ui.text(
            panel,
            "Pro Console FX · 6-Band Parametric EQ · Sidechain & Mastering",
            TextStyle::Heading,
        );

        let initial_pixels = Self::render_eq_and_spectrum(None, &[], &[None; 3]);
        let eq_image = ui.image(panel, EQ_WIDTH, EQ_HEIGHT, initial_pixels);
        ui.set_label(
            eq_image,
            "6-Band Parametric EQ curve, stereo FFT spectrum, and detected resonance markers",
        );

        let eq_row = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        let toggle_eq_btn = ui.button(eq_row, "Toggle 6-Band EQ");
        let hpf_btn = ui.button(eq_row, "HPF 80Hz");
        let auto_cut_btn = ui.button(eq_row, "Auto Cut Resonance");
        let reset_eq_btn = ui.button(eq_row, "Reset 6-Band EQ");

        let dyn_row = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        let sc_key_btn = ui.button(dyn_row, "SC Key: Off");
        let sc_duck_btn = ui.button(dyn_row, "Enable Sidechain Duck");
        let gate_preset_btn = ui.button(dyn_row, &format!("Gate: {}", GATE_PRESETS[0].name));
        let apply_gate_btn = ui.button(dyn_row, "Apply Gate Preset");
        let send_btn = ui.button(dyn_row, "Post Send: Off");
        let bus_btn = ui.button(dyn_row, "Out Bus: Master");
        let gr_readout = ui.text(dyn_row, "GR: 0.0 dB", TextStyle::Caption);

        let color_row = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        let tape_preset_btn = ui.button(color_row, &format!("Tape: {}", TAPE_PRESETS[0].name));
        let apply_tape_btn = ui.button(color_row, "Apply Tape Preset");
        let transient_btn = ui.button(color_row, "Toggle Transient");
        let crusher_btn = ui.button(color_row, "Toggle BitCrush");
        let chorus_btn = ui.button(color_row, "Toggle Chorus");
        let master_preset_btn =
            ui.button(color_row, &format!("Master: {}", MASTERING_PRESETS[0].name));
        let apply_master_btn = ui.button(color_row, "Apply Mastering Preset");
        let disable_master_btn = ui.button(color_row, "Disable Mastering");

        Self {
            panel,
            eq_image,
            toggle_eq_btn,
            hpf_btn,
            auto_cut_btn,
            reset_eq_btn,
            sc_key_btn,
            sc_duck_btn,
            gate_preset_btn,
            apply_gate_btn,
            send_btn,
            bus_btn,
            gr_readout,
            tape_preset_btn,
            apply_tape_btn,
            transient_btn,
            crusher_btn,
            chorus_btn,
            master_preset_btn,
            apply_master_btn,
            disable_master_btn,

            gate_preset_idx: 0,
            tape_preset_idx: 0,
            master_preset_idx: 0,
            last_spectrum_db: Vec::new(),
            last_resonances: [None; 3],
        }
    }

    pub fn cycle_gate_preset(&mut self, ui: &mut Ui) -> &'static str {
        self.gate_preset_idx = (self.gate_preset_idx + 1) % GATE_PRESETS.len();
        let name = GATE_PRESETS[self.gate_preset_idx].name;
        ui.set_text(self.gate_preset_btn, &format!("Gate: {}", name));
        name
    }

    pub fn select_gate_preset(&mut self, ui: &mut Ui, name: &str) -> bool {
        if let Some(pos) = GATE_PRESETS
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
        {
            self.gate_preset_idx = pos;
            ui.set_text(
                self.gate_preset_btn,
                &format!("Gate: {}", GATE_PRESETS[pos].name),
            );
            true
        } else {
            false
        }
    }

    pub fn build_active_gate(&self) -> Result<SidechainGate, &'static str> {
        let p = GATE_PRESETS[self.gate_preset_idx % GATE_PRESETS.len()];
        let mut g = SidechainGate::new(
            p.threshold_db,
            p.hysteresis_db,
            p.hold_ms,
            p.attack_ms,
            p.release_ms,
            p.range_db,
        )
        .map_err(|_| "Invalid gate preset")?;
        let _ = g.set_sidechain_hpf_hz(p.sidechain_hp_hz);
        Ok(g)
    }

    pub fn cycle_tape_preset(&mut self, ui: &mut Ui) -> &'static str {
        self.tape_preset_idx = (self.tape_preset_idx + 1) % TAPE_PRESETS.len();
        let name = TAPE_PRESETS[self.tape_preset_idx].name;
        ui.set_text(self.tape_preset_btn, &format!("Tape: {}", name));
        name
    }

    pub fn select_tape_preset(&mut self, ui: &mut Ui, name: &str) -> bool {
        if let Some(pos) = TAPE_PRESETS
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
        {
            self.tape_preset_idx = pos;
            ui.set_text(
                self.tape_preset_btn,
                &format!("Tape: {}", TAPE_PRESETS[pos].name),
            );
            true
        } else {
            false
        }
    }

    pub fn build_active_tape(&self) -> Result<TapeMachine, &'static str> {
        let p = TAPE_PRESETS[self.tape_preset_idx % TAPE_PRESETS.len()];
        TapeMachine::new(
            p.drive,
            p.head_bump_db,
            p.tone_hz,
            p.wow,
            p.flutter,
            p.hiss,
            p.mix,
        )
        .map_err(|_| "Invalid tape preset")
    }

    pub fn cycle_master_preset(&mut self, ui: &mut Ui) -> &'static str {
        self.master_preset_idx = (self.master_preset_idx + 1) % MASTERING_PRESETS.len();
        let name = MASTERING_PRESETS[self.master_preset_idx].name;
        ui.set_text(self.master_preset_btn, &format!("Master: {}", name));
        name
    }

    pub fn select_master_preset(&mut self, ui: &mut Ui, name: &str) -> bool {
        if let Some(pos) = MASTERING_PRESETS
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
        {
            self.master_preset_idx = pos;
            ui.set_text(
                self.master_preset_btn,
                &format!("Master: {}", MASTERING_PRESETS[pos].name),
            );
            true
        } else {
            false
        }
    }

    pub fn build_active_mastering(&self) -> Result<MasteringProcessor, &'static str> {
        let p = MASTERING_PRESETS[self.master_preset_idx % MASTERING_PRESETS.len()];
        let mut m = MasteringProcessor::new().map_err(|_| "Mastering init failed")?;
        m.set_trim_db(p.input_db).map_err(|_| "Invalid trim")?;
        m.set_eq_gains(p.low_db, p.body_db, p.air_db)
            .map_err(|_| "Invalid mastering EQ")?;
        m.set_glue_compressor(
            p.comp_thresh_db,
            p.comp_ratio,
            p.comp_attack_ms,
            p.comp_release_ms,
            p.makeup_db,
        )
        .map_err(|_| "Invalid glue compressor")?;
        m.set_stereo_width(p.stereo_width)
            .map_err(|_| "Invalid stereo width")?;
        m.limiter_mut()
            .set_ceiling_db(p.ceiling_db)
            .map_err(|_| "Invalid limiter ceiling")?;
        m.limiter_mut()
            .set_release_ms(p.limiter_release_ms)
            .map_err(|_| "Invalid limiter release")?;
        Ok(m)
    }

    pub fn default_sidechain_compressor() -> SidechainCompressor {
        SidechainCompressor::new(-24.0, 6.0, 1.0, 80.0, 0.0).unwrap()
    }

    pub fn default_transient() -> TransientShaper {
        TransientShaper::new(45.0, -15.0).unwrap()
    }

    pub fn default_bitcrusher() -> BitCrusher {
        BitCrusher::new(4, 8, 0.75).unwrap()
    }

    pub fn default_chorus() -> StereoChorus {
        StereoChorus::new(1.2, 4.5, 0.25, 0.45).unwrap()
    }

    /// Analyze the loudest 512-frame slice of `track`'s audio and return spectrum + resonances.
    pub fn analyze_track_spectrum(
        track: &Track<'_>,
    ) -> Result<(Vec<f32>, [Option<ResonanceCandidate>; 3]), &'static str> {
        let mut analyzer =
            SpectrumAnalyzer::new(512).map_err(|_| "Spectrum analyzer init failed")?;
        let mut spectrum_db = vec![-120.0f32; analyzer.bin_count()];
        let Some(clip) = track.clips.first() else {
            return Ok((spectrum_db, [None; 3]));
        };
        let src = clip.source_frames();
        let off = clip.source_offset().min(src.len());
        let len = clip.len().min(src.len().saturating_sub(off)).min(8192);
        let slice: &[Frame] = &src[off..off + len];
        if slice.is_empty() {
            return Ok((spectrum_db, [None; 3]));
        }

        // Choose the most energetic 512-frame chunk (handles both steady beds and transients).
        let mut best_slice = slice;
        if slice.len() > 512 {
            let mut best_energy = -1i64;
            for chunk in slice.chunks(512) {
                if chunk.len() < 256 {
                    continue;
                }
                let e: i64 = chunk
                    .iter()
                    .map(|f| (f[0] as i64).abs() + (f[1] as i64).abs())
                    .sum();
                if e > best_energy {
                    best_energy = e;
                    best_slice = chunk;
                }
            }
        }
        analyzer.analyze_frames(best_slice, &mut spectrum_db);
        let resonances = detect_resonances(&spectrum_db, RATE);
        Ok((spectrum_db, resonances))
    }

    /// Render live FFT spectrum + 6-band ParametricEq response curve + resonance cut markers.
    pub fn render_eq_and_spectrum(
        eq: Option<&ParametricEq>,
        spectrum_db: &[f32],
        resonances: &[Option<ResonanceCandidate>; 3],
    ) -> Vec<u32> {
        let w = EQ_WIDTH;
        let h = EQ_HEIGHT;
        let mut pixels = vec![0x141a24u32; w * h];

        // Horizontal 0 dB line
        let mid_y = h / 2;
        for x in 0..w {
            pixels[mid_y * w + x] = 0x2a3648;
        }
        // Log-frequency grid ticks (100 Hz, 1 kHz, 10 kHz across 20..20000 Hz)
        for freq in [100.0f32, 1000.0, 10000.0] {
            let norm = (log10_f32(freq / 20.0) / 3.0).clamp(0.0, 1.0);
            let gx = (norm * (w - 1) as f32) as usize;
            for y in 0..h {
                pixels[y * w + gx] = 0x243042;
            }
        }

        // Spectrum trace
        if spectrum_db.len() >= 2 {
            let fft_size = (spectrum_db.len() - 1) * 2;
            let bin_hz = RATE as f32 / fft_size as f32;
            for x in 0..w {
                let frac = x as f32 / (w - 1) as f32;
                let hz = 20.0 * exp2_f32(frac * 9.965784); // 20 * 1000^frac = 20..20000 Hz
                let bin = ((hz / bin_hz) as usize).min(spectrum_db.len() - 1);
                let db = spectrum_db[bin].clamp(-96.0, 0.0);
                let top_y = (((-db) / 96.0) * (h - 4) as f32) as usize;
                for y in top_y.min(h - 1)..h {
                    pixels[y * w + x] = 0x263f54;
                }
            }
        }

        // 6-Band Parametric EQ curve (-18..+18 dB)
        let mut freqs = Vec::with_capacity(w);
        for x in 0..w {
            let frac = x as f32 / (w - 1) as f32;
            freqs.push(20.0 * exp2_f32(frac * 9.965784));
        }
        let mut db_curve = vec![0.0f32; w];
        if let Some(eq) = eq {
            eq.response_curve_db(&freqs, &mut db_curve);
        }
        for (x, &db) in db_curve.iter().enumerate() {
            let norm = (db / 18.0).clamp(-1.0, 1.0);
            let cy = ((0.5 - norm * 0.45) * (h - 2) as f32) as usize;
            let cy = cy.clamp(1, h - 2);
            pixels[cy * w + x] = 0x38bdf8;
            pixels[(cy + 1) * w + x] = 0x38bdf8;
        }

        // Detected resonance markers
        for cand in resonances.iter().flatten() {
            let norm = (log10_f32((cand.freq_hz / 20.0).max(1.0)) / 3.0).clamp(0.0, 1.0);
            let rx = (norm * (w - 1) as f32) as usize;
            for y in 4..(h - 4) {
                if y % 3 != 0 {
                    pixels[y * w + rx] = 0xf59e0b;
                }
            }
        }

        pixels
    }

    pub fn refresh_gr(&self, ui: &mut Ui, session: &Session) {
        let track_gr = session
            .last_gr_db_q8
            .get(session.selected)
            .copied()
            .unwrap_or(0) as f32
            / 256.0;
        let master_gr = session
            .mastering
            .as_ref()
            .map_or(0.0, |m| m.gain_reduction_db());
        ui.set_text(
            self.gr_readout,
            &format!("GR: {:.1} dB (M {:.1})", track_gr, master_gr),
        );
    }

    pub fn refresh(&mut self, ui: &mut Ui, session: &Session) {
        let track = session.tracks.get(session.selected);
        if let Some(t) = track {
            ui.set_text(
                self.sc_key_btn,
                &match t.sidechain_from {
                    Some(k) => format!("SC Key: T{}", k + 1),
                    None => alloc::string::String::from("SC Key: Off"),
                },
            );
            ui.set_text(
                self.send_btn,
                &match t.post_fader_send {
                    Some((dst, pct)) => format!("Post Send: T{} {}%", dst + 1, pct),
                    None => alloc::string::String::from("Post Send: Off"),
                },
            );
            ui.set_text(
                self.bus_btn,
                &match t.output_bus {
                    Some(bus) => format!("Out Bus: T{}", bus + 1),
                    None => alloc::string::String::from("Out Bus: Master"),
                },
            );
            if let Ok((spec, res)) = Self::analyze_track_spectrum(t) {
                self.last_spectrum_db = spec;
                self.last_resonances = res;
            }
            let pixels = Self::render_eq_and_spectrum(
                t.parametric_eq.as_ref(),
                &self.last_spectrum_db,
                &self.last_resonances,
            );
            ui.set_image(self.eq_image, EQ_WIDTH, EQ_HEIGHT, pixels);
        } else {
            ui.set_text(self.sc_key_btn, "SC Key: Off");
            ui.set_text(self.send_btn, "Post Send: Off");
            ui.set_text(self.bus_btn, "Out Bus: Master");
            let pixels = Self::render_eq_and_spectrum(None, &[], &[None; 3]);
            ui.set_image(self.eq_image, EQ_WIDTH, EQ_HEIGHT, pixels);
        }
        self.refresh_gr(ui, session);
    }
}

/// Cycle an `Option<u8>` track reference (`None -> 0 -> 1 -> ... -> None`), skipping `self_idx`.
pub fn next_other_track(current: Option<u8>, track_count: usize, self_idx: usize) -> Option<u8> {
    let n = track_count.min(MAX_TRACKS);
    if n <= 1 {
        return None;
    }
    let start = match current {
        None => 0,
        Some(v) => v as usize + 1,
    };
    for candidate in start..n {
        if candidate != self_idx {
            return Some(candidate as u8);
        }
    }
    None
}
