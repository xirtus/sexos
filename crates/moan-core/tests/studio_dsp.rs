// SPDX-License-Identifier: GPL-3.0-only
//! Studio 2.0 DSP test suite for moan-core:
//! Verifies Ariel formant synthesis, Prism wavetable, Distortion modes & EQ curve,
//! WSOLA Warp settings, Extract MIDI pitch accuracy, and Automation interpolation.

extern crate alloc;
use alloc::vec::Vec;
use moan_core::{
    audio_to_midi::extract_midi,
    automation::{AutomationLane, AutomationPoint},
    dsp_math::{
        abs_f32, ceil_f32, cos_f32, exp2_f32, floor_f32, log10_f32, log2_f32, powf_f32,
        round_f32, sin_f32, sqrt_f32, tan_f32, PI,
    },
    effects::{Distortion, DistortionMode, Equalizer},
    midi::{Instrument, Note, Pattern},
    vocal::{self, text_to_arpabet, VoiceParams},
    warp::{bake_warped, WarpSpec},
    wavetable::{preview_curve, PrismParams},
    Clip, Frame, Track, RATE,
};

#[test]
fn dsp_math_accuracy() {
    assert_eq!(floor_f32(2.7), 2.0);
    assert_eq!(floor_f32(-2.1), -3.0);
    assert_eq!(ceil_f32(2.1), 3.0);
    assert_eq!(ceil_f32(-2.7), -2.0);
    assert_eq!(round_f32(2.4), 2.0);
    assert_eq!(round_f32(2.6), 3.0);

    assert!(abs_f32(sin_f32(0.0)) < 1e-5);
    assert!(abs_f32(sin_f32(PI / 2.0) - 1.0) < 1e-5);
    assert!(abs_f32(sin_f32(PI)) < 1e-5);
    assert!(abs_f32(cos_f32(0.0) - 1.0) < 1e-5);
    assert!(abs_f32(cos_f32(PI) + 1.0) < 1e-5);
    assert!(abs_f32(tan_f32(PI / 4.0) - 1.0) < 1e-4);

    assert!(abs_f32(sqrt_f32(4.0) - 2.0) < 1e-5);
    assert!(abs_f32(sqrt_f32(2.0) - 1.41421356) < 1e-5);

    assert!(abs_f32(exp2_f32(0.0) - 1.0) < 1e-5);
    assert!(abs_f32(exp2_f32(3.0) - 8.0) < 1e-4);
    assert!(abs_f32(exp2_f32(-2.0) - 0.25) < 1e-4);

    assert!(abs_f32(log2_f32(8.0) - 3.0) < 1e-4);
    assert!(abs_f32(log2_f32(1.0)) < 1e-5);
    assert!(abs_f32(log10_f32(100.0) - 2.0) < 1e-4);

    assert!(abs_f32(powf_f32(2.0, 3.0) - 8.0) < 1e-4);
    assert!(abs_f32(powf_f32(10.0, 2.0) - 100.0) < 1e-3);
}

#[test]
fn ariel_formant_vowels_non_silent_and_distinct() {
    let params = VoiceParams::default();

    let (aa_pcm, _) = vocal::synthesize("AA", &params).unwrap();
    let (iy_pcm, _) = vocal::synthesize("IY", &params).unwrap();

    assert!(!aa_pcm.is_empty());
    assert!(!iy_pcm.is_empty());

    let energy_aa: i64 = aa_pcm.iter().map(|f| (f[0] as i64).pow(2)).sum();
    let energy_iy: i64 = iy_pcm.iter().map(|f| (f[0] as i64).pow(2)).sum();

    assert!(energy_aa > 100_000, "AA must be audible");
    assert!(energy_iy > 100_000, "IY must be audible");

    // Count zero crossings over the center of the vowel: IY (high F2/F3) has more zero crossings than AA
    let zcr_aa = aa_pcm
        .windows(2)
        .filter(|w| (w[0][0] >= 0 && w[1][0] < 0) || (w[0][0] < 0 && w[1][0] >= 0))
        .count();
    let zcr_iy = iy_pcm
        .windows(2)
        .filter(|w| (w[0][0] >= 0 && w[1][0] < 0) || (w[0][0] < 0 && w[1][0] >= 0))
        .count();

    assert_ne!(
        zcr_aa, zcr_iy,
        "AA and IY must have distinct spectral properties (zcr AA: {zcr_aa}, IY: {zcr_iy})"
    );
}

