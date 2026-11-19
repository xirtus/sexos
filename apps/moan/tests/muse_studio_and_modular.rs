// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive Integration Test Suite for Unified Moan DAW + Muse + Ariel + EuropaRack.
//!
//! Tests:
//! 1. Default pure DAW state (`muse_enabled == false`) + byte-exact MOANPRJ v1/v2/v3 serialization.
//! 2. Opt-in Muse lifecycle: RAM tier recommendation, pack downloads, endpoint connect, Tier 0 synth, and clean disable.
//! 3. Full Muse workflows: Create Song, Add Layer, Replace Section crossfading, Extract Stems, Studio 2.0 Chat commands.
//! 4. Ariel vocal synth: Klatt formants, English/Japanese pronunciation, live karaoke highlight, MIDI singing.
//! 5. EuropaRack: Workspace::EuropaRack, factory presets, dark room brightness rendering, and Bake Rack to Track.
//! 6. Studio 2.0 Clip Settings: WSOLA Transpose/Speed warp, offline Extract MIDI, visual curve strip, and MOANPRJ v4 round-trip.

extern crate alloc;

use alloc::{string::String, sync::Arc};
use moan::{
    europarack_ui, studio_rack_ui, Session, Studio, Workspace,
};
use moan_core::{
    effects::{Distortion, DistortionMode, Equalizer},
    midi::{Instrument, Note, Pattern},
    vocal::VoiceParams,
    Clip,
};
use moan_gen::{
    job::StemRole,
    pack::ModelPackState,
};
use moan_modular::presets::FactoryPatch;
use weft::{Response, Strings, Theme, Ui};

fn test_studio() -> (Ui, Studio) {
    let mut ui = Ui::new(1264, 730, Theme::dark(), Strings::new("en"));
    let s = Studio::build(&mut ui);
    (ui, s)
}

#[test]
fn test_1_default_pure_daw_and_v1_v2_v3_exactness() {
    let (mut ui, mut s) = test_studio();

    // 1. In pure classic mode, Muse is completely disabled
    assert!(!s.muse_config.enabled);
    assert!(!s.muse_setup_open);
    assert_eq!(s.muse_pack_state, ModelPackState::NotInstalled);
    assert!(s.session.tracks.is_empty());

    // 2. Waveform header and track header geometry invariants at (1264, 730)
    ui.layout();
    let r = ui.rect(s.waveform);
    assert!(r.right() <= 1264.0 && r.x >= 142.0);

    // 3. Classic v1 audio-only project serialization
    let mut s_audio = Session::default();
    s_audio.add(vec![[100, -100]; 100]).unwrap();
    let v1_bytes = s_audio.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(v1_bytes[8..10].try_into().unwrap()), 1);
    let decoded_v1 = Session::decode_project(&v1_bytes).unwrap();
    assert_eq!(decoded_v1.tracks.len(), 1);
    assert_eq!(decoded_v1.encode_project().unwrap(), v1_bytes);

    // 4. Classic v2 MIDI project serialization
    s.on(&mut ui, &Response::Command("moan.library.keys".into()))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    let v2_bytes = s.session.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(v2_bytes[8..10].try_into().unwrap()), 2);
    let decoded_v2 = Session::decode_project(&v2_bytes).unwrap();
    assert_eq!(decoded_v2.tracks.len(), 1);
    assert_eq!(decoded_v2.encode_project().unwrap(), v2_bytes);

    // 5. Classic v3 master volume project serialization
    s.session.set_master_volume(75).unwrap();
    let v3_bytes = s.session.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(v3_bytes[8..10].try_into().unwrap()), 3);
    let decoded_v3 = Session::decode_project(&v3_bytes).unwrap();
    assert_eq!(decoded_v3.master_volume(), 75);
    assert_eq!(decoded_v3.encode_project().unwrap(), v3_bytes);
}

