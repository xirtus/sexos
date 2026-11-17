// SPDX-License-Identifier: GPL-3.0-only
extern crate alloc;

// Moan Factory Device Preset Catalogs.
//
// Provides `#![no_std]`-compatible factory preset definitions and lookups for
// Moan's built-in instruments and dynamics/saturation processors:
// - `Sub808Preset` (14 presets)
// - `DrumKitPreset` (10 presets)
// - `TapePreset` (6 presets)
// - `GatePreset` (5 presets)
// - `MasteringPreset` (8 presets)

// ============================================================================
// 1. Sub808 Presets
// ============================================================================

pub const SUB808_GLIDE_OFF: u8 = 0;
pub const SUB808_GLIDE_LEGATO: u8 = 1;
pub const SUB808_GLIDE_ALWAYS: u8 = 2;

pub const SUB808_DRIVE_WARM: u8 = 0;
pub const SUB808_DRIVE_TUBE: u8 = 1;
pub const SUB808_DRIVE_CLIP: u8 = 2;
pub const SUB808_DRIVE_FOLD: u8 = 3;

/// Factory preset specification for Moan's Sub808 bass synthesizer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sub808Preset {
    pub name: &'static str,
    pub shape: f32,
    pub octave: f32,
    pub punch_st: f32,
    pub punch_ms: f32,
    pub glide_mode: u8, // 0=Off, 1=Legato, 2=Always
    pub glide_ms: f32,
    pub hold_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
    pub ring_on_release: bool,
    pub decay_track: f32,
    pub drive_curve: u8, // 0=Warm, 1=Tube, 2=Clip, 3=Fold
    pub drive: f32,
    pub tone_hz: f32,
    pub click: f32,
    pub growl: f32,
}

