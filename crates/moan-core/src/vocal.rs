// SPDX-License-Identifier: GPL-3.0-only
//! Ariel: Native `#![no_std]` parallel-formant vocal & singing synthesizer at 48 kHz.
//! Inspired by tgies/klattsch (MIT) and Dennis Klatt (1980).

use crate::{
    dsp_math::{
        abs_f32, cos_f32, exp2_f32, floor_f32, sin_f32, PI,
    },
    midi, Frame, RATE,
};
use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Memory,
    Parse,
    Range,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceParams {
    pub f0: f32,            // default 120.0 Hz
    pub rate_ms: f32,       // default 110.0 ms per phoneme slot
    pub formant_scale: f32, // default 1.0 (clamped 0.70..=1.50)
    pub vibrato_depth: f32, // default 0.0 (0..=15 Hz peak deviation)
    pub vibrato_rate: f32,  // default 5.0 Hz
    pub tremolo_depth: f32, // default 0.0 (0..=1.0 depth)
    pub tremolo_rate: f32,  // default 5.0 Hz
    pub aspiration: f32,    // default 0.0 (0..=1.0 breathiness)
    pub tilt: f32,          // default 0.0 (-0.9..=0.9 spectral tilt)
    pub effort: f32,        // default 0.5 (0..=1.0 Rosenberg open/closed quotient)
    pub gain: f32,          // default 1.0
}

