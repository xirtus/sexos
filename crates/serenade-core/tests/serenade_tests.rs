extern crate alloc;

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use serenade_core::edit::{AudioClip, FadeCurve};
use serenade_core::meter::{
    dbfs_to_linear, linear_to_dbfs, linear_to_dbfs_approx, LevelMeter,
};
use serenade_core::recorder::{
    CaptureSession, RecorderConfig, RecorderState, VoxConfig, VoxStatus,
};
use serenade_core::wav::{
    decode_wav, encode_wav, WavError, WavFormat, WavMetadata,
};
use serenade_core::waveform::WaveformMipmap;
use serenade_core::SerenadeError;

#[test]
fn test_meter_dbfs_conversions() {
    // 1.0 linear = 0.0 dBFS
    assert!((linear_to_dbfs(1.0) - 0.0).abs() < 1e-4);
    assert_eq!(linear_to_dbfs_approx(1.0), 0);

    // 0.5 linear ~ -6.02 dBFS
    assert!((linear_to_dbfs(0.5) - (-6.0206)).abs() < 1e-2);
    let approx_half = linear_to_dbfs_approx(0.5);
    assert!(approx_half == -6 || approx_half == -7, "approx: {}", approx_half);

    // 0.25 linear ~ -12.04 dBFS
    assert!((linear_to_dbfs(0.25) - (-12.041)).abs() < 1e-2);
    let approx_quarter = linear_to_dbfs_approx(0.25);
    assert!(approx_quarter >= -13 && approx_quarter <= -11, "approx: {}", approx_quarter);

    // 0.1 linear = -20 dBFS
    assert!((linear_to_dbfs(0.1) - (-20.0)).abs() < 1e-2);
    let approx_tenth = linear_to_dbfs_approx(0.1);
    assert!(approx_tenth >= -21 && approx_tenth <= -19, "approx: {}", approx_tenth);

    // dbfs to linear round trip
    assert!((dbfs_to_linear(0.0) - 1.0).abs() < 1e-4);
    assert!((dbfs_to_linear(-6.0206) - 0.5).abs() < 1e-3);
    assert!((dbfs_to_linear(-20.0) - 0.1).abs() < 1e-3);
}

#[test]
fn test_meter_peak_rms_and_clipping() {
    let mut meter = LevelMeter::new(2, 48000, 100, 20.0);

    // Feed stereo audio: Left = 0.5, Right = 1.05 (clipping!)
    let frames = [0.5f32, 1.05f32, -0.5f32, -1.05f32];
    meter.process_block(&frames);

    let snap = meter.snapshot();
    // Left channel
    assert!((snap[0].peak - 0.5).abs() < 1e-4);
    assert_eq!(snap[0].clip_count, 0);

    // Right channel had clipping
    assert!((snap[1].peak - 1.05).abs() < 1e-4);
    assert_eq!(snap[1].clip_count, 2);

    meter.reset_clip_counts();
    assert_eq!(meter.snapshot()[1].clip_count, 0);
}

#[test]
fn test_waveform_mipmap_pyramid() {
    let mut mipmap = WaveformMipmap::new(2);

    // Feed 512 stereo frames of a sine/step pattern
    let mut samples = Vec::new();
    for i in 0..512 {
        let v = (i as f32 / 256.0) - 1.0;
        samples.push(v);
        samples.push(-v);
    }
    mipmap.feed_block(&samples);

    assert_eq!(mipmap.total_frames(), 512);
    // Level 0 has 64 frames per bin -> 512 / 64 = 8 bins
    assert_eq!(mipmap.level_len(0), 8);
    // Level 1 has 256 frames per bin -> 512 / 256 = 2 bins
    assert_eq!(mipmap.level_len(1), 2);

    // Query 4 columns
    let cols = mipmap.query_range(0, 512, 4);
    assert_eq!(cols.len(), 4);
    // Peak min and max should span [-1.0, 1.0]
    assert!(cols[0].min_left < 0.0);
    assert!(cols[3].max_left > 0.0);
}

