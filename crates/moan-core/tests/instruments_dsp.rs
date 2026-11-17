// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive DSP verification suite for Moan's 7 expressive instruments
//! (`Sub808`, `DrumKit`, `TineKeys`, `ModalGrand`, `PluckGuitar`, `ModalMallet`, `CrossSynth`)
//! and unified `Instrument` / `NoteExpr` / `Pattern::bake` integration.

use moan_core::{
    instruments::{
        render_notes_with_params, render_pattern_with_params, CrossSynth, DriveCurve, DrumKit,
        DrumModel, ExpressiveNote, GlideMode, GuitarAmp, InstrumentParams, MalletKind, ModalGrand,
        ModalMallet, NoteExpr, PluckGuitar, PluckMode, Sub808, TineKeys,
    },
    midi::{Error, Instrument, Note, Pattern, MAX_FRAMES, MAX_NOTES, MAX_VOICES},
    Clip, Frame, RATE,
};

fn rms_i64(frames: &[Frame]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let sum: f64 = frames
        .iter()
        .map(|f| {
            let l = f[0] as f64;
            let r = f[1] as f64;
            0.5 * (l * l + r * r)
        })
        .sum();
    (sum / frames.len() as f64).sqrt()
}

fn zero_crossings_left(frames: &[Frame]) -> usize {
    frames
        .windows(2)
        .filter(|w| w[0][0] <= 0 && w[1][0] > 0)
        .count()
}

/// First-difference RMS / signal RMS (spectral brightness proxy: higher = brighter/more HF).
fn brightness_ratio(frames: &[Frame]) -> f64 {
    if frames.len() < 2 {
        return 0.0;
    }
    let sig_e: f64 = frames.iter().map(|f| (f[0] as f64).powi(2)).sum();
    let diff_e: f64 = frames
        .windows(2)
        .map(|w| ((w[1][0] as f64) - (w[0][0] as f64)).powi(2))
        .sum();
    if sig_e <= 1.0 {
        0.0
    } else {
        (diff_e / sig_e).sqrt()
    }
}

fn simple_note(start: u32, frames: u32, key: u8, velocity: u8) -> Note {
    Note {
        start,
        frames,
        key,
        velocity,
    }
}

#[test]
fn all_12_instruments_metadata_and_non_silent_bake() {
    assert_eq!(Instrument::ALL.len(), 12);
    let expected_names = [
        "Silk Keys",
        "Satin Bass",
        "Velvet Kit",
        "Ariel Voice",
        "Prism Synth",
        "Sub 808",
        "Studio Drums",
        "Tine Keys",
        "Grand Piano",
        "Electric Guitar",
        "Mallet Bells",
        "Cross Synth",
    ];
    for (idx, (&inst, &exp_name)) in Instrument::ALL.iter().zip(&expected_names).enumerate() {
        assert_eq!(inst as u8, idx as u8);
        assert_eq!(Instrument::try_from(idx as u8), Ok(inst));
        assert_eq!(inst.name(), exp_name);

        let pat = Pattern::preset(inst);
        let pcm = pat.bake().expect("all 12 presets must bake cleanly");
        assert_eq!(pcm.len(), (RATE * 4) as usize);
        let rms = rms_i64(&pcm);
        assert!(
            rms > 250.0,
            "Instrument {:?} ({}) must produce non-silent stereo PCM (rms={rms})",
            inst,
            inst.name()
        );
        // Clip::midi round-trip must succeed for all 12 variants
        let clip = Clip::midi(pat, 0).expect("Clip::midi must accept all 12 instruments");
        assert_eq!(clip.len(), (RATE * 4) as usize);
    }

    assert_eq!(Instrument::try_from(12), Err(Error::Instrument));
    assert_eq!(Instrument::try_from(255), Err(Error::Instrument));
}

