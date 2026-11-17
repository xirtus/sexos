// SPDX-License-Identifier: GPL-3.0-only
extern crate alloc;

// Multi-channel kit starters, genre song starter templates, and beat-group
// step colors for `Tapestry` and `Moan`.
//
// Zero external crate dependencies. Compatible with `#![no_std]` crates via
// `pub mod kits;` as well as standalone `rustc --test` verification.

/// Instrument code matching `Moan`'s core synthesizer/sampler engine table.
pub const INSTR_SILK_TRIANGLE: u8 = 0;
pub const INSTR_SATIN_PULSE: u8 = 1;
pub const INSTR_VELVET_SAMPLER: u8 = 2;
pub const INSTR_ARIEL_FORMANT: u8 = 3;
pub const INSTR_PRISM_WAVETABLE: u8 = 4;

/// Default choke group for closed/open hi-hat pairs.
pub const CUT_GROUP_HIHAT: u8 = 1;
/// Choke group for monophonic bass / vocal chop channels.
pub const CUT_GROUP_MONO: u8 = 2;

/// Template for a single `Tapestry` channel created when loading a kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelTemplate {
    pub name: &'static str,
    pub instrument: u8,
    pub root_key: u8,
    pub volume_pct: u8,
    pub pan: i8,
    /// Choke group (`0` = polyphonic/none, `1..=15` = mutually choking group).
    pub cut_group: u8,
    /// 1-based target mixer strip index.
    pub mixer_strip: u8,
    /// Enables portamento/legato glide for overlapping notes (e.g. Sub 808).
    pub legato: bool,
    /// Channel badge color in `0xRRGGBB`.
    pub color_rgb: u32,
}

impl ChannelTemplate {
    pub const fn new(
        name: &'static str,
        instrument: u8,
        root_key: u8,
        volume_pct: u8,
        pan: i8,
        cut_group: u8,
        mixer_strip: u8,
        legato: bool,
        color_rgb: u32,
    ) -> Self {
        Self {
            name,
            instrument,
            root_key,
            volume_pct,
            pan,
            cut_group,
            mixer_strip,
            legato,
            color_rgb,
        }
    }
}

/// Multi-channel kit specification populated in a single undo step in `Tapestry`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KitSpec {
    pub name: &'static str,
    pub genre: &'static str,
    pub description: &'static str,
    pub default_bpm: u16,
    pub channels: &'static [ChannelTemplate],
}

/// A single note event inside a song starter grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StarterStepNote {
    pub channel_idx: u8,
    pub step: u16,
    pub key: u8,
    pub velocity: u8,
    pub length_steps: u8,
    /// True when this note glides smoothly from a preceding overlapping note.
    pub is_slide: bool,
}

impl StarterStepNote {
    pub const fn hit(channel_idx: u8, step: u16, key: u8, velocity: u8) -> Self {
        Self {
            channel_idx,
            step,
            key,
            velocity,
            length_steps: 1,
            is_slide: false,
        }
    }

    pub const fn held(
        channel_idx: u8,
        step: u16,
        key: u8,
        velocity: u8,
        length_steps: u8,
    ) -> Self {
        Self {
            channel_idx,
            step,
            key,
            velocity,
            length_steps,
            is_slide: false,
        }
    }

    pub const fn slide(
        channel_idx: u8,
        step: u16,
        key: u8,
        velocity: u8,
        length_steps: u8,
    ) -> Self {
        Self {
            channel_idx,
            step,
            key,
            velocity,
            length_steps,
            is_slide: true,
        }
    }
}

/// Complete ready-to-play 1-bar or 2-bar multi-channel pattern starter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SongStarterSpec {
    pub name: &'static str,
    pub genre: &'static str,
    pub kit_name: Option<&'static str>,
    pub channels: &'static [ChannelTemplate],
    pub bpm: u16,
    pub time_signature: [u8; 2],
    pub bars: u8,
    pub steps_per_beat: u8,
    pub total_steps: u16,
    pub swing_pct: u8,
    pub grid_label: &'static str,
    pub notes: &'static [StarterStepNote],
}

// ============================================================================
// 1. Multi-Channel Kit Starters
// ============================================================================

const KIT_808_GLIDE_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new("Punch Kick", INSTR_VELVET_SAMPLER, 36, 105, 0, 0, 1, false, 0xe05d44),
    ChannelTemplate::new("Crisp Snare", INSTR_VELVET_SAMPLER, 38, 95, 0, 0, 2, false, 0xe69f38),
    ChannelTemplate::new("Layered Clap", INSTR_VELVET_SAMPLER, 39, 90, -6, 0, 3, false, 0xd8b848),
    ChannelTemplate::new(
        "Tight Closed Hat",
        INSTR_VELVET_SAMPLER,
        42,
        82,
        18,
        CUT_GROUP_HIHAT,
        4,
        false,
        0x48b5a3,
    ),
    ChannelTemplate::new(
        "Sizzle Open Hat",
        INSTR_VELVET_SAMPLER,
        46,
        80,
        22,
        CUT_GROUP_HIHAT,
        5,
        false,
        0x66c7b4,
    ),
    ChannelTemplate::new(
        "Sliding Sub 808",
        INSTR_SATIN_PULSE,
        36,
        100,
        0,
        CUT_GROUP_MONO,
        6,
        true,
        0x9b5de5,
    ),
];

