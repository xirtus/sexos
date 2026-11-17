// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive integration tests for 6-band ParametricEq, Stereo-Power FFT SpectrumAnalyzer,
//! narrow-peak Resonance Detection, SidechainCompressor, SidechainGate, TapeMachine,
//! TransientShaper, BitCrusher, StereoChorus, LookaheadLimiter, MasteringProcessor, and
//! console bus/send/sidechain routing.

extern crate alloc;

use alloc::{vec, vec::Vec};
use moan_core::{
    dsp_math::{abs_f32, log10_f32, sin_f32, sqrt_f32, TAU},
    effects::Reverb,
    mastering::{LookaheadLimiter, MasteringProcessor},
    parametric_eq::{detect_resonances, ParametricEq, SpectrumAnalyzer},
    render_console_mix,
    tape_fx::{
        BitCrusher, SidechainCompressor, SidechainGate, StereoChorus, TapeMachine, TransientShaper,
    },
    Clip, Frame, MasterBus, SendRoute, SidechainSource, Track, RATE,
};

fn sine_frames(freq_hz: f32, amp: f32, len: usize) -> Vec<Frame> {
    (0..len)
        .map(|i| {
            let s = (sin_f32(TAU * freq_hz * i as f32 / RATE as f32) * amp)
                .clamp(-32768.0, 32767.0) as i16;
            [s, s]
        })
        .collect()
}

fn rms(frames: &[Frame]) -> f32 {
    if frames.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for f in frames {
        let l = f[0] as f64;
        let r = f[1] as f64;
        sum += 0.5 * (l * l + r * r);
    }
    sqrt_f32((sum / frames.len() as f64) as f32)
}

#[test]
fn parametric_eq_6_band_response_and_filtering() {
    let mut eq = ParametricEq::new();

    // Flat default EQ is bit-exact bypass and ~0 dB across 20..20000 Hz
    let src = sine_frames(1000.0, 12000.0, 1024);
    let mut out = src.clone();
    eq.process(&mut out);
    assert_eq!(out, src);

    let test_freqs = [30.0, 100.0, 150.0, 1000.0, 4000.0, 10000.0, 18000.0];
    let mut flat_db = [0.0f32; 7];
    eq.response_curve_db(&test_freqs, &mut flat_db);
    for &db in &flat_db {
        assert!(abs_f32(db) < 1e-3, "flat EQ curve expected 0 dB, got {db}");
    }

    // Configure all 6 bands
    eq.set_high_pass(true, 100.0, 0.707).unwrap();
    eq.set_low_shelf(150.0, 6.0, 0.707).unwrap();
    eq.set_bell_1(1000.0, -9.0, 2.5).unwrap();
    eq.set_bell_2(4000.0, 6.0, 2.0).unwrap();
    eq.set_high_shelf(10000.0, -6.0, 0.707).unwrap();
    eq.set_low_pass(true, 14000.0, 0.707).unwrap();

    // Verify invalid settings are refused
    assert!(eq.set_high_pass(true, 10.0, 0.707).is_err());
    assert!(eq.set_low_shelf(100.0, 20.0, 0.707).is_err());
    assert!(eq.set_bell_1(1000.0, 0.0, 20.0).is_err());
    assert!(eq.set_low_pass(true, 22000.0, 0.707).is_err());

    let mut curve_db = [0.0f32; 7];
    eq.response_curve_db(&test_freqs, &mut curve_db);

    // Measure actual steady-state filter gain at each test frequency and compare with curve
    for (&freq, &expected_db) in test_freqs.iter().zip(curve_db.iter()) {
        eq.reset();
        let tone = sine_frames(freq, 8000.0, 16000);
        let mut filtered = tone.clone();
        eq.process(&mut filtered);

        let in_rms = rms(&tone[8000..]);
        let out_rms = rms(&filtered[8000..]);
        let measured_db = 20.0 * log10_f32((out_rms / in_rms).max(1e-6));
        assert!(
            abs_f32(measured_db - expected_db) < 0.35,
            "at {freq} Hz: measured {measured_db:.2} dB vs predicted {expected_db:.2} dB"
        );
    }

    // Confirm specific band behavior: 30 Hz cut by HPF, 1 kHz notched, 4 kHz boosted, 18 kHz cut by LPF
    assert!(curve_db[0] < -12.0, "30 Hz HPF cut: {}", curve_db[0]);
    assert!(abs_f32(curve_db[3] + 9.0) < 1.0, "1 kHz Bell 1: {}", curve_db[3]);
    assert!(abs_f32(curve_db[4] - 6.0) < 1.0, "4 kHz Bell 2: {}", curve_db[4]);
    assert!(curve_db[6] < -8.0, "18 kHz LPF cut: {}", curve_db[6]);
}