#[test]
fn sub808_legato_glide_is_phase_locked_and_click_free() {
    // Overlapping notes: Note A (C2=36, 0..24000) overlaps Note B (C3=48, 12000..36000)
    let notes = [
        simple_note(0, 24_000, 36, 110),
        simple_note(12_000, 24_000, 48, 110),
    ];
    let sub = Sub808 {
        shape: 0.0,
        octave: 0.0,
        punch_semitones: 0.0,
        punch_ms: 10.0,
        hold_ms: 800.0,
        decay_ms: 2000.0,
        decay_key_track: 0.0,
        glide_mode: GlideMode::Legato,
        glide_ms: 100.0, // 4800 frames glide
        drive: 0.0,
        drive_curve: DriveCurve::WarmSym,
        tone_hz: 12_000.0,
        click: 0.0,
        growl: 0.0,
    };

    let pcm = sub.render_pattern(&notes, RATE).unwrap();
    assert_eq!(pcm.len(), RATE as usize);

    // Measure frequency before glide (65.41 Hz), mid-glide, and post-glide (130.81 Hz)
    let zc_pre = zero_crossings_left(&pcm[2_000..11_600]); // 0.2s -> ~13.08 cycles
    let hz_pre = zc_pre as f64 / (9_600.0 / RATE as f64);
    assert!(
        (60.0..=71.0).contains(&hz_pre),
        "pre-glide pitch should be ~65.4 Hz, got {hz_pre}"
    );

    let zc_mid = zero_crossings_left(&pcm[12_500..16_500]); // mid-glide
    let hz_mid = zc_mid as f64 / (4_000.0 / RATE as f64);
    assert!(
        hz_mid > 75.0 && hz_mid < 122.0,
        "mid-glide pitch should glide smoothly between C2 and C3, got {hz_mid}"
    );

    let zc_post = zero_crossings_left(&pcm[19_000..28_600]); // 0.2s -> ~26.16 cycles
    let hz_post = zc_post as f64 / (9_600.0 / RATE as f64);
    assert!(
        (124.0..=138.0).contains(&hz_post),
        "post-glide pitch should settle at ~130.8 Hz, got {hz_post}"
    );

    // Check continuity around the overlap boundary (frame 12_000): zero phase-reset click!
    let max_step_at_join = pcm[11_980..12_050]
        .windows(2)
        .map(|w| (w[1][0] as i32 - w[0][0] as i32).abs())
        .max()
        .unwrap();
    assert!(
        max_step_at_join < 800,
        "legato slide must be phase-locked without clicks (max step = {max_step_at_join})"
    );

    // Also test Pattern::bake integration invokes overlapping-note Legato slide on Instrument::Sub808
    let pat = Pattern {
        frames: RATE,
        bpm: 120,
        instrument: Instrument::Sub808,
        notes: notes.to_vec(),
    };
    let baked = pat.bake().unwrap();
    let max_step_baked = baked[11_980..12_050]
        .windows(2)
        .map(|w| (w[1][0] as i32 - w[0][0] as i32).abs())
        .max()
        .unwrap();
    assert!(
        max_step_baked < 1800,
        "Pattern::bake Sub808 legato join must not click (max step = {max_step_baked})"
    );

    // Test GlideMode::Off retriggering over sounding voice crossfades over 4 ms (192 frames) without clicking
    let mut sub_retrig = sub;
    sub_retrig.glide_mode = GlideMode::Off;
    let pcm_retrig = sub_retrig.render_pattern(&notes, RATE).unwrap();
    let max_step_retrig = pcm_retrig[11_995..12_210]
        .windows(2)
        .map(|w| (w[1][0] as i32 - w[0][0] as i32).abs())
        .max()
        .unwrap();
    assert!(
        max_step_retrig < 1500,
        "4 ms crossfade retrigger must stay click-free (max step = {max_step_retrig})"
    );

    // Test explicit slide_from_key in NoteExpr
    let expr_notes = [ExpressiveNote::new(
        simple_note(0, 24_000, 48, 100),
        NoteExpr {
            slide_from_key: Some(36),
            glide_frames: 4800,
            ..NoteExpr::default()
        },
    )];
    let pcm_expr = sub.render_expressive(&expr_notes, 24_000).unwrap();
    let hz_expr_start = zero_crossings_left(&pcm_expr[200..2200]) as f64 / (2000.0 / RATE as f64);
    let hz_expr_end = zero_crossings_left(&pcm_expr[10_000..19_600]) as f64 / (9600.0 / RATE as f64);
    assert!(hz_expr_end > hz_expr_start * 1.4, "explicit slide_from_key must glide upward");
}