pub const SUB808_PRESETS: &[Sub808Preset] = &[
    Sub808Preset {
        name: "Classic Sub",
        shape: 0.12,
        octave: 0.10,
        punch_st: 12.0,
        punch_ms: 35.0,
        glide_mode: SUB808_GLIDE_OFF,
        glide_ms: 0.0,
        hold_ms: 25.0,
        decay_ms: 850.0,
        sustain: 0.0,
        release_ms: 90.0,
        ring_on_release: true,
        decay_track: 0.25,
        drive_curve: SUB808_DRIVE_WARM,
        drive: 0.18,
        tone_hz: 420.0,
        click: 0.15,
        growl: 0.05,
    },
    Sub808Preset {
        name: "Clean Sub",
        shape: 0.0,
        octave: 0.0,
        punch_st: 4.0,
        punch_ms: 18.0,
        glide_mode: SUB808_GLIDE_OFF,
        glide_ms: 0.0,
        hold_ms: 20.0,
        decay_ms: 700.0,
        sustain: 0.35,
        release_ms: 80.0,
        ring_on_release: false,
        decay_track: 0.15,
        drive_curve: SUB808_DRIVE_WARM,
        drive: 0.0,
        tone_hz: 260.0,
        click: 0.0,
        growl: 0.0,
    },
    Sub808Preset {
        name: "Long Sustain",
        shape: 0.10,
        octave: 0.12,
        punch_st: 10.0,
        punch_ms: 30.0,
        glide_mode: SUB808_GLIDE_LEGATO,
        glide_ms: 45.0,
        hold_ms: 120.0,
        decay_ms: 2400.0,
        sustain: 0.55,
        release_ms: 350.0,
        ring_on_release: true,
        decay_track: 0.30,
        drive_curve: SUB808_DRIVE_WARM,
        drive: 0.15,
        tone_hz: 380.0,
        click: 0.10,
        growl: 0.05,
    },
    Sub808Preset {
        name: "Punchy Knock",
        shape: 0.22,
        octave: 0.20,
        punch_st: 24.0,
        punch_ms: 48.0,
        glide_mode: SUB808_GLIDE_OFF,
        glide_ms: 0.0,
        hold_ms: 15.0,
        decay_ms: 480.0,
        sustain: 0.0,
        release_ms: 60.0,
        ring_on_release: true,
        decay_track: 0.20,
        drive_curve: SUB808_DRIVE_TUBE,
        drive: 0.32,
        tone_hz: 750.0,
        click: 0.65,
        growl: 0.15,
    },
    Sub808Preset {
        name: "Warm Tube",
        shape: 0.25,
        octave: 0.22,
        punch_st: 12.0,
        punch_ms: 32.0,
        glide_mode: SUB808_GLIDE_OFF,
        glide_ms: 0.0,
        hold_ms: 30.0,
        decay_ms: 950.0,
        sustain: 0.20,
        release_ms: 120.0,
        ring_on_release: true,
        decay_track: 0.25,
        drive_curve: SUB808_DRIVE_TUBE,
        drive: 0.48,
        tone_hz: 680.0,
        click: 0.18,
        growl: 0.22,
    },
    Sub808Preset {
        name: "Heavy Fuzz",
        shape: 0.65,
        octave: 0.35,
        punch_st: 14.0,
        punch_ms: 28.0,
        glide_mode: SUB808_GLIDE_LEGATO,
        glide_ms: 55.0,
        hold_ms: 40.0,
        decay_ms: 900.0,
        sustain: 0.30,
        release_ms: 110.0,
        ring_on_release: true,
        decay_track: 0.20,
        drive_curve: SUB808_DRIVE_CLIP,
        drive: 0.85,
        tone_hz: 2400.0,
        click: 0.30,
        growl: 0.60,
    },
    Sub808Preset {
        name: "Legato Slide",
        shape: 0.18,
        octave: 0.18,
        punch_st: 10.0,
        punch_ms: 25.0,
        glide_mode: SUB808_GLIDE_LEGATO,
        glide_ms: 110.0,
        hold_ms: 35.0,
        decay_ms: 1100.0,
        sustain: 0.45,
        release_ms: 140.0,
        ring_on_release: false,
        decay_track: 0.25,
        drive_curve: SUB808_DRIVE_TUBE,
        drive: 0.28,
        tone_hz: 520.0,
        click: 0.12,
        growl: 0.10,
    },
    Sub808Preset {
        name: "Portamento Glide",
        shape: 0.20,
        octave: 0.25,
        punch_st: 8.0,
        punch_ms: 22.0,
        glide_mode: SUB808_GLIDE_ALWAYS,
        glide_ms: 165.0,
        hold_ms: 40.0,
        decay_ms: 1300.0,
        sustain: 0.60,
        release_ms: 160.0,
        ring_on_release: false,
        decay_track: 0.20,
        drive_curve: SUB808_DRIVE_WARM,
        drive: 0.25,
        tone_hz: 600.0,
        click: 0.08,
        growl: 0.12,
    },
    Sub808Preset {
        name: "Deep Infrasonic",
        shape: 0.04,
        octave: 0.0,
        punch_st: 7.0,
        punch_ms: 42.0,
        glide_mode: SUB808_GLIDE_OFF,
        glide_ms: 0.0,
        hold_ms: 60.0,
        decay_ms: 1600.0,
        sustain: 0.25,
        release_ms: 200.0,
        ring_on_release: true,
        decay_track: 0.35,
        drive_curve: SUB808_DRIVE_WARM,
        drive: 0.08,
        tone_hz: 160.0,
        click: 0.04,
        growl: 0.0,
    },
    Sub808Preset {
        name: "Small Speaker Octave",
        shape: 0.30,
        octave: 0.78,
        punch_st: 15.0,
        punch_ms: 30.0,
        glide_mode: SUB808_GLIDE_OFF,
        glide_ms: 0.0,
        hold_ms: 25.0,
        decay_ms: 780.0,
        sustain: 0.15,
        release_ms: 95.0,
        ring_on_release: true,
        decay_track: 0.25,
        drive_curve: SUB808_DRIVE_TUBE,
        drive: 0.42,
        tone_hz: 1350.0,
        click: 0.25,
        growl: 0.20,
    },
    Sub808Preset {
        name: "Tape Saturated",
        shape: 0.28,
        octave: 0.24,
        punch_st: 13.0,
        punch_ms: 36.0,
        glide_mode: SUB808_GLIDE_LEGATO,
        glide_ms: 35.0,
        hold_ms: 35.0,
        decay_ms: 1000.0,
        sustain: 0.20,
        release_ms: 130.0,
        ring_on_release: true,
        decay_track: 0.25,
        drive_curve: SUB808_DRIVE_WARM,
        drive: 0.55,
        tone_hz: 820.0,
        click: 0.20,
        growl: 0.18,
    },
    Sub808Preset {
        name: "Hard Clip 808",
        shape: 0.40,
        octave: 0.30,
        punch_st: 20.0,
        punch_ms: 32.0,
        glide_mode: SUB808_GLIDE_LEGATO,
        glide_ms: 50.0,
        hold_ms: 20.0,
        decay_ms: 760.0,
        sustain: 0.0,
        release_ms: 85.0,
        ring_on_release: true,
        decay_track: 0.20,
        drive_curve: SUB808_DRIVE_CLIP,
        drive: 0.78,
        tone_hz: 1850.0,
        click: 0.45,
        growl: 0.35,
    },
    Sub808Preset {
        name: "Wavefold Growl",
        shape: 0.45,
        octave: 0.32,
        punch_st: 16.0,
        punch_ms: 28.0,
        glide_mode: SUB808_GLIDE_LEGATO,
        glide_ms: 65.0,
        hold_ms: 30.0,
        decay_ms: 920.0,
        sustain: 0.25,
        release_ms: 110.0,
        ring_on_release: true,
        decay_track: 0.22,
        drive_curve: SUB808_DRIVE_FOLD,
        drive: 0.72,
        tone_hz: 2800.0,
        click: 0.28,
        growl: 0.75,
    },
    Sub808Preset {
        name: "Formant Growl",
        shape: 0.52,
        octave: 0.40,
        punch_st: 12.0,
        punch_ms: 26.0,
        glide_mode: SUB808_GLIDE_ALWAYS,
        glide_ms: 85.0,
        hold_ms: 45.0,
        decay_ms: 1050.0,
        sustain: 0.40,
        release_ms: 140.0,
        ring_on_release: false,
        decay_track: 0.25,
        drive_curve: SUB808_DRIVE_FOLD,
        drive: 0.62,
        tone_hz: 2100.0,
        click: 0.20,
        growl: 0.92,
    },
];

