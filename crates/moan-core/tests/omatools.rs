// SPDX-License-Identifier: GPL-3.0-only
//! Integration tests for OmaAudio capabilities in moan-core:
//! - Quick Voice Fixes (`HumRemover`, `VoiceDenoise`, `DeEsser`, `SpeechLeveler`, `VoiceDucker`, `VoiceCleanupChain`)
//! - ITU-R BS.1770-4 / EBU R128 loudness & True-Peak normalization
//! - `ElasticStretch` pitch-preserving time stretch & pitch shift
//! - `InterchangeTimeline` OTIO / CMX 3600 EDL / AAF-XML round-trip

extern crate alloc;
use alloc::{string::String, vec, vec::Vec};
use moan_core::{
    dsp_math::{abs_f32, sin_f32, sqrt_f32, TAU},
    interchange::{
        InterchangeClip, InterchangeLoudness, InterchangeMarker, InterchangeTimeline,
        InterchangeTrack, InterchangeTrackKind,
    },
    loudness::{
        measure_f32, measure_frames, normalize_to_target, DeliveryTarget, LoudnessMeter,
        TruePeakDetector,
    },
    stretch::{ElasticStretch, StretchError},
    voice_cleanup::{
        DeEsser, HumFrequency, HumRemover, SpeechLeveler, VoiceCleanupChain, VoiceDenoise,
        VoiceDucker, VoicePreset,
    },
    Clip, Frame, Track, RATE,
};

fn sine_frames(freq_hz: f32, amp: f32, len: usize) -> Vec<Frame> {
    (0..len)
        .map(|i| {
            let v = (amp * sin_f32(TAU * freq_hz * i as f32 / RATE as f32) * 32767.0) as i16;
            [v, v]
        })
        .collect()
}

fn sine_f32(freq_hz: f32, amp: f32, len: usize) -> Vec<[f32; 2]> {
    (0..len)
        .map(|i| {
            let v = amp * sin_f32(TAU * freq_hz * i as f32 / RATE as f32);
            [v, v]
        })
        .collect()
}

fn rms_f32(buf: &[[f32; 2]]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    let sum: f64 = buf
        .iter()
        .map(|f| 0.5 * ((f[0] as f64).powi(2) + (f[1] as f64).powi(2)))
        .sum();
    sqrt_f32((sum / buf.len() as f64) as f32)
}

fn rms_frames(buf: &[Frame]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    let sum: f64 = buf
        .iter()
        .map(|f| {
            let l = f[0] as f64 / 32768.0;
            let r = f[1] as f64 / 32768.0;
            0.5 * (l * l + r * r)
        })
        .sum();
    sqrt_f32((sum / buf.len() as f64) as f32)
}

fn estimate_freq_hz(buf: &[Frame]) -> f32 {
    // Zero-crossing pitch estimate over steady center 60% of signal
    let start = buf.len() / 5;
    let end = (buf.len() * 4) / 5;
    let slice = &buf[start..end];
    let mut crossings = 0usize;
    let mut first = None;
    let mut last = 0usize;
    for i in 1..slice.len() {
        if slice[i - 1][0] < 0 && slice[i][0] >= 0 {
            if first.is_none() {
                first = Some(i);
            }
            last = i;
            crossings += 1;
        }
    }
    let Some(f) = first else {
        return 0.0;
    };
    if crossings <= 1 || last <= f {
        return 0.0;
    }
    let cycles = (crossings - 1) as f32;
    cycles * RATE as f32 / (last - f) as f32
}