#[test]
fn sub808_4x_oversampled_drive_foldback_growl_and_keytrack() {
    let notes = [simple_note(0, 24_000, 36, 120)];
    let mut rendered = Vec::new();
    for curve in [
        DriveCurve::WarmSym,
        DriveCurve::TubeAsym,
        DriveCurve::HardClip,
        DriveCurve::Foldback,
    ] {
        let sub = Sub808 {
            drive: 1.0,
            drive_curve: curve,
            tone_hz: 4000.0,
            ..Sub808::default()
        };
        let pcm = sub.render_pattern(&notes, 24_000).unwrap();
        assert!(rms_i64(&pcm) > 2000.0, "{curve:?} must be loud at max drive");
        // Verify DC blocker removes DC offset even on TubeAsym max drive
        let mean_dc: f64 = pcm[2400..].iter().map(|f| f[0] as f64).sum::<f64>() / (pcm.len() - 2400) as f64;
        assert!(
            mean_dc.abs() < 1200.0,
            "{curve:?} DC blocker must keep DC offset small, got {mean_dc}"
        );
        rendered.push(pcm);
    }
    for i in 0..rendered.len() {
        for j in (i + 1)..rendered.len() {
            assert_ne!(rendered[i], rendered[j], "drive curves must sound distinct");
        }
    }

    // High-note Foldback above 260 Hz stays finite and bounded at max drive
    let high_fold = Sub808 {
        drive: 1.0,
        drive_curve: DriveCurve::Foldback,
        tone_hz: 16_000.0,
        ..Sub808::default()
    };
    let high_pcm = high_fold
        .render_pattern(&[simple_note(0, 12_000, 74, 127)], 12_000)
        .unwrap();
    assert!(rms_i64(&high_pcm) > 500.0);

    // Formant growl + click tests
    let dry_sub = Sub808 {
        growl: 0.0,
        click: 0.0,
        ..Sub808::default()
    };
    let growl_sub = Sub808 {
        growl: 0.9,
        click: 0.8,
        ..Sub808::default()
    };
    let a = dry_sub.render_pattern(&notes, 24_000).unwrap();
    let b = growl_sub.render_pattern(&notes, 24_000).unwrap();
    assert_ne!(a, b);
    assert!(brightness_ratio(&b[..1000]) > brightness_ratio(&a[..1000]));

    // Decay key tracking: higher note decays faster than lower note
    let kt_sub = Sub808 {
        hold_ms: 10.0,
        decay_ms: 500.0,
        decay_key_track: 1.0,
        ..Sub808::default()
    };
    let low = kt_sub
        .render_pattern(&[simple_note(0, 36_000, 30, 100)], 36_000)
        .unwrap();
    let high = kt_sub
        .render_pattern(&[simple_note(0, 36_000, 66, 100)], 36_000)
        .unwrap();
    let low_tail_ratio = rms_i64(&low[20_000..35_000]) / rms_i64(&low[1000..5000]);
    let high_tail_ratio = rms_i64(&high[20_000..35_000]) / rms_i64(&high[1000..5000]);
    assert!(
        high_tail_ratio < low_tail_ratio,
        "decay_key_track must make high notes decay faster ({high_tail_ratio} vs {low_tail_ratio})"
    );
}