// ============================================================================
// 2. DrumKit Presets
// ============================================================================

pub const DRUM_MODEL_ELECTRONIC: u8 = 0;
pub const DRUM_MODEL_CIRCUIT: u8 = 1;
pub const DRUM_MODEL_MODAL_ACOUSTIC: u8 = 2;

/// Factory preset specification for Moan's synthesized and modal DrumKit device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumKitPreset {
    pub name: &'static str,
    pub model: u8, // 0=Electronic, 1=Circuit, 2=ModalAcoustic
    pub pitch_semitones: f32,
    pub decay_scale: f32,
    pub tone_tilt: f32,
    pub drive: f32,
    pub room_mix: f32,
}

pub const DRUMKIT_PRESETS: &[DrumKitPreset] = &[
    DrumKitPreset {
        name: "Electronic Core",
        model: DRUM_MODEL_ELECTRONIC,
        pitch_semitones: 0.0,
        decay_scale: 1.0,
        tone_tilt: 0.05,
        drive: 0.15,
        room_mix: 0.12,
    },
    DrumKitPreset {
        name: "Circuit 808",
        model: DRUM_MODEL_CIRCUIT,
        pitch_semitones: -1.0,
        decay_scale: 1.35,
        tone_tilt: -0.25,
        drive: 0.22,
        room_mix: 0.05,
    },
    DrumKitPreset {
        name: "Circuit 909",
        model: DRUM_MODEL_CIRCUIT,
        pitch_semitones: 0.5,
        decay_scale: 0.95,
        tone_tilt: 0.22,
        drive: 0.35,
        room_mix: 0.10,
    },
    DrumKitPreset {
        name: "Acoustic Studio",
        model: DRUM_MODEL_MODAL_ACOUSTIC,
        pitch_semitones: 0.0,
        decay_scale: 1.10,
        tone_tilt: 0.08,
        drive: 0.08,
        room_mix: 0.32,
    },
    DrumKitPreset {
        name: "Heavy Metal Kit",
        model: DRUM_MODEL_MODAL_ACOUSTIC,
        pitch_semitones: 1.0,
        decay_scale: 0.85,
        tone_tilt: 0.45,
        drive: 0.52,
        room_mix: 0.25,
    },
    DrumKitPreset {
        name: "Dusty Vinyl Kit",
        model: DRUM_MODEL_MODAL_ACOUSTIC,
        pitch_semitones: -2.0,
        decay_scale: 0.78,
        tone_tilt: -0.45,
        drive: 0.42,
        room_mix: 0.20,
    },
    DrumKitPreset {
        name: "Gated 80s Kit",
        model: DRUM_MODEL_ELECTRONIC,
        pitch_semitones: -0.5,
        decay_scale: 0.65,
        tone_tilt: 0.18,
        drive: 0.30,
        room_mix: 0.58,
    },
    DrumKitPreset {
        name: "Tight Trap Kit",
        model: DRUM_MODEL_CIRCUIT,
        pitch_semitones: 0.0,
        decay_scale: 0.72,
        tone_tilt: 0.30,
        drive: 0.45,
        room_mix: 0.08,
    },
    DrumKitPreset {
        name: "Crisp Brush Kit",
        model: DRUM_MODEL_MODAL_ACOUSTIC,
        pitch_semitones: 1.5,
        decay_scale: 0.90,
        tone_tilt: 0.38,
        drive: 0.04,
        room_mix: 0.28,
    },
    DrumKitPreset {
        name: "Industrial Crush Kit",
        model: DRUM_MODEL_ELECTRONIC,
        pitch_semitones: -3.0,
        decay_scale: 0.88,
        tone_tilt: 0.25,
        drive: 0.92,
        room_mix: 0.35,
    },
];

