// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive integration test suite for Moan's rational musical time engine,
//! TempoMap analytical integration, ComposeTool transforms, Tapestry Channel Rack
//! & SceneMatrix, RetrospectiveNoteBuffer, and zero-dependency SMF (.mid) codec.

extern crate alloc;

use alloc::{sync::Arc, vec::Vec};
use moan_core::{
    compose::{
        apply_tool, apply_tool_to_midi, midi_pattern_to_notes, notes_to_midi_pattern, ArpOrder,
        ComposeTool, ExpressiveNote, StrumDir,
    },
    dsp_math::abs_f32,
    midi::{Instrument, Note as MidiNote, Pattern as MidiPattern},
    pattern::{
        ChannelPreviewMode, PatternId, PatternInstance, RetrospectiveNote, RetrospectiveNoteBuffer,
        SceneMatrix, StepCell, TapestryBank, TransportContext, MAX_RETROSPECTIVE_NOTES,
        SCENE_COUNT, SCENE_LANES,
    },
    smf::{
        export_smf_bank, export_smf_midi_pattern, export_smf_pattern, import_smf, SmfError,
        MAX_SMF_BYTES, MAX_SMF_NOTES,
    },
    time::{TempoMap, TempoSegment, Time, TimeSignature, DISPLAY_PPQ, MAX_DENOMINATOR},
    RATE,
};

#[test]
fn rational_time_zero_drift_over_ten_thousand_triplets_and_subdivisions() {
    // 10,000 triplet eighth additions (1/3 beat each) must remain exact with den == 3 or 1.
    let mut t = Time::ZERO;
    let triplet = Time::TRIPLET_EIGHTH;
    for _ in 0..10_000 {
        t = t.checked_add(triplet).unwrap();
    }
    assert_eq!(t, Time::new(10_000, 3).unwrap());
    assert_eq!(t.num, 10_000);
    assert_eq!(t.den, 3);

    // 30,000 triplet eighths = exact 10,000 beats (den reduces to 1).
    let mut whole = Time::ZERO;
    for _ in 0..30_000 {
        whole = whole.checked_add(triplet).unwrap();
    }
    assert_eq!(whole, Time::from_beats(10_000));
    assert_eq!(whole.den, 1);

    // 64th rolls (1/16 beat), 1/12 triplets, and 960-PPQ ticks never drift.
    let mut roll = Time::ZERO;
    for _ in 0..16_000 {
        roll = roll.checked_add(Time::SIXTY_FOURTH).unwrap();
    }
    assert_eq!(roll, Time::from_beats(1_000));

    let one_tick = Time::from_display_ticks(1);
    assert_eq!(one_tick, Time::new(1, DISPLAY_PPQ as u32).unwrap());
    let mut ticks_acc = Time::ZERO;
    for _ in 0..9_600 {
        ticks_acc = ticks_acc.checked_add(one_tick).unwrap();
    }
    assert_eq!(ticks_acc, Time::from_beats(10));

    // Arithmetic & Ord / Eq checks (including unreduced manual comparison).
    let unreduced = Time { num: 2, den: 4 };
    assert_eq!(unreduced, Time::EIGHTH);
    assert!(Time::TRIPLET_EIGHTH > Time::SIXTEENTH);
    assert!(Time::TRIPLET_THIRTY_SECOND < Time::TRIPLET_SIXTEENTH);

    let prod = Time::TRIPLET_THIRTY_SECOND.mul_int(12).unwrap();
    assert_eq!(prod, Time::ONE);
    assert_eq!(Time::ONE.div_int(6).unwrap(), Time::TRIPLET_SIXTEENTH);
    assert_eq!(
        prod.checked_sub(Time::TRIPLET_EIGHTH).unwrap(),
        Time::TRIPLET_QUARTER
    );

    // MAX_DENOMINATOR enforcement.
    assert!(Time::new(1, MAX_DENOMINATOR).is_some());
    assert!(Time::new(1, MAX_DENOMINATOR + 1).is_none());
    assert!(Time::new(1, 0).is_none());
    assert!(Time::new(1, MAX_DENOMINATOR).unwrap().div_int(2).is_none());
}

