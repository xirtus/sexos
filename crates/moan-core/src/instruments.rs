// SPDX-License-Identifier: GPL-3.0-only
//! Moan's expressive `#![no_std]` synthesized, circuit-modeled, and physical-modeled
//! instruments at 48 kHz (`RATE`).
//!
//! All heap buffers are pre-allocated with `try_reserve_exact` before synthesis and
//! return `midi::Error::Memory` on OOM, with zero allocations inside per-sample loops.

use crate::{
    dsp_math::{
        abs_f32, cos_f32, exp2_f32, exp_f32, floor_f32, powf_f32, sin_f32, sqrt_f32, tan_f32,
        FRAC_PI_2, PI, TAU,
    },
    midi::{self, Instrument, Note, MAX_FRAMES, MAX_NOTES, MAX_VOICES},
    Frame, RATE,
};
use alloc::vec::Vec;

const SR: f32 = RATE as f32;
const SR_INV: f32 = 1.0 / SR;

// ============================================================================
// 1. Per-Note Expression & Unified Rendering API
// ============================================================================

/// Per-note continuous expression controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteExpr {
    /// Pitch bend offset in semitones (`-48.0..=48.0`).
    pub pitch_bend_semitones: f32,
    /// Polyphonic pressure / aftertouch (`0.0..=1.0`).
    pub pressure: f32,
    /// Per-note timbre / brightness (`0.0..=1.0`, `0.5` neutral).
    pub timbre: f32,
    /// Optional starting MIDI key for explicit note-entry pitch slide.
    pub slide_from_key: Option<u8>,
    /// Explicit glide duration in frames (if `0`, instrument `glide_ms` is used).
    pub glide_frames: u32,
}

impl Default for NoteExpr {
    fn default() -> Self {
        Self {
            pitch_bend_semitones: 0.0,
            pressure: 0.0,
            timbre: 0.5,
            slide_from_key: None,
            glide_frames: 0,
        }
    }
}

impl NoteExpr {
    #[inline]
    fn sanitized(self) -> Self {
        Self {
            pitch_bend_semitones: clamp_finite(self.pitch_bend_semitones, -48.0, 48.0),
            pressure: clamp_finite(self.pressure, 0.0, 1.0),
            timbre: if self.timbre.is_finite() {
                self.timbre.clamp(0.0, 1.0)
            } else {
                0.5
            },
            slide_from_key: self.slide_from_key.map(|k| k.min(127)),
            glide_frames: self.glide_frames,
        }
    }
}

/// Note paired with per-note expression.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpressiveNote {
    pub note: Note,
    pub expr: NoteExpr,
}

impl ExpressiveNote {
    pub const fn new(note: Note, expr: NoteExpr) -> Self {
        Self { note, expr }
    }
}

impl From<Note> for ExpressiveNote {
    fn from(note: Note) -> Self {
        Self {
            note,
            expr: NoteExpr::default(),
        }
    }
}

/// Custom parameter bundle for any of Moan's expressive instruments (`5..=11`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstrumentParams {
    Sub808(Sub808),
    DrumKit(DrumKit),
    TineKeys(TineKeys),
    ModalGrand(ModalGrand),
    PluckGuitar(PluckGuitar),
    ModalMallet(ModalMallet),
    CrossSynth(CrossSynth),
}

impl InstrumentParams {
    /// Return default parameters for an `Instrument` variant (`5..=11`), or `None` for legacy `0..=4`.
    pub fn for_instrument(inst: Instrument) -> Option<Self> {
        match inst {
            Instrument::Sub808 => Some(Self::Sub808(Sub808::default())),
            Instrument::DrumKit => Some(Self::DrumKit(DrumKit::default())),
            Instrument::TineKeys => Some(Self::TineKeys(TineKeys::default())),
            Instrument::ModalGrand => Some(Self::ModalGrand(ModalGrand::default())),
            Instrument::PluckGuitar => Some(Self::PluckGuitar(PluckGuitar::default())),
            Instrument::ModalMallet => Some(Self::ModalMallet(ModalMallet::default())),
            Instrument::CrossSynth => Some(Self::CrossSynth(CrossSynth::default())),
            _ => None,
        }
    }
}

/// Render expressive notes with custom instrument parameters into stereo 48 kHz PCM.
pub fn render_notes_with_params(
    notes: &[ExpressiveNote],
    total_frames: u32,
    params: &InstrumentParams,
) -> Result<Vec<Frame>, midi::Error> {
    match params {
        InstrumentParams::Sub808(p) => p.render_expressive(notes, total_frames),
        InstrumentParams::DrumKit(p) => p.render_expressive(notes, total_frames),
        InstrumentParams::TineKeys(p) => p.render_expressive(notes, total_frames),
        InstrumentParams::ModalGrand(p) => p.render_expressive(notes, total_frames),
        InstrumentParams::PluckGuitar(p) => p.render_expressive(notes, total_frames),
        InstrumentParams::ModalMallet(p) => p.render_expressive(notes, total_frames),
        InstrumentParams::CrossSynth(p) => p.render_expressive(notes, total_frames),
    }
}

/// Render plain MIDI notes with custom instrument parameters into stereo 48 kHz PCM.
pub fn render_pattern_with_params(
    notes: &[Note],
    total_frames: u32,
    params: &InstrumentParams,
) -> Result<Vec<Frame>, midi::Error> {
    let expressive = notes_to_expressive(notes)?;
    render_notes_with_params(&expressive, total_frames, params)
}

/// Hook called by `midi::Pattern::bake` for instruments `5..=11`.
pub(crate) fn bake_instrument_pattern(
    pattern: &midi::Pattern,
) -> Result<Option<Vec<Frame>>, midi::Error> {
    let pcm = match pattern.instrument {
        Instrument::Sub808 => Sub808::default().render_pattern(&pattern.notes, pattern.frames)?,
        Instrument::DrumKit => DrumKit::default().render_pattern(&pattern.notes, pattern.frames)?,
        Instrument::TineKeys => TineKeys::default().render_pattern(&pattern.notes, pattern.frames)?,
        Instrument::ModalGrand => {
            ModalGrand::default().render_pattern(&pattern.notes, pattern.frames)?
        }
        Instrument::PluckGuitar => {
            PluckGuitar::default().render_pattern(&pattern.notes, pattern.frames)?
        }
        Instrument::ModalMallet => {
            ModalMallet::default().render_pattern(&pattern.notes, pattern.frames)?
        }
        Instrument::CrossSynth => {
            CrossSynth::default().render_pattern(&pattern.notes, pattern.frames)?
        }
        _ => return Ok(None),
    };
    Ok(Some(pcm))
}

// ============================================================================
// 2. Bounded Allocation & Core `#![no_std]` DSP Primitives
// ============================================================================

fn try_alloc_vec<T: Clone>(len: usize, fill: T) -> Result<Vec<T>, midi::Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| midi::Error::Memory)?;
    v.resize(len, fill);
    Ok(v)
}

fn notes_to_expressive(notes: &[Note]) -> Result<Vec<ExpressiveNote>, midi::Error> {
    if notes.len() > MAX_NOTES {
        return Err(midi::Error::Range);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| midi::Error::Memory)?;
    for &n in notes {
        out.push(ExpressiveNote::from(n));
    }
    Ok(out)
}

fn validate_expressive(notes: &[ExpressiveNote], total_frames: u32) -> Result<(), midi::Error> {
    if total_frames == 0 || total_frames > MAX_FRAMES || notes.len() > MAX_NOTES {
        return Err(midi::Error::Range);
    }
    for en in notes {
        let n = en.note;
        if n.key > 127
            || !(1..=127).contains(&n.velocity)
            || n.frames == 0
            || n.start
                .checked_add(n.frames)
                .is_none_or(|end| end > total_frames)
        {
            return Err(midi::Error::Note);
        }
    }
    for en in notes {
        let n = en.note;
        let active = notes
            .iter()
            .filter(|other| {
                let o = other.note;
                o.start <= n.start && o.start + o.frames > n.start
            })
            .count();
        if active > MAX_VOICES {
            return Err(midi::Error::Polyphony);
        }
    }
    Ok(())
}

fn sorted_expressive(notes: &[ExpressiveNote]) -> Result<Vec<ExpressiveNote>, midi::Error> {
    let mut sorted = Vec::new();
    sorted
        .try_reserve_exact(notes.len())
        .map_err(|_| midi::Error::Memory)?;
    for &en in notes {
        sorted.push(ExpressiveNote {
            note: en.note,
            expr: en.expr.sanitized(),
        });
    }
    sorted.sort_by_key(|en| en.note.start);
    Ok(sorted)
}

fn mix_to_frames(mix: &[[f32; 2]], scale: f32, soft_limit: bool) -> Result<Vec<Frame>, midi::Error> {
    let mut pcm = Vec::new();
    pcm.try_reserve_exact(mix.len())
        .map_err(|_| midi::Error::Memory)?;
    for &[l, r] in mix {
        let sl = sanitize_audio(l);
        let sr_ch = sanitize_audio(r);
        let (ol, or) = if soft_limit {
            (soft_ceiling(sl * scale), soft_ceiling(sr_ch * scale))
        } else {
            (sl * scale, sr_ch * scale)
        };
        pcm.push([
            (ol * 32767.0).clamp(-32768.0, 32767.0) as i16,
            (or * 32767.0).clamp(-32768.0, 32767.0) as i16,
        ]);
    }
    Ok(pcm)
}

#[inline]
fn soft_ceiling(x: f32) -> f32 {
    let ax = abs_f32(x);
    if ax <= 0.90 {
        x
    } else {
        let excess = ax - 0.90;
        let shaped = 0.90 + 0.10 * tanh_f32(excess * 10.0);
        if x < 0.0 {
            -shaped
        } else {
            shaped
        }
    }
}

#[inline]
fn sanitize_audio(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

#[inline]
fn clamp_finite(x: f32, lo: f32, hi: f32) -> f32 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        lo.max(0.0_f32.min(hi))
    }
}

#[inline]
fn midi_to_hz(semi: f32) -> f32 {
    440.0 * exp2_f32((semi.clamp(-24.0, 144.0) - 69.0) / 12.0)
}

#[inline]
fn tanh_f32(x: f32) -> f32 {
    if !x.is_finite() {
        if x.is_nan() {
            return 0.0;
        }
        return if x > 0.0 { 1.0 } else { -1.0 };
    }
    if x > 10.0 {
        1.0
    } else if x < -10.0 {
        -1.0
    } else {
        let e2x = exp_f32(2.0 * x);
        (e2x - 1.0) / (e2x + 1.0)
    }
}

#[inline]
fn next_noise(state: &mut u32) -> f32 {
    let mut x = *state;
    if x == 0 {
        x = 0x9E37_79B9;
    }
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    ((x >> 8) as i32 as f32 / 8_388_608.0) - 1.0
}

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 || dt >= 0.5 {
        return 0.0;
    }
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[inline]
fn wrap_phase(p: f32) -> f32 {
    if !(0.0..1.0).contains(&p) {
        p - floor_f32(p)
    } else {
        p
    }
}

/// Topology-preserving 2-pole state variable filter (12 dB/oct).
#[derive(Clone, Copy, Debug, Default)]
struct TptSvf {
    ic1eq: f32,
    ic2eq: f32,
}

impl TptSvf {
    /// Returns `(lowpass, bandpass, highpass)` at sample rate `sr`.
    #[inline]
    fn step(&mut self, v0: f32, cutoff_hz: f32, q: f32, sr: f32) -> (f32, f32, f32) {
        let fc = clamp_finite(cutoff_hz, 12.0, sr * 0.45);
        let q_safe = clamp_finite(q, 0.5, 24.0);
        let g = tan_f32(PI * fc / sr);
        let k = 1.0 / q_safe;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = v0 - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        let lp = v2;
        let bp = v1;
        let hp = v0 - k * v1 - v2;
        (lp, bp, hp)
    }

    #[inline]
    fn lowpass(&mut self, v0: f32, cutoff_hz: f32, q: f32, sr: f32) -> f32 {
        self.step(v0, cutoff_hz, q, sr).0
    }

    /// Normalized bandpass with unity gain at center frequency.
    #[inline]
    fn bandpass_norm(&mut self, v0: f32, cutoff_hz: f32, q: f32, sr: f32) -> f32 {
        let q_safe = clamp_finite(q, 0.5, 24.0);
        let (_, bp, _) = self.step(v0, cutoff_hz, q_safe, sr);
        bp / q_safe
    }

    #[inline]
    fn highpass(&mut self, v0: f32, cutoff_hz: f32, q: f32, sr: f32) -> f32 {
        self.step(v0, cutoff_hz, q, sr).2
    }
}