#[test]
fn ariel_formant_directives_and_polyphony() {
    let params = VoiceParams::default();

    // Utterance with pitch glides, ornaments, pauses, and [voice=1] polyphony
    let utterance_str = "b140 r100 AY+15 AE(+40)! , p100 [voice=1] b220 IY";
    let utterance = vocal::compile_utterance(utterance_str, &params).unwrap();

    assert_eq!(utterance.voices.len(), 2, "Must compile two polyphonic voices");
    assert!(!utterance.spans.is_empty(), "Must track karaoke phrase spans");

    let pcm = utterance.render().unwrap();
    assert!(!pcm.is_empty());
    let energy: i64 = pcm.iter().map(|f| (f[0] as i64).pow(2)).sum();
    assert!(energy > 1_000_000, "Polyphonic utterance must be audible");
}

#[test]
fn ariel_karaoke_spans_and_text_to_arpabet() {
    let lyrics = "hello world";
    let arpa = text_to_arpabet(lyrics);
    assert!(arpa.contains("HH"), "Arpabet conversion must detect phonemes");
    assert!(arpa.contains("W"), "Arpabet conversion must detect world");

    let harder = text_to_arpabet("harder better faster stronger");
    assert!(harder.contains("HH AA R D ER"));
    assert!(harder.contains("B EH T ER"));

    let clip = Clip::vocal(lyrics, VoiceParams::default(), 0).unwrap();
    assert_eq!(clip.start(), 0);
    assert!(!clip.is_empty());
    let v_clip = clip.vocal_clip().unwrap();
    assert_eq!(v_clip.text, lyrics);
    assert!(!v_clip.spans.is_empty(), "Karaoke spans must be generated");
}

#[test]
fn ariel_midi_singing() {
    let mut pattern = Pattern {
        frames: RATE * 2,
        bpm: 120,
        instrument: Instrument::SilkTriangle,
        notes: Vec::new(),
    };
    // 4 notes: C4 (60), D4 (62), E4 (64), F4 (65)
    for (i, &key) in [60, 62, 64, 65].iter().enumerate() {
        pattern.notes.push(Note {
            start: (i as u32) * (RATE / 2),
            frames: RATE / 3,
            key,
            velocity: 100,
        });
    }

    let pcm = vocal::sing_midi_pattern(&pattern, "sing moan with me", &VoiceParams::default()).unwrap();
    assert_eq!(pcm.len(), (RATE * 2) as usize);

    let energy: i64 = pcm.iter().map(|f| (f[0] as i64).pow(2)).sum();
    assert!(energy > 5_000_000, "MIDI singing must produce audible stereo PCM");
}

#[test]
fn prism_wavetable_morphing_and_preview_curve() {
    let mut params = PrismParams::default();
    let mut curve_a = [0i16; 128];
    let mut curve_b = [0i16; 128];

    params.osc_a_pos = 0; // Sine
    preview_curve(&params, &mut curve_a);

    params.osc_a_pos = 255; // Pulse
    preview_curve(&params, &mut curve_b);

    assert_ne!(curve_a, curve_b, "Morphing position must produce different curves");

    // Instrument baking
    let pattern = Pattern::preset(Instrument::PrismWavetable);
    let pcm = pattern.bake().unwrap();
    assert!(!pcm.is_empty());

    let formant_pattern = Pattern::preset(Instrument::ArielFormant);
    let formant_pcm = formant_pattern.bake().unwrap();
    assert!(!formant_pcm.is_empty());
}

#[test]
fn distortion_modes_and_transfer_curves() {
    let mut curves = Vec::new();
    for mode in [
        DistortionMode::Symmetric,
        DistortionMode::Asymmetric,
        DistortionMode::HardClip,
        DistortionMode::Foldback,
    ] {
        let dist = Distortion::new(200, 0, 0, 100, 10000, mode).unwrap();
        let mut curve = [0i16; 64];
        dist.transfer_curve(&mut curve);
        curves.push(curve);
    }

    // All modes must produce unique nonlinear curves
    for i in 0..curves.len() {
        for j in (i + 1)..curves.len() {
            assert_ne!(curves[i], curves[j], "Distortion modes must have distinct curves");
        }
    }

    // Test Track.distortion integration
    let mut track = Track::new();
    track.distortion = Some(Distortion::new(300, 20, 10, 100, 8000, DistortionMode::Symmetric).unwrap());
    let mut buf: Vec<Frame> = (0..1000).map(|i| [((i * 50) % 20000) as i16; 2]).collect();
    let original = buf.clone();
    track.process(&mut buf);
    assert_ne!(buf, original, "Track process must apply distortion");

    track.reset_effects();
}