#[test]
fn hum_remover_notches_50hz_and_60hz_harmonics_without_dulling_speech() {
    // 60 Hz fundamental + 2nd (120) + 3rd (180) + 6th (360) harmonic hum
    let len = RATE as usize; // 1.0 s
    let mut hum_60: Vec<[f32; 2]> = (0..len)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let v = 0.25 * sin_f32(TAU * 60.0 * t)
                + 0.15 * sin_f32(TAU * 120.0 * t)
                + 0.10 * sin_f32(TAU * 180.0 * t)
                + 0.08 * sin_f32(TAU * 360.0 * t);
            [v, v]
        })
        .collect();
    let before_hum = rms_f32(&hum_60);
    let mut remover_60 = HumRemover::new(HumFrequency::Hz60);
    remover_60.process_f32(&mut hum_60);
    let after_hum = rms_f32(&hum_60[len / 2..]);
    assert!(
        after_hum < before_hum * 0.08,
        "60 Hz hum + harmonics must be notched deeply: before={before_hum}, after={after_hum}"
    );

    // Speech tone at 440 Hz passes virtually untouched
    let mut speech = sine_f32(440.0, 0.3, len);
    let before_sp = rms_f32(&speech);
    remover_60.reset();
    remover_60.process_f32(&mut speech);
    let after_sp = rms_f32(&speech[len / 4..]);
    assert!(
        abs_f32(after_sp - before_sp) / before_sp < 0.03,
        "440 Hz speech must be preserved: before={before_sp}, after={after_sp}"
    );

    // 50 Hz fundamental + 6th harmonic (300 Hz)
    let mut hum_50: Vec<Frame> = (0..len)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let v = ((0.25 * sin_f32(TAU * 50.0 * t) + 0.15 * sin_f32(TAU * 300.0 * t)) * 32767.0)
                as i16;
            [v, v]
        })
        .collect();
    let before_50 = rms_frames(&hum_50);
    let mut remover_50 = HumRemover::new(HumFrequency::Hz50);
    remover_50.process_frames(&mut hum_50);
    let after_50 = rms_frames(&hum_50[len / 2..]);
    assert!(
        after_50 < before_50 * 0.08,
        "50 Hz hum + harmonics must be notched: before={before_50}, after={after_50}"
    );
}

#[test]
fn voice_denoise_suppresses_stationary_noise_and_preserves_voice() {
    // Pseudo-random room hiss noise (~ -32 dBFS)
    let mut seed: u32 = 0x1234_5678;
    let mut next_noise = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let u = ((seed >> 8) as f32) / (16777216.0) * 2.0 - 1.0;
        u * 0.025
    };

    let noise_len = (RATE / 2) as usize; // 0.5 s noise profile
    let noise_only: Vec<[f32; 2]> = (0..noise_len)
        .map(|_| {
            let n = next_noise();
            [n, n]
        })
        .collect();

    let mut denoise = VoiceDenoise::with_config(20.0, 1.5, RATE as f32);
    denoise.learn_noise_f32(&noise_only);

    let mut test_noise = noise_only.clone();
    let before_noise = rms_f32(&test_noise);
    denoise.process_f32(&mut test_noise);
    let after_noise = rms_f32(&test_noise[noise_len / 2..]);
    assert!(
        after_noise < before_noise * 0.25,
        "stationary noise must be suppressed: before={before_noise}, after={after_noise}"
    );

    // Speech tone (500 Hz + 1600 Hz @ 0.25 amp) + same noise
    let mut speech: Vec<[f32; 2]> = (0..noise_len)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let s = 0.25 * sin_f32(TAU * 500.0 * t) + 0.15 * sin_f32(TAU * 1600.0 * t);
            let n = next_noise();
            [s + n, s + n]
        })
        .collect();
    let before_speech = rms_f32(&speech);
    denoise.process_f32(&mut speech);
    let after_speech = rms_f32(&speech[noise_len / 4..]);
    assert!(
        after_speech > before_speech * 0.78,
        "clean voice must pass with minimal loss: before={before_speech}, after={after_speech}"
    );
}