#[test]
fn test_recorder_session_and_memory_cap() {
    let config = RecorderConfig {
        channels: 2,
        sample_rate: 48000,
        max_samples: 100, // strict 100 samples cap
        vox: None,
    };
    let mut session = CaptureSession::new(config).unwrap();
    assert_eq!(session.state(), RecorderState::Idle);

    // Feeding while Idle does not record
    let block = [0.1f32; 20];
    let stats = session.feed_f32(&block).unwrap();
    assert_eq!(stats.frames_recorded, 0);
    assert_eq!(session.frame_count(), 0);

    // Start
    session.start().unwrap();
    assert_eq!(session.state(), RecorderState::Recording);

    // Add marker
    let m1 = session.add_marker("Intro").unwrap();
    assert_eq!(m1.id, 1);
    assert_eq!(m1.name, "Intro");
    assert_eq!(m1.frame_index, 0);

    // Feed 40 stereo frames (80 samples)
    let samples = vec![0.2f32; 80];
    let stats = session.feed_f32(&samples).unwrap();
    assert_eq!(stats.frames_recorded, 40);
    assert_eq!(session.frame_count(), 40);

    // Feed i16 frames: 10 frames = 20 samples -> reaches 100 cap
    let i16_samples = vec![3276i16; 20];
    let stats = session.feed_i16(&i16_samples).unwrap();
    assert_eq!(stats.frames_recorded, 10);
    assert_eq!(session.frame_count(), 50); // 50 frames * 2 channels = 100 samples

    // Exceeding cap returns BufferFull without crashing
    let over_samples = vec![0.5f32; 4];
    let err = session.feed_f32(&over_samples).unwrap_err();
    match err {
        SerenadeError::BufferFull { current_samples, max_samples } => {
            assert_eq!(current_samples, 100);
            assert_eq!(max_samples, 100);
        }
        _ => panic!("Expected BufferFull error"),
    }

    // Pause and Resume
    session.pause().unwrap();
    assert_eq!(session.state(), RecorderState::Paused);
    session.resume().unwrap();
    assert_eq!(session.state(), RecorderState::Recording);

    // Stop and extract AudioClip
    let clip = session.stop().unwrap();
    assert_eq!(clip.channels, 2);
    assert_eq!(clip.sample_rate, 48000);
    assert_eq!(clip.frame_count(), 50);
    assert_eq!(clip.markers.len(), 1);
    assert_eq!(clip.markers[0].name, "Intro");
}

#[test]
fn test_vox_voice_activated_recording() {
    let vox = VoxConfig {
        threshold_dbfs: -30.0, // ~0.0316 linear
        hold_ms: 10,           // short hold for testing
        pre_roll_ms: 5,        // short pre-roll
    };
    let config = RecorderConfig {
        channels: 1,
        sample_rate: 1000, // 1000 Hz: 1 sample = 1 ms
        max_samples: 10000,
        vox: Some(vox),
    };

    let mut session = CaptureSession::new(config).unwrap();
    session.start().unwrap();

    // 1. Feed 20 frames of silence (0.001 << threshold)
    let silence = vec![0.001f32; 20];
    let stats = session.feed_f32(&silence).unwrap();
    assert_eq!(stats.frames_recorded, 0);
    assert_eq!(stats.frames_gated, 20);
    assert_eq!(session.vox_status(), VoxStatus::Gated);

    // 2. Feed voice (0.2 > threshold) -> triggers VOX and flushes pre-roll (5 frames)
    let voice = vec![0.2f32; 10];
    let _stats = session.feed_f32(&voice).unwrap();
    // 5 frames pre-roll + 10 frames voice = 15 frames recorded!
    assert_eq!(session.frame_count(), 15);
    assert_eq!(session.vox_status(), VoxStatus::Active);

    // 3. Audio drops to silence -> enters Hold for 10 ms (10 frames)
    let drop = vec![0.001f32; 5];
    session.feed_f32(&drop).unwrap();
    assert_eq!(session.vox_status(), VoxStatus::Hold);
    assert_eq!(session.frame_count(), 20);

    // 4. Another 10 frames of silence -> hold expires and returns to Gated
    let more_silence = vec![0.001f32; 15];
    let stats = session.feed_f32(&more_silence).unwrap();
    // 5 frames of hold recorded, then 10 frames gated
    assert_eq!(stats.frames_recorded, 5);
    assert_eq!(stats.frames_gated, 10);
    assert_eq!(session.vox_status(), VoxStatus::Gated);
}

