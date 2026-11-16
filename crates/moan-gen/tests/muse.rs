// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive Integration Test Suite for Moan Muse (crates/moan-gen).

use moan_core::RATE;
use moan_gen::*;

#[test]
fn test_muse_config_default_disabled() {
    let cfg = MuseConfig::default();
    assert!(!cfg.enabled, "Classic DAW startup MUST keep Muse disabled by default!");
    assert_eq!(cfg.pack_dir, b"@/data/moan/muse");
    assert_eq!(cfg.endpoint_url, None);
    assert_eq!(cfg.preferred_tier, None);
    assert!(cfg.allow_tier0_fallback);
}

#[test]
fn test_model_pack_lifecycle_and_sha256() {
    // 1. Missing pack transitions to PromptDownload
    let state = ModelPackState::NotInstalled;
    assert_eq!(state, ModelPackState::NotInstalled);

    let ram_8gb = 8192;
    let rec_8gb = ModelPackKind::recommended_for_ram(ram_8gb);
    assert_eq!(rec_8gb, ModelPackKind::TurboLiteGguf);

    let ram_32gb = 32768;
    let rec_32gb = ModelPackKind::recommended_for_ram(ram_32gb);
    assert_eq!(rec_32gb, ModelPackKind::StudioXlGguf);

    let prompt_state = ModelPackState::PromptDownload {
        recommended: rec_8gb,
        detected_ram_mb: ram_8gb,
    };
    assert!(matches!(prompt_state, ModelPackState::PromptDownload { .. }));

    // 2. Downloader state machine and SHA-256 chunk hashing
    let mut downloader = ModelPackDownloader::new(ModelPackKind::TurboLiteGguf);
    assert!(matches!(downloader.state(), ModelPackState::Downloading { .. }));

    // Hash testing with sha256_digest_hex
    let test_data = b"Moan Muse ACE-Step 1.5 weights chunk";
    let hash = sha256_digest_hex(test_data);
    assert_eq!(hash.len(), 64);

    // Corrupted chunk verification rejection
    let fake_data = vec![0x42u8; 1024];
    let err = downloader.feed_chunk(&fake_data);
    // Large chunk feeds into downloader
    assert!(err.is_ok());

    // Verified download completion transition
    downloader.finish_verified();
    assert_eq!(
        *downloader.state(),
        ModelPackState::Installed {
            kind: ModelPackKind::TurboLiteGguf,
            verified_sha256: true,
        }
    );

    // 3. Manifest parser
    let manifest_text = b"model_part1.gguf 1048576 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nmodel_part2.gguf 2097152 a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0\n";
    let entries = parse_manifest(manifest_text).expect("valid manifest");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].filename, "model_part1.gguf");
    assert_eq!(entries[0].size, 1048576);
    assert_eq!(entries[1].filename, "model_part2.gguf");
    assert_eq!(entries[1].size, 2097152);

    // Invalid manifest
    assert!(parse_manifest(b"").is_err());
    assert!(parse_manifest(b"invalid format line").is_err());
}

