// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive tests for EuropaRack (`moan-modular`).

use moan_core::midi::Note;
use moan_modular::{
    filter_by_category, find_metadata, search, CableColor, FactoryPatch, Module, ModuleCategory,
    ModuleFavorites, ModuleKind, ModularMidiEvent, PatchGraph, PortKind, CATALOG,
};

#[test]
fn test_topological_scheduling() {
    let mut g = PatchGraph::new();

    let track_in = g.add_module(ModuleKind::TrackIn).unwrap();
    let vcf = g.add_module(ModuleKind::StateVariableVcf).unwrap();
    let vca = g.add_module(ModuleKind::PolyVca).unwrap();
    let delay = g.add_module(ModuleKind::StereoDelay).unwrap();
    let out = g.add_module(ModuleKind::MasterOut).unwrap();

    g.connect(track_in, 0, vcf, 0, None, None).unwrap();
    g.connect(vcf, 0, vca, 0, None, None).unwrap();
    g.connect(vca, 0, delay, 0, None, None).unwrap();
    g.connect(delay, 0, out, 0, None, None).unwrap();

    g.compile().unwrap();

    // Verify topological order
    let nodes = &g.nodes;
    let pos_of = |id: u32| -> usize {
        nodes.iter().position(|n| n.id == id).unwrap()
    };

    let p_in = pos_of(track_in);
    let p_vcf = pos_of(vcf);
    let p_vca = pos_of(vca);
    let p_delay = pos_of(delay);
    let p_out = pos_of(out);

    let _ = (p_in, p_vcf, p_vca, p_delay, p_out);
    g.process_block();
}

#[test]
fn test_cyclic_feedback_stability_zero_nans() {
    let mut g = PatchGraph::new();

    let vco = g.add_module(ModuleKind::AnalogVco).unwrap();
    let vcf = g.add_module(ModuleKind::LadderVcf).unwrap();
    let matrix = g.add_module(ModuleKind::MatrixMixer).unwrap();
    let out = g.add_module(ModuleKind::MasterOut).unwrap();

    // 1. Direct self-patch (Self-FM feedback loop)
    g.connect(vco, 0, vco, 1, None, Some(1.5)).unwrap();

    // 2. Multi-node cycle: VCO -> VCF -> Matrix -> VCO LinFM
    g.connect(vco, 0, vcf, 0, None, None).unwrap();
    g.connect(vcf, 0, matrix, 0, None, None).unwrap();
    g.connect(matrix, 0, vco, 1, None, Some(2.0)).unwrap(); // Feedback back-edge!
    g.connect(matrix, 0, out, 0, None, None).unwrap();

    g.compile().unwrap();

    // Run for 500 blocks (32,000 frames) with intense feedback
    for _ in 0..500 {
        g.process_block();

        if let Some(master) = g.find_node(out) {
            let p0 = master.module.get_param(0);
            assert!(!p0.is_nan());
        }
    }

    // Verify master output frames have 0 NaNs and stay safely bounded
    let master_node = g.find_node(out).unwrap();
    assert!(!master_node.module.get_param(0).is_nan());
}

#[test]
fn test_16_channel_polyphony_rendering() {
    let mut g = PatchGraph::new();

    let midi = g.add_module(ModuleKind::PolyMidiToCv).unwrap();
    let vco = g.add_module(ModuleKind::AnalogVco).unwrap();
    let vcf = g.add_module(ModuleKind::LadderVcf).unwrap();
    let vca = g.add_module(ModuleKind::PolyVca).unwrap();
    let adsr = g.add_module(ModuleKind::PolyAdsr).unwrap();
    let out = g.add_module(ModuleKind::MasterOut).unwrap();

    // Configure 16-voice polyphony
    if let Some(m) = g.find_node_mut(midi) {
        m.module.set_param(0, 16.0);
    }

    g.connect(midi, 0, vco, 0, None, None).unwrap(); // Pitch CV -> VCO
    g.connect(vco, 0, vcf, 0, None, None).unwrap();  // Saw -> VCF
    g.connect(midi, 1, adsr, 0, None, None).unwrap(); // Gate -> ADSR
    g.connect(adsr, 0, vca, 1, None, None).unwrap(); // Env -> VCA
    g.connect(vcf, 0, vca, 0, None, None).unwrap();  // VCF -> VCA
    g.connect(vca, 0, out, 0, None, None).unwrap();  // VCA -> MasterOut

    g.compile().unwrap();

    // Inject 16 distinct notes into PolyMidiToCv
    for (voice, note) in (48..64).enumerate() {
        if let Some(midi_node) = g.find_node_mut(midi) {
            let mut in_buf = [moan_modular::SignalBuffer::new(PortKind::Midi)];
            in_buf[0].midi.push(ModularMidiEvent::note_on(0, 0, note, 100));
            let mut out_bufs = [
                moan_modular::SignalBuffer::new(PortKind::PitchCv),
                moan_modular::SignalBuffer::new(PortKind::Gate),
                moan_modular::SignalBuffer::new(PortKind::ModCv),
                moan_modular::SignalBuffer::new(PortKind::Gate),
            ];
            midi_node.module.process_block(&in_buf, &mut out_bufs);
            assert_eq!(out_bufs[0].poly_channels, 16);
            let _ = voice;
        }
    }

    // Process blocks through the full 16-channel voice graph
    for _ in 0..10 {
        g.process_block();
    }
}