#[test]
fn test_clip_editing_operations() {
    let mut samples = Vec::new();
    // Create 100 frames of mono audio with a DC offset of 0.5
    for i in 0..100 {
        samples.push(0.5 + (i as f32 / 200.0));
    }

    let markers = vec![
        serenade_core::Marker {
            id: 1,
            name: "Midpoint".to_string(),
            frame_index: 50,
            timestamp_ms: 50,
        },
    ];

    let mut clip = AudioClip::new(1, 1000, samples, markers).unwrap();
    assert_eq!(clip.frame_count(), 100);
    assert_eq!(clip.duration_ms(), 100);

    // DC offset removal
    clip.remove_dc_offset();
    let mean: f32 = clip.samples.iter().sum::<f32>() / 100.0;
    assert!(mean.abs() < 1e-5);

    // Normalize peak to 0.8
    let gain = clip.normalize_peak(0.8).unwrap();
    assert!(gain > 0.0);
    let max_abs = clip.samples.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
    assert!((max_abs - 0.8).abs() < 1e-4);

    // Fade curves
    clip.fade_in(20, FadeCurve::Cosine).unwrap();
    assert_eq!(clip.samples[0], 0.0);
    clip.fade_out(20, FadeCurve::Linear).unwrap();
    assert_eq!(*clip.samples.last().unwrap(), 0.0);

    // Trim
    let trimmed = clip.trim(20, 80).unwrap();
    assert_eq!(trimmed.frame_count(), 60);
    // Marker at 50 was re-indexed to 30
    assert_eq!(trimmed.markers.len(), 1);
    assert_eq!(trimmed.markers[0].frame_index, 30);

    // Split at markers
    let splits = trimmed.split_at_markers().unwrap();
    assert_eq!(splits.len(), 2);
    assert_eq!(splits[0].frame_count(), 30);
    assert_eq!(splits[1].frame_count(), 30);
}

#[test]
fn test_wav_roundtrip_all_formats_and_markers() {
    let mut samples = Vec::new();
    for i in 0..480 {
        let s = (i as f32 / 240.0) - 1.0;
        samples.push(s);
        samples.push(-s);
    }

    let markers = vec![
        serenade_core::Marker {
            id: 1,
            name: "Verse".to_string(),
            frame_index: 100,
            timestamp_ms: 2,
        },
        serenade_core::Marker {
            id: 2,
            name: "Chorus".to_string(),
            frame_index: 250,
            timestamp_ms: 5,
        },
    ];

    let clip = AudioClip::new(2, 48000, samples, markers).unwrap();
    let metadata = WavMetadata {
        title: Some("Serenade Anthem".to_string()),
        artist: Some("Andreas".to_string()),
        date: Some("2026-11-14".to_string()),
        comment: Some("Clean pure Rust SASOS audio".to_string()),
        genre: Some("Cyberpunk".to_string()),
        software: Some("Serenade 1.0".to_string()),
    };

    // 1. PCM16 round-trip
    let pcm16_bytes = encode_wav(&clip, WavFormat::Pcm16, &metadata).unwrap();
    let decoded_pcm16 = decode_wav(&pcm16_bytes).unwrap();
    assert_eq!(decoded_pcm16.format, WavFormat::Pcm16);
    assert_eq!(decoded_pcm16.clip.channels, 2);
    assert_eq!(decoded_pcm16.clip.sample_rate, 48000);
    assert_eq!(decoded_pcm16.clip.frame_count(), 480);
    assert_eq!(decoded_pcm16.clip.markers.len(), 2);
    assert_eq!(decoded_pcm16.clip.markers[0].name, "Verse");
    assert_eq!(decoded_pcm16.clip.markers[1].name, "Chorus");
    assert_eq!(decoded_pcm16.metadata.title.as_deref(), Some("Serenade Anthem"));
    assert_eq!(decoded_pcm16.metadata.artist.as_deref(), Some("Andreas"));

    // 2. PCM24 round-trip
    let pcm24_bytes = encode_wav(&clip, WavFormat::Pcm24, &metadata).unwrap();
    let decoded_pcm24 = decode_wav(&pcm24_bytes).unwrap();
    assert_eq!(decoded_pcm24.format, WavFormat::Pcm24);
    assert_eq!(decoded_pcm24.clip.frame_count(), 480);
    assert_eq!(decoded_pcm24.clip.markers.len(), 2);

    // 3. Float32 round-trip (bit-exact)
    let f32_bytes = encode_wav(&clip, WavFormat::Float32, &metadata).unwrap();
    let decoded_f32 = decode_wav(&f32_bytes).unwrap();
    assert_eq!(decoded_f32.format, WavFormat::Float32);
    assert_eq!(decoded_f32.clip.frame_count(), 480);
    for (orig, dec) in clip.samples.iter().zip(decoded_f32.clip.samples.iter()) {
        assert!((orig - dec).abs() < 1e-6);
    }
}

