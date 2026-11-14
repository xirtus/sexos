// SPDX-License-Identifier: GPL-3.0-only
use moan_core::{
    effects::{Compressor, Equalizer, Rack, Reverb},
    render, render_processed, Clip, Track,
};
fn track(source: &[[i16; 2]]) -> Track<'_> {
    let mut t = Track::new();
    t.clips.push(Clip::new(source, 0).unwrap());
    t
}
fn rack() -> Rack {
    Rack {
        eq: Some(Equalizer::new(80, 120, 70).unwrap()),
        compressor: Some(Compressor::new(6000, 4, 10, 480).unwrap()),
        reverb: Some(Reverb::new(65, 40).unwrap()),
    }
}
fn energy(frames: &[[i16; 2]]) -> i64 {
    frames.iter().map(|f| (f[0] as i64).pow(2)).sum()
}
#[test]
fn bypass_and_flat_eq_are_bit_exact() {
    let source: Vec<_> = (0..10000)
        .map(|i| [(i * 113) as i16, (-i * 271) as i16])
        .collect();
    for mut effects in [
        Rack::default(),
        Rack {
            eq: Some(Equalizer::new(100, 100, 100).unwrap()),
            ..Rack::default()
        },
    ] {
        let mut out = source.clone();
        effects.process(&mut out);
        assert_eq!(out, source);
    }
}
#[test]
fn eq_separates_low_and_high_frequencies() {
    for period in [480, 8] {
        let source: Vec<_> = (0..48000)
            .map(|i| [if i % period < period / 2 { 1000 } else { -1000 }; 2])
            .collect();
        let mut low = source.clone();
        let mut high = source.clone();
        Rack {
            eq: Some(Equalizer::new(100, 0, 0).unwrap()),
            ..Rack::default()
        }
        .process(&mut low);
        Rack {
            eq: Some(Equalizer::new(0, 0, 100).unwrap()),
            ..Rack::default()
        }
        .process(&mut high);
        let a = energy(&low[1000..]);
        let b = energy(&high[1000..]);
        if period == 480 {
            assert!(a > b * 3, "low energy={a}, high={b}");
        } else {
            assert!(b > a * 3, "low energy={a}, high={b}");
        }
    }
}
#[test]
fn compressor_curve_stereo_link_and_unity_ratio() {
    let source = [[12000, 6000]; 100];
    let mut out = source;
    Rack {
        compressor: Some(Compressor::new(4000, 4, 1, 100).unwrap()),
        ..Rack::default()
    }
    .process(&mut out);
    // Static curve: 4000 + (12000 - 4000)/4 = 6000. Q16 rounding <=1.
    for f in out {
        assert!((f[0] - 6000).abs() <= 1);
        assert!((f[1] - 3000).abs() <= 1);
    }
    let mut out = source;
    Rack {
        compressor: Some(Compressor::new(4000, 1, 10, 100).unwrap()),
        ..Rack::default()
    }
    .process(&mut out);
    assert_eq!(out, source);
}
#[test]
fn compressor_attack_and_release_are_stateful() {
    let mut slow = Rack {
        compressor: Some(Compressor::new(4000, 4, 100, 100).unwrap()),
        ..Rack::default()
    };
    let mut transient = [[16000; 2]; 500];
    slow.process(&mut transient);
    assert_eq!(transient[0], [16000; 2]);
    assert!(transient[499][0] < 8000);
    // A quiet signal immediately after the loud passage remains attenuated,
    // then recovers as the detector releases.
    let mut quiet = [[1000; 2]; 2000];
    slow.process(&mut quiet);
    assert!(quiet[0][0] < 1000);
    assert_eq!(quiet[1999], [1000; 2]);
}
#[test]
fn reverb_has_a_decaying_tail_and_no_pre_echo() {
    let mut source = vec![[0; 2]; 144000];
    source[0] = [20000; 2];
    Rack {
        reverb: Some(Reverb::new(65, 100).unwrap()),
        ..Rack::default()
    }
    .process(&mut source);
    assert_eq!(source[0], [20000; 2]);
    assert!(source[1..1429].iter().all(|f| *f == [0; 2]));
    assert!(energy(&source[1429..48000]) > 0);
    assert!(energy(&source[96000..]) < energy(&source[1429..48000]) / 10);
    assert!(source.iter().all(|f| f[0].abs() < 32767));
}
#[test]
fn zero_wet_reverb_is_exact_bypass() {
    let mut r = Rack {
        reverb: Some(Reverb::new(90, 0).unwrap()),
        ..Rack::default()
    };
    let source: Vec<_> = (0..10000)
        .map(|i| [(i * 193) as i16, (i * 157) as i16])
        .collect();
    let mut out = source.clone();
    r.process(&mut out);
    assert_eq!(out, source);
}
#[test]
fn rack_chunk_boundaries_are_invisible_and_reset_clears_tail() {
    let source: Vec<_> = (0..16000)
        .map(|i| [if i % 120 < 60 { 10000 } else { -10000 }; 2])
        .collect();
    let mut full = source.clone();
    let mut a = rack();
    a.process(&mut full);
    let mut chunks = source.clone();
    let mut b = rack();
    for block in chunks.chunks_mut(113) {
        b.process(block);
    }
    assert_eq!(full, chunks);
    b.reset();
    let mut silence = vec![[0; 2]; 10000];
    b.process(&mut silence);
    assert!(silence.iter().all(|f| *f == [0; 2]));
}
#[test]
fn processed_bypass_equals_dry_mix_including_overload_cancellation() {
    let pos = [[30000; 2]; 10];
    let neg = [[-30000; 2]; 10];
    let mut tracks = [track(&pos), track(&pos), track(&neg)];
    let mut dry = [[0; 2]; 10];
    render(&tracks, 0, &mut dry);
    let mut processed = [[0; 2]; 10];
    render_processed(&mut tracks, 0, &mut processed);
    assert_eq!(processed, dry);
}
#[test]
fn mixer_seek_resets_effects_and_empty_calls_preserve_state() {
    let source = vec![[8000; 2]; 10000];
    let mut t = track(&source);
    t.effects = rack();
    let mut tracks = [t];
    let mut first = vec![[0; 2]; 5000];
    render_processed(&mut tracks, 0, &mut first);
    render_processed(&mut tracks, 999, &mut []);
    let mut next = vec![[0; 2]; 5000];
    render_processed(&mut tracks, 5000, &mut next);
    let mut fresh = track(&source);
    fresh.effects = rack();
    let mut full = vec![[0; 2]; 10000];
    render_processed(&mut [fresh], 0, &mut full);
    assert_eq!([first.clone(), next].concat(), full);
    let mut seek = vec![[0; 2]; 5000];
    render_processed(&mut tracks, 0, &mut seek);
    assert_eq!(seek, first);
    tracks[0].reset_effects();
    let mut silence = vec![[0; 2]; 5000];
    render_processed(&mut tracks, 10000, &mut silence);
    assert!(silence.iter().all(|f| *f == [0; 2]));
}
#[test]
fn muted_effects_advance_and_tail_is_audible_after_clip_end() {
    let source = [[16000; 2]];
    let mut t = track(&source);
    t.effects.reverb = Some(Reverb::new(65, 100).unwrap());
    t.muted = true;
    let mut tracks = [t];
    let mut muted = vec![[1; 2]; 1000];
    render_processed(&mut tracks, 0, &mut muted);
    assert!(muted.iter().all(|f| *f == [0; 2]));
    tracks[0].muted = false;
    let mut tail = vec![[0; 2]; 5000];
    render_processed(&mut tracks, 1000, &mut tail);
    assert!(energy(&tail) > 0);
}
#[test]
fn invalid_configuration_is_refused() {
    assert!(Equalizer::new(401, 100, 100).is_err());
    for (threshold, ratio, attack, release) in [
        (0, 4, 1, 1),
        (32768, 4, 1, 1),
        (1000, 0, 1, 1),
        (1000, 21, 1, 1),
        (1000, 4, 0, 1),
        (1000, 4, 1, 0),
        (1000, 4, 480001, 1),
    ] {
        assert!(Compressor::new(threshold, ratio, attack, release).is_err());
    }
    assert!(Reverb::new(91, 100).is_err());
    assert!(Reverb::new(65, 101).is_err());
}