#[test]
fn drumkit_closed_hat_chokes_open_hat_over_3ms() {
    let kit = DrumKit::default();

    // Open Hat (46) alone
    let open_only = kit
        .render_pattern(&[simple_note(0, 2400, 46, 115)], RATE)
        .unwrap();

    // Open Hat (46) at 0, choked by Closed Hat (42) at 4800 (100 ms)
    let choked_42 = kit
        .render_pattern(
            &[simple_note(0, 2400, 46, 115), simple_note(4800, 2400, 42, 115)],
            RATE,
        )
        .unwrap();

    // Pedal Hat (44) also chokes Open Hat (46)
    let choked_44 = kit
        .render_pattern(
            &[simple_note(0, 2400, 46, 115), simple_note(4800, 2400, 44, 115)],
            RATE,
        )
        .unwrap();

    let open_late = rms_i64(&open_only[12_000..36_000]);
    let choked42_late = rms_i64(&choked_42[12_000..36_000]);
    let choked44_late = rms_i64(&choked_44[12_000..36_000]);

    assert!(open_late > 200.0, "open hat alone must ring past 250ms (got {open_late})");
    assert!(
        choked42_late < open_late * 0.12,
        "key 42 must choke key 46 (open={open_late}, choked={choked42_late})"
    );
    assert!(
        choked44_late < open_late * 0.12,
        "key 44 must choke key 46 (open={open_late}, choked={choked44_late})"
    );
}

#[test]
fn drumkit_all_gm_pieces_and_models_distinct_spectra() {
    let gm_keys = [
        35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 57,
    ];
    for model in [
        DrumModel::Electronic,
        DrumModel::Circuit,
        DrumModel::ModalAcoustic,
    ] {
        let kit = DrumKit {
            model,
            ..DrumKit::default()
        };
        for &key in &gm_keys {
            let pcm = kit
                .render_pattern(&[simple_note(0, 4800, key, 110)], 24_000)
                .unwrap();
            assert!(
                rms_i64(&pcm) > 80.0,
                "{model:?} key {key} must produce non-silent sound"
            );
        }
    }

    // Distinct spectral properties between Electronic, Circuit, and ModalAcoustic
    for &key in &[36u8, 38, 39, 42] {
        let mut pcms = Vec::new();
        for model in [
            DrumModel::Electronic,
            DrumModel::Circuit,
            DrumModel::ModalAcoustic,
        ] {
            let kit = DrumKit {
                model,
                ..DrumKit::default()
            };
            pcms.push(
                kit.render_pattern(&[simple_note(0, 6000, key, 110)], 24_000)
                    .unwrap(),
            );
        }
        assert_ne!(pcms[0], pcms[1], "Electronic vs Circuit must differ on key {key}");
        assert_ne!(pcms[1], pcms[2], "Circuit vs ModalAcoustic must differ on key {key}");
        assert_ne!(pcms[0], pcms[2], "Electronic vs ModalAcoustic must differ on key {key}");

        let zcr0 = zero_crossings_left(&pcms[0]);
        let zcr1 = zero_crossings_left(&pcms[1]);
        let zcr2 = zero_crossings_left(&pcms[2]);
        assert!(
            !(zcr0 == zcr1 && zcr1 == zcr2),
            "Models should exhibit distinct spectra/ZCR on key {key}: {zcr0}, {zcr1}, {zcr2}"
        );
    }

    // ModalAcoustic: higher velocity brightens beater/stick excitation and high cymbal partials decay faster
    let modal_kit = DrumKit {
        model: DrumModel::ModalAcoustic,
        ..DrumKit::default()
    };
    let soft_crash = modal_kit
        .render_pattern(&[simple_note(0, 4000, 49, 25)], RATE)
        .unwrap();
    let hard_crash = modal_kit
        .render_pattern(&[simple_note(0, 4000, 49, 125)], RATE)
        .unwrap();
    assert!(
        brightness_ratio(&hard_crash[..6000]) > brightness_ratio(&soft_crash[..6000]),
        "higher velocity in ModalAcoustic must brighten stick excitation"
    );
    assert!(
        brightness_ratio(&hard_crash[100..4000]) > brightness_ratio(&hard_crash[20_000..44_000]),
        "high cymbal partials must decay faster than low partials"
    );
}