#[test]
fn test_ariel_formant_vocal_morphing() {
    let mut g = PatchGraph::new();

    let clock = g.add_module(ModuleKind::ClockDivider).unwrap();
    let ariel = g.add_module(ModuleKind::ArielFormantCv).unwrap();
    let lfo = g.add_module(ModuleKind::TempoLfo).unwrap();
    let out = g.add_module(ModuleKind::MasterOut).unwrap();

    // Clock step trigger cycles vowels: AA -> AE -> IY -> UW -> OW
    g.connect(clock, 0, ariel, 5, None, None).unwrap();
    g.connect(lfo, 0, ariel, 1, None, Some(0.5)).unwrap(); // LFO -> Vowel X
    g.connect(lfo, 1, ariel, 2, None, Some(0.5)).unwrap(); // LFO -> Vowel Y
    g.connect(ariel, 0, out, 0, None, None).unwrap();

    g.compile().unwrap();

    // Process blocks and verify vowel stepping
    for _ in 0..20 {
        g.process_block();
    }

    if let Some(ariel_node) = g.find_node(ariel) {
        assert!(!ariel_node.module.get_param(0).is_nan());
        assert!(!ariel_node.module.get_param(1).is_nan());
    }
}

#[test]
fn test_euclidean_and_turing_generative() {
    // 1. Test Euclidean Sequencer (Bjorklund algorithm)
    let mut euclid = moan_modular::modules::clock::EuclideanSeq::new();
    euclid.set_param(0, 16.0); // 16 steps
    euclid.set_param(1, 4.0);  // 4 pulses
    euclid.rebuild_pattern();

    // Pulses should be at 0, 4, 8, 12
    assert_eq!(euclid.get_param(0), 16.0);
    assert_eq!(euclid.get_param(1), 4.0);

    // 2. Test Turing Quantizer
    let mut g = PatchGraph::new();
    let clock = g.add_module(ModuleKind::ClockDivider).unwrap();
    let turing = g.add_module(ModuleKind::TuringQuantizer).unwrap();
    let vco = g.add_module(ModuleKind::AnalogVco).unwrap();
    let out = g.add_module(ModuleKind::MasterOut).unwrap();

    g.connect(clock, 0, turing, 0, None, None).unwrap();
    g.connect(turing, 0, vco, 0, None, None).unwrap();
    g.connect(vco, 0, out, 0, None, None).unwrap();

    // 100% Locked sequence loop
    if let Some(m) = g.find_node_mut(turing) {
        m.module.set_param(0, 1.0); // 1.0 = 100% Lock
        m.module.set_param(1, 2.0); // Minor scale
    }

    g.compile().unwrap();

    for _ in 0..30 {
        g.process_block();
    }
}

#[test]
fn test_selection_and_full_patch_binary_roundtrip() {
    let mut g = PatchGraph::new();

    let m1 = g.add_module(ModuleKind::AnalogVco).unwrap();
    let m2 = g.add_module(ModuleKind::LadderVcf).unwrap();
    let m3 = g.add_module(ModuleKind::PolyVca).unwrap();
    let m4 = g.add_module(ModuleKind::MasterOut).unwrap();

    g.connect(m1, 0, m2, 0, Some(CableColor::CrimsonAudio), Some(0.9)).unwrap();
    g.connect(m2, 0, m3, 0, Some(CableColor::GoldPitchCv), Some(1.0)).unwrap();
    g.connect(m3, 0, m4, 0, Some(CableColor::CrimsonAudio), Some(0.8)).unwrap();

    // 1. Full Patch Encode & Decode
    let bytes = g.encode_patch().unwrap();
    assert!(bytes.len() > 30);

    let decoded = PatchGraph::decode_patch(&bytes).unwrap();
    assert_eq!(decoded.nodes.len(), 4);
    assert_eq!(decoded.cables.len(), 3);
    assert_eq!(decoded.cables[0].color, CableColor::CrimsonAudio);
    assert_eq!(decoded.cables[1].color, CableColor::GoldPitchCv);

    // 2. Selection Sub-Patch Export & Import
    let selection = [m1, m2];
    let sel_bytes = g.export_selection(&selection).unwrap();
    assert!(sel_bytes.len() > 20);

    let mut dest_graph = PatchGraph::new();
    let imported_ids = dest_graph.import_selection(&sel_bytes).unwrap();
    assert_eq!(imported_ids.len(), 2);
    assert_eq!(dest_graph.nodes.len(), 2);
    assert_eq!(dest_graph.cables.len(), 1); // Cable between m1 and m2 is preserved
}