const KIT_ACOUSTIC_STUDIO_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new("Modal Kick", INSTR_VELVET_SAMPLER, 36, 100, 0, 0, 1, false, 0xc96444),
    ChannelTemplate::new("Snare Wire", INSTR_VELVET_SAMPLER, 38, 96, -4, 0, 2, false, 0xd9923b),
    ChannelTemplate::new(
        "Closed Hat",
        INSTR_VELVET_SAMPLER,
        42,
        82,
        24,
        CUT_GROUP_HIHAT,
        3,
        false,
        0x52a898,
    ),
    ChannelTemplate::new(
        "Open Hat",
        INSTR_VELVET_SAMPLER,
        46,
        84,
        26,
        CUT_GROUP_HIHAT,
        4,
        false,
        0x72bfb0,
    ),
    ChannelTemplate::new("Rack Tom", INSTR_VELVET_SAMPLER, 48, 88, -22, 0, 5, false, 0x6c88c4),
    ChannelTemplate::new("Crash Cymbal", INSTR_VELVET_SAMPLER, 49, 78, -35, 0, 6, false, 0x9aaec9),
    ChannelTemplate::new("Picked Bass", INSTR_SATIN_PULSE, 40, 94, 0, 0, 7, false, 0x5c73b8),
    ChannelTemplate::new("Electric Guitar", INSTR_PRISM_WAVETABLE, 60, 86, 30, 0, 8, false, 0xb870a8),
];

const KIT_NEON_SYNTHWAVE_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new("Gated Kick", INSTR_VELVET_SAMPLER, 36, 102, 0, 0, 1, false, 0xe84393),
    ChannelTemplate::new("Gated Reverb Snare", INSTR_VELVET_SAMPLER, 38, 96, 0, 0, 2, false, 0xfd79a8),
    ChannelTemplate::new(
        "909 Hat",
        INSTR_VELVET_SAMPLER,
        42,
        80,
        20,
        CUT_GROUP_HIHAT,
        3,
        false,
        0x00cec9,
    ),
    ChannelTemplate::new("Driving Pulse Bass", INSTR_SATIN_PULSE, 36, 94, 0, 0, 4, false, 0x6c5ce7),
    ChannelTemplate::new("Lush Poly Pad", INSTR_PRISM_WAVETABLE, 60, 78, -14, 0, 5, false, 0x74b9ff),
    ChannelTemplate::new("Cross Lead", INSTR_SILK_TRIANGLE, 60, 85, 16, 0, 6, true, 0xfdcb6e),
];

const KIT_DUSTY_BOOMBAP_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new("Warm Vinyl Kick", INSTR_VELVET_SAMPLER, 36, 104, 0, 0, 1, false, 0xb86f42),
    ChannelTemplate::new("Rim Snare", INSTR_VELVET_SAMPLER, 37, 94, -5, 0, 2, false, 0xc99148),
    ChannelTemplate::new(
        "Dusty Hat",
        INSTR_VELVET_SAMPLER,
        42,
        78,
        18,
        CUT_GROUP_HIHAT,
        3,
        false,
        0x799c72,
    ),
    ChannelTemplate::new("Tine Keys", INSTR_SILK_TRIANGLE, 60, 84, -18, 0, 4, false, 0x5c8fa6),
    ChannelTemplate::new("Upright Sub Bass", INSTR_SATIN_PULSE, 36, 96, 0, 0, 5, false, 0x6e6896),
    ChannelTemplate::new(
        "Ariel Vocal Chop",
        INSTR_ARIEL_FORMANT,
        60,
        86,
        14,
        CUT_GROUP_MONO,
        6,
        false,
        0xc76b8f,
    ),
];

const KIT_DOUBLE_KICK_METAL_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new("Metal Kick", INSTR_VELVET_SAMPLER, 36, 104, 0, 0, 1, false, 0xc0392b),
    ChannelTemplate::new("Crack Snare", INSTR_VELVET_SAMPLER, 38, 98, 0, 0, 2, false, 0xd35400),
    ChannelTemplate::new("Ride Bell", INSTR_VELVET_SAMPLER, 53, 82, 28, 0, 3, false, 0x95a5a6),
    ChannelTemplate::new("Floor Tom", INSTR_VELVET_SAMPLER, 41, 90, -28, 0, 4, false, 0x7f8c8d),
    ChannelTemplate::new("Palm-Muted Guitar", INSTR_PRISM_WAVETABLE, 40, 92, -24, 0, 5, false, 0x8e44ad),
    ChannelTemplate::new("Driven Bass", INSTR_SATIN_PULSE, 40, 94, 0, 0, 6, false, 0x2c3e50),
];

const KIT_CLUB_HOUSE_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new(
        "Four-on-Floor 909 Kick",
        INSTR_VELVET_SAMPLER,
        36,
        105,
        0,
        0,
        1,
        false,
        0xd94f4f,
    ),
    ChannelTemplate::new(
        "Offbeat Open Hat",
        INSTR_VELVET_SAMPLER,
        46,
        84,
        20,
        CUT_GROUP_HIHAT,
        2,
        false,
        0x3fb8af,
    ),
    ChannelTemplate::new(
        "Closed Shuffle Hat",
        INSTR_VELVET_SAMPLER,
        42,
        78,
        16,
        CUT_GROUP_HIHAT,
        3,
        false,
        0x52c4bc,
    ),
    ChannelTemplate::new("House Clap", INSTR_VELVET_SAMPLER, 39, 92, -8, 0, 4, false, 0xe8a838),
    ChannelTemplate::new("Organ Tine Chord", INSTR_SILK_TRIANGLE, 60, 84, -12, 0, 5, false, 0x5c80d6),
    ChannelTemplate::new("Sub Bass", INSTR_SATIN_PULSE, 36, 98, 0, 0, 6, false, 0x7b5cd6),
];