impl Default for VoiceParams {
    fn default() -> Self {
        Self {
            f0: 120.0,
            rate_ms: 110.0,
            formant_scale: 1.0,
            vibrato_depth: 0.0,
            vibrato_rate: 5.0,
            tremolo_depth: 0.0,
            tremolo_rate: 5.0,
            aspiration: 0.0,
            tilt: 0.0,
            effort: 0.5,
            gain: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhraseSpan {
    pub byte_start: usize,
    pub byte_end: usize,
    pub start_frame: usize,
    pub end_frame: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormantTargets {
    pub f1: f32,
    pub f2: f32,
    pub f3: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phoneme {
    pub voicing: f32,
    pub f1: f32,
    pub f2: f32,
    pub f3: f32,
    pub bw1: f32,
    pub bw2: f32,
    pub bw3: f32,
    pub a1: f32,
    pub a2: f32,
    pub a3: f32,
    pub is_stop: bool,
    pub glide_to: Option<FormantTargets>,
}

impl Phoneme {
    pub const fn new(
        voicing: f32,
        f1: f32,
        f2: f32,
        f3: f32,
        bw1: f32,
        bw2: f32,
        bw3: f32,
        a1: f32,
        a2: f32,
        a3: f32,
        is_stop: bool,
        glide_to: Option<FormantTargets>,
    ) -> Self {
        Self {
            voicing,
            f1,
            f2,
            f3,
            bw1,
            bw2,
            bw3,
            a1,
            a2,
            a3,
            is_stop,
            glide_to,
        }
    }
}

// Low-level DSP primitives

/// Constant-skirt-gain bandpass biquad (RBJ Audio EQ Cookbook).
#[derive(Clone, Debug, Default)]
pub struct BandpassBiquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    last_f: f32,
    last_bw: f32,
}

impl BandpassBiquad {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_freq(&mut self, mut f: f32, mut bw: f32, sr: f32) {
        if f == self.last_f && bw == self.last_bw {
            return;
        }
        self.last_f = f;
        self.last_bw = bw;
        if f < 40.0 {
            f = 40.0;
        } else if f > sr * 0.45 {
            f = sr * 0.45;
        }
        if bw < 20.0 {
            bw = 20.0;
        }
        let w0 = 2.0 * PI * f / sr;
        let cosw0 = cos_f32(w0);
        let sinw0 = sin_f32(w0);
        let q = f / bw;
        let alpha = sinw0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b1 = 0.0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * cosw0 / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
        self.last_f = -1.0;
        self.last_bw = -1.0;
    }
}

/// Derivative of the Rosenberg glottal pulse. Phase normalized to [0, 1).
#[inline]
pub fn glottal_pulse(phase: f32, effort: f32) -> f32 {
    let e = effort.clamp(0.0, 1.0);
    let tp = 0.5 - e * 0.2;
    let tn = 0.25 - e * 0.17;
    const NORM: f32 = 0.1;
    if phase < tp {
        NORM * 0.5 * (PI / tp) * sin_f32(PI * phase / tp)
    } else if phase < tp + tn {
        -NORM * (PI / (2.0 * tn)) * sin_f32(PI * (phase - tp) / (2.0 * tn))
    } else {
        0.0
    }
}

/// 32-bit xorshift LFSR returning normalized float in [-1.0, 1.0].
#[inline]
pub fn xorshift(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as i32 as f32) / 2147483648.0
}

/// Soft-clip with linear region up to ±0.85 and smooth knee.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const T: f32 = 0.85;
    let a = abs_f32(x);
    if a <= T {
        x
    } else {
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        let excess = a - T;
        sign * (T + (1.0 - T) * excess / (excess + 1.0))
    }
}

// Built-in ARPABET & Japanese Mokhtari banks

pub trait PhonemeBank: Send + Sync {
    fn get(&self, code: &str) -> Option<Phoneme>;
}

pub struct BuiltinBank {
    pub is_japanese: bool,
}

impl PhonemeBank for BuiltinBank {
    fn get(&self, code: &str) -> Option<Phoneme> {
        if self.is_japanese {
            if let Some(p) = lookup_ja_mokhtari(code) {
                return Some(p);
            }
        }
        lookup_arpabet(code)
    }
}

fn lookup_ja_mokhtari(code: &str) -> Option<Phoneme> {
    match code {
        "A" => Some(Phoneme::new(1.0, 744.0, 1240.0, 2426.0, 182.0, 117.0, 222.0, 1.0, 0.9, 0.7, false, None)),
        "I" => Some(Phoneme::new(1.0, 298.0, 2083.0, 2954.0, 67.0, 130.0, 145.0, 1.0, 0.9, 0.7, false, None)),
        "U" => Some(Phoneme::new(1.0, 355.0, 1282.0, 2233.0, 55.0, 129.0, 135.0, 1.0, 0.9, 0.7, false, None)),
        "E" => Some(Phoneme::new(1.0, 480.0, 1857.0, 2437.0, 65.0, 97.0, 257.0, 1.0, 0.9, 0.7, false, None)),
        "O" => Some(Phoneme::new(1.0, 460.0, 857.0, 2405.0, 61.0, 114.0, 134.0, 1.0, 0.9, 0.7, false, None)),
        "DX" => Some(Phoneme::new(0.6, 255.0, 1325.0, 2740.0, 55.0, 100.0, 225.0, 0.65, 0.55, 0.5, true, None)),
        _ => None,
    }
}

fn lookup_arpabet(code: &str) -> Option<Phoneme> {
    match code {
        // Vowels
        "IY" => Some(Phoneme::new(1.0, 310.0, 2020.0, 2960.0, 45.0, 200.0, 400.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 290.0, f2: 2070.0, f3: 2960.0 }))),
        "IH" => Some(Phoneme::new(1.0, 400.0, 1800.0, 2570.0, 50.0, 100.0, 140.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 470.0, f2: 1600.0, f3: 2600.0 }))),
        "EH" => Some(Phoneme::new(1.0, 530.0, 1680.0, 2500.0, 60.0, 90.0, 200.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 620.0, f2: 1530.0, f3: 2530.0 }))),
        "AE" => Some(Phoneme::new(1.0, 620.0, 1660.0, 2430.0, 70.0, 150.0, 320.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 650.0, f2: 1490.0, f3: 2470.0 }))),
        "AA" => Some(Phoneme::new(1.0, 700.0, 1220.0, 2600.0, 130.0, 70.0, 160.0, 1.0, 0.9, 0.7, false, None)),
        "AO" => Some(Phoneme::new(1.0, 600.0, 990.0, 2570.0, 90.0, 100.0, 80.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 630.0, f2: 1040.0, f3: 2600.0 }))),
        "AH" => Some(Phoneme::new(1.0, 620.0, 1220.0, 2550.0, 80.0, 50.0, 140.0, 1.0, 0.9, 0.7, false, None)),
        "UH" => Some(Phoneme::new(1.0, 450.0, 1100.0, 2350.0, 80.0, 100.0, 80.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 500.0, f2: 1180.0, f3: 2390.0 }))),
        "UW" => Some(Phoneme::new(1.0, 350.0, 1250.0, 2200.0, 65.0, 110.0, 140.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 320.0, f2: 900.0, f3: 2200.0 }))),
        "ER" => Some(Phoneme::new(1.0, 470.0, 1270.0, 1540.0, 100.0, 60.0, 110.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 420.0, f2: 1310.0, f3: 1540.0 }))),
        "AY" => Some(Phoneme::new(1.0, 660.0, 1200.0, 2550.0, 100.0, 70.0, 200.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 400.0, f2: 1880.0, f3: 2500.0 }))),
        "AW" => Some(Phoneme::new(1.0, 640.0, 1230.0, 2550.0, 80.0, 70.0, 140.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 420.0, f2: 940.0, f3: 2350.0 }))),
        "EY" => Some(Phoneme::new(1.0, 480.0, 1720.0, 2520.0, 70.0, 100.0, 200.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 330.0, f2: 2020.0, f3: 2600.0 }))),
        "OW" => Some(Phoneme::new(1.0, 540.0, 1100.0, 2300.0, 80.0, 70.0, 70.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 450.0, f2: 900.0, f3: 2300.0 }))),
        "OY" => Some(Phoneme::new(1.0, 550.0, 960.0, 2400.0, 80.0, 50.0, 130.0, 1.0, 0.9, 0.7, false, Some(FormantTargets { f1: 360.0, f2: 1820.0, f3: 2450.0 }))),

        // Semivowels & Liquids
        "W" => Some(Phoneme::new(1.0, 290.0, 610.0, 2150.0, 50.0, 80.0, 60.0, 0.8, 0.7, 0.5, false, None)),
        "Y" => Some(Phoneme::new(1.0, 260.0, 2070.0, 3020.0, 40.0, 250.0, 500.0, 0.8, 0.7, 0.5, false, None)),
        "R" => Some(Phoneme::new(1.0, 310.0, 1060.0, 1380.0, 70.0, 100.0, 120.0, 0.8, 0.7, 0.5, false, None)),
        "L" => Some(Phoneme::new(1.0, 310.0, 1050.0, 2880.0, 50.0, 100.0, 280.0, 0.8, 0.7, 0.5, false, None)),

        // Nasals
        "M" => Some(Phoneme::new(1.0, 270.0, 1270.0, 2130.0, 40.0, 200.0, 200.0, 0.7, 0.18, 0.1, false, None)),
        "N" => Some(Phoneme::new(1.0, 270.0, 1340.0, 2470.0, 40.0, 300.0, 300.0, 0.7, 0.2, 0.12, false, None)),
        "NG" => Some(Phoneme::new(1.0, 270.0, 2000.0, 2700.0, 40.0, 300.0, 300.0, 0.7, 0.2, 0.12, false, None)),

        // Fricatives
        "F" => Some(Phoneme::new(0.0, 340.0, 1100.0, 2080.0, 200.0, 200.0, 1000.0, 0.0, 0.1, 0.15, false, None)),
        "TH" => Some(Phoneme::new(0.0, 320.0, 1290.0, 2540.0, 200.0, 200.0, 1000.0, 0.0, 0.08, 0.18, false, None)),
        "S" => Some(Phoneme::new(0.0, 320.0, 1390.0, 5500.0, 200.0, 200.0, 1000.0, 0.0, 0.0, 0.95, false, None)),
        "SH" => Some(Phoneme::new(0.0, 300.0, 1840.0, 2750.0, 200.0, 200.0, 1000.0, 0.0, 0.55, 0.65, false, None)),
        "V" => Some(Phoneme::new(0.45, 220.0, 1100.0, 2080.0, 80.0, 100.0, 800.0, 0.4, 0.12, 0.18, false, None)),
        "DH" => Some(Phoneme::new(0.45, 270.0, 1290.0, 2540.0, 80.0, 100.0, 800.0, 0.4, 0.1, 0.2, false, None)),
        "Z" => Some(Phoneme::new(0.45, 240.0, 1390.0, 5500.0, 80.0, 100.0, 800.0, 0.4, 0.0, 0.65, false, None)),
        "ZH" => Some(Phoneme::new(0.45, 270.0, 1840.0, 2750.0, 80.0, 100.0, 800.0, 0.4, 0.45, 0.55, false, None)),
        "HH" => Some(Phoneme::new(0.0, 500.0, 1500.0, 2500.0, 300.0, 200.0, 300.0, 0.4, 0.4, 0.3, false, None)),

        // Stops
        "P" => Some(Phoneme::new(0.0, 400.0, 1100.0, 2150.0, 300.0, 150.0, 220.0, 0.1, 0.2, 0.25, true, None)),
        "B" => Some(Phoneme::new(0.6, 200.0, 1100.0, 2150.0, 60.0, 110.0, 130.0, 0.5, 0.2, 0.2, true, None)),
        "T" => Some(Phoneme::new(0.0, 400.0, 1600.0, 2600.0, 300.0, 120.0, 250.0, 0.0, 0.3, 0.55, true, None)),
        "D" => Some(Phoneme::new(0.6, 200.0, 1600.0, 2600.0, 60.0, 100.0, 170.0, 0.5, 0.4, 0.5, true, None)),
        "K" => Some(Phoneme::new(0.0, 300.0, 1990.0, 2850.0, 250.0, 160.0, 330.0, 0.0, 0.5, 0.4, true, None)),
        "G" => Some(Phoneme::new(0.6, 200.0, 1990.0, 2850.0, 60.0, 150.0, 280.0, 0.5, 0.5, 0.4, true, None)),
        "CH" => Some(Phoneme::new(0.0, 350.0, 1800.0, 2820.0, 200.0, 90.0, 300.0, 0.0, 0.4, 0.55, true, None)),
        "JH" => Some(Phoneme::new(0.5, 260.0, 1800.0, 2820.0, 60.0, 80.0, 270.0, 0.4, 0.4, 0.5, true, None)),

        "_" => Some(Phoneme::new(0.0, 500.0, 1500.0, 2500.0, 80.0, 120.0, 160.0, 0.0, 0.0, 0.0, false, None)),
        _ => None,
    }
}