#[test]
fn de_esser_targets_4500_to_10000hz_sibilance_without_attenuating_vowels() {
    let len = (RATE / 2) as usize;
    let mut de_esser = DeEsser::new();

    // Harsh 6.5 kHz sibilance at 0.35 amp (-9 dBFS, well above -24 dBFS threshold)
    let mut sibilance = sine_f32(6500.0, 0.35, len);
    let before_sib = rms_f32(&sibilance);
    de_esser.process_f32(&mut sibilance);
    let after_sib = rms_f32(&sibilance[len / 4..]);
    assert!(
        after_sib < before_sib * 0.45,
        "6.5 kHz sibilance must be strongly reduced: before={before_sib}, after={after_sib}"
    );
    assert!(de_esser.last_gain_reduction_db() > 5.0);

    // 450 Hz vowel fundamental at same high amplitude must remain untouched
    de_esser.reset();
    let mut vowel = sine_f32(450.0, 0.35, len);
    let before_v = rms_f32(&vowel);
    de_esser.process_f32(&mut vowel);
    let after_v = rms_f32(&vowel[len / 4..]);
    assert!(
        abs_f32(after_v - before_v) / before_v < 0.04,
        "450 Hz vowel must remain intact: before={before_v}, after={after_v}"
    );
}

#[test]
fn speech_leveler_evens_dynamics_and_ignores_silence() {
    let mut leveler = SpeechLeveler::new();
    let seg = (RATE / 2) as usize;

    // Quiet phrase (~0.03 RMS -> ~-30 dBFS) vs loud phrase (~0.50 RMS -> ~-6 dBFS)
    let mut quiet = sine_f32(300.0, 0.045, seg);
    leveler.process_f32(&mut quiet);
    let quiet_out = rms_f32(&quiet[seg / 2..]);

    leveler.reset();
    let mut loud = sine_f32(300.0, 0.65, seg);
    leveler.process_f32(&mut loud);
    let loud_out = rms_f32(&loud[seg / 2..]);

    // Original ratio was 0.65/0.045 = 14.4x (~23 dB); leveled ratio should be much smaller (< 3x)
    assert!(
        loud_out / quiet_out.max(1e-6) < 3.0,
        "leveler must even out quiet ({quiet_out}) and loud ({loud_out})"
    );

    // Silence (-72 dBFS) must not be boosted up to target
    leveler.reset();
    let mut silence = sine_f32(300.0, 0.00025, seg);
    let before_sil = rms_f32(&silence);
    leveler.process_f32(&mut silence);
    let after_sil = rms_f32(&silence[seg / 2..]);
    assert!(
        after_sil <= before_sil * 1.15,
        "noise gate must prevent boosting silence: before={before_sil}, after={after_sil}"
    );
}

#[test]
fn voice_ducker_ducks_bed_under_speech_with_hold_and_release() {
    let mut ducker = VoiceDucker::with_config(-30.0, -12.0, 10.0, 100.0, 150.0, RATE as f32);
    let bed = sine_f32(220.0, 0.25, RATE as usize * 2);

    // Speech active during 0.2s..0.7s, silent before and after
    let mut speech = vec![[0.0f32; 2]; RATE as usize * 2];
    for i in (RATE as usize / 5)..(RATE as usize * 7 / 10) {
        let s = 0.4 * sin_f32(TAU * 350.0 * i as f32 / RATE as f32);
        speech[i] = [s, s];
    }

    let mut ducked_bed = bed.clone();
    ducker.process_bed_f32(&speech, &mut ducked_bed);

    let pre_rms = rms_f32(&ducked_bed[..RATE as usize / 10]);
    let during_rms = rms_f32(&ducked_bed[(RATE as usize * 4 / 10)..(RATE as usize * 6 / 10)]);
    let post_rms = rms_f32(&ducked_bed[(RATE as usize * 15 / 10)..]);

    assert!(
        during_rms < pre_rms * 0.35,
        "bed must duck during speech: pre={pre_rms}, during={during_rms}"
    );
    assert!(
        post_rms > pre_rms * 0.85,
        "bed must recover after speech + hold + release: pre={pre_rms}, post={post_rms}"
    );
}