#[test]
fn tine_keys_bark_pickup_asymmetry_and_stereo_tremolo() {
    let notes = [simple_note(0, 24_000, 60, 115)];
    let mono_tine = TineKeys {
        bark: 0.0,
        pickup_asym: 0.0,
        tremolo_depth: 0.0,
        ..TineKeys::default()
    };
    let bark_stereo = TineKeys {
        bark: 0.9,
        pickup_asym: 0.9,
        tremolo_rate_hz: 5.0,
        tremolo_depth: 0.8,
        ..TineKeys::default()
    };

    let a = mono_tine.render_pattern(&notes, 36_000).unwrap();
    let b = bark_stereo.render_pattern(&notes, 36_000).unwrap();

    // Tremolo creates real stereo L/R modulation difference
    let lr_diff: i64 = b.iter().map(|f| (f[0] as i32 - f[1] as i32).abs() as i64).sum();
    assert!(lr_diff > 100_000, "stereo tremolo must produce L/R difference");
    assert_ne!(a, b, "bark & pickup asymmetry must alter timbre");
}

#[test]
fn modal_grand_velocity_hammer_and_unison_inharmonicity() {
    let grand = ModalGrand::default();
    let soft = grand
        .render_pattern(&[simple_note(0, 24_000, 60, 24)], 36_000)
        .unwrap();
    let hard = grand
        .render_pattern(&[simple_note(0, 24_000, 60, 124)], 36_000)
        .unwrap();

    assert!(
        brightness_ratio(&hard[..8000]) > brightness_ratio(&soft[..8000]),
        "velocity-hardened felt hammer must make fortissimo brighter than pianissimo"
    );

    let no_body = ModalGrand {
        unison_detune: 0.0,
        body_mix: 0.0,
        ..ModalGrand::default()
    };
    let flat = no_body
        .render_pattern(&[simple_note(0, 24_000, 60, 100)], 36_000)
        .unwrap();
    let rich = grand
        .render_pattern(&[simple_note(0, 24_000, 60, 100)], 36_000)
        .unwrap();
    assert_ne!(flat, rich);
}

#[test]
fn pluck_guitar_palm_mute_decays_faster_than_sustain_and_amp_controls() {
    let notes = [simple_note(0, 36_000, 57, 110)]; // A3 (220 Hz)
    let sustain_gtr = PluckGuitar {
        mode: PluckMode::Sustained,
        ..PluckGuitar::default()
    };
    let muted_gtr = PluckGuitar {
        mode: PluckMode::PalmMuted,
        ..PluckGuitar::default()
    };

    let sustain = sustain_gtr.render_pattern(&notes, RATE).unwrap();
    let muted = muted_gtr.render_pattern(&notes, RATE).unwrap();

    let sus_early = rms_i64(&sustain[400..4000]);
    let sus_late = rms_i64(&sustain[16_000..36_000]);
    let mut_early = rms_i64(&muted[400..4000]);
    let mut_late = rms_i64(&muted[16_000..36_000]);

    assert!(sus_early > 500.0 && mut_early > 300.0);
    let sus_ratio = sus_late / sus_early;
    let mut_ratio = mut_late / mut_early;
    assert!(
        mut_ratio < sus_ratio * 0.25,
        "PalmMuted must decay much faster than Sustained (muted_ratio={mut_ratio}, sus_ratio={sus_ratio})"
    );

    // Check waveguide pitch accuracy on clean sustained A3 (220 Hz) via autocorrelation
    let clean_gtr = PluckGuitar {
        mode: PluckMode::Sustained,
        amp: GuitarAmp {
            gain: 0.0,
            bass: 0.5,
            mid: 0.5,
            treble: 0.5,
            cabinet: 0.0,
        },
        ..PluckGuitar::default()
    };
    let a3 = clean_gtr.render_pattern(&notes, RATE).unwrap();
    let seg = &a3[6000..16_000];
    let mut best_lag = 0usize;
    let mut best_corr = f64::NEG_INFINITY;
    for lag in 180usize..=260 {
        let mut corr = 0.0f64;
        for i in 0..(seg.len() - 270) {
            corr += (seg[i][0] as f64) * (seg[i + lag][0] as f64);
        }
        if corr > best_corr {
            best_corr = corr;
            best_lag = lag;
        }
    }
    let hz = RATE as f64 / best_lag as f64;
    assert!(
        (214.0..=226.0).contains(&hz),
        "PluckGuitar waveguide A3 fundamental must tune near 220 Hz, got {hz} (lag={best_lag})"
    );

    // Check GuitarAmp gain/tone stack/cabinet differences
    let hi_gain_cab = PluckGuitar {
        amp: GuitarAmp {
            gain: 0.95,
            bass: 0.85,
            mid: 0.20,
            treble: 0.80,
            cabinet: 1.0,
        },
        ..PluckGuitar::default()
    };
    let driven = hi_gain_cab.render_pattern(&notes, RATE).unwrap();
    assert_ne!(a3, driven);
    assert!(rms_i64(&driven) > 500.0);
}