fn is_directive_token(s: &str) -> bool {
    if s.starts_with('[') && s.ends_with(']') {
        return true;
    }
    if matches!(s, "b" | "r" | "s" | "v" | "w" | "m" | "n" | "h" | "t" | "g" | "(" | ")" | "," | ";" | ".") {
        return true;
    }
    if s.starts_with('b') && s.len() > 1 {
        let rest = &s[1..];
        if rest.starts_with('+') || rest.starts_with('-') || rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
        if note_name_to_hz(rest).is_some() {
            return true;
        }
    }
    if (s.starts_with('r') || s.starts_with('p') || s.starts_with('s') || s.starts_with('v') || s.starts_with('w') || s.starts_with('m') || s.starts_with('n') || s.starts_with('h') || s.starts_with('t') || s.starts_with('g')) && s.len() > 1 {
        let rest = &s[1..];
        if rest.starts_with('+') || rest.starts_with('-') || rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
    }
    false
}

// English text-to-ARPABET fallback converter
pub fn text_to_arpabet(input: &str) -> String {
    let mut out = String::new();
    for word in input.split_whitespace() {
        if is_directive_token(word) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
            continue;
        }

        // Strip punctuation
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'')
            .collect();
        if clean.is_empty() {
            continue;
        }

        if is_directive_token(&clean) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
            continue;
        }

        let upper: String = clean.to_uppercase();
        let arpa = word_to_arpabet(&upper);
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&arpa);
    }
    out
}

fn word_to_arpabet(word: &str) -> String {
    match word {
        "HELLO" => "( HH AH L OW )".to_string(),
        "WORLD" => "( W ER L D )".to_string(),
        "HARDER" => "( HH AA R D ER )".to_string(),
        "BETTER" => "( B EH T ER )".to_string(),
        "FASTER" => "( F AE S T ER )".to_string(),
        "STRONGER" => "( S T R AO NG G ER )".to_string(),
        "MOAN" => "( M OW N )".to_string(),
        "DAW" => "( D AO )".to_string(),
        "STUDIO" => "( S T UW D IY OW )".to_string(),
        "ARIEL" => "( EH R IY AH L )".to_string(),
        "PRISM" => "( P R IH Z AH M )".to_string(),
        "MUSIC" => "( M Y UW Z IH K )".to_string(),
        "SONG" => "( S AO NG )".to_string(),
        "SING" => "( S IH NG )".to_string(),
        "VOICE" => "( V OY S )".to_string(),
        "BEAT" => "( B IY T )".to_string(),
        "BASS" => "( B EY S )".to_string(),
        "DRUM" => "( D R AH M )".to_string(),
        "SYNTH" => "( S IH N TH )".to_string(),
        "LOVE" => "( L AH V )".to_string(),
        "NIGHT" => "( N AY T )".to_string(),
        "DAY" => "( D EY )".to_string(),
        "LIFE" => "( L AY F )".to_string(),
        "TIME" => "( T AY M )".to_string(),
        "WAY" => "( W EY )".to_string(),
        "YOU" => "( Y UW )".to_string(),
        "ME" => "( M IY )".to_string(),
        "MY" => "( M AY )".to_string(),
        "WE" => "( W IY )".to_string(),
        "ARE" => "( AA R )".to_string(),
        "IS" => "( IH Z )".to_string(),
        "THE" => "( DH AH )".to_string(),
        "A" => "( AH )".to_string(),
        "TO" => "( T UW )".to_string(),
        "IN" => "( IH N )".to_string(),
        "OF" => "( AH V )".to_string(),
        "ON" => "( AA N )".to_string(),
        "FOR" => "( F AO R )".to_string(),
        "IT" => "( IH T )".to_string(),
        "AND" => "( AE N D )".to_string(),
        "BUT" => "( B AH T )".to_string(),
        "CAN" => "( K AE N )".to_string(),
        "MAKE" => "( M EY K )".to_string(),
        "DO" => "( D UW )".to_string(),
        "GO" => "( G OW )".to_string(),
        "ONE" => "( W AH N )".to_string(),
        "TWO" => "( T UW )".to_string(),
        "THREE" => "( TH R IY )".to_string(),
        "FOUR" => "( F AO R )".to_string(),
        _ => fallback_letter_to_arpabet(word),
    }
}

fn fallback_letter_to_arpabet(word: &str) -> String {
    // If it's already an ARPABET phoneme (e.g. AA, EH, etc.), return it
    if lookup_arpabet(word).is_some() {
        return word.to_string();
    }
    let chars: Vec<char> = word.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = if i + 1 < chars.len() { Some(chars[i + 1]) } else { None };

        match (c, next) {
            ('T', Some('H')) => { tokens.push("TH"); i += 2; }
            ('S', Some('H')) => { tokens.push("SH"); i += 2; }
            ('C', Some('H')) => { tokens.push("CH"); i += 2; }
            ('N', Some('G')) => { tokens.push("NG"); i += 2; }
            ('E', Some('E')) => { tokens.push("IY"); i += 2; }
            ('O', Some('O')) => { tokens.push("UW"); i += 2; }
            ('A', Some('Y')) | ('A', Some('I')) => { tokens.push("EY"); i += 2; }
            ('E', Some('Y')) => { tokens.push("EY"); i += 2; }
            ('O', Some('Y')) | ('O', Some('I')) => { tokens.push("OY"); i += 2; }
            ('O', Some('W')) | ('O', Some('U')) => { tokens.push("AW"); i += 2; }
            ('A', Some('R')) => { tokens.push("AA"); tokens.push("R"); i += 2; }
            ('E', Some('R')) | ('I', Some('R')) | ('U', Some('R')) => { tokens.push("ER"); i += 2; }
            ('O', Some('R')) => { tokens.push("AO"); tokens.push("R"); i += 2; }
            ('Q', Some('U')) => { tokens.push("K"); tokens.push("W"); i += 2; }
            ('B', _) => { tokens.push("B"); i += 1; }
            ('C', _) => { tokens.push("K"); i += 1; }
            ('D', _) => { tokens.push("D"); i += 1; }
            ('F', _) => { tokens.push("F"); i += 1; }
            ('G', _) => { tokens.push("G"); i += 1; }
            ('H', _) => { tokens.push("HH"); i += 1; }
            ('J', _) => { tokens.push("JH"); i += 1; }
            ('K', _) => { tokens.push("K"); i += 1; }
            ('L', _) => { tokens.push("L"); i += 1; }
            ('M', _) => { tokens.push("M"); i += 1; }
            ('N', _) => { tokens.push("N"); i += 1; }
            ('P', _) => { tokens.push("P"); i += 1; }
            ('R', _) => { tokens.push("R"); i += 1; }
            ('S', _) => { tokens.push("S"); i += 1; }
            ('T', _) => { tokens.push("T"); i += 1; }
            ('V', _) => { tokens.push("V"); i += 1; }
            ('W', _) => { tokens.push("W"); i += 1; }
            ('X', _) => { tokens.push("K"); tokens.push("S"); i += 1; }
            ('Y', _) => { tokens.push("Y"); i += 1; }
            ('Z', _) => { tokens.push("Z"); i += 1; }
            ('A', _) => { tokens.push("AE"); i += 1; }
            ('E', _) => { tokens.push("EH"); i += 1; }
            ('I', _) => { tokens.push("IH"); i += 1; }
            ('O', _) => { tokens.push("AA"); i += 1; }
            ('U', _) => { tokens.push("AH"); i += 1; }
            _ => { i += 1; }
        }
    }
    if tokens.is_empty() {
        return "( AH )".to_string();
    }
    let mut s = String::from("( ");
    for (idx, t) in tokens.iter().enumerate() {
        if idx > 0 {
            s.push(' ');
        }
        s.push_str(t);
    }
    s.push_str(" )");
    s
}

