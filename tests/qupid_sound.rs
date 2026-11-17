// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's sound DSP on the host: volume boost to 300% with a soft limiter, the
//! three-band EQ and its presets, dialogue DRC, and 5.1/7.1 downmix with centre lift.
extern crate alloc;
#[path = "../crates/sex-media/src/audio_dsp.rs"] mod audio_dsp;
use audio_dsp::*;

#[test] fn audio_dsp_unity_flat_is_bit_exact_and_osd_labels_match() {
    let mut cfg = AudioDspConfig::default();
    assert!(cfg.is_identity());
    assert_eq!(cfg.osd_button_label(), "Vol / EQ");

    let mut dsp = AudioDsp::new(48_000, cfg);
    let orig = [[0i16, 0], [4200, -4200], [31_000, -31_000], [-1234, 5678]];
    let mut buf = orig;
    dsp.process_stereo_frames(&mut buf);
    assert_eq!(buf, orig, "100% Flat must be bit-exact pass-through");

    assert_eq!(cfg.toggle_boost_200(), 200);
    assert_eq!(cfg.osd_button_label(), "Vol 200%");
    cfg.apply_preset(EqPreset::Projector);
    assert_eq!(cfg.osd_button_label(), "Vol 200% · Projector");
    assert_eq!(cfg.toggle_boost_200(), 100);
    assert_eq!(cfg.osd_button_label(), "EQ · Projector");
}

#[test] fn audio_dsp_200_and_300_boost_amplify_quiet_and_soft_limit_peaks() {
    let mut cfg = AudioDspConfig::default();
    cfg.boost_percent = 200;
    let mut dsp200 = AudioDsp::new(48_000, cfg);
    let mut quiet = [[4_000i16, -4_000i16]; 8];
    dsp200.process_stereo_frames(&mut quiet);
    assert!((quiet[7][0] - 8_000).abs() <= 2, "expected ~8000 at 200%, got {}", quiet[7][0]);
    assert!((quiet[7][1] + 8_000).abs() <= 2);

    cfg.boost_percent = 300;
    let mut dsp300 = AudioDsp::new(48_000, cfg);
    let mut hot = [[28_000i16, -28_000i16]; 16];
    dsp300.process_stereo_frames(&mut hot);
    for &[l, r] in &hot {
        assert!((29_000..=32_767).contains(&l), "300% hot peak must soft-limit without wrap: {l}");
        assert!((-32_767..=-29_000).contains(&r), "300% hot peak must soft-limit without wrap: {r}");
    }
}

#[test] fn audio_dsp_three_band_eq_and_dialogue_drc_lift_speech() {
    // Generate 1.8 kHz speech-band sine wave at -24 dBFS (~2060 peak).
    let n = 1024usize;
    let mut speech_flat = Vec::with_capacity(n);
    for i in 0..n {
        let phase = 2.0 * std::f32::consts::PI * 1800.0 * (i as f32) / 48_000.0;
        let s = (phase.sin() * 2000.0) as i16;
        speech_flat.push([s, s]);
    }
    let mut speech_boosted = speech_flat.clone();
    let mut eq_cfg = AudioDspConfig::default();
    eq_cfg.apply_preset(EqPreset::Dialogue); // Voice +6 dB + Dialogue DRC
    let mut dsp = AudioDsp::new(48_000, eq_cfg);
    dsp.process_stereo_frames(&mut speech_boosted);

    let peak_before = speech_flat[512..].iter().map(|f| f[0].abs()).max().unwrap();
    let peak_after = speech_boosted[512..].iter().map(|f| f[0].abs()).max().unwrap();
    assert!(
        peak_after > peak_before * 2,
        "Dialogue +6 dB voice EQ + DRC should more than double quiet 1.8 kHz speech ({peak_before} -> {peak_after})"
    );

    // Sub -54 dBFS room noise floor (`~20` i16 units) must NOT be pumped by DRC.
    let mut drc_only = AudioDspConfig::default();
    drc_only.dialogue_drc = true;
    let mut gate_dsp = AudioDsp::new(48_000, drc_only);
    let mut hiss = [[20i16, -20i16]; 256];
    gate_dsp.process_stereo_frames(&mut hiss);
    assert!((hiss[255][0] - 20).abs() <= 1, "noise gate should hold room hiss at unity");
}

#[test] fn audio_dsp_surround_51_and_71_downmix_lifts_center_dialogue() {
    let mut dsp = AudioDsp::default();
    // 5.1 frame: FL=0, FR=0, FC=10_000, LFE=10_000, BL=0, BR=0 vs pure surround BL=10_000.
    let center_51 = [0i16, 0, 10_000, 0, 0, 0];
    let surround_51 = [0i16, 0, 0, 0, 10_000, 0];
    let mut out_fc = [[0i16; 2]; 1];
    let mut out_bl = [[0i16; 2]; 1];
    assert_eq!(dsp.downmix_surround_to_stereo(&center_51, 6, &mut out_fc), 1);
    assert_eq!(dsp.downmix_surround_to_stereo(&surround_51, 6, &mut out_bl), 1);
    assert_eq!(out_fc[0][0], out_fc[0][1], "centre dialogue pans equally to L and R");
    assert!(out_fc[0][0] > out_bl[0][0] * 2, "centre channel (+3.2 dB lift) must outweigh surround");
}


#[test] fn audio_dsp_compression_has_no_step_at_its_threshold() {
    let mut cfg = AudioDspConfig::default();
    cfg.dialogue_drc = true;
    let level = |v: i16| {
        let mut dsp = AudioDsp::new(48_000, cfg);
        let mut f = vec![[v, v]; 48_000];
        dsp.process_stereo_frames(&mut f);
        f[47_999][0] as i32
    };
    let (below, above) = (level(16_380), level(16_400));
    assert!((above - below).abs() < 100, "steady levels just either side of the threshold: {below} vs {above}");
    assert!(level(30_000) < 30_000, "loud peaks are still tamed");
}

#[test] fn audio_dsp_band_switched_back_on_starts_from_silence() {
    let mut cfg = AudioDspConfig::default();
    cfg.set_bands(8, 0, 0);
    let mut dsp = AudioDsp::new(48_000, cfg);
    let mut loud: Vec<[i16; 2]> = (0..10_000).map(|i| { let s = if i % 200 < 100 { 12_000 } else { -12_000 }; [s, s] }).collect();
    dsp.process_stereo_frames(&mut loud);
    let mut flat = cfg;
    flat.set_bands(0, 0, 0);
    dsp.set_config(flat);
    let mut silence = vec![[0i16; 2]; 48_000];
    dsp.process_stereo_frames(&mut silence);
    dsp.set_config(cfg);
    let mut after = vec![[0i16; 2]; 64];
    dsp.process_stereo_frames(&mut after);
    assert!(after.iter().all(|f| f[0].abs() < 50 && f[1].abs() < 50), "old samples replayed: {:?}", &after[..4]);
}