// ============================================================================
// 3. Tape Presets
// ============================================================================

/// Factory preset specification for Moan's analog Tape saturation & flutter processor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapePreset {
    pub name: &'static str,
    pub drive: f32,
    pub head_bump_db: f32,
    pub tone_hz: f32,
    pub wow: f32,
    pub flutter: f32,
    pub hiss: f32,
    pub mix: f32,
}

pub const TAPE_PRESETS: &[TapePreset] = &[
    TapePreset {
        name: "Warm Reel",
        drive: 0.32,
        head_bump_db: 2.0,
        tone_hz: 14500.0,
        wow: 0.08,
        flutter: 0.06,
        hiss: 0.05,
        mix: 1.0,
    },
    TapePreset {
        name: "Master Glue",
        drive: 0.18,
        head_bump_db: 1.2,
        tone_hz: 17500.0,
        wow: 0.03,
        flutter: 0.02,
        hiss: 0.01,
        mix: 0.85,
    },
    TapePreset {
        name: "Lo-Fi Cassette",
        drive: 0.55,
        head_bump_db: 3.2,
        tone_hz: 5800.0,
        wow: 0.38,
        flutter: 0.32,
        hiss: 0.40,
        mix: 1.0,
    },
    TapePreset {
        name: "Worn Flutter",
        drive: 0.42,
        head_bump_db: 2.4,
        tone_hz: 7600.0,
        wow: 0.28,
        flutter: 0.72,
        hiss: 0.26,
        mix: 1.0,
    },
    TapePreset {
        name: "Pitch Wobble",
        drive: 0.28,
        head_bump_db: 1.6,
        tone_hz: 9200.0,
        wow: 0.82,
        flutter: 0.45,
        hiss: 0.16,
        mix: 1.0,
    },
    TapePreset {
        name: "Hot Oxide",
        drive: 0.84,
        head_bump_db: 3.8,
        tone_hz: 11800.0,
        wow: 0.12,
        flutter: 0.14,
        hiss: 0.12,
        mix: 0.90,
    },
];