const GLASS_CHOIR_CHANNELS: &[ChannelTemplate] = &[
    ChannelTemplate::new("Ariel Soprano Choir", INSTR_ARIEL_FORMANT, 64, 86, -12, 0, 1, false, 0x88b4e8),
    ChannelTemplate::new("Ariel Alto Harmony", INSTR_ARIEL_FORMANT, 60, 82, 12, 0, 2, false, 0x9ea3e6),
    ChannelTemplate::new("Glass Mallet", INSTR_SILK_TRIANGLE, 72, 88, 18, 0, 3, false, 0x6cd4c8),
    ChannelTemplate::new("Warm Sub Pedal", INSTR_SATIN_PULSE, 36, 92, 0, 0, 4, false, 0x5a6fa8),
    ChannelTemplate::new(
        "Brushed Click",
        INSTR_VELVET_SAMPLER,
        42,
        72,
        -16,
        CUT_GROUP_HIHAT,
        5,
        false,
        0xb8c4d4,
    ),
];

/// Complete catalog of multi-channel kit starters for `Tapestry`.
pub const ALL_KITS: &[KitSpec] = &[
    KitSpec {
        name: "808 Glide Kit",
        genre: "Trap",
        description: "Punchy drums with choked hat pair and legato Sliding Sub 808.",
        default_bpm: 140,
        channels: KIT_808_GLIDE_CHANNELS,
    },
    KitSpec {
        name: "Acoustic Studio Kit",
        genre: "Rock / Indie",
        description: "Live-room modal drums paired with picked bass and electric guitar.",
        default_bpm: 112,
        channels: KIT_ACOUSTIC_STUDIO_CHANNELS,
    },
    KitSpec {
        name: "Neon Synthwave Kit",
        genre: "Synthwave",
        description: "Gated drums, driving pulse bass, poly pad, and cross lead.",
        default_bpm: 108,
        channels: KIT_NEON_SYNTHWAVE_CHANNELS,
    },
    KitSpec {
        name: "Dusty Boom-Bap Kit",
        genre: "Boom-Bap",
        description: "Warm vinyl percussion, tine keys, upright sub, and Ariel chop.",
        default_bpm: 90,
        channels: KIT_DUSTY_BOOMBAP_CHANNELS,
    },
    KitSpec {
        name: "Double-Kick Metal Kit",
        genre: "Metal",
        description: "Triplet-ready metal kick, crack snare, ride bell, floor tom, and chug rig.",
        default_bpm: 150,
        channels: KIT_DOUBLE_KICK_METAL_CHANNELS,
    },
    KitSpec {
        name: "Club House Kit",
        genre: "House",
        description: "Four-on-floor 909 kick, choked shuffle/open hats, clap, organ, and sub.",
        default_bpm: 124,
        channels: KIT_CLUB_HOUSE_CHANNELS,
    },
];

/// Looks up a kit starter by exact name (case-sensitive).
pub fn find_kit(name: &str) -> Option<&'static KitSpec> {
    ALL_KITS.iter().find(|kit| kit.name == name)
}

// ============================================================================
// 2. Genre Song Starter Templates
// ============================================================================

/// 2 bars at 140 BPM on a 1/32 grid (64 steps total).
/// Features 1/32 closed-hat rolls and overlapping octave 808 slide notes at bar ends.
const NOTES_NIGHT_SLIDE: &[StarterStepNote] = &[
    // Punch Kick (ch 0)
    StarterStepNote::hit(0, 0, 36, 122),
    StarterStepNote::hit(0, 12, 36, 114),
    StarterStepNote::hit(0, 22, 36, 112),
    StarterStepNote::hit(0, 32, 36, 122),
    StarterStepNote::hit(0, 44, 36, 116),
    StarterStepNote::hit(0, 52, 36, 110),
    // Crisp Snare (ch 1) & Layered Clap (ch 2) on beat 3 of each bar
    StarterStepNote::hit(1, 16, 38, 118),
    StarterStepNote::hit(1, 48, 38, 120),
    StarterStepNote::hit(2, 16, 39, 108),
    StarterStepNote::hit(2, 48, 39, 110),
    StarterStepNote::hit(2, 60, 39, 86),
    // Tight Closed Hat (ch 3): steady 1/8 pulse + 1/32 rolls at steps 28..31 and 56..63
    StarterStepNote::hit(3, 0, 42, 98),
    StarterStepNote::hit(3, 4, 42, 86),
    StarterStepNote::hit(3, 8, 42, 96),
    StarterStepNote::hit(3, 12, 42, 84),
    StarterStepNote::hit(3, 16, 42, 98),
    StarterStepNote::hit(3, 20, 42, 88),
    StarterStepNote::hit(3, 24, 42, 96),
    StarterStepNote::hit(3, 28, 42, 74),
    StarterStepNote::hit(3, 29, 42, 84),
    StarterStepNote::hit(3, 30, 42, 94),
    StarterStepNote::hit(3, 31, 42, 104),
    StarterStepNote::hit(3, 32, 42, 98),
    StarterStepNote::hit(3, 36, 42, 86),
    StarterStepNote::hit(3, 40, 42, 96),
    StarterStepNote::hit(3, 44, 42, 84),
    StarterStepNote::hit(3, 48, 42, 98),
    StarterStepNote::hit(3, 52, 42, 88),
    StarterStepNote::hit(3, 56, 42, 72),
    StarterStepNote::hit(3, 57, 42, 78),
    StarterStepNote::hit(3, 58, 42, 84),
    StarterStepNote::hit(3, 59, 42, 90),
    StarterStepNote::hit(3, 60, 42, 96),
    StarterStepNote::hit(3, 61, 42, 100),
    StarterStepNote::hit(3, 62, 42, 106),
    StarterStepNote::hit(3, 63, 42, 112),
    // Sizzle Open Hat (ch 4) choked by Closed Hat via cut_group 1
    StarterStepNote::hit(4, 14, 46, 92),
    StarterStepNote::hit(4, 46, 46, 94),
    // Sliding Sub 808 (ch 5): overlapping root + octave glide notes at bar ends
    StarterStepNote::held(5, 0, 29, 118, 12),
    StarterStepNote::held(5, 12, 29, 112, 10),
    StarterStepNote::held(5, 22, 36, 114, 10),
    StarterStepNote::slide(5, 28, 48, 108, 4), // overlaps step 22..32 with octave glide C2->C3
    StarterStepNote::held(5, 32, 29, 118, 12),
    StarterStepNote::held(5, 44, 37, 114, 8),
    StarterStepNote::held(5, 52, 36, 114, 12),
    StarterStepNote::slide(5, 58, 48, 110, 4), // overlaps step 52..64 with octave glide C2->C3
    StarterStepNote::slide(5, 62, 41, 104, 2), // trailing glide into F2 turnaround
];