// Note name to Hz (e.g. "C4" -> 261.63, "A4" -> 440.0)
pub fn note_name_to_hz(name: &str) -> Option<f32> {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let semitone_base = match bytes[0] {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut idx = 1;
    let mut accidental = 0i32;
    if idx < bytes.len() && bytes[idx] == b'#' {
        accidental = 1;
        idx += 1;
    } else if idx < bytes.len() && bytes[idx] == b'b' {
        accidental = -1;
        idx += 1;
    }
    if idx >= bytes.len() {
        return None;
    }
    let octave_str = core::str::from_utf8(&bytes[idx..]).ok()?;
    let octave: i32 = octave_str.parse().ok()?;
    let midi_key = (octave + 1) * 12 + semitone_base + accidental;
    Some(440.0 * exp2_f32((midi_key as f32 - 69.0) / 12.0))
}

// Utterance Directive Compiler

#[derive(Clone, Debug)]
pub struct ScheduleEvent {
    pub at_sample: usize,
    pub f0: f32,
    pub voicing: f32,
    pub f1: f32,
    pub bw1: f32,
    pub a1: f32,
    pub f2: f32,
    pub bw2: f32,
    pub a2: f32,
    pub f3: f32,
    pub bw3: f32,
    pub a3: f32,
    pub gain: f32,
    pub vibrato_depth: f32,
    pub vibrato_rate: f32,
    pub tremolo_depth: f32,
    pub tremolo_rate: f32,
    pub aspiration: f32,
    pub tilt: f32,
    pub effort: f32,
    pub transition_samples: usize,
}

#[derive(Clone, Debug)]
pub struct VoiceSchedule {
    pub events: Vec<ScheduleEvent>,
    pub total_samples: usize,
    pub spans: Vec<PhraseSpan>,
}

#[derive(Clone, Debug)]
pub struct VocalUtterance {
    pub voices: Vec<VoiceSchedule>,
    pub total_samples: usize,
    pub spans: Vec<PhraseSpan>,
}

impl VocalUtterance {
    pub fn render(&self) -> Result<Arc<Vec<Frame>>, Error> {
        if self.total_samples == 0 {
            let mut empty = Vec::new();
            empty.try_reserve_exact(0).map_err(|_| Error::Memory)?;
            return Ok(Arc::new(empty));
        }

        let mut mix = Vec::new();
        mix.try_reserve_exact(self.total_samples)
            .map_err(|_| Error::Memory)?;
        mix.resize(self.total_samples, 0.0f32);

        for voice in &self.voices {
            render_voice_schedule(voice, &mut mix)?;
        }

        let mut pcm = Vec::new();
        pcm.try_reserve_exact(self.total_samples)
            .map_err(|_| Error::Memory)?;
        for s in mix {
            let clipped = soft_clip(s);
            let sample_i16 = (clipped * 32767.0).clamp(-32768.0, 32767.0) as i16;
            pcm.push([sample_i16, sample_i16]);
        }

        Ok(Arc::new(pcm))
    }
}

fn render_voice_schedule(voice: &VoiceSchedule, mix: &mut [f32]) -> Result<(), Error> {
    let mut bp1 = BandpassBiquad::new();
    let mut bp2 = BandpassBiquad::new();
    let mut bp3 = BandpassBiquad::new();

    let mut glottal_phase = 0.0f32;
    let mut vibrato_phase = 0.0f32;
    let mut tremolo_phase = 0.0f32;
    let mut lfsr: u32 = 0x51A7_BA55;
    let mut tilt_prev = 0.0f32;

    let mut cur_f0 = 120.0f32;
    let mut cur_voicing = 0.0f32;
    let mut cur_f1 = 500.0f32;
    let mut cur_bw1 = 80.0f32;
    let mut cur_a1 = 0.0f32;
    let mut cur_f2 = 1500.0f32;
    let mut cur_bw2 = 120.0f32;
    let mut cur_a2 = 0.0f32;
    let mut cur_f3 = 2500.0f32;
    let mut cur_bw3 = 160.0f32;
    let mut cur_a3 = 0.0f32;
    let mut cur_gain = 1.0f32;
    let mut cur_vibrato_depth = 0.0f32;
    let mut cur_vibrato_rate = 5.0f32;
    let mut cur_tremolo_depth = 0.0f32;
    let mut cur_tremolo_rate = 5.0f32;
    let mut cur_aspiration = 0.0f32;
    let mut cur_tilt = 0.0f32;
    let mut cur_effort = 0.5f32;

    let mut tgt_f0 = cur_f0;
    let mut tgt_voicing = cur_voicing;
    let mut tgt_f1 = cur_f1;
    let mut tgt_bw1 = cur_bw1;
    let mut tgt_a1 = cur_a1;
    let mut tgt_f2 = cur_f2;
    let mut tgt_bw2 = cur_bw2;
    let mut tgt_a2 = cur_a2;
    let mut tgt_f3 = cur_f3;
    let mut tgt_bw3 = cur_bw3;
    let mut tgt_a3 = cur_a3;
    let mut tgt_gain = cur_gain;
    let mut tgt_vibrato_depth = cur_vibrato_depth;
    let mut tgt_vibrato_rate = cur_vibrato_rate;
    let mut tgt_tremolo_depth = cur_tremolo_depth;
    let mut tgt_tremolo_rate = cur_tremolo_rate;
    let mut tgt_aspiration = cur_aspiration;
    let mut tgt_tilt = cur_tilt;
    let mut tgt_effort = cur_effort;

    let mut inc_f0 = 0.0f32;
    let mut inc_voicing = 0.0f32;
    let mut inc_f1 = 0.0f32;
    let mut inc_bw1 = 0.0f32;
    let mut inc_a1 = 0.0f32;
    let mut inc_f2 = 0.0f32;
    let mut inc_bw2 = 0.0f32;
    let mut inc_a2 = 0.0f32;
    let mut inc_f3 = 0.0f32;
    let mut inc_bw3 = 0.0f32;
    let mut inc_a3 = 0.0f32;
    let mut inc_gain = 0.0f32;
    let mut inc_vibrato_depth = 0.0f32;
    let mut inc_vibrato_rate = 0.0f32;
    let mut inc_tremolo_depth = 0.0f32;
    let mut inc_tremolo_rate = 0.0f32;
    let mut inc_aspiration = 0.0f32;
    let mut inc_tilt = 0.0f32;
    let mut inc_effort = 0.0f32;

    let mut transition_samples = 0usize;
    let mut evt_idx = 0usize;
    let total = mix.len().min(voice.total_samples);
    let sr = RATE as f32;

    for i in 0..total {
        while evt_idx < voice.events.len() && voice.events[evt_idx].at_sample <= i {
            let e = &voice.events[evt_idx];
            evt_idx += 1;
            let n = e.transition_samples.max(1);
            transition_samples = n;
            let fn_n = n as f32;

            tgt_f0 = e.f0;
            tgt_voicing = e.voicing;
            tgt_f1 = e.f1;
            tgt_bw1 = e.bw1;
            tgt_a1 = e.a1;
            tgt_f2 = e.f2;
            tgt_bw2 = e.bw2;
            tgt_a2 = e.a2;
            tgt_f3 = e.f3;
            tgt_bw3 = e.bw3;
            tgt_a3 = e.a3;
            tgt_gain = e.gain;
            tgt_vibrato_depth = e.vibrato_depth;
            tgt_vibrato_rate = e.vibrato_rate;
            tgt_tremolo_depth = e.tremolo_depth;
            tgt_tremolo_rate = e.tremolo_rate;
            tgt_aspiration = e.aspiration;
            tgt_tilt = e.tilt;
            tgt_effort = e.effort;

            inc_f0 = (tgt_f0 - cur_f0) / fn_n;
            inc_voicing = (tgt_voicing - cur_voicing) / fn_n;
            inc_f1 = (tgt_f1 - cur_f1) / fn_n;
            inc_bw1 = (tgt_bw1 - cur_bw1) / fn_n;
            inc_a1 = (tgt_a1 - cur_a1) / fn_n;
            inc_f2 = (tgt_f2 - cur_f2) / fn_n;
            inc_bw2 = (tgt_bw2 - cur_bw2) / fn_n;
            inc_a2 = (tgt_a2 - cur_a2) / fn_n;
            inc_f3 = (tgt_f3 - cur_f3) / fn_n;
            inc_bw3 = (tgt_bw3 - cur_bw3) / fn_n;
            inc_a3 = (tgt_a3 - cur_a3) / fn_n;
            inc_gain = (tgt_gain - cur_gain) / fn_n;
            inc_vibrato_depth = (tgt_vibrato_depth - cur_vibrato_depth) / fn_n;
            inc_vibrato_rate = (tgt_vibrato_rate - cur_vibrato_rate) / fn_n;
            inc_tremolo_depth = (tgt_tremolo_depth - cur_tremolo_depth) / fn_n;
            inc_tremolo_rate = (tgt_tremolo_rate - cur_tremolo_rate) / fn_n;
            inc_aspiration = (tgt_aspiration - cur_aspiration) / fn_n;
            inc_tilt = (tgt_tilt - cur_tilt) / fn_n;
            inc_effort = (tgt_effort - cur_effort) / fn_n;
        }

        if transition_samples > 0 {
            cur_f0 += inc_f0;
            cur_voicing += inc_voicing;
            cur_f1 += inc_f1;
            cur_bw1 += inc_bw1;
            cur_a1 += inc_a1;
            cur_f2 += inc_f2;
            cur_bw2 += inc_bw2;
            cur_a2 += inc_a2;
            cur_f3 += inc_f3;
            cur_bw3 += inc_bw3;
            cur_a3 += inc_a3;
            cur_gain += inc_gain;
            cur_vibrato_depth += inc_vibrato_depth;
            cur_vibrato_rate += inc_vibrato_rate;
            cur_tremolo_depth += inc_tremolo_depth;
            cur_tremolo_rate += inc_tremolo_rate;
            cur_aspiration += inc_aspiration;
            cur_tilt += inc_tilt;
            cur_effort += inc_effort;

            transition_samples -= 1;
            if transition_samples == 0 {
                cur_f0 = tgt_f0;
                cur_voicing = tgt_voicing;
                cur_f1 = tgt_f1;
                cur_bw1 = tgt_bw1;
                cur_a1 = tgt_a1;
                cur_f2 = tgt_f2;
                cur_bw2 = tgt_bw2;
                cur_a2 = tgt_a2;
                cur_f3 = tgt_f3;
                cur_bw3 = tgt_bw3;
                cur_a3 = tgt_a3;
                cur_gain = tgt_gain;
                cur_vibrato_depth = tgt_vibrato_depth;
                cur_vibrato_rate = tgt_vibrato_rate;
                cur_tremolo_depth = tgt_tremolo_depth;
                cur_tremolo_rate = tgt_tremolo_rate;
                cur_aspiration = tgt_aspiration;
                cur_tilt = tgt_tilt;
                cur_effort = tgt_effort;
            }
        }

        // Vibrato LFO
        vibrato_phase += 2.0 * PI * cur_vibrato_rate / sr;
        if vibrato_phase > 2.0 * PI {
            vibrato_phase -= 2.0 * PI * floor_f32(vibrato_phase / (2.0 * PI));
        }
        let eff_f0 = (cur_f0 + cur_vibrato_depth * sin_f32(vibrato_phase)).max(20.0);

        // Tremolo LFO
        tremolo_phase += 2.0 * PI * cur_tremolo_rate / sr;
        if tremolo_phase > 2.0 * PI {
            tremolo_phase -= 2.0 * PI * floor_f32(tremolo_phase / (2.0 * PI));
        }
        let tremolo_mod = 1.0 - cur_tremolo_depth * (0.5 + 0.5 * sin_f32(tremolo_phase));

        let v = cur_voicing.clamp(0.0, 1.0);
        let noise_sample = xorshift(&mut lfsr);
        let pulse_val = glottal_pulse(glottal_phase, cur_effort);
        let voiced_gain = 1.0 - cur_aspiration * 0.85;
        let exc = v * pulse_val * voiced_gain
            + (1.0 - v) * noise_sample * 0.35
            + cur_aspiration * noise_sample * 0.5;

        glottal_phase += eff_f0 / sr;
        if glottal_phase >= 1.0 {
            glottal_phase -= floor_f32(glottal_phase);
        }

        bp1.set_freq(cur_f1, cur_bw1, sr);
        bp2.set_freq(cur_f2, cur_bw2, sr);
        bp3.set_freq(cur_f3, cur_bw3, sr);

        let y = (bp1.process(exc) * cur_a1
            + bp2.process(exc) * cur_a2
            + bp3.process(exc) * cur_a3)
            * cur_gain
            * tremolo_mod;

        let tilted = y - cur_tilt * tilt_prev;
        tilt_prev = y;

        mix[i] += tilted;
    }

    Ok(())
}

// Tokenizer & Directive Compiler

#[derive(Clone, Debug, PartialEq)]
enum TokenKind {
    Phoneme {
        code: String,
        stressed: bool,
        pitch_delta: f32,
        transient: bool,
    },
    Directive {
        key: String,
        value: f32,
        relative: bool,
        reset: bool,
    },
    BankSwitch(String),
    SyllableOpen,
    SyllableClose,
    Pause(f32),
    VoiceSwitch(usize),
}

#[derive(Clone, Debug)]
struct ParsedToken {
    kind: TokenKind,
    byte_start: usize,
    byte_end: usize,
}

pub fn compile_utterance(input: &str, defaults: &VoiceParams) -> Result<VocalUtterance, Error> {
    let bank = BuiltinBank { is_japanese: false };
    compile_with_bank(input, defaults, &bank)
}

pub fn compile_with_bank<B: PhonemeBank>(
    input: &str,
    defaults: &VoiceParams,
    bank: &B,
) -> Result<VocalUtterance, Error> {
    // If input contains plain English words (letters without directives), convert to ARPABET
    let expanded = if input.contains('[') || input.chars().any(|c| c.is_ascii_uppercase()) {
        input.to_string()
    } else {
        text_to_arpabet(input)
    };

    let tokens = tokenize(&expanded)?;
    let mut voices_tokens: Vec<Vec<ParsedToken>> = vec![Vec::new()];
    for tok in tokens {
        if let TokenKind::VoiceSwitch(v) = tok.kind {
            while voices_tokens.len() <= v {
                voices_tokens.push(Vec::new());
            }
        } else {
            let last_idx = voices_tokens.len() - 1;
            voices_tokens[last_idx].push(tok);
        }
    }

    let mut voices = Vec::new();
    let mut total_samples = 0;
    let mut all_spans = Vec::new();

    for (v_idx, toks) in voices_tokens.into_iter().enumerate() {
        let sched = compile_tokens_to_schedule(&toks, defaults, bank, v_idx == 0)?;
        if sched.total_samples > total_samples {
            total_samples = sched.total_samples;
        }
        if v_idx == 0 {
            all_spans = sched.spans.clone();
        }
        voices.push(sched);
    }

    Ok(VocalUtterance {
        voices,
        total_samples,
        spans: all_spans,
    })
}

fn tokenize(input: &str) -> Result<Vec<ParsedToken>, Error> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment '#'
        if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment '/*'
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }

        let start = i;
        while i < len && !bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                break;
            }
            i += 1;
        }
        let end = i;
        let part = match core::str::from_utf8(&bytes[start..end]) {
            Ok(s) => s,
            Err(_) => return Err(Error::Parse),
        };

        if let Some(kind) = classify_token(part) {
            tokens.push(ParsedToken {
                kind,
                byte_start: start,
                byte_end: end,
            });
        }
    }

    Ok(tokens)
}