// ============================================================================
// 4. Gate Presets
// ============================================================================

/// Factory preset specification for Moan's dynamics Gate processor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatePreset {
    pub name: &'static str,
    pub threshold_db: f32,
    pub hysteresis_db: f32,
    pub hold_ms: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub range_db: f32,
    pub sidechain_hp_hz: f32,
}

pub const GATE_PRESETS: &[GatePreset] = &[
    GatePreset {
        name: "Clean Threshold",
        threshold_db: -42.0,
        hysteresis_db: 4.0,
        hold_ms: 45.0,
        attack_ms: 0.8,
        release_ms: 140.0,
        range_db: -60.0,
        sidechain_hp_hz: 40.0,
    },
    GatePreset {
        name: "Tight Drums",
        threshold_db: -24.0,
        hysteresis_db: 6.0,
        hold_ms: 28.0,
        attack_ms: 0.2,
        release_ms: 65.0,
        range_db: -48.0,
        sidechain_hp_hz: 85.0,
    },
    GatePreset {
        name: "Gated Snare Reverb",
        threshold_db: -18.0,
        hysteresis_db: 3.0,
        hold_ms: 185.0,
        attack_ms: 0.5,
        release_ms: 35.0,
        range_db: -72.0,
        sidechain_hp_hz: 140.0,
    },
    GatePreset {
        name: "Vocal Noise Floor",
        threshold_db: -48.0,
        hysteresis_db: 5.5,
        hold_ms: 110.0,
        attack_ms: 2.5,
        release_ms: 240.0,
        range_db: -24.0,
        sidechain_hp_hz: 110.0,
    },
    GatePreset {
        name: "Rhythmic Chop",
        threshold_db: -14.0,
        hysteresis_db: 2.0,
        hold_ms: 12.0,
        attack_ms: 0.15,
        release_ms: 22.0,
        range_db: -80.0,
        sidechain_hp_hz: 60.0,
    },
];

// ============================================================================
// 5. Mastering Presets
// ============================================================================

/// Factory preset specification for Moan's final Mastering bus processor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasteringPreset {
    pub name: &'static str,
    pub input_db: f32,
    pub low_db: f32,
    pub body_db: f32,
    pub air_db: f32,
    pub comp_thresh_db: f32,
    pub comp_ratio: f32,
    pub comp_attack_ms: f32,
    pub comp_release_ms: f32,
    pub makeup_db: f32,
    pub stereo_width: f32,
    pub ceiling_db: f32,
    pub limiter_release_ms: f32,
}