/// 2 bars at 108 BPM on a 1/16 grid (32 steps total).
const NOTES_AFTERGLOW: &[StarterStepNote] = &[
    // Gated Kick (ch 0)
    StarterStepNote::hit(0, 0, 36, 118),
    StarterStepNote::hit(0, 4, 36, 114),
    StarterStepNote::hit(0, 8, 36, 118),
    StarterStepNote::hit(0, 12, 36, 114),
    StarterStepNote::hit(0, 16, 36, 118),
    StarterStepNote::hit(0, 20, 36, 114),
    StarterStepNote::hit(0, 24, 36, 118),
    StarterStepNote::hit(0, 28, 36, 114),
    // Gated Reverb Snare (ch 1)
    StarterStepNote::hit(1, 4, 38, 118),
    StarterStepNote::hit(1, 12, 38, 118),
    StarterStepNote::hit(1, 20, 38, 118),
    StarterStepNote::hit(1, 28, 38, 120),
    // 909 Hat (ch 2)
    StarterStepNote::hit(2, 2, 42, 92),
    StarterStepNote::hit(2, 6, 42, 92),
    StarterStepNote::hit(2, 10, 42, 92),
    StarterStepNote::hit(2, 14, 42, 94),
    StarterStepNote::hit(2, 18, 42, 92),
    StarterStepNote::hit(2, 22, 42, 92),
    StarterStepNote::hit(2, 26, 42, 92),
    StarterStepNote::hit(2, 30, 42, 98),
    // Driving Pulse Bass (ch 3): driving 8th-note Am -> F -> G -> Em
    StarterStepNote::held(3, 0, 33, 108, 2),
    StarterStepNote::held(3, 2, 33, 98, 2),
    StarterStepNote::held(3, 4, 33, 106, 2),
    StarterStepNote::held(3, 6, 45, 96, 2),
    StarterStepNote::held(3, 8, 29, 108, 2),
    StarterStepNote::held(3, 10, 29, 98, 2),
    StarterStepNote::held(3, 12, 29, 106, 2),
    StarterStepNote::held(3, 14, 41, 96, 2),
    StarterStepNote::held(3, 16, 31, 108, 2),
    StarterStepNote::held(3, 18, 31, 98, 2),
    StarterStepNote::held(3, 20, 31, 106, 2),
    StarterStepNote::held(3, 22, 43, 96, 2),
    StarterStepNote::held(3, 24, 28, 108, 2),
    StarterStepNote::held(3, 26, 28, 98, 2),
    StarterStepNote::held(3, 28, 31, 106, 2),
    StarterStepNote::held(3, 30, 40, 98, 2),
    // Lush Poly Pad (ch 4)
    StarterStepNote::held(4, 0, 60, 86, 8),
    StarterStepNote::held(4, 0, 64, 84, 8),
    StarterStepNote::held(4, 8, 57, 86, 8),
    StarterStepNote::held(4, 8, 60, 84, 8),
    StarterStepNote::held(4, 16, 59, 86, 8),
    StarterStepNote::held(4, 16, 62, 84, 8),
    StarterStepNote::held(4, 24, 55, 86, 8),
    StarterStepNote::held(4, 24, 59, 84, 8),
    // Cross Lead (ch 5)
    StarterStepNote::held(5, 0, 72, 100, 3),
    StarterStepNote::held(5, 3, 76, 96, 3),
    StarterStepNote::held(5, 6, 79, 102, 2),
    StarterStepNote::held(5, 8, 76, 98, 4),
    StarterStepNote::held(5, 16, 74, 100, 3),
    StarterStepNote::held(5, 19, 79, 96, 3),
    StarterStepNote::held(5, 22, 83, 104, 2),
    StarterStepNote::held(5, 24, 79, 98, 6),
];

