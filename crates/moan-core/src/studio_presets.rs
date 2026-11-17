// SPDX-License-Identifier: GPL-3.0-only
extern crate alloc;

// Muse Natural-Language Preset Designer & Genre Style Blueprint Catalog.
//
// Provides deterministic natural-language sound design for the Studio Chat Bar,
// converting prompt descriptions into concrete synthesizer and FX specifications,
// plus the comprehensive Genre & Song Structure Blueprint Catalog powering both
// Studio 2.0 Chat MIDI generation and Muse Tier 0 Featherweight generation.
//
// Zero external crate dependencies. Strict `#![no_std]` compatible.

// ============================================================================
// 1. Concrete Parameter Structs & Patch Specs
// ============================================================================

/// Synthesizer patch specification for Muse synth engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynthPatchSpec {
    pub name: &'static str,
    pub osc_a_pos: u8,
    pub osc_b_pos: u8,
    pub warp: u8,
    pub cutoff_hz: u16,
    pub resonance: u8,
    pub attack_ms: u16,
    pub release_ms: u16,
    pub lfo_rate_hz_q8: u16,
    pub lfo_cutoff_depth: u8,
}

impl SynthPatchSpec {
    pub const fn new(
        name: &'static str,
        osc_a_pos: u8,
        osc_b_pos: u8,
        warp: u8,
        cutoff_hz: u16,
        resonance: u8,
        attack_ms: u16,
        release_ms: u16,
        lfo_rate_hz_q8: u16,
        lfo_cutoff_depth: u8,
    ) -> Self {
        Self {
            name,
            osc_a_pos,
            osc_b_pos,
            warp,
            cutoff_hz,
            resonance,
            attack_ms,
            release_ms,
            lfo_rate_hz_q8,
            lfo_cutoff_depth,
        }
    }
}

pub const DISTORTION_MODE_SYM: u8 = 0;
pub const DISTORTION_MODE_ASYM: u8 = 1;
pub const DISTORTION_MODE_CLIP: u8 = 2;
pub const DISTORTION_MODE_FOLD: u8 = 3;

/// Distortion / Saturation FX patch specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DistortionPatchSpec {
    pub name: &'static str,
    pub mode: u8, // 0=Sym, 1=Asym, 2=Clip, 3=Fold
    pub drive_pct: u16,
    pub tilt: i8,
    pub bias: i8,
    pub cutoff_hz: u16,
    pub mix_pct: u8,
}

impl DistortionPatchSpec {
    pub const fn new(
        name: &'static str,
        mode: u8,
        drive_pct: u16,
        tilt: i8,
        bias: i8,
        cutoff_hz: u16,
        mix_pct: u8,
    ) -> Self {
        Self {
            name,
            mode,
            drive_pct,
            tilt,
            bias,
            cutoff_hz,
            mix_pct,
        }
    }

    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            DISTORTION_MODE_SYM => "Symmetric Soft Clip",
            DISTORTION_MODE_ASYM => "Asymmetric Tape Saturation",
            DISTORTION_MODE_CLIP => "Hard Clip Overdrive",
            DISTORTION_MODE_FOLD => "Wavefolder Diode",
            _ => "Custom Distortion",
        }
    }
}

/// 3-band Equalizer patch specification aligned with `effects::Equalizer` bounds.
/// 100 = unity (0 dB), 50 = -6 dB, 200 = +6 dB, max 400.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EqPatchSpec {
    pub name: &'static str,
    pub low_pct: u16,
    pub mid_pct: u16,
    pub high_pct: u16,
}

impl EqPatchSpec {
    pub const fn new(name: &'static str, low_pct: u16, mid_pct: u16, high_pct: u16) -> Self {
        Self {
            name,
            low_pct,
            mid_pct,
            high_pct,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.low_pct <= 400 && self.mid_pct <= 400 && self.high_pct <= 400
    }
}

/// Ariel vocal synthesis model parameter specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArielVoicePatchSpec {
    pub name: &'static str,
    pub base_f0_hz: u16,
    pub formant_scale_pct: u16,
    pub effort_pct: u8,
    pub breathiness_pct: u8,
    pub vibrato_hz_x10: u8,
    pub tilt_pct: i8,
}

impl ArielVoicePatchSpec {
    pub const fn new(
        name: &'static str,
        base_f0_hz: u16,
        formant_scale_pct: u16,
        effort_pct: u8,
        breathiness_pct: u8,
        vibrato_hz_x10: u8,
        tilt_pct: i8,
    ) -> Self {
        Self {
            name,
            base_f0_hz,
            formant_scale_pct,
            effort_pct,
            breathiness_pct,
            vibrato_hz_x10,
            tilt_pct,
        }
    }
}

/// Resulting preset created by the natural-language preset designer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignedPreset {
    Synth(SynthPatchSpec),
    Distortion(DistortionPatchSpec),
    Eq(EqPatchSpec),
    Voice(ArielVoicePatchSpec),
}

