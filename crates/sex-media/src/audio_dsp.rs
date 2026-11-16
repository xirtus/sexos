// SPDX-License-Identifier: GPL-3.0-only
//! Real-time sound amplification, 3-band equalizer, dialogue range compression and
//! surround downmix for Qupid's in-play scrub bar. Adapted from brootlite's projector
//! audio DSP so quiet 5.1/7.1 rips can be boosted up to 300% (Kodi's 200% projector
//! lift) without clipping `i16` samples, with zero external math crates in `#![no_std]`.
//!
//! Signal chain per stereo frame (`48_000` Hz default, `8_000..=192_000` Hz supported):
//! 1. Optional `5.1` / `7.1` -> stereo downmix with `+3.2 dB` centre-channel (`FC`) lift
//!    and `LFE` sub-rumble trim (`0.22x`).
//! 2. 3-band RBJ Direct-Form II Transposed biquad cascade:
//!    - Bass low-shelf at `100 Hz` (`S = 1`, `-8..=+8 dB`)
//!    - Voice dialogue peaking filter at `1800 Hz` (`Q = 0.85`, `-8..=+8 dB`)
//!    - Treble high-shelf at `7500 Hz` (`S = 1`, `-8..=+8 dB`)
//! 3. Optional upward/downward envelope Dialogue DRC with a `-54 dBFS` noise gate.
//! 4. Click-free per-frame linear gain ramp (`50%..=300%`) into a smooth Padé `tanh`
//!    soft-knee peak limiter above `0.84` full-scale (`[-32767, 32767]`).
//!
//! At `100%` Flat with DRC off, [`AudioDsp::process_stereo_frames`] takes a zero-work
//! bit-exact fast path once any gain transition has settled.

use alloc::format;
use alloc::string::String;

/// Default SexOS sound ring sample rate (`sex_app::sound::RATE`).
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
/// Minimum volume setting (`50%`).
pub const MIN_BOOST_PERCENT: u16 = 50;
/// Maximum volume amplification (`300%`, `+9.5 dB`).
pub const MAX_BOOST_PERCENT: u16 = 300;
/// Minimum per-band EQ cut in dB.
pub const MIN_BAND_DB: i8 = -8;
/// Maximum per-band EQ boost in dB.
pub const MAX_BAND_DB: i8 = 8;

const PI: f32 = core::f32::consts::PI;
const HALF_PI: f32 = core::f32::consts::FRAC_PI_2;
const LIMITER_KNEE: f32 = 0.84;

/// `10^(dB / 40)` for integer `dB` in `-12..=12` (index `(db + 12) as usize`).
const DB_QUARTER_AMP: [f32; 25] = [
    0.50118723, 0.53088444, 0.56234133, 0.59566214, 0.63095734,
    0.66834392, 0.70794578, 0.74989421, 0.79432823, 0.84139514,
    0.89125094, 0.94406088, 1.00000000, 1.05925373, 1.12201845,
    1.18850223, 1.25892541, 1.33352143, 1.41253754, 1.49623566,
    1.58489319, 1.67880402, 1.77827941, 1.88364909, 1.99526231,
];

/// Named equalizer and dynamics curves in the in-play scrub bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EqPreset {
    Flat,
    Projector,
    Dialogue,
    Cinema,
    Night,
    Custom,
}

impl EqPreset {
    pub const CYCLE_ORDER: [EqPreset; 5] = [
        EqPreset::Flat,
        EqPreset::Projector,
        EqPreset::Dialogue,
        EqPreset::Cinema,
        EqPreset::Night,
    ];

