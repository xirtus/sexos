// SPDX-License-Identifier: GPL-3.0-only
use moan::{lv2_catalog::Lv2Catalog, Studio};
use moan_core::{
    interchange::InterchangeTimeline,
    loudness::DeliveryTarget,
    voice_cleanup::VoicePreset,
    Frame, RATE,
};
use weft::{Response, Strings, Theme, Ui};

fn studio() -> (Ui, Studio) {
    let mut ui = Ui::new(1264, 730, Theme::dark(), Strings::new("en"));
    let s = Studio::build(&mut ui);
    (ui, s)
}

fn synth_voice_with_hum(seconds: usize) -> Vec<Frame> {
    let total = RATE as usize * seconds;
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let t = i as f32 / RATE as f32;
        // 220 Hz speech fundamental + 60 Hz mains hum
        let speech = 0.18 * (2.0 * core::f32::consts::PI * 220.0 * t).sin();
        let hum = 0.08 * (2.0 * core::f32::consts::PI * 60.0 * t).sin();
        let sample = ((speech + hum) * 32767.0).clamp(-32768.0, 32767.0) as i16;
        out.push([sample, sample]);
    }
    out
}

#[test]
fn voice_cleanup_presets_and_undo_work_inside_studio() {
    let (mut ui, mut s) = studio();
    let original = synth_voice_with_hum(2);
    s.session.add(original.clone()).unwrap();
    s.refresh(&mut ui);

    // Cycle preset via command hook
    s.on(&mut ui, &Response::Command("moan.voice.preset".into()))
        .unwrap();
    assert_eq!(s.voice_preset, VoicePreset::BroadcastVoice);

    // Apply 1-click Voice Cleanup via command hook
    s.on(&mut ui, &Response::Command("moan.voice.cleanup".into()))
        .unwrap();
    let cleaned = s.session.selected_region().unwrap().source_frames().to_vec();
    assert_eq!(cleaned.len(), original.len());
    assert_ne!(cleaned, original);

    // Explicit NoisyRoom preset call also works and stays undoable
    s.voice_cleanup(&mut ui, VoicePreset::NoisyRoom).unwrap();
    assert_eq!(s.voice_preset, VoicePreset::NoisyRoom);
    assert!(s.session.undo());
    assert_eq!(
        s.session.selected_region().unwrap().source_frames(),
        cleaned.as_slice()
    );
    assert!(s.session.undo());
    assert_eq!(
        s.session.selected_region().unwrap().source_frames(),
        original.as_slice()
    );
}

#[test]
fn loudness_delivery_targets_normalize_and_update_readout() {
    let (mut ui, mut s) = studio();
    s.session.add(synth_voice_with_hum(2)).unwrap();

    // Measure live loudness first
    s.on(&mut ui, &Response::Command("moan.loudness.measure".into()))
        .unwrap();
    assert!(s.last_loudness.is_some());
    assert!(s.loudness_readout_text().contains("LUFS"));
    assert!(s.loudness_readout_text().contains("dBTP"));

    // Normalize to -14 LUFS Streaming, -16 LUFS Podcast, -23 LUFS Broadcast
    for (cmd, target, expected_lufs) in [
        ("moan.loudness.streaming14", DeliveryTarget::StreamingMinus14, -14.0),
        ("moan.loudness.podcast16", DeliveryTarget::PodcastMinus16, -16.0),
        ("moan.loudness.broadcast23", DeliveryTarget::BroadcastMinus23, -23.0),
    ] {
        s.on(&mut ui, &Response::Command(cmd.into())).unwrap();
        assert_eq!(s.delivery_target, target);
        let m = s.last_loudness.unwrap();
        assert!(
            (m.integrated_lufs - expected_lufs).abs() <= 1.0,
            "expected ~{expected_lufs} LUFS, got {}",
            m.integrated_lufs
        );
        assert!(
            m.true_peak_dbtp <= -0.95,
            "true peak ceiling exceeded: {} dBTP",
            m.true_peak_dbtp
        );
    }

    let report = s
        .normalize_loudness(&mut ui, DeliveryTarget::PodcastMinus16)
        .unwrap();
    assert!((report.after.integrated_lufs - (-16.0)).abs() <= 1.0);
}