#[test]
fn voice_cleanup_chain_presets_and_bed_mix() {
    let mut frames = sine_frames(440.0, 0.2, RATE as usize / 2);
    let bed = sine_frames(180.0, 0.3, RATE as usize / 2);
    // Inject 60 Hz hum
    for (i, f) in frames.iter_mut().enumerate() {
        let hum = (0.12 * sin_f32(TAU * 60.0 * i as f32 / RATE as f32) * 32767.0) as i16;
        f[0] = f[0].saturating_add(hum);
        f[1] = f[1].saturating_add(hum);
    }

    for preset in [
        VoicePreset::PodcastClean,
        VoicePreset::BroadcastVoice,
        VoicePreset::NoisyRoom,
    ] {
        let mut chain = VoiceCleanupChain::from_preset(preset);
        let mut copy = frames.clone();
        chain.process_with_bed_frames(&mut copy, &bed);
        assert!(rms_frames(&copy) > 0.05);
    }
}

#[test]
fn loudness_bs1770_accuracy_gating_and_momentary_short_term() {
    // 1 kHz stereo sine with peak 0.1 (-20 dBFS peak each channel) -> ~ -20.0 LUFS
    let mut tone = sine_f32(1000.0, 0.1, RATE as usize * 3);
    // Append 4 seconds of -45 dBFS quiet tail (above -70 LKFS, below -10 dB relative gate!)
    let quiet = sine_f32(1000.0, 0.005623, RATE as usize * 4);
    tone.extend_from_slice(&quiet);

    let mut meter = LoudnessMeter::new();
    meter.process_f32(&tone);
    let stats = meter.finish();

    // Relative -10 dB gate must exclude the -45 LUFS tail so integrated remains near -20.0 LUFS
    assert!(
        abs_f32(stats.integrated_lufs - (-20.0)) < 0.35,
        "gated integrated LUFS must exclude quiet tail and read ~-20.0, got {}",
        stats.integrated_lufs
    );
    assert!(
        abs_f32(stats.momentary_max_lufs - (-20.0)) < 0.35,
        "momentary max must read ~-20.0, got {}",
        stats.momentary_max_lufs
    );
    assert!(
        stats.short_term_lufs < stats.momentary_max_lufs - 2.0,
        "latest 3s short-term window includes quiet tail and must be lower: st={}, mmax={}",
        stats.short_term_lufs,
        stats.momentary_max_lufs
    );
    assert!(stats.loudness_range_lu > 5.0);
}

#[test]
fn true_peak_4x_oversampling_catches_intersample_peak() {
    // fs/4 (12 kHz) at 45 degrees: every discrete sample is +-0.7071 (-3.01 dBFS),
    // but true continuous inter-sample peak is 1.000 (0.0 dBTP).
    let mut frames = Vec::with_capacity(2048);
    for i in 0..2048 {
        let s = match i % 4 {
            0 | 1 => 0.70710677_f32,
            _ => -0.70710677_f32,
        };
        frames.push([s, s]);
    }
    let tp = TruePeakDetector::measure_slice(&frames);
    assert!(
        tp > 0.96,
        "4x True-Peak must detect ~1.0 intersample peak from 0.7071 samples, got {tp}"
    );
    let stats = measure_f32(&frames, RATE).unwrap();
    assert!(
        stats.true_peak_dbtp > -0.35,
        "dBTP must be near 0.0 dBTP (not -3.01 dBFS), got {}",
        stats.true_peak_dbtp
    );
}