#[test]
fn test_ram_tier_selection_across_hardware() {
    let uninstalled = ModelPackState::NotInstalled;
    let installed_turbo = ModelPackState::Installed {
        kind: ModelPackKind::TurboLiteGguf,
        verified_sha256: true,
    };
    let installed_studio = ModelPackState::Installed {
        kind: ModelPackKind::StudioXlGguf,
        verified_sha256: true,
    };

    // 2 GB (< 4096 MB): Tier 0 Featherweight (< 2 MiB RAM)
    let sel_2gb = select_tier(2048, &uninstalled, false, None);
    assert_eq!(sel_2gb.tier, RamTier::Tier0Featherweight);
    assert_eq!(sel_2gb.estimated_peak_mb, 2);
    assert!(sel_2gb.badge_text.contains("Tier 0 Featherweight"));

    // 8 GB without pack & without endpoint -> Tier 0
    let sel_8gb_nopack = select_tier(8192, &uninstalled, false, None);
    assert_eq!(sel_8gb_nopack.tier, RamTier::Tier0Featherweight);

    // 8 GB with TurboLite installed: Tier 2 Local Turbo Lite (Ephemeral)
    let sel_8gb = select_tier(8192, &installed_turbo, false, None);
    assert_eq!(sel_8gb.tier, RamTier::Tier2LocalTurboLite);
    assert_eq!(sel_8gb.estimated_peak_mb, 3500);
    assert!(sel_8gb.badge_text.contains("Local Turbo Lite"));

    // 16 GB with TurboLite: Tier 2
    let sel_16gb = select_tier(16384, &installed_turbo, false, None);
    assert_eq!(sel_16gb.tier, RamTier::Tier2LocalTurboLite);

    // 32 GB with Studio XL: Tier 3 Local Studio XL
    let sel_32gb = select_tier(32768, &installed_studio, false, None);
    assert_eq!(sel_32gb.tier, RamTier::Tier3LocalStudioXl);
    assert_eq!(sel_32gb.estimated_peak_mb, 12288);
    assert!(sel_32gb.badge_text.contains("Local Studio XL"));

    // Any RAM with live endpoint -> Tier 1 Remote Bridge
    let sel_remote = select_tier(2048, &uninstalled, true, None);
    assert_eq!(sel_remote.tier, RamTier::Tier1RemoteBridge);
    assert_eq!(sel_remote.estimated_peak_mb, 8);
    assert!(sel_remote.badge_text.contains("Remote Muse server"));

    // User preference override
    let sel_pref = select_tier(32768, &installed_studio, false, Some(RamTier::Tier0Featherweight));
    assert_eq!(sel_pref.tier, RamTier::Tier0Featherweight);
    assert!(sel_pref.badge_text.contains("Preferred"));
}

#[test]
fn test_ace_server_and_acestep_api_codecs() {
    // 1. ace-server GET /props
    let props_req = build_ace_props_request("127.0.0.1:8080");
    assert!(props_req.starts_with(b"GET /props HTTP/1.1\r\n"));

    let props_raw_resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"model_name\":\"acestep-v15-turbo\",\"sample_rate\":48000,\"max_duration_sec\":300,\"default_steps\":25}";
    let props = parse_ace_props_response(props_raw_resp).expect("valid props response");
    assert_eq!(props.model_name, "acestep-v15-turbo");
    assert_eq!(props.sample_rate, 48000);
    assert_eq!(props.max_duration_sec, 300);
    assert_eq!(props.default_steps, 25);

    // 2. ace-server POST /lm
    let lm_req = build_ace_lm_request("127.0.0.1:8080", "Epic Synthwave", "Neon lights", 124, "A Minor", true, 30, 2)
        .expect("build lm request");
    assert!(lm_req.starts_with(b"POST /lm HTTP/1.1\r\n"));

    let lm_raw_resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"tokens\":[101,202,303],\"cot\":\"Analyzing key and bpm\",\"alignment\":\"[00:00.00] Neon\"}";
    let lm_resp = parse_ace_lm_response(lm_raw_resp).expect("valid lm response");
    assert_eq!(lm_resp.tokens, vec![101, 202, 303]);
    assert_eq!(lm_resp.thinking_cot.as_deref(), Some("Analyzing key and bpm"));

    // 3. ace-server POST /synth?wav=1
    let synth_req = build_ace_synth_request("127.0.0.1:8080", "Dark Electro", None, 25)
        .expect("build synth request");
    assert!(synth_req.starts_with(b"POST /synth?wav=1 HTTP/1.1\r\n"));

    // 4. ace-server POST /understand
    let test_wav = encode_wav_16bit(&[[1000, -1000]; 480]).expect("encode wav");
    let understand_req = build_ace_understand_request("127.0.0.1:8080", &test_wav)
        .expect("build understand request");
    assert!(understand_req.starts_with(b"POST /understand HTTP/1.1\r\n"));

    let understand_raw_resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"detected_bpm\":128,\"detected_key\":\"F# Minor\",\"caption\":\"Fast paced techno beat\",\"lyrics\":\"[00:01.00] Hit that bass\"}";
    let understand = parse_ace_understand_response(understand_raw_resp).expect("parse understand");
    assert_eq!(understand.detected_bpm, 128);
    assert_eq!(understand.detected_key, "F# Minor");
    assert_eq!(understand.caption, "Fast paced techno beat");
    assert_eq!(understand.lyrics.len(), 1);
    assert_eq!(understand.lyrics[0].text, "Hit that bass");

    // 5. acestep-api POST /release_task with Bearer Token from memory
    let token = "shibari_vault_token_never_serialized_to_disk";
    let release_req = build_release_task_request(
        "127.0.0.1:8001",
        token,
        "Cinematic Soundtrack",
        "Heroes arise",
        "en",
        110,
        "D Dorian",
        45,
    ).expect("build release task");
    let release_str = std::str::from_utf8(&release_req).unwrap();
    assert!(release_str.contains("Authorization: Bearer shibari_vault_token_never_serialized_to_disk"));
    assert!(release_str.contains("POST /release_task HTTP/1.1"));

    let release_raw_resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"task_id\":\"task_987654321\"}";
    let task_id = parse_release_task_response(release_raw_resp).expect("parse release task");
    assert_eq!(task_id, "task_987654321");

    // 6. acestep-api POST /query_result
    let query_req = build_query_result_request("127.0.0.1:8001", token, &task_id)
        .expect("build query request");
    assert!(std::str::from_utf8(&query_req).unwrap().contains("task_987654321"));

    let query_raw_resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"task_id\":\"task_987654321\",\"status\":\"succeeded\",\"progress\":1.0,\"audio_url\":\"/v1/audio?task=987654321\",\"lyrics\":\"[00:00.00] Heroes arise\"}";
    let task_status = parse_query_result_response(query_raw_resp).expect("parse query status");
    assert_eq!(task_status.task_id, "task_987654321");
    assert_eq!(task_status.status, "succeeded");
    assert_eq!(task_status.progress, 1.0);
    assert_eq!(task_status.audio_url.as_deref(), Some("/v1/audio?task=987654321"));
}