    pub fn id(self) -> &'static str {
        match self {
            EqPreset::Flat => "flat",
            EqPreset::Projector => "projector",
            EqPreset::Dialogue => "dialogue",
            EqPreset::Cinema => "cinema",
            EqPreset::Night => "night",
            EqPreset::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> EqPreset {
        if id.eq_ignore_ascii_case("projector") { EqPreset::Projector }
        else if id.eq_ignore_ascii_case("dialogue") { EqPreset::Dialogue }
        else if id.eq_ignore_ascii_case("cinema") { EqPreset::Cinema }
        else if id.eq_ignore_ascii_case("night") { EqPreset::Night }
        else if id.eq_ignore_ascii_case("custom") { EqPreset::Custom }
        else { EqPreset::Flat }
    }

    pub fn label(self) -> &'static str {
        match self {
            EqPreset::Flat => "Flat",
            EqPreset::Projector => "Projector",
            EqPreset::Dialogue => "Dialogue",
            EqPreset::Cinema => "Cinema Bass",
            EqPreset::Night => "Night DRC",
            EqPreset::Custom => "Custom",
        }
    }

    /// `(bass_db, voice_db, treble_db, dialogue_drc)` for built-in presets.
    pub fn curve(self) -> (i8, i8, i8, bool) {
        match self {
            EqPreset::Flat => (0, 0, 0, false),
            EqPreset::Projector => (2, 5, 2, true),
            EqPreset::Dialogue => (-2, 6, 2, true),
            EqPreset::Cinema => (5, 2, 2, false),
            EqPreset::Night => (-3, 5, 0, true),
            EqPreset::Custom => (0, 0, 0, false),
        }
    }
}

/// User-facing sound boost, 3-band EQ and dialogue DRC settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioDspConfig {
    /// Volume percentage in `50..=300` (`100` = unity, `200` = `2.0x` projector boost).
    pub boost_percent: u16,
    pub preset: EqPreset,
    /// `100 Hz` low-shelf gain in `-8..=8` dB.
    pub bass_db: i8,
    /// `1.8 kHz` voice peaking gain in `-8..=8` dB.
    pub voice_db: i8,
    /// `7.5 kHz` high-shelf gain in `-8..=8` dB.
    pub treble_db: i8,
    /// Upward whisper lift + loud peak taming for quiet rooms and projectors.
    pub dialogue_drc: bool,
}

impl Default for AudioDspConfig {
    fn default() -> Self {
        Self {
            boost_percent: 100,
            preset: EqPreset::Flat,
            bass_db: 0,
            voice_db: 0,
            treble_db: 0,
            dialogue_drc: false,
        }
    }
}

impl AudioDspConfig {
    pub fn normalized(mut self) -> Self {
        self.boost_percent = self.boost_percent.clamp(MIN_BOOST_PERCENT, MAX_BOOST_PERCENT);
        self.bass_db = self.bass_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self.voice_db = self.voice_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self.treble_db = self.treble_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self
    }

    /// True when the whole chain is bit-exact pass-through (`100%` Flat, DRC off).
    pub fn is_identity(&self) -> bool {
        self.boost_percent == 100
            && self.bass_db == 0
            && self.voice_db == 0
            && self.treble_db == 0
            && !self.dialogue_drc
    }

    pub fn apply_preset(&mut self, preset: EqPreset) {
        self.preset = preset;
        if preset != EqPreset::Custom {
            let (b, v, t, drc) = preset.curve();
            self.bass_db = b;
            self.voice_db = v;
            self.treble_db = t;
            self.dialogue_drc = drc;
        }
    }

    pub fn cycle_preset(&mut self) -> EqPreset {
        let idx = EqPreset::CYCLE_ORDER
            .iter()
            .position(|&p| p == self.preset)
            .map_or(0, |i| (i + 1) % EqPreset::CYCLE_ORDER.len());
        let next = EqPreset::CYCLE_ORDER[idx];
        self.apply_preset(next);
        next
    }

    pub fn step_boost(&mut self, delta_percent: i16) -> u16 {
        let next = (self.boost_percent as i32 + delta_percent as i32)
            .clamp(MIN_BOOST_PERCENT as i32, MAX_BOOST_PERCENT as i32) as u16;
        self.boost_percent = next;
        next
    }

    /// One-press Kodi projector toggle between `100%` and `200%`.
    pub fn toggle_boost_200(&mut self) -> u16 {
        self.boost_percent = if self.boost_percent == 200 { 100 } else { 200 };
        self.boost_percent
    }

