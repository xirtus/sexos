// SPDX-License-Identifier: GPL-3.0-only
//! Module metadata browser catalog for EuropaRack.

extern crate alloc;
use alloc::vec::Vec;

use crate::modules::ModuleKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ModuleCategory {
    AudioIo = 0,
    Midi = 1,
    Oscillator = 2,
    Filter = 3,
    Amplifier = 4,
    Modulator = 5,
    Sequencer = 6,
    Utility = 7,
    Effect = 8,
    Host = 9,
}

#[derive(Clone, Copy, Debug)]
pub struct ModuleMetadata {
    pub kind: ModuleKind,
    pub name: &'static str,
    pub category: ModuleCategory,
    pub description: &'static str,
    pub tags: &'static [&'static str],
    pub hp_width: u8,
    pub inputs_count: usize,
    pub outputs_count: usize,
}

pub const CATALOG: [ModuleMetadata; 19] = [
    ModuleMetadata {
        kind: ModuleKind::TrackIn,
        name: "Track In",
        category: ModuleCategory::AudioIo,
        description: "Injects Moan track stereo PCM audio into EuropaRack",
        tags: &["audio", "io", "input", "track"],
        hp_width: 4,
        inputs_count: 0,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::MasterOut,
        name: "Master Out",
        category: ModuleCategory::AudioIo,
        description: "Stereo output with soft-clip limiter and peak meter",
        tags: &["audio", "io", "output", "master", "limiter", "meter"],
        hp_width: 6,
        inputs_count: 1,
        outputs_count: 0,
    },
    ModuleMetadata {
        kind: ModuleKind::PolyMidiToCv,
        name: "Poly MIDI-to-CV",
        category: ModuleCategory::Midi,
        description: "1..16 voice polyphonic allocator: MIDI notes -> Pitch CV, Gate, Velocity, Retrigger",
        tags: &["midi", "poly", "pitch", "gate", "velocity", "allocator"],
        hp_width: 8,
        inputs_count: 1,
        outputs_count: 4,
    },
    ModuleMetadata {
        kind: ModuleKind::AnalogVco,
        name: "Analog VCO",
        category: ModuleCategory::Oscillator,
        description: "PolyBLEP anti-aliased Saw, Pulse w/ PWM, Triangle, Sine, Lin FM and Hard Sync",
        tags: &["vco", "analog", "saw", "pulse", "pwm", "triangle", "sine", "fm", "polyblep"],
        hp_width: 12,
        inputs_count: 4,
        outputs_count: 4,
    },
    ModuleMetadata {
        kind: ModuleKind::Fm4OpMacro,
        name: "4-Op FM Synth",
        category: ModuleCategory::Oscillator,
        description: "4-operator phase-modulation synth voice with ratio, index, and feedback CVs",
        tags: &["fm", "pm", "4-op", "dx7", "feedback", "carrier", "modulator"],
        hp_width: 14,
        inputs_count: 4,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::ArielFormantCv,
        name: "Ariel Formant Vocal",
        category: ModuleCategory::Oscillator,
        description: "Klattsch / Ariel Eurorack vocal module with Vowel X/Y morphing, Effort, Breath, and ARPABET stepping",
        tags: &["vocal", "ariel", "klattsch", "formant", "vowel", "arpabet", "speech", "choir"],
        hp_width: 16,
        inputs_count: 6,
        outputs_count: 2,
    },
    ModuleMetadata {
        kind: ModuleKind::LadderVcf,
        name: "Moog Ladder VCF",
        category: ModuleCategory::Filter,
        description: "4-pole 24dB/oct resonant lowpass ladder filter with drive and CV attenuverters",
        tags: &["vcf", "filter", "ladder", "moog", "24db", "resonance", "drive"],
        hp_width: 10,
        inputs_count: 3,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::StateVariableVcf,
        name: "State Variable VCF",
        category: ModuleCategory::Filter,
        description: "12dB/oct multi-mode filter with simultaneous LP, BP, HP, and Notch outputs",
        tags: &["vcf", "filter", "svf", "multimode", "lowpass", "bandpass", "highpass", "notch"],
        hp_width: 10,
        inputs_count: 3,
        outputs_count: 4,
    },
    ModuleMetadata {
        kind: ModuleKind::PolyVca,
        name: "Poly VCA",
        category: ModuleCategory::Amplifier,
        description: "16-channel linear/exponential voltage controlled amplifier",
        tags: &["vca", "amp", "poly", "linear", "exponential"],
        hp_width: 6,
        inputs_count: 2,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::Wavefolder,
        name: "Buchla Wavefolder",
        category: ModuleCategory::Amplifier,
        description: "West-coast Buchla-style multi-stage wavefolder with asymmetry bias",
        tags: &["wavefolder", "buchla", "west-coast", "harmonics", "distortion"],
        hp_width: 8,
        inputs_count: 3,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::PolyAdsr,
        name: "Poly ADSR",
        category: ModuleCategory::Modulator,
        description: "16-channel Attack/Decay/Sustain/Release envelope generator",
        tags: &["adsr", "envelope", "poly", "modulator"],
        hp_width: 8,
        inputs_count: 2,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::TempoLfo,
        name: "Tempo LFO",
        category: ModuleCategory::Modulator,
        description: "BPM-synced or free-running Sine, Triangle, Saw, Square, and S&H LFO",
        tags: &["lfo", "tempo", "bpm", "sync", "modulation", "random"],
        hp_width: 8,
        inputs_count: 2,
        outputs_count: 5,
    },
    ModuleMetadata {
        kind: ModuleKind::ClockDivider,
        name: "Clock Divider / Mult",
        category: ModuleCategory::Sequencer,
        description: "Master BPM clock generator with /2, /4, /8, *2, *4 gate outputs",
        tags: &["clock", "divider", "multiplier", "bpm", "timing", "rhythm"],
        hp_width: 8,
        inputs_count: 2,
        outputs_count: 6,
    },
    ModuleMetadata {
        kind: ModuleKind::EuclideanSeq,
        name: "Euclidean Sequencer",
        category: ModuleCategory::Sequencer,
        description: "Bjorklund Euclidean rhythm generator with 16-step pitch CV sequence",
        tags: &["euclidean", "sequencer", "bjorklund", "rhythm", "gate", "pitch"],
        hp_width: 12,
        inputs_count: 4,
        outputs_count: 2,
    },
    ModuleMetadata {
        kind: ModuleKind::TuringQuantizer,
        name: "Turing Machine Quantizer",
        category: ModuleCategory::Sequencer,
        description: "Generative shift-register melody generator with Lock/Prob and musical scale quantizer",
        tags: &["turing", "generative", "shift-register", "quantizer", "melody", "scale"],
        hp_width: 10,
        inputs_count: 2,
        outputs_count: 3,
    },
    ModuleMetadata {
        kind: ModuleKind::SampleAndHoldSlew,
        name: "S&H + Slew Limiter",
        category: ModuleCategory::Utility,
        description: "White/pink noise source, triggered Sample & Hold, and dual rise/fall slew limiter",
        tags: &["sample-and-hold", "noise", "slew", "glide", "utility", "pink-noise"],
        hp_width: 8,
        inputs_count: 4,
        outputs_count: 3,
    },
    ModuleMetadata {
        kind: ModuleKind::MatrixMixer,
        name: "4x4 Matrix Mixer",
        category: ModuleCategory::Utility,
        description: "4-input, 4-output attenuverter CV and audio matrix mixer & router",
        tags: &["matrix", "mixer", "router", "attenuverter", "utility"],
        hp_width: 12,
        inputs_count: 4,
        outputs_count: 4,
    },
    ModuleMetadata {
        kind: ModuleKind::StereoDelay,
        name: "Stereo Ping-Pong Delay",
        category: ModuleCategory::Effect,
        description: "Tempo-synced stereo ping-pong delay with feedback CV and soft-limiting",
        tags: &["delay", "echo", "ping-pong", "stereo", "fx", "effect"],
        hp_width: 10,
        inputs_count: 3,
        outputs_count: 1,
    },
    ModuleMetadata {
        kind: ModuleKind::BidulePluginHost,
        name: "Bidule Plugin Host",
        category: ModuleCategory::Host,
        description: "Plogue Bidule / VCV Host slot storing plugin path, format (CLAP/VST3), and 8 CV parameters",
        tags: &["host", "bidule", "vst3", "clap", "plugin", "moanplug"],
        hp_width: 14,
        inputs_count: 9,
        outputs_count: 1,
    },
];