/// Peaking/shelf biquad equalizer filter.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn set_peaking(&mut self, freq_hz: f32, q: f32, gain_db: f32, sr: f32) {
        let fc = clamp_finite(freq_hz, 20.0, sr * 0.45);
        let a = exp2_f32(gain_db * 0.1660964); // 10^(dB/40) = 2^(dB * log2(10)/40)
        let w0 = TAU * fc / sr;
        let sin_w = sin_f32(w0);
        let cos_w = cos_f32(w0);
        let alpha = sin_w / (2.0 * q.max(0.25));
        let a0_inv = 1.0 / (1.0 + alpha / a);
        self.b0 = (1.0 + alpha * a) * a0_inv;
        self.b1 = (-2.0 * cos_w) * a0_inv;
        self.b2 = (1.0 - alpha * a) * a0_inv;
        self.a1 = (-2.0 * cos_w) * a0_inv;
        self.a2 = (1.0 - alpha / a) * a0_inv;
    }

    fn set_low_shelf(&mut self, freq_hz: f32, gain_db: f32, sr: f32) {
        let fc = clamp_finite(freq_hz, 20.0, sr * 0.45);
        let a = exp2_f32(gain_db * 0.1660964);
        let w0 = TAU * fc / sr;
        let sin_w = sin_f32(w0);
        let cos_w = cos_f32(w0);
        let alpha = sin_w * 0.70710678;
        let two_sqrt_a_alpha = 2.0 * sqrt_f32(a) * alpha;
        let a0 = (a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha;
        let a0_inv = 1.0 / a0;
        self.b0 = (a * ((a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha)) * a0_inv;
        self.b1 = (2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w)) * a0_inv;
        self.b2 = (a * ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha)) * a0_inv;
        self.a1 = (-2.0 * ((a - 1.0) + (a + 1.0) * cos_w)) * a0_inv;
        self.a2 = ((a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha) * a0_inv;
    }

    fn set_high_shelf(&mut self, freq_hz: f32, gain_db: f32, sr: f32) {
        let fc = clamp_finite(freq_hz, 20.0, sr * 0.45);
        let a = exp2_f32(gain_db * 0.1660964);
        let w0 = TAU * fc / sr;
        let sin_w = sin_f32(w0);
        let cos_w = cos_f32(w0);
        let alpha = sin_w * 0.70710678;
        let two_sqrt_a_alpha = 2.0 * sqrt_f32(a) * alpha;
        let a0 = (a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha;
        let a0_inv = 1.0 / a0;
        self.b0 = (a * ((a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha)) * a0_inv;
        self.b1 = (-2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w)) * a0_inv;
        self.b2 = (a * ((a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha)) * a0_inv;
        self.a1 = (2.0 * ((a - 1.0) - (a + 1.0) * cos_w)) * a0_inv;
        self.a2 = ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha) * a0_inv;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// First-order DC blocker (~7 Hz at 48 kHz).
#[derive(Clone, Copy, Debug, Default)]
struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.999 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// 2x Polyphase Halfband Allpass Up/Downsampler (`N` sections per branch).
#[derive(Clone, Copy, Debug)]
struct HalfbandStage<const N: usize> {
    c0: [f32; N],
    c1: [f32; N],
    up0: [f32; N],
    up1: [f32; N],
    dn0: [f32; N],
    dn1: [f32; N],
}

impl<const N: usize> HalfbandStage<N> {
    const fn new(c0: [f32; N], c1: [f32; N]) -> Self {
        Self {
            c0,
            c1,
            up0: [0.0; N],
            up1: [0.0; N],
            dn0: [0.0; N],
            dn1: [0.0; N],
        }
    }

    #[inline]
    fn step_branch(coefs: &[f32; N], state: &mut [f32; N], mut x: f32) -> f32 {
        for i in 0..N {
            let c = coefs[i];
            let y = c * x + state[i];
            state[i] = x - c * y;
            x = y;
        }
        x
    }

    #[inline]
    fn upsample(&mut self, x: f32) -> [f32; 2] {
        let s0 = Self::step_branch(&self.c0, &mut self.up0, x);
        let s1 = Self::step_branch(&self.c1, &mut self.up1, x);
        [s0, s1]
    }

    #[inline]
    fn downsample(&mut self, s: [f32; 2]) -> f32 {
        let y0 = Self::step_branch(&self.c0, &mut self.dn0, s[1]);
        let y1 = Self::step_branch(&self.c1, &mut self.dn1, s[0]);
        0.5 * (y0 + y1)
    }
}

/// 2x Polyphase Halfband Oversampler (~64 dB rejection, flat to 17 kHz) for `GuitarAmp`.
#[derive(Clone, Copy, Debug)]
struct Oversampler2x {
    stage: HalfbandStage<2>,
}

impl Default for Oversampler2x {
    fn default() -> Self {
        Self {
            stage: HalfbandStage::new(
                [0.099_622_54, 0.700_563_5],
                [0.356_925_43, 0.952_190_64],
            ),
        }
    }
}

impl Oversampler2x {
    #[inline]
    fn process(&mut self, x: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let up = self.stage.upsample(x);
        let p0 = f(up[0]);
        let p1 = f(up[1]);
        self.stage.downsample([p0, p1])
    }
}

/// 4x Polyphase Halfband Oversampler (`48 kHz <-> 96 kHz <-> 192 kHz`, ~68 dB rejection) for `Sub808`.
#[derive(Clone, Copy, Debug)]
struct Oversampler4x {
    stage1: HalfbandStage<3>, // 48 <-> 96 kHz, flat to 20.16 kHz
    stage2: HalfbandStage<2>, // 96 <-> 192 kHz, flat to 24.0 kHz
    dc: DcBlocker,
}

impl Default for Oversampler4x {
    fn default() -> Self {
        Self {
            stage1: HalfbandStage::new(
                [0.013_795_603, 0.306_264_4, 0.737_595_26],
                [0.109_031_41, 0.533_169_15, 0.913_800_2],
            ),
            stage2: HalfbandStage::new(
                [0.087_225_04, 0.664_868],
                [0.327_581_97, 0.931_007_2],
            ),
            dc: DcBlocker::default(),
        }
    }
}

impl Oversampler4x {
    #[inline]
    fn process(&mut self, x: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let mid = self.stage1.upsample(x);
        let q0 = self.stage2.upsample(mid[0]);
        let p00 = f(q0[0]);
        let p01 = f(q0[1]);
        let d0 = self.stage2.downsample([p00, p01]);

        let q1 = self.stage2.upsample(mid[1]);
        let p10 = f(q1[0]);
        let p11 = f(q1[1]);
        let d1 = self.stage2.downsample([p10, p11]);

        let out = self.stage1.downsample([d0, d1]);
        self.dc.process(out)
    }
}

/// Driven 2nd-order modal resonator using coupled-form rotation (stable at all frequencies).
#[derive(Clone, Copy, Debug, Default)]
struct ModalResonator {
    u: f32,
    v: f32,
    cos_w: f32,
    sin_w: f32,
    r: f32,
    gain: f32,
}

impl ModalResonator {
    fn configure(&mut self, freq_hz: f32, decay_s: f32, gain: f32) {
        if freq_hz <= 10.0 || freq_hz >= SR * 0.45 {
            self.r = 0.0;
            self.gain = 0.0;
            self.u = 0.0;
            self.v = 0.0;
            return;
        }
        let w = TAU * freq_hz * SR_INV;
        self.cos_w = cos_f32(w);
        self.sin_w = sin_f32(w);
        let tau_samples = (decay_s.max(0.002) * SR).max(1.0);
        // Decay by -60 dB (e^-6.907755) over decay_s
        self.r = exp_f32(-6.907_755 / tau_samples);
        self.gain = gain;
    }

    #[inline]
    fn excite(&mut self, impulse: f32) {
        self.u += impulse * self.gain;
    }

    #[inline]
    fn step(&mut self) -> f32 {
        let next_u = self.r * (self.u * self.cos_w - self.v * self.sin_w);
        let next_v = self.r * (self.u * self.sin_w + self.v * self.cos_w);
        self.u = next_u;
        self.v = next_v;
        next_v
    }
}

// ============================================================================
// 3. Instrument 1: Sub808 (Instrument::Sub808 = 5, "Sub 808")
// ============================================================================

/// Pitch glide behavior between notes in `Sub808`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlideMode {
    Off,
    Legato,
    Always,
}

/// Nonlinear waveshaper curve for `Sub808`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveCurve {
    WarmSym,
    TubeAsym,
    HardClip,
    Foldback,
}

/// Phase-locked sub-bass synthesizer with pitch-drop punch, automatic legato glide,
/// 4x oversampled waveshaper, post-filter transient click, and vocal formant growl.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sub808 {
    /// Oscillator morph from pure Sine (`0.0`) to Triangle (`1.0`).
    pub shape: f32,
    /// Phase-locked 2nd harmonic blend (`0.0..=1.0`).
    pub octave: f32,
    /// Initial downward pitch sweep depth in semitones (`0.0..=36.0`).
    pub punch_semitones: f32,
    /// Pitch sweep decay time in milliseconds (`5.0..=250.0`).
    pub punch_ms: f32,
    /// Amplitude sustain hold before decay (`0.0..=1000.0` ms).
    pub hold_ms: f32,
    /// Amplitude decay time (`50.0..=4000.0` ms).
    pub decay_ms: f32,
    /// Key tracking on decay time (`0.0..=1.0`, higher notes decay faster).
    pub decay_key_track: f32,
    /// Legato / continuous glide mode.
    pub glide_mode: GlideMode,
    /// Pitch glide time in milliseconds (`5.0..=500.0`).
    pub glide_ms: f32,
    /// Waveshaper saturation amount (`0.0..=1.0`).
    pub drive: f32,
    /// Waveshaper transfer curve.
    pub drive_curve: DriveCurve,
    /// 12 dB/oct post-drive SVF lowpass cutoff (`80.0..=18000.0` Hz).
    pub tone_hz: f32,
    /// Post-filter bandpassed transient click (`0.0..=1.0`).
    pub click: f32,
    /// Dual-bandpass vocal formant growl (`0.0..=1.0`).
    pub growl: f32,
}

pub type Sub808Params = Sub808;