    pub fn set_bands(&mut self, bass_db: i8, voice_db: i8, treble_db: i8) {
        self.bass_db = bass_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self.voice_db = voice_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self.treble_db = treble_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self.preset = EqPreset::CYCLE_ORDER
            .iter()
            .copied()
            .find(|p| {
                let (b, v, t, drc) = p.curve();
                b == self.bass_db && v == self.voice_db && t == self.treble_db && drc == self.dialogue_drc
            })
            .unwrap_or(EqPreset::Custom);
    }

    /// Compact label for Qupid's scrub-bar button alongside Audio / Subtitles / Next.
    pub fn osd_button_label(&self) -> String {
        if self.is_identity() {
            return String::from("Vol / EQ");
        }
        let eq_active = self.bass_db != 0 || self.voice_db != 0 || self.treble_db != 0 || self.dialogue_drc;
        match (self.boost_percent != 100, eq_active) {
            (true, true) => format!("Vol {}% · {}", self.boost_percent, self.preset.label()),
            (true, false) => format!("Vol {}%", self.boost_percent),
            (false, _) => format!("EQ · {}", self.preset.label()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    bypass: bool,
}

impl Default for BiquadCoeffs {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, bypass: true }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct BiquadState {
    z1: [f32; 2],
    z2: [f32; 2],
}

impl BiquadState {
    #[inline]
    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }

    #[inline]
    fn tick(&mut self, ch: usize, x: f32, c: &BiquadCoeffs) -> f32 {
        if c.bypass {
            return x;
        }
        let y = c.b0 * x + self.z1[ch];
        self.z1[ch] = c.b1 * x - c.a1 * y + self.z2[ch];
        self.z2[ch] = c.b2 * x - c.a2 * y;
        y
    }
}

/// Stateful `#![no_std]` stereo audio processor and surround downmixer.
#[derive(Clone, Debug)]
pub struct AudioDsp {
    sample_rate: u32,
    cfg: AudioDspConfig,
    target_gain: f32,
    current_gain: f32,
    bass: BiquadCoeffs,
    voice: BiquadCoeffs,
    treble: BiquadCoeffs,
    bass_state: BiquadState,
    voice_state: BiquadState,
    treble_state: BiquadState,
    env: f32,
    drc_attack: f32,
    drc_release: f32,
}

impl Default for AudioDsp {
    fn default() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE, AudioDspConfig::default())
    }
}

impl AudioDsp {
    pub fn new(sample_rate: u32, config: AudioDspConfig) -> Self {
        let sr = sample_rate.clamp(8_000, 192_000);
        let cfg = config.normalized();
        let gain = cfg.boost_percent as f32 * 0.01;
        let mut dsp = Self {
            sample_rate: sr,
            cfg,
            target_gain: gain,
            current_gain: gain,
            bass: BiquadCoeffs::default(),
            voice: BiquadCoeffs::default(),
            treble: BiquadCoeffs::default(),
            bass_state: BiquadState::default(),
            voice_state: BiquadState::default(),
            treble_state: BiquadState::default(),
            env: 0.0,
            drc_attack: one_pole_coeff(sr, 0.012),
            drc_release: one_pole_coeff(sr, 0.180),
        };
        dsp.rebuild_filters();
        dsp
    }

    pub fn config(&self) -> AudioDspConfig {
        self.cfg
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let sr = sample_rate.clamp(8_000, 192_000);
        if sr != self.sample_rate {
            self.sample_rate = sr;
            self.drc_attack = one_pole_coeff(sr, 0.012);
            self.drc_release = one_pole_coeff(sr, 0.180);
            self.reset_state();
            self.rebuild_filters();
        }
    }

