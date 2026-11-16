// SPDX-License-Identifier: GPL-3.0-only
//! Standalone `#![no_std]` Muse Musical Typing keyboard, scale quantization,
//! chord expansion, and 48 kHz arpeggiator engine.

extern crate alloc;

use alloc::vec::Vec;

/// Musical typing computer keyboard mapping.
/// Maps keys: A W S E D F T G Y H U J K O L P ; across chromatic notes.
/// Z and X are octave shift control keys and return None from this function.
/// `base_octave` specifies the octave for the primary 'A' key (typically 4 for Middle C = 60).
pub fn musical_typing_key(ascii: u8, base_octave: i8) -> Option<u8> {
    let offset: i16 = match ascii {
        b'a' | b'A' => 0,  // C
        b'w' | b'W' => 1,  // C#
        b's' | b'S' => 2,  // D
        b'e' | b'E' => 3,  // D#
        b'd' | b'D' => 4,  // E
        b'f' | b'F' => 5,  // F
        b't' | b'T' => 6,  // F#
        b'g' | b'G' => 7,  // G
        b'y' | b'Y' => 8,  // G#
        b'h' | b'H' => 9,  // A
        b'u' | b'U' => 10, // A#
        b'j' | b'J' => 11, // B
        b'k' | b'K' => 12, // C (+1 octave)
        b'o' | b'O' => 13, // C# (+1 octave)
        b'l' | b'L' => 14, // D (+1 octave)
        b'p' | b'P' => 15, // D# (+1 octave)
        b';' => 16,        // E (+1 octave)
        _ => return None,
    };

    // Middle C (C4) is MIDI note 60 = (4 + 1) * 12
    let c_root = (base_octave as i16 + 1) * 12;
    let note = c_root + offset;
    if (0..=127).contains(&note) {
        Some(note as u8)
    } else {
        None
    }
}

/// Check if an ASCII key triggers an octave shift:
/// 'Z' / 'z' -> -1 octave
/// 'X' / 'x' -> +1 octave
pub fn octave_shift_key(ascii: u8) -> Option<i8> {
    match ascii {
        b'z' | b'Z' => Some(-1),
        b'x' | b'X' => Some(1),
        _ => None,
    }
}

/// Musical scale kinds for diatonic quantization and chord generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleKind {
    Chromatic,
    Major,
    NaturalMinor,
    Dorian,
    Mixolydian,
    PentatonicMinor,
    Blues,
}

/// Return the pitch class intervals relative to root (0..11) for a given scale.
pub fn scale_intervals(scale: ScaleKind) -> &'static [u8] {
    match scale {
        ScaleKind::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        ScaleKind::Major => &[0, 2, 4, 5, 7, 9, 11],
        ScaleKind::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
        ScaleKind::Dorian => &[0, 2, 3, 5, 7, 9, 10],
        ScaleKind::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
        ScaleKind::PentatonicMinor => &[0, 3, 5, 7, 10],
        ScaleKind::Blues => &[0, 3, 5, 6, 7, 10],
    }
}

/// Quantize an arbitrary MIDI note to the nearest note in the given musical scale.
/// If equidistant, snaps downwards.
pub fn quantize_to_scale(note: u8, root: u8, scale: ScaleKind) -> u8 {
    let intervals = scale_intervals(scale);
    let root_class = (root % 12) as i16;
    let note_class = (note % 12) as i16;
    let rel = (note_class - root_class).rem_euclid(12) as u8;

    if intervals.contains(&rel) {
        return note;
    }

    // Find nearest scale pitch class (with wrap-around distance)
    let mut best_diff = 12i16;
    let mut best_signed = 0i16;

    for &cand in intervals {
        let diff = cand as i16 - rel as i16;
        let alt_diff = if diff > 0 { diff - 12 } else { diff + 12 };
        let signed_diff = if diff.abs() <= alt_diff.abs() {
            diff
        } else {
            alt_diff
        };
        let dist = signed_diff.abs();

        if dist < best_diff || (dist == best_diff && signed_diff < 0) {
            best_diff = dist;
            best_signed = signed_diff;
        }
    }

    let quantized = (note as i16 + best_signed).clamp(0, 127) as u8;
    quantized
}