/// 2 bars at 90 BPM on a 1/16 grid (32 steps total) with 22% swing.
const NOTES_DUSTY_CRATE: &[StarterStepNote] = &[
    // Warm Vinyl Kick (ch 0)
    StarterStepNote::hit(0, 0, 36, 118),
    StarterStepNote::hit(0, 3, 36, 94),
    StarterStepNote::hit(0, 7, 36, 104),
    StarterStepNote::hit(0, 10, 36, 112),
    StarterStepNote::hit(0, 16, 36, 118),
    StarterStepNote::hit(0, 19, 36, 96),
    StarterStepNote::hit(0, 23, 36, 106),
    StarterStepNote::hit(0, 26, 36, 112),
    StarterStepNote::hit(0, 30, 36, 92),
    // Rim Snare (ch 1)
    StarterStepNote::hit(1, 4, 37, 114),
    StarterStepNote::hit(1, 12, 37, 116),
    StarterStepNote::hit(1, 15, 37, 68),
    StarterStepNote::hit(1, 20, 37, 114),
    StarterStepNote::hit(1, 28, 37, 116),
    StarterStepNote::hit(1, 31, 37, 72),
    // Dusty Hat (ch 2)
    StarterStepNote::hit(2, 0, 42, 92),
    StarterStepNote::hit(2, 2, 42, 74),
    StarterStepNote::hit(2, 4, 42, 90),
    StarterStepNote::hit(2, 6, 42, 72),
    StarterStepNote::hit(2, 8, 42, 92),
    StarterStepNote::hit(2, 10, 42, 74),
    StarterStepNote::hit(2, 12, 42, 90),
    StarterStepNote::hit(2, 14, 42, 76),
    StarterStepNote::hit(2, 16, 42, 92),
    StarterStepNote::hit(2, 18, 42, 74),
    StarterStepNote::hit(2, 20, 42, 90),
    StarterStepNote::hit(2, 22, 42, 72),
    StarterStepNote::hit(2, 24, 42, 92),
    StarterStepNote::hit(2, 26, 42, 74),
    StarterStepNote::hit(2, 28, 42, 90),
    StarterStepNote::hit(2, 30, 42, 78),
    // Tine Keys (ch 3): Dm9 -> Gm7
    StarterStepNote::held(3, 0, 62, 88, 6),
    StarterStepNote::held(3, 0, 65, 84, 6),
    StarterStepNote::held(3, 0, 69, 86, 6),
    StarterStepNote::held(3, 10, 64, 82, 5),
    StarterStepNote::held(3, 16, 55, 88, 6),
    StarterStepNote::held(3, 16, 62, 84, 6),
    StarterStepNote::held(3, 16, 65, 86, 6),
    StarterStepNote::held(3, 26, 60, 82, 5),
    // Upright Sub Bass (ch 4)
    StarterStepNote::held(4, 0, 38, 108, 6),
    StarterStepNote::held(4, 7, 38, 96, 3),
    StarterStepNote::held(4, 10, 45, 104, 5),
    StarterStepNote::held(4, 16, 31, 108, 6),
    StarterStepNote::held(4, 23, 33, 98, 3),
    StarterStepNote::held(4, 26, 36, 104, 5),
    // Ariel Vocal Chop (ch 5)
    StarterStepNote::held(5, 6, 69, 96, 3),
    StarterStepNote::held(5, 14, 72, 92, 2),
    StarterStepNote::held(5, 22, 67, 96, 3),
    StarterStepNote::held(5, 29, 69, 90, 3),
];

/// 1 bar at 150 BPM on a 1/16T triplet grid (24 steps total, 6 steps/beat).
const NOTES_IRON_PULSE: &[StarterStepNote] = &[
    // Metal Kick (ch 0): rapid 1/16T double-kick gallops
    StarterStepNote::hit(0, 0, 36, 122),
    StarterStepNote::hit(0, 1, 36, 114),
    StarterStepNote::hit(0, 2, 36, 118),
    StarterStepNote::hit(0, 3, 36, 112),
    StarterStepNote::hit(0, 4, 36, 116),
    StarterStepNote::hit(0, 5, 36, 110),
    StarterStepNote::hit(0, 6, 36, 120),
    StarterStepNote::hit(0, 8, 36, 114),
    StarterStepNote::hit(0, 12, 36, 122),
    StarterStepNote::hit(0, 13, 36, 114),
    StarterStepNote::hit(0, 14, 36, 118),
    StarterStepNote::hit(0, 18, 36, 120),
    StarterStepNote::hit(0, 19, 36, 112),
    StarterStepNote::hit(0, 20, 36, 116),
    StarterStepNote::hit(0, 21, 36, 114),
    StarterStepNote::hit(0, 22, 36, 118),
    StarterStepNote::hit(0, 23, 36, 120),
    // Crack Snare (ch 1) on beats 2 & 4
    StarterStepNote::hit(1, 6, 38, 124),
    StarterStepNote::hit(1, 18, 38, 124),
    // Ride Bell (ch 2)
    StarterStepNote::hit(2, 0, 53, 102),
    StarterStepNote::hit(2, 3, 53, 92),
    StarterStepNote::hit(2, 6, 53, 102),
    StarterStepNote::hit(2, 9, 53, 92),
    StarterStepNote::hit(2, 12, 53, 102),
    StarterStepNote::hit(2, 15, 53, 92),
    StarterStepNote::hit(2, 18, 53, 102),
    // Floor Tom (ch 3) triplet fill at bar end
    StarterStepNote::hit(3, 21, 41, 110),
    StarterStepNote::hit(3, 22, 41, 114),
    StarterStepNote::hit(3, 23, 41, 118),
    // Palm-Muted Guitar (ch 4)
    StarterStepNote::held(4, 0, 40, 116, 2),
    StarterStepNote::held(4, 3, 40, 110, 2),
    StarterStepNote::held(4, 6, 43, 114, 2),
    StarterStepNote::held(4, 9, 40, 108, 2),
    StarterStepNote::held(4, 12, 40, 116, 2),
    StarterStepNote::held(4, 15, 45, 112, 2),
    StarterStepNote::held(4, 18, 43, 114, 3),
    // Driven Bass (ch 5)
    StarterStepNote::held(5, 0, 28, 116, 2),
    StarterStepNote::held(5, 3, 28, 110, 2),
    StarterStepNote::held(5, 6, 31, 114, 2),
    StarterStepNote::held(5, 12, 28, 116, 2),
    StarterStepNote::held(5, 15, 33, 112, 2),
    StarterStepNote::held(5, 18, 31, 114, 3),
];