pub fn find_metadata(kind: ModuleKind) -> &'static ModuleMetadata {
    for meta in &CATALOG {
        if meta.kind == kind {
            return meta;
        }
    }
    &CATALOG[0]
}

pub fn filter_by_category(category: ModuleCategory) -> Vec<&'static ModuleMetadata> {
    let mut results = Vec::new();
    for meta in &CATALOG {
        if meta.category == category {
            results.push(meta);
        }
    }
    results
}

pub fn search(query: &str) -> Vec<&'static ModuleMetadata> {
    let mut results = Vec::new();
    let q_bytes = query.as_bytes();
    for meta in &CATALOG {
        let mut matched = contains_case_insensitive(meta.name.as_bytes(), q_bytes)
            || contains_case_insensitive(meta.description.as_bytes(), q_bytes);
        if !matched {
            for &tag in meta.tags {
                if contains_case_insensitive(tag.as_bytes(), q_bytes) {
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            results.push(meta);
        }
    }
    results
}

fn contains_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    for i in 0..=(haystack.len() - needle.len()) {
        let mut eq = true;
        for j in 0..needle.len() {
            let h = to_ascii_lower(haystack[i + j]);
            let n = to_ascii_lower(needle[j]);
            if h != n {
                eq = false;
                break;
            }
        }
        if eq {
            return true;
        }
    }
    false
}

#[inline]
fn to_ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleFavorites {
    mask: u32,
}

impl ModuleFavorites {
    pub const fn new() -> Self {
        Self { mask: 0 }
    }

    pub fn set_favorite(&mut self, kind: ModuleKind, is_favorite: bool) {
        let bit = 1u32 << (kind as u8);
        if is_favorite {
            self.mask |= bit;
        } else {
            self.mask &= !bit;
        }
    }

    pub fn is_favorite(&self, kind: ModuleKind) -> bool {
        (self.mask & (1u32 << (kind as u8))) != 0
    }
}