#[test]
fn spectrum_analyzer_and_resonance_detection() {
    assert!(SpectrumAnalyzer::new(128).is_err());
    let mut analyzer = SpectrumAnalyzer::new(512).unwrap();
    let mut spectrum_db = vec![0.0f32; analyzer.bin_count()];

    // 1. Digital silence suppresses all resonance candidates
    let silence = vec![[0i16; 2]; 1024];
    analyzer.analyze_frames(&silence, &mut spectrum_db);
    assert_eq!(detect_resonances(&spectrum_db, RATE), [None, None, None]);

    // 2. Quiet signal below -55 dBFS is ignored
    let quiet = sine_frames(2000.0, 20.0, 1024); // ~ -64 dBFS
    analyzer.analyze_frames(&quiet, &mut spectrum_db);
    assert_eq!(detect_resonances(&spectrum_db, RATE), [None, None, None]);

    // 3. Smooth broadband spectral tilt without narrow peaks returns no candidates
    let tilted: Vec<f32> = (0..257).map(|k| -22.0 - 0.08 * k as f32).collect();
    assert_eq!(detect_resonances(&tilted, RATE), [None, None, None]);

    // 4. Out-of-phase stereo 2 kHz sine (L = -R, mono sum = 0) must still appear at full power
    let anti_phase: Vec<Frame> = (0..512)
        .map(|i| {
            let v = (sin_f32(TAU * 2000.0 * i as f32 / RATE as f32) * 16000.0) as i16;
            [v, -v]
        })
        .collect();
    analyzer.analyze_frames(&anti_phase, &mut spectrum_db);
    let bin_2k = (2000.0 * 512.0 / RATE as f32) as usize; // bin 21
    assert!(
        spectrum_db[bin_2k] > -10.0,
        "L^2 + R^2 power must preserve out-of-phase 2 kHz tone, got {} dBFS",
        spectrum_db[bin_2k]
    );

    // 5. Resonant 2 kHz sine over harmonic bed: detect_resonances finds ~2 kHz and applies cut to Bell 1
    let mut mixed = vec![[0i16; 2]; 1024];
    for (i, frame) in mixed.iter_mut().enumerate() {
        let t = i as f32 / RATE as f32;
        let bed = 1400.0 * sin_f32(TAU * 220.0 * t)
            + 1000.0 * sin_f32(TAU * 550.0 * t)
            + 900.0 * sin_f32(TAU * 4800.0 * t);
        let res = 11000.0 * sin_f32(TAU * 2000.0 * t);
        let s = (bed + res).clamp(-32768.0, 32767.0) as i16;
        *frame = [s, s];
    }
    analyzer.push_frames(&mixed);
    analyzer.compute_spectrum_db(&mut spectrum_db);
    let candidates = detect_resonances(&spectrum_db, RATE);
    let first = candidates[0].expect("must detect strong 2 kHz resonance");
    assert!(
        abs_f32(first.freq_hz - 2000.0) < 120.0,
        "expected ~2000 Hz, got {} Hz",
        first.freq_hz
    );
    assert!(
        (-6.0..0.0).contains(&first.cut_db),
        "cut_db must be negative and capped at -6.0 dB, got {}",
        first.cut_db
    );
    assert!(
        (4.0..=8.0).contains(&first.q),
        "q must be in 4.0..=8.0, got {}",
        first.q
    );

    let mut eq = ParametricEq::new();
    eq.apply_resonance_cut(2, first).unwrap();
    let mut curve = [0.0f32; 1];
    eq.response_curve_db(&[first.freq_hz], &mut curve);
    assert!(
        curve[0] <= -1.4 && curve[0] >= -6.2,
        "Bell 1 applied cut at resonance: {} dB",
        curve[0]
    );
}