#[test]
fn time_signature_and_tempo_map_linear_ramp_and_swing_analytical_integration() {
    assert_eq!(TimeSignature::FOUR_FOUR.beats_per_bar(), Time::from_beats(4));
    assert_eq!(TimeSignature::THREE_FOUR.beats_per_bar(), Time::from_beats(3));
    assert_eq!(TimeSignature::FIVE_FOUR.beats_per_bar(), Time::from_beats(5));
    assert_eq!(TimeSignature::SIX_EIGHT.beats_per_bar(), Time::from_beats(3));
    assert_eq!(
        TimeSignature::SEVEN_EIGHT.beats_per_bar(),
        Time::new(7, 2).unwrap()
    );

    // Constant 120 BPM exact conversions.
    let constant = TempoMap::constant(12_000).unwrap();
    assert_eq!(constant.beat_to_frame(Time::ONE, 0), 24_000);
    assert_eq!(constant.beat_to_frame(Time::TRIPLET_EIGHTH, 0), 8_000);
    assert_eq!(
        constant.frame_to_beat(8_000, 0),
        Time::TRIPLET_EIGHTH
    );

    // Swing delays odd 16th subdivisions cleanly while leaving even 16ths anchored.
    // 50% swing delays 1/4 beat by 50% of a 16th -> 3/8 beat -> 9,000 frames at 120 BPM.
    assert_eq!(constant.beat_to_frame(Time::ZERO, 50), 0);
    assert_eq!(constant.beat_to_frame(Time::SIXTEENTH, 50), 9_000);
    assert_eq!(constant.beat_to_frame(Time::EIGHTH, 50), 12_000);
    assert_eq!(constant.frame_to_beat(9_000, 50), Time::SIXTEENTH);
    // Strict monotonicity across 75% max swing.
    let f_a = constant.beat_to_frame(Time::SIXTEENTH, 75);
    let f_b = constant.beat_to_frame(Time::new(3, 8).unwrap(), 75);
    let f_c = constant.beat_to_frame(Time::EIGHTH, 75);
    assert!(f_a < f_b && f_b < f_c);
    assert_eq!(f_c, 12_000);

    // Linear BPM ramp: 120.00 BPM -> 180.00 BPM over 4 beats (k = 15 BPM/beat).
    // Compare analytical result against high-resolution numerical integration.
    let ramp = TempoMap::new(
        alloc::vec![
            TempoSegment {
                start: Time::ZERO,
                bpm_x100: 12_000,
                ramp_to_bpm_x100: Some(18_000),
            },
            TempoSegment {
                start: Time::from_beats(4),
                bpm_x100: 18_000,
                ramp_to_bpm_x100: None,
            },
        ],
        TimeSignature::FOUR_FOUR,
    )
    .unwrap();

    let steps = 20_000usize;
    let dq = 4.0f64 / steps as f64;
    let mut numeric_secs = 0.0f64;
    for i in 0..steps {
        let q = (i as f64 + 0.5) * dq;
        let bpm = 120.0 + 15.0 * q;
        numeric_secs += (60.0 / bpm) * dq;
    }
    let expected_frame = (numeric_secs * RATE as f64 + 0.5) as u64;
    let actual_frame = ramp.beat_to_frame(Time::from_beats(4), 0);
    let diff = actual_frame.abs_diff(expected_frame);
    assert!(
        diff <= 2,
        "analytical ramp frame {actual_frame} differed from numerical {expected_frame} by {diff}"
    );

    // Inverse frame_to_beat inside the ramp recovers midpoint beat accurately.
    let mid_frame = ramp.beat_to_frame(Time::from_beats(2), 0);
    let recovered = ramp.frame_to_beat(mid_frame, 0);
    assert!(
        abs_f32(recovered.to_f32() - 2.0) < 1e-3,
        "recovered {recovered:?}"
    );

    // Tiny slope |k| < 1e-4 fallback avoids 0/0 cancellation.
    let flat_ramp = TempoMap::new(
        alloc::vec![
            TempoSegment {
                start: Time::ZERO,
                bpm_x100: 12_000,
                ramp_to_bpm_x100: Some(12_001),
            },
            TempoSegment {
                start: Time::from_beats(200),
                bpm_x100: 12_001,
                ramp_to_bpm_x100: None,
            },
        ],
        TimeSignature::FOUR_FOUR,
    )
    .unwrap();
    let flat_secs = flat_ramp.beat_to_seconds(Time::from_beats(200), 0);
    assert!(abs_f32(flat_secs as f32 - 100.0) < 0.02);
}