#[test]
fn elastic_stretch_and_otio_edl_interchange_export_roundtrip() {
    let (mut ui, mut s) = studio();
    s.session.add(synth_voice_with_hum(1)).unwrap();

    let before_len = s.session.selected_region().unwrap().len();
    // 0.5x tempo => 2x duration
    s.elastic_stretch(&mut ui, 0.5, 0.0, false).unwrap();
    let after_len = s.session.selected_region().unwrap().len();
    assert!(
        after_len > before_len * 3 / 2,
        "expected ~2x stretched length, got {after_len} from {before_len}"
    );

    // Export OTIO and EDL via Studio commands
    s.on(&mut ui, &Response::Command("moan.export.otio".into()))
        .unwrap();
    let otio = s.last_interchange_export.clone().unwrap();
    let parsed_otio = InterchangeTimeline::from_otio_json(&otio).unwrap();
    assert_eq!(parsed_otio.tracks.len(), 1);
    assert_eq!(parsed_otio.tracks[0].clips.len(), 1);
    assert!(parsed_otio.loudness.integrated_lufs.is_some());

    s.on(&mut ui, &Response::Command("moan.export.edl".into()))
        .unwrap();
    let edl = s.last_interchange_export.clone().unwrap();
    let parsed_edl = InterchangeTimeline::from_cmx3600_edl(&edl, RATE, 30, 1).unwrap();
    assert_eq!(parsed_edl.tracks.len(), 1);
    assert_eq!(parsed_edl.tracks[0].clips.len(), 1);
}

#[test]
fn lv2_bundle_manifest_discovery_parses_ports_and_presets() {
    let manifest_ttl = r#"
        @prefix lv2: <http://lv2plug.in/ns/lv2core#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        <https://example.org/lv2/oma-deesser>
            a lv2:Plugin ;
            lv2:binary <oma_deesser.so> ;
            rdfs:seeAlso <oma_deesser.ttl> .
    "#;
    let plugin_ttl = r#"
        @prefix lv2: <http://lv2plug.in/ns/lv2core#> .
        @prefix doap: <http://usefulinc.com/ns/doap#> .
        @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        <https://example.org/lv2/oma-deesser>
            a lv2:Plugin , lv2:DynamicsPlugin ;
            doap:name "Oma De-Esser" ;
            lv2:port [
                a lv2:AudioPort , lv2:InputPort ;
                lv2:index 0 ;
                lv2:symbol "in_l" ;
                lv2:name "In L"
            ] , [
                a lv2:AudioPort , lv2:InputPort ;
                lv2:index 1 ;
                lv2:symbol "in_r" ;
                lv2:name "In R"
            ] , [
                a lv2:AudioPort , lv2:OutputPort ;
                lv2:index 2 ;
                lv2:symbol "out_l" ;
                lv2:name "Out L"
            ] , [
                a lv2:AudioPort , lv2:OutputPort ;
                lv2:index 3 ;
                lv2:symbol "out_r" ;
                lv2:name "Out R"
            ] , [
                a lv2:ControlPort , lv2:InputPort ;
                lv2:index 4 ;
                lv2:symbol "thresh" ;
                lv2:name "Threshold" ;
                lv2:default -24.0 ;
                lv2:minimum -60.0 ;
                lv2:maximum 0.0
            ] .
    "#;

    let (mut ui, mut s) = studio();
    let count = s
        .discover_lv2_bundle(
            &mut ui,
            "/usr/lib/lv2/oma-deesser.lv2",
            manifest_ttl,
            &[plugin_ttl],
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(s.lv2_catalog.plugin_count(), 1);
    assert_eq!(s.lv2_catalog.stereo_plugins().len(), 1);

    let (bundle, info) = s
        .lv2_catalog
        .find_plugin("https://example.org/lv2/oma-deesser")
        .unwrap();
    assert_eq!(bundle, "/usr/lib/lv2/oma-deesser.lv2");
    assert_eq!(info.name, "Oma De-Esser");
    let controls = Lv2Catalog::control_ui(info);
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0].symbol, "thresh");
    assert_eq!(controls[0].default_value, -24.0);
    assert!(s.lv2_catalog.summary_text().contains("Oma De-Esser"));
}