#[test]
fn test_adversarial_malformed_wav_inputs() {
    // Empty buffer
    assert!(matches!(decode_wav(&[]), Err(SerenadeError::Wav(WavError::InvalidRiffHeader))));

    // Truncated header
    assert!(matches!(decode_wav(b"RIFF\x00\x00\x00\x00WAV"), Err(SerenadeError::Wav(WavError::InvalidRiffHeader))));

    // Wrong magic
    assert!(matches!(decode_wav(b"FORM\x04\x00\x00\x00AIFF"), Err(SerenadeError::Wav(WavError::InvalidRiffHeader))));

    // Valid RIFF/WAVE header but chunk size causes integer overflow / truncated
    let mut overflow_chunk = Vec::new();
    overflow_chunk.extend_from_slice(b"RIFF\x20\x00\x00\x00WAVE");
    overflow_chunk.extend_from_slice(b"fmt ");
    overflow_chunk.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes()); // huge chunk size
    overflow_chunk.extend_from_slice(&[0u8; 16]);
    assert!(matches!(decode_wav(&overflow_chunk), Err(SerenadeError::Wav(WavError::TruncatedData))));

    // Valid header but missing fmt chunk
    let mut missing_fmt = Vec::new();
    missing_fmt.extend_from_slice(b"RIFF\x10\x00\x00\x00WAVE");
    missing_fmt.extend_from_slice(b"data\x04\x00\x00\x00\x00\x00\x00\x00");
    assert!(matches!(decode_wav(&missing_fmt), Err(SerenadeError::Wav(WavError::MissingFmtChunk))));

    // Valid header and fmt chunk, but zero sample rate
    let mut zero_sr = Vec::new();
    zero_sr.extend_from_slice(b"RIFF\x24\x00\x00\x00WAVE");
    zero_sr.extend_from_slice(b"fmt \x10\x00\x00\x00");
    zero_sr.extend_from_slice(&1u16.to_le_bytes()); // PCM
    zero_sr.extend_from_slice(&2u16.to_le_bytes()); // 2 ch
    zero_sr.extend_from_slice(&0u32.to_le_bytes()); // 0 sample rate!
    zero_sr.extend_from_slice(&0u32.to_le_bytes());
    zero_sr.extend_from_slice(&4u16.to_le_bytes());
    zero_sr.extend_from_slice(&16u16.to_le_bytes());
    zero_sr.extend_from_slice(b"data\x04\x00\x00\x00\x00\x00\x00\x00");
    assert!(matches!(decode_wav(&zero_sr), Err(SerenadeError::Wav(WavError::InvalidSampleRate))));

    // Unsupported channel count (e.g. 6 channels surround)
    let mut six_ch = Vec::new();
    six_ch.extend_from_slice(b"RIFF\x24\x00\x00\x00WAVE");
    six_ch.extend_from_slice(b"fmt \x10\x00\x00\x00");
    six_ch.extend_from_slice(&1u16.to_le_bytes());
    six_ch.extend_from_slice(&6u16.to_le_bytes()); // 6 channels!
    six_ch.extend_from_slice(&48000u32.to_le_bytes());
    six_ch.extend_from_slice(&576000u32.to_le_bytes());
    six_ch.extend_from_slice(&12u16.to_le_bytes());
    six_ch.extend_from_slice(&16u16.to_le_bytes());
    assert!(matches!(decode_wav(&six_ch), Err(SerenadeError::Wav(WavError::InvalidChannelCount))));

    // 44-byte WAV header claiming 4 GiB data size (0xFFFF_FFFF) without actual data
    let mut huge_data = Vec::new();
    huge_data.extend_from_slice(b"RIFF\x24\x00\x00\x00WAVE");
    huge_data.extend_from_slice(b"fmt \x10\x00\x00\x00");
    huge_data.extend_from_slice(&1u16.to_le_bytes()); // PCM
    huge_data.extend_from_slice(&2u16.to_le_bytes()); // 2 ch
    huge_data.extend_from_slice(&48000u32.to_le_bytes());
    huge_data.extend_from_slice(&192000u32.to_le_bytes());
    huge_data.extend_from_slice(&4u16.to_le_bytes());
    huge_data.extend_from_slice(&16u16.to_le_bytes());
    huge_data.extend_from_slice(b"data");
    huge_data.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // 4 GiB!
    assert!(matches!(decode_wav(&huge_data), Err(SerenadeError::Wav(WavError::TruncatedData))));
}