#[test]
fn compose_tools_all_ten_transforms_and_midi_bridge() {
    // 1. Legato
    let chord_seq = [
        ExpressiveNote::new(1, Time::ZERO, Time::SIXTEENTH, 60, 100),
        ExpressiveNote::new(2, Time::ZERO, Time::SIXTEENTH, 64, 100),
        ExpressiveNote::new(3, Time::ONE, Time::SIXTEENTH, 67, 100),
    ];
    let legato = apply_tool(
        &chord_seq,
        &ComposeTool::Legato {
            gap: Time::SIXTY_FOURTH,
        },
    )
    .unwrap();
    assert_eq!(legato[0].length, Time::new(15, 16).unwrap());
    assert_eq!(legato[1].length, Time::new(15, 16).unwrap());
    assert_eq!(legato[2].length, Time::SIXTEENTH);

    // 2. Articulate (staccato + min clamp >= 1/64 beat)
    let artic = apply_tool(&chord_seq, &ComposeTool::Articulate { scale_pct: 50 }).unwrap();
    assert_eq!(artic[0].length, Time::THIRTY_SECOND);
    let tiny_artic = apply_tool(
        &[ExpressiveNote::new(1, Time::ZERO, Time::THIRTY_SECOND, 60, 90)],
        &ComposeTool::Articulate { scale_pct: 1 },
    )
    .unwrap();
    assert_eq!(tiny_artic[0].length, Time::MIN_BEAT_STEP);

    // 3. Chop (1/12 triplet rolls)
    let long_note = [ExpressiveNote::new(1, Time::ZERO, Time::ONE, 72, 110)];
    let chopped = apply_tool(
        &long_note,
        &ComposeTool::Chop {
            division: Time::TRIPLET_THIRTY_SECOND,
            gate_pct: 75,
        },
    )
    .unwrap();
    assert_eq!(chopped.len(), 12);
    assert_eq!(chopped[1].start, Time::TRIPLET_THIRTY_SECOND);
    assert_eq!(chopped[0].length, Time::new(1, 16).unwrap());

    // 4. Arpeggiate (Up, Down, UpDown, deterministic Random)
    let triad = [
        ExpressiveNote::new(1, Time::ZERO, Time::from_beats(2), 60, 100),
        ExpressiveNote::new(2, Time::ZERO, Time::from_beats(2), 64, 100),
        ExpressiveNote::new(3, Time::ZERO, Time::from_beats(2), 67, 100),
    ];
    let arp_up = apply_tool(
        &triad,
        &ComposeTool::Arpeggiate {
            step: Time::SIXTEENTH,
            order: ArpOrder::Up,
            octaves: 2,
            gate_pct: 80,
            seed: 42,
        },
    )
    .unwrap();
    assert_eq!(arp_up.len(), 8);
    let up_keys: Vec<u8> = arp_up.iter().map(|n| n.key).collect();
    assert_eq!(up_keys, [60, 64, 67, 72, 76, 79, 60, 64]);

    let arp_down = apply_tool(
        &triad,
        &ComposeTool::Arpeggiate {
            step: Time::EIGHTH,
            order: ArpOrder::Down,
            octaves: 1,
            gate_pct: 100,
            seed: 1,
        },
    )
    .unwrap();
    let down_keys: Vec<u8> = arp_down.iter().map(|n| n.key).collect();
    assert_eq!(down_keys, [67, 64, 60, 67]);

    let arp_ud = apply_tool(
        &triad,
        &ComposeTool::Arpeggiate {
            step: Time::EIGHTH,
            order: ArpOrder::UpDown,
            octaves: 1,
            gate_pct: 100,
            seed: 1,
        },
    )
    .unwrap();
    let ud_keys: Vec<u8> = arp_ud.iter().map(|n| n.key).collect();
    assert_eq!(ud_keys, [60, 64, 67, 64]);

    let arp_rand_a = apply_tool(
        &triad,
        &ComposeTool::Arpeggiate {
            step: Time::SIXTEENTH,
            order: ArpOrder::Random,
            octaves: 2,
            gate_pct: 100,
            seed: 99,
        },
    )
    .unwrap();
    let arp_rand_b = apply_tool(
        &triad,
        &ComposeTool::Arpeggiate {
            step: Time::SIXTEENTH,
            order: ArpOrder::Random,
            octaves: 2,
            gate_pct: 100,
            seed: 99,
        },
    )
    .unwrap();
    assert_eq!(arp_rand_a, arp_rand_b);

    // 5. Strum (spreads onsets while keeping note ends intact)
    let strum_in = [
        ExpressiveNote::new(1, Time::ZERO, Time::ONE, 60, 100),
        ExpressiveNote::new(2, Time::ZERO, Time::ONE, 64, 100),
        ExpressiveNote::new(3, Time::ZERO, Time::ONE, 67, 100),
    ];
    let strummed = apply_tool(
        &strum_in,
        &ComposeTool::Strum {
            spread: Time::EIGHTH,
            direction: StrumDir::Up,
            vel_falloff_pct: 20,
        },
    )
    .unwrap();
    assert_eq!(strummed[0].start, Time::ZERO);
    assert_eq!(strummed[1].start, Time::SIXTEENTH);
    assert_eq!(strummed[2].start, Time::EIGHTH);
    for n in &strummed {
        assert_eq!(n.end().unwrap(), Time::ONE);
    }
    assert!(strummed[2].velocity < strummed[0].velocity);

    // 6. Flam (adds grace hits before note)
    let snare = [ExpressiveNote::new(1, Time::ONE, Time::EIGHTH, 38, 100)];
    let flammed = apply_tool(
        &snare,
        &ComposeTool::Flam {
            count: 2,
            spacing: Time::SIXTY_FOURTH,
            vel_pct: 50,
        },
    )
    .unwrap();
    assert_eq!(flammed.len(), 3);
    assert_eq!(flammed[0].start, Time::new(7, 8).unwrap());
    assert_eq!(flammed[0].velocity, 25);
    assert_eq!(flammed[1].start, Time::new(15, 16).unwrap());
    assert_eq!(flammed[1].velocity, 50);
    assert_eq!(flammed[2].start, Time::ONE);
    assert_eq!(flammed[2].velocity, 100);

    // 7. Humanize (deterministic & bounded)
    let hum_a = apply_tool(
        &snare,
        &ComposeTool::Humanize {
            max_timing_ticks: 20,
            max_vel_delta: 12,
            max_len_pct: 15,
            seed: 777,
        },
    )
    .unwrap();
    let hum_b = apply_tool(
        &snare,
        &ComposeTool::Humanize {
            max_timing_ticks: 20,
            max_vel_delta: 12,
            max_len_pct: 15,
            seed: 777,
        },
    )
    .unwrap();
    assert_eq!(hum_a, hum_b);
    let dt = (hum_a[0].start.to_display_ticks().unwrap() - DISPLAY_PPQ).abs();
    assert!(dt <= 20);
    assert!((hum_a[0].velocity as i16 - 100).abs() <= 12);

    // 8. Levels
    let leveled = apply_tool(
        &snare,
        &ComposeTool::Levels {
            multiply_pct: 150,
            offset: -20,
        },
    )
    .unwrap();
    assert_eq!(leveled[0].velocity, 127);

    // 9. Flip (involution for both time and pitch)
    let melody = [
        ExpressiveNote::new(1, Time::ZERO, Time::EIGHTH, 60, 100),
        ExpressiveNote::new(2, Time::from_beats(3), Time::ONE, 72, 100),
    ];
    let flipped = apply_tool(
        &melody,
        &ComposeTool::Flip {
            reverse_time: true,
            mirror_pitch: true,
        },
    )
    .unwrap();
    assert_eq!(flipped[0].key, 60); // 72 mirrored around [60, 72] -> 60 at beat 0
    assert_eq!(flipped[0].start, Time::ZERO);
    assert_eq!(flipped[1].key, 72);
    assert_eq!(flipped[1].start, Time::new(7, 2).unwrap());
    let unflipped = apply_tool(
        &flipped,
        &ComposeTool::Flip {
            reverse_time: true,
            mirror_pitch: true,
        },
    )
    .unwrap();
    assert_eq!(unflipped[0].start, melody[0].start);
    assert_eq!(unflipped[0].key, melody[0].key);
    assert_eq!(unflipped[1].start, melody[1].start);
    assert_eq!(unflipped[1].key, melody[1].key);

    // 10. Tidy (removes exact dupes, trims same-pitch overlaps, keeps cross-pitch slide overlaps)
    let mut slide_note = ExpressiveNote::new(10, Time::ZERO, Time::from_beats(2), 60, 100);
    slide_note.slide_to_key = Some(64);
    let messy = [
        slide_note,
        ExpressiveNote::new(11, Time::ZERO, Time::from_beats(2), 60, 90), // exact dupe (same onset & key)
        ExpressiveNote::new(12, Time::ONE, Time::ONE, 60, 100), // same-pitch overlap -> trims id 10 to len 1
        ExpressiveNote::new(13, Time::EIGHTH, Time::ONE, 64, 100), // cross-pitch overlap preserved!
    ];
    let tidy = apply_tool(&messy, &ComposeTool::Tidy).unwrap();
    assert_eq!(tidy.len(), 3);
    let c4 = tidy.iter().find(|n| n.key == 60 && n.start == Time::ZERO).unwrap();
    assert_eq!(c4.length, Time::ONE);
    let e4 = tidy.iter().find(|n| n.key == 64).unwrap();
    assert_eq!(e4.start, Time::EIGHTH);
    assert_eq!(e4.length, Time::ONE);

    // Lossless ExpressiveNote <-> midi::Note round-trip and apply_tool_to_midi.
    let tempo = TempoMap::constant(12_000).unwrap();
    let midi_pat = notes_to_midi_pattern(
        &[ExpressiveNote::new(1, Time::ZERO, Time::ONE, 69, 96)],
        Time::from_beats(2),
        Instrument::SilkTriangle,
        &tempo,
        0,
    )
    .unwrap();
    let (back, _, total_beats) = midi_pattern_to_notes(&midi_pat).unwrap();
    assert_eq!(total_beats, Time::from_beats(2));
    assert_eq!(back[0].start, Time::ZERO);
    assert_eq!(back[0].length, Time::ONE);
    assert_eq!(back[0].to_midi_note(&tempo, 0).unwrap(), MidiNote {
        start: 0,
        frames: 24_000,
        key: 69,
        velocity: 96,
    });

    let chopped_midi = apply_tool_to_midi(
        &midi_pat,
        &ComposeTool::Chop {
            division: Time::SIXTEENTH,
            gate_pct: 50,
        },
    )
    .unwrap();
    assert_eq!(chopped_midi.notes.len(), 4);
    assert!(chopped_midi.bake().is_ok());

    // Ensure MidiPattern constructor sanity
    let _: MidiPattern = chopped_midi;
}