    pub fn set_config(&mut self, config: AudioDspConfig) {
        let next = config.normalized();
        let filters_changed = next.bass_db != self.cfg.bass_db
            || next.voice_db != self.cfg.voice_db
            || next.treble_db != self.cfg.treble_db;
        // A band at 0 dB is bypassed and its delay line stops; one switched on again
        // starts from silence instead of replaying old samples.
        if (next.bass_db == 0) != (self.cfg.bass_db == 0) { self.bass_state.reset(); }
        if (next.voice_db == 0) != (self.cfg.voice_db == 0) { self.voice_state.reset(); }
        if (next.treble_db == 0) != (self.cfg.treble_db == 0) { self.treble_state.reset(); }
        self.cfg = next;
        self.target_gain = next.boost_percent as f32 * 0.01;
        if filters_changed {
            self.rebuild_filters();
        }
    }

    /// Clears filter delay lines and DRC envelope on seek or track switch.
    pub fn reset_state(&mut self) {
        self.bass_state.reset();
        self.voice_state.reset();
        self.treble_state.reset();
        self.env = 0.0;
        self.current_gain = self.target_gain;
    }

    fn rebuild_filters(&mut self) {
        let fs = self.sample_rate as f32;
        self.bass = design_low_shelf(fs, 100.0, self.cfg.bass_db);
        self.voice = design_peaking(fs, 1800.0, 0.85, self.cfg.voice_db);
        self.treble = design_high_shelf(fs, 7500.0, self.cfg.treble_db);
    }

    /// Processes `[[i16; 2]]` stereo PCM frames in-place (`sex_media` native layout).
    pub fn process_stereo_frames(&mut self, frames: &mut [[i16; 2]]) {
        if frames.is_empty() {
            return;
        }
        if self.cfg.is_identity() && abs_f32(self.current_gain - 1.0) < 1e-5 {
            self.current_gain = 1.0;
            return;
        }

        let gain_step = (self.target_gain - self.current_gain) / (frames.len() as f32);
        let eq_active = !self.bass.bypass || !self.voice.bypass || !self.treble.bypass;
        let drc_active = self.cfg.dialogue_drc;

        for frame in frames.iter_mut() {
            self.current_gain += gain_step;
            let mut l = frame[0] as f32 * (1.0 / 32768.0);
            let mut r = frame[1] as f32 * (1.0 / 32768.0);

            if eq_active {
                l = self.bass_state.tick(0, l, &self.bass);
                l = self.voice_state.tick(0, l, &self.voice);
                l = self.treble_state.tick(0, l, &self.treble);

                r = self.bass_state.tick(1, r, &self.bass);
                r = self.voice_state.tick(1, r, &self.voice);
                r = self.treble_state.tick(1, r, &self.treble);
            }

            let mut gain = self.current_gain;
            if drc_active {
                gain *= self.compute_drc_gain(l, r);
            }

            frame[0] = float_to_i16_soft_limited(l * gain);
            frame[1] = float_to_i16_soft_limited(r * gain);
        }
        self.current_gain = self.target_gain;
    }

    /// Processes interleaved mono (`channels == 1`) or stereo (`channels == 2`) `i16` PCM.
    pub fn process_interleaved_i16(&mut self, samples: &mut [i16], channels: usize) {
        if samples.is_empty() || (channels != 1 && channels != 2) {
            return;
        }
        if self.cfg.is_identity() && abs_f32(self.current_gain - 1.0) < 1e-5 {
            self.current_gain = 1.0;
            return;
        }
        let n_frames = samples.len() / channels;
        if n_frames == 0 {
            return;
        }
        let gain_step = (self.target_gain - self.current_gain) / (n_frames as f32);
        let eq_active = !self.bass.bypass || !self.voice.bypass || !self.treble.bypass;
        let drc_active = self.cfg.dialogue_drc;

        if channels == 2 {
            for pair in samples.chunks_exact_mut(2) {
                self.current_gain += gain_step;
                let mut l = pair[0] as f32 * (1.0 / 32768.0);
                let mut r = pair[1] as f32 * (1.0 / 32768.0);
                if eq_active {
                    l = self.treble_state.tick(0, self.voice_state.tick(0, self.bass_state.tick(0, l, &self.bass), &self.voice), &self.treble);
                    r = self.treble_state.tick(1, self.voice_state.tick(1, self.bass_state.tick(1, r, &self.bass), &self.voice), &self.treble);
                }
                let mut gain = self.current_gain;
                if drc_active {
                    gain *= self.compute_drc_gain(l, r);
                }
                pair[0] = float_to_i16_soft_limited(l * gain);
                pair[1] = float_to_i16_soft_limited(r * gain);
            }
        } else {
            for s in samples.iter_mut() {
                self.current_gain += gain_step;
                let mut x = *s as f32 * (1.0 / 32768.0);
                if eq_active {
                    x = self.treble_state.tick(0, self.voice_state.tick(0, self.bass_state.tick(0, x, &self.bass), &self.voice), &self.treble);
                }
                let mut gain = self.current_gain;
                if drc_active {
                    gain *= self.compute_drc_gain(x, x);
                }
                *s = float_to_i16_soft_limited(x * gain);
            }
        }
        self.current_gain = self.target_gain;
    }