fn classify_token(part: &str) -> Option<TokenKind> {
    if part == "(" {
        return Some(TokenKind::SyllableOpen);
    }
    if part == ")" {
        return Some(TokenKind::SyllableClose);
    }
    if part == "," {
        return Some(TokenKind::Pause(100.0));
    }
    if part == ";" {
        return Some(TokenKind::Pause(200.0));
    }
    if part == "." {
        return Some(TokenKind::Pause(300.0));
    }

    if part.starts_with("[voice=") && part.ends_with(']') {
        let inner = &part[7..part.len() - 1];
        if let Ok(v) = inner.parse::<usize>() {
            return Some(TokenKind::VoiceSwitch(v));
        }
    }

    if part.starts_with("[bank=") && part.ends_with(']') {
        let bank_name = &part[6..part.len() - 1];
        return Some(TokenKind::BankSwitch(bank_name.to_string()));
    }

    // Directives: e.g. [f0=140], [rate=120]
    if part.starts_with('[') && part.ends_with(']') && part.contains('=') {
        let inner = &part[1..part.len() - 1];
        if let Some((k, v)) = inner.split_once('=') {
            if let Ok(num) = v.parse::<f32>() {
                return Some(TokenKind::Directive {
                    key: k.to_string(),
                    value: num,
                    relative: false,
                    reset: false,
                });
            }
        }
    }

    // Note pitch directive: bC#4, b=C4, etc.
    if part.starts_with('b') && part.len() >= 2 {
        let after_b = if part.as_bytes()[1] == b'=' { &part[2..] } else { &part[1..] };
        if let Some(hz) = note_name_to_hz(after_b) {
            return Some(TokenKind::Directive {
                key: "base".to_string(),
                value: hz,
                relative: false,
                reset: false,
            });
        }
    }

    // Compact single-letter directives: b140, b+10, b-10, b, r120, s1.17, v5, w6, m0.2, n5, h0.2, t0.3, g0.7, p150
    let b = part.as_bytes()[0];
    if b >= b'a' && b <= b'z' {
        let key_str = match b {
            b'b' => "base",
            b'r' => "rate",
            b's' => "scale",
            b'v' => "vibrato",
            b'w' => "vibratoRate",
            b'm' => "tremolo",
            b'n' => "tremoloRate",
            b'h' => "aspiration",
            b't' => "tilt",
            b'g' => "effort",
            b'p' => "pause",
            _ => "",
        };
        if !key_str.is_empty() {
            if part.len() == 1 {
                if key_str == "pause" {
                    return None;
                }
                return Some(TokenKind::Directive {
                    key: key_str.to_string(),
                    value: 0.0,
                    relative: false,
                    reset: true,
                });
            }
            let rest = &part[1..];
            let (is_rel, val_str) = if rest.starts_with('+') {
                (true, &rest[1..])
            } else if rest.starts_with('-') {
                (true, rest)
            } else {
                (false, rest)
            };
            if let Ok(val) = val_str.parse::<f32>() {
                if key_str == "pause" {
                    return Some(TokenKind::Pause(val));
                }
                return Some(TokenKind::Directive {
                    key: key_str.to_string(),
                    value: val,
                    relative: is_rel,
                    reset: false,
                });
            }
        }
    }

    // Phoneme with optional stress '!' or pitch delta: AY+15, AY-10, AE(+40), AA!
    let mut chars_peek = part.chars().peekable();
    let mut code = String::new();
    while let Some(&c) = chars_peek.peek() {
        if c.is_ascii_uppercase() || c == '_' {
            code.push(c);
            chars_peek.next();
        } else {
            break;
        }
    }

    if !code.is_empty() {
        let mut stressed = false;
        let mut pitch_delta = 0.0f32;
        let mut transient = false;

        let rem: String = chars_peek.collect();
        if rem.starts_with('!') || rem.starts_with('\'') {
            stressed = true;
        }

        // Transient ornament: ( +40 )
        if let Some(open) = rem.find('(') {
            if let Some(close) = rem.find(')') {
                let inner = &rem[open + 1..close];
                if let Ok(d) = inner.trim_start_matches('+').parse::<f32>() {
                    pitch_delta = d;
                    transient = true;
                }
            }
        } else {
            // Sticky delta: +15, -10
            let delta_part = rem.trim_start_matches(|c| c == '!' || c == '\'');
            if delta_part.starts_with('+') || delta_part.starts_with('-') {
                if let Ok(d) = delta_part.parse::<f32>() {
                    pitch_delta = d;
                    transient = false;
                }
            }
        }

        return Some(TokenKind::Phoneme {
            code,
            stressed,
            pitch_delta,
            transient,
        });
    }

    None
}