#[test]
fn tapestry_bank_step_piano_unity_unique_instances_and_cut_groups() {
    let mut bank = TapestryBank::new();
    let open_hat = bank.add_channel("Open Hat", Instrument::VelvetSampler as u8).unwrap();
    let closed_hat = bank.add_channel("Closed Hat", Instrument::VelvetSampler as u8).unwrap();
    let bass = bank.add_channel("Satin Bass", Instrument::SatinPulse as u8).unwrap();

    bank.channel_mut(open_hat).unwrap().cut_group = 1;
    bank.channel_mut(closed_hat).unwrap().cut_group = 1;
    bank.channel_mut(bass).unwrap().cut_group = 0;

    let pat_a = bank.create_pattern("Main Beat", 1, 4).unwrap();
    {
        let p = bank.pattern_mut(pat_a).unwrap();
        // Open Hat at beat 0 lasting 2 beats (frames 0..48_000 at 120 BPM)
        p.add_note(
            open_hat,
            ExpressiveNote::new(1, Time::ZERO, Time::from_beats(2), 46, 110),
        )
        .unwrap();
        // Closed Hat via step grid toggle at step 2 = beat 1/2 (12_000 frames)
        assert!(p.toggle_step(closed_hat, 2, 42, 100).unwrap());
        // Verify StepGrid reads back the exact toggled note
        let grid = p.step_grid(closed_hat, 42, TimeSignature::FOUR_FOUR).unwrap();
        assert_eq!(grid.len(), 16);
        assert!(grid[2].active);
        assert_eq!(grid[2].key, 42);
        assert!(!grid[0].active);

        // Ratchet roll via set_step creates 3 underlying notes and reads back repeat_subdiv == 3
        p.set_step(
            closed_hat,
            4,
            StepCell {
                active: true,
                key: 42,
                velocity: 90,
                pan: 0,
                fine_cents: 0,
                repeat_subdiv: 3,
                slide: false,
            },
        )
        .unwrap();
        let grid_after = p.step_grid(closed_hat, 42, TimeSignature::FOUR_FOUR).unwrap();
        assert_eq!(grid_after[4].repeat_subdiv, 3);
        assert_eq!(p.channel_notes(closed_hat).len(), 4);
        assert_eq!(p.preview_mode(closed_hat, 42), ChannelPreviewMode::StepGrid);

        // Bass chord / varied pitch switches preview mode to MiniRoll
        p.add_note(
            bass,
            ExpressiveNote::new(10, Time::ZERO, Time::ONE, 36, 100),
        )
        .unwrap();
        p.add_note(
            bass,
            ExpressiveNote::new(11, Time::ONE, Time::ONE, 43, 100),
        )
        .unwrap();
        assert_eq!(p.preview_mode(bass, 36), ChannelPreviewMode::MiniRoll);
    }

    // Verify Cut/Choke group in PatternMode: Open Hat (start 0, natural end 48_000) is choked
    // by Closed Hat at beat 1/2 (start 12_000)!
    let realized = bank.realize_events(TransportContext::PatternMode).unwrap();
    let oh_event = realized.iter().find(|e| e.channel == open_hat).unwrap();
    assert_eq!(oh_event.start_frame, 0);
    assert_eq!(oh_event.end_frame, 12_000);
    assert_eq!(oh_event.end_beat, Time::EIGHTH);
    // Bass in cut_group = 0 is unaffected
    let bass_ev = realized.iter().find(|e| e.channel == bass && e.key == 36).unwrap();
    assert_eq!(bass_ev.end_frame, 24_000);

    // Linked PatternInstances & make_instance_unique isolation
    let idx0 = bank
        .place_instance(PatternInstance {
            pattern: pat_a,
            start_beat: Time::ZERO,
            length_beats: Time::from_beats(4),
            offset_beats: Time::ZERO,
            lane: 0,
        })
        .unwrap();
    let idx1 = bank
        .place_instance(PatternInstance {
            pattern: pat_a,
            start_beat: Time::from_beats(4),
            length_beats: Time::from_beats(4),
            offset_beats: Time::ZERO,
            lane: 0,
        })
        .unwrap();
    assert_eq!(idx0, 0);
    assert_eq!(idx1, 1);

    // Copy-on-write Arc reference count starts shared when untouched
    assert_eq!(Arc::strong_count(&bank.patterns[0]), 1);

    // Make ONLY instance 1 unique, then mutate instance 1's pattern bass pitch
    let pat_b = bank.make_instance_unique(idx1).unwrap();
    assert_ne!(pat_a, pat_b);
    assert_eq!(bank.instances[idx0].pattern, pat_a);
    assert_eq!(bank.instances[idx1].pattern, pat_b);

    {
        let p_b = bank.pattern_mut(pat_b).unwrap();
        let bass_notes = p_b.channel_notes_mut(bass).unwrap();
        bass_notes[0].key = 48;
    }

    let song = bank.realize_events(TransportContext::SongMode).unwrap();
    let bar1_bass = song
        .iter()
        .find(|e| e.channel == bass && e.start_beat == Time::ZERO)
        .unwrap();
    let bar2_bass = song
        .iter()
        .find(|e| e.channel == bass && e.start_beat == Time::from_beats(4))
        .unwrap();
    assert_eq!(bar1_bass.key, 36);
    assert_eq!(bar2_bass.key, 48);

    // Split pattern by channel produces 3 isolated single-channel patterns
    let split = bank.split_pattern_by_channel(pat_a).unwrap();
    assert_eq!(split.len(), 3);
    for pid in split {
        let sp = bank.pattern(pid).unwrap();
        assert_eq!(sp.channel_notes.len(), 1);
    }

    // Solo / Mute filtering
    bank.channel_mut(bass).unwrap().solo = true;
    let solo_only = bank.realize_events(TransportContext::PatternMode).unwrap();
    assert!(solo_only.iter().all(|e| e.channel == bass));
    bank.channel_mut(bass).unwrap().muted = true;
    let muted_solo = bank.realize_events(TransportContext::PatternMode).unwrap();
    assert_ne!(muted_solo.len(), solo_only.len());
}