#[test]
fn sidechain_compressor_ducks_pad_when_kick_hits_and_filters_sub_rumble() {
    // Track 0: Kick hitting between frame 2400..4800 (100 Hz burst)
    let mut kick_src = vec![[0i16; 2]; 12000];
    for (i, f) in kick_src[2400..4800].iter_mut().enumerate() {
        let s = (sin_f32(TAU * 100.0 * i as f32 / RATE as f32) * 24000.0) as i16;
        *f = [s, s];
    }
    // Track 1: Continuous 440 Hz pad
    let pad_src = sine_frames(440.0, 12000.0, 12000);

    let mut kick_track = Track::new();
    kick_track.clips.push(Clip::new(&kick_src, 0).unwrap());

    let mut pad_track = Track::new();
    pad_track.clips.push(Clip::new(&pad_src, 0).unwrap());
    pad_track.sc_compressor =
        Some(SidechainCompressor::new(-24.0, 8.0, 1.0, 40.0, 0.0).unwrap());
    pad_track.set_sidechain_source(Some(SidechainSource::new(0)));

    let mut tracks = [kick_track, pad_track];
    let mut mix = vec![[0i16; 2]; 12000];
    let mut pad_taps = Vec::with_capacity(12000);
    let mut pad_gr_q8 = Vec::with_capacity(12000);

    render_console_mix(&mut tracks, 0, &mut mix, 100, None, |idx, frame, gr| {
        if idx == 1 {
            pad_taps.push(frame);
            pad_gr_q8.push(gr);
        }
    });

    let before_kick = rms(&pad_taps[400..2200]);
    let during_kick = rms(&pad_taps[3000..4600]);
    let after_release = rms(&pad_taps[9600..11800]);

    assert!(
        during_kick < before_kick * 0.45,
        "pad must duck strongly during kick (before={before_kick}, during={during_kick})"
    );
    assert!(
        after_release > during_kick * 1.8,
        "pad must recover after kick release (during={during_kick}, after={after_release})"
    );
    assert_eq!(pad_gr_q8[1000], 0);
    assert!(
        pad_gr_q8[3600] > 512,
        "GR meter must exceed 2 dB (512 Q8) during kick, got {}",
        pad_gr_q8[3600]
    );

    // Verify 80 Hz sidechain HPF prevents 15 Hz sub rumble from pumping as hard as 180 Hz key
    let mut comp_sub = SidechainCompressor::new(-20.0, 6.0, 1.0, 50.0, 0.0).unwrap();
    let mut comp_mid = SidechainCompressor::new(-20.0, 6.0, 1.0, 50.0, 0.0).unwrap();
    let sub_key = sine_frames(15.0, 16000.0, 4800);
    let mid_key = sine_frames(180.0, 16000.0, 4800);
    let carrier = [12000i16, 12000i16];
    for i in 0..4800 {
        comp_sub.process_with_key(carrier, Some(sub_key[i]));
        comp_mid.process_with_key(carrier, Some(mid_key[i]));
    }
    assert!(
        comp_mid.gain_reduction_db() > comp_sub.gain_reduction_db() + 3.0,
        "80 Hz SC HPF must suppress 15 Hz sub rumble pumping (mid GR={}, sub GR={})",
        comp_mid.gain_reduction_db(),
        comp_sub.gain_reduction_db()
    );
}