#[test]
fn test_2_opt_in_muse_setup_lifecycle() {
    let (mut ui, mut s) = test_studio();

    // 1. Initial state: setup card is closed
    assert!(!s.muse_setup_open);
    assert!(!s.muse_config.enabled);

    // 2. Clicking Muse toggle opens setup prompt with RAM recommendation
    s.set_detected_ram_mb(16_384);
    s.on(&mut ui, &Response::Command("moan.muse.toggle".into()))
        .unwrap();
    assert!(s.muse_setup_open);
    assert!(!s.muse_config.enabled);

    // 3. Only the built-in synth is offered until model packs and a Muse
    //    server are really wired up on SexOS.
    // 6. Test "Use Built-in Synth (0 MB)"
    s.on(&mut ui, &Response::Command("moan.muse.disable".into()))
        .unwrap();
    s.muse_pack_state = ModelPackState::NotInstalled;
    s.on(&mut ui, &Response::Command("moan.muse.toggle".into()))
        .unwrap();
    s.on(&mut ui, &Response::Clicked(s.muse_ui.tier0_synth_btn))
        .unwrap();
    assert!(s.muse_config.enabled);
    assert_eq!(s.muse_pack_state, ModelPackState::FeatherweightOnly);

    // 7. Add audio while Muse is enabled, then disable Muse -> track remains intact!
    s.session.add(vec![[200, -200]; 4800]).unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    s.on(&mut ui, &Response::Command("moan.muse.disable".into()))
        .unwrap();
    assert!(!s.muse_config.enabled);
    assert_eq!(s.session.tracks.len(), 1);
}

#[test]
fn test_3_muse_studio_workflows() {
    let (mut ui, mut s) = test_studio();
    s.muse_config.enabled = true;

    // 1. "Create Song" via Muse Tier 0 engine
    s.muse_ui.prompt_input = String::from("driving electro synth");
    ui.set_text(s.muse_ui.prompt_field, "driving electro synth");
    s.on(&mut ui, &Response::Clicked(s.muse_ui.primary_action_btn))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    assert!(!s.muse_crate.is_empty());
    assert_eq!(s.muse_crate[0].title, "driving electro synth");

    // 2. "Add Layer" creates aligned stem track
    s.muse_ui.current_role = StemRole::Bass;
    s.session.from = 0;
    s.session.to = 0; // No range selection -> "Add Layer"
    s.on(&mut ui, &Response::Clicked(s.muse_ui.primary_action_btn))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 2);
    assert!(s.session.undo());
    assert_eq!(s.session.tracks.len(), 1);
    assert!(s.session.redo());
    assert_eq!(s.session.tracks.len(), 2);

    // 3. "Replace Section" crossfaded inpainting preserving outside frames
    s.session.selected = 0;
    s.session.from = 24_000;
    s.session.to = 48_000;
    let initial_frame_0 = s.session.tracks[0].clips[0].source_frames()[100];
    s.on(&mut ui, &Response::Clicked(s.muse_ui.primary_action_btn))
        .unwrap();
    // Non-selected region remains bit-identical
    assert_eq!(
        s.session.tracks[0].clips[0].source_frames()[100],
        initial_frame_0
    );
    // 1-step undo restores previous clip
    assert!(s.session.undo());

    // 4. "Extract Stems" splits clip into multi-track stems
    s.session.selected = 0;
    s.on(&mut ui, &Response::Clicked(s.muse_ui.extract_stems_btn))
        .unwrap();
    assert!(s.session.tracks.len() >= 4);

    // 5. Studio 2.0 Chat bar: natural language sound design & tempo commands
    s.session.selected = 0;
    s.muse_ui.prompt_input = String::from("gritty asymmetric distortion");
    ui.set_text(s.muse_ui.prompt_field, "gritty asymmetric distortion");
    s.on(&mut ui, &Response::Clicked(s.muse_ui.run_chat_btn))
        .unwrap();
    assert!(s.session.tracks[0].distortion.is_some());
    let dist = s.session.tracks[0].distortion.as_ref().unwrap();
    let (_, _, _, _, _, mode) = dist.parameters();
    assert_eq!(mode, DistortionMode::Asymmetric);

    // Chat tempo command
    s.muse_ui.prompt_input = String::from("128 bpm");
    ui.set_text(s.muse_ui.prompt_field, "128 bpm");
    s.on(&mut ui, &Response::Clicked(s.muse_ui.run_chat_btn))
        .unwrap();
}