impl Default for Sub808 {
    fn default() -> Self {
        Self {
            shape: 0.25,
            octave: 0.15,
            punch_semitones: 18.0,
            punch_ms: 45.0,
            hold_ms: 40.0,
            decay_ms: 650.0,
            decay_key_track: 0.4,
            glide_mode: GlideMode::Legato,
            glide_ms: 65.0,
            drive: 0.35,
            drive_curve: DriveCurve::TubeAsym,
            tone_hz: 900.0,
            click: 0.25,
            growl: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Sub808Voice {
    active: bool,
    phase: f32,
    age_frames: u32,
    gate_end: u32,
    release_frames: u32,
    release_total: u32,
    glide_start_semi: f32,
    glide_target_semi: f32,
    glide_pos: u32,
    glide_total: u32,
    current_base_semi: f32,
    vel: f32,
    target_vel: f32,
    expr: NoteExpr,
    key_decay_tau_s: f32,
    fade_pos: u32,
    fade_len: u32,
    fading_out: bool,
    last_env: f32,
}

impl Default for Sub808Voice {
    fn default() -> Self {
        Self {
            active: false,
            phase: 0.0,
            age_frames: 0,
            gate_end: 0,
            release_frames: 0,
            release_total: 960,
            glide_start_semi: 36.0,
            glide_target_semi: 36.0,
            glide_pos: 0,
            glide_total: 0,
            current_base_semi: 36.0,
            vel: 0.0,
            target_vel: 0.0,
            expr: NoteExpr::default(),
            key_decay_tau_s: 0.65,
            fade_pos: 0,
            fade_len: 192,
            fading_out: false,
            last_env: 0.0,
        }
    }
}

impl Sub808Voice {
    fn start_new(
        &mut self,
        en: ExpressiveNote,
        glide_from: Option<f32>,
        glide_frames: u32,
        decay_ms: f32,
        decay_key_track: f32,
    ) {
        let target_semi = en.note.key as f32;
        let key_factor = exp2_f32(-decay_key_track * (target_semi - 36.0) / 24.0).clamp(0.2, 2.5);
        let tau_s = (decay_ms.clamp(50.0, 4000.0) * 0.001 * key_factor) / 3.0;
        let start_semi = glide_from.unwrap_or(target_semi);
        *self = Self {
            active: true,
            phase: 0.0,
            age_frames: 0,
            gate_end: en.note.start + en.note.frames,
            release_frames: 0,
            release_total: 960, // 20 ms smooth release
            glide_start_semi: start_semi,
            glide_target_semi: target_semi,
            glide_pos: 0,
            glide_total: if glide_from.is_some() { glide_frames } else { 0 },
            current_base_semi: start_semi,
            vel: en.note.velocity as f32 / 127.0,
            target_vel: en.note.velocity as f32 / 127.0,
            expr: en.expr,
            key_decay_tau_s: tau_s.max(0.015),
            fade_pos: 0,
            fade_len: 192, // 4 ms retrigger crossfade
            fading_out: false,
            last_env: 0.0,
        };
    }

    fn slide_to(
        &mut self,
        en: ExpressiveNote,
        glide_frames: u32,
        decay_ms: f32,
        decay_key_track: f32,
    ) {
        let target_semi = en.note.key as f32;
        let key_factor = exp2_f32(-decay_key_track * (target_semi - 36.0) / 24.0).clamp(0.2, 2.5);
        self.key_decay_tau_s = ((decay_ms.clamp(50.0, 4000.0) * 0.001 * key_factor) / 3.0).max(0.015);
        self.glide_start_semi = self.current_base_semi;
        self.glide_target_semi = target_semi;
        self.glide_pos = 0;
        self.glide_total = glide_frames.max(1);
        self.gate_end = self.gate_end.max(en.note.start + en.note.frames);
        self.target_vel = en.note.velocity as f32 / 127.0;
        self.expr = en.expr;
    }

    fn begin_fadeout(&mut self) {
        if self.active && !self.fading_out {
            self.fading_out = true;
            self.fade_pos = 0;
            self.fade_len = 192; // exact 4 ms (192 frames at 48 kHz)
        }
    }

    #[inline]
    fn step(
        &mut self,
        frame_idx: u32,
        shape: f32,
        octave: f32,
        punch_semi: f32,
        punch_tau_s: f32,
        hold_frames: u32,
    ) -> (f32, f32, f32) {
        if !self.active {
            return (0.0, 0.0, 65.0);
        }

        // Smooth velocity slew during legato slides
        self.vel += (self.target_vel - self.vel) * 0.015;

        // Linear pitch glide (linear in semitones = perceptual linear pitch)
        if self.glide_pos < self.glide_total && self.glide_total > 0 {
            let t = self.glide_pos as f32 / self.glide_total as f32;
            self.current_base_semi =
                self.glide_start_semi + (self.glide_target_semi - self.glide_start_semi) * t;
            self.glide_pos += 1;
        } else {
            self.current_base_semi = self.glide_target_semi;
        }

        // Pitch-drop punch envelope
        let age_s = self.age_frames as f32 * SR_INV;
        let punch_env = if age_s < punch_tau_s * 8.0 {
            exp_f32(-age_s / punch_tau_s)
        } else {
            0.0
        };
        let pitch_semi = self.current_base_semi
            + punch_semi * punch_env
            + self.expr.pitch_bend_semitones;
        let freq_hz = midi_to_hz(pitch_semi).clamp(16.0, 8000.0);

        // Hold + key-tracked decay + smooth gate release
        let decay_env = if self.age_frames <= hold_frames {
            1.0
        } else {
            let dt = (self.age_frames - hold_frames) as f32 * SR_INV;
            exp_f32(-dt / self.key_decay_tau_s)
        };
        let attack_env = (self.age_frames as f32 / 16.0).min(1.0); // 0.33 ms zero-cross guard
        let gate_env = if frame_idx >= self.gate_end {
            let r = 1.0 - (self.release_frames as f32 / self.release_total.max(1) as f32);
            self.release_frames = self.release_frames.saturating_add(1);
            if self.release_frames >= self.release_total {
                self.active = false;
            }
            r.max(0.0)
        } else {
            1.0
        };

        let xfade = if self.fading_out {
            let g = 1.0 - (self.fade_pos as f32 / self.fade_len as f32);
            self.fade_pos += 1;
            if self.fade_pos >= self.fade_len {
                self.active = false;
            }
            g.max(0.0)
        } else {
            1.0
        };

        let env = attack_env * decay_env * gate_env * xfade * self.vel;
        self.last_env = env;
        if env < 1e-6 && self.age_frames > 32 {
            self.active = false;
            return (0.0, 0.0, freq_hz);
        }

        // Phase-locked Sine <-> Triangle + 2nd harmonic
        let p = self.phase;
        let s = sin_f32(TAU * p);
        let tri = if p < 0.25 {
            4.0 * p
        } else if p < 0.75 {
            2.0 - 4.0 * p
        } else {
            4.0 * p - 4.0
        };
        let fund = s * (1.0 - shape) + tri * shape;
        let oct_amt = (octave + self.expr.pressure * 0.25).clamp(0.0, 1.0);
        let oct = sin_f32(2.0 * TAU * p) * oct_amt * 0.45;
        let osc = (fund + oct) * env;

        // Post-filter transient click burst (first ~6 ms)
        let click_env = if self.age_frames < 288 {
            exp_f32(-(self.age_frames as f32) / 36.0) * self.vel
        } else {
            0.0
        };

        self.phase += freq_hz * SR_INV;
        if self.phase >= 1.0 {
            self.phase -= floor_f32(self.phase);
        }
        self.age_frames = self.age_frames.saturating_add(1);

        (osc, click_env, freq_hz)
    }
}

impl Sub808 {
    /// Render MIDI notes with automatic overlapping-note Legato pitch slide detection
    /// and 4 ms retrigger anti-click crossfading.
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    /// Render expressive MIDI notes for `Sub808`.
    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let sorted = sorted_expressive(notes)?;
        let mut mix = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        if sorted.is_empty() {
            return mix_to_frames(&mix, 1.0, false);
        }

        let shape = clamp_finite(self.shape, 0.0, 1.0);
        let octave = clamp_finite(self.octave, 0.0, 1.0);
        let punch_semi = clamp_finite(self.punch_semitones, 0.0, 36.0);
        let punch_tau_s = (clamp_finite(self.punch_ms, 5.0, 250.0) * 0.001) / 3.0;
        let hold_frames = (clamp_finite(self.hold_ms, 0.0, 1000.0) * 0.001 * SR) as u32;
        let decay_ms = clamp_finite(self.decay_ms, 50.0, 4000.0);
        let key_track = clamp_finite(self.decay_key_track, 0.0, 1.0);
        let default_glide_frames = (clamp_finite(self.glide_ms, 5.0, 500.0) * 0.001 * SR) as u32;
        let drive = clamp_finite(self.drive, 0.0, 1.0);
        let base_tone = clamp_finite(self.tone_hz, 80.0, 18000.0);
        let click_amt = clamp_finite(self.click, 0.0, 1.0);
        let growl_amt = clamp_finite(self.growl, 0.0, 1.0);

        let mut active = Sub808Voice::default();
        let mut fading = [Sub808Voice::default(); 4];
        let mut os4 = Oversampler4x::default();
        let mut tone_svf = TptSvf::default();
        let mut click_svf = TptSvf::default();
        let mut growl_f1 = TptSvf::default();
        let mut growl_f2 = TptSvf::default();
        let mut noise_state = 0x8080_1337u32;
        let mut next_note = 0usize;
        let mut last_semi: Option<f32> = None;

        for frame_idx in 0..total_frames {
            while next_note < sorted.len() && sorted[next_note].note.start == frame_idx {
                let en = sorted[next_note];
                next_note += 1;
                let glide_frames = if en.expr.glide_frames > 0 {
                    en.expr.glide_frames
                } else {
                    default_glide_frames
                };
                let overlapping_gate = active.active && !active.fading_out && frame_idx < active.gate_end;
                let should_legato = match self.glide_mode {
                    GlideMode::Off => false,
                    GlideMode::Legato => overlapping_gate,
                    GlideMode::Always => active.active && !active.fading_out && active.last_env > 1e-4,
                };

                if should_legato {
                    active.slide_to(en, glide_frames, decay_ms, key_track);
                } else {
                    if active.active {
                        active.begin_fadeout();
                        let slot = fading
                            .iter()
                            .position(|v| !v.active)
                            .unwrap_or_else(|| {
                                fading
                                    .iter()
                                    .enumerate()
                                    .max_by_key(|(_, v)| v.fade_pos)
                                    .map(|(i, _)| i)
                                    .unwrap_or(0)
                            });
                        fading[slot] = active;
                    }
                    let explicit_from = en.expr.slide_from_key.map(|k| k as f32);
                    let always_from = if self.glide_mode == GlideMode::Always {
                        last_semi
                    } else {
                        None
                    };
                    active.start_new(
                        en,
                        explicit_from.or(always_from),
                        glide_frames,
                        decay_ms,
                        key_track,
                    );
                }
                last_semi = Some(en.note.key as f32);
            }

            let (sig_a, clk_a, freq_a) =
                active.step(frame_idx, shape, octave, punch_semi, punch_tau_s, hold_frames);
            let mut sum_osc = sig_a;
            let mut sum_clk = clk_a;
            for fv in &mut fading {
                if fv.active {
                    let (sf, cf, _) =
                        fv.step(frame_idx, shape, octave, punch_semi, punch_tau_s, hold_frames);
                    sum_osc += sf;
                    sum_clk += cf;
                }
            }

            // 4x Oversampled Waveshaper (192 kHz internal) + DC Blocker
            let shaped = if drive <= 0.0001 {
                sum_osc
            } else {
                let pre_gain = 1.0 + drive * 15.0;
                let fold_ease = if freq_a <= 260.0 {
                    1.0
                } else {
                    (260.0 / freq_a).clamp(0.15, 1.0)
                };
                let curve = self.drive_curve;
                let norm = tanh_f32(pre_gain.min(6.0)).max(0.5);
                os4.process(sum_osc, |s| {
                    let u = s * pre_gain;
                    match curve {
                        DriveCurve::WarmSym => tanh_f32(u) / norm,
                        DriveCurve::TubeAsym => {
                            let biased = u + 0.16 * drive;
                            if biased >= 0.0 {
                                tanh_f32(biased * 1.15)
                            } else {
                                tanh_f32(biased * 0.72)
                            }
                        }
                        DriveCurve::HardClip => u.clamp(-0.92, 0.92),
                        DriveCurve::Foldback => {
                            let eff = s * (1.0 + (pre_gain - 1.0) * 0.45 * fold_ease);
                            let p = (eff + 1.0) - 4.0 * floor_f32((eff + 1.0) * 0.25);
                            let folded = if p < 2.0 { p - 1.0 } else { 3.0 - p };
                            folded * 0.92
                        }
                    }
                })
            };

            // 12 dB/oct post-drive SVF tone filter modulated by NoteExpr::timbre
            let timbre = active.expr.timbre;
            let tone_mod = exp2_f32((timbre - 0.5) * 2.6);
            let tone_cutoff = (base_tone * tone_mod).clamp(80.0, 18000.0);
            let filtered = tone_svf.lowpass(shaped, tone_cutoff, 0.75, SR);

            // Optional Formant Growl (dual-bandpass vocal vowel resonance)
            let growled = if growl_amt > 0.0001 {
                let vowel_pos = (timbre + active.expr.pressure * 0.35).clamp(0.0, 1.0);
                let f1_hz = 520.0 + vowel_pos * 260.0;
                let f2_hz = 1100.0 + vowel_pos * 750.0;
                let g1 = growl_f1.bandpass_norm(shaped, f1_hz, 4.5, SR);
                let g2 = growl_f2.bandpass_norm(shaped, f2_hz, 5.5, SR);
                let vowel = (g1 * 0.7 + g2 * 0.55) * 2.2;
                filtered * (1.0 - growl_amt * 0.35) + vowel * growl_amt
            } else {
                filtered
            };

            // Post-filter bandpassed transient click
            let click_sig = if click_amt > 0.0001 && sum_clk > 1e-5 {
                let n_raw = next_noise(&mut noise_state) * 0.6 + 0.4;
                let bp = click_svf.bandpass_norm(sum_clk * n_raw, 2400.0, 2.2, SR);
                bp * click_amt * 0.65
            } else {
                let _ = click_svf.bandpass_norm(0.0, 2400.0, 2.2, SR);
                0.0
            };

            let out = (growled + click_sig) * 0.80;
            mix[frame_idx as usize] = [out, out];
        }

        mix_to_frames(&mix, 1.0, true)
    }
}

// ============================================================================
// 4. Instrument 2: DrumKit (Instrument::DrumKit = 6, "Studio Drums")
// ============================================================================

/// Selectable synthesis engine for `DrumKit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumModel {
    Electronic,
    Circuit,
    ModalAcoustic,
}

/// GM-mapped percussion pieces for `DrumKit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumPiece {
    Kick,
    Rim,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
    Tom(u8), // 0..=5 corresponding to GM 41,43,45,47,48,50
    Crash,
    Ride,
}

impl DrumPiece {
    pub const fn from_midi_key(key: u8) -> Self {
        match key {
            35 | 36 => Self::Kick,
            37 => Self::Rim,
            38 | 40 => Self::Snare,
            39 => Self::Clap,
            42 | 44 => Self::ClosedHat,
            46 => Self::OpenHat,
            41 => Self::Tom(0),
            43 => Self::Tom(1),
            45 => Self::Tom(2),
            47 => Self::Tom(3),
            48 => Self::Tom(4),
            50 => Self::Tom(5),
            49 | 57 => Self::Crash,
            51 => Self::Ride,
            k if k < 40 => Self::Kick,
            k if k < 48 => Self::Tom(2),
            k if k < 56 => Self::Ride,
            _ => Self::Crash,
        }
    }

    #[inline]
    fn pan(self) -> f32 {
        match self {
            Self::Kick | Self::Snare => 0.0,
            Self::Rim => -0.15,
            Self::Clap => 0.08,
            Self::ClosedHat | Self::OpenHat => -0.28,
            Self::Tom(idx) => -0.35 + (idx as f32 / 5.0) * 0.70,
            Self::Crash => -0.40,
            Self::Ride => 0.35,
        }
    }
}

/// Studio percussion kit with 3 synthesis models and GM Closed-Hat -> Open-Hat choke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumKit {
    pub model: DrumModel,
    /// Global drum kit pitch offset in semitones (`-12.0..=12.0`).
    pub tune_semitones: f32,
    /// Global brightness (`0.0..=1.0`, `0.5` neutral).
    pub tone: f32,
    /// Transient punch / compression bite (`0.0..=1.0`).
    pub punch: f32,
    /// Subtle stereo room ambience (`0.0..=1.0`).
    pub room: f32,
}

pub type DrumKitParams = DrumKit;

