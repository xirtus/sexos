// SPDX-License-Identifier: GPL-3.0-only
//! Prism Wavetable: Suno Studio 2.0 dual morphing wavetable synthesizer.
//! Features Basic Analog, Harmonic/Formant, and Digital/FM tables with phase warp,
//! resonant lowpass filter, and visual curve rendering for Weft UI.

use crate::{
    dsp_math::{floor_f32, sin_f32, PI},
    Frame, RATE,
};
use alloc::vec::Vec;

pub const TABLE_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrismParams {
    pub osc_a_pos: u8,   // 0..255 morph position in wavetable A
    pub osc_b_pos: u8,   // 0..255 morph position in wavetable B
    pub warp: u8,        // 0..255 phase warp / bend
    pub cutoff: u16,     // 20..20000 Hz filter cutoff
    pub resonance: u8,   // 0..100 filter resonance
    pub mix_ab: u8,      // 0..100 mix (0 = 100% Osc A, 100 = 100% Osc B)
}

impl Default for PrismParams {
    fn default() -> Self {
        Self {
            osc_a_pos: 0,
            osc_b_pos: 128,
            warp: 0,
            cutoff: 8000,
            resonance: 20,
            mix_ab: 50,
        }
    }
}

/// Sample the visual waveform curve across one full cycle for Weft UI rendering.
pub fn preview_curve(params: &PrismParams, points: &mut [i16]) {
    if points.is_empty() {
        return;
    }
    let n = points.len();
    let mix_b = params.mix_ab.min(100) as f32 / 100.0;
    let mix_a = 1.0 - mix_b;

    for (i, p) in points.iter_mut().enumerate() {
        let mut phase = i as f32 / n as f32;
        phase = apply_warp(phase, params.warp);

        let sa = sample_morph_bank(0, params.osc_a_pos, phase);
        let sb = sample_morph_bank(1, params.osc_b_pos, phase);
        let sample = sa * mix_a + sb * mix_b;

        *p = (sample * 28000.0).clamp(-32768.0, 32767.0) as i16;
    }
}

/// Apply phase warp / bend distortion to normalized phase [0, 1).
#[inline]
fn apply_warp(phase: f32, warp: u8) -> f32 {
    if warp == 0 {
        return phase;
    }
    let w = warp as f32 / 255.0;
    if w <= 0.5 {
        // Bend towards start of cycle
        let mut p = phase;
        for _ in 0..2 {
            p = p * (1.0 + w) - w * p * p;
        }
        p.clamp(0.0, 0.999999)
    } else {
        // Asymmetric pinch
        let center = 0.5;
        let factor = 1.0 + (w - 0.5) * 4.0;
        if phase < center {
            (phase / center) * center / factor
        } else {
            1.0 - ((1.0 - phase) / (1.0 - center)) * (1.0 - center) / factor
        }
    }
}

/// Sample one of three wavetable banks (0: Analog, 1: Harmonic/Formant, 2: Digital/FM).
fn sample_morph_bank(bank: usize, pos: u8, phase: f32) -> f32 {
    let norm_pos = (pos as f32 / 255.0) * 3.0; // 4 keyframes: 0, 1, 2, 3
    let frame_idx = floor_f32(norm_pos) as usize;
    let frac = norm_pos - frame_idx as f32;

    let f0 = frame_idx.min(3);
    let f1 = (f0 + 1).min(3);

    let s0 = sample_keyframe(bank, f0, phase);
    let s1 = sample_keyframe(bank, f1, phase);
    s0 * (1.0 - frac) + s1 * frac
}