#[test]
fn scene_matrix_bar_quantized_launch_and_retrospective_note_buffer() {
    let mut bank = TapestryBank::new();
    let lead = bank.add_channel("Prism Lead", Instrument::PrismWavetable as u8).unwrap();
    let pat0 = bank.create_pattern("Verse", 2, 4).unwrap();
    bank.pattern_mut(pat0)
        .unwrap()
        .add_note(lead, ExpressiveNote::new(1, Time::ZERO, Time::ONE, 60, 100))
        .unwrap();

    let mut matrix = SceneMatrix::new();
    assert_eq!(matrix.scenes.len(), SCENE_COUNT);
    assert_eq!(matrix.scenes[0].slots.len(), SCENE_LANES);
    matrix.set_slot(0, 2, Some(pat0)).unwrap();

    // Queue at frame 24_000 (beat 1 of bar 1 in 4/4 @ 120 BPM).
    // Must schedule at exact next bar boundary: beat 4 (frame 96_000).
    let q = matrix.queue_scene_for_bank(0, 24_000, &bank).unwrap();
    assert_eq!(q.launch_beat, Time::from_beats(4));
    assert_eq!(q.launch_frame, 96_000);

    // Before launch frame: nothing recorded yet.
    assert_eq!(matrix.advance_to(95_999, &mut bank).unwrap(), 0);
    assert!(bank.instances.is_empty());

    // At launch frame: records PatternInstance into TapestryBank!
    assert_eq!(matrix.advance_to(96_000, &mut bank).unwrap(), 1);
    assert_eq!(matrix.active_scene, Some(0));
    assert!(matrix.queued.is_none());
    assert_eq!(bank.instances.len(), 1);
    assert_eq!(bank.instances[0].pattern, pat0);
    assert_eq!(bank.instances[0].start_beat, Time::from_beats(4));
    assert_eq!(bank.instances[0].length_beats, Time::from_beats(8));
    assert_eq!(bank.instances[0].lane, 2);

    // RetrospectiveNoteBuffer bounded ring (<= 512 notes) & riff recovery
    let mut retro = RetrospectiveNoteBuffer::new();
    for i in 0..600u64 {
        retro
            .push_note(RetrospectiveNote {
                channel: lead,
                start_frame: i * 12_000,
                length_frames: 6_000,
                key: 60 + (i % 12) as u8,
                velocity: 95,
            })
            .unwrap();
    }
    assert_eq!(retro.len(), MAX_RETROSPECTIVE_NOTES);
    assert_eq!(retro.iter().next().unwrap().start_frame, 88 * 12_000);

    // Also test note_on / note_off live capture & quantized recovery
    retro.clear();
    retro.note_on(lead, 2_000, 64, 105).unwrap(); // slight human timing offset from 0
    assert!(retro.note_off(lead, 26_000, 64).unwrap());
    retro.note_on(lead, 24_100, 67, 100).unwrap(); // near beat 1
    assert!(retro.note_off(lead, 48_100, 67).unwrap());

    let recovered = retro
        .recover_to_pattern(
            PatternId(99),
            "Recovered Riff",
            &bank.tempo_map,
            0,
            Some(Time::SIXTEENTH),
        )
        .unwrap();
    let rec_notes = recovered.channel_notes(lead);
    assert_eq!(rec_notes.len(), 2);
    assert_eq!(rec_notes[0].start, Time::ZERO);
    assert_eq!(rec_notes[0].key, 64);
    assert_eq!(rec_notes[1].start, Time::ONE);
    assert_eq!(rec_notes[1].key, 67);
}