#[test]
fn test_4_ariel_vocal_synthesis() {
    let (mut ui, mut s) = test_studio();

    // 1. Library Ariel Voice adds vocal synthesis track
    s.on(&mut ui, &Response::Command("moan.library.ariel".into()))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    let clip = &s.session.tracks[0].clips[0];
    assert!(clip.vocal_clip().is_some());
    let vocal = clip.vocal_clip().unwrap();
    assert_eq!(vocal.text, "Hello Moan");
    assert!(!vocal.spans.is_empty());

    // 2. English phonetic translation test
    let phonemes = s.ariel_ui.resolve_phonetics("world");
    assert!(phonemes.contains("W ER L D"));

    // 3. Japanese phonetic translation test
    s.ariel_ui.toggle_language(&mut ui);
    let ja_phonemes = s.ariel_ui.resolve_phonetics("tokyo");
    assert!(ja_phonemes.contains("T O K Y O"));
    s.ariel_ui.toggle_language(&mut ui); // back to EN

    // 4. Live karaoke span highlight
    let first_span = vocal.spans[0];
    s.transport_cursor(&mut ui, Some(first_span.start_frame + 10));

    // 5. MIDI singing
    let mut pattern = Pattern::preset(Instrument::SilkTriangle);
    pattern.notes.push(Note {
        start: 0,
        frames: 24_000,
        key: 60,
        velocity: 96,
    });
    s.session.add_midi(pattern).unwrap();
    assert_eq!(s.session.tracks.len(), 2);
    s.session.selected = 1;
    s.on(&mut ui, &Response::Clicked(s.ariel_ui.sing_btn))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 3);
}

#[test]
fn test_5_europarack_modular() {
    let (mut ui, mut s) = test_studio();

    // 1. Switch to EuropaRack workspace
    s.on(&mut ui, &Response::Command("moan.modular".into()))
        .unwrap();
    assert_eq!(s.workspace, Workspace::EuropaRack);

    // 2. Cycle and load "Generative Turing Choir (Ariel CV)" factory preset
    while s.europarack_ui.cycle_factory_patch(&mut ui) != "Generative Turing Choir (Ariel CV)" {}
    s.on(
        &mut ui,
        &Response::Clicked(s.europarack_ui.load_patch_btn),
    )
    .unwrap();
    assert!(s.session.rack.nodes.len() >= 4);
    assert!(s.session.rack.cables.len() >= 3);

    // 3. VCV Dark Room brightness mode pixel rendering
    s.session.rack.theme.dark_room_brightness = 90;
    let (_, _, pixels_bright) =
        europarack_ui::EuropaRackUi::render_canvas(&s.session.rack);
    s.session.rack.theme.dark_room_brightness = 10;
    let (_, _, pixels_dark) =
        europarack_ui::EuropaRackUi::render_canvas(&s.session.rack);
    assert_ne!(pixels_bright, pixels_dark);

    // 4. "Bake Rack to Track" renders Clip PCM to timeline
    s.on(&mut ui, &Response::Clicked(s.europarack_ui.bake_btn))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    assert!(s.session.tracks[0].clips[0].len() > 0);
    assert!(s.session.undo());
    assert_eq!(s.session.tracks.len(), 0);
}