pub const MASTERING_PRESETS: &[MasteringPreset] = &[
    MasteringPreset {
        name: "Balanced Clear",
        input_db: 0.0,
        low_db: 0.3,
        body_db: 0.0,
        air_db: 0.8,
        comp_thresh_db: -16.0,
        comp_ratio: 1.8,
        comp_attack_ms: 20.0,
        comp_release_ms: 140.0,
        makeup_db: 1.5,
        stereo_width: 1.05,
        ceiling_db: -1.0,
        limiter_release_ms: 80.0,
    },
    MasteringPreset {
        name: "Warm Glue",
        input_db: 0.5,
        low_db: 1.2,
        body_db: 0.8,
        air_db: -0.4,
        comp_thresh_db: -18.0,
        comp_ratio: 2.2,
        comp_attack_ms: 15.0,
        comp_release_ms: 180.0,
        makeup_db: 2.2,
        stereo_width: 0.98,
        ceiling_db: -1.0,
        limiter_release_ms: 110.0,
    },
    MasteringPreset {
        name: "Punchy Bus",
        input_db: 1.0,
        low_db: 1.5,
        body_db: -0.5,
        air_db: 1.2,
        comp_thresh_db: -15.0,
        comp_ratio: 3.0,
        comp_attack_ms: 30.0,
        comp_release_ms: 90.0,
        makeup_db: 2.8,
        stereo_width: 1.12,
        ceiling_db: -0.8,
        limiter_release_ms: 60.0,
    },
    MasteringPreset {
        name: "Loud Ceiling",
        input_db: 3.5,
        low_db: 0.8,
        body_db: 0.0,
        air_db: 1.0,
        comp_thresh_db: -14.0,
        comp_ratio: 3.5,
        comp_attack_ms: 12.0,
        comp_release_ms: 75.0,
        makeup_db: 4.0,
        stereo_width: 1.10,
        ceiling_db: -0.3,
        limiter_release_ms: 45.0,
    },
    MasteringPreset {
        name: "Sub Heavy",
        input_db: 1.2,
        low_db: 3.2,
        body_db: 0.5,
        air_db: 0.4,
        comp_thresh_db: -16.0,
        comp_ratio: 2.4,
        comp_attack_ms: 25.0,
        comp_release_ms: 120.0,
        makeup_db: 2.5,
        stereo_width: 1.02,
        ceiling_db: -0.8,
        limiter_release_ms: 70.0,
    },
    MasteringPreset {
        name: "Neon Wide",
        input_db: 1.5,
        low_db: 0.5,
        body_db: -0.6,
        air_db: 2.4,
        comp_thresh_db: -15.0,
        comp_ratio: 2.5,
        comp_attack_ms: 18.0,
        comp_release_ms: 100.0,
        makeup_db: 2.4,
        stereo_width: 1.42,
        ceiling_db: -0.6,
        limiter_release_ms: 65.0,
    },
    MasteringPreset {
        name: "Dusty Vintage",
        input_db: 0.0,
        low_db: 1.6,
        body_db: 1.4,
        air_db: -2.2,
        comp_thresh_db: -20.0,
        comp_ratio: 2.8,
        comp_attack_ms: 10.0,
        comp_release_ms: 220.0,
        makeup_db: 2.6,
        stereo_width: 0.86,
        ceiling_db: -1.5,
        limiter_release_ms: 140.0,
    },
    MasteringPreset {
        name: "Gentle Broadcast",
        input_db: -0.5,
        low_db: 0.0,
        body_db: 0.2,
        air_db: 0.5,
        comp_thresh_db: -19.0,
        comp_ratio: 1.5,
        comp_attack_ms: 22.0,
        comp_release_ms: 200.0,
        makeup_db: 1.2,
        stereo_width: 1.0,
        ceiling_db: -2.0,
        limiter_release_ms: 120.0,
    },
];

// ============================================================================
// 6. Lookup Helpers
// ============================================================================

/// Find a `Sub808Preset` by name (case-insensitive ASCII match).
pub fn find_sub808_preset(name: &str) -> Option<&'static Sub808Preset> {
    SUB808_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Find a `DrumKitPreset` by name (case-insensitive ASCII match).
pub fn find_drum_preset(name: &str) -> Option<&'static DrumKitPreset> {
    DRUMKIT_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Find a `TapePreset` by name (case-insensitive ASCII match).
pub fn find_tape_preset(name: &str) -> Option<&'static TapePreset> {
    TAPE_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Find a `GatePreset` by name (case-insensitive ASCII match).
pub fn find_gate_preset(name: &str) -> Option<&'static GatePreset> {
    GATE_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Find a `MasteringPreset` by name (case-insensitive ASCII match).