impl Default for DrumKit {
    fn default() -> Self {
        Self {
            model: DrumModel::Circuit,
            tune_semitones: 0.0,
            tone: 0.5,
            punch: 0.55,
            room: 0.2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SquareBank6 {
    phases: [f32; 6],
}

impl SquareBank6 {
    #[inline]
    fn step(&mut self, pitch_mult: f32) -> f32 {
        const FREQS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
        let mut sum = 0.0;
        for i in 0..6 {
            let p = self.phases[i];
            sum += if p < 0.5 { 1.0 } else { -1.0 };
            let next = p + (FREQS[i] * pitch_mult) * SR_INV;
            self.phases[i] = if next >= 1.0 { next - floor_f32(next) } else { next };
        }
        sum * (1.0 / 6.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CircuitSharedState {
    squares: SquareBank6,
    snare_res_a: ModalResonator,
    snare_res_b: ModalResonator,
    tom_res: [ModalResonator; 6],
    initialized: bool,
}

impl CircuitSharedState {
    fn ensure_init(&mut self, pitch_mult: f32) {
        if !self.initialized {
            self.snare_res_a.configure(182.0 * pitch_mult, 0.24, 1.0);
            self.snare_res_b.configure(338.0 * pitch_mult, 0.16, 0.75);
            const TOM_HZ: [f32; 6] = [74.0, 92.0, 112.0, 134.0, 160.0, 192.0];
            for (i, &hz) in TOM_HZ.iter().enumerate() {
                self.tom_res[i].configure(hz * pitch_mult, 0.34, 1.0);
            }
            self.initialized = true;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DrumVoice {
    active: bool,
    piece: DrumPiece,
    age: u32,
    max_age: u32,
    vel: f32,
    expr: NoteExpr,
    phase: f32,
    phase2: f32,
    choke_remaining: u32,
    choke_total: u32,
    svf_a: TptSvf,
    svf_b: TptSvf,
    modes: [ModalResonator; 8],
    pan: f32,
}

impl Default for DrumVoice {
    fn default() -> Self {
        Self {
            active: false,
            piece: DrumPiece::Kick,
            age: 0,
            max_age: 1,
            vel: 0.0,
            expr: NoteExpr::default(),
            phase: 0.0,
            phase2: 0.0,
            choke_remaining: 0,
            choke_total: 144,
            svf_a: TptSvf::default(),
            svf_b: TptSvf::default(),
            modes: [ModalResonator::default(); 8],
            pan: 0.0,
        }
    }
}

impl DrumVoice {
    fn trigger(
        &mut self,
        en: ExpressiveNote,
        model: DrumModel,
        tune_semi: f32,
        tone: f32,
        circuit: &mut CircuitSharedState,
    ) {
        let piece = DrumPiece::from_midi_key(en.note.key);
        let vel = en.note.velocity as f32 / 127.0;
        let pitch_mult = exp2_f32((tune_semi + en.expr.pitch_bend_semitones) / 12.0);
        let brightness = (tone + (en.expr.timbre - 0.5) * 0.7).clamp(0.05, 1.0);

        *self = Self {
            active: true,
            piece,
            age: 0,
            max_age: match piece {
                DrumPiece::Kick => (0.55 * SR) as u32,
                DrumPiece::Rim => (0.14 * SR) as u32,
                DrumPiece::Snare => (0.42 * SR) as u32,
                DrumPiece::Clap => (0.48 * SR) as u32,
                DrumPiece::ClosedHat => (0.11 * SR) as u32,
                DrumPiece::OpenHat => (0.85 * SR) as u32,
                DrumPiece::Tom(_) => (0.52 * SR) as u32,
                DrumPiece::Crash => (1.45 * SR) as u32,
                DrumPiece::Ride => (1.15 * SR) as u32,
            },
            vel,
            expr: en.expr,
            phase: 0.0,
            phase2: 0.0,
            choke_remaining: 0,
            choke_total: 144, // 3 ms exact at 48 kHz
            svf_a: TptSvf::default(),
            svf_b: TptSvf::default(),
            modes: [ModalResonator::default(); 8],
            pan: piece.pan(),
        };

        // Configure persistent circuit resonators or modal acoustic resonators
        match model {
            DrumModel::Circuit => {
                circuit.ensure_init(pitch_mult);
                let hit = vel * (0.7 + 0.3 * brightness);
                match piece {
                    DrumPiece::Snare => {
                        circuit.snare_res_a.excite(hit * 0.9);
                        circuit.snare_res_b.excite(hit * 0.65);
                    }
                    DrumPiece::Tom(idx) => {
                        let i = (idx as usize).min(5);
                        circuit.tom_res[i].excite(hit);
                    }
                    _ => {}
                }
            }
            DrumModel::ModalAcoustic => {
                self.configure_modal(piece, vel, pitch_mult, brightness);
            }
            DrumModel::Electronic => {}
        }
    }

    fn configure_modal(&mut self, piece: DrumPiece, vel: f32, pitch_mult: f32, brightness: f32) {
        // Velocity hardens stick/beater -> boosts higher modal partials!
        let hard = (vel * 0.7 + brightness * 0.3).clamp(0.1, 1.0);
        match piece {
            DrumPiece::Kick => {
                let f0 = 54.0 * pitch_mult;
                const RATIOS: [f32; 5] = [1.0, 1.59, 2.14, 2.30, 2.65];
                for (i, &r) in RATIOS.iter().enumerate() {
                    let decay = 0.42 / powf_f32(r, 0.9);
                    let amp = if i == 0 { 1.0 } else { 0.38 * powf_f32(hard, i as f32 * 0.45) };
                    self.modes[i].configure(f0 * r, decay, amp);
                    self.modes[i].excite(vel);
                }
            }
            DrumPiece::Snare => {
                let f0 = 192.0 * pitch_mult;
                const RATIOS: [f32; 6] = [1.0, 1.59, 2.14, 2.65, 3.16, 3.89];
                for (i, &r) in RATIOS.iter().enumerate() {
                    let decay = 0.24 / powf_f32(r, 0.85);
                    let amp = 0.55 * powf_f32(hard, i as f32 * 0.35);
                    self.modes[i].configure(f0 * r, decay, amp);
                    self.modes[i].excite(vel);
                }
            }
            DrumPiece::Tom(idx) => {
                const BASE: [f32; 6] = [78.0, 96.0, 116.0, 140.0, 168.0, 202.0];
                let f0 = BASE[(idx as usize).min(5)] * pitch_mult;
                const RATIOS: [f32; 5] = [1.0, 1.59, 2.14, 2.30, 2.92];
                for (i, &r) in RATIOS.iter().enumerate() {
                    let decay = 0.46 / powf_f32(r, 0.8);
                    let amp = 0.72 * powf_f32(hard, i as f32 * 0.4);
                    self.modes[i].configure(f0 * r, decay, amp);
                    self.modes[i].excite(vel);
                }
            }
            DrumPiece::Rim => {
                let f0 = 460.0 * pitch_mult;
                const RATIOS: [f32; 4] = [1.0, 2.76, 5.40, 8.20];
                for (i, &r) in RATIOS.iter().enumerate() {
                    let decay = 0.11 / powf_f32(r, 0.75);
                    let amp = 0.65 * powf_f32(hard, i as f32 * 0.3);
                    self.modes[i].configure(f0 * r, decay, amp);
                    self.modes[i].excite(vel);
                }
            }
            DrumPiece::Clap => {
                const FREQS: [f32; 4] = [680.0, 1140.0, 1890.0, 2950.0];
                for (i, &f) in FREQS.iter().enumerate() {
                    self.modes[i].configure(f * pitch_mult, 0.16 / (1.0 + i as f32 * 0.3), 0.45);
                    self.modes[i].excite(vel);
                }
            }
            DrumPiece::ClosedHat | DrumPiece::OpenHat | DrumPiece::Crash | DrumPiece::Ride => {
                // Cymbal-plate physical modal resonators: high partials decay faster than low partials!
                let (base_f, base_decay) = match piece {
                    DrumPiece::ClosedHat => (980.0, 0.22),
                    DrumPiece::OpenHat => (880.0, 1.65),
                    DrumPiece::Crash => (410.0, 3.20),
                    DrumPiece::Ride => (560.0, 2.60),
                    _ => (600.0, 1.0),
                };
                const CYM_RATIOS: [f32; 8] = [1.0, 1.47, 2.09, 2.86, 3.74, 4.91, 6.45, 8.62];
                for (i, &r) in CYM_RATIOS.iter().enumerate() {
                    let freq = base_f * r * pitch_mult;
                    // High partials decay dramatically faster than low partials
                    let decay = (base_decay / powf_f32(r, 1.15)).max(0.015);
                    let amp = 0.24 * powf_f32(hard, 0.2 + i as f32 * 0.25);
                    self.modes[i].configure(freq, decay, amp);
                    self.modes[i].excite(vel);
                }
            }
        }
    }

    fn begin_choke(&mut self) {
        if self.active && self.choke_remaining == 0 {
            self.choke_total = 144; // 3 ms at 48 kHz
            self.choke_remaining = 144;
        }
    }

    #[inline]
    fn step(
        &mut self,
        model: DrumModel,
        tune_semi: f32,
        tone: f32,
        punch: f32,
        circuit_sq: f32,
        noise_val: f32,
    ) -> [f32; 2] {
        if !self.active {
            return [0.0, 0.0];
        }
        let t_s = self.age as f32 * SR_INV;
        let pitch_mult = exp2_f32((tune_semi + self.expr.pitch_bend_semitones) / 12.0);
        let brightness = (tone + (self.expr.timbre - 0.5) * 0.6).clamp(0.05, 1.0);

        let raw = match model {
            DrumModel::Electronic => self.step_electronic(t_s, pitch_mult, brightness, punch, noise_val),
            DrumModel::Circuit => self.step_circuit(t_s, pitch_mult, brightness, punch, circuit_sq, noise_val),
            DrumModel::ModalAcoustic => self.step_modal(t_s, brightness, punch, noise_val),
        };

        let choke_gain = if self.choke_remaining > 0 {
            let g = self.choke_remaining as f32 / self.choke_total as f32;
            self.choke_remaining -= 1;
            if self.choke_remaining == 0 {
                self.active = false;
            }
            g
        } else {
            1.0
        };

        self.age += 1;
        if self.age >= self.max_age {
            self.active = false;
        }

        let s = raw * choke_gain;
        let angle = (self.pan + 1.0) * 0.5 * FRAC_PI_2;
        [s * cos_f32(angle), s * sin_f32(angle)]
    }

    fn step_electronic(
        &mut self,
        t_s: f32,
        pitch_mult: f32,
        brightness: f32,
        punch: f32,
        noise_val: f32,
    ) -> f32 {
        match self.piece {
            DrumPiece::Kick => {
                let f = (48.0 + 135.0 * exp_f32(-t_s / 0.022)) * pitch_mult;
                let s = sin_f32(TAU * self.phase);
                self.phase += f * SR_INV;
                if self.phase >= 1.0 {
                    self.phase -= floor_f32(self.phase);
                }
                let amp = exp_f32(-t_s / (0.14 + 0.08 * self.expr.pressure));
                let click = exp_f32(-t_s / 0.002) * (0.4 + 0.6 * brightness);
                tanh_f32((s * amp + click * 0.35) * (1.2 + punch * 1.4)) * self.vel * 0.9
            }
            DrumPiece::Rim => {
                let f1 = 440.0 * pitch_mult;
                let f2 = 1650.0 * pitch_mult;
                let s1 = sin_f32(TAU * self.phase) * exp_f32(-t_s / 0.016);
                let s2 = sin_f32(TAU * self.phase2) * exp_f32(-t_s / 0.006);
                self.phase = wrap_phase(self.phase + f1 * SR_INV);
                self.phase2 = wrap_phase(self.phase2 + f2 * SR_INV);
                (s1 * 0.7 + s2 * 0.5 * brightness) * self.vel
            }
            DrumPiece::Snare => {
                let f = (170.0 + 90.0 * exp_f32(-t_s / 0.014)) * pitch_mult;
                let body = sin_f32(TAU * self.phase) * exp_f32(-t_s / 0.055);
                self.phase = wrap_phase(self.phase + f * SR_INV);
                let hp = self.svf_a.highpass(noise_val, 1400.0 + brightness * 2400.0, 0.8, SR);
                let wires = hp * exp_f32(-t_s / 0.095);
                (body * (0.6 + punch * 0.3) + wires * 0.75) * self.vel
            }
            DrumPiece::Clap => {
                let burst = if t_s < 0.036 {
                    let sub_t = t_s - 0.011 * floor_f32(t_s / 0.011);
                    exp_f32(-sub_t / 0.004)
                } else {
                    exp_f32(-(t_s - 0.036) / 0.08) * 0.55
                };
                let bp = self.svf_a.bandpass_norm(noise_val, 1100.0 + brightness * 900.0, 1.8, SR);
                bp * burst * self.vel * 1.3
            }
            DrumPiece::ClosedHat | DrumPiece::OpenHat => {
                let decay = if self.piece == DrumPiece::ClosedHat { 0.022 } else { 0.24 };
                let f_a = 3300.0 * pitch_mult;
                let f_b = 5400.0 * pitch_mult;
                let metal = sin_f32(TAU * self.phase) * sin_f32(TAU * self.phase2);
                self.phase = wrap_phase(self.phase + f_a * SR_INV);
                self.phase2 = wrap_phase(self.phase2 + f_b * SR_INV);
                let hp = self.svf_a.highpass(noise_val * 0.75 + metal * 0.45, 5800.0 + brightness * 2500.0, 0.9, SR);
                hp * exp_f32(-t_s / decay) * self.vel * 0.85
            }
            DrumPiece::Tom(idx) => {
                const BASE: [f32; 6] = [80.0, 96.0, 114.0, 136.0, 162.0, 192.0];
                let base_f = BASE[(idx as usize).min(5)] * pitch_mult;
                let f = base_f * (1.0 + 0.85 * exp_f32(-t_s / 0.025));
                let s = sin_f32(TAU * self.phase);
                self.phase = wrap_phase(self.phase + f * SR_INV);
                let env = exp_f32(-t_s / 0.13);
                (s * env + noise_val * exp_f32(-t_s / 0.004) * 0.18 * brightness) * self.vel * 0.85
            }
            DrumPiece::Crash | DrumPiece::Ride => {
                let (fc, decay) = if self.piece == DrumPiece::Crash {
                    (4200.0 + brightness * 3000.0, 0.45)
                } else {
                    (5200.0 + brightness * 2800.0, 0.34)
                };
                let ping = if self.piece == DrumPiece::Ride {
                    sin_f32(TAU * self.phase) * exp_f32(-t_s / 0.045) * 0.35
                } else {
                    0.0
                };
                self.phase = wrap_phase(self.phase + 2650.0 * pitch_mult * SR_INV);
                let bp = self.svf_a.bandpass_norm(noise_val, fc, 0.75, SR);
                (bp * exp_f32(-t_s / decay) * 1.1 + ping) * self.vel * 0.85
            }
        }
    }

    fn step_circuit(
        &mut self,
        t_s: f32,
        pitch_mult: f32,
        brightness: f32,
        punch: f32,
        circuit_sq: f32,
        noise_val: f32,
    ) -> f32 {
        match self.piece {
            DrumPiece::Kick => {
                // Accented pitch-swept kick with clipped peak hold
                let accent = 1.0 + (self.vel - 0.5) * 0.45;
                let f = (51.0 + 185.0 * accent * exp_f32(-t_s / 0.016)) * pitch_mult;
                let raw_sin = sin_f32(TAU * self.phase);
                self.phase = wrap_phase(self.phase + f * SR_INV);
                // Peak hold diode clip on initial attack cycles
                let drive_env = 1.0 + (1.8 + punch * 2.5) * exp_f32(-t_s / 0.04);
                let clipped = (raw_sin * drive_env).clamp(-0.82, 0.82);
                let body = clipped * exp_f32(-t_s / (0.16 + 0.08 * self.expr.pressure));
                let click = self.svf_a.bandpass_norm(
                    if self.age < 24 { 1.0 } else { 0.0 },
                    1400.0 * brightness + 500.0,
                    1.5,
                    SR,
                );
                (body + click * 0.45) * self.vel * 0.95
            }
            DrumPiece::Rim => {
                // Twin bridged-T cross-coupled pulse oscillators
                let s1 = if self.phase < 0.35 { 1.0 } else { -1.0 };
                let s2 = if self.phase2 < 0.5 { 1.0 } else { -1.0 };
                self.phase = wrap_phase(self.phase + 475.0 * pitch_mult * SR_INV);
                self.phase2 = wrap_phase(self.phase2 + 1690.0 * pitch_mult * SR_INV);
                let bp = self.svf_a.bandpass_norm((s1 + s2 * 0.8) * exp_f32(-t_s / 0.011), 1150.0, 1.4, SR);
                bp * self.vel * 1.15
            }
            DrumPiece::Snare => {
                // Snare noise burst (dual-resonator body is summed persistently in CircuitSharedState!)
                let bp = self.svf_a.bandpass_norm(noise_val, 2200.0 + brightness * 1800.0, 1.1, SR);
                let snap = bp * exp_f32(-t_s / 0.068);
                snap * self.vel * (0.85 + punch * 0.3)
            }
            DrumPiece::Clap => {
                // Multi-burst saw-enveloped filtered noise (3 bursts @ 10 ms) + room tail
                let env = if t_s < 0.030 {
                    let slot = t_s - 0.010 * floor_f32(t_s / 0.010);
                    (1.0 - slot / 0.010).max(0.0)
                } else {
                    exp_f32(-(t_s - 0.030) / 0.115) * 0.65
                };
                let bp = self.svf_a.bandpass_norm(noise_val, 1050.0 + brightness * 450.0, 2.2, SR);
                bp * env * self.vel * 1.45
            }
            DrumPiece::ClosedHat | DrumPiece::OpenHat => {
                // 6 free-running inharmonic square oscillators -> steep bandpass/highpass -> VCA
                let decay = if self.piece == DrumPiece::ClosedHat { 0.024 } else { 0.34 };
                let bp = self.svf_a.bandpass_norm(circuit_sq, 7600.0 + brightness * 2400.0, 2.2, SR);
                let hp = self.svf_b.highpass(bp, 5200.0, 0.8, SR);
                hp * exp_f32(-t_s / decay) * self.vel * 2.2
            }
            DrumPiece::Tom(_) => {
                // Subtle stick attack transient; main tom body rings persistently in CircuitSharedState
                let attack = self.svf_a.lowpass(noise_val, 900.0 + brightness * 1200.0, 0.8, SR);
                attack * exp_f32(-t_s / 0.008) * 0.22 * self.vel
            }
            DrumPiece::Crash | DrumPiece::Ride => {
                let (fc, decay) = if self.piece == DrumPiece::Crash {
                    (4600.0 + brightness * 2800.0, 0.48)
                } else {
                    (6100.0 + brightness * 2400.0, 0.36)
                };
                let metal = circuit_sq * 0.72 + noise_val * 0.28;
                let bp = self.svf_a.bandpass_norm(metal, fc, 1.3, SR);
                bp * exp_f32(-t_s / decay) * self.vel * 1.25
            }
        }
    }

    fn step_modal(
        &mut self,
        t_s: f32,
        brightness: f32,
        punch: f32,
        noise_val: f32,
    ) -> f32 {
        let mut modal_sum = 0.0;
        for m in &mut self.modes {
            if m.r > 0.0 {
                modal_sum += m.step();
            }
        }

        // Velocity-brightened beater/stick contact transient
        let stick_fc = 750.0 + self.vel * self.vel * 10_500.0 * brightness;
        let stick_env = exp_f32(-t_s / (0.002 + (1.0 - self.vel) * 0.003));
        let stick = self.svf_a.lowpass(noise_val, stick_fc, 0.9, SR)
            * stick_env
            * (0.25 + 0.35 * punch)
            * self.vel;

        match self.piece {
            DrumPiece::Snare => {
                // Nonlinear bottom-head snare-wire buzz modulated by membrane motion
                let buzz_env = abs_f32(modal_sum).min(1.0) * 0.65 + exp_f32(-t_s / 0.08) * 0.55 * self.vel;
                let wire_hp = self.svf_b.highpass(noise_val, 2400.0 + (brightness * 0.5 + self.vel * 0.5) * 2600.0, 1.1, SR);
                modal_sum * 0.75 + wire_hp * buzz_env * 0.75 + stick * 0.3
            }
            DrumPiece::ClosedHat | DrumPiece::OpenHat | DrumPiece::Crash | DrumPiece::Ride => {
                // Stick impact shimmer + frequency-damped modal cymbal plate
                let wash_decay = match self.piece {
                    DrumPiece::ClosedHat => 0.014,
                    DrumPiece::OpenHat => 0.075,
                    DrumPiece::Crash => 0.090,
                    DrumPiece::Ride => 0.055,
                    _ => 0.05,
                };
                let wash_fc = 2400.0 + (brightness * 0.35 + self.vel * 0.65) * 6200.0;
                let wash = self.svf_b.highpass(noise_val, wash_fc, 0.9, SR)
                    * exp_f32(-t_s / wash_decay)
                    * self.vel
                    * (0.16 + 0.55 * self.vel);
                modal_sum * 0.85 + wash + stick * 0.45
            }
            _ => modal_sum * 0.85 + stick * 0.45,
        }
    }
}

impl DrumKit {
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let sorted = sorted_expressive(notes)?;
        let mut mix = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        if sorted.is_empty() {
            return mix_to_frames(&mix, 1.0, false);
        }

        let tune_semi = clamp_finite(self.tune_semitones, -12.0, 12.0);
        let pitch_mult = exp2_f32(tune_semi / 12.0);
        let tone = clamp_finite(self.tone, 0.0, 1.0);
        let punch = clamp_finite(self.punch, 0.0, 1.0);
        let room = clamp_finite(self.room, 0.0, 1.0);

        let mut voices = try_alloc_vec(MAX_VOICES, DrumVoice::default())?;
        let mut room_l = try_alloc_vec(384usize, 0.0f32)?; // 8 ms stereo room early reflection
        let mut room_r = try_alloc_vec(528usize, 0.0f32)?; // 11 ms
        let mut circuit = CircuitSharedState::default();
        let mut noise_state = 0xD407_B007u32;
        let mut next_note = 0usize;
        let mut room_pos_l = 0usize;
        let mut room_pos_r = 0usize;

        for frame_idx in 0..total_frames {
            while next_note < sorted.len() && sorted[next_note].note.start == frame_idx {
                let en = sorted[next_note];
                next_note += 1;
                let piece = DrumPiece::from_midi_key(en.note.key);

                // Closed-Hat (42/44) smoothly chokes any ringing Open-Hat (46) over 3 ms!
                if piece == DrumPiece::ClosedHat {
                    for v in voices.iter_mut() {
                        if v.active && v.piece == DrumPiece::OpenHat {
                            v.begin_choke();
                        }
                    }
                }

                // Allocate free slot or steal oldest active voice
                let slot = voices
                    .iter()
                    .position(|v| !v.active)
                    .unwrap_or_else(|| {
                        voices
                            .iter()
                            .enumerate()
                            .max_by_key(|(_, v)| v.age)
                            .map(|(i, _)| i)
                            .unwrap_or(0)
                    });
                voices[slot].trigger(en, self.model, tune_semi, tone, &mut circuit);
            }

            let circuit_sq = if self.model == DrumModel::Circuit {
                circuit.squares.step(pitch_mult)
            } else {
                0.0
            };

            let mut frame_sum = [0.0f32; 2];
            for v in voices.iter_mut() {
                if v.active {
                    let n = next_noise(&mut noise_state);
                    let lr = v.step(self.model, tune_semi, tone, punch, circuit_sq, n);
                    frame_sum[0] += lr[0];
                    frame_sum[1] += lr[1];
                }
            }

            // Sum persistent bridged-T resonators for Circuit snare & toms
            if self.model == DrumModel::Circuit && circuit.initialized {
                let snare_body = (circuit.snare_res_a.step() + circuit.snare_res_b.step()) * 0.45;
                frame_sum[0] += snare_body;
                frame_sum[1] += snare_body;
                for (i, tr) in circuit.tom_res.iter_mut().enumerate() {
                    let ts = tr.step() * 0.72;
                    let pan = -0.35 + (i as f32 / 5.0) * 0.70;
                    let angle = (pan + 1.0) * 0.5 * FRAC_PI_2;
                    frame_sum[0] += ts * cos_f32(angle);
                    frame_sum[1] += ts * sin_f32(angle);
                }
            }

            if room > 0.001 {
                let d_l = room_l[room_pos_l];
                let d_r = room_r[room_pos_r];
                room_l[room_pos_l] = frame_sum[0] + d_r * 0.28;
                room_r[room_pos_r] = frame_sum[1] + d_l * 0.28;
                room_pos_l = (room_pos_l + 1) % room_l.len();
                room_pos_r = (room_pos_r + 1) % room_r.len();
                frame_sum[0] += d_l * room * 0.24;
                frame_sum[1] += d_r * room * 0.24;
            }

            mix[frame_idx as usize] = frame_sum;
        }

        mix_to_frames(&mix, 0.72, true)
    }
}

// ============================================================================
// 5. Instrument 3: TineKeys (Instrument::TineKeys = 7, "Tine Keys")
// ============================================================================

/// Electric tine piano with physical/FM tine + hammer bark + pickup asymmetry + stereo tremolo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TineKeys {
    /// Tine bell FM harmonic amount (`0.0..=1.0`).
    pub bell: f32,
    /// Hammer strike bark / mid punch (`0.0..=1.0`).
    pub bark: f32,
    /// Electromagnetic pickup off-axis asymmetry (`0.0..=1.0`).
    pub pickup_asym: f32,
    /// Stereo tremolo LFO rate in Hz (`0.0..=12.0`).
    pub tremolo_rate_hz: f32,
    /// Stereo autopan / tremolo depth (`0.0..=1.0`).
    pub tremolo_depth: f32,
    /// Note sustain decay in milliseconds (`300.0..=5000.0`).
    pub decay_ms: f32,
    /// Key-off felt damper release in milliseconds (`20.0..=500.0`).
    pub release_ms: f32,
}

pub type TineKeysParams = TineKeys;

impl Default for TineKeys {
    fn default() -> Self {
        Self {
            bell: 0.55,
            bark: 0.45,
            pickup_asym: 0.40,
            tremolo_rate_hz: 4.2,
            tremolo_depth: 0.38,
            decay_ms: 2100.0,
            release_ms: 95.0,
        }
    }
}

impl TineKeys {
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let mut mix = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        if notes.is_empty() {
            return mix_to_frames(&mix, 1.0, false);
        }

        let bell = clamp_finite(self.bell, 0.0, 1.0);
        let bark = clamp_finite(self.bark, 0.0, 1.0);
        let asym = clamp_finite(self.pickup_asym, 0.0, 1.0);
        let decay_s = clamp_finite(self.decay_ms, 300.0, 5000.0) * 0.001;
        let rel_frames = (clamp_finite(self.release_ms, 20.0, 500.0) * 0.001 * SR) as u32;
        let trem_rate = clamp_finite(self.tremolo_rate_hz, 0.0, 12.0);
        let trem_depth = clamp_finite(self.tremolo_depth, 0.0, 1.0);

        for raw_en in notes {
            let en = ExpressiveNote {
                note: raw_en.note,
                expr: raw_en.expr.sanitized(),
            };
            let vel = en.note.velocity as f32 / 127.0;
            let key_semi = en.note.key as f32 + en.expr.pitch_bend_semitones;
            let f0 = midi_to_hz(key_semi);
            let key_decay = decay_s * exp2_f32(-(en.note.key as f32 - 60.0) / 36.0).clamp(0.35, 1.8);
            let total_len = (en.note.frames + rel_frames).min(total_frames - en.note.start);

            let mut p_fund = 0.0f32;
            let mut p_tine = 0.0f32;
            let mut p_bark = 0.0f32;
            let mut dc = DcBlocker::default();
            let mut hammer_svf = TptSvf::default();
            let mut noise_state = 0x714E_0001u32 ^ (en.note.key as u32 * 997);

            // Stereo key pan subtle spread (bass slightly left, treble slightly right)
            let pan = ((en.note.key as f32 - 60.0) / 48.0).clamp(-0.35, 0.35);
            let pan_ang = (pan + 1.0) * 0.5 * FRAC_PI_2;
            let gain_l = cos_f32(pan_ang);
            let gain_r = sin_f32(pan_ang);

            for i in 0..total_len {
                let t_s = i as f32 * SR_INV;
                let attack = (i as f32 / 36.0).min(1.0);
                let sustain_env = exp_f32(-t_s / key_decay);
                let rel_env = if i >= en.note.frames {
                    (1.0 - (i - en.note.frames) as f32 / rel_frames.max(1) as f32).max(0.0)
                } else {
                    1.0
                };
                let amp_env = attack * sustain_env * rel_env * vel;

                // Tine bell FM (7:1 + 2:1 harmonics, decays quickly into warm tonebar)
                let tine_mod_env = exp_f32(-t_s / 0.14) * (0.3 + 0.7 * vel) * bell * (0.4 + 1.2 * en.expr.timbre);
                let tine_mod = sin_f32(TAU * p_tine) * tine_mod_env * 1.45;
                let tonebar = sin_f32(TAU * p_fund + tine_mod);

                // Hammer bark on hard velocity (2nd/3rd partial transient + neoprene thud)
                let bark_drive = vel * vel * bark * (0.5 + en.expr.timbre);
                let bark_harm = sin_f32(TAU * p_bark) * exp_f32(-t_s / 0.055) * bark_drive * 0.55;
                let hammer_click = hammer_svf.bandpass_norm(
                    next_noise(&mut noise_state) * exp_f32(-t_s / 0.004),
                    (f0 * 4.5).clamp(400.0, 6000.0),
                    1.6,
                    SR,
                ) * vel * 0.32;

                let tine_disp = tonebar + bark_harm;
                // Pickup asymmetry generates dynamic 2nd harmonic growl
                let pickup_raw = tine_disp + asym * 0.42 * tine_disp * tine_disp * (0.5 + vel);
                let pickup = dc.process(tanh_f32(pickup_raw * (0.85 + bark_drive * 0.9)));

                let sample = (pickup + hammer_click) * amp_env * (1.0 + en.expr.pressure * 0.25);

                let idx = (en.note.start + i) as usize;
                mix[idx][0] += sample * gain_l;
                mix[idx][1] += sample * gain_r;

                p_fund = wrap_phase(p_fund + f0 * SR_INV);
                p_tine = wrap_phase(p_tine + (f0 * 7.0).min(SR * 0.45) * SR_INV);
                p_bark = wrap_phase(p_bark + (f0 * 2.0).min(SR * 0.45) * SR_INV);
            }
        }

        // Global stereo tremolo / autopan
        if trem_depth > 0.001 && trem_rate > 0.001 {
            for (i, frame) in mix.iter_mut().enumerate() {
                let lfo = sin_f32(TAU * trem_rate * (i as f32 * SR_INV));
                let mod_l = 1.0 - trem_depth * 0.45 * (1.0 + lfo);
                let mod_r = 1.0 - trem_depth * 0.45 * (1.0 - lfo);
                frame[0] *= mod_l;
                frame[1] *= mod_r;
            }
        }

        mix_to_frames(&mix, 0.50, true)
    }
}

// ============================================================================
// 6. Instrument 4: ModalGrand (Instrument::ModalGrand = 8, "Grand Piano")
// ============================================================================

/// Synthesized acoustic concert piano: velocity-hardened felt hammer filter,
/// detuned 3-string unison inharmonic partials, and soundboard body resonance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModalGrand {
    /// Hammer felt brightness (`0.0..=1.0`).
    pub brightness: f32,
    /// 3-string unison detuning spread (`0.0..=1.0`).
    pub unison_detune: f32,
    /// Stiff-string inharmonicity coefficient scale (`0.0..=1.0`).
    pub inharmonicity: f32,
    /// Soundboard wooden body resonance mix (`0.0..=1.0`).
    pub body_mix: f32,
    /// Base string sustain decay in milliseconds (`500.0..=6000.0`).
    pub decay_ms: f32,
    /// Felt damper release in milliseconds (`30.0..=800.0`).
    pub release_ms: f32,
}

pub type ModalGrandParams = ModalGrand;

impl Default for ModalGrand {
    fn default() -> Self {
        Self {
            brightness: 0.60,
            unison_detune: 0.45,
            inharmonicity: 0.45,
            body_mix: 0.35,
            decay_ms: 2600.0,
            release_ms: 140.0,
        }
    }
}

impl ModalGrand {
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let mut mix = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        if notes.is_empty() {
            return mix_to_frames(&mix, 1.0, false);
        }

        let brightness = clamp_finite(self.brightness, 0.0, 1.0);
        let detune_amt = clamp_finite(self.unison_detune, 0.0, 1.0);
        let inharm = clamp_finite(self.inharmonicity, 0.0, 1.0);
        let body_mix = clamp_finite(self.body_mix, 0.0, 1.0);
        let decay_s = clamp_finite(self.decay_ms, 500.0, 6000.0) * 0.001;
        let rel_frames = (clamp_finite(self.release_ms, 30.0, 800.0) * 0.001 * SR) as u32;

        for raw_en in notes {
            let en = ExpressiveNote {
                note: raw_en.note,
                expr: raw_en.expr.sanitized(),
            };
            let vel = en.note.velocity as f32 / 127.0;
            let key_f = en.note.key as f32 + en.expr.pitch_bend_semitones;
            let f0 = midi_to_hz(key_f);
            // Physical stiff-string dispersion coefficient B increases across scale
            let b_stiff = (0.00004 * exp2_f32((en.note.key as f32 - 21.0) / 14.0) * inharm).clamp(0.0, 0.006);

            // Detuned 3-string unison ratios (cents -> multiplier)
            let cents = 0.25 + detune_amt * 1.45;
            let str_mult = [
                exp2_f32(-cents / 1200.0),
                1.0,
                exp2_f32(cents / 1200.0),
            ];

            // Bass decays slower, high treble decays faster
            let key_decay = decay_s * exp2_f32(-(en.note.key as f32 - 54.0) / 28.0).clamp(0.18, 2.2);
            let total_len = (en.note.frames + rel_frames).min(total_frames - en.note.start);

            // Velocity-hardened felt hammer spectral slope: soft = steep rolloff, hard = rich harmonics
            let eff_hard = (vel * 0.72 + brightness * 0.28 + (en.expr.timbre - 0.5) * 0.45).clamp(0.08, 1.0);
            let slope = 2.6 - eff_hard * 1.45;
            let hammer_start_fc = (450.0 + eff_hard * eff_hard * 11500.0).clamp(300.0, 16000.0);
            let hammer_end_fc = (f0 * (2.5 + eff_hard * 5.0)).clamp(350.0, 9500.0);

            const NUM_PARTIALS: usize = 8;
            let mut p_inc = [[0.0f32; 3]; NUM_PARTIALS];
            let mut p_phase = [[0.0f32; 3]; NUM_PARTIALS];
            let mut p_amp = [0.0f32; NUM_PARTIALS];
            let mut p_decay = [0.0f32; NUM_PARTIALS];

            for k in 0..NUM_PARTIALS {
                let kf = (k + 1) as f32;
                let fk = f0 * kf * sqrt_f32(1.0 + b_stiff * kf * kf);
                if fk < SR * 0.44 {
                    p_amp[k] = 1.0 / powf_f32(kf, slope);
                    p_decay[k] = (key_decay / powf_f32(kf, 0.55)).max(0.05);
                    for s in 0..3 {
                        p_inc[k][s] = fk * str_mult[s] * SR_INV;
                    }
                }
            }

            let mut hammer_svf = TptSvf::default();
            let mut noise_state = 0x64A4_D001u32 ^ (en.note.key as u32 * 313);
            let pan = ((en.note.key as f32 - 60.0) / 52.0).clamp(-0.45, 0.45);
            let pan_ang = (pan + 1.0) * 0.5 * FRAC_PI_2;
            let gain_l = cos_f32(pan_ang);
            let gain_r = sin_f32(pan_ang);

            for i in 0..total_len {
                let t_s = i as f32 * SR_INV;
                let attack = (i as f32 / 24.0).min(1.0);
                let rel_env = if i >= en.note.frames {
                    (1.0 - (i - en.note.frames) as f32 / rel_frames.max(1) as f32).max(0.0)
                } else {
                    1.0
                };

                let mut str_sum = 0.0;
                for k in 0..NUM_PARTIALS {
                    if p_amp[k] <= 0.0 {
                        break;
                    }
                    let partial_env = exp_f32(-t_s / p_decay[k]);
                    let mut unison_s = 0.0;
                    for s in 0..3 {
                        let ph = p_phase[k][s];
                        unison_s += sin_f32(TAU * ph);
                        let next = ph + p_inc[k][s];
                        p_phase[k][s] = if next >= 1.0 { next - floor_f32(next) } else { next };
                    }
                    str_sum += unison_s * (1.0 / 3.0) * p_amp[k] * partial_env;
                }

                // Velocity-hardened felt hammer filter sweeping from impact brightness to warm body
                let sweep = exp_f32(-t_s / 0.035);
                let fc = hammer_end_fc + (hammer_start_fc - hammer_end_fc) * sweep;
                let felt_filtered = hammer_svf.lowpass(str_sum, fc, 0.72, SR);

                // Subtle wooden key-bed / felt thud at impact
                let thud = if i < 192 {
                    next_noise(&mut noise_state) * exp_f32(-t_s / 0.0018) * 0.08 * vel
                } else {
                    0.0
                };

                let sample = (felt_filtered + thud) * attack * rel_env * vel * (1.0 + en.expr.pressure * 0.2);
                let idx = (en.note.start + i) as usize;
                mix[idx][0] += sample * gain_l;
                mix[idx][1] += sample * gain_r;
            }
        }

        // Shared wooden soundboard body resonance (4 stereo modes + subtle wood reflection)
        if body_mix > 0.001 {
            let mut sb_modes_l = [Biquad::default(); 4];
            let mut sb_modes_r = [Biquad::default(); 4];
            const SB_F: [f32; 4] = [108.0, 224.0, 415.0, 880.0];
            const SB_G: [f32; 4] = [3.5, 2.8, 2.2, 1.6];
            for k in 0..4 {
                sb_modes_l[k].set_peaking(SB_F[k], 2.4, SB_G[k] * body_mix, SR);
                sb_modes_r[k].set_peaking(SB_F[k] * 1.03, 2.4, SB_G[k] * body_mix, SR);
            }
            let mut body_del_l = try_alloc_vec(132usize, 0.0f32)?; // 2.75 ms soundboard travel
            let mut body_del_r = try_alloc_vec(199usize, 0.0f32)?; // 4.15 ms
            let mut pos_l = 0usize;
            let mut pos_r = 0usize;
            for frame in mix.iter_mut() {
                let mut l = frame[0];
                let mut r = frame[1];
                for m in &mut sb_modes_l {
                    l = m.process(l);
                }
                for m in &mut sb_modes_r {
                    r = m.process(r);
                }
                let ref_l = body_del_l[pos_l];
                let ref_r = body_del_r[pos_r];
                body_del_l[pos_l] = l * 0.45 + ref_r * 0.15;
                body_del_r[pos_r] = r * 0.45 + ref_l * 0.15;
                pos_l = (pos_l + 1) % body_del_l.len();
                pos_r = (pos_r + 1) % body_del_r.len();
                frame[0] = l + ref_l * body_mix * 0.25;
                frame[1] = r + ref_r * body_mix * 0.25;
            }
        }

        mix_to_frames(&mix, 0.46, true)
    }
}

// ============================================================================
// 7. Instrument 5: PluckGuitar (Instrument::PluckGuitar = 9, "Electric Guitar")
// ============================================================================

/// String damping articulation for `PluckGuitar`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluckMode {
    Sustained,
    PalmMuted,
}

/// 2x-oversampled electric guitar amplifier + tone stack + 4x12 speaker cabinet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuitarAmp {
    /// Preamp / power-tube saturation gain (`0.0..=1.0`).
    pub gain: f32,
    /// Low tone-stack control (`0.0..=1.0`, `0.5` neutral).
    pub bass: f32,
    /// Mid tone-stack control (`0.0..=1.0`, `0.5` neutral).
    pub mid: f32,
    /// High tone-stack control (`0.0..=1.0`, `0.5` neutral).
    pub treble: f32,
    /// 4x12 speaker cabinet resonance & rolloff (`0.0..=1.0`).
    pub cabinet: f32,
}

impl Default for GuitarAmp {
    fn default() -> Self {
        Self {
            gain: 0.45,
            bass: 0.55,
            mid: 0.50,
            treble: 0.60,
            cabinet: 0.85,
        }
    }
}

/// Waveguide plucked electric guitar feeding a 2x-oversampled `GuitarAmp`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PluckGuitar {
    pub mode: PluckMode,
    /// Pick position along string (`0.0..=1.0`, near bridge bright vs neck hollow).
    pub pick_pos: f32,
    /// String / pickup brightness (`0.0..=1.0`).
    pub brightness: f32,
    /// Release fret slide/squeak noise (`0.0..=1.0`).
    pub fret_noise: f32,
    pub amp: GuitarAmp,
}

pub type PluckGuitarParams = PluckGuitar;

impl Default for PluckGuitar {
    fn default() -> Self {
        Self {
            mode: PluckMode::Sustained,
            pick_pos: 0.22,
            brightness: 0.65,
            fret_noise: 0.30,
            amp: GuitarAmp::default(),
        }
    }
}

impl PluckGuitar {
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let mut dry_mix = try_alloc_vec(total_frames as usize, 0.0f32)?;
        if notes.is_empty() {
            let empty_stereo = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
            return mix_to_frames(&empty_stereo, 1.0, false);
        }