fn compile_tokens_to_schedule<B: PhonemeBank>(
    tokens: &[ParsedToken],
    defaults: &VoiceParams,
    bank: &B,
    record_spans: bool,
) -> Result<VoiceSchedule, Error> {
    let mut state = CompilerState {
        events: Vec::new(),
        spans: Vec::new(),
        time_ms: 0.0,
        f0: defaults.f0,
        rate_ms: defaults.rate_ms,
        formant_scale: defaults.formant_scale.clamp(0.70, 1.50),
        vibrato_depth: defaults.vibrato_depth,
        vibrato_rate: defaults.vibrato_rate,
        tremolo_depth: defaults.tremolo_depth,
        tremolo_rate: defaults.tremolo_rate,
        aspiration: defaults.aspiration,
        tilt: defaults.tilt,
        effort: defaults.effort,
        gain: defaults.gain,
        bank,
        defaults,
        record_spans,
        sr: RATE as f32,
    };
    state.events.try_reserve(tokens.len() * 3).map_err(|_| Error::Memory)?;

    let mut in_syllable = false;
    let mut syllable_queue: Vec<ParsedToken> = Vec::new();

    for tok in tokens {
        match &tok.kind {
            TokenKind::SyllableOpen => {
                in_syllable = true;
                syllable_queue.clear();
            }
            TokenKind::SyllableClose => {
                if in_syllable && !syllable_queue.is_empty() {
                    let slot = state.rate_ms / syllable_queue.len() as f32;
                    let queue_copy = syllable_queue.clone();
                    for t in &queue_copy {
                        state.render_phone(t, slot);
                    }
                    syllable_queue.clear();
                }
                in_syllable = false;
            }
            TokenKind::Phoneme { stressed, .. } => {
                if in_syllable {
                    syllable_queue.push(tok.clone());
                } else {
                    let slot = if *stressed { state.rate_ms * 1.25 } else { state.rate_ms };
                    state.render_phone(tok, slot);
                }
            }
            TokenKind::Pause(ms) => {
                let start_frame = floor_f32(state.time_ms * state.sr / 1000.0) as usize;
                state.emit_silence(state.time_ms, 30.0);
                state.time_ms += ms;
                let end_frame = floor_f32(state.time_ms * state.sr / 1000.0) as usize;
                if state.record_spans {
                    state.spans.push(PhraseSpan {
                        byte_start: tok.byte_start,
                        byte_end: tok.byte_end,
                        start_frame,
                        end_frame,
                    });
                }
            }
            TokenKind::Directive {
                key,
                value,
                relative,
                reset,
            } => {
                match key.as_str() {
                    "base" => {
                        if *reset {
                            state.f0 = state.defaults.f0;
                        } else if *relative {
                            state.f0 += value;
                        } else {
                            state.f0 = *value;
                        }
                    }
                    "rate" => {
                        if *reset {
                            state.rate_ms = state.defaults.rate_ms;
                        } else if *relative {
                            state.rate_ms += value;
                        } else {
                            state.rate_ms = *value;
                        }
                    }
                    "scale" => {
                        if *reset {
                            state.formant_scale = state.defaults.formant_scale;
                        } else if *relative {
                            state.formant_scale += value;
                        } else {
                            state.formant_scale = *value;
                        }
                        state.formant_scale = state.formant_scale.clamp(0.70, 1.50);
                    }
                    "vibrato" => {
                        if *reset {
                            state.vibrato_depth = state.defaults.vibrato_depth;
                        } else if *relative {
                            state.vibrato_depth += value;
                        } else {
                            state.vibrato_depth = *value;
                        }
                    }
                    "vibratoRate" => {
                        if *reset {
                            state.vibrato_rate = state.defaults.vibrato_rate;
                        } else if *relative {
                            state.vibrato_rate += value;
                        } else {
                            state.vibrato_rate = *value;
                        }
                    }
                    "tremolo" => {
                        if *reset {
                            state.tremolo_depth = state.defaults.tremolo_depth;
                        } else if *relative {
                            state.tremolo_depth += value;
                        } else {
                            state.tremolo_depth = *value;
                        }
                    }
                    "tremoloRate" => {
                        if *reset {
                            state.tremolo_rate = state.defaults.tremolo_rate;
                        } else if *relative {
                            state.tremolo_rate += value;
                        } else {
                            state.tremolo_rate = *value;
                        }
                    }
                    "aspiration" => {
                        if *reset {
                            state.aspiration = state.defaults.aspiration;
                        } else if *relative {
                            state.aspiration += value;
                        } else {
                            state.aspiration = *value;
                        }
                    }
                    "tilt" => {
                        if *reset {
                            state.tilt = state.defaults.tilt;
                        } else if *relative {
                            state.tilt += value;
                        } else {
                            state.tilt = *value;
                        }
                    }
                    "effort" => {
                        if *reset {
                            state.effort = state.defaults.effort;
                        } else if *relative {
                            state.effort += value;
                        } else {
                            state.effort = *value;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if in_syllable && !syllable_queue.is_empty() {
        let slot = state.rate_ms / syllable_queue.len() as f32;
        let queue_copy = syllable_queue.clone();
        for t in &queue_copy {
            state.render_phone(t, slot);
        }
    }

    // Trailing silence
    state.emit_silence(state.time_ms, 100.0);
    state.time_ms += 150.0;

    let total_samples = floor_f32(state.time_ms * state.sr / 1000.0) as usize;

    Ok(VoiceSchedule {
        events: state.events,
        total_samples,
        spans: state.spans,
    })
}

struct CompilerState<'a, B: PhonemeBank> {
    events: Vec<ScheduleEvent>,
    spans: Vec<PhraseSpan>,
    time_ms: f32,
    f0: f32,
    rate_ms: f32,
    formant_scale: f32,
    vibrato_depth: f32,
    vibrato_rate: f32,
    tremolo_depth: f32,
    tremolo_rate: f32,
    aspiration: f32,
    tilt: f32,
    effort: f32,
    gain: f32,
    bank: &'a B,
    defaults: &'a VoiceParams,
    record_spans: bool,
    sr: f32,
}

impl<'a, B: PhonemeBank> CompilerState<'a, B> {
    fn emit(&mut self, at_ms: f32, p: &Phoneme, f0_val: f32, trans_ms: f32) {
        let at_sample = floor_f32(at_ms * self.sr / 1000.0) as usize;
        let trans_samples = (floor_f32(trans_ms * self.sr / 1000.0) as usize).max(1);
        self.events.push(ScheduleEvent {
            at_sample,
            f0: f0_val,
            voicing: p.voicing,
            f1: p.f1 * self.formant_scale,
            bw1: p.bw1 * self.formant_scale,
            a1: p.a1,
            f2: p.f2 * self.formant_scale,
            bw2: p.bw2 * self.formant_scale,
            a2: p.a2,
            f3: p.f3 * self.formant_scale,
            bw3: p.bw3 * self.formant_scale,
            a3: p.a3,
            gain: self.gain,
            vibrato_depth: self.vibrato_depth,
            vibrato_rate: self.vibrato_rate,
            tremolo_depth: self.tremolo_depth,
            tremolo_rate: self.tremolo_rate,
            aspiration: self.aspiration,
            tilt: self.tilt,
            effort: self.effort,
            transition_samples: trans_samples,
        });
    }

    fn emit_silence(&mut self, at_ms: f32, trans_ms: f32) {
        let at_sample = floor_f32(at_ms * self.sr / 1000.0) as usize;
        let trans_samples = (floor_f32(trans_ms * self.sr / 1000.0) as usize).max(1);
        self.events.push(ScheduleEvent {
            at_sample,
            f0: self.f0,
            voicing: 0.0,
            f1: 500.0 * self.formant_scale,
            bw1: 80.0 * self.formant_scale,
            a1: 0.0,
            f2: 1500.0 * self.formant_scale,
            bw2: 120.0 * self.formant_scale,
            a2: 0.0,
            f3: 2500.0 * self.formant_scale,
            bw3: 160.0 * self.formant_scale,
            a3: 0.0,
            gain: self.gain,
            vibrato_depth: self.vibrato_depth,
            vibrato_rate: self.vibrato_rate,
            tremolo_depth: self.tremolo_depth,
            tremolo_rate: self.tremolo_rate,
            aspiration: self.aspiration,
            tilt: self.tilt,
            effort: self.effort,
            transition_samples: trans_samples,
        });
    }

    fn render_phone(&mut self, t: &ParsedToken, slot_ms: f32) {
        if let TokenKind::Phoneme {
            code,
            stressed,
            pitch_delta,
            transient,
        } = &t.kind
        {
            let p = match self.bank.get(code) {
                Some(ph) => ph,
                None => Phoneme::new(0.0, 500.0, 1500.0, 2500.0, 80.0, 120.0, 160.0, 0.0, 0.0, 0.0, false, None),
            };

            let start_f0 = if *stressed { self.f0 + 8.0 } else { self.f0 };
            let end_f0 = start_f0 + *pitch_delta;

            let start_frame = floor_f32(self.time_ms * self.sr / 1000.0) as usize;

            if p.is_stop {
                let burst_ms = (slot_ms * 0.3).min(25.0);
                let silence_ms = slot_ms - burst_ms;
                self.emit_silence(self.time_ms, (silence_ms * 0.4).min(20.0));
                self.time_ms += silence_ms;
                self.emit(self.time_ms, &p, start_f0, (burst_ms * 0.2).min(5.0));
                self.time_ms += burst_ms;
            } else if let Some(glide) = p.glide_to {
                let onset = slot_ms * 0.25;
                let glide_ms = slot_ms * 0.50;
                let offset = slot_ms * 0.25;
                self.emit(self.time_ms, &p, start_f0, onset.min(20.0));
                self.time_ms += onset;
                let mut glide_p = p;
                glide_p.f1 = glide.f1;
                glide_p.f2 = glide.f2;
                glide_p.f3 = glide.f3;
                self.emit(self.time_ms, &glide_p, end_f0, glide_ms);
                self.time_ms += glide_ms + offset;
            } else if *pitch_delta != 0.0 {
                self.emit(self.time_ms, &p, start_f0, (slot_ms * 0.25).min(25.0));
                self.time_ms += slot_ms * 0.25;
                self.emit(self.time_ms, &p, end_f0, slot_ms * 0.6);
                self.time_ms += slot_ms * 0.75;
            } else {
                let trans = (slot_ms * 0.4).min(35.0);
                self.emit(self.time_ms, &p, start_f0, trans);
                self.time_ms += slot_ms;
            }

            let end_frame = floor_f32(self.time_ms * self.sr / 1000.0) as usize;
            if self.record_spans {
                self.spans.push(PhraseSpan {
                    byte_start: t.byte_start,
                    byte_end: t.byte_end,
                    start_frame,
                    end_frame,
                });
            }

            if !*transient {
                self.f0 += *pitch_delta;
            }
        }
    }
}

/// Direct helper to synthesize an utterance and return PCM + PhraseSpans.
pub fn synthesize(input: &str, params: &VoiceParams) -> Result<(Arc<Vec<Frame>>, Vec<PhraseSpan>), Error> {
    let utterance = compile_utterance(input, params)?;
    let pcm = utterance.render()?;
    let spans = utterance.spans;
    Ok((pcm, spans))
}

/// MIDI Singing: maps lyrics syllables across pattern notes pitches and durations.
pub fn sing_midi_pattern(
    pattern: &midi::Pattern,
    lyrics: &str,
    params: &VoiceParams,
) -> Result<Arc<Vec<Frame>>, Error> {
    pattern.validate().map_err(|_| Error::Range)?;

    let mut master = Vec::new();
    master
        .try_reserve_exact(pattern.frames as usize)
        .map_err(|_| Error::Memory)?;
    master.resize(pattern.frames as usize, [0i16; 2]);

    if pattern.notes.is_empty() {
        return Ok(Arc::new(master));
    }

    // Convert lyrics to phonetic syllables/words
    let expanded = text_to_arpabet(lyrics);
    let tokens = tokenize(&expanded)?;
    let mut words: Vec<String> = Vec::new();

    let mut in_grp = false;
    let mut current_grp = String::new();

    for tok in &tokens {
        match &tok.kind {
            TokenKind::SyllableOpen => {
                in_grp = true;
                current_grp = String::from("( ");
            }
            TokenKind::SyllableClose => {
                in_grp = false;
                current_grp.push_str(" )");
                words.push(current_grp.clone());
                current_grp.clear();
            }
            TokenKind::Phoneme { code, .. } => {
                if in_grp {
                    current_grp.push_str(code);
                    current_grp.push(' ');
                } else {
                    words.push(code.clone());
                }
            }
            _ => {}
        }
    }

    if words.is_empty() {
        words.push("( AA )".to_string());
    }

    let mut word_idx = 0;
    for note in &pattern.notes {
        let note_hz = 440.0 * exp2_f32((note.key as f32 - 69.0) / 12.0);
        let note_ms = (note.frames as f32 * 1000.0 / RATE as f32).max(10.0);

        let syllable = &words[word_idx % words.len()];
        word_idx += 1;

        // Compile a single note utterance with exact pitch and note duration
        let mut note_params = params.clone();
        note_params.f0 = note_hz;
        note_params.rate_ms = note_ms;

        let utterance_str = alloc::format!("b{} r{} {}", note_hz as u32, note_ms as u32, syllable);
        let (note_pcm, _) = synthesize(&utterance_str, &note_params)?;

        let note_start = note.start as usize;
        let note_len = (note.frames as usize).min(note_pcm.len());
        let vel_scale = note.velocity as i32;

        for offset in 0..note_len {
            let target_idx = note_start + offset;
            if target_idx < master.len() {
                let sample = note_pcm[offset];
                for c in 0..2 {
                    let scaled = (sample[c] as i32 * vel_scale / 127) as i16;
                    master[target_idx][c] = master[target_idx][c].saturating_add(scaled);
                }
            }
        }
    }

    Ok(Arc::new(master))
}