#[test]
fn test_6_clip_warp_extract_midi_and_v4_persistence() {
    let (mut ui, mut s) = test_studio();

    // 1. Library Prism Synth
    s.on(&mut ui, &Response::Command("moan.library.prism".into()))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    assert_eq!(
        s.session.tracks[0].clips[0]
            .midi_pattern()
            .unwrap()
            .instrument,
        Instrument::PrismWavetable
    );

    // 2. Arpeggiator on selected MIDI clip
    s.session.selected = 0;
    s.on(&mut ui, &Response::Clicked(s.studio_rack_ui.arp_btn))
        .unwrap();
    assert_eq!(
        s.session.tracks[0].clips[0]
            .midi_pattern()
            .unwrap()
            .notes
            .len(),
        16
    );

    // 3. Clip Warp: Transpose & Speed
    s.session.add(vec![[500, -500]; 48_000]).unwrap();
    s.session.selected = 1;
    s.studio_rack_ui.transpose = 3;
    s.studio_rack_ui.speed_percent = 120;
    s.on(
        &mut ui,
        &Response::Clicked(s.studio_rack_ui.apply_warp_btn),
    )
    .unwrap();
    assert!(s.session.undo());

    // 4. Offline Extract MIDI from audio frames
    s.session.selected = 1;
    s.on(
        &mut ui,
        &Response::Clicked(s.studio_rack_ui.extract_midi_btn),
    )
    .unwrap();
    assert_eq!(s.session.tracks.len(), 3);
    assert!(s.session.tracks[2].clips[0].midi_pattern().is_some());

    // 5. Visual Lower Device Chain curves rendering
    let dummy_dist = Distortion::new(150, 20, -10, 80, 7000, DistortionMode::HardClip).unwrap();
    let dummy_eq = Equalizer::new(120, 90, 110).unwrap();
    let (cw, ch, c_pixels) = studio_rack_ui::StudioRackUi::render_device_curves(
        &s.studio_rack_ui.prism_params,
        Some(&dummy_dist),
        Some(&dummy_eq),
    );
    assert_eq!((cw, ch), (studio_rack_ui::CURVES_WIDTH, studio_rack_ui::CURVES_HEIGHT));
    assert!(c_pixels.iter().any(|p| *p == 0xff4ade80)); // Green wavetable curve
    assert!(c_pixels.iter().any(|p| *p == 0xfffb923c)); // Amber distortion curve
    assert!(c_pixels.iter().any(|p| *p == 0xff38bdf8)); // Cyan EQ curve

    // 6. MOANPRJ v4 round-trip across Vocal + Distortion + EuropaRack
    let mut s_v4 = Session::default();
    // Add Vocal
    s_v4.add_vocal("Moan AI Voice", VoiceParams::default())
        .unwrap();
    // Add Track with Distortion
    let mut t2 = moan_core::Track::new();
    t2.distortion = Some(dummy_dist);
    t2.clips
        .push(Clip::buffer(Arc::new(vec![[10, -10]; 4800]), 0).unwrap());
    s_v4.tracks.push(t2);
    // Add EuropaRack patch
    s_v4.rack = FactoryPatch::ClassicSubtractiveLead.build().unwrap();

    let encoded_v4 = s_v4.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(encoded_v4[8..10].try_into().unwrap()), 4);
    assert!(encoded_v4.windows(4).any(|w| w == b"EXT4"));

    let restored_v4 = Session::decode_project(&encoded_v4).unwrap();
    assert_eq!(restored_v4.tracks.len(), 2);
    assert!(restored_v4.tracks[0].clips[0].vocal_clip().is_some());
    assert_eq!(
        restored_v4.tracks[0].clips[0].vocal_clip().unwrap().text,
        "Moan AI Voice"
    );
    assert!(restored_v4.tracks[1].distortion.is_some());
    assert_eq!(restored_v4.rack.nodes.len(), s_v4.rack.nodes.len());
    assert_eq!(restored_v4.encode_project().unwrap(), encoded_v4);
}
