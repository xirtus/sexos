extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::TAU;

use serenade_core::edit::AudioClip;
use serenade_core::polish::{
    clean_hum, de_ess, measure_loudness, normalize_podcast_lufs, polish_podcast_voice,
    suppress_background_noise, PODCAST_TARGET_LUFS, TRUE_PEAK_CEILING_DBTP,
};
use serenade_core::SerenadeError;

fn make_sine_clip(channels: u16, sample_rate: u32, freq_hz: f32, amp: f32, secs: f32) -> AudioClip {
    let frames = (sample_rate as f32 * secs) as usize;
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let v = amp * libm::sinf(TAU * freq_hz * i as f32 / sample_rate as f32);
        for _ in 0..channels {
            samples.push(v);
        }
    }
    AudioClip::new(channels, sample_rate, samples, Vec::new()).unwrap()
}

fn clip_rms(clip: &AudioClip, start_frame: usize) -> f32 {
    let ch = clip.channels as usize;
    let slice = &clip.samples[start_frame * ch..];
    if slice.is_empty() {
        return 0.0;
    }
    let sum: f64 = slice.iter().map(|&s| (s as f64) * (s as f64)).sum();
    libm::sqrt(sum / slice.len() as f64) as f32
}

#[test]
fn serenade_clean_hum_50_and_60_hz_mono_and_stereo() {
    // 60 Hz hum + 120 Hz + 180 Hz on mono 48 kHz clip
    let sr = 48_000u32;
    let frames = sr as usize;
    let samples: Vec<f32> = (0..frames)
        .map(|i| {
            let t = i as f32 / sr as f32;
            0.2 * libm::sinf(TAU * 60.0 * t)
                + 0.12 * libm::sinf(TAU * 120.0 * t)
                + 0.08 * libm::sinf(TAU * 180.0 * t)
        })
        .collect();
    let mut mono = AudioClip::new(1, sr, samples, Vec::new()).unwrap();
    let before = clip_rms(&mono, frames / 2);
    clean_hum(&mut mono, 60.0).unwrap();
    let after = clip_rms(&mono, frames / 2);
    assert!(
        after < before * 0.08,
        "mono 60 Hz hum must be deeply notched: before={before}, after={after}"
    );

    // 440 Hz voice tone on stereo 44.1 kHz clip preserved
    let mut voice = make_sine_clip(2, 44_100, 440.0, 0.3, 1.0);
    let before_v = clip_rms(&voice, 11_025);
    voice.clean_hum(50.0).unwrap();
    let after_v = clip_rms(&voice, 11_025);
    assert!(
        libm::fabsf(after_v - before_v) / before_v < 0.03,
        "440 Hz voice must remain intact: before={before_v}, after={after_v}"
    );

    // Invalid hum frequency rejected
    assert!(matches!(
        voice.clean_hum(5.0),
        Err(SerenadeError::InvalidParameter(_))
    ));
}

#[test]
fn serenade_suppress_background_noise_auto_learns_room_tone() {
    let sr = 48_000u32;
    let mut seed: u32 = 0xCAFE_BABEu32;
    let mut next_noise = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let u = ((seed >> 8) as f32) / 16777216.0 * 2.0 - 1.0;
        u * 0.025
    };

    // 0.5 s room noise followed by 0.5 s speech (500 Hz + 1400 Hz) + room noise
    let half = (sr / 2) as usize;
    let mut samples = Vec::with_capacity(sr as usize);
    for _ in 0..half {
        samples.push(next_noise());
    }
    for i in 0..half {
        let t = i as f32 / sr as f32;
        let speech = 0.28 * libm::sinf(TAU * 500.0 * t) + 0.16 * libm::sinf(TAU * 1400.0 * t);
        samples.push(speech + next_noise());
    }

    let mut clip = AudioClip::new(1, sr, samples, Vec::new()).unwrap();
    let noise_before = {
        let s = &clip.samples[sr as usize / 8..half];
        let sum: f64 = s.iter().map(|&v| (v as f64) * (v as f64)).sum();
        libm::sqrt(sum / s.len() as f64) as f32
    };
    let speech_before = clip_rms(&clip, half + half / 4);

    suppress_background_noise(&mut clip, 18.0).unwrap();

    let noise_after = {
        let s = &clip.samples[sr as usize / 8..half];
        let sum: f64 = s.iter().map(|&v| (v as f64) * (v as f64)).sum();
        libm::sqrt(sum / s.len() as f64) as f32
    };
    let speech_after = clip_rms(&clip, half + half / 4);

    assert!(
        noise_after < noise_before * 0.30,
        "background noise must be suppressed: before={noise_before}, after={noise_after}"
    );
    assert!(
        speech_after > speech_before * 0.78,
        "speech must stay intelligible: before={speech_before}, after={speech_after}"
    );
}