/// 2 bars at 124 BPM on a 1/16 grid (32 steps total).
const NOTES_VELVET_CLUB: &[StarterStepNote] = &[
    // Four-on-Floor 909 Kick (ch 0)
    StarterStepNote::hit(0, 0, 36, 122),
    StarterStepNote::hit(0, 4, 36, 120),
    StarterStepNote::hit(0, 8, 36, 122),
    StarterStepNote::hit(0, 12, 36, 120),
    StarterStepNote::hit(0, 16, 36, 122),
    StarterStepNote::hit(0, 20, 36, 120),
    StarterStepNote::hit(0, 24, 36, 122),
    StarterStepNote::hit(0, 28, 36, 120),
    // Offbeat Open Hat (ch 1) + Closed Shuffle Hat (ch 2) sharing cut_group 1
    StarterStepNote::hit(1, 2, 46, 102),
    StarterStepNote::hit(2, 3, 42, 78),
    StarterStepNote::hit(1, 6, 46, 102),
    StarterStepNote::hit(2, 7, 42, 78),
    StarterStepNote::hit(1, 10, 46, 102),
    StarterStepNote::hit(2, 11, 42, 80),
    StarterStepNote::hit(1, 14, 46, 104),
    StarterStepNote::hit(2, 15, 42, 82),
    StarterStepNote::hit(1, 18, 46, 102),
    StarterStepNote::hit(2, 19, 42, 78),
    StarterStepNote::hit(1, 22, 46, 102),
    StarterStepNote::hit(2, 23, 42, 78),
    StarterStepNote::hit(1, 26, 46, 102),
    StarterStepNote::hit(2, 27, 42, 80),
    StarterStepNote::hit(1, 30, 46, 104),
    StarterStepNote::hit(2, 31, 42, 84),
    // House Clap (ch 3)
    StarterStepNote::hit(3, 4, 39, 114),
    StarterStepNote::hit(3, 12, 39, 114),
    StarterStepNote::hit(3, 20, 39, 114),
    StarterStepNote::hit(3, 28, 39, 116),
    // Organ Tine Chord (ch 4)
    StarterStepNote::held(4, 3, 60, 96, 2),
    StarterStepNote::held(4, 3, 63, 94, 2),
    StarterStepNote::held(4, 6, 67, 94, 2),
    StarterStepNote::held(4, 14, 65, 96, 2),
    StarterStepNote::held(4, 19, 60, 96, 2),
    StarterStepNote::held(4, 19, 63, 94, 2),
    StarterStepNote::held(4, 26, 62, 92, 2),
    StarterStepNote::held(4, 30, 67, 98, 2),
    // Sub Bass (ch 5)
    StarterStepNote::held(5, 0, 36, 112, 2),
    StarterStepNote::held(5, 3, 36, 104, 2),
    StarterStepNote::held(5, 6, 43, 108, 2),
    StarterStepNote::held(5, 10, 41, 106, 2),
    StarterStepNote::held(5, 14, 39, 108, 2),
    StarterStepNote::held(5, 16, 36, 112, 2),
    StarterStepNote::held(5, 19, 36, 104, 2),
    StarterStepNote::held(5, 22, 43, 108, 2),
    StarterStepNote::held(5, 26, 41, 106, 2),
    StarterStepNote::held(5, 30, 43, 110, 2),
];

/// 2 bars at 116 BPM on a 1/16 grid (32 steps total).
const NOTES_GLASS_CHOIR: &[StarterStepNote] = &[
    // Ariel Soprano Choir (ch 0)
    StarterStepNote::held(0, 0, 64, 92, 16),
    StarterStepNote::held(0, 16, 67, 94, 16),
    // Ariel Alto Harmony (ch 1)
    StarterStepNote::held(1, 0, 60, 88, 16),
    StarterStepNote::held(1, 16, 62, 90, 16),
    // Glass Mallet (ch 2): luminous arpeggio
    StarterStepNote::held(2, 0, 72, 96, 2),
    StarterStepNote::held(2, 2, 76, 88, 2),
    StarterStepNote::held(2, 4, 79, 94, 2),
    StarterStepNote::held(2, 6, 84, 90, 2),
    StarterStepNote::held(2, 8, 79, 92, 2),
    StarterStepNote::held(2, 10, 76, 86, 2),
    StarterStepNote::held(2, 12, 72, 92, 2),
    StarterStepNote::held(2, 14, 74, 88, 2),
    StarterStepNote::held(2, 16, 74, 96, 2),
    StarterStepNote::held(2, 18, 79, 88, 2),
    StarterStepNote::held(2, 20, 83, 94, 2),
    StarterStepNote::held(2, 22, 86, 92, 2),
    StarterStepNote::held(2, 24, 83, 92, 2),
    StarterStepNote::held(2, 26, 79, 88, 2),
    StarterStepNote::held(2, 28, 74, 90, 2),
    StarterStepNote::held(2, 30, 79, 94, 2),
    // Warm Sub Pedal (ch 3)
    StarterStepNote::held(3, 0, 36, 104, 16),
    StarterStepNote::held(3, 16, 43, 104, 16),
    // Brushed Click (ch 4)
    StarterStepNote::hit(4, 2, 42, 70),
    StarterStepNote::hit(4, 6, 42, 68),
    StarterStepNote::hit(4, 10, 42, 72),
    StarterStepNote::hit(4, 14, 42, 76),
    StarterStepNote::hit(4, 18, 42, 70),
    StarterStepNote::hit(4, 22, 42, 68),
    StarterStepNote::hit(4, 26, 42, 72),
    StarterStepNote::hit(4, 30, 42, 78),
];