    /// Downmixes interleaved `5.1` (`6` ch: `FL, FR, FC, LFE, BL, BR`) or `7.1` (`8` ch:
    /// `FL, FR, FC, LFE, BL, BR, SL, SR`) `i16` PCM into stereo frames with a `+3.2 dB`
    /// centre-channel dialogue lift (`FC * 1.02` vs standard `0.707`) and `LFE` trim,
    /// then applies the configured EQ, DRC, volume boost and soft-knee limiter.
    /// Returns the number of stereo frames written to `out_frames`.
    pub fn downmix_surround_to_stereo(
        &mut self,
        interleaved_src: &[i16],
        channels: usize,
        out_frames: &mut [[i16; 2]],
    ) -> usize {
        if channels == 0 {
            return 0;
        }
        let count = (interleaved_src.len() / channels).min(out_frames.len());
        const INV_I16: f32 = 1.0 / 32768.0;
        // FC lifted +3.2 dB above -3 dB pan (`1.02` vs `0.7071`), normalized by `0.54`
        // so full-scale 5.1/7.1 bursts stay clean before user amplification.
        for (i, out) in out_frames[..count].iter_mut().enumerate() {
            let base = i * channels;
            let (l, r) = match channels {
                1 => {
                    let m = interleaved_src[base];
                    (m, m)
                }
                2 => (interleaved_src[base], interleaved_src[base + 1]),
                6 => {
                    let fl = interleaved_src[base] as f32 * INV_I16;
                    let fr = interleaved_src[base + 1] as f32 * INV_I16;
                    let fc = interleaved_src[base + 2] as f32 * INV_I16;
                    let lfe = interleaved_src[base + 3] as f32 * INV_I16;
                    let bl = interleaved_src[base + 4] as f32 * INV_I16;
                    let br = interleaved_src[base + 5] as f32 * INV_I16;
                    let center_lfe = fc * 1.02 + lfe * 0.22;
                    let dl = (fl * 0.82 + center_lfe + bl * 0.50) * 0.54;
                    let dr = (fr * 0.82 + center_lfe + br * 0.50) * 0.54;
                    (float_to_i16_soft_limited(dl), float_to_i16_soft_limited(dr))
                }
                8 => {
                    let fl = interleaved_src[base] as f32 * INV_I16;
                    let fr = interleaved_src[base + 1] as f32 * INV_I16;
                    let fc = interleaved_src[base + 2] as f32 * INV_I16;
                    let lfe = interleaved_src[base + 3] as f32 * INV_I16;
                    let bl = interleaved_src[base + 4] as f32 * INV_I16;
                    let br = interleaved_src[base + 5] as f32 * INV_I16;
                    let sl = interleaved_src[base + 6] as f32 * INV_I16;
                    let sr = interleaved_src[base + 7] as f32 * INV_I16;
                    let center_lfe = fc * 1.02 + lfe * 0.22;
                    let dl = (fl * 0.80 + center_lfe + (bl + sl) * 0.38) * 0.50;
                    let dr = (fr * 0.80 + center_lfe + (br + sr) * 0.38) * 0.50;
                    (float_to_i16_soft_limited(dl), float_to_i16_soft_limited(dr))
                }
                _ => {
                    let l = interleaved_src[base];
                    let r = interleaved_src[base + 1.min(channels - 1)];
                    (l, r)
                }
            };
            *out = [l, r];
        }
        self.process_stereo_frames(&mut out_frames[..count]);
        count
    }