#[test]
fn test_i16_min_extreme_pcm_and_metering() {
    let config = RecorderConfig {
        channels: 1,
        sample_rate: 48000,
        max_samples: 1000,
        vox: None,
    };
    let mut session = CaptureSession::new(config).unwrap();
    session.start().unwrap();

    // i16::MIN is -32768: must not overflow on abs() in debug mode!
    let samples = [i16::MIN, i16::MAX, 0, -1, 1];
    let stats = session.feed_i16(&samples).unwrap();
    assert_eq!(stats.frames_recorded, 5);

    let snap = session.meter().snapshot();
    // Instantaneous peak for i16::MIN (-32768 / 32768.0 = -1.0) is 1.0
    assert!((snap[0].peak - 1.0).abs() < 1e-4);
    assert_eq!(snap[0].peak_dbfs_approx, 0);
}

#[test]
fn test_normalize_peak_pure_silence_and_edge_cases() {
    // Pure silence clip: normalize_peak must return Ok(1.0) without division-by-zero or NaN
    let samples = vec![0.0f32; 100];
    let mut clip = AudioClip::new(1, 48000, samples, Vec::new()).unwrap();
    let gain = clip.normalize_peak(0.95).unwrap();
    assert_eq!(gain, 1.0);
    for &s in &clip.samples {
        assert_eq!(s, 0.0);
        assert!(!s.is_nan());
    }

    // Empty clip
    let mut empty_clip = AudioClip::new(1, 48000, Vec::new(), Vec::new()).unwrap();
    let empty_gain = empty_clip.normalize_peak(1.0).unwrap();
    assert_eq!(empty_gain, 1.0);
}

#[test]
fn test_waveform_query_edge_cases() {
    let mipmap = WaveformMipmap::new(2);
    // target_columns == 0
    assert!(mipmap.query_range(0, 100, 0).is_empty());
    // start >= end
    assert!(mipmap.query_range(100, 50, 10).is_empty());
    assert!(mipmap.query_range(50, 50, 10).is_empty());
    // Empty waveform mipmap query
    assert!(mipmap.query_range(0, 50, 10).is_empty());
}

#[test]
fn test_zero_channels_and_sample_rate_validation() {
    // RecorderConfig validation
    let cfg_zero_ch = RecorderConfig {
        channels: 0,
        sample_rate: 48000,
        max_samples: 1000,
        vox: None,
    };
    assert!(matches!(CaptureSession::new(cfg_zero_ch), Err(SerenadeError::InvalidParameter(_))));

    let cfg_zero_sr = RecorderConfig {
        channels: 1,
        sample_rate: 0,
        max_samples: 1000,
        vox: None,
    };
    assert!(matches!(CaptureSession::new(cfg_zero_sr), Err(SerenadeError::InvalidSampleRate)));

    // AudioClip validation
    assert!(matches!(AudioClip::new(0, 48000, vec![0.0], Vec::new()), Err(SerenadeError::InvalidFormat(_))));
    assert!(matches!(AudioClip::new(1, 0, vec![0.0], Vec::new()), Err(SerenadeError::InvalidSampleRate)));
}