pub fn find_mastering_preset(name: &str) -> Option<&'static MasteringPreset> {
    MASTERING_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unique_names(names: &[&str]) {
        for (i, a) in names.iter().enumerate() {
            assert!(!a.is_empty(), "preset name must not be empty");
            for b in &names[i + 1..] {
                assert!(
                    !a.eq_ignore_ascii_case(b),
                    "duplicate preset name: '{}' vs '{}'",
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn test_sub808_presets_count_uniqueness_and_ranges() {
        assert_eq!(SUB808_PRESETS.len(), 14);

        let expected = [
            "Classic Sub",
            "Clean Sub",
            "Long Sustain",
            "Punchy Knock",
            "Warm Tube",
            "Heavy Fuzz",
            "Legato Slide",
            "Portamento Glide",
            "Deep Infrasonic",
            "Small Speaker Octave",
            "Tape Saturated",
            "Hard Clip 808",
            "Wavefold Growl",
            "Formant Growl",
        ];

        let mut names = alloc::vec::Vec::new();
        for (p, exp) in SUB808_PRESETS.iter().zip(expected.iter()) {
            assert_eq!(p.name, *exp);
            names.push(p.name);

            assert!((0.0..=1.0).contains(&p.shape), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.octave), "{}", p.name);
            assert!((0.0..=36.0).contains(&p.punch_st), "{}", p.name);
            assert!((5.0..=150.0).contains(&p.punch_ms), "{}", p.name);
            assert!(p.glide_mode <= SUB808_GLIDE_ALWAYS, "{}", p.name);
            assert!((0.0..=500.0).contains(&p.glide_ms), "{}", p.name);
            assert!((0.0..=500.0).contains(&p.hold_ms), "{}", p.name);
            assert!((50.0..=5000.0).contains(&p.decay_ms), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.sustain), "{}", p.name);
            assert!((5.0..=2000.0).contains(&p.release_ms), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.decay_track), "{}", p.name);
            assert!(p.drive_curve <= SUB808_DRIVE_FOLD, "{}", p.name);
            assert!((0.0..=1.0).contains(&p.drive), "{}", p.name);
            assert!((80.0..=12000.0).contains(&p.tone_hz), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.click), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.growl), "{}", p.name);
        }
        assert_unique_names(&names);
    }

    #[test]
    fn test_drumkit_presets_count_uniqueness_and_ranges() {
        assert_eq!(DRUMKIT_PRESETS.len(), 10);

        let expected = [
            "Electronic Core",
            "Circuit 808",
            "Circuit 909",
            "Acoustic Studio",
            "Heavy Metal Kit",
            "Dusty Vinyl Kit",
            "Gated 80s Kit",
            "Tight Trap Kit",
            "Crisp Brush Kit",
            "Industrial Crush Kit",
        ];

        let mut names = alloc::vec::Vec::new();
        for (p, exp) in DRUMKIT_PRESETS.iter().zip(expected.iter()) {
            assert_eq!(p.name, *exp);
            names.push(p.name);

            assert!(p.model <= DRUM_MODEL_MODAL_ACOUSTIC, "{}", p.name);
            assert!((-12.0..=12.0).contains(&p.pitch_semitones), "{}", p.name);
            assert!((0.25..=2.5).contains(&p.decay_scale), "{}", p.name);
            assert!((-1.0..=1.0).contains(&p.tone_tilt), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.drive), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.room_mix), "{}", p.name);
        }
        assert_unique_names(&names);
    }

    #[test]
    fn test_tape_presets_count_uniqueness_and_ranges() {
        assert_eq!(TAPE_PRESETS.len(), 6);

        let expected = [
            "Warm Reel",
            "Master Glue",
            "Lo-Fi Cassette",
            "Worn Flutter",
            "Pitch Wobble",
            "Hot Oxide",
        ];

        let mut names = alloc::vec::Vec::new();
        for (p, exp) in TAPE_PRESETS.iter().zip(expected.iter()) {
            assert_eq!(p.name, *exp);
            names.push(p.name);

            assert!((0.0..=1.0).contains(&p.drive), "{}", p.name);
            assert!((0.0..=6.0).contains(&p.head_bump_db), "{}", p.name);
            assert!((1500.0..=20000.0).contains(&p.tone_hz), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.wow), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.flutter), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.hiss), "{}", p.name);
            assert!((0.0..=1.0).contains(&p.mix), "{}", p.name);
        }
        assert_unique_names(&names);
    }

    #[test]
    fn test_gate_presets_count_uniqueness_and_ranges() {
        assert_eq!(GATE_PRESETS.len(), 5);

        let expected = [
            "Clean Threshold",
            "Tight Drums",
            "Gated Snare Reverb",
            "Vocal Noise Floor",
            "Rhythmic Chop",
        ];

        let mut names = alloc::vec::Vec::new();
        for (p, exp) in GATE_PRESETS.iter().zip(expected.iter()) {
            assert_eq!(p.name, *exp);
            names.push(p.name);

            assert!((-80.0..=0.0).contains(&p.threshold_db), "{}", p.name);
            assert!((0.0..=18.0).contains(&p.hysteresis_db), "{}", p.name);
            assert!((0.0..=1000.0).contains(&p.hold_ms), "{}", p.name);
            assert!((0.05..=100.0).contains(&p.attack_ms), "{}", p.name);
            assert!((5.0..=2000.0).contains(&p.release_ms), "{}", p.name);
            assert!((-96.0..=0.0).contains(&p.range_db), "{}", p.name);
            assert!((20.0..=1000.0).contains(&p.sidechain_hp_hz), "{}", p.name);
        }
        assert_unique_names(&names);
    }

    #[test]
    fn test_mastering_presets_count_uniqueness_and_ranges() {
        assert_eq!(MASTERING_PRESETS.len(), 8);

        let expected = [
            "Balanced Clear",
            "Warm Glue",
            "Punchy Bus",
            "Loud Ceiling",
            "Sub Heavy",
            "Neon Wide",
            "Dusty Vintage",
            "Gentle Broadcast",
        ];

        let mut names = alloc::vec::Vec::new();
        for (p, exp) in MASTERING_PRESETS.iter().zip(expected.iter()) {
            assert_eq!(p.name, *exp);
            names.push(p.name);

            assert!((-12.0..=12.0).contains(&p.input_db), "{}", p.name);
            assert!((-12.0..=12.0).contains(&p.low_db), "{}", p.name);
            assert!((-12.0..=12.0).contains(&p.body_db), "{}", p.name);
            assert!((-12.0..=12.0).contains(&p.air_db), "{}", p.name);
            assert!((-40.0..=0.0).contains(&p.comp_thresh_db), "{}", p.name);
            assert!((1.0..=20.0).contains(&p.comp_ratio), "{}", p.name);
            assert!((0.5..=200.0).contains(&p.comp_attack_ms), "{}", p.name);
            assert!((10.0..=1000.0).contains(&p.comp_release_ms), "{}", p.name);
            assert!((-6.0..=18.0).contains(&p.makeup_db), "{}", p.name);
            assert!((0.0..=2.0).contains(&p.stereo_width), "{}", p.name);
            assert!((-6.0..=0.0).contains(&p.ceiling_db), "{}", p.name);
            assert!((10.0..=500.0).contains(&p.limiter_release_ms), "{}", p.name);
        }
        assert_unique_names(&names);
    }

    #[test]
    fn test_lookup_helpers() {
        for p in SUB808_PRESETS {
            assert_eq!(find_sub808_preset(p.name), Some(p));
        }
        assert_eq!(
            find_sub808_preset("classic sub").map(|p| p.name),
            Some("Classic Sub")
        );
        assert!(find_sub808_preset("Nonexistent 808").is_none());

        for p in DRUMKIT_PRESETS {
            assert_eq!(find_drum_preset(p.name), Some(p));
        }
        assert_eq!(
            find_drum_preset("circuit 909").map(|p| p.name),
            Some("Circuit 909")
        );
        assert!(find_drum_preset("Nonexistent Drum").is_none());

        for p in TAPE_PRESETS {
            assert_eq!(find_tape_preset(p.name), Some(p));
        }
        assert_eq!(
            find_tape_preset("warm reel").map(|p| p.name),
            Some("Warm Reel")
        );
        assert!(find_tape_preset("Nonexistent Tape").is_none());

        for p in GATE_PRESETS {
            assert_eq!(find_gate_preset(p.name), Some(p));
        }
        assert_eq!(
            find_gate_preset("tight drums").map(|p| p.name),
            Some("Tight Drums")
        );
        assert!(find_gate_preset("Nonexistent Gate").is_none());

        for p in MASTERING_PRESETS {
            assert_eq!(find_mastering_preset(p.name), Some(p));
        }
        assert_eq!(
            find_mastering_preset("neon wide").map(|p| p.name),
            Some("Neon Wide")
        );
        assert!(find_mastering_preset("Nonexistent Master").is_none());
    }
}
