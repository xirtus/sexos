// SPDX-License-Identifier: GPL-3.0-only
//! Bell's rate conversion: pitch and length survive, joins between periods are seamless.
#[path="../servers/sexbell/src/resample.rs"]mod resample;
use resample::*;

fn tone(rate: u32, hz: f64, frames: usize) -> Vec<[i16; 2]> {
    (0..frames).map(|i| { let s = ((i as f64) * hz * 2.0 * std::f64::consts::PI / rate as f64).sin() * 12000.0; [s as i16, s as i16] }).collect()
}

/// Runs a whole input through in periods of `period` output frames, as Bell does.
fn convert(input: &[[i16; 2]], rate: u32, period: usize) -> Vec<i32> {
    let st = step(rate);
    let (mut taken, mut frac, mut out) = (0usize, 0u32, Vec::new());
    loop {
        let have = input.len() - taken;
        let n = outputs(have, frac, st, period);
        if n == 0 { break; }
        let mut mix = vec![[0i32; 2]; period];
        let (used, f) = mix_into(&mut mix, n, frac, st, 65536, |i| input[taken + i]);
        taken += used; frac = f;
        out.extend(mix[..n].iter().map(|m| m[0]));
    }
    out
}

fn crossings(x: &[i32]) -> usize { x.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count() }

#[test] fn rates_out_of_range_are_48k() {
    assert_eq!(rate_of(0), 48_000); assert_eq!(rate_of(7_999), 48_000); assert_eq!(rate_of(400_000), 48_000);
    assert_eq!(rate_of(44_100), 44_100); assert_eq!(rate_of(8_000), 8_000);
}

#[test] fn rate_44100_keeps_pitch_and_length() {
    let input = tone(44_100, 440.0, 44_100);
    let out = convert(&input, 44_100, 1024);
    // One second in is one second out, and 440 cycles stay 440.
    assert!((out.len() as i64 - 48_000).abs() <= 2, "{} frames", out.len());
    let c = crossings(&out) as i64;
    assert!((c - 440).abs() <= 1, "{c} cycles");
}

#[test] fn rate_22050_and_96000() {
    for rate in [22_050u32, 96_000, 8_000, 192_000] {
        let input = tone(rate, 300.0, rate as usize);
        let out = convert(&input, rate, 480);
        // The last input frame has no next one to blend with: up to two input frames short.
        let slack = 2 * (48_000 / rate) as i64 + 1;
        assert!((out.len() as i64 - 48_000).abs() <= slack, "{rate}: {} frames", out.len());
        assert!((crossings(&out) as i64 - 300).abs() <= 1, "{rate}");
    }
}

#[test] fn joins_between_periods_are_smooth() {
    // The same input in one go and in small periods gives the same samples.
    let input = tone(44_100, 1000.0, 9_000);
    let whole = convert(&input, 44_100, 1 << 20);
    let pieces = convert(&input, 44_100, 37);
    assert_eq!(whole, pieces);
}

#[test] fn rate_48000_is_a_copy() {
    let input = tone(48_000, 440.0, 4_800);
    let out = convert(&input, 48_000, 512);
    assert_eq!(out.len(), 4_799);
    assert!(out.iter().zip(&input).all(|(o, i)| *o == i[0] as i32));
}

#[test] fn outputs_need_two_frames() {
    assert_eq!(outputs(0, 0, step(44_100), 10), 0);
    assert_eq!(outputs(1, 0, step(44_100), 10), 0);
    assert_eq!(outputs(2, 0, step(44_100), 10), 1);
}