        const MAX_DELAY: usize = 6144; // handles down to 7.8 Hz (MIDI 0)
        let mut delay_buf = try_alloc_vec(MAX_DELAY, 0.0f32)?;
        let mut pick_buf = try_alloc_vec(MAX_DELAY, 0.0f32)?;
        let pick_pos = clamp_finite(self.pick_pos, 0.05, 0.5);
        let brightness = clamp_finite(self.brightness, 0.05, 1.0);
        let fret_amt = clamp_finite(self.fret_noise, 0.0, 1.0);
        let rel_frames = if self.mode == PluckMode::PalmMuted { 960u32 } else { 3840u32 };

        for (note_idx, raw_en) in notes.iter().enumerate() {
            let en = ExpressiveNote {
                note: raw_en.note,
                expr: raw_en.expr.sanitized(),
            };
            let vel = en.note.velocity as f32 / 127.0;
            let key_f = en.note.key as f32 + en.expr.pitch_bend_semitones;
            let freq = midi_to_hz(key_f).clamp(20.0, 4200.0);

            let eff_bright = (brightness + (en.expr.timbre - 0.5) * 0.5).clamp(0.05, 0.98);
            let (base_fb, damp_lp) = match self.mode {
                PluckMode::Sustained => (0.9965 + 0.002 * en.expr.pressure, 0.14 + (1.0 - eff_bright) * 0.35),
                PluckMode::PalmMuted => (0.935, 0.68 + (1.0 - eff_bright) * 0.22),
            };
            let lp_delay = (damp_lp / (1.0 - damp_lp).max(0.05)).min(3.0);

            // Waveguide delay length with 1st-order Thiran allpass fractional delay
            let exact_delay = (SR / freq - lp_delay).clamp(5.0, (MAX_DELAY - 6) as f32);
            let mut int_delay = floor_f32(exact_delay) as usize;
            let mut frac = exact_delay - int_delay as f32;
            if frac < 0.5 && int_delay > 4 {
                int_delay -= 1;
                frac += 1.0;
            }
            let eta = (1.0 - frac) / (1.0 + frac);

            delay_buf.fill(0.0);
            pick_buf.fill(0.0);

            // Fill initial pick burst + comb filter for pick position
            let mut noise_state = 0x57B1_6001u32 ^ ((note_idx as u32 + 1) * 7919);
            let comb_offset = ((int_delay as f32 * pick_pos) as usize).clamp(1, int_delay.max(2) - 1);
            let mut pick_lp = 0.0f32;
            let pick_alpha = (0.18 + brightness * 0.55 * (0.5 + vel * 0.5)).clamp(0.1, 0.85);
            for k in 0..int_delay {
                let r = next_noise(&mut noise_state);
                pick_lp += (r - pick_lp) * pick_alpha;
                pick_buf[k] = pick_lp;
            }
            for k in 0..int_delay {
                let delayed_pick = if k >= comb_offset {
                    pick_buf[k - comb_offset]
                } else {
                    0.0
                };
                delay_buf[k] = (pick_buf[k] - delayed_pick * 0.72) * vel;
            }

            let mut write_pos = int_delay % MAX_DELAY;
            let mut lp_state = 0.0f32;
            let mut ap_x1 = 0.0f32;
            let mut ap_y1 = 0.0f32;
            let mut fret_svf = TptSvf::default();
            let total_len = (en.note.frames + rel_frames).min(total_frames - en.note.start);

            for i in 0..total_len {
                let read_pos = (write_pos + MAX_DELAY - int_delay) % MAX_DELAY;
                let delayed = delay_buf[read_pos];

                // Allpass fractional delay interpolation
                let ap_out = eta * (delayed - ap_y1) + ap_x1;
                ap_x1 = delayed;
                ap_y1 = ap_out;

                // String damping lowpass inside waveguide loop
                lp_state += (ap_out - lp_state) * (1.0 - damp_lp);

                // Gate release increases string damping + injects release fret noise
                let (fb, rel_fret) = if i >= en.note.frames {
                    let ri = i - en.note.frames;
                    let rel_env = (1.0 - ri as f32 / rel_frames.max(1) as f32).max(0.0);
                    let fret_env = if ri < 1440 && fret_amt > 0.001 {
                        exp_f32(-(ri as f32) / 360.0) * fret_amt * vel
                    } else {
                        0.0
                    };
                    let fret_sig = fret_svf.bandpass_norm(
                        next_noise(&mut noise_state) * fret_env,
                        2700.0 + (en.note.key as f32 * 18.0),
                        3.5,
                        SR,
                    );
                    (base_fb * (0.92 + 0.08 * rel_env), fret_sig * 0.25)
                } else {
                    (base_fb, 0.0)
                };

                delay_buf[write_pos] = lp_state * fb;
                write_pos = (write_pos + 1) % MAX_DELAY;

                let idx = (en.note.start + i) as usize;
                dry_mix[idx] += ap_out * 0.55 + rel_fret;
            }
        }