#[test]
fn modal_mallet_ratios_and_hardness_control() {
    let note = simple_note(0, 24_000, 60, 100);
    let marimba = ModalMallet {
        kind: MalletKind::Marimba,
        ..ModalMallet::default()
    }
    .render_pattern(&[note], 36_000)
    .unwrap();
    let bell = ModalMallet {
        kind: MalletKind::TubularBell,
        ..ModalMallet::default()
    }
    .render_pattern(&[note], 36_000)
    .unwrap();
    let vibe = ModalMallet {
        kind: MalletKind::Vibraphone,
        ..ModalMallet::default()
    }
    .render_pattern(&[note], 36_000)
    .unwrap();

    assert_ne!(marimba, bell);
    assert_ne!(marimba, vibe);
    assert_ne!(bell, vibe);

    // Mallet hardness controlled by velocity & timbre
    let mallet = ModalMallet::default();
    let soft = mallet
        .render_expressive(
            &[ExpressiveNote::new(
                simple_note(0, 24_000, 60, 35),
                NoteExpr {
                    timbre: 0.0,
                    ..NoteExpr::default()
                },
            )],
            24_000,
        )
        .unwrap();
    let hard = mallet
        .render_expressive(
            &[ExpressiveNote::new(
                simple_note(0, 24_000, 60, 125),
                NoteExpr {
                    timbre: 1.0,
                    ..NoteExpr::default()
                },
            )],
            24_000,
        )
        .unwrap();
    assert!(
        brightness_ratio(&hard[..3000]) > brightness_ratio(&soft[..3000]),
        "harder mallet strike must excite stronger upper modal partials"
    );
}

#[test]
fn cross_synth_fm_hard_sync_ring_mod_and_expression() {
    let base = CrossSynth {
        cross_fm: 0.0,
        hard_sync: 0.0,
        ring_mod: 0.0,
        cutoff_hz: 6000.0,
        ..CrossSynth::default()
    };
    let fm = CrossSynth {
        cross_fm: 0.85,
        ..base
    };
    let sync = CrossSynth {
        hard_sync: 0.90,
        ..base
    };
    let ring = CrossSynth {
        ring_mod: 0.90,
        ..base
    };

    let n = [simple_note(0, 24_000, 60, 105)];
    let p_base = base.render_pattern(&n, 30_000).unwrap();
    let p_fm = fm.render_pattern(&n, 30_000).unwrap();
    let p_sync = sync.render_pattern(&n, 30_000).unwrap();
    let p_ring = ring.render_pattern(&n, 30_000).unwrap();

    assert_ne!(p_base, p_fm);
    assert_ne!(p_base, p_sync);
    assert_ne!(p_base, p_ring);
    assert_ne!(p_fm, p_sync);

    // Pitch bend +12 semitones doubles Osc A fundamental frequency
    let solo_a = CrossSynth {
        mix_ab: 0.0,
        cutoff_hz: 14_000.0,
        ..base
    };
    let normal = solo_a
        .render_expressive(&[ExpressiveNote::from(n[0])], 30_000)
        .unwrap();
    let bent_up = solo_a
        .render_expressive(
            &[ExpressiveNote::new(
                n[0],
                NoteExpr {
                    pitch_bend_semitones: 12.0,
                    ..NoteExpr::default()
                },
            )],
            30_000,
        )
        .unwrap();
    let zc_norm = zero_crossings_left(&normal[4000..20_000]);
    let zc_up = zero_crossings_left(&bent_up[4000..20_000]);
    assert!(
        (zc_up as f64 / zc_norm as f64 - 2.0).abs() < 0.08,
        "+12 st pitch bend must double pitch (norm={zc_norm}, up={zc_up})"
    );
}