#[test]
fn sidechain_gate_hysteresis_hold_and_gated_reverb_cutoff() {
    // 1. Standalone hysteresis and hold verification
    // Open @ -20 dBFS (~3277), close @ -26 dBFS (~1642), hold 20 ms (960 frames)
    let mut gate = SidechainGate::new(-20.0, 6.0, 20.0, 0.5, 5.0, -80.0).unwrap();

    // Signal at -23 dBFS (~2320) stays CLOSED initially
    let mid_level = sine_frames(400.0, 2320.0, 1200);
    let mut out_mid_closed = Vec::with_capacity(1200);
    for &f in &mid_level {
        out_mid_closed.push(gate.process_with_key(f, None));
    }
    assert!(!gate.is_open());
    assert!(rms(&out_mid_closed[600..]) < 1.0);
    assert!(gate.gain_reduction_db() >= 79.0);

    // Drive above -20 dBFS (-14 dBFS ~ 6538) -> opens
    let loud = sine_frames(400.0, 6538.0, 1200);
    for &f in &loud {
        gate.process_with_key(f, None);
    }
    assert!(gate.is_open());
    assert!(gate.gain_reduction_db() < 0.5);

    // Drop back to -23 dBFS (between open -20 and close -26): hysteresis keeps gate OPEN
    let mut out_mid_open = Vec::with_capacity(1200);
    for &f in &mid_level {
        out_mid_open.push(gate.process_with_key(f, None));
    }
    assert!(gate.is_open(), "hysteresis must keep gate open at -23 dBFS");
    assert!(rms(&out_mid_open[600..]) > 1400.0);

    // Drop to -40 dBFS (~328): stays open during 20 ms hold (960 frames), then closes to full mute
    let quiet = sine_frames(400.0, 328.0, 4800);
    let mut out_quiet = Vec::with_capacity(4800);
    for &f in &quiet {
        out_quiet.push(gate.process_with_key(f, None));
    }
    assert!(
        rms(&out_quiet[100..600]) > 150.0,
        "gate must stay open during 20 ms hold"
    );
    assert!(!gate.is_open());
    assert_eq!(out_quiet[4799], [0, 0]);

    // 2. Gated snare reverb tail cutoff in render_console_mix
    let mut snare_src = vec![[0i16; 2]; 14400];
    for (i, f) in snare_src[0..960].iter_mut().enumerate() {
        let s = (sin_f32(TAU * 350.0 * i as f32 / RATE as f32) * 20000.0) as i16;
        *f = [s, s];
    }

    let mut dry_ungated = Track::new();
    dry_ungated.clips.push(Clip::new(&snare_src, 0).unwrap());
    dry_ungated.set_send_route(Some(SendRoute::new(1, 100)));

    let mut reverb_ungated = Track::new();
    reverb_ungated.effects.reverb = Some(Reverb::new(82, 100).unwrap());

    let mut mix_ungated = vec![[0i16; 2]; 14400];
    let mut ungated_ret = Vec::with_capacity(14400);
    render_console_mix(
        &mut [dry_ungated, reverb_ungated],
        0,
        &mut mix_ungated,
        100,
        None,
        |idx, f, _| {
            if idx == 1 {
                ungated_ret.push(f);
            }
        },
    );

    let mut dry_gated = Track::new();
    dry_gated.clips.push(Clip::new(&snare_src, 0).unwrap());
    dry_gated.set_send_route(Some(SendRoute::new(1, 100)));

    let mut reverb_gated = Track::new();
    reverb_gated.effects.reverb = Some(Reverb::new(82, 100).unwrap());
    reverb_gated.gate = Some(SidechainGate::new(-22.0, 4.0, 20.0, 0.5, 10.0, -80.0).unwrap());
    reverb_gated.sidechain_from = Some(0);

    let mut mix_gated = vec![[0i16; 2]; 14400];
    let mut gated_ret = Vec::with_capacity(14400);
    render_console_mix(
        &mut [dry_gated, reverb_gated],
        0,
        &mut mix_gated,
        100,
        None,
        |idx, f, _| {
            if idx == 1 {
                gated_ret.push(f);
            }
        },
    );

    // Reverb is audible immediately after hit + during hold, then cut off after release
    assert!(rms(&ungated_ret[9600..14000]) > 200.0);
    assert!(rms(&gated_ret[1500..2200]) > 200.0);
    assert!(
        rms(&gated_ret[9600..14000]) < 1.0,
        "gated reverb tail must be cut off, got rms={}",
        rms(&gated_ret[9600..14000])
    );
}