        // 2x-oversampled GuitarAmp + Tone Stack + 4x12 Cabinet Speaker Resonance
        let mut stereo = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        let gain = clamp_finite(self.amp.gain, 0.0, 1.0);
        let bass_db = (clamp_finite(self.amp.bass, 0.0, 1.0) - 0.5) * 14.0;
        let mid_db = (clamp_finite(self.amp.mid, 0.0, 1.0) - 0.5) * 14.0;
        let treble_db = (clamp_finite(self.amp.treble, 0.0, 1.0) - 0.5) * 14.0;
        let cab_amt = clamp_finite(self.amp.cabinet, 0.0, 1.0);

        let mut os2 = Oversampler2x::default();
        let mut dc = DcBlocker::default();
        let mut eq_bass = Biquad::default();
        let mut eq_mid = Biquad::default();
        let mut eq_treble = Biquad::default();
        eq_bass.set_low_shelf(125.0, bass_db, SR);
        eq_mid.set_peaking(750.0, 1.1, mid_db, SR);
        eq_treble.set_high_shelf(2900.0, treble_db, SR);

        // 4x12 Cabinet Speaker model: resonant thump @ 95Hz, cone presence @ 2.2kHz, dual steep rolloff @ 4.6kHz
        let mut cab_hp = TptSvf::default();
        let mut cab_thump = Biquad::default();
        let mut cab_cone = Biquad::default();
        let mut cab_lp1 = TptSvf::default();
        let mut cab_lp2 = TptSvf::default();
        cab_thump.set_peaking(98.0, 1.6, 4.0 * cab_amt, SR);
        cab_cone.set_peaking(2250.0, 1.5, 3.2 * cab_amt, SR);