/// Complete catalog of genre song starter templates for `Tapestry`.
pub const ALL_SONG_STARTERS: &[SongStarterSpec] = &[
    SongStarterSpec {
        name: "Night Slide (Trap 140)",
        genre: "Trap",
        kit_name: Some("808 Glide Kit"),
        channels: KIT_808_GLIDE_CHANNELS,
        bpm: 140,
        time_signature: [4, 4],
        bars: 2,
        steps_per_beat: 8,
        total_steps: 64,
        swing_pct: 0,
        grid_label: "1/32",
        notes: NOTES_NIGHT_SLIDE,
    },
    SongStarterSpec {
        name: "Afterglow (Synthwave 108)",
        genre: "Synthwave",
        kit_name: Some("Neon Synthwave Kit"),
        channels: KIT_NEON_SYNTHWAVE_CHANNELS,
        bpm: 108,
        time_signature: [4, 4],
        bars: 2,
        steps_per_beat: 4,
        total_steps: 32,
        swing_pct: 0,
        grid_label: "1/16",
        notes: NOTES_AFTERGLOW,
    },
    SongStarterSpec {
        name: "Dusty Crate (Boom-Bap 90, 22% Swing)",
        genre: "Boom-Bap",
        kit_name: Some("Dusty Boom-Bap Kit"),
        channels: KIT_DUSTY_BOOMBAP_CHANNELS,
        bpm: 90,
        time_signature: [4, 4],
        bars: 2,
        steps_per_beat: 4,
        total_steps: 32,
        swing_pct: 22,
        grid_label: "1/16",
        notes: NOTES_DUSTY_CRATE,
    },
    SongStarterSpec {
        name: "Iron Pulse (Double-Kick 150, 1/16T)",
        genre: "Metal",
        kit_name: Some("Double-Kick Metal Kit"),
        channels: KIT_DOUBLE_KICK_METAL_CHANNELS,
        bpm: 150,
        time_signature: [4, 4],
        bars: 1,
        steps_per_beat: 6,
        total_steps: 24,
        swing_pct: 0,
        grid_label: "1/16T",
        notes: NOTES_IRON_PULSE,
    },
    SongStarterSpec {
        name: "Velvet Club (House 124)",
        genre: "House",
        kit_name: Some("Club House Kit"),
        channels: KIT_CLUB_HOUSE_CHANNELS,
        bpm: 124,
        time_signature: [4, 4],
        bars: 2,
        steps_per_beat: 4,
        total_steps: 32,
        swing_pct: 0,
        grid_label: "1/16",
        notes: NOTES_VELVET_CLUB,
    },
    SongStarterSpec {
        name: "Glass Choir (Ariel & Mallet 116)",
        genre: "Ambient Choral",
        kit_name: None,
        channels: GLASS_CHOIR_CHANNELS,
        bpm: 116,
        time_signature: [4, 4],
        bars: 2,
        steps_per_beat: 4,
        total_steps: 32,
        swing_pct: 0,
        grid_label: "1/16",
        notes: NOTES_GLASS_CHOIR,
    },
];

/// Looks up a song starter template by exact name.
pub fn find_song_starter(name: &str) -> Option<&'static SongStarterSpec> {
    ALL_SONG_STARTERS.iter().find(|starter| starter.name == name)
}

// ============================================================================
// 3. Beat-Color Palette
// ============================================================================

/// Alternating 4-beat step button colors (`0xRRGGBB`) so bars read at a glance in `Tapestry`.
pub const STEP_BEAT_COLORS: [u32; 4] = [
    0x2d3e54, // Beat 1: cool slate
    0x3f3247, // Beat 2: warm plum
    0x28414f, // Beat 3: deep teal-slate
    0x3b3040, // Beat 4: muted wine
];