/// Chord expansion modes for single-key chord accompaniment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordMode {
    Off,
    Triad,
    Seventh,
    Power,
    Lush9th,
}

/// Expand a root MIDI note into up to 5 chord notes according to the given scale and chord mode.
pub fn expand_chord(root_note: u8, scale: ScaleKind, mode: ChordMode) -> Vec<u8> {
    match mode {
        ChordMode::Off => alloc::vec![root_note],
        ChordMode::Power => {
            let mut notes = Vec::with_capacity(3);
            notes.push(root_note);
            if root_note as u16 + 7 <= 127 {
                notes.push(root_note + 7);
            }
            if root_note as u16 + 12 <= 127 {
                notes.push(root_note + 12);
            }
            notes
        }
        ChordMode::Triad | ChordMode::Seventh | ChordMode::Lush9th => {
            let intervals = scale_intervals(scale);
            let n_int = intervals.len();

            // Find scale degree of root_note
            let root_class = (root_note % 12) as i16;
            let rel = root_class; // assuming root is key 0

            // Locate base degree in intervals
            let base_degree = intervals
                .iter()
                .position(|&x| x == (rel as u8))
                .unwrap_or(0);

            let degrees: &[usize] = match mode {
                ChordMode::Triad => &[0, 2, 4],
                ChordMode::Seventh => &[0, 2, 4, 6],
                ChordMode::Lush9th => &[0, 2, 4, 6, 8],
                _ => unreachable!(),
            };

            let mut notes = Vec::with_capacity(degrees.len());
            for &deg_step in degrees {
                let target_idx = (base_degree + deg_step) % n_int;
                let octaves_up = (base_degree + deg_step) / n_int;
                let semitones_from_root =
                    intervals[target_idx] as i16 + (octaves_up as i16 * 12) - intervals[base_degree] as i16;
                let note = root_note as i16 + semitones_from_root;
                if (0..=127).contains(&note) {
                    notes.push(note as u8);
                }
            }
            notes
        }
    }
}

/// Arpeggiator playback patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpMode {
    Up,
    Down,
    UpDown,
    Random,
    ChordGate,
}

/// Note step division for arpeggiator clocking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepDivision {
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    EighthTriplet,
    SixteenthTriplet,
}

impl StepDivision {
    /// Multiplier relative to 1 quarter note beat.
    #[inline]
    pub fn beat_fraction(self) -> f64 {
        match self {
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
            Self::ThirtySecond => 0.125,
            Self::EighthTriplet => 1.0 / 3.0,
            Self::SixteenthTriplet => 1.0 / 6.0,
        }
    }
}

/// Generate deterministic arpeggiated note events at 48 kHz capture rate.
/// Each event is `(start_frame, len_frames, midi_key, velocity)`.
/// Allocation is completely `try_reserve` safe.
pub fn generate_arp(
    held_keys: &[u8],
    bpm: f32,
    step: StepDivision,
    gate_percent: f32,
    mode: ArpMode,
    num_steps: usize,
) -> Vec<(u32, u32, u8, u8)> {
    generate_arp_custom(held_keys, bpm, step, gate_percent, mode, num_steps, 48_000, 100)
}