#[test]
fn serenade_de_ess_and_loudness_readout_and_podcast_normalization() {
    // 1. De-ess harsh 7 kHz sibilance while preserving 400 Hz tone
    let mut sib = make_sine_clip(2, 48_000, 7000.0, 0.35, 0.5);
    let sib_before = clip_rms(&sib, 6000);
    de_ess(&mut sib, -24.0, 12.0).unwrap();
    let sib_after = clip_rms(&sib, 6000);
    assert!(
        sib_after < sib_before * 0.48,
        "7 kHz sibilance must drop significantly: before={sib_before}, after={sib_after}"
    );

    let mut vowel = make_sine_clip(2, 48_000, 400.0, 0.35, 0.5);
    let v_before = clip_rms(&vowel, 6000);
    vowel.de_ess(-24.0, 12.0).unwrap();
    let v_after = clip_rms(&vowel, 6000);
    assert!(libm::fabsf(v_after - v_before) / v_before < 0.03);

    // 2. LUFS & True-Peak readout + Podcast normalization (-16 LUFS, <= -1 dBTP)
    let mut quiet = make_sine_clip(2, 48_000, 1000.0, 0.03, 1.5);
    // Inject transient spikes
    for k in (2000..quiet.frame_count()).step_by(8000) {
        quiet.samples[k * 2] = 0.75;
        quiet.samples[k * 2 + 1] = 0.75;
    }

    let before_lufs = measure_loudness(&quiet).unwrap();
    assert_eq!(before_lufs, quiet.loudness().unwrap());
    assert!(before_lufs.integrated_lufs < -22.0);

    let after_lufs = normalize_podcast_lufs(&mut quiet, PODCAST_TARGET_LUFS).unwrap();
    assert!(
        libm::fabsf(after_lufs.integrated_lufs - PODCAST_TARGET_LUFS) < 0.75,
        "normalized LUFS should hit -16.0 target, got {}",
        after_lufs.integrated_lufs
    );
    assert!(
        after_lufs.true_peak_dbtp <= TRUE_PEAK_CEILING_DBTP + 0.02,
        "True-Peak must respect -1.0 dBTP ceiling, got {}",
        after_lufs.true_peak_dbtp
    );

    // 3. 1-click full studio podcast voice polish on mono 44.1 kHz recording
    let mut voice_memo = make_sine_clip(1, 44_100, 320.0, 0.08, 1.5);
    for i in 0..voice_memo.frame_count() {
        voice_memo.samples[i] += 0.05 + 0.06 * libm::sinf(TAU * 60.0 * i as f32 / 44_100.0);
    }
    let polished = polish_podcast_voice(&mut voice_memo, 60.0).unwrap();
    assert!(libm::fabsf(polished.integrated_lufs - PODCAST_TARGET_LUFS) < 0.5);
    assert!(polished.true_peak_dbtp <= TRUE_PEAK_CEILING_DBTP + 0.02);

    let _unused_vec: Vec<f32> = vec![];
}

#[test]
fn serenade_polish_edge_cases_and_true_peak_intersample() {
    // Empty and pure silence clips are handled cleanly
    let mut empty = AudioClip::new(1, 48_000, Vec::new(), Vec::new()).unwrap();
    let r_empty = empty.normalize_podcast_lufs(PODCAST_TARGET_LUFS).unwrap();
    assert!(r_empty.integrated_lufs <= -100.0);

    let mut silence = AudioClip::new(2, 48_000, vec![0.0; 96_000], Vec::new()).unwrap();
    let r_sil = polish_podcast_voice(&mut silence, 50.0).unwrap();
    assert!(r_sil.integrated_lufs <= -100.0);
    assert!(silence.samples.iter().all(|&s| s == 0.0));

    // Parameter validation
    assert!(matches!(
        silence.suppress_background_noise(50.0),
        Err(SerenadeError::InvalidParameter(_))
    ));
    assert!(matches!(
        silence.de_ess(5.0, 10.0),
        Err(SerenadeError::InvalidParameter(_))
    ));
    assert!(matches!(
        silence.normalize_podcast_lufs(2.0),
        Err(SerenadeError::InvalidParameter(_))
    ));

    // fs/4 45-deg inter-sample peak test on AudioClip::loudness()
    let mut is_samples = Vec::with_capacity(2048);
    for i in 0..2048 {
        is_samples.push(match i % 4 {
            0 | 1 => 0.70710677_f32,
            _ => -0.70710677_f32,
        });
    }
    let is_clip = AudioClip::new(1, 48_000, is_samples, Vec::new()).unwrap();
    let readout = is_clip.loudness().unwrap();
    assert!(
        readout.true_peak_dbtp > -0.35,
        "4x True-Peak on AudioClip must exceed -3.01 dBFS sample peak, got {}",
        readout.true_peak_dbtp
    );
}