/// Analytical mathematical generation of keyframes for zero memory overhead.
fn sample_keyframe(bank: usize, frame: usize, phase: f32) -> f32 {
    let p = phase - floor_f32(phase);
    let angle = 2.0 * PI * p;

    match (bank % 3, frame) {
        // Bank 0: Analog (Sine -> Triangle -> Saw -> Pulse)
        (0, 0) => sin_f32(angle),
        (0, 1) => {
            // Triangle
            if p < 0.25 {
                p * 4.0
            } else if p < 0.75 {
                2.0 - p * 4.0
            } else {
                p * 4.0 - 4.0
            }
        }
        (0, 2) => 1.0 - 2.0 * p, // Sawtooth
        (0, 3) => if p < 0.5 { 0.9 } else { -0.9 }, // Square / Pulse

        // Bank 1: Harmonic / Formant
        (1, 0) => {
            // Organ / Octaves (1st + 2nd + 4th harmonic)
            0.6 * sin_f32(angle) + 0.3 * sin_f32(2.0 * angle) + 0.15 * sin_f32(4.0 * angle)
        }
        (1, 1) => {
            // Vocal Formant "AA"
            0.5 * sin_f32(angle) + 0.4 * sin_f32(3.0 * angle) + 0.3 * sin_f32(5.0 * angle)
        }
        (1, 2) => {
            // Vocal Formant "EE"
            0.4 * sin_f32(angle) + 0.1 * sin_f32(2.0 * angle) + 0.5 * sin_f32(7.0 * angle)
        }
        (1, 3) => {
            // Resonant Bell Harmonic
            0.3 * sin_f32(angle) + 0.3 * sin_f32(4.0 * angle) + 0.25 * sin_f32(9.0 * angle) + 0.15 * sin_f32(11.0 * angle)
        }

        // Bank 2: Digital / FM
        (2, 0) => {
            // Chime FM 1:1.414
            sin_f32(angle + 1.2 * sin_f32(1.414 * angle))
        }
        (2, 1) => {
            // Bell FM 1:2
            sin_f32(angle + 2.0 * sin_f32(2.0 * angle))
        }
        (2, 2) => {
            // Metallic FM 1:3.5
            sin_f32(angle + 2.5 * sin_f32(3.5 * angle))
        }
        _ => {
            // Gritty Digital
            let s = sin_f32(angle) + 0.5 * sin_f32(3.0 * angle);
            if s > 0.4 { 0.8 } else if s < -0.4 { -0.8 } else { s * 2.0 }
        }
    }
}

/// 2-pole state variable filter (SVF) for lowpass filtering.
#[derive(Clone, Copy, Debug, Default)]
pub struct SvfFilter {
    ic1eq: f32,
    ic2eq: f32,
}

impl SvfFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, input: f32, cutoff: f32, res: f32, sr: f32) -> f32 {
        let f = (PI * cutoff / sr).clamp(0.001, 0.49);
        let q = 1.0 / (0.1 + (res.clamp(0.0, 100.0) / 100.0) * 9.9);
        let g = f;
        let k = q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        v2 // lowpass output
    }

    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}

/// Render a single note for the Prism Wavetable instrument.
pub fn render_prism_note(
    key: u8,
    velocity: u8,
    frames: u32,
    params: &PrismParams,
) -> Result<Vec<Frame>, crate::midi::Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(frames as usize)
        .map_err(|_| crate::midi::Error::Memory)?;

    let sr = RATE as f32;
    // Calculate pitch from MIDI key
    let freq = 440.0 * crate::dsp_math::exp2_f32((key as f32 - 69.0) / 12.0);
    let phase_inc = freq / sr;
    let mut phase = 0.0f32;

    let mix_b = params.mix_ab.min(100) as f32 / 100.0;
    let mix_a = 1.0 - mix_b;

    let mut filter = SvfFilter::new();
    let cutoff = params.cutoff.clamp(20, 20000) as f32;
    let resonance = params.resonance as f32;

    for i in 0..frames {
        let warped_phase = apply_warp(phase, params.warp);
        let sa = sample_morph_bank(0, params.osc_a_pos, warped_phase);
        let sb = sample_morph_bank(1, params.osc_b_pos, warped_phase);
        let mixed = sa * mix_a + sb * mix_b;

        let filtered = filter.process(mixed, cutoff, resonance, sr);

        // Linear envelope: 2ms attack, 10ms release
        let attack = ((i as f32 * 1000.0) / (sr * 2.0)).min(1.0);
        let release = (((frames - 1 - i) as f32 * 1000.0) / (sr * 10.0)).min(1.0);
        let env = attack * release;

        let val = (filtered * env * (velocity as f32 / 127.0) * 16000.0).clamp(-32768.0, 32767.0) as i16;
        out.push([val, val]);

        phase += phase_inc;
        if phase >= 1.0 {
            phase -= floor_f32(phase);
        }
    }

    Ok(out)
}