/// Returns the beat-group background color (`0xRRGGBB`) for `step_idx` given `steps_per_beat`.
/// Falls back to 4 steps per beat if `steps_per_beat == 0`.
pub const fn step_group_color(step_idx: usize, steps_per_beat: u8) -> u32 {
    let group_size = if steps_per_beat == 0 {
        4
    } else {
        steps_per_beat as usize
    };
    let beat_idx = step_idx / group_size;
    STEP_BEAT_COLORS[beat_idx % STEP_BEAT_COLORS.len()]
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_kits_present_and_discoverable() {
        let expected = [
            "808 Glide Kit",
            "Acoustic Studio Kit",
            "Neon Synthwave Kit",
            "Dusty Boom-Bap Kit",
            "Double-Kick Metal Kit",
            "Club House Kit",
        ];
        assert_eq!(ALL_KITS.len(), expected.len());
        for name in expected {
            let kit = find_kit(name).expect("missing expected kit");
            assert_eq!(kit.name, name);
            assert!(kit.channels.len() >= 6, "kit {} should have >= 6 channels", name);
            for (idx, ch) in kit.channels.iter().enumerate() {
                assert!(ch.instrument <= INSTR_PRISM_WAVETABLE);
                assert!(ch.root_key <= 127);
                assert!(ch.volume_pct > 0 && ch.volume_pct <= 150);
                assert_eq!(ch.mixer_strip as usize, idx + 1);
            }
        }
        assert!(find_kit("Nonexistent Kit").is_none());
    }

    #[test]
    fn hihat_choke_groups_and_legato_configured() {
        let glide = find_kit("808 Glide Kit").unwrap();
        let closed = glide.channels.iter().find(|c| c.name == "Tight Closed Hat").unwrap();
        let open = glide.channels.iter().find(|c| c.name == "Sizzle Open Hat").unwrap();
        assert_eq!(closed.cut_group, 1);
        assert_eq!(open.cut_group, 1);
        let sub = glide.channels.iter().find(|c| c.name == "Sliding Sub 808").unwrap();
        assert!(sub.legato);

        let acoustic = find_kit("Acoustic Studio Kit").unwrap();
        let ac_closed = acoustic.channels.iter().find(|c| c.name == "Closed Hat").unwrap();
        let ac_open = acoustic.channels.iter().find(|c| c.name == "Open Hat").unwrap();
        assert_eq!(ac_closed.cut_group, 1);
        assert_eq!(ac_open.cut_group, 1);

        let synth = find_kit("Neon Synthwave Kit").unwrap();
        assert_eq!(
            synth.channels.iter().find(|c| c.name == "909 Hat").unwrap().cut_group,
            1
        );

        let dusty = find_kit("Dusty Boom-Bap Kit").unwrap();
        assert_eq!(
            dusty.channels.iter().find(|c| c.name == "Dusty Hat").unwrap().cut_group,
            1
        );

        let house = find_kit("Club House Kit").unwrap();
        let h_open = house.channels.iter().find(|c| c.name == "Offbeat Open Hat").unwrap();
        let h_closed = house.channels.iter().find(|c| c.name == "Closed Shuffle Hat").unwrap();
        assert_eq!(h_open.cut_group, 1);
        assert_eq!(h_closed.cut_group, 1);
    }

    #[test]
    fn all_six_song_starters_valid_grids_and_bounds() {
        let expected = [
            "Night Slide (Trap 140)",
            "Afterglow (Synthwave 108)",
            "Dusty Crate (Boom-Bap 90, 22% Swing)",
            "Iron Pulse (Double-Kick 150, 1/16T)",
            "Velvet Club (House 124)",
            "Glass Choir (Ariel & Mallet 116)",
        ];
        assert_eq!(ALL_SONG_STARTERS.len(), expected.len());
        for name in expected {
            let starter = find_song_starter(name).expect("missing song starter");
            assert_eq!(starter.name, name);
            assert_eq!(
                starter.total_steps,
                starter.bars as u16 * starter.time_signature[0] as u16 * starter.steps_per_beat as u16
            );
            assert!(!starter.notes.is_empty());
            for note in starter.notes {
                assert!(
                    (note.channel_idx as usize) < starter.channels.len(),
                    "channel_idx out of bounds in {}",
                    name
                );
                assert!(note.step < starter.total_steps, "step out of bounds in {}", name);
                assert!(note.key <= 127);
                assert!(note.velocity >= 1 && note.velocity <= 127);
                assert!(note.length_steps >= 1);
            }
        }
    }

    #[test]
    fn night_slide_has_1_32_hat_rolls_and_overlapping_808_slides() {
        let night = find_song_starter("Night Slide (Trap 140)").unwrap();
        assert_eq!(night.bpm, 140);
        assert_eq!(night.steps_per_beat, 8);
        assert_eq!(night.grid_label, "1/32");
        assert_eq!(night.total_steps, 64);

        // Consecutive 1/32 closed-hat roll steps (channel 3)
        let hat_steps: alloc::vec::Vec<u16> = night
            .notes
            .iter()
            .filter(|n| n.channel_idx == 3)
            .map(|n| n.step)
            .collect();
        assert!(hat_steps.windows(4).any(|w| w[1] == w[0] + 1 && w[2] == w[1] + 1 && w[3] == w[2] + 1));

        // Overlapping 808 slide notes (channel 5)
        let slides: alloc::vec::Vec<&StarterStepNote> = night
            .notes
            .iter()
            .filter(|n| n.channel_idx == 5 && n.is_slide)
            .collect();
        assert!(slides.len() >= 2);
        for s in slides {
            let overlaps_base = night.notes.iter().any(|base| {
                base.channel_idx == 5
                    && !base.is_slide
                    && base.step < s.step
                    && base.step + base.length_steps as u16 > s.step
            });
            assert!(overlaps_base, "slide note at step {} should overlap a sustained 808 note", s.step);
        }
    }

    #[test]
    fn boom_bap_swing_and_double_kick_triplet_grid() {
        let dusty = find_song_starter("Dusty Crate (Boom-Bap 90, 22% Swing)").unwrap();
        assert_eq!(dusty.bpm, 90);
        assert_eq!(dusty.swing_pct, 22);

        let iron = find_song_starter("Iron Pulse (Double-Kick 150, 1/16T)").unwrap();
        assert_eq!(iron.bpm, 150);
        assert_eq!(iron.steps_per_beat, 6);
        assert_eq!(iron.grid_label, "1/16T");
        assert_eq!(iron.total_steps, 24);
    }

    #[test]
    fn beat_color_palette_alternates_every_beat_group() {
        assert_ne!(STEP_BEAT_COLORS[0], STEP_BEAT_COLORS[1]);
        assert_ne!(STEP_BEAT_COLORS[2], STEP_BEAT_COLORS[3]);

        // Standard 4-step beat groups
        for s in 0..4 {
            assert_eq!(step_group_color(s, 4), STEP_BEAT_COLORS[0]);
        }
        for s in 4..8 {
            assert_eq!(step_group_color(s, 4), STEP_BEAT_COLORS[1]);
        }
        for s in 8..12 {
            assert_eq!(step_group_color(s, 4), STEP_BEAT_COLORS[2]);
        }
        for s in 12..16 {
            assert_eq!(step_group_color(s, 4), STEP_BEAT_COLORS[3]);
        }
        assert_eq!(step_group_color(16, 4), STEP_BEAT_COLORS[0]);

        // Triplet 6-step beat groups & zero-guard fallback
        assert_eq!(step_group_color(5, 6), STEP_BEAT_COLORS[0]);
        assert_eq!(step_group_color(6, 6), STEP_BEAT_COLORS[1]);
        assert_eq!(step_group_color(4, 0), STEP_BEAT_COLORS[1]);
    }
}