#[test]
fn smf_format1_roundtrip_and_format0_running_status_plus_truncation_fuzz() {
    let mut bank = TapestryBank::new();
    bank.tempo_map = TempoMap::new(
        alloc::vec![
            TempoSegment {
                start: Time::ZERO,
                bpm_x100: 12_800,
                ramp_to_bpm_x100: None,
            },
            TempoSegment {
                start: Time::from_beats(3),
                bpm_x100: 14_000,
                ramp_to_bpm_x100: None,
            },
        ],
        TimeSignature::THREE_FOUR,
    )
    .unwrap();

    let keys_ch = bank.add_channel("Silk Keys", Instrument::SilkTriangle as u8).unwrap();
    let drums_ch = bank.add_channel("Velvet Kit", Instrument::VelvetSampler as u8).unwrap();
    let pat_id = bank.create_pattern("Waltz", 2, 4).unwrap();
    {
        let p = bank.pattern_mut(pat_id).unwrap();
        p.add_note(
            keys_ch,
            ExpressiveNote::new(1, Time::ZERO, Time::ONE, 60, 100),
        )
        .unwrap();
        p.add_note(
            keys_ch,
            ExpressiveNote::new(2, Time::TRIPLET_EIGHTH, Time::TRIPLET_EIGHTH, 64, 92),
        )
        .unwrap();
        p.add_note(
            drums_ch,
            ExpressiveNote::new(3, Time::ONE, Time::SIXTEENTH, 36, 120),
        )
        .unwrap();
    }
    bank.active_pattern = pat_id;

    let bytes = export_smf_bank(&bank).unwrap();
    assert_eq!(&bytes[0..4], b"MThd");
    let imported = import_smf(&bytes).unwrap();
    assert_eq!(imported.format, 1);
    assert_eq!(imported.division_ppq, DISPLAY_PPQ as u16);
    assert_eq!(imported.tracks.len(), 3); // conductor + keys + drums
    assert_eq!(
        imported.tempo_map.time_signature_at(Time::ZERO),
        TimeSignature::THREE_FOUR
    );
    assert_eq!(imported.tempo_map.bpm_x100_at(Time::ZERO), 12_800);
    assert_eq!(imported.tempo_map.bpm_x100_at(Time::from_beats(3)), 14_000);

    let trk_keys = &imported.tracks[1];
    assert_eq!(trk_keys.name, "Silk Keys");
    assert!(!trk_keys.is_percussion);
    assert_eq!(trk_keys.notes.len(), 2);
    assert_eq!(trk_keys.notes[0].key, 60);
    assert_eq!(trk_keys.notes[0].start, Time::ZERO);
    assert_eq!(trk_keys.notes[0].length, Time::ONE);
    assert_eq!(trk_keys.notes[1].start, Time::TRIPLET_EIGHTH);

    let trk_drums = &imported.tracks[2];
    assert_eq!(trk_drums.name, "Velvet Kit");
    assert!(trk_drums.is_percussion);
    assert_eq!(trk_drums.midi_channel, Some(9));
    assert_eq!(trk_drums.notes[0].key, 36);
    assert_eq!(trk_drums.notes[0].velocity, 120);

    // export_smf_pattern & export_smf_midi_pattern
    let pat_bytes = export_smf_pattern(
        bank.pattern(pat_id).unwrap(),
        &bank.channels,
        &bank.tempo_map,
    )
    .unwrap();
    assert_eq!(import_smf(&pat_bytes).unwrap().tracks.len(), 3);

    let midi_pat = MidiPattern::preset(Instrument::PrismWavetable);
    let midi_bytes = export_smf_midi_pattern(&midi_pat).unwrap();
    assert_eq!(import_smf(&midi_bytes).unwrap().tracks[1].notes.len(), 8);

    // Truncation fuzz: every single prefix [0..bytes.len()) must fail cleanly without panicking.
    for cut in 0..bytes.len() {
        let res = import_smf(&bytes[..cut]);
        assert!(res.is_err(), "truncated length {cut} unexpectedly succeeded");
    }

    // Hand-crafted Format 0 SMF with Running Status, velocity-0 NoteOff, PitchBend, and ch 10 percussion.
    #[rustfmt::skip]
    let format0_trk: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // delta 0: SetTempo 500_000 us = 120 BPM
        0x00, 0x99, 0x24, 0x7F,                   // delta 0: NoteOn ch 9 (percussion), key 36, vel 127
        0x60, 0x24, 0x00,                         // delta 96 (1/4 beat @ 384 PPQ): RUNNING STATUS vel-0 NoteOff!
        0x00, 0xE0, 0x00, 0x60,                   // delta 0: PitchBend ch 0 (+4096)
        0x00, 0xFF, 0x2F, 0x00,                   // End of Track
    ];
    let mut format0 = Vec::new();
    format0.extend_from_slice(b"MThd");
    format0.extend_from_slice(&6u32.to_be_bytes());
    format0.extend_from_slice(&0u16.to_be_bytes()); // Format 0
    format0.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    format0.extend_from_slice(&384u16.to_be_bytes()); // 384 PPQ
    format0.extend_from_slice(b"MTrk");
    format0.extend_from_slice(&(format0_trk.len() as u32).to_be_bytes());
    format0.extend_from_slice(format0_trk);

    let f0 = import_smf(&format0).unwrap();
    assert_eq!(f0.format, 0);
    assert_eq!(f0.division_ppq, 384);
    assert!(f0.tracks[0].is_percussion);
    assert_eq!(f0.tracks[0].notes.len(), 1);
    assert_eq!(f0.tracks[0].notes[0].length, Time::SIXTEENTH);
    assert_eq!(f0.tracks[0].pitch_bends, alloc::vec![(Time::SIXTEENTH, 4096)]);
    assert!(f0.to_tapestry_bank().is_ok());

    // Malformed / oversized rejection checks.
    assert_eq!(import_smf(b"RIFF1234567890"), Err(SmfError::InvalidHeader));
    let mut smpte = format0.clone();
    smpte[12] = 0xE7; // SMPTE bit set
    assert_eq!(import_smf(&smpte), Err(SmfError::InvalidTiming));
    let mut fmt2 = format0.clone();
    fmt2[9] = 2;
    assert_eq!(import_smf(&fmt2), Err(SmfError::UnsupportedFormat));
    let mut too_many_tracks = format0.clone();
    too_many_tracks[11] = 33;
    assert_eq!(import_smf(&too_many_tracks), Err(SmfError::TooManyTracks));

    // > 4 MiB rejected immediately
    let huge = alloc::vec![0u8; MAX_SMF_BYTES + 1];
    assert_eq!(import_smf(&huge), Err(SmfError::TooLarge));
}