/// Generate deterministic arpeggiated note events with configurable sample rate and velocity.
pub fn generate_arp_custom(
    held_keys: &[u8],
    bpm: f32,
    step: StepDivision,
    gate_percent: f32,
    mode: ArpMode,
    num_steps: usize,
    sample_rate: u32,
    velocity: u8,
) -> Vec<(u32, u32, u8, u8)> {
    if held_keys.is_empty() || num_steps == 0 || bpm <= 0.0 || sample_rate == 0 {
        return Vec::new();
    }

    let mut keys: Vec<u8> = held_keys.to_vec();
    keys.sort_unstable();
    keys.dedup();
    let num_keys = keys.len();
    if num_keys == 0 {
        return Vec::new();
    }

    let total_notes = if mode == ArpMode::ChordGate {
        num_steps.saturating_mul(num_keys)
    } else {
        num_steps
    };

    let mut out = Vec::new();
    if out.try_reserve(total_notes).is_err() {
        return Vec::new();
    }

    let step_frames_f64 = (60.0 * sample_rate as f64 * step.beat_fraction()) / (bpm as f64);
    let gate = (gate_percent.clamp(0.01, 1.0) as f64) * step_frames_f64;
    let len_frames = ((gate + 0.5) as u32).max(1);

    // Deterministic LCG seed for Random mode
    let mut rng_state: u32 = 0xA5A5_5A5A;

    for s in 0..num_steps {
        let start_frame = (s as f64 * step_frames_f64 + 0.5) as u32;

        match mode {
            ArpMode::Up => {
                let key = keys[s % num_keys];
                out.push((start_frame, len_frames, key, velocity));
            }
            ArpMode::Down => {
                let key = keys[num_keys - 1 - (s % num_keys)];
                out.push((start_frame, len_frames, key, velocity));
            }
            ArpMode::UpDown => {
                let key = if num_keys <= 1 {
                    keys[0]
                } else {
                    let period = 2 * num_keys - 2;
                    let phase = s % period;
                    let idx = if phase < num_keys {
                        phase
                    } else {
                        period - phase
                    };
                    keys[idx]
                };
                out.push((start_frame, len_frames, key, velocity));
            }
            ArpMode::Random => {
                // Deterministic LCG step
                rng_state = rng_state.wrapping_mul(1664525).wrapping_add(1013904223);
                let idx = (rng_state as usize) % num_keys;
                out.push((start_frame, len_frames, keys[idx], velocity));
            }
            ArpMode::ChordGate => {
                for &k in &keys {
                    out.push((start_frame, len_frames, k, velocity));
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_musical_typing_keyboard_mapping() {
        // Octave 4: Middle C (60)
        assert_eq!(musical_typing_key(b'a', 4), Some(60)); // C4
        assert_eq!(musical_typing_key(b'w', 4), Some(61)); // C#4
        assert_eq!(musical_typing_key(b's', 4), Some(62)); // D4
        assert_eq!(musical_typing_key(b'e', 4), Some(63)); // D#4
        assert_eq!(musical_typing_key(b'd', 4), Some(64)); // E4
        assert_eq!(musical_typing_key(b'f', 4), Some(65)); // F4
        assert_eq!(musical_typing_key(b't', 4), Some(66)); // F#4
        assert_eq!(musical_typing_key(b'g', 4), Some(67)); // G4
        assert_eq!(musical_typing_key(b'y', 4), Some(68)); // G#4
        assert_eq!(musical_typing_key(b'h', 4), Some(69)); // A4 (440 Hz)
        assert_eq!(musical_typing_key(b'u', 4), Some(70)); // A#4
        assert_eq!(musical_typing_key(b'j', 4), Some(71)); // B4
        assert_eq!(musical_typing_key(b'k', 4), Some(72)); // C5
        assert_eq!(musical_typing_key(b'o', 4), Some(73)); // C#5
        assert_eq!(musical_typing_key(b'l', 4), Some(74)); // D5
        assert_eq!(musical_typing_key(b'p', 4), Some(75)); // D#5
        assert_eq!(musical_typing_key(b';', 4), Some(76)); // E5

        // Case insensitivity
        assert_eq!(musical_typing_key(b'A', 4), Some(60));
        assert_eq!(musical_typing_key(b'K', 4), Some(72));

        // Z and X octave shift control keys return None as pitch notes
        assert_eq!(musical_typing_key(b'z', 4), None);
        assert_eq!(musical_typing_key(b'x', 4), None);
        assert_eq!(octave_shift_key(b'z'), Some(-1));
        assert_eq!(octave_shift_key(b'x'), Some(1));
    }

    #[test]
    fn test_scale_quantization() {
        // C Major: 60 (C), 62 (D), 64 (E), 65 (F), 67 (G), 69 (A), 71 (B)
        assert_eq!(quantize_to_scale(60, 60, ScaleKind::Major), 60); // C -> C
        assert_eq!(quantize_to_scale(61, 60, ScaleKind::Major), 60); // C# -> C (snaps down)
        assert_eq!(quantize_to_scale(62, 60, ScaleKind::Major), 62); // D -> D

        // A Minor: root 69
        assert_eq!(quantize_to_scale(69, 69, ScaleKind::NaturalMinor), 69); // A -> A
        assert_eq!(quantize_to_scale(70, 69, ScaleKind::NaturalMinor), 69); // A# -> A
    }

    #[test]
    fn test_chord_expansion() {
        // Power chord on 60 (C): [60, 67, 72]
        let pwr = expand_chord(60, ScaleKind::Major, ChordMode::Power);
        assert_eq!(pwr, alloc::vec![60, 67, 72]);

        // Triad on 60 (C) in C Major: C (60), E (64), G (67)
        let triad = expand_chord(60, ScaleKind::Major, ChordMode::Triad);
        assert_eq!(triad, alloc::vec![60, 64, 67]);

        // Seventh on 60 (C) in C Major: C (60), E (64), G (67), B (71)
        let seventh = expand_chord(60, ScaleKind::Major, ChordMode::Seventh);
        assert_eq!(seventh, alloc::vec![60, 64, 67, 71]);

        // Lush 9th on 60 (C) in C Major: C (60), E (64), G (67), B (71), D (74)
        let lush = expand_chord(60, ScaleKind::Major, ChordMode::Lush9th);
        assert_eq!(lush, alloc::vec![60, 64, 67, 71, 74]);
    }

    #[test]
    fn test_arpeggiator_up_down_modes() {
        let keys = [60, 64, 67]; // C E G
        let bpm = 120.0;
        let step = StepDivision::Sixteenth; // 1/16 note = 0.25 beat = 0.125s at 120 BPM = 6000 frames at 48 kHz
        let gate = 0.8;

        // Up: 60, 64, 67, 60
        let arp_up = generate_arp(&keys, bpm, step, gate, ArpMode::Up, 4);
        assert_eq!(arp_up.len(), 4);
        assert_eq!(arp_up[0].2, 60);
        assert_eq!(arp_up[1].2, 64);
        assert_eq!(arp_up[2].2, 67);
        assert_eq!(arp_up[3].2, 60);

        // Check frame timing: at 120 BPM, 48 kHz, 1/16 note = 6000 frames
        assert_eq!(arp_up[0].0, 0);
        assert_eq!(arp_up[1].0, 6000);
        assert_eq!(arp_up[2].0, 12000);
        assert_eq!(arp_up[3].0, 18000);
        assert_eq!(arp_up[0].1, 4800); // 80% of 6000 = 4800

        // Down: 67, 64, 60, 67
        let arp_down = generate_arp(&keys, bpm, step, gate, ArpMode::Down, 4);
        assert_eq!(arp_down[0].2, 67);
        assert_eq!(arp_down[1].2, 64);
        assert_eq!(arp_down[2].2, 60);
        assert_eq!(arp_down[3].2, 67);

        // UpDown: 60, 64, 67, 64, 60, 64
        let arp_updown = generate_arp(&keys, bpm, step, gate, ArpMode::UpDown, 6);
        assert_eq!(arp_updown[0].2, 60);
        assert_eq!(arp_updown[1].2, 64);
        assert_eq!(arp_updown[2].2, 67);
        assert_eq!(arp_updown[3].2, 64);
        assert_eq!(arp_updown[4].2, 60);
        assert_eq!(arp_updown[5].2, 64);
    }

    #[test]
    fn test_arpeggiator_random_deterministic() {
        let keys = [48, 55, 60, 64];
        let run1 = generate_arp(&keys, 128.0, StepDivision::Eighth, 0.5, ArpMode::Random, 16);
        let run2 = generate_arp(&keys, 128.0, StepDivision::Eighth, 0.5, ArpMode::Random, 16);
        assert_eq!(run1.len(), 16);
        assert_eq!(run1, run2, "ArpMode::Random must be 100% deterministic");
    }

    #[test]
    fn test_arpeggiator_chord_gate() {
        let keys = [60, 64, 67]; // Triad
        let arp_gate = generate_arp(&keys, 120.0, StepDivision::Quarter, 0.5, ArpMode::ChordGate, 2);
        // 2 steps * 3 keys = 6 note events
        assert_eq!(arp_gate.len(), 6);
        // Step 0: all 3 keys at start_frame 0
        assert_eq!(arp_gate[0].0, 0);
        assert_eq!(arp_gate[1].0, 0);
        assert_eq!(arp_gate[2].0, 0);
        assert_eq!(arp_gate[0].2, 60);
        assert_eq!(arp_gate[1].2, 64);
        assert_eq!(arp_gate[2].2, 67);
        // Step 1: all 3 keys at start_frame 24000 (1 quarter note at 120 BPM = 0.5s = 24000 frames)
        assert_eq!(arp_gate[3].0, 24000);
        assert_eq!(arp_gate[4].0, 24000);
        assert_eq!(arp_gate[5].0, 24000);
    }

    #[test]
    fn test_arpeggiator_edge_cases_and_triplets() {
        // Empty keys
        assert!(generate_arp(&[], 120.0, StepDivision::Sixteenth, 0.8, ArpMode::Up, 8).is_empty());
        // Zero steps
        assert!(generate_arp(&[60], 120.0, StepDivision::Sixteenth, 0.8, ArpMode::Up, 0).is_empty());
        // Negative / zero BPM
        assert!(generate_arp(&[60], 0.0, StepDivision::Sixteenth, 0.8, ArpMode::Up, 4).is_empty());
        assert!(generate_arp(&[60], -100.0, StepDivision::Sixteenth, 0.8, ArpMode::Up, 4).is_empty());

        // Triplet division: EighthTriplet at 120 BPM
        // 1 beat = 24000 frames; 1/3 beat = 8000 frames
        let arp_triplet = generate_arp(&[60], 120.0, StepDivision::EighthTriplet, 0.5, ArpMode::Up, 3);
        assert_eq!(arp_triplet.len(), 3);
        assert_eq!(arp_triplet[0].0, 0);
        assert_eq!(arp_triplet[1].0, 8000);
        assert_eq!(arp_triplet[2].0, 16000);
        assert_eq!(arp_triplet[0].1, 4000);
    }

    #[test]
    fn test_all_scale_kinds() {
        let scales = [
            ScaleKind::Chromatic,
            ScaleKind::Major,
            ScaleKind::NaturalMinor,
            ScaleKind::Dorian,
            ScaleKind::Mixolydian,
            ScaleKind::PentatonicMinor,
            ScaleKind::Blues,
        ];
        for scale in scales {
            let intervals = scale_intervals(scale);
            assert!(!intervals.is_empty());
            assert_eq!(intervals[0], 0); // All scales start at 0 relative to root
            // Ensure quantize_to_scale is idempotent for notes in scale
            for &int in intervals {
                let note = 60 + int;
                assert_eq!(quantize_to_scale(note, 60, scale), note);
            }
        }
    }
}