#[test]
fn equalizer_response_curve_log_spaced() {
    let flat_eq = Equalizer::new(100, 100, 100).unwrap();
    let mut flat_curve = [0i16; 32];
    flat_eq.response_curve(&mut flat_curve);

    for &db_q8 in &flat_curve {
        assert!(
            db_q8.abs() <= 5,
            "Flat EQ must have ~0 dB response across all frequencies, got {db_q8}"
        );
    }

    let bass_eq = Equalizer::new(250, 100, 100).unwrap();
    let mut bass_curve = [0i16; 32];
    bass_eq.response_curve(&mut bass_curve);

    // Bass response at low frequencies (index 0) must be higher than high frequencies (index 31)
    assert!(
        bass_curve[0] > bass_curve[31] + 256,
        "Low frequencies must have boosted response"
    );
}

#[test]
fn warp_transpose_and_speed() {
    let freq = 440.0;
    let len = RATE as usize; // 1 second
    let source: Vec<Frame> = (0..len)
        .map(|i| {
            let s = (sin_f32(2.0 * PI * freq * i as f32 / RATE as f32) * 16000.0) as i16;
            [s, s]
        })
        .collect();

    // 2x speed (speed_percent = 200) -> output duration is half
    let double_speed = bake_warped(
        &source,
        WarpSpec {
            transpose_semitones: 0,
            speed_percent: 200,
            on_beat_bpm: None,
        },
    )
    .unwrap();
    let diff = (double_speed.len() as i64 - (len as i64 / 2)).abs();
    assert!(diff < 100, "Double speed must halve duration");

    // 0.5x speed (speed_percent = 50) -> output duration is double
    let half_speed = bake_warped(
        &source,
        WarpSpec {
            transpose_semitones: 0,
            speed_percent: 50,
            on_beat_bpm: None,
        },
    )
    .unwrap();
    let diff_half = (half_speed.len() as i64 - (len as i64 * 2)).abs();
    assert!(diff_half < 100, "Half speed must double duration");

    // Transpose 12 semitones (1 octave)
    let transposed = bake_warped(
        &source,
        WarpSpec {
            transpose_semitones: 12,
            speed_percent: 100,
            on_beat_bpm: None,
        },
    )
    .unwrap();
    assert!(!transposed.is_empty());
}

#[test]
fn audio_to_midi_pitch_accuracy() {
    // Generate 1 second of pure A4 (440 Hz) tone, MIDI 69
    let len = RATE as usize;
    let frames: Vec<Frame> = (0..len)
        .map(|i| {
            let s = (sin_f32(2.0 * PI * 440.0 * i as f32 / RATE as f32) * 20000.0) as i16;
            [s, s]
        })
        .collect();

    let pattern = extract_midi(&frames, 120).unwrap();
    assert!(!pattern.notes.is_empty(), "Must detect notes");
    assert_eq!(pattern.notes[0].key, 69, "Must detect exact A4 pitch (69)");
}

#[test]
fn automation_lane_interpolation() {
    let mut lane = AutomationLane::new(50);
    assert_eq!(lane.sample_at(0), 50);

    lane.add_point(AutomationPoint {
        frame: 1000,
        value: 100,
    })
    .unwrap();
    lane.add_point(AutomationPoint {
        frame: 2000,
        value: 200,
    })
    .unwrap();

    assert_eq!(lane.sample_at(500), 100); // Clamped before first point
    assert_eq!(lane.sample_at(1000), 100);
    assert_eq!(lane.sample_at(1500), 150); // Midpoint linear interpolation
    assert_eq!(lane.sample_at(2000), 200);
    assert_eq!(lane.sample_at(3000), 200); // Clamped after last point
}

#[test]
fn oom_and_range_rejections() {
    let mut lane = AutomationLane::new(0);
    for i in 0..256 {
        lane.add_point(AutomationPoint {
            frame: i * 10,
            value: i as u16,
        })
        .unwrap();
    }
    // 257th point must be refused
    assert_eq!(
        lane.add_point(AutomationPoint {
            frame: 3000,
            value: 99,
        }),
        Err(moan_core::Error::Range)
    );

    // Warp range checks
    assert!(bake_warped(
        &[[0, 0]],
        WarpSpec {
            transpose_semitones: 30, // Out of range -24..=24
            speed_percent: 100,
            on_beat_bpm: None,
        }
    )
    .is_err());

    // Distortion range checks
    assert!(Distortion::new(500, 0, 0, 100, 1000, DistortionMode::Symmetric).is_err());
    assert!(Distortion::new(100, 150, 0, 100, 1000, DistortionMode::Symmetric).is_err());
    assert!(Distortion::new(100, 0, 0, 150, 1000, DistortionMode::Symmetric).is_err());
}