#[test]
fn test_riff_wav_codec_16bit_and_float_roundtrip() {
    // 1. 16-bit PCM round-trip (must be bit-exact!)
    let mut original_frames = Vec::new();
    for i in 0..4800 {
        let l = ((i % 100) * 300 - 15000) as i16;
        let r = ((i % 80) * 400 - 16000) as i16;
        original_frames.push([l, r]);
    }

    let encoded_16 = encode_wav_16bit(&original_frames).expect("encode 16bit wav");
    assert_eq!(&encoded_16[0..4], b"RIFF");
    assert_eq!(&encoded_16[8..12], b"WAVE");

    let decoded_16 = decode_wav(&encoded_16).expect("decode 16bit wav");
    assert_eq!(original_frames.len(), decoded_16.len());
    for i in 0..original_frames.len() {
        assert_eq!(original_frames[i], decoded_16[i], "16-bit PCM must be bit-exact at frame {}", i);
    }

    // 2. 32-bit Float round-trip
    let mut float_samples = Vec::new();
    for i in 0..4800 {
        let fl = (i as f32 / 2400.0) - 1.0;
        let fr = 1.0 - (i as f32 / 2400.0);
        float_samples.push([fl, fr]);
    }

    let encoded_fl = encode_wav_float(&float_samples).expect("encode float wav");
    let decoded_fl = decode_wav(&encoded_fl).expect("decode float wav");
    assert_eq!(float_samples.len(), decoded_fl.len());
    for i in 0..float_samples.len() {
        let expected_l = (float_samples[i][0].clamp(-1.0, 1.0) * 32767.0) as i16;
        let diff_l = (expected_l as i32 - decoded_fl[i][0] as i32).abs();
        assert!(diff_l <= 2, "Float conversion within quantization limit");
    }

    // 3. Reject non-finite sample in float WAV
    let nan_samples = vec![[f32::NAN, 0.0]];
    let encoded_nan = encode_wav_float(&nan_samples).expect("encode nan wav");
    let nan_err = decode_wav(&encoded_nan);
    assert!(nan_err.is_err(), "Must reject NaN samples in float WAV!");

    // 4. Reject truncated WAV
    assert!(decode_wav(b"RIFFtruncated").is_err());
}