    #[inline]
    fn compute_drc_gain(&mut self, l: f32, r: f32) -> f32 {
        let peak = abs_f32(l).max(abs_f32(r));
        let coeff = if peak > self.env { self.drc_attack } else { self.drc_release };
        self.env += coeff * (peak - self.env);
        // -54 dBFS noise gate (`0.0020`): never pump room hiss during silent pauses.
        if self.env < 0.0020 {
            1.0
        } else if self.env < 0.12 {
            // Smooth upward dialogue lift up to ~2.1x (+6.4 dB) on quiet speech,
            // ramping cleanly from 1.0 at the -54 dBFS gate up to peak lift around -28 dBFS.
            let gate_ramp = ((self.env - 0.0020) * (1.0 / 0.018)).clamp(0.0, 1.0);
            let lift = 1.0 + (0.12 - self.env) * (1.10 / 0.12);
            1.0 + gate_ramp * (lift - 1.0)
        } else if self.env > 0.50 {
            // Gentle downward taming on sudden explosions above -6 dBFS.
            // Output level 0.50 + 0.45 * (env - 0.50), as a gain: 1.0 at the threshold.
            (0.50 + (self.env - 0.50) * 0.45) / self.env
        } else {
            1.0
        }
    }
}

#[inline]
fn abs_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

/// Exact `#![no_std]` IEEE-754 square root (bit-seed + 3 Newton-Raphson steps).
#[inline]
fn sqrt_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f32::from_bits(0x1fbd_1df5 + (x.to_bits() >> 1));
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    0.5 * (y + x / y)
}

/// Accurate `(sin(w), cos(w))` for `w` in `[0, PI]` using range reduction to `[0, PI/2]`.
#[inline]
fn sin_cos_omega(w: f32) -> (f32, f32) {
    let w = w.clamp(0.0, PI);
    let (t, cos_sign) = if w > HALF_PI { (PI - w, -1.0f32) } else { (w, 1.0f32) };
    let t2 = t * t;
    let sin_t = t * (1.0 + t2 * (-1.0 / 6.0 + t2 * (1.0 / 120.0 + t2 * (-1.0 / 5040.0 + t2 * (1.0 / 362_880.0)))));
    let cos_t = 1.0 + t2 * (-0.5 + t2 * (1.0 / 24.0 + t2 * (-1.0 / 720.0 + t2 * (1.0 / 40_320.0))));
    (sin_t, cos_sign * cos_t)
}

#[inline]
fn db_quarter_amp(db: i8) -> f32 {
    let idx = (db.clamp(-12, 12) + 12) as usize;
    DB_QUARTER_AMP[idx]
}

#[inline]
fn one_pole_coeff(sample_rate: u32, tau_seconds: f32) -> f32 {
    let x = 1.0 / (sample_rate as f32 * tau_seconds).max(1.0);
    // `1 - exp(-x)` via [2/2] Padé on small positive `x`.
    let x2 = x * x;
    let exp_neg = (1.0 - 0.5 * x + x2 * (1.0 / 12.0)) / (1.0 + 0.5 * x + x2 * (1.0 / 12.0));
    (1.0 - exp_neg).clamp(0.000_01, 1.0)
}