        // Subtle stereo cabinet air delay (2.2 ms right-side mic offset)
        let mut cab_air = try_alloc_vec(106usize, 0.0f32)?;
        let mut air_pos = 0usize;

        let drive = 1.1 + gain * 14.0;
        let bias = 0.14 * gain;
        for (i, &dry) in dry_mix.iter().enumerate() {
            // Stage 1: 2x oversampled asymmetric preamp/power-tube stage
            let amped = os2.process(dry, |s| {
                let u = s * drive + bias;
                let s1 = if u >= 0.0 {
                    tanh_f32(u * 1.15)
                } else {
                    tanh_f32(u * 0.78)
                };
                tanh_f32(s1 * (1.0 + gain * 1.6))
            });
            let clean_dc = dc.process(amped);

            // Stage 2: Bass / Mid / Treble tone stack
            let stacked = eq_treble.process(eq_mid.process(eq_bass.process(clean_dc)));

            // Stage 3: 4x12 speaker cabinet resonance & steep rolloff
            let sig = if cab_amt > 0.001 {
                let hp = cab_hp.highpass(stacked, 72.0, 0.8, SR);
                let thumped = cab_cone.process(cab_thump.process(hp));
                let lp1 = cab_lp1.lowpass(thumped, 4800.0 - cab_amt * 600.0, 0.75, SR);
                let lp2 = cab_lp2.lowpass(lp1, 5200.0 - cab_amt * 700.0, 0.75, SR);
                stacked * (1.0 - cab_amt) + lp2 * cab_amt
            } else {
                stacked
            };

            let delayed = cab_air[air_pos];
            cab_air[air_pos] = sig;
            air_pos = (air_pos + 1) % cab_air.len();
            let l = sig * 0.85 + delayed * 0.15 * cab_amt;
            let r = delayed * 0.85 + sig * 0.15 * cab_amt;
            stereo[i] = [l * 0.65, r * 0.65];
        }

        mix_to_frames(&stereo, 1.0, true)
    }
}

// ============================================================================
// 8. Instrument 6: ModalMallet (Instrument::ModalMallet = 10, "Mallet Bells")
// ============================================================================

/// Tuned bar / bell resonator geometry for `ModalMallet`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MalletKind {
    /// Rosewood marimba tuning (`1 : 3.99 : 9.2`).
    Marimba,
    /// Chime / tubular bell inharmonic partials.
    TubularBell,
    /// Metallic vibraphone bar with motor tremolo shimmer.
    Vibraphone,
}

/// Modal bar/bell percussion synthesizer with velocity- & timbre-controlled mallet hardness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModalMallet {
    pub kind: MalletKind,
    /// Base mallet hardness (`0.0..=1.0`).
    pub hardness: f32,
    /// Fundamental modal decay in milliseconds (`200.0..=4000.0`).
    pub decay_ms: f32,
    /// Vibraphone / bell shimmer tremolo depth (`0.0..=1.0`).
    pub shimmer: f32,
    /// Stereo keyspread width (`0.0..=1.0`).
    pub stereo_width: f32,
}

pub type ModalMalletParams = ModalMallet;

impl Default for ModalMallet {
    fn default() -> Self {
        Self {
            kind: MalletKind::Marimba,
            hardness: 0.45,
            decay_ms: 1400.0,
            shimmer: 0.25,
            stereo_width: 0.55,
        }
    }
}

impl ModalMallet {
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let mut mix = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        if notes.is_empty() {
            return mix_to_frames(&mix, 1.0, false);
        }

        let hardness = clamp_finite(self.hardness, 0.0, 1.0);
        let decay_s = clamp_finite(self.decay_ms, 200.0, 4000.0) * 0.001;
        let shimmer = clamp_finite(self.shimmer, 0.0, 1.0);
        let width = clamp_finite(self.stereo_width, 0.0, 1.0);