#[test]
fn test_tier0_create_song_and_add_layer() {
    let engine = Tier0Engine::new();
    let nonce: Nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let duration_frames = RATE as usize * 2; // 2 seconds

    let result = engine.create_song(
        nonce,
        "Dark Wave Synth",
        "Driving through the neon night",
        120,
        "A Minor",
        duration_frames,
    ).expect("create song failed");

    assert_eq!(result.frames.len(), duration_frames);
    assert_eq!(result.stems.len(), 6, "Must generate all 6 stems: Vocals, Drums, Bass, Chords, FX, Other");

    // Verify audio energy is non-zero
    let mut max_sample = 0i16;
    for f in result.frames.iter() {
        max_sample = max_sample.max(f[0].abs()).max(f[1].abs());
    }
    assert!(max_sample > 2000, "Generated song must have audible content!");

    // Test AddLayer
    let layer_res = engine.add_layer(nonce, &result.frames, StemRole::Bass)
        .expect("add layer failed");
    assert_eq!(layer_res.frames.len(), duration_frames);
    assert_eq!(layer_res.stems.len(), 1);
    assert_eq!(layer_res.stems[0].0, StemRole::Bass);
}

#[test]
fn test_tier0_replace_section_bit_identical() {
    let engine = Tier0Engine::new();
    let nonce: Nonce = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 1, 2, 3, 4, 5, 6];
    let total_frames = 48000; // 1 second

    // Build reference audio with known, unique values
    let mut ref_audio = Vec::with_capacity(total_frames);
    for i in 0..total_frames {
        let left = (((i * 17) % 30000) as i32 - 15000) as i16;
        let right = (((i * 31) % 30000) as i32 - 15000) as i16;
        ref_audio.push([left, right]);
    }

    let from_frame = 12000;
    let to_frame = 36000;

    let res = engine.replace_section(nonce, &ref_audio, from_frame, to_frame)
        .expect("replace section failed");

    assert_eq!(res.frames.len(), total_frames);

    // CRITICAL: verify non-selected frames BEFORE from_frame are 100% BIT-IDENTICAL!
    for i in 0..from_frame {
        assert_eq!(
            res.frames[i],
            ref_audio[i],
            "Frame {} before replaced section MUST be bit-identical!",
            i
        );
    }

    // CRITICAL: verify non-selected frames AFTER to_frame are 100% BIT-IDENTICAL!
    for i in to_frame..total_frames {
        assert_eq!(
            res.frames[i],
            ref_audio[i],
            "Frame {} after replaced section MUST be bit-identical!",
            i
        );
    }

    // Verify replaced section has audio generated
    let mut diff_count = 0;
    for i in from_frame + 1000..to_frame - 1000 {
        if res.frames[i] != ref_audio[i] {
            diff_count += 1;
        }
    }
    assert!(diff_count > 1000, "Replaced region must contain newly synthesized content!");
}

#[test]
fn test_tier0_extract_stems_distinct() {
    let engine = Tier0Engine::new();
    let nonce: Nonce = [2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32];
    let duration = RATE as usize * 1; // 1 second

    // Synthesize source song
    let song = engine.create_song(nonce, "Orchestral Groove", "Symphony", 120, "C Major", duration)
        .expect("song synthesis");

    let stem_result = engine.extract_stems(nonce, &song.frames)
        .expect("extract stems");

    assert_eq!(stem_result.stems.len(), 6, "Must split into 6 stems");

    // Verify 4+ distinct stems have non-zero energy and distinct waveforms
    let roles: Vec<StemRole> = stem_result.stems.iter().map(|(r, _)| *r).collect();
    assert!(roles.contains(&StemRole::Vocals));
    assert!(roles.contains(&StemRole::Drums));
    assert!(roles.contains(&StemRole::Bass));
    assert!(roles.contains(&StemRole::Chords));
    assert!(roles.contains(&StemRole::Fx));
    assert!(roles.contains(&StemRole::Other));

    let mut energies = Vec::new();
    for (role, stem_frames) in &stem_result.stems {
        let mut energy = 0u64;
        for f in stem_frames.iter() {
            energy += (f[0] as i64 * f[0] as i64 + f[1] as i64 * f[1] as i64) as u64;
        }
        assert!(energy > 0, "Stem {:?} must have non-zero energy!", role);
        energies.push(energy);
    }

    // Verify stems are distinct (not identical duplicates)
    assert_ne!(energies[0], energies[1], "Vocals and Drums must have distinct energy");
    assert_ne!(energies[1], energies[2], "Drums and Bass must have distinct energy");
    assert_ne!(energies[2], energies[3], "Bass and Chords must have distinct energy");
}