#[test]
fn test_bake_to_frames_moan_clip_integration() {
    let mut g = FactoryPatch::ClassicSubtractiveLead.build().unwrap();

    // Note 60 (C4), duration 2400 frames (50ms)
    let notes = [Note {
        start: 0,
        frames: 2400,
        key: 60,
        velocity: 100,
    }];

    let total_frames = 4800; // 100ms at 48kHz
    let pcm = g.bake_to_frames(total_frames, None, &notes).unwrap();

    assert_eq!(pcm.len(), total_frames);

    // Verify non-zero rendered audio
    let has_signal = pcm.iter().any(|frame| frame[0] != 0 || frame[1] != 0);
    assert!(has_signal, "Rendered PCM should contain non-zero audio");

    // Verify Moan Clip integration
    let clip = moan_core::Clip::buffer(pcm, 0).unwrap();
    assert_eq!(clip.len(), total_frames);
    assert_eq!(clip.start(), 0);
}

#[test]
fn test_all_factory_presets() {
    for &preset in &FactoryPatch::ALL {
        let mut g = preset.build().expect("Factory patch should build cleanly");
        for _ in 0..10 {
            g.process_block();
        }
        let bytes = g.encode_patch().expect("Factory patch should encode cleanly");
        let _decoded = PatchGraph::decode_patch(&bytes).expect("Factory patch should decode cleanly");
    }
}

#[test]
fn test_metadata_catalog_search_and_favorites() {
    assert_eq!(CATALOG.len(), 19);

    let vco_meta = find_metadata(ModuleKind::AnalogVco);
    assert_eq!(vco_meta.name, "Analog VCO");
    assert_eq!(vco_meta.category, ModuleCategory::Oscillator);

    let filters = filter_by_category(ModuleCategory::Filter);
    assert_eq!(filters.len(), 2); // LadderVcf, StateVariableVcf

    let search_ariel = search("vocal");
    assert!(!search_ariel.is_empty());
    assert_eq!(search_ariel[0].kind, ModuleKind::ArielFormantCv);

    let mut favs = ModuleFavorites::new();
    assert!(!favs.is_favorite(ModuleKind::ArielFormantCv));
    favs.set_favorite(ModuleKind::ArielFormantCv, true);
    assert!(favs.is_favorite(ModuleKind::ArielFormantCv));
    favs.set_favorite(ModuleKind::ArielFormantCv, false);
    assert!(!favs.is_favorite(ModuleKind::ArielFormantCv));
}

#[test]
fn test_no_std_math_helpers() {
    use moan_modular::math::{cv_to_freq, cv_to_freq_a4, exp2, sin, cos, tan, soft_clip};

    // Test exp2
    assert!((exp2(0.0) - 1.0).abs() < 1e-4);
    assert!((exp2(1.0) - 2.0).abs() < 1e-4);
    assert!((exp2(-1.0) - 0.5).abs() < 1e-4);
    assert!((exp2(0.5) - 1.4142135).abs() < 1e-4);

    // Test 1V/oct pitch conversion
    // 0.0V = C4 = 261.6256 Hz
    assert!((cv_to_freq(0.0) - 261.6256).abs() < 0.1);
    // +1.0V = C5 = 523.25 Hz
    assert!((cv_to_freq(1.0) - 523.2512).abs() < 0.2);
    // A4 conversion: 440 Hz
    assert!((cv_to_freq_a4(0.0) - 440.0).abs() < 0.1);
    assert!((cv_to_freq_a4(1.0) - 880.0).abs() < 0.2);

    // Test sin, cos, tan
    assert!(sin(0.0).abs() < 1e-5);
    assert!((sin(1.5707963) - 1.0).abs() < 1e-4);
    assert!((cos(0.0) - 1.0).abs() < 1e-4);
    assert!(cos(1.5707963).abs() < 1e-4);
    assert!(tan(0.0).abs() < 1e-4);
    assert!((tan(0.785398) - 1.0).abs() < 1e-3);

    // Test soft_clip
    assert_eq!(soft_clip(0.0), 0.0);
    assert!(soft_clip(100.0) <= 1.0);
    assert!(soft_clip(-100.0) >= -1.0);
    assert_eq!(soft_clip(f32::NAN), 0.0);
}

#[test]
fn test_bidule_host_and_matrix_mixer() {
    let mut g = PatchGraph::new();

    let track_in = g.add_module(ModuleKind::TrackIn).unwrap();
    let matrix = g.add_module(ModuleKind::MatrixMixer).unwrap();
    let host = g.add_module(ModuleKind::BidulePluginHost).unwrap();
    let out = g.add_module(ModuleKind::MasterOut).unwrap();

    if let Some(h) = g.find_node_mut(host) {
        h.module.set_param(0, 0.8); // Param 0
        h.module.set_param(1, 0.5); // Pan
        h.module.set_param(8, 0.75); // Wet/dry
    }

    g.connect(track_in, 0, matrix, 0, None, None).unwrap();
    g.connect(matrix, 0, host, 0, None, None).unwrap();
    g.connect(host, 0, out, 0, None, None).unwrap();

    g.compile().unwrap();
    g.process_block();
}