fn design_low_shelf(fs: f32, f0: f32, gain_db: i8) -> BiquadCoeffs {
    if gain_db == 0 {
        return BiquadCoeffs::default();
    }
    let a = db_quarter_amp(gain_db);
    let w0 = (2.0 * PI * f0 / fs).clamp(0.0005, PI * 0.92);
    let (sin_w0, cos_w0) = sin_cos_omega(w0);
    // Shelf slope `S = 1.0` -> `alpha = sin(w0)/2 * sqrt(2) = sin(w0) * 0.70710678`.
    let two_sqrt_a_alpha = 2.0 * sqrt_f32(a) * (sin_w0 * 0.70710678);
    let ap1 = a + 1.0;
    let am1 = a - 1.0;
    let a0 = ap1 + am1 * cos_w0 + two_sqrt_a_alpha;
    let inv_a0 = 1.0 / a0;
    BiquadCoeffs {
        b0: a * (ap1 - am1 * cos_w0 + two_sqrt_a_alpha) * inv_a0,
        b1: 2.0 * a * (am1 - ap1 * cos_w0) * inv_a0,
        b2: a * (ap1 - am1 * cos_w0 - two_sqrt_a_alpha) * inv_a0,
        a1: -2.0 * (am1 + ap1 * cos_w0) * inv_a0,
        a2: (ap1 + am1 * cos_w0 - two_sqrt_a_alpha) * inv_a0,
        bypass: false,
    }
}

fn design_peaking(fs: f32, f0: f32, q: f32, gain_db: i8) -> BiquadCoeffs {
    if gain_db == 0 {
        return BiquadCoeffs::default();
    }
    let a = db_quarter_amp(gain_db);
    let w0 = (2.0 * PI * f0 / fs).clamp(0.0005, PI * 0.92);
    let (sin_w0, cos_w0) = sin_cos_omega(w0);
    let alpha = sin_w0 / (2.0 * q);
    let a0 = 1.0 + alpha / a;
    let inv_a0 = 1.0 / a0;
    BiquadCoeffs {
        b0: (1.0 + alpha * a) * inv_a0,
        b1: (-2.0 * cos_w0) * inv_a0,
        b2: (1.0 - alpha * a) * inv_a0,
        a1: (-2.0 * cos_w0) * inv_a0,
        a2: (1.0 - alpha / a) * inv_a0,
        bypass: false,
    }
}

fn design_high_shelf(fs: f32, f0: f32, gain_db: i8) -> BiquadCoeffs {
    if gain_db == 0 {
        return BiquadCoeffs::default();
    }
    let a = db_quarter_amp(gain_db);
    let w0 = (2.0 * PI * f0 / fs).clamp(0.0005, PI * 0.92);
    let (sin_w0, cos_w0) = sin_cos_omega(w0);
    let two_sqrt_a_alpha = 2.0 * sqrt_f32(a) * (sin_w0 * 0.70710678);
    let ap1 = a + 1.0;
    let am1 = a - 1.0;
    let a0 = ap1 - am1 * cos_w0 + two_sqrt_a_alpha;
    let inv_a0 = 1.0 / a0;
    BiquadCoeffs {
        b0: a * (ap1 + am1 * cos_w0 + two_sqrt_a_alpha) * inv_a0,
        b1: -2.0 * a * (am1 + ap1 * cos_w0) * inv_a0,
        b2: a * (ap1 + am1 * cos_w0 - two_sqrt_a_alpha) * inv_a0,
        a1: 2.0 * (am1 - ap1 * cos_w0) * inv_a0,
        a2: (ap1 - am1 * cos_w0 - two_sqrt_a_alpha) * inv_a0,
        bypass: false,
    }
}

/// Linear below `0.84` full-scale; smooth `C^1` Padé `tanh` knee above `0.84` into `[-32767, 32767]`.
#[inline]
pub fn float_to_i16_soft_limited(x: f32) -> i16 {
    if x.is_nan() {
        return 0;
    }
    let ax = abs_f32(x);
    let y = if ax <= LIMITER_KNEE {
        x
    } else {
        let headroom = 1.0 - LIMITER_KNEE;
        let u = (ax - LIMITER_KNEE) / headroom;
        let shaped = if u >= 3.0 {
            1.0
        } else {
            let u2 = u * u;
            u * (27.0 + u2) / (27.0 + 9.0 * u2)
        };
        let mag = LIMITER_KNEE + headroom * shaped;
        if x < 0.0 { -mag } else { mag }
    };
    let scaled = y * 32767.0;
    let rounded = if scaled >= 0.0 { scaled + 0.5 } else { scaled - 0.5 };
    (rounded as i32).clamp(-32767, 32767) as i16
}
