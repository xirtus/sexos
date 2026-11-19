use moan_core::{
    midi::{Error, Instrument, Note, Pattern, MAX_FRAMES, MAX_NOTES, MAX_VOICES},
    Clip, Track, RATE,
};
fn note() -> Pattern {
    Pattern {
        frames: RATE,
        bpm: 120,
        instrument: Instrument::SilkTriangle,
        notes: vec![Note {
            start: 0,
            frames: RATE,
            key: 69,
            velocity: 100,
        }],
    }
}
#[test]
fn concert_a_pitch_and_silence_have_real_pcm() {
    let pcm = note().bake().unwrap();
    let crossings = pcm
        .windows(2)
        .filter(|p| p[0][0] <= 0 && p[1][0] > 0)
        .count();
    assert!((439..=441).contains(&crossings), "{crossings}");
    assert!(pcm.iter().any(|f| f[0].abs() > 5000));
    assert!(pcm.iter().all(|f| f[0] == f[1]));
    let mut silent = note();
    silent.notes.clear();
    assert!(silent.bake().unwrap().iter().all(|f| *f == [0; 2]));
}
#[test]
fn distinct_presets_and_sample_pitch_playback() {
    let triangle = Pattern::preset(Instrument::SilkTriangle).bake().unwrap();
    let pulse = Pattern::preset(Instrument::SatinPulse).bake().unwrap();
    assert_ne!(triangle, pulse);
    let mut sampler = note();
    sampler.instrument = Instrument::VelvetSampler;
    sampler.notes[0].key = 60;
    let base = sampler.bake().unwrap();
    assert!(base[100..9000].iter().any(|f| f[0] != 0));
    assert!(base[9600..].iter().all(|f| *f == [0; 2]));
    sampler.notes[0].key = 72;
    let octave = sampler.bake().unwrap();
    assert!(octave[5000..].iter().all(|f| *f == [0; 2]));
    assert_ne!(base, octave);
}
#[test]
fn malformed_notes_and_resource_budgets_reject_before_bake() {
    let mut p = note();
    p.frames = MAX_FRAMES + 1;
    assert_eq!(p.bake(), Err(Error::Range));
    p = note();
    p.notes[0].velocity = 0;
    assert_eq!(p.validate(), Err(Error::Note));
    p = note();
    p.notes[0].key = 128;
    assert_eq!(p.validate(), Err(Error::Note));
    p = note();
    p.notes[0].frames = 0;
    assert_eq!(p.validate(), Err(Error::Note));
    p = note();
    p.notes.push(Note {
        start: u32::MAX,
        frames: u32::MAX,
        key: 60,
        velocity: 1,
    });
    assert_eq!(p.validate(), Err(Error::Note));
    p = note();
    p.notes = vec![p.notes[0]; MAX_NOTES + 1];
    assert_eq!(p.validate(), Err(Error::Range));
    p = note();
    p.notes = vec![p.notes[0]; MAX_VOICES + 1];
    assert_eq!(p.validate(), Err(Error::Polyphony));
    p = note();
    p.bpm = 0;
    assert_eq!(p.validate(), Err(Error::Range));
}
#[test]
fn midi_cache_preserves_split_trim_mix_and_edit_invalidation() {
    let mut left = Clip::midi(note(), 0).unwrap();
    let mut right = left.split(RATE as usize / 2).unwrap();
    assert_eq!(left.midi_pattern(), right.midi_pattern());
    assert_eq!(
        left.source_frames().as_ptr(),
        right.source_frames().as_ptr()
    );
    let mut t = Track::new();
    t.clips = vec![left.clone(), right.clone()];
    let mut all = vec![[0; 2]; RATE as usize];
    moan_core::render(&[t], 0, &mut all);
    assert_eq!(all.as_slice(), note().bake().unwrap().as_ref());
    let mut changed = note();
    changed.notes[0].key = 72;
    let rebaked = Clip::midi(changed.clone(), 0).unwrap();
    left.replace_midi_source(&rebaked).unwrap();
    right.replace_midi_source(&rebaked).unwrap();
    assert_eq!(right.source_offset(), RATE as usize / 2);
    assert_eq!(right.start(), RATE as usize / 2);
    assert_eq!(right.midi_pattern(), Some(&changed));
    let mut t = Track::new();
    t.clips = vec![left, right];
    moan_core::render(&[t], 0, &mut all);
    assert_eq!(all.as_slice(), rebaked.source_frames());
}
#[test]
fn observed_meter_samples_are_post_pan_fader_and_match_the_only_dsp_pass() {
    let source = [[4000, -4000]; 8];
    let mut t = Track::new();
    t.clips.push(Clip::new(&source, 0).unwrap());
    t.set_volume(50).unwrap();
    t.set_pan(100).unwrap();
    let mut out = [[0; 2]; 8];
    let mut taps = Vec::new();
    moan_core::render_processed_observed(
        core::slice::from_mut(&mut t),
        0,
        &mut out,
        |id, frame| taps.push((id, frame)),
    );
    assert_eq!(out, [[0, -2000]; 8]);
    assert_eq!(taps, vec![(0, [0, -2000]); 8]);
    t.muted = true;
    taps.clear();
    moan_core::render_processed_observed(
        core::slice::from_mut(&mut t),
        8,
        &mut out,
        |id, frame| taps.push((id, frame)),
    );
    assert_eq!(out, [[0; 2]; 8]);
    assert_eq!(taps, vec![(0, [0; 2]); 8]);
}
#[test]
fn musical_steps_use_one_rational_clock_without_accumulated_rounding() {
    use moan_core::midi::step_frame;
    for bpm in [30, 120, 137, 240] {
        for step in 0..=16 {
            assert_eq!(
                step_frame(bpm, step).unwrap() as u64,
                48_000u64 * 30 * step as u64 / bpm as u64
            );
        }
    }
    assert!(step_frame(137, 16).unwrap() > step_frame(137, 1).unwrap() * 16);
    assert!(step_frame(0, 0).is_err());
    assert!(step_frame(120, 17).is_err());
}