#[test]
fn normalize_to_delivery_targets_with_brickwall_ceiling() {
    // Quiet speech-like tone + occasional sharp transient spike
    let len = RATE as usize * 2;
    let mut frames = sine_frames(1000.0, 0.04, len); // ~ -28 LUFS
    // Add periodic spikes that will hit the -1.0 dBTP brickwall when boosted
    for k in (1000..len).step_by(8000) {
        frames[k] = [24000, 24000];
        frames[k + 1] = [24000, 24000];
    }

    for target in [
        DeliveryTarget::StreamingMinus14,
        DeliveryTarget::PodcastMinus16,
        DeliveryTarget::BroadcastMinus23,
        DeliveryTarget::FilmMinus24,
    ] {
        let mut buf = frames.clone();
        let report = normalize_to_target(&mut buf, target).unwrap();
        let verify = measure_frames(&buf);
        assert!(
            verify.true_peak_dbtp <= target.max_true_peak_dbtp() + 0.02,
            "True-Peak must respect -1.0 dBTP brickwall for {:?}, got {}",
            target,
            verify.true_peak_dbtp
        );
        assert!(
            abs_f32(report.after.integrated_lufs - target.target_lufs()) < 0.8,
            "Target {:?} LUFS={} achieved={}",
            target,
            target.target_lufs(),
            report.after.integrated_lufs
        );
    }
}

#[test]
fn elastic_stretch_time_scale_0_25x_to_4_0x_preserves_pitch() {
    let src = sine_frames(440.0, 0.4, RATE as usize); // 1.0 s @ 440 Hz

    for ratio in [0.25f32, 0.5, 2.0, 4.0] {
        let stretch = ElasticStretch::new(ratio, 0.0).unwrap();
        let out = stretch.process_frames(&src).unwrap();
        let expected_len = (src.len() as f32 / ratio) as usize;
        assert!(
            (out.len() as isize - expected_len as isize).abs() <= 2,
            "ratio {ratio}: len {} != expected {expected_len}",
            out.len()
        );
        let freq = estimate_freq_hz(&out);
        assert!(
            abs_f32(freq - 440.0) < 12.0,
            "ratio {ratio} must preserve 440 Hz pitch without chipmunk effect, got {freq} Hz"
        );
    }
}

#[test]
fn elastic_stretch_pitch_shift_preserves_duration_and_validates_bounds() {
    let src = sine_frames(440.0, 0.4, RATE as usize);

    for (semitones, expected_hz) in [(12.0f32, 880.0f32), (7.0, 659.25), (-12.0, 220.0)] {
        let stretch = ElasticStretch::new(1.0, semitones).unwrap();
        let out = stretch.process_frames(&src).unwrap();
        assert_eq!(out.len(), src.len(), "duration must be preserved");
        let freq = estimate_freq_hz(&out);
        assert!(
            abs_f32(freq - expected_hz) < 18.0,
            "shift {semitones} st expected ~{expected_hz} Hz, got {freq} Hz"
        );
    }

    // Vocal formant-preserving mode
    let vocal_shift = ElasticStretch::vocal(1.0, 5.0).unwrap();
    let v_out = vocal_shift.process_frames(&src).unwrap();
    assert_eq!(v_out.len(), src.len());

    // Bounds validation
    assert_eq!(ElasticStretch::new(0.2, 0.0).err(), Some(StretchError::InvalidParameter));
    assert_eq!(ElasticStretch::new(4.5, 0.0).err(), Some(StretchError::InvalidParameter));
    assert_eq!(ElasticStretch::new(1.0, 13.0).err(), Some(StretchError::InvalidParameter));
    assert_eq!(ElasticStretch::new(1.0, -13.0).err(), Some(StretchError::InvalidParameter));
}