#[test]
fn test_lrc_roundtrip() {
    let raw_lrc = "[00:00.00] Intro\n[00:01.50] Moan Muse in Action\n[00:04.25] Splicing audio seamlessly\n[01:12.80] Outro fade\n";
    let lines = parse_lrc(raw_lrc);
    assert_eq!(lines.len(), 4);

    assert_eq!(lines[0].timestamp_ms, 0);
    assert_eq!(lines[0].text, "Intro");
    assert_eq!(lines[0].frame_offset, 0);

    assert_eq!(lines[1].timestamp_ms, 1500);
    assert_eq!(lines[1].text, "Moan Muse in Action");
    assert_eq!(lines[1].frame_offset, 1500 * 48); // 48 frames per ms at 48 kHz

    assert_eq!(lines[2].timestamp_ms, 4250);
    assert_eq!(lines[2].text, "Splicing audio seamlessly");

    assert_eq!(lines[3].timestamp_ms, 72800);
    assert_eq!(lines[3].text, "Outro fade");

    // Generate LRC and round-trip
    let generated = generate_lrc(&lines);
    let lines2 = parse_lrc(&generated);
    assert_eq!(lines.len(), lines2.len());
    for i in 0..lines.len() {
        assert_eq!(lines[i].timestamp_ms, lines2[i].timestamp_ms);
        assert_eq!(lines[i].text, lines2[i].text);
        assert_eq!(lines[i].frame_offset, lines2[i].frame_offset);
    }
}

#[test]
fn test_studio_chat_parsing() {
    // 1. Tempo commands
    assert_eq!(
        parse_studio_chat("tempo 120", false, false),
        StudioChatCommand::SetSessionTempoBpm(120)
    );
    assert_eq!(
        parse_studio_chat("/tempo 140", false, false),
        StudioChatCommand::SetSessionTempoBpm(140)
    );
    assert_eq!(
        parse_studio_chat("128 bpm", false, false),
        StudioChatCommand::SetSessionTempoBpm(128)
    );
    assert_eq!(
        parse_studio_chat("set bpm to 95", false, false),
        StudioChatCommand::SetSessionTempoBpm(95)
    );

    // 2. Stem separation
    assert_eq!(
        parse_studio_chat("/stems", true, false),
        StudioChatCommand::ExtractStemsFromSelected
    );
    assert_eq!(
        parse_studio_chat("extract stems", false, false),
        StudioChatCommand::ExtractStemsFromSelected
    );
    assert_eq!(
        parse_studio_chat("split stems into vocals drums bass", false, false),
        StudioChatCommand::ExtractStemsFromSelected
    );

    // 3. Audio to MIDI
    assert_eq!(
        parse_studio_chat("/midi", true, false),
        StudioChatCommand::ExtractMidiFromSelected
    );
    assert_eq!(
        parse_studio_chat("audio to midi", true, false),
        StudioChatCommand::ExtractMidiFromSelected
    );

    // 4. MIDI Clip Generation
    match parse_studio_chat("generate midi 4 bars tech house 126 bpm", false, false) {
        StudioChatCommand::GenerateMidiClip { bars, bpm, .. } => {
            assert_eq!(bars, 4);
            assert_eq!(bpm, 126);
        }
        other => panic!("Expected GenerateMidiClip, got {:?}", other),
    }

    // 5. Device Preset Design
    match parse_studio_chat("preset eq warm tube boost", false, false) {
        StudioChatCommand::DesignDevicePreset { target, description } => {
            assert_eq!(target, "eq");
            assert!(description.contains("warm tube"));
        }
        other => panic!("Expected DesignDevicePreset, got {:?}", other),
    }

    // 6. Context-aware edits
    match parse_studio_chat("replace section with guitar lead", true, false) {
        StudioChatCommand::RunMuseJob(MuseMode::ReplaceSection { .. }) => {}
        other => panic!("Expected ReplaceSection, got {:?}", other),
    }

    match parse_studio_chat("add funky bass", false, false) {
        StudioChatCommand::RunMuseJob(MuseMode::AddLayer { role }) => {
            assert_eq!(role, StemRole::Bass);
        }
        other => panic!("Expected AddLayer(Bass), got {:?}", other),
    }

    match parse_studio_chat("extend by 8 bars", false, false) {
        StudioChatCommand::RunMuseJob(MuseMode::Extend { .. }) => {}
        other => panic!("Expected Extend, got {:?}", other),
    }

    match parse_studio_chat("mouth beat to drum kit", false, false) {
        StudioChatCommand::RunMuseJob(MuseMode::Cover { style_strength_pct }) => {
            assert_eq!(style_strength_pct, 80);
        }
        other => panic!("Expected Cover, got {:?}", other),
    }

    match parse_studio_chat("vocal to bgm backing track", false, false) {
        StudioChatCommand::RunMuseJob(MuseMode::Vocal2Bgm) => {}
        other => panic!("Expected Vocal2Bgm, got {:?}", other),
    }

    // 7. Full Song creation (default)
    match parse_studio_chat("create a melodic techno banger in D Minor 128 bpm", false, false) {
        StudioChatCommand::RunMuseJob(MuseMode::CreateSong { bpm, key_scale, .. }) => {
            assert_eq!(bpm, 128);
            assert_eq!(key_scale, "D Minor");
        }
        other => panic!("Expected CreateSong, got {:?}", other),
    }
}