        for raw_en in notes {
            let en = ExpressiveNote {
                note: raw_en.note,
                expr: raw_en.expr.sanitized(),
            };
            let vel = en.note.velocity as f32 / 127.0;
            let key_f = en.note.key as f32 + en.expr.pitch_bend_semitones;
            let f0 = midi_to_hz(key_f);

            // Mallet hardness driven by base hardness + velocity + per-note timbre
            let eff_hard = (hardness * 0.45 + vel * 0.35 + (en.expr.timbre - 0.5) * 0.6).clamp(0.05, 1.0);
            let strike_frames = ((1.0 - eff_hard) * 96.0 + 10.0) as u32;

            let (ratios, base_amps, key_decay_s): ([f32; 6], [f32; 6], f32) = match self.kind {
                MalletKind::Marimba => (
                    [1.0, 3.99, 9.2, 16.5, 1.0, 1.0],
                    [1.0, 0.55, 0.32, 0.14, 0.0, 0.0],
                    decay_s * exp2_f32(-(en.note.key as f32 - 60.0) / 36.0).clamp(0.25, 1.8),
                ),
                MalletKind::TubularBell => (
                    [1.0, 2.0, 2.76, 4.07, 5.40, 8.93],
                    [0.9, 0.72, 0.65, 0.48, 0.36, 0.22],
                    (decay_s * 1.6).clamp(0.4, 5.5),
                ),
                MalletKind::Vibraphone => (
                    [1.0, 3.98, 9.95, 18.2, 1.0, 1.0],
                    [1.0, 0.48, 0.24, 0.10, 0.0, 0.0],
                    decay_s * 1.35,
                ),
            };

            let mut modes = [ModalResonator::default(); 6];
            for k in 0..6 {
                if base_amps[k] <= 0.0 {
                    continue;
                }
                let fk = f0 * ratios[k];
                // Higher mallet hardness disproportionately boosts upper modal partials
                let hard_boost = powf_f32(eff_hard, k as f32 * 0.75);
                let mode_decay = (key_decay_s / powf_f32(ratios[k], 0.62)).max(0.02);
                modes[k].configure(fk, mode_decay, base_amps[k] * hard_boost);
            }

            let tail_frames = (key_decay_s * 0.75 * SR) as u32;
            let total_len = (en.note.frames + tail_frames).min(total_frames - en.note.start);

            let pan = ((en.note.key as f32 - 60.0) / 48.0 * width).clamp(-0.6, 0.6);
            let pan_ang = (pan + 1.0) * 0.5 * FRAC_PI_2;
            let gain_l = cos_f32(pan_ang);
            let gain_r = sin_f32(pan_ang);

            for i in 0..total_len {
                // Raised half-sine mallet contact pulse
                let pulse = if i < strike_frames {
                    sin_f32(PI * (i as f32 + 0.5) / strike_frames as f32)
                        * (1.9 / strike_frames as f32)
                        * vel
                } else {
                    0.0
                };

                let mut s = 0.0;
                for m in &mut modes {
                    if m.r > 0.0 {
                        if pulse > 0.0 {
                            m.excite(pulse);
                        }
                        s += m.step();
                    }
                }

                // Gentle vibraphone / bell motor shimmer
                let shim_mod = if shimmer > 0.001 {
                    let rate = if self.kind == MalletKind::Vibraphone { 5.4 } else { 3.2 };
                    1.0 - shimmer * 0.35 * (0.5 + 0.5 * sin_f32(TAU * rate * (i as f32 * SR_INV)))
                } else {
                    1.0
                };

                // Softly release after note gate for damped notes unless ringing tubular bell
                let gate_damp = if i >= en.note.frames && self.kind != MalletKind::TubularBell {
                    let over = (i - en.note.frames) as f32 * SR_INV;
                    exp_f32(-over / 0.12)
                } else {
                    1.0
                };

                let val = s * shim_mod * gate_damp * (1.0 + en.expr.pressure * 0.25);
                let idx = (en.note.start + i) as usize;
                mix[idx][0] += val * gain_l;
                mix[idx][1] += val * gain_r;
            }
        }

        mix_to_frames(&mix, 0.55, true)
    }
}

// ============================================================================
// 9. Instrument 7: CrossSynth (Instrument::CrossSynth = 11, "Cross Synth")
// ============================================================================

/// Dual-oscillator Cross-FM + Oscillator Hard Sync + Ring Modulation synthesizer
/// with resonant 12 dB/oct TPT SVF filter and ADSR envelopes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossSynth {
    /// Oscillator B frequency ratio relative to Oscillator A (`0.25..=8.0`).
    pub osc_b_ratio: f32,
    /// Oscillator B fine detune in cents (`-50.0..=50.0`).
    pub osc_b_detune_cents: f32,
    /// Bidirectional Cross-FM modulation depth (`0.0..=1.0`).
    pub cross_fm: f32,
    /// Oscillator Hard Sync sweep amount (`0.0..=1.0`, `0.0` disables hard sync).
    pub hard_sync: f32,
    /// Ring modulation blend (`0.0..=1.0`).
    pub ring_mod: f32,
    /// Balance between Osc A (`0.0`) and Osc B (`1.0`).
    pub mix_ab: f32,
    /// Resonant SVF lowpass cutoff (`40.0..=18000.0` Hz).
    pub cutoff_hz: f32,
    /// SVF resonance (`0.0..=0.95`).
    pub resonance: f32,
    /// Filter envelope modulation amount (`0.0..=1.0`).
    pub env_to_cutoff: f32,
    /// Amplitude/filter attack (`1.0..=2000.0` ms).
    pub attack_ms: f32,
    /// Amplitude/filter decay (`20.0..=3000.0` ms).
    pub decay_ms: f32,
    /// Sustain level (`0.0..=1.0`).
    pub sustain: f32,
    /// Release time (`10.0..=3000.0` ms).
    pub release_ms: f32,
    /// Stereo unison/detune spread (`0.0..=1.0`).
    pub stereo_spread: f32,
}

pub type CrossSynthParams = CrossSynth;

impl Default for CrossSynth {
    fn default() -> Self {
        Self {
            osc_b_ratio: 1.5,
            osc_b_detune_cents: 7.0,
            cross_fm: 0.30,
            hard_sync: 0.35,
            ring_mod: 0.15,
            mix_ab: 0.50,
            cutoff_hz: 2400.0,
            resonance: 0.35,
            env_to_cutoff: 0.55,
            attack_ms: 12.0,
            decay_ms: 280.0,
            sustain: 0.70,
            release_ms: 160.0,
            stereo_spread: 0.40,
        }
    }
}

impl CrossSynth {
    pub fn render_pattern(
        &self,
        notes: &[Note],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        let expressive = notes_to_expressive(notes)?;
        self.render_expressive(&expressive, total_frames)
    }

    pub fn render_expressive(
        &self,
        notes: &[ExpressiveNote],
        total_frames: u32,
    ) -> Result<Vec<Frame>, midi::Error> {
        validate_expressive(notes, total_frames)?;
        let mut mix = try_alloc_vec(total_frames as usize, [0.0f32; 2])?;
        if notes.is_empty() {
            return mix_to_frames(&mix, 1.0, false);
        }

        let ratio_b = clamp_finite(self.osc_b_ratio, 0.25, 8.0);
        let detune_mult = exp2_f32(clamp_finite(self.osc_b_detune_cents, -50.0, 50.0) / 1200.0);
        let cross_fm = clamp_finite(self.cross_fm, 0.0, 1.0);
        let hard_sync = clamp_finite(self.hard_sync, 0.0, 1.0);
        let ring_mod = clamp_finite(self.ring_mod, 0.0, 1.0);
        let mix_ab = clamp_finite(self.mix_ab, 0.0, 1.0);
        let base_cutoff = clamp_finite(self.cutoff_hz, 40.0, 18000.0);
        let res = clamp_finite(self.resonance, 0.0, 0.95);
        let q = 0.707 + res * 9.0;
        let env_cut = clamp_finite(self.env_to_cutoff, 0.0, 1.0);
        let attack_frames = (clamp_finite(self.attack_ms, 1.0, 2000.0) * 0.001 * SR).max(1.0);
        let decay_tau_s = (clamp_finite(self.decay_ms, 20.0, 3000.0) * 0.001) / 3.0;
        let sustain = clamp_finite(self.sustain, 0.0, 1.0);
        let rel_frames = (clamp_finite(self.release_ms, 10.0, 3000.0) * 0.001 * SR) as u32;
        let spread = clamp_finite(self.stereo_spread, 0.0, 1.0);

        for raw_en in notes {
            let en = ExpressiveNote {
                note: raw_en.note,
                expr: raw_en.expr.sanitized(),
            };
            let vel = en.note.velocity as f32 / 127.0;
            let key_f = en.note.key as f32 + en.expr.pitch_bend_semitones;
            let f_a = midi_to_hz(key_f).clamp(16.0, 12000.0);
            let total_len = (en.note.frames + rel_frames).min(total_frames - en.note.start);

            let mut p_a = 0.0f32;
            let mut p_b = 0.0f32;
            let mut prev_a = 0.0f32;
            let mut prev_b = 0.0f32;
            let mut svf_l = TptSvf::default();
            let mut svf_r = TptSvf::default();
            let pan = ((en.note.key as f32 - 60.0) / 48.0 * spread * 0.5).clamp(-0.4, 0.4);
            let pan_ang = (pan + 1.0) * 0.5 * FRAC_PI_2;
            let gain_l = cos_f32(pan_ang);
            let gain_r = sin_f32(pan_ang);

            for i in 0..total_len {
                // ADSR envelope
                let env_a = if (i as f32) < attack_frames {
                    i as f32 / attack_frames
                } else {
                    let dt = (i as f32 - attack_frames) * SR_INV;
                    sustain + (1.0 - sustain) * exp_f32(-dt / decay_tau_s.max(0.005))
                };
                let rel_env = if i >= en.note.frames {
                    (1.0 - (i - en.note.frames) as f32 / rel_frames.max(1) as f32).max(0.0)
                } else {
                    1.0
                };
                let env = env_a * rel_env;

                let vibrato = if en.expr.pressure > 0.01 {
                    1.0 + sin_f32(TAU * 5.2 * (i as f32 * SR_INV)) * 0.012 * en.expr.pressure
                } else {
                    1.0
                };
                let cur_fa = (f_a * vibrato).clamp(16.0, 14000.0);
                let dt_a = cur_fa * SR_INV;

                // Hard Sync multiplier sweeps Osc B upward when hard_sync > 0
                let sync_sweep = 1.0 + hard_sync * (2.6 * env + en.expr.timbre * 1.4);
                let cur_fb = (cur_fa * ratio_b * detune_mult * sync_sweep).clamp(16.0, 18000.0);
                let dt_b = cur_fb * SR_INV;

                // Cross-FM phase modulation
                let fm_depth = cross_fm * (0.4 + 1.1 * en.expr.timbre);
                let pa_mod = p_a + prev_b * fm_depth * 0.25;
                let pa_wrap = pa_mod - floor_f32(pa_mod);
                let pb_mod = p_b + prev_a * fm_depth * 0.25;
                let pb_wrap = pb_mod - floor_f32(pb_mod);

                // PolyBLEP Saw for Osc A, PolyBLEP Pulse/Saw for Osc B
                let out_a = 2.0 * pa_wrap - 1.0 - poly_blep(pa_wrap, dt_a);
                let mut out_b = if pb_wrap < 0.5 { 1.0 } else { -1.0 };
                out_b += poly_blep(pb_wrap, dt_b);
                out_b -= poly_blep((pb_wrap + 0.5) - floor_f32(pb_wrap + 0.5), dt_b);
                // Blend Osc B with sine fundamental for smooth cross-FM pads
                out_b = out_b * 0.65 + sin_f32(TAU * pb_wrap) * 0.35;

                prev_a = out_a;
                prev_b = out_b;

                // Ring Modulation + Osc A/B Mix
                let osc_mix = out_a * (1.0 - mix_ab) + out_b * mix_ab;
                let ring = out_a * out_b;
                let raw = osc_mix * (1.0 - ring_mod * 0.65) + ring * (ring_mod * 0.9);

                // Resonant TPT SVF Filter modulated by ADSR + velocity + timbre
                let mod_oct = (env * env_cut * 3.5 + (en.expr.timbre - 0.5) * 2.5 + vel * 0.8)
                    .clamp(-2.5, 4.5);
                let fc = (base_cutoff * exp2_f32(mod_oct)).clamp(40.0, 18000.0);
                let filt_l = svf_l.lowpass(raw, fc, q, SR);
                let filt_r = svf_r.lowpass(raw * (1.0 - spread * 0.15) + out_b * spread * 0.15, fc * (1.0 + spread * 0.04), q, SR);

                let idx = (en.note.start + i) as usize;
                mix[idx][0] += filt_l * env * vel * gain_l;
                mix[idx][1] += filt_r * env * vel * gain_r;

                // Advance phases & apply Hard Sync reset when master Osc A wraps
                p_a += dt_a;
                p_b += dt_b;
                if p_a >= 1.0 {
                    p_a -= floor_f32(p_a);
                    if hard_sync > 0.01 {
                        // Sub-sample accurate hard sync reset
                        p_b = p_a * (dt_b / dt_a.max(1e-6));
                    }
                }
                if p_b >= 1.0 {
                    p_b -= floor_f32(p_b);
                }
            }
        }

        mix_to_frames(&mix, 0.42, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_alloc_vec_returns_memory_error_on_oom() {
        let res = try_alloc_vec::<[f32; 2]>(usize::MAX / 4, [0.0; 2]);
        assert_eq!(res, Err(midi::Error::Memory));
    }

    #[test]
    fn oversamplers_preserve_unity_gain_at_dc() {
        let mut os2 = Oversampler2x::default();
        let mut y2 = 0.0;
        for _ in 0..256 {
            y2 = os2.process(1.0, |s| s);
        }
        assert!(abs_f32(y2 - 1.0) < 1e-3, "2x oversampler DC={y2}");

        let mut os4 = Oversampler4x::default();
        let mut y4 = 0.0;
        for _ in 0..256 {
            y4 = os4.process(1.0, |s| s);
        }
        // DC blocker cancels steady DC after settling; check impulse/1kHz sine passes cleanly
        assert!(abs_f32(y4) < 0.9, "4x oversampler + DC blocker suppresses steady DC ({y4})");

        let mut os4_sine = Oversampler4x::default();
        let mut peak = 0.0f32;
        for i in 0..480 {
            let x = sin_f32(TAU * 440.0 * i as f32 / SR);
            let y = os4_sine.process(x, |s| s);
            if i > 120 {
                peak = peak.max(abs_f32(y));
            }
        }
        assert!(abs_f32(peak - 1.0) < 0.02, "4x oversampler 440Hz gain={peak}");
    }
}