#[test]
fn interchange_otio_edl_aaf_roundtrip_and_splicy_compat() {
    let mut tl = InterchangeTimeline::new("Episode 42 \"Launch\"");
    tl.fps_num = 30;
    tl.fps_den = 1;
    tl.video_sync_offset_frames = -2400; // -50 ms audio lead for video sync
    tl.loudness = InterchangeLoudness {
        integrated_lufs: Some(-16.0),
        true_peak_dbtp: Some(-1.2),
        loudness_range_lu: Some(4.5),
        target_lufs: Some(-16.0),
    };
    tl.markers.push(InterchangeMarker::new("Intro", 48_000));

    let mut track = InterchangeTrack::audio("Host Mic");
    track.volume_percent = 110;
    track.pan = -15;
    let mut clip = InterchangeClip::new(
        "host_take1",
        "file:///data/takes/host_take1.wav",
        48_000,
        24_000,
        96_000,
    );
    clip.fade_in_frames = 4_800;
    clip.fade_out_frames = 9_600;
    clip.gain_percent = 125;
    clip.markers.push(InterchangeMarker::new("Chapter 1", 12_000));
    track.clips.push(clip);
    tl.tracks.push(track);

    // 1. OTIO JSON roundtrip
    let otio = tl.to_otio_json();
    let from_otio = InterchangeTimeline::from_otio_json(&otio).unwrap();
    assert_eq!(from_otio.name, tl.name);
    assert_eq!(from_otio.video_sync_offset_frames, -2400);
    assert_eq!(from_otio.loudness.integrated_lufs, Some(-16.0));
    assert_eq!(from_otio.markers.len(), 1);
    assert_eq!(from_otio.tracks[0].volume_percent, 110);
    assert_eq!(from_otio.tracks[0].pan, -15);
    let c_otio = &from_otio.tracks[0].clips[0];
    assert_eq!(c_otio.timeline_start_frame, 48_000);
    assert_eq!(c_otio.source_start_frame, 24_000);
    assert_eq!(c_otio.duration_frames, 96_000);
    assert_eq!(c_otio.fade_in_frames, 4_800);
    assert_eq!(c_otio.fade_out_frames, 9_600);
    assert_eq!(c_otio.gain_percent, 125);
    assert_eq!(c_otio.markers[0].name, "Chapter 1");

    // 2. CMX 3600 EDL roundtrip
    let edl = tl.to_cmx3600_edl();
    let from_edl = InterchangeTimeline::from_cmx3600_edl(&edl, RATE, 30, 1).unwrap();
    assert_eq!(from_edl.video_sync_offset_frames, -2400);
    assert_eq!(from_edl.loudness.target_lufs, Some(-16.0));
    let c_edl = &from_edl.tracks[0].clips[0];
    assert_eq!(c_edl.name, "host_take1");
    assert_eq!(c_edl.timeline_start_frame, 48_000);
    assert_eq!(c_edl.source_start_frame, 24_000);
    assert_eq!(c_edl.duration_frames, 96_000);
    assert_eq!(c_edl.fade_in_frames, 4_800);
    assert_eq!(c_edl.fade_out_frames, 9_600);

    // 3. AAF-XML summary roundtrip
    let xml = tl.to_aaf_xml();
    let from_xml = InterchangeTimeline::from_aaf_xml(&xml).unwrap();
    assert_eq!(from_xml, tl);

    // 4. Splicy-style 30 fps OTIO import (Gap.1 + Clip.2)
    let splicy_otio = r#"{
        "OTIO_SCHEMA": "Timeline.1",
        "name": "Splicy Cut",
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "children": [{
                "OTIO_SCHEMA": "Track.1",
                "name": "V1",
                "kind": "Video",
                "children": [
                    {"OTIO_SCHEMA":"Gap.1","name":"","source_range":{"OTIO_SCHEMA":"TimeRange.1","start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0.0},"duration":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":30.0}}},
                    {"OTIO_SCHEMA":"Clip.2","name":"cam.webm","source_range":{"OTIO_SCHEMA":"TimeRange.1","start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":60.0},"duration":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":90.0}},"media_references":{"DEFAULT_MEDIA":{"OTIO_SCHEMA":"ExternalReference.1","target_url":"/media/cam.webm"}},"active_media_reference_key":"DEFAULT_MEDIA"}
                ]
            }]
        }
    }"#;
    let imported = InterchangeTimeline::from_otio_json(splicy_otio).unwrap();
    assert_eq!(imported.tracks[0].kind, InterchangeTrackKind::Video);
    assert_eq!(imported.tracks[0].clips[0].timeline_start_frame, 48_000); // 30 frames @ 30fps = 1s
    assert_eq!(imported.tracks[0].clips[0].source_start_frame, 96_000); // 60 frames = 2s
    assert_eq!(imported.tracks[0].clips[0].duration_frames, 144_000); // 90 frames = 3s
    assert_eq!(imported.tracks[0].clips[0].media_url, "/media/cam.webm");

    // 5. Moan Track conversion
    let pcm = vec![[1000i16, -1000i16]; 48_000];
    let mut clip_moan = Clip::new(&pcm, 24_000).unwrap();
    clip_moan.set_fades(1200, 2400).unwrap();
    clip_moan.set_gain(150).unwrap();
    let mut tr = Track::new();
    tr.clips.push(clip_moan);
    tr.set_volume(90).unwrap();
    tr.set_pan(-20).unwrap();
    let exported = InterchangeTimeline::from_tracks("Moan Mix", &[tr]);
    assert_eq!(exported.tracks[0].volume_percent, 90);
    assert_eq!(exported.tracks[0].pan, -20);
    assert_eq!(exported.tracks[0].clips[0].timeline_start_frame, 24_000);
    assert_eq!(exported.tracks[0].clips[0].fade_in_frames, 1200);
    assert_eq!(exported.tracks[0].clips[0].fade_out_frames, 2400);
    assert_eq!(exported.tracks[0].clips[0].gain_percent, 150);
    let _check_str = String::from("ok");
}