#[test]
fn render_notes_with_params_bounds_nan_and_oom_safety() {
    let en = [ExpressiveNote::new(
        simple_note(0, 4800, 60, 100),
        NoteExpr {
            pitch_bend_semitones: 2.0,
            pressure: 0.7,
            timbre: 0.8,
            slide_from_key: None,
            glide_frames: 0,
        },
    )];

    for inst in [
        Instrument::Sub808,
        Instrument::DrumKit,
        Instrument::TineKeys,
        Instrument::ModalGrand,
        Instrument::PluckGuitar,
        Instrument::ModalMallet,
        Instrument::CrossSynth,
    ] {
        let params = InstrumentParams::for_instrument(inst).unwrap();
        let pcm = render_notes_with_params(&en, 9600, &params).unwrap();
        assert_eq!(pcm.len(), 9600);
        assert!(rms_i64(&pcm) > 50.0);

        let plain = render_pattern_with_params(&[simple_note(0, 4800, 60, 100)], 9600, &params).unwrap();
        assert_eq!(plain.len(), 9600);

        // Bounds checks
        assert_eq!(render_notes_with_params(&en, 0, &params), Err(Error::Range));
        assert_eq!(
            render_notes_with_params(&en, MAX_FRAMES + 1, &params),
            Err(Error::Range)
        );
        let too_many_notes = vec![en[0]; MAX_NOTES + 1];
        assert_eq!(
            render_notes_with_params(&too_many_notes, 9600, &params),
            Err(Error::Range)
        );
        let too_many_voices = vec![en[0]; MAX_VOICES + 1];
        assert_eq!(
            render_notes_with_params(&too_many_voices, 9600, &params),
            Err(Error::Polyphony)
        );
        let bad_key = [ExpressiveNote::from(simple_note(0, 1000, 128, 100))];
        assert_eq!(
            render_notes_with_params(&bad_key, 9600, &params),
            Err(Error::Note)
        );
        let bad_vel = [ExpressiveNote::from(simple_note(0, 1000, 60, 0))];
        assert_eq!(
            render_notes_with_params(&bad_vel, 9600, &params),
            Err(Error::Note)
        );
        let bad_len = [ExpressiveNote::from(simple_note(0, 0, 60, 100))];
        assert_eq!(
            render_notes_with_params(&bad_len, 9600, &params),
            Err(Error::Note)
        );
        let overflow_note = [ExpressiveNote::from(simple_note(u32::MAX, 100, 60, 100))];
        assert_eq!(
            render_notes_with_params(&overflow_note, 9600, &params),
            Err(Error::Note)
        );
    }

    assert!(InstrumentParams::for_instrument(Instrument::SilkTriangle).is_none());

    // NaN / Infinity inside NoteExpr and parameters must sanitize cleanly without panic
    let nan_expr = [ExpressiveNote::new(
        simple_note(0, 4800, 60, 100),
        NoteExpr {
            pitch_bend_semitones: f32::NAN,
            pressure: f32::INFINITY,
            timbre: f32::NEG_INFINITY,
            slide_from_key: Some(250),
            glide_frames: u32::MAX,
        },
    )];
    let nan_sub = InstrumentParams::Sub808(Sub808 {
        shape: f32::NAN,
        drive: f32::INFINITY,
        tone_hz: f32::NEG_INFINITY,
        ..Sub808::default()
    });
    let safe_pcm = render_notes_with_params(&nan_expr, 4800, &nan_sub).unwrap();
    assert_eq!(safe_pcm.len(), 4800);
}