#[test]
fn extra_edge_cases_decelerando_flam_at_zero_and_smf_limits() {
    // Decelerando 180 -> 120 BPM over 4 beats matches numerical integral & accelerando duration.
    let decel = TempoMap::new(
        alloc::vec![
            TempoSegment {
                start: Time::ZERO,
                bpm_x100: 18_000,
                ramp_to_bpm_x100: Some(12_000),
            },
            TempoSegment {
                start: Time::from_beats(4),
                bpm_x100: 12_000,
                ramp_to_bpm_x100: None,
            },
        ],
        TimeSignature::SEVEN_EIGHT,
    )
    .unwrap();
    let f4 = decel.beat_to_frame(Time::from_beats(4), 0);
    assert!(f4.abs_diff(77_849) <= 2, "decel frame {f4}");

    // 7/8 bar-quantized SceneMatrix launch: 7/8 bar = 7/2 beats (3.5 beats).
    let mut bank = TapestryBank::new();
    bank.tempo_map = decel;
    let ch = bank.add_channel("Keys", Instrument::SilkTriangle as u8).unwrap();
    let pat = bank.create_pattern("OddMeter", 1, 4).unwrap();
    bank.pattern_mut(pat)
        .unwrap()
        .add_note(ch, ExpressiveNote::new(1, Time::ZERO, Time::ONE, 60, 100))
        .unwrap();
    let mut matrix = SceneMatrix::new();
    matrix.set_slot(1, 0, Some(pat)).unwrap();
    let q = matrix.queue_scene_for_bank(1, 10_000, &bank).unwrap();
    assert_eq!(q.launch_beat, Time::new(7, 2).unwrap());

    // Flam at start == 0 produces non-negative grace hit + shifted main note.
    let flam_zero = apply_tool(
        &[ExpressiveNote::new(1, Time::ZERO, Time::EIGHTH, 38, 100)],
        &ComposeTool::Flam {
            count: 1,
            spacing: Time::SIXTY_FOURTH,
            vel_pct: 60,
        },
    )
    .unwrap();
    assert_eq!(flam_zero.len(), 2);
    assert_eq!(flam_zero[0].start, Time::ZERO);
    assert_eq!(flam_zero[1].start, Time::SIXTY_FOURTH);

    // Strum Alternate: chord 0 Up (low first), chord 1 Down (high first).
    let two_chords = [
        ExpressiveNote::new(1, Time::ZERO, Time::ONE, 60, 100),
        ExpressiveNote::new(2, Time::ZERO, Time::ONE, 67, 100),
        ExpressiveNote::new(3, Time::from_beats(2), Time::ONE, 60, 100),
        ExpressiveNote::new(4, Time::from_beats(2), Time::ONE, 67, 100),
    ];
    let alt = apply_tool(
        &two_chords,
        &ComposeTool::Strum {
            spread: Time::SIXTEENTH,
            direction: StrumDir::Alternate,
            vel_falloff_pct: 0,
        },
    )
    .unwrap();
    assert_eq!(alt[0].key, 60); // beat 0: 60 before 67
    assert_eq!(alt[1].key, 67);
    assert_eq!(alt[2].key, 67); // beat 2: 67 before 60
    assert_eq!(alt[3].key, 60);

    // Toggle step OFF removes underlying ExpressiveNote; PatternInstance offset trimming works.
    let mut tp = bank.pattern(pat).unwrap().clone();
    assert!(!tp.toggle_step(ch, 0, 60, 100).unwrap());
    assert!(tp.channel_notes(ch).is_empty());

    // Corrupt SMF checks: 5-byte VLQ, running status without prior status, > MAX_SMF_NOTES.
    let bad_vlq: &[u8] = &[
        b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 1, 0xE0,
        b'M', b'T', b'r', b'k', 0, 0, 0, 5, 0x80, 0x80, 0x80, 0x80, 0x00,
    ];
    assert_eq!(import_smf(bad_vlq), Err(SmfError::CorruptTrack));

    let bad_running: &[u8] = &[
        b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 1, 0xE0,
        b'M', b'T', b'r', b'k', 0, 0, 0, 3, 0x00, 0x40, 0x7F,
    ];
    assert_eq!(import_smf(bad_running), Err(SmfError::CorruptTrack));

    // > MAX_SMF_NOTES triggers TooManyNotes cleanly.
    let mut many_trk = Vec::new();
    for _ in 0..=MAX_SMF_NOTES {
        many_trk.extend_from_slice(&[0x00, 0x90, 0x3C, 0x64, 0x10, 0x80, 0x3C, 0x00]);
    }
    many_trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
    let mut many_smf = Vec::new();
    many_smf.extend_from_slice(b"MThd");
    many_smf.extend_from_slice(&6u32.to_be_bytes());
    many_smf.extend_from_slice(&0u16.to_be_bytes());
    many_smf.extend_from_slice(&1u16.to_be_bytes());
    many_smf.extend_from_slice(&480u16.to_be_bytes());
    many_smf.extend_from_slice(b"MTrk");
    many_smf.extend_from_slice(&(many_trk.len() as u32).to_be_bytes());
    many_smf.extend_from_slice(&many_trk);
    assert_eq!(import_smf(&many_smf), Err(SmfError::TooManyNotes));
}