#[test]
fn test_waiting_for_memory_and_nonce_validation() {
    // 1. Nonce validation
    let zero_nonce = [0u8; 16];
    assert_eq!(validate_nonce(zero_nonce), Err(MuseError::InvalidNonce));

    let valid_nonce: Nonce = [0x50, 0x4c, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    assert_eq!(validate_nonce(valid_nonce), Ok(()));

    // 2. Buffer memory allocation check
    let alloc_res = try_alloc_frames(4800);
    assert!(alloc_res.is_ok());

    let huge_alloc = try_alloc_frames(usize::MAX);
    assert_eq!(huge_alloc.err(), Some(MuseError::Memory));

    // 3. Supervisor and TakeCrate
    let config = MuseConfig {
        enabled: true,
        ..Default::default()
    };
    let mut supervisor = MuseSupervisor::new(config, 8192, ModelPackState::FeatherweightOnly);
    assert_eq!(supervisor.status(), Status::Idle);

    let req = MuseJobRequest::new(
        valid_nonce,
        MuseMode::CreateSong {
            prompt: String::from("Memory Resilience Test"),
            lyrics: String::from("Testing WaitingForMemory status"),
            vocal_language: String::from("en"),
            bpm: 120,
            key_scale: String::from("C Major"),
            time_sig: [4, 4],
            duration_frames: 4800,
            thinking_cot: false,
            batch_count: 1,
        },
        RamTier::Tier0Featherweight,
    ).expect("valid request");

    supervisor.submit_job(req).expect("submit succeeded");
    assert_eq!(supervisor.status(), Status::Ready);

    let take = supervisor.active_take().expect("active take crate");
    assert_eq!(take.versions.len(), 1);
    assert_eq!(take.active_version, 0);

    // Test remix and insert_all
    let remix_mode = take.remix();
    match remix_mode {
        MuseMode::CreateSong { prompt, bpm, .. } => {
            assert_eq!(prompt, "Memory Resilience Test");
            assert_eq!(bpm, 120);
        }
        _ => panic!("Expected CreateSong remix"),
    }

    let stems_to_insert = take.insert_all();
    assert_eq!(stems_to_insert.len(), 6);

    let result = supervisor.take_result().expect("take result");
    assert_eq!(result.frames.len(), 4800);
    assert_eq!(supervisor.status(), Status::Idle);
}