#[test]
fn voice_cleanup_allpass_bypass_and_nan_safety() {
    // 0 dB denoise reduction must pass a 1 kHz tone with flat unity amplitude
    let mut denoise = VoiceDenoise::with_config(0.0, 1.5, RATE as f32);
    let mut tone = sine_f32(1000.0, 0.25, RATE as usize / 4);
    let before = rms_f32(&tone);
    denoise.process_f32(&mut tone);
    let after = rms_f32(&tone[1000..]);
    assert!(abs_f32(after - before) / before < 0.01);

    // NaN sample in a VoiceCleanupChain must not permanently poison IIR state
    let mut chain = VoiceCleanupChain::from_preset(VoicePreset::BroadcastVoice);
    let out_nan = chain.process_frame_f32([f32::NAN, f32::INFINITY]);
    assert!(out_nan[0].is_finite() && out_nan[1].is_finite());
    let out_next = chain.process_frame_f32([0.2, -0.2]);
    assert!(out_next[0].is_finite() && out_next[1].is_finite());
}

#[test]
fn loudness_handles_empty_and_digital_silence_safely() {
    let empty: Vec<Frame> = Vec::new();
    let m_empty = measure_frames(&empty);
    assert!(m_empty.integrated_lufs <= -100.0);

    let mut silence = vec![[0i16; 2]; RATE as usize];
    let rep = normalize_to_target(&mut silence, DeliveryTarget::PodcastMinus16).unwrap();
    assert_eq!(rep.gain_applied_db, 0.0);
    assert!(!rep.limiter_engaged);
    assert!(silence.iter().all(|&f| f == [0, 0]));
}

#[test]
fn interchange_rejects_malformed_inputs() {
    assert!(InterchangeTimeline::from_otio_json("{broken").is_err());
    assert!(InterchangeTimeline::from_otio_json("{\"OTIO_SCHEMA\":\"Clip.2\"}").is_err());
    assert!(InterchangeTimeline::from_cmx3600_edl("just garbage", RATE, 30, 1).is_err());
    assert!(InterchangeTimeline::from_aaf_xml("<WrongRoot/>").is_err());
}