#[test]
fn tape_machine_gated_hiss_and_wow_flutter_modulation() {
    // 1. Zero hiss on digital silence
    let mut tape = TapeMachine::new(0.6, 3.0, 9000.0, 0.5, 0.5, 1.0, 1.0).unwrap();
    assert_eq!(tape.latency_frames(), 144);
    for _ in 0..4800 {
        assert_eq!(tape.process_frame([0, 0]), [0, 0]);
    }

    // 2. Active signal triggers hiss/warmth; stopping signal cleanly gates hiss back to [0, 0]
    let tone = sine_frames(440.0, 14000.0, 4800);
    let mut wet_tone = tone.clone();
    tape.process(&mut wet_tone);
    assert!(rms(&wet_tone[500..]) > 2000.0);

    let mut tail_silence = vec![[0i16; 2]; 24000];
    tape.process(&mut tail_silence);
    assert_eq!(
        tail_silence[23999],
        [0, 0],
        "signal-gated tape hiss must stop cleanly after input stops"
    );

    // 3. Wow & flutter pitch modulation varies instantaneous cycle period of a steady 440 Hz sine
    let mut tape_still = TapeMachine::new(0.0, 0.0, 20000.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    let mut tape_wobble = TapeMachine::new(0.0, 0.0, 20000.0, 1.0, 0.8, 0.0, 1.0).unwrap();
    let pure_440 = sine_frames(440.0, 16000.0, 48000);
    let mut out_still = pure_440.clone();
    let mut out_wobble = pure_440;
    tape_still.process(&mut out_still);
    tape_wobble.process(&mut out_wobble);

    let crossing_spacings = |frames: &[Frame]| -> Vec<f32> {
        let mut crossings = Vec::new();
        for i in 1000..frames.len() {
            let a = frames[i - 1][0] as f32;
            let b = frames[i][0] as f32;
            if a <= 0.0 && b > 0.0 {
                let frac = -a / (b - a);
                crossings.push((i - 1) as f32 + frac);
            }
        }
        crossings.windows(2).map(|w| w[1] - w[0]).collect()
    };

    let still_periods = crossing_spacings(&out_still);
    let wobble_periods = crossing_spacings(&out_wobble);
    let min_max_span = |vals: &[f32]| -> f32 {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for &v in vals {
            if v < lo {
                lo = v;
            }
            if v > hi {
                hi = v;
            }
        }
        hi - lo
    };
    let still_span = min_max_span(&still_periods);
    let wobble_span = min_max_span(&wobble_periods);

    assert!(
        still_span < 0.05,
        "unmodulated 440 Hz period should be constant, got span={still_span}"
    );
    assert!(
        wobble_span > 2.0,
        "wow/flutter must modulate cycle spacing (still={still_span}, wobble={wobble_span})"
    );
}

#[test]
fn transient_shaper_bit_crusher_and_stereo_chorus() {
    // 1. TransientShaper attack boost vs cut
    let mut hit = vec![[0i16; 2]; 4800];
    for (i, f) in hit.iter_mut().enumerate() {
        let env = if i < 240 {
            1.0
        } else {
            moan_core::dsp_math::exp_f32(-((i - 240) as f32) / 1400.0)
        };
        let s = (sin_f32(TAU * 220.0 * i as f32 / RATE as f32) * 10000.0 * env) as i16;
        *f = [s, s];
    }
    let mut boosted = hit.clone();
    let mut softened = hit.clone();
    TransientShaper::new(100.0, 0.0).unwrap().process(&mut boosted);
    TransientShaper::new(-100.0, 0.0).unwrap().process(&mut softened);
    assert!(rms(&boosted[20..240]) > rms(&softened[20..240]) * 1.5);

    // 2. BitCrusher sample-and-hold and silence preservation
    let mut crusher = BitCrusher::new(8, 3, 1.0).unwrap();
    assert_eq!(crusher.process_frame([0, 0]), [0, 0]);
    crusher.reset();
    let mut crushed = sine_frames(330.0, 16000.0, 64);
    crusher.process(&mut crushed);
    for chunk in crushed.chunks_exact(8) {
        assert!(chunk.iter().all(|&f| f == chunk[0]), "downsample=8 must hold samples");
    }

    // 3. StereoChorus turns mono input into wide stereo via quadrature LFO
    let mut chorus = StereoChorus::new(1.5, 4.0, 0.2, 0.8).unwrap();
    let mut mono = sine_frames(300.0, 12000.0, 4800);
    chorus.process(&mut mono);
    let diff_energy: f32 = mono[1200..]
        .iter()
        .map(|f| abs_f32(f[0] as f32 - f[1] as f32))
        .sum();
    assert!(
        diff_energy > 10000.0,
        "quadrature chorus must create stereo width on mono input"
    );
}

#[test]
fn lookahead_limiter_enforces_ceiling_and_mastering_chain() {
    let mut limiter = LookaheadLimiter::new(-1.0, 80.0).unwrap();
    assert_eq!(limiter.latency_frames(), 192);

    // +12 dB hot signal (~3.981x full scale)
    let hot_gain = 3.9810717f32;
    let ceiling_lin = 0.89125094f32; // 10^(-1/20)
    let mut max_out = 0.0f32;
    let mut max_gr = 0.0f32;

    for i in 0..4800 {
        let s = sin_f32(TAU * 440.0 * i as f32 / RATE as f32) * hot_gain;
        let out = limiter.process_stereo_f32([s, s]);
        max_out = max_out.max(abs_f32(out[0])).max(abs_f32(out[1]));
        max_gr = max_gr.max(limiter.gain_reduction_db());
    }
    assert!(
        max_out <= ceiling_lin,
        "limiter output {max_out} exceeded -1.0 dBFS ceiling {ceiling_lin}"
    );
    assert!(max_gr > 10.0, "expected >10 dB GR on +12 dB hot signal, got {max_gr}");

    // MasteringProcessor in render_console_mix with +12 dB trim + stereo width
    let mut master = MasteringProcessor::new().unwrap();
    master.set_trim_db(12.0).unwrap();
    master.set_eq_gains(2.0, -1.0, 2.0).unwrap();
    assert_eq!(master.latency_frames(), 192);

    let hot_src = sine_frames(330.0, 28000.0, 4800);
    let mut t = Track::new();
    t.clips.push(Clip::new(&hot_src, 0).unwrap());

    let mut out = vec![[0i16; 2]; 4800];
    let mut bus = MasterBus::new(100, Some(master));
    bus.render(&mut [t], 0, &mut out, |_, _, _| {});

    let max_i16 = out
        .iter()
        .flat_map(|f| [f[0].saturating_abs(), f[1].saturating_abs()])
        .max()
        .unwrap();
    let max_dbfs = 20.0 * log10_f32(max_i16 as f32 / 32768.0);
    assert!(
        max_dbfs <= -1.0,
        "master output peak must stay <= -1.0 dBFS, got {max_dbfs:.3} dBFS (i16={max_i16})"
    );
    assert!(bus.mastering.as_ref().unwrap().gain_reduction_db() > 5.0);

    // M/S stereo_width = 0.0 collapses stereo to exact mono
    let mut mono_master = MasteringProcessor::new().unwrap();
    mono_master.set_glue_enabled(false);
    mono_master.set_stereo_width(0.0).unwrap();
    for i in 0..400 {
        let out_m = mono_master.process_frame([8000, -4000 + (i as i16 % 100)]);
        assert_eq!(out_m[0], out_m[1], "width=0.0 must yield exact mono");
    }
}

#[test]
fn console_bus_and_post_fader_send_routing_across_arbitrary_track_order() {
    // Deliberately put Return (Track 0) and Drum Bus (Track 1) BEFORE Source tracks (Track 2, 3)
    // to prove source -> bus -> return dependency ordering works in any order.
    let kick_src = sine_frames(90.0, 12000.0, 2048);
    let snare_src = sine_frames(450.0, 8000.0, 2048);

    // Track 0: Parallel Reverb Return (receives 50% post-fader send from Track 3)
    let mut return_track = Track::new();
    return_track.effects.reverb = Some(Reverb::new(60, 100).unwrap());

    // Track 1: Drum Bus (receives serial output_bus from Track 2 and Track 3)
    let mut drum_bus = Track::new();
    drum_bus.set_volume(50).unwrap(); // halves combined drum bus

    // Track 2: Kick -> Drum Bus (Track 1)
    let mut kick = Track::new();
    kick.clips.push(Clip::new(&kick_src, 0).unwrap());
    kick.output_bus = Some(1);

    // Track 3: Snare -> Drum Bus (Track 1) AND 50% post-fader send -> Return (Track 0)
    let mut snare = Track::new();
    snare.clips.push(Clip::new(&snare_src, 0).unwrap());
    snare.output_bus = Some(1);
    snare.post_fader_send = Some((0, 50));

    let mut tracks_full = [
        return_track.clone(),
        drum_bus.clone(),
        kick.clone(),
        snare.clone(),
    ];
    let mut full_out = vec![[0i16; 2]; 2048];
    let mut bus_taps = Vec::with_capacity(2048);
    let mut ret_taps = Vec::with_capacity(2048);

    let mut full_master = MasteringProcessor::new().unwrap();
    full_master.set_trim_db(3.0).unwrap();
    render_console_mix(
        &mut tracks_full,
        0,
        &mut full_out,
        100,
        Some(&mut full_master),
        |idx, frame, _| {
            if idx == 1 {
                bus_taps.push(frame);
            } else if idx == 0 {
                ret_taps.push(frame);
            }
        },
    );

    // Verify Drum Bus receives both Kick and Snare (at 50% bus volume)
    assert!(rms(&bus_taps) > 2000.0);
    // Verify Reverb Return receives Snare send
    assert!(rms(&ret_taps) > 1000.0);

    // Verify chunked render (including MasteringProcessor state) matches single full render bit-for-bit
    let mut tracks_chunked = [return_track, drum_bus, kick, snare];
    let mut chunk_master = MasteringProcessor::new().unwrap();
    chunk_master.set_trim_db(3.0).unwrap();
    let mut chunk_a = vec![[0i16; 2]; 1024];
    let mut chunk_b = vec![[0i16; 2]; 1024];
    render_console_mix(
        &mut tracks_chunked,
        0,
        &mut chunk_a,
        100,
        Some(&mut chunk_master),
        |_, _, _| {},
    );
    render_console_mix(
        &mut tracks_chunked,
        1024,
        &mut chunk_b,
        100,
        Some(&mut chunk_master),
        |_, _, _| {},
    );

    assert_eq!([chunk_a, chunk_b].concat(), full_out);
}

#[test]
fn spectrum_256_head_bump_gentle_gate_and_track_process() {
    // 1. 256-point FFT analyzer resolves 1.5 kHz resonance
    let mut spec256 = SpectrumAnalyzer::new(256).unwrap();
    let mut bins = vec![0.0f32; spec256.bin_count()];
    assert_eq!(bins.len(), 129);
    let tone_1500 = sine_frames(1500.0, 14000.0, 512);
    spec256.analyze_frames(&tone_1500, &mut bins);
    let cands = detect_resonances(&bins, RATE);
    let c0 = cands[0].expect("256-pt FFT must detect 1.5 kHz peak");
    assert!(abs_f32(c0.freq_hz - 1500.0) < 200.0);

    // 2. TapeMachine 85 Hz head bump boosts 85 Hz and tone_hz cuts 12 kHz
    let low_85 = sine_frames(85.0, 8000.0, 9600);
    let mut flat_tape = TapeMachine::new(0.0, 0.0, 20000.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    let mut bump_tape = TapeMachine::new(0.0, 5.0, 20000.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    let mut out_flat = low_85.clone();
    let mut out_bump = low_85;
    flat_tape.process(&mut out_flat);
    bump_tape.process(&mut out_bump);
    assert!(rms(&out_bump[4800..]) > rms(&out_flat[4800..]) * 1.5);

    let hi_12k = sine_frames(12000.0, 8000.0, 4800);
    let mut dark_tape = TapeMachine::new(0.0, 0.0, 2000.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    let mut out_dark = hi_12k.clone();
    dark_tape.process(&mut out_dark);
    assert!(rms(&out_dark[1200..]) < rms(&hi_12k[1200..]) * 0.35);

    // 3. SidechainGate gentle -6 dB duck range (does not mute completely)
    let mut duck_gate = SidechainGate::new(-18.0, 2.0, 0.0, 0.5, 5.0, -6.0).unwrap();
    let quiet = sine_frames(400.0, 2000.0, 2400); // ~ -24 dBFS (below -18)
    let mut ducked = quiet.clone();
    duck_gate.process(&mut ducked);
    let ratio = rms(&ducked[1200..]) / rms(&quiet[1200..]);
    assert!((0.45..=0.56).contains(&ratio), "expected -6 dB (~0.5x) duck, got {ratio}");

    // 4. Track::process integrates new pro FX fields cleanly
    let mut t = Track::new();
    let mut eq = ParametricEq::new();
    eq.set_bell_1(1000.0, -12.0, 2.0).unwrap();
    t.parametric_eq = Some(eq);
    let mut buf = sine_frames(1000.0, 12000.0, 4800);
    let raw_rms = rms(&buf[2400..]);
    t.process(&mut buf);
    assert!(rms(&buf[2400..]) < raw_rms * 0.35);
    t.reset_effects();
}