impl DesignedPreset {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Synth(s) => s.name,
            Self::Distortion(d) => d.name,
            Self::Eq(e) => e.name,
            Self::Voice(v) => v.name,
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::Synth(_) => "Synth",
            Self::Distortion(_) => "Distortion",
            Self::Eq(_) => "EQ",
            Self::Voice(_) => "Ariel Voice",
        }
    }

    pub fn as_synth(&self) -> Option<&SynthPatchSpec> {
        match self {
            Self::Synth(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_distortion(&self) -> Option<&DistortionPatchSpec> {
        match self {
            Self::Distortion(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_eq(&self) -> Option<&EqPatchSpec> {
        match self {
            Self::Eq(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_voice(&self) -> Option<&ArielVoicePatchSpec> {
        match self {
            Self::Voice(v) => Some(v),
            _ => None,
        }
    }
}

// ============================================================================
// 2. Curated Factory Preset Catalog (24 Presets across 4 Categories)
// ============================================================================

// --- Synth Presets ---
pub const FACTORY_SYNTH_DEEP_SUB_BASS: SynthPatchSpec = SynthPatchSpec::new(
    "Deep Sub Bass",
    0,   // Pure sine
    32,  // Sub triangle
    0,   // No warp
    280, // 280 Hz lowpass
    10,  // Low resonance
    15,  // 15ms attack
    400, // 400ms release
    0,   // No LFO
    0,
);

pub const FACTORY_SYNTH_ACID_303: SynthPatchSpec = SynthPatchSpec::new(
    "Acid 303 Squelch",
    128,  // Saw
    192,  // Square
    85,   // Wave warp
    1850, // 1850 Hz cutoff
    215,  // Screaming resonance
    3,    // Fast 3ms attack
    220,  // Snappy decay/release
    512,  // 2.0 Hz LFO
    140,  // High filter modulation
);

pub const FACTORY_SYNTH_WARM_ANALOG_PAD: SynthPatchSpec = SynthPatchSpec::new(
    "Warm Analog Pad",
    64,   // Triangle
    128,  // Saw
    20,   // Analog drift
    2400, // 2.4 kHz
    28,
    650,  // 650ms slow attack
    1800, // 1800ms lush release
    64,   // 0.25 Hz slow sweep
    50,
);

pub const FACTORY_SYNTH_PLUCKY_ARP_LEAD: SynthPatchSpec = SynthPatchSpec::new(
    "Plucky Arp Lead",
    128,  // Saw
    134,  // Detuned saw
    30,
    3800, // Bright cutoff
    80,
    2,    // 2ms transient attack
    160,  // Short snappy release
    1024, // 4.0 Hz
    25,
);

pub const FACTORY_SYNTH_CYBERPUNK_LEAD: SynthPatchSpec = SynthPatchSpec::new(
    "Cyberpunk Lead",
    192,  // Square
    240,  // Pulse sync
    115,  // Aggressive warp
    6500, // Wide open cutoff
    120,
    5,
    320,
    768, // 3.0 Hz
    90,
);

pub const FACTORY_SYNTH_LOFI_POLY_KEYS: SynthPatchSpec = SynthPatchSpec::new(
    "Lo-Fi Poly Keys",
    48,   // Sine-tri
    180,  // Square
    15,
    1600, // Warm rolloff
    25,
    40,
    580,
    85, // 0.33 Hz wobble
    35,
);

pub const FACTORY_SYNTH_808_BOOM: SynthPatchSpec = SynthPatchSpec::new(
    "808 Boom Sub",
    0, // Pure sine
    0,
    8,
    220, // Sub rumble
    6,
    2,    // Punchy instant attack
    1500, // Long sub sustain
    0,
    0,
);

pub const FACTORY_SYNTH_FUNK_BRASS_STAB: SynthPatchSpec = SynthPatchSpec::new(
    "Funk Brass Stab",
    128, // Dual saw
    142,
    40,
    3600,
    65,
    35,  // Horn swelling attack
    260, // Tight release
    0,
    0,
);

// --- Distortion Presets ---
pub const FACTORY_DISTORTION_TAPE_SATURATION: DistortionPatchSpec = DistortionPatchSpec::new(
    "Warm Tape Saturation",
    DISTORTION_MODE_ASYM,
    140,   // 140% drive
    -15,   // Warm bass tilt
    25,    // DC tape bias
    14000, // 14 kHz tape rolloff
    100,
);

pub const FACTORY_DISTORTION_GRITTY_ASYM: DistortionPatchSpec = DistortionPatchSpec::new(
    "Gritty Asymmetric Tape",
    DISTORTION_MODE_ASYM,
    320,   // 320% drive
    10,    // Slight bite
    45,    // Heavy asymmetric bias
    12500, // 12.5 kHz
    90,    // 90% wet
);

pub const FACTORY_DISTORTION_HARD_CLIP: DistortionPatchSpec = DistortionPatchSpec::new(
    "Hard Clip Overdrive",
    DISTORTION_MODE_CLIP,
    460,   // 460% hard drive
    20,    // Bright bite
    0,     // Symmetric clip
    18000, // Wide open
    100,
);

pub const FACTORY_DISTORTION_CYBER_WAVEFOLD: DistortionPatchSpec = DistortionPatchSpec::new(
    "Cyber Wavefold Diode",
    DISTORTION_MODE_FOLD,
    270,  // 270% fold drive
    -5,
    15,
    9500, // 9.5 kHz smoothing
    90,
);

pub const FACTORY_DISTORTION_LOFI_TELEPHONE: DistortionPatchSpec = DistortionPatchSpec::new(
    "Lo-Fi Telephone Crunch",
    DISTORTION_MODE_CLIP,
    280,
    55,   // Sharp midrange emphasis
    35,
    3400, // 3.4 kHz telephone bandwidth
    100,
);

// --- EQ Presets ---
pub const FACTORY_EQ_BRIGHT_VOCAL_AIR: EqPatchSpec = EqPatchSpec::new(
    "Bright Vocal Air Boost",
    85,  // Clean low cut
    110, // Gentle presence
    165, // +4.5 dB high air boost
);

pub const FACTORY_EQ_WARM_CLUB_BASS: EqPatchSpec = EqPatchSpec::new(
    "Warm Club Bass Boost",
    165, // Sub/bass weight
    95,
    80, // Tamed highs
);

pub const FACTORY_EQ_MID_SCOOP: EqPatchSpec = EqPatchSpec::new(
    "Aggressive Mid Scoop",
    140, // Low punch
    60,  // -4.5 dB scoop
    145, // Crisp highs
);

pub const FACTORY_EQ_TELEPHONE_FILTER: EqPatchSpec = EqPatchSpec::new(
    "Telephone Bandpass EQ",
    20,  // Low kill
    185, // Heavy midrange peak
    35,  // High rolloff
);

pub const FACTORY_EQ_LOFI_WARM_ROLLOFF: EqPatchSpec = EqPatchSpec::new(
    "Lo-Fi Warm Roll-off",
    130, // Warm low end
    100, // Flat mids
    65,  // -3.8 dB cozy treble rolloff
);

pub const FACTORY_EQ_ACOUSTIC_CLARITY: EqPatchSpec = EqPatchSpec::new(
    "Acoustic Clarity",
    90,  // Controlled rumble
    105,
    135, // Sparkling sheen
);

// --- Ariel Voice Presets ---
pub const FACTORY_VOICE_CYBERPUNK_ROBOT: ArielVoicePatchSpec = ArielVoicePatchSpec::new(
    "Cyberpunk Robotic Choir",
    110, // 110 Hz robotic base
    85,  // Compressed formants
    90,  // High vocal effort
    5,   // Low breath
    0,   // Zero vibrato (machine pitch)
    35,  // Metallic spectral tilt
);

pub const FACTORY_VOICE_ETHEREAL_CHOIR: ArielVoicePatchSpec = ArielVoicePatchSpec::new(
    "Ethereal Lush Choir",
    440, // A4 choir tone
    115, // Expanded feminine/ethereal formants
    45,  // Gentle effort
    60,  // High breathiness/aspiration
    52,  // 5.2 Hz gentle vibrato
    15,  // Air tilt
);

pub const FACTORY_VOICE_INTIMATE_LEAD: ArielVoicePatchSpec = ArielVoicePatchSpec::new(
    "Intimate Lead Vocal",
    220, // A3 natural lead
    100, // 100% natural formant scale
    55,  // Conversational effort
    35,  // Subtle breath
    58,  // 5.8 Hz human vibrato
    5,
);

pub const FACTORY_VOICE_AIRY_WHISPER: ArielVoicePatchSpec = ArielVoicePatchSpec::new(
    "Airy Whisper",
    280,
    108,
    20, // Low vocal fold effort
    90, // Dominant aspiration noise
    40, // 4.0 Hz slow flutter
    25,
);

pub const FACTORY_VOICE_DEEP_ANNOUNCER: ArielVoicePatchSpec = ArielVoicePatchSpec::new(
    "Deep Vocal Announcer",
    95,  // 95 Hz authoritative chest voice
    78,  // Deepened masculine formants
    80,  // Projected effort
    15,
    45,  // 4.5 Hz steady vibrato
    -10, // Rich chest resonance
);

/// Catalog of all 24 curated factory presets.
pub const ALL_FACTORY_PRESETS: &[DesignedPreset] = &[
    DesignedPreset::Synth(FACTORY_SYNTH_DEEP_SUB_BASS),
    DesignedPreset::Synth(FACTORY_SYNTH_ACID_303),
    DesignedPreset::Synth(FACTORY_SYNTH_WARM_ANALOG_PAD),
    DesignedPreset::Synth(FACTORY_SYNTH_PLUCKY_ARP_LEAD),
    DesignedPreset::Synth(FACTORY_SYNTH_CYBERPUNK_LEAD),
    DesignedPreset::Synth(FACTORY_SYNTH_LOFI_POLY_KEYS),
    DesignedPreset::Synth(FACTORY_SYNTH_808_BOOM),
    DesignedPreset::Synth(FACTORY_SYNTH_FUNK_BRASS_STAB),
    DesignedPreset::Distortion(FACTORY_DISTORTION_TAPE_SATURATION),
    DesignedPreset::Distortion(FACTORY_DISTORTION_GRITTY_ASYM),
    DesignedPreset::Distortion(FACTORY_DISTORTION_HARD_CLIP),
    DesignedPreset::Distortion(FACTORY_DISTORTION_CYBER_WAVEFOLD),
    DesignedPreset::Distortion(FACTORY_DISTORTION_LOFI_TELEPHONE),
    DesignedPreset::Eq(FACTORY_EQ_BRIGHT_VOCAL_AIR),
    DesignedPreset::Eq(FACTORY_EQ_WARM_CLUB_BASS),
    DesignedPreset::Eq(FACTORY_EQ_MID_SCOOP),
    DesignedPreset::Eq(FACTORY_EQ_TELEPHONE_FILTER),
    DesignedPreset::Eq(FACTORY_EQ_LOFI_WARM_ROLLOFF),
    DesignedPreset::Eq(FACTORY_EQ_ACOUSTIC_CLARITY),
    DesignedPreset::Voice(FACTORY_VOICE_CYBERPUNK_ROBOT),
    DesignedPreset::Voice(FACTORY_VOICE_ETHEREAL_CHOIR),
    DesignedPreset::Voice(FACTORY_VOICE_INTIMATE_LEAD),
    DesignedPreset::Voice(FACTORY_VOICE_AIRY_WHISPER),
    DesignedPreset::Voice(FACTORY_VOICE_DEEP_ANNOUNCER),
];

// ============================================================================
// 3. Natural Language Preset Designer (`design_preset_from_prompt`)
// ============================================================================

/// Case-insensitive substring search in `#![no_std]` without memory allocation.
fn has_word_ci(haystack: &str, needle: &str) -> bool {
    let h_bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();
    if n_bytes.is_empty() || h_bytes.len() < n_bytes.len() {
        return false;
    }
    for i in 0..=(h_bytes.len() - n_bytes.len()) {
        let mut matched = true;
        for j in 0..n_bytes.len() {
            if h_bytes[i + j].to_ascii_lowercase() != n_bytes[j].to_ascii_lowercase() {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

/// Synthesize a concrete patch specification from a natural language prompt.
///
/// Deterministically parses adjectives, sound qualities, and instrument types
/// like *"soft analog bass with a slow filter sweep"*, *"gritty asymmetric tape distortion"*,
/// *"bright vocal air EQ boost"*, *"cyberpunk robotic choir"*, or *"plucky arpeggio lead"*.
pub fn design_preset_from_prompt(prompt: &str) -> DesignedPreset {
    let is_eq = has_word_ci(prompt, "eq")
        || has_word_ci(prompt, "equalizer")
        || has_word_ci(prompt, "air boost")
        || has_word_ci(prompt, "mid scoop")
        || has_word_ci(prompt, "low boost")
        || has_word_ci(prompt, "high boost");

    let is_dist = has_word_ci(prompt, "distortion")
        || has_word_ci(prompt, "distort")
        || has_word_ci(prompt, "overdrive")
        || has_word_ci(prompt, "saturation")
        || has_word_ci(prompt, "saturat")
        || has_word_ci(prompt, "tape distortion")
        || has_word_ci(prompt, "wavefold")
        || has_word_ci(prompt, "clipping")
        || (has_word_ci(prompt, "gritty") && has_word_ci(prompt, "tape"))
        || (has_word_ci(prompt, "hard clip") && !has_word_ci(prompt, "synth"));

    let is_voice = has_word_ci(prompt, "vocal")
        || has_word_ci(prompt, "voice")
        || has_word_ci(prompt, "choir")
        || has_word_ci(prompt, "whisper")
        || has_word_ci(prompt, "formant")
        || has_word_ci(prompt, "sing")
        || has_word_ci(prompt, "ariel");

    if is_eq {
        DesignedPreset::Eq(design_eq(prompt))
    } else if is_dist {
        DesignedPreset::Distortion(design_distortion(prompt))
    } else if is_voice && !has_word_ci(prompt, "lead") && !has_word_ci(prompt, "bass") {
        DesignedPreset::Voice(design_voice(prompt))
    } else {
        DesignedPreset::Synth(design_synth(prompt))
    }
}

fn design_synth(prompt: &str) -> SynthPatchSpec {
    let mut osc_a = 128u8; // Default saw
    let mut osc_b = 128u8;
    let mut warp = 0u8;
    let mut cutoff = 3000u16;
    let mut resonance = 40u8;
    let mut attack = 10u16;
    let mut release = 300u16;
    let mut lfo_rate = 0u16;
    let mut lfo_depth = 0u8;
    let mut name = "Custom Synth";

    let has_bass = has_word_ci(prompt, "bass") || has_word_ci(prompt, "sub") || has_word_ci(prompt, "808");
    let has_soft = has_word_ci(prompt, "soft") || has_word_ci(prompt, "warm") || has_word_ci(prompt, "analog");
    let has_acid = has_word_ci(prompt, "acid") || has_word_ci(prompt, "303") || has_word_ci(prompt, "squelch");
    let has_pluck = has_word_ci(prompt, "pluck") || has_word_ci(prompt, "plucky") || has_word_ci(prompt, "arp") || has_word_ci(prompt, "arpeggio");
    let has_pad = has_word_ci(prompt, "pad") || has_word_ci(prompt, "lush") || has_word_ci(prompt, "ambient") || has_word_ci(prompt, "choir");
    let has_lead = has_word_ci(prompt, "lead");
    let has_brass = has_word_ci(prompt, "brass") || has_word_ci(prompt, "horn");
    let has_sweep = has_word_ci(prompt, "sweep") || has_word_ci(prompt, "wobble");
    let has_lofi = has_word_ci(prompt, "lofi") || has_word_ci(prompt, "lo-fi") || has_word_ci(prompt, "telephone");
    let has_cyber = has_word_ci(prompt, "cyberpunk") || has_word_ci(prompt, "industrial") || has_word_ci(prompt, "heavy");

    if has_808(prompt) {
        name = "Custom 808 Boom";
        osc_a = 0;
        osc_b = 0;
        warp = 10;
        cutoff = 220;
        resonance = 8;
        attack = 2;
        release = 1500;
    } else if has_acid {
        name = "Custom Acid Squelch";
        osc_a = 128;
        osc_b = 192;
        warp = 85;
        cutoff = 1900;
        resonance = 210;
        attack = 2;
        release = 220;
        lfo_rate = 512;
        lfo_depth = 135;
    } else if has_bass {
        if has_soft {
            name = "Custom Soft Analog Bass";
            osc_a = 32; // Sine/tri
            osc_b = 0;  // Sub sine
            warp = 18;
            cutoff = 950;
            resonance = 25;
            attack = 25;
            release = 350;
        } else {
            name = "Custom Punch Bass";
            osc_a = 128; // Saw
            osc_b = 192; // Square sub
            warp = 40;
            cutoff = 1400;
            resonance = 65;
            attack = 8;
            release = 280;
        }
    } else if has_pluck {
        name = "Custom Plucky Lead";
        osc_a = 128;
        osc_b = 134;
        warp = 30;
        cutoff = 4000;
        resonance = 75;
        attack = 2;
        release = 175;
    } else if has_pad {
        name = "Custom Lush Pad";
        osc_a = 64;  // Triangle
        osc_b = 128; // Saw
        warp = 20;
        cutoff = 2300;
        resonance = 30;
        attack = 650;
        release = 1700;
        lfo_rate = 64;
        lfo_depth = 50;
    } else if has_brass {
        name = "Custom Brass Lead";
        osc_a = 128;
        osc_b = 142;
        warp = 40;
        cutoff = 3600;
        resonance = 60;
        attack = 35;
        release = 260;
    } else if has_lead {
        name = "Custom Synth Lead";
        osc_a = 128;
        osc_b = 192;
        warp = 35;
        cutoff = 4800;
        resonance = 70;
        attack = 5;
        release = 320;
    }

    // Filter sweep modulation adjustments
    if has_sweep {
        if has_word_ci(prompt, "slow") {
            lfo_rate = 64;  // 0.25 Hz slow sweep
            lfo_depth = 65;
        } else if has_word_ci(prompt, "wobble") {
            lfo_rate = 768; // 3.0 Hz wobble
            lfo_depth = 110;
        } else {
            lfo_rate = 128; // 0.5 Hz standard sweep
            lfo_depth = 70;
        }
        if has_soft && has_bass {
            name = "Soft Analog Sweep Bass";
        }
    }

    if has_lofi {
        cutoff = cutoff.min(1800);
        warp = warp.saturating_add(15);
        if lfo_rate == 0 {
            lfo_rate = 85;
            lfo_depth = 30;
        }
    }

    if has_cyber {
        warp = warp.saturating_add(60);
        resonance = resonance.saturating_add(40);
        cutoff = cutoff.max(4500);
    }

    if has_word_ci(prompt, "bright") {
        cutoff = cutoff.saturating_add(2000).min(18000);
    } else if has_word_ci(prompt, "dark") {
        cutoff = cutoff.saturating_sub(800).max(180);
    }

    SynthPatchSpec::new(
        name,
        osc_a,
        osc_b,
        warp,
        cutoff,
        resonance,
        attack,
        release,
        lfo_rate,
        lfo_depth,
    )
}

fn has_808(prompt: &str) -> bool {
    has_word_ci(prompt, "808") || has_word_ci(prompt, "boom")
}

fn design_distortion(prompt: &str) -> DistortionPatchSpec {
    let mut mode = DISTORTION_MODE_SYM;
    let mut drive = 150u16;
    let mut tilt = 0i8;
    let mut bias = 0i8;
    let mut cutoff = 16000u16;
    let mut mix = 100u8;
    let mut name = "Custom Distortion";

    if has_word_ci(prompt, "asymmetric") || has_word_ci(prompt, "asym") || has_word_ci(prompt, "tape") {
        mode = DISTORTION_MODE_ASYM;
        bias = 40;
        tilt = -10;
        drive = 280;
        cutoff = 13500;
        name = "Custom Asymmetric Tape";

        if has_word_ci(prompt, "gritty") {
            drive = 320;
            tilt = 10;
            bias = 45;
            cutoff = 12000;
            mix = 90;
            name = "Gritty Asymmetric Tape";
        }
    } else if has_word_ci(prompt, "clip") || has_word_ci(prompt, "hard") || has_word_ci(prompt, "overdrive") {
        mode = DISTORTION_MODE_CLIP;
        drive = 450;
        tilt = 20;
        bias = 0;
        cutoff = 18000;
        name = "Hard Clip Overdrive";
    } else if has_word_ci(prompt, "fold") || has_word_ci(prompt, "wavefold") || has_word_ci(prompt, "cyberpunk") {
        mode = DISTORTION_MODE_FOLD;
        drive = 260;
        tilt = -5;
        bias = 15;
        cutoff = 9500;
        mix = 90;
        name = "Cyber Wavefolder";
    } else if has_word_ci(prompt, "telephone") || has_word_ci(prompt, "lofi") || has_word_ci(prompt, "crunch") {
        mode = DISTORTION_MODE_CLIP;
        drive = 280;
        tilt = 50;
        bias = 30;
        cutoff = 3500;
        name = "Lo-Fi Telephone Crunch";
    }

    if has_word_ci(prompt, "heavy") {
        drive = drive.saturating_add(200);
    }
    if has_word_ci(prompt, "warm") {
        tilt = tilt.saturating_sub(15);
    }

    DistortionPatchSpec::new(name, mode, drive, tilt, bias, cutoff, mix)
}

fn design_eq(prompt: &str) -> EqPatchSpec {
    let mut low = 100u16;
    let mut mid = 100u16;
    let mut high = 100u16;
    let mut name = "Custom EQ";

    if (has_word_ci(prompt, "bright") || has_word_ci(prompt, "air")) && has_word_ci(prompt, "boost") {
        low = 85;   // Low clean cut
        mid = 110;  // Subtle presence
        high = 165; // +4.5 dB high air boost
        name = "Bright Vocal Air Boost";
    } else if has_word_ci(prompt, "warm") || has_word_ci(prompt, "bass") || has_word_ci(prompt, "club") {
        low = 160;
        mid = 95;
        high = 80;
        name = "Warm Club Bass Boost";
    } else if has_word_ci(prompt, "scoop") || has_word_ci(prompt, "metal") {
        low = 140;
        mid = 60;
        high = 145;
        name = "Aggressive Mid Scoop";
    } else if has_word_ci(prompt, "telephone") || has_word_ci(prompt, "radio") {
        low = 20;
        mid = 180;
        high = 35;
        name = "Telephone Bandpass EQ";
    } else if has_word_ci(prompt, "lofi") || has_word_ci(prompt, "lo-fi") {
        low = 130;
        mid = 100;
        high = 65;
        name = "Lo-Fi Warm Roll-off";
    } else {
        if has_word_ci(prompt, "high") && has_word_ci(prompt, "boost") {
            high = 150;
        }
        if has_word_ci(prompt, "low") && has_word_ci(prompt, "boost") {
            low = 150;
        }
    }

    EqPatchSpec::new(name, low.min(400), mid.min(400), high.min(400))
}

fn design_voice(prompt: &str) -> ArielVoicePatchSpec {
    let mut f0 = 220u16;
    let mut formant = 100u16;
    let mut effort = 60u8;
    let mut breath = 30u8;
    let mut vibrato = 55u8;
    let mut tilt = 0i8;
    let mut name = "Custom Voice";

    if has_word_ci(prompt, "robot") || has_word_ci(prompt, "cyberpunk") {
        f0 = 110;
        formant = 85;
        effort = 90;
        breath = 5;
        vibrato = 0; // Pure robotic pitch
        tilt = 35;  // Metallic edge
        name = "Cyberpunk Robotic Choir";
    } else if has_word_ci(prompt, "choir") || has_word_ci(prompt, "ethereal") || has_word_ci(prompt, "lush") {
        f0 = 440;
        formant = 115;
        effort = 45;
        breath = 60;
        vibrato = 52;
        tilt = 15;
        name = "Ethereal Lush Choir";
    } else if has_word_ci(prompt, "whisper") || has_word_ci(prompt, "airy") {
        f0 = 280;
        formant = 108;
        effort = 20;
        breath = 90;
        vibrato = 40;
        tilt = 25;
        name = "Airy Whisper";
    } else if has_word_ci(prompt, "deep") || has_word_ci(prompt, "announcer") {
        f0 = 95;
        formant = 78;
        effort = 80;
        breath = 15;
        vibrato = 45;
        tilt = -10;
        name = "Deep Vocal Announcer";
    }

    ArielVoicePatchSpec::new(name, f0, formant, effort, breath, vibrato, tilt)
}

// ============================================================================
// 4. Genre Scales, Euclidean Helpers & Song Structure Blueprint Catalog
// ============================================================================

pub const SCALE_NATURAL_MINOR: &[u8] = &[0, 2, 3, 5, 7, 8, 10];
pub const SCALE_DORIAN: &[u8] = &[0, 2, 3, 5, 7, 9, 10];
pub const SCALE_MAJOR: &[u8] = &[0, 2, 4, 5, 7, 9, 11];
pub const SCALE_MIXOLYDIAN: &[u8] = &[0, 2, 4, 5, 7, 9, 10];
pub const SCALE_PHRYGIAN: &[u8] = &[0, 1, 3, 5, 7, 8, 10];
pub const SCALE_HARMONIC_MINOR: &[u8] = &[0, 2, 3, 5, 7, 8, 11];
pub const SCALE_MINOR_PENTATONIC: &[u8] = &[0, 3, 5, 7, 10];

/// Generates a 16-step Euclidean rhythm bitmask `E(hits, 16)`.
/// Bit `(15 - step)` is 1 if step is active, where step 0 is the downbeat (MSB `0x8000`).
pub fn euclidean_pattern_16(hits: u8) -> u16 {
    if hits == 0 {
        return 0;
    }
    if hits >= 16 {
        return 0xFFFF;
    }
    let mut mask = 0u16;
    for i in 0..16 {
        if ((i as u32 * hits as u32) % 16) < hits as u32 {
            mask |= 1 << (15 - i);
        }
    }
    mask
}

/// Generates backbeat or distributed snare masks for 16-step measures.
pub fn snare_pattern_16(hits: u8) -> u16 {
    match hits {
        0 => 0,
        2 => (1 << 11) | (1 << 3), // Steps 4 and 12 (beats 2 & 4)
        4 => (1 << 11) | (1 << 7) | (1 << 3) | (1 << 1), // Steps 4, 8, 12, 14
        _ => euclidean_pattern_16(hits),
    }
}

/// Genre & Song Structure Blueprint for Muse Chat and Muse Generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SongBlueprint {
    pub genre: &'static str,
    pub bpm: u16,
    pub root_midi: u8,
    pub scale_intervals: &'static [u8],
    pub chord_progression_degrees: [u8; 4],
    pub drum_euclid_hits: (u8, u8, u8), // (kick, snare, hat)
    pub bass_rhythm_mask: u16,          // 16-step bitmask (bit 15 = step 0)
    pub brass_or_lead_motif: [i8; 8],   // 8-note relative semitone offsets
}

impl SongBlueprint {
    /// True if the bass plays a note on the given step (0..15).
    pub fn is_bass_hit(&self, step: u8) -> bool {
        let bit = 15 - (step % 16);
        ((self.bass_rhythm_mask >> bit) & 1) != 0
    }

    /// Kick drum rhythm mask in 16 steps.
    pub fn kick_mask(&self) -> u16 {
        euclidean_pattern_16(self.drum_euclid_hits.0)
    }

    /// Snare drum rhythm mask in 16 steps.
    pub fn snare_mask(&self) -> u16 {
        snare_pattern_16(self.drum_euclid_hits.1)
    }

    /// Hi-hat rhythm mask in 16 steps.
    pub fn hat_mask(&self) -> u16 {
        euclidean_pattern_16(self.drum_euclid_hits.2)
    }

    /// Computes the MIDI root note for a chord in the progression (0..3).
    pub fn chord_root_midi(&self, chord_index: usize) -> u8 {
        let degree = self.chord_progression_degrees[chord_index % 4];
        let idx = (degree.saturating_sub(1) as usize) % self.scale_intervals.len();
        let semitones = self.scale_intervals[idx];
        self.root_midi.saturating_add(semitones)
    }
}

// --- 10 Curated Genre Style Blueprints ---

pub const BLUEPRINT_HOUSE: SongBlueprint = SongBlueprint {
    genre: "House",
    bpm: 125,
    root_midi: 57, // A3
    scale_intervals: SCALE_NATURAL_MINOR,
    chord_progression_degrees: [1, 6, 3, 7], // i - VI - III - VII
    drum_euclid_hits: (4, 4, 16),            // 4-on-floor, 4 claps, 16 driving hats
    bass_rhythm_mask: 0x2222,                // Offbeat bass on '&' of every beat (steps 2, 6, 10, 14)
    brass_or_lead_motif: [0, 3, 7, 10, 7, 3, 0, -2],
};

pub const BLUEPRINT_SYNTHWAVE: SongBlueprint = SongBlueprint {
    genre: "Synthwave",
    bpm: 110,
    root_midi: 45, // A2
    scale_intervals: SCALE_NATURAL_MINOR,
    chord_progression_degrees: [1, 7, 6, 7], // i - VII - VI - VII
    drum_euclid_hits: (4, 2, 8),             // 4 kicks, 2 big gated snares on 2 & 4, 8th hats
    bass_rhythm_mask: 0xAAAA,                // Driving 8th-note rolling bass
    brass_or_lead_motif: [0, 7, 12, 7, 10, 7, 3, 2],
};

pub const BLUEPRINT_LOFI: SongBlueprint = SongBlueprint {
    genre: "Lo-Fi Hip-Hop",
    bpm: 85,
    root_midi: 53, // F3
    scale_intervals: SCALE_DORIAN,
    chord_progression_degrees: [2, 5, 1, 6], // ii - V - I - vi jazz turnaround
    drum_euclid_hits: (3, 2, 6),             // Syncopated boom-bap kick, 2 snares, 6 swung hats
    bass_rhythm_mask: 0x8284,                // Syncopated walking low-end groove
    brass_or_lead_motif: [0, 2, 4, 7, 9, 7, 4, 2],
};

pub const BLUEPRINT_DNB: SongBlueprint = SongBlueprint {
    genre: "Drum & Bass",
    bpm: 174,
    root_midi: 43, // G2
    scale_intervals: SCALE_NATURAL_MINOR,
    chord_progression_degrees: [1, 6, 4, 5],
    drum_euclid_hits: (3, 4, 12), // Amen-style breakbeat hits
    bass_rhythm_mask: 0x9044,     // Rolling reese/sub groove
    brass_or_lead_motif: [0, 12, 7, 10, 12, 3, 5, 0],
};

pub const BLUEPRINT_AMBIENT_TECHNO: SongBlueprint = SongBlueprint {
    genre: "Ambient Techno",
    bpm: 130,
    root_midi: 48, // C3
    scale_intervals: SCALE_DORIAN,
    chord_progression_degrees: [1, 1, 4, 1], // Hypnotic dub-chord tonic
    drum_euclid_hits: (4, 2, 8),             // Deep 4-on-floor, subtle backbeat, 8 hats
    bass_rhythm_mask: 0x8888,                // Sub rumble on downbeats
    brass_or_lead_motif: [0, 3, 7, 10, 0, 3, 7, 10],
};

pub const BLUEPRINT_INDIE_POP: SongBlueprint = SongBlueprint {
    genre: "Indie Pop",
    bpm: 118,
    root_midi: 60, // C4
    scale_intervals: SCALE_MAJOR,
    chord_progression_degrees: [1, 5, 6, 4], // I - V - vi - IV anthem
    drum_euclid_hits: (4, 2, 8),
    bass_rhythm_mask: 0x8A88, // Melodic bounce
    brass_or_lead_motif: [0, 4, 7, 9, 7, 4, 2, 0],
};

pub const BLUEPRINT_TRAP: SongBlueprint = SongBlueprint {
    genre: "Trap",
    bpm: 140,
    root_midi: 38, // D2 low 808
    scale_intervals: SCALE_PHRYGIAN,
    chord_progression_degrees: [1, 6, 5, 6],
    drum_euclid_hits: (3, 2, 14), // 3 808 hits, 2 sharp claps, 14 rolling hi-hats
    bass_rhythm_mask: 0x8020,     // Sustained 808 slides (steps 0 and 10)
    brass_or_lead_motif: [0, 1, 0, 7, 8, 7, 1, 0],
};

pub const BLUEPRINT_FUNK_BRASS: SongBlueprint = SongBlueprint {
    genre: "Funk Brass",
    bpm: 108,
    root_midi: 50, // D3
    scale_intervals: SCALE_MIXOLYDIAN,
    chord_progression_degrees: [1, 4, 1, 5], // Dominant funk vamps
    drum_euclid_hits: (5, 4, 12),            // Tight syncopated pocket
    bass_rhythm_mask: 0x9248,                // Slap bass syncopation
    brass_or_lead_motif: [0, 0, 3, 5, 7, 5, 3, 0],
};

pub const BLUEPRINT_ACOUSTIC_BALLAD: SongBlueprint = SongBlueprint {
    genre: "Acoustic Ballad",
    bpm: 72,
    root_midi: 60, // C4
    scale_intervals: SCALE_MAJOR,
    chord_progression_degrees: [1, 6, 4, 5], // Emotional I - vi - IV - V
    drum_euclid_hits: (2, 2, 4),             // Gentle heartbeat kick, soft rimshot, 4 hats
    bass_rhythm_mask: 0x8080,                // Slow acoustic downbeats
    brass_or_lead_motif: [0, 2, 4, 5, 7, 4, 2, 0],
};

pub const BLUEPRINT_INDUSTRIAL: SongBlueprint = SongBlueprint {
    genre: "Industrial",
    bpm: 135,
    root_midi: 40, // E2
    scale_intervals: SCALE_PHRYGIAN,
    chord_progression_degrees: [1, 2, 1, 7], // Dissonant semitone tension
    drum_euclid_hits: (4, 4, 10),            // Heavy stomping kicks, metallic claps
    bass_rhythm_mask: 0xEEEE,                // Relentless driving EBM 16th pulse
    brass_or_lead_motif: [0, 12, 1, 13, 0, 10, 1, 0],
};

pub const ALL_BLUEPRINTS: &[SongBlueprint] = &[
    BLUEPRINT_HOUSE,
    BLUEPRINT_SYNTHWAVE,
    BLUEPRINT_LOFI,
    BLUEPRINT_DNB,
    BLUEPRINT_AMBIENT_TECHNO,
    BLUEPRINT_INDIE_POP,
    BLUEPRINT_TRAP,
    BLUEPRINT_FUNK_BRASS,
    BLUEPRINT_ACOUSTIC_BALLAD,
    BLUEPRINT_INDUSTRIAL,
];

/// Returns all 10 curated genre blueprints.
pub fn all_song_blueprints() -> &'static [SongBlueprint] {
    ALL_BLUEPRINTS
}

/// Extract BPM integer from prompt if written like "125 bpm", "at 130bpm", etc.
fn extract_bpm_from_prompt(prompt: &str) -> Option<u16> {
    let bytes = prompt.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            let mut val = 0u32;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                val = val * 10 + (bytes[j] - b'0') as u32;
                if val > 999 {
                    break;
                }
                j += 1;
            }
            if val >= 30 && val <= 240 {
                // Check if followed by "bpm" or "BPM"
                let mut k = j;
                while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'-') {
                    k += 1;
                }
                if k + 2 < bytes.len()
                    && bytes[k].to_ascii_lowercase() == b'b'
                    && bytes[k + 1].to_ascii_lowercase() == b'p'
                    && bytes[k + 2].to_ascii_lowercase() == b'm'
                {
                    return Some(val as u16);
                }
            }
        }
    }
    None
}

/// Map a natural-language genre or style prompt to a concrete `SongBlueprint`.
///
/// Supports House / Club Beat, Synthwave, Lo-Fi Hip-Hop, Drum & Bass, Ambient Techno,
/// Indie Pop, Trap, Funk Brass, Acoustic Ballad, and Industrial.
/// Applies `bpm_override` if provided, or parses inline tempo (e.g. "at 125 bpm").
pub fn plan_song_blueprint(prompt: &str, bpm_override: Option<u16>) -> SongBlueprint {
    let mut blueprint = if has_word_ci(prompt, "synthwave")
        || has_word_ci(prompt, "retrowave")
        || has_word_ci(prompt, "outrun")
        || has_word_ci(prompt, "80s")
    {
        BLUEPRINT_SYNTHWAVE
    } else if has_word_ci(prompt, "lo-fi")
        || has_word_ci(prompt, "lofi")
        || has_word_ci(prompt, "chillhop")
        || has_word_ci(prompt, "boom bap")
        || has_word_ci(prompt, "hip-hop")
        || has_word_ci(prompt, "hip hop")
    {
        BLUEPRINT_LOFI
    } else if has_word_ci(prompt, "drum & bass")
        || has_word_ci(prompt, "drum and bass")
        || has_word_ci(prompt, "dnb")
        || has_word_ci(prompt, "jungle")
    {
        BLUEPRINT_DNB
    } else if has_word_ci(prompt, "ambient") || has_word_ci(prompt, "techno") || has_word_ci(prompt, "minimal") {
        BLUEPRINT_AMBIENT_TECHNO
    } else if has_word_ci(prompt, "indie") || has_word_ci(prompt, "bedroom pop") {
        BLUEPRINT_INDIE_POP
    } else if has_word_ci(prompt, "trap") || has_word_ci(prompt, "drill") {
        BLUEPRINT_TRAP
    } else if has_word_ci(prompt, "funk") || has_word_ci(prompt, "brass") || has_word_ci(prompt, "horn") {
        BLUEPRINT_FUNK_BRASS
    } else if has_word_ci(prompt, "ballad") || has_word_ci(prompt, "acoustic") || has_word_ci(prompt, "folk") {
        BLUEPRINT_ACOUSTIC_BALLAD
    } else if has_word_ci(prompt, "industrial") || has_word_ci(prompt, "ebm") || has_word_ci(prompt, "darkwave") {
        BLUEPRINT_INDUSTRIAL
    } else if has_word_ci(prompt, "pop") {
        BLUEPRINT_INDIE_POP
    } else {
        // Default to House / Club Beat
        BLUEPRINT_HOUSE
    };

    if let Some(bpm) = bpm_override {
        blueprint.bpm = bpm.clamp(30, 240);
    } else if let Some(bpm) = extract_bpm_from_prompt(prompt) {
        blueprint.bpm = bpm.clamp(30, 240);
    }

    blueprint
}

// ============================================================================
// 5. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_designer_prompt_examples() {
        // 1. "soft analog bass with a slow filter sweep"
        let p1 = design_preset_from_prompt("soft analog bass with a slow filter sweep");
        assert_eq!(p1.category(), "Synth");
        let s1 = p1.as_synth().expect("Expected Synth preset");
        assert_eq!(s1.name, "Soft Analog Sweep Bass");
        assert_eq!(s1.lfo_rate_hz_q8, 64); // 0.25 Hz slow sweep
        assert!(s1.lfo_cutoff_depth >= 60);
        assert!(s1.cutoff_hz <= 1200);

        // 2. "gritty asymmetric tape distortion"
        let p2 = design_preset_from_prompt("gritty asymmetric tape distortion");
        assert_eq!(p2.category(), "Distortion");
        let d2 = p2.as_distortion().expect("Expected Distortion preset");
        assert_eq!(d2.name, "Gritty Asymmetric Tape");
        assert_eq!(d2.mode, DISTORTION_MODE_ASYM);
        assert_eq!(d2.drive_pct, 320);
        assert!(d2.bias > 0);

        // 3. "bright vocal air EQ boost"
        let p3 = design_preset_from_prompt("bright vocal air EQ boost");
        assert_eq!(p3.category(), "EQ");
        let e3 = p3.as_eq().expect("Expected EQ preset");
        assert_eq!(e3.name, "Bright Vocal Air Boost");
        assert_eq!(e3.high_pct, 165);
        assert!(e3.is_valid());

        // 4. "cyberpunk robotic choir"
        let p4 = design_preset_from_prompt("cyberpunk robotic choir");
        assert_eq!(p4.category(), "Ariel Voice");
        let v4 = p4.as_voice().expect("Expected Ariel Voice preset");
        assert_eq!(v4.name, "Cyberpunk Robotic Choir");
        assert_eq!(v4.vibrato_hz_x10, 0); // No vibrato for robotic voice
        assert!(v4.effort_pct >= 85);
        assert!(v4.tilt_pct > 0);

        // 5. "plucky arpeggio lead"
        let p5 = design_preset_from_prompt("plucky arpeggio lead");
        assert_eq!(p5.category(), "Synth");
        let s5 = p5.as_synth().expect("Expected Synth preset");
        assert_eq!(s5.name, "Custom Plucky Lead");
        assert!(s5.attack_ms <= 5);
        assert!(s5.release_ms <= 200);
        assert!(s5.cutoff_hz >= 3000);
    }

    #[test]
    fn test_designer_acid_and_808() {
        let acid = design_preset_from_prompt("acid 303 squelch bass");
        let s_acid = acid.as_synth().unwrap();
        assert_eq!(s_acid.name, "Custom Acid Squelch");
        assert!(s_acid.resonance >= 200);

        let sub808 = design_preset_from_prompt("deep sub 808 boom bass");
        let s_808 = sub808.as_synth().unwrap();
        assert_eq!(s_808.name, "Custom 808 Boom");
        assert_eq!(s_808.osc_a_pos, 0); // Pure sine
        assert!(s_808.release_ms >= 1000);
    }

    #[test]
    fn test_designer_case_insensitivity_and_determinism() {
        let lower = design_preset_from_prompt("soft analog bass");
        let upper = design_preset_from_prompt("SOFT ANALOG BASS");
        let mixed = design_preset_from_prompt("SoFt AnAlOg BaSs");

        assert_eq!(lower, upper);
        assert_eq!(upper, mixed);
    }

    #[test]
    fn test_designer_bounds_safety() {
        let prompts = [
            "",
            "random text noise 12345",
            "heavy hard clip overdrive with extreme distortion",
            "super high bright vocal air boost eq equalizer",
            "deep low dark bass sub pad choir",
        ];

        for prompt in prompts {
            let preset = design_preset_from_prompt(prompt);
            match preset {
                DesignedPreset::Synth(s) => {
                    assert!(s.cutoff_hz <= 20000);
                }
                DesignedPreset::Distortion(d) => {
                    assert!(d.mode <= 3);
                    assert!(d.mix_pct <= 100);
                }
                DesignedPreset::Eq(e) => {
                    assert!(e.is_valid());
                }
                DesignedPreset::Voice(v) => {
                    assert!(v.formant_scale_pct >= 50);
                    assert!(v.effort_pct <= 100);
                    assert!(v.breathiness_pct <= 100);
                }
            }
        }
    }

    #[test]
    fn test_factory_preset_catalog() {
        assert!(ALL_FACTORY_PRESETS.len() >= 12);
        assert_eq!(ALL_FACTORY_PRESETS.len(), 24);

        for preset in ALL_FACTORY_PRESETS {
            assert!(!preset.name().is_empty());
            match preset {
                DesignedPreset::Eq(e) => assert!(e.is_valid()),
                DesignedPreset::Distortion(d) => assert!(d.mode <= 3),
                _ => {}
            }
        }
    }

    #[test]
    fn test_genre_blueprints_all_10() {
        let blueprints = all_song_blueprints();
        assert_eq!(blueprints.len(), 10);

        let house = plan_song_blueprint("create a house club beat", None);
        assert_eq!(house.genre, "House");
        assert_eq!(house.bpm, 125);
        assert_eq!(house.bass_rhythm_mask, 0x2222); // Classic offbeat

        let synthwave = plan_song_blueprint("80s retrowave synthwave track", None);
        assert_eq!(synthwave.genre, "Synthwave");
        assert_eq!(synthwave.bpm, 110);
        assert_eq!(synthwave.bass_rhythm_mask, 0xAAAA); // 8th note rolling bass

        let lofi = plan_song_blueprint("chill lofi hip-hop beat", None);
        assert_eq!(lofi.genre, "Lo-Fi Hip-Hop");
        assert_eq!(lofi.bpm, 85);

        let dnb = plan_song_blueprint("drum and bass roller", None);
        assert_eq!(dnb.genre, "Drum & Bass");
        assert_eq!(dnb.bpm, 174);

        let techno = plan_song_blueprint("ambient minimal techno", None);
        assert_eq!(techno.genre, "Ambient Techno");
        assert_eq!(techno.bpm, 130);

        let pop = plan_song_blueprint("indie pop bedroom guitar", None);
        assert_eq!(pop.genre, "Indie Pop");
        assert_eq!(pop.bpm, 118);

        let trap = plan_song_blueprint("dark 808 trap beat", None);
        assert_eq!(trap.genre, "Trap");
        assert_eq!(trap.bpm, 140);
        assert_eq!(trap.bass_rhythm_mask, 0x8020);

        let funk = plan_song_blueprint("funky brass groove", None);
        assert_eq!(funk.genre, "Funk Brass");
        assert_eq!(funk.bpm, 108);

        let ballad = plan_song_blueprint("acoustic ballad piano", None);
        assert_eq!(ballad.genre, "Acoustic Ballad");
        assert_eq!(ballad.bpm, 72);

        let industrial = plan_song_blueprint("ebm industrial darkwave", None);
        assert_eq!(industrial.genre, "Industrial");
        assert_eq!(industrial.bpm, 135);
        assert_eq!(industrial.bass_rhythm_mask, 0xEEEE);
    }

    #[test]
    fn test_blueprint_bpm_override_and_inline_parse() {
        // Override via parameter
        let bp1 = plan_song_blueprint("House club track", Some(128));
        assert_eq!(bp1.bpm, 128);

        // Inline parse from prompt
        let bp2 = plan_song_blueprint("Create a Synthwave bassline at 116 bpm", None);
        assert_eq!(bp2.genre, "Synthwave");
        assert_eq!(bp2.bpm, 116);
    }

    #[test]
    fn test_euclidean_patterns() {
        assert_eq!(euclidean_pattern_16(0), 0x0000);
        assert_eq!(euclidean_pattern_16(4), 0x8888); // 4-on-the-floor
        assert_eq!(euclidean_pattern_16(8), 0xAAAA); // 8th notes
        assert_eq!(euclidean_pattern_16(16), 0xFFFF); // All 16 steps

        // Snare backbeats on beats 2 and 4 (steps 4 and 12)
        assert_eq!(snare_pattern_16(2), (1 << 11) | (1 << 3));
    }

    #[test]
    fn test_blueprint_helper_methods() {
        let house = BLUEPRINT_HOUSE;
        assert_eq!(house.kick_mask(), 0x8888);
        assert!(house.is_bass_hit(2));  // Offbeat step 2
        assert!(!house.is_bass_hit(0)); // No bass on downbeat step 0

        // Chord roots
        let root0 = house.chord_root_midi(0);
        assert_eq!(root0, house.root_midi); // Degree 1 = root
    }
}
