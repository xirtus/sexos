// SPDX-License-Identifier: GPL-3.0-only
//! Host integration tests for Cabaret (`apps/cabaret`):
//! Retained Weft UI, Studio Mode + Move + Wipe29 transitions, Downstream Keyer,
//! Multiview hit-testing, Burlesque deck stepping, Sidechain Ducking + R128/dBTP meters,
//! all 12 OBS plugins, wide-gamut sex-color scopes & FilmStock LUTs, Multi-Stream +
//! Shibari redaction + HybridMP4 6-track recording + Replay + QC J/K/L + Splicy handoff,
//! HIDE_FROM_CAPTURE Teleprompter, F1 6-step Coach, and OBS WS v5 / Browser / StreamDeck /
//! ScriptRPC / CLI control plane.

use cabaret::control::{
    dispatch_browser_message, dispatch_obs_message, dispatch_obs_request, dispatch_script_batch,
    dispatch_streamdeck_event, run_cli_args, ObsAuthSession,
};
use cabaret::render::{
    render_multiview_frame, render_program_frame, render_scope_argb, render_secondary_canvas_native,
    UI_PREVIEW_H, UI_PREVIEW_W,
};
use cabaret::studio::{ActiveTransitionPreset, CabaretStudio, ScopeKind};
use cabaret::{Cabaret, CabaretTab};
use cabaret_core::outputs::RecordingState;
use cabaret_core::scene::SourceKind;
use cabaret_core::transitions::{MultiviewAction, MultiviewPreset};
use cabaret_plugins::local_captions::{decode_cea608_pairs, depacketize_cea708};
use cabaret_plugins::studio_link::{LinkPacket, LinkPayload};
use cabaret_proto::browser_bridge::{BrowserToHostMessage, HostToBrowserMessage};
use cabaret_proto::json::JsonVal;
use cabaret_proto::obs_ws::{
    compute_obs_auth, status_code, ObsBatchRequest, ObsMessage, ObsRequest, ObsResponseData,
    OBS_WS_RPC_VERSION,
};
use cabaret_proto::script_rpc::{ScriptAction, ScriptRpcMessage};
use cabaret_proto::streamdeck::{StreamDeckInputDecoder, StreamDeckModel};
use sex_color::{ColorSpace, FilmStock};
use weft::input::{code, Button, Event, Key, Pointer, PointerKind};
use weft::widget::Id;
use weft::{Response, Strings, Theme, Ui};

fn pump(ui: &mut Ui, app: &mut Cabaret, rs: Vec<Response>) {
    for r in rs {
        app.on(ui, &r);
    }
}

fn click_id(ui: &mut Ui, app: &mut Cabaret, id: Id) {
    ui.layout();
    let rect = ui.rect(id);
    assert!(!rect.is_empty(), "widget {id:?} has empty rect");
    let c = rect.center();
    let s = ui.theme().scale;
    click_at(ui, app, c.x * s, c.y * s);
}

fn click_btn(ui: &mut Ui, app: &mut Cabaret, pick: impl FnOnce(&Cabaret) -> Id) {
    let id = pick(app);
    click_id(ui, app, id);
}

fn click_at(ui: &mut Ui, app: &mut Cabaret, x: f32, y: f32) {
    for kind in [
        PointerKind::Move,
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        let rs = ui.handle(Event::Pointer(Pointer { x, y, kind }));
        pump(ui, app, rs);
    }
}

fn paint_assert(ui: &mut Ui, w: usize, h: usize) -> Vec<u32> {
    let mut px = vec![0u32; w * h];
    assert!(ui.paint(&mut px).is_some(), "expected ui.paint to draw");
    px
}

#[test]
fn window_builds_seeds_all_tabs_and_paints_live_canvases() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);

    assert_eq!(app.studio.collection.studios.len(), 4);
    let studio_names: Vec<&str> = app
        .studio
        .collection
        .studios
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        studio_names,
        vec![
            "Main Camera & Slides",
            "Screen + Picture-in-Picture",
            "Coding & Terminal",
            "BRB / Starting Soon",
        ]
    );
    assert_eq!(app.studio.audio.strips.len(), 4);
    assert!(ui.text_of(app.tally_bar).contains("Cabaret Virtual Camera"));
    assert!(ui.text_of(app.tally_bar).contains("HIDE_FROM_CAPTURE"));

    let px0 = paint_assert(&mut ui, 1280, 860);
    assert!(px0.iter().any(|&p| p != px0[0]), "canvas should not be uniform");

    // Verify all 6 tabs via Ctrl+1..Ctrl+6 shortcuts and paint each view cleanly
    for (digit_code, tab) in [
        (0x03u8, CabaretTab::Multiview),
        (0x04, CabaretTab::PluginsVertical),
        (0x05, CabaretTab::ScopesColor),
        (0x06, CabaretTab::OutputsMultiStream),
        (0x07, CabaretTab::PrompterQc),
        (0x02, CabaretTab::StudioMixer),
    ] {
        let rs = ui.handle(Event::Key(Key::ctrl(digit_code)));
        pump(&mut ui, &mut app, rs);
        assert_eq!(app.active_tab, tab);
        paint_assert(&mut ui, 1280, 860);
    }
}

#[test]
fn studio_mode_transitions_move_and_wipe29_with_dsk_pinned() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);

    let main_id = app.studio.studios.main_camera_slides;
    let screen_id = app.studio.studios.screen_pip;
    assert_eq!(app.studio.studio_mode.program_studio, main_id);
    assert_eq!(app.studio.studio_mode.preview_studio, screen_id);

    // DSK Channel 1 ("Logo Bug") is active & visible above transitions
    assert!(app.studio.dsk.channel(1).unwrap().is_visible_on_air());
    let before_bug_px = app
        .last_program_frame
        .get_pixel(UI_PREVIEW_W - 20, 12)
        .unwrap();
    // Magenta logo bug pixel has high red channel
    assert!(before_bug_px[0] > 0.7);

    // Select Move transition & start non-immediate Take to inspect mid-transition interpolation + DSK pin
    click_btn(&mut ui, &mut app, |a| {
        a.transition_btns
            .iter()
            .find(|(_, p)| *p == ActiveTransitionPreset::Move)
            .unwrap()
            .0
    });
    assert_eq!(app.studio.transition_preset, ActiveTransitionPreset::Move);

    let _ = app.studio.trigger_take(false);
    assert!(app.studio.studio_mode.running.is_some());
    app.studio.step_frame(200);
    let mid_move = render_program_frame(&mut app.studio, UI_PREVIEW_W, UI_PREVIEW_H);
    let mid_bug_px = mid_move.get_pixel(UI_PREVIEW_W - 20, 12).unwrap();
    assert!(
        mid_bug_px[0] > 0.7,
        "DSK logo bug must remain pinned above Move transitions"
    );

    // Finish Move transition
    app.studio.step_frame(400);
    assert_eq!(app.studio.studio_mode.program_studio, screen_id);
    assert_eq!(app.studio.studio_mode.preview_studio, main_id);

    // Switch to Wipe29 & cycle wipe pattern, verify mid-wipe render differs from source
    click_btn(&mut ui, &mut app, |a| a.wipe_next_btn);
    assert_eq!(app.studio.transition_preset, ActiveTransitionPreset::Wipe29);
    assert!(ui.text_of(app.wipe_label).contains("Wipe #6/29"));

    // Drive T-Bar partially then to 100% commit
    app.studio.set_t_bar(0.5);
    let mid_wipe = render_program_frame(&mut app.studio, UI_PREVIEW_W, UI_PREVIEW_H);
    assert_ne!(mid_wipe.pixels, mid_move.pixels);
    let committed = app.studio.set_t_bar(1.0);
    assert_eq!(committed, Some(main_id));
    assert_eq!(app.studio.studio_mode.program_studio, main_id);

    // Click UI Take button to transition back to Screen + Picture-in-Picture
    click_btn(&mut ui, &mut app, |a| a.take_btn);
    assert_eq!(app.studio.studio_mode.program_studio, screen_id);
}

#[test]
fn multiview_presets_safe_areas_and_hit_testing() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);
    app.set_tab(&mut ui, CabaretTab::Multiview);
    paint_assert(&mut ui, 1280, 860);

    // Cycle Multiview presets via UI buttons (4, 9, 16, 25, Standard8)
    for (preset, expected_slots) in [
        (MultiviewPreset::Grid4, 4usize),
        (MultiviewPreset::Grid9, 9),
        (MultiviewPreset::Grid16, 16),
        (MultiviewPreset::Grid25, 25),
        (MultiviewPreset::Standard8, 10),
    ] {
        click_btn(&mut ui, &mut app, |a| {
            a.mv_preset_btns
                .iter()
                .find(|(_, p)| *p == preset)
                .unwrap()
                .0
        });
        let mv = app.studio.multiview_layout(480.0, 270.0);
        assert_eq!(mv.slots.len(), expected_slots);
    }

    // Hit-testing on Multiview coordinates: click slot #2 ("Coding & Terminal") -> Preview
    let mv_layout = app.studio.multiview_layout(480.0, 270.0);
    let slot_coding = mv_layout
        .slots
        .iter()
        .find(|s| s.studio_id == Some(app.studio.studios.coding_terminal))
        .expect("Coding & Terminal slot in Multiview");
    let cx = slot_coding.rect.x + slot_coding.rect.w * 0.5;
    let cy = slot_coding.rect.y + slot_coding.rect.h * 0.5;

    let act1 = app.studio.multiview_click(480.0, 270.0, cx, cy, 1);
    assert_eq!(
        act1,
        Some(MultiviewAction::ClickToPreview(
            app.studio.studios.coding_terminal
        ))
    );
    assert_eq!(
        app.studio.studio_mode.preview_studio,
        app.studio.studios.coding_terminal
    );

    // Double-click slot -> Take to Program
    let act2 = app.studio.multiview_click(480.0, 270.0, cx, cy, 2);
    assert_eq!(
        act2,
        Some(MultiviewAction::DoubleClickToTake(
            app.studio.studios.coding_terminal
        ))
    );
    assert_eq!(
        app.studio.studio_mode.program_studio,
        app.studio.studios.coding_terminal
    );

    let mv_fb = render_multiview_frame(&mut app.studio, 480, 270);
    assert_eq!(mv_fb.width, 480);
    assert_eq!(mv_fb.height, 270);
}

#[test]
fn burlesque_presentation_deck_steps_builds_and_notes_live() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);

    assert_eq!(app.studio.presentation.state(), (0, 3, 0, 2));
    let before_fb = render_program_frame(&mut app.studio, UI_PREVIEW_W, UI_PREVIEW_H);

    // Step through build steps 1 and 2, then advance to slide 2
    click_btn(&mut ui, &mut app, |a| a.deck_next_btn);
    assert_eq!(app.studio.presentation.state(), (0, 3, 1, 2));
    click_btn(&mut ui, &mut app, |a| a.deck_next_btn);
    assert_eq!(app.studio.presentation.state(), (0, 3, 2, 2));
    click_btn(&mut ui, &mut app, |a| a.deck_next_btn);
    assert_eq!(app.studio.presentation.state().0, 1);
    assert!(ui
        .text_of(app.deck_status_label)
        .contains("Wide-Gamut Color & Audio Mixing"));
    assert!(ui
        .text_of(app.deck_status_label)
        .contains("ducks automatically"));

    // Verify SourceKind::PresentationDeck metadata updated & Program canvas pixels changed
    if let SourceKind::PresentationDeck { slide_idx, .. } = &app
        .studio
        .collection
        .source(app.studio.sources.presentation_deck)
        .unwrap()
        .kind
    {
        assert_eq!(*slide_idx, 1);
    } else {
        panic!("expected PresentationDeck source kind");
    }

    let after_fb = render_program_frame(&mut app.studio, UI_PREVIEW_W, UI_PREVIEW_H);
    assert_ne!(before_fb.pixels, after_fb.pixels);

    click_btn(&mut ui, &mut app, |a| a.deck_prev_btn);
    assert_eq!(app.studio.presentation.state().0, 0);
}

#[test]
fn audio_mixer_sidechain_ducking_and_r128_dbtp_metering() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);

    // Feed 500ms of silent mic + active desktop signal -> baseline un-ducked Desktop level
    let mut unducked = app.studio.tick_audio(0.0, 0.50, 24_000);
    for _ in 0..2 {
        unducked = app.studio.tick_audio(0.0, 0.50, 12_000);
    }
    let desk_id = app.studio.strips.desktop_bell;
    let mic_id = app.studio.strips.mic_serenade;
    let desk_unducked_db = unducked.strip_meters[&desk_id].sample_peak_db;

    // Now feed strong Mic / Serenade signal + same Desktop signal -> Desktop ducks via sidechain!
    let mut ducked = app.studio.tick_audio(0.85, 0.50, 24_000);
    for _ in 0..2 {
        ducked = app.studio.tick_audio(0.85, 0.50, 12_000);
    }
    let desk_ducked_db = ducked.strip_meters[&desk_id].sample_peak_db;
    let mic_meters = ducked.strip_meters[&mic_id];

    assert!(
        desk_ducked_db < desk_unducked_db - 4.0,
        "Expected sidechain ducking on Desktop strip: unducked={desk_unducked_db:.2} dB, ducked={desk_ducked_db:.2} dB"
    );
    assert!(
        mic_meters.true_peak_dbtp > -30.0 && mic_meters.integrated_lufs > -50.0,
        "Expected real 4x True-Peak ({:.2} dBTP) and EBU R128 integrated LUFS ({:.2})",
        mic_meters.true_peak_dbtp,
        mic_meters.integrated_lufs
    );

    // Click UI Sample Audio & Mute button on Mic strip
    click_btn(&mut ui, &mut app, |a| a.audio_pulse_btn);
    assert!(ui
        .text_of(app.strip_uis[0].meters_label)
        .contains("dBTP"));
    assert!(ui
        .text_of(app.strip_uis[0].meters_label)
        .contains("LUFS"));

    click_btn(&mut ui, &mut app, |a| a.strip_uis[0].mute_btn);
    assert!(app.studio.audio.strip(mic_id).unwrap().muted);
    assert_eq!(ui.text_of(app.strip_uis[0].mute_btn), "Unmute");
}

#[test]
fn all_12_plugins_and_color_scopes_work_end_to_end() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);
    app.set_tab(&mut ui, CabaretTab::PluginsVertical);

    // 1. Secondary 9:16 Vertical Canvas & backtrack replay
    click_btn(&mut ui, &mut app, |a| a.vertical_backtrack_btn);
    let vert_fb = render_secondary_canvas_native(&mut app.studio)
        .expect("SecondaryCanvasManager::render_canvas");
    assert_eq!((vert_fb.width, vert_fb.height), (1080, 1920));

    // 2. Downstream Keyer Toggle (Channel 2 Lower-Third)
    click_btn(&mut ui, &mut app, |a| a.dsk_uis[1].toggle_btn);
    assert!(app.studio.dsk.channel(2).unwrap().active);
    assert_eq!(ui.text_of(app.dsk_uis[1].toggle_btn), "ON AIR");

    // 3. Shader Filter + 3D Corner-Pin + Freeze + TimeWarp + Dynamic Delay + Source-Record Tap
    click_btn(&mut ui, &mut app, |a| a.shader_cycle_btn);
    assert!(app.studio.active_shader_preset().is_some());
    click_btn(&mut ui, &mut app, |a| a.warp_3d_check);
    assert!(app.studio.warp_3d_enabled);
    click_btn(&mut ui, &mut app, |a| a.freeze_btn);
    click_btn(&mut ui, &mut app, |a| a.time_warp_check);
    assert!(app.studio.time_warp_enabled);
    click_btn(&mut ui, &mut app, |a| a.delay_slowmo_btn);
    assert!(!app.studio.delay_buffer.live_follow);
    click_btn(&mut ui, &mut app, |a| a.source_tap_record_btn);
    assert_eq!(app.studio.source_tap.recording.state, RecordingState::Recording);
    app.studio.step_frame(16);
    app.refresh_all(&mut ui);
    assert!(app.studio.source_tap.video_frames_captured >= 1);

    // 4. Local Captions (CEA-608, CEA-708, SRT/VTT)
    click_btn(&mut ui, &mut app, |a| a.captions_burn_check);
    click_btn(&mut ui, &mut app, |a| a.captions_push_btn);
    let decoded_608 = decode_cea608_pairs(&app.studio.last_cea608).expect("valid odd-parity 608");
    assert!(!decoded_608.is_empty());
    let pkt708 = app.studio.last_cea708.as_ref().expect("CEA-708 packet");
    let decoded_708 = depacketize_cea708(&pkt708.cc_triplets).expect("valid 708");
    assert!(!decoded_708.is_empty());
    assert!(app.studio.captions.export_srt().contains("-->"));
    assert!(app.studio.captions.export_vtt().starts_with("WEBVTT"));

    // 5. Macro Engine (`adv-switcher`) Silk window focus auto-switch
    click_btn(&mut ui, &mut app, |a| a.macro_spindle_btn);
    assert_eq!(
        app.studio.studio_mode.program_studio,
        app.studio.studios.coding_terminal
    );

    // 6. CabaretLink LAN video & tally packet roundtrip
    let pkt_bytes = app
        .studio
        .emit_cabaret_link_packet(&app.last_program_frame)
        .expect("CabaretLink packet");
    let decoded_pkt = LinkPacket::decode(&pkt_bytes).expect("decode LinkPacket");
    assert!(matches!(decoded_pkt.payload, LinkPayload::Video(_)));
    let tally_bytes = app.studio.link_tally_packet();
    let decoded_tally = LinkPacket::decode(&tally_bytes).expect("decode tally packet");
    assert!(matches!(decoded_tally.payload, LinkPayload::Tally(_)));

    // 7. Scopes & Color tab: wide-gamut ColorSpace + FilmStock LUT + all 4 ScopeKinds
    app.set_tab(&mut ui, CabaretTab::ScopesColor);
    for cs in [
        ColorSpace::LinearRec2020,
        ColorSpace::Hdr10Pq,
        ColorSpace::Hlg,
        ColorSpace::Rec709,
    ] {
        click_btn(&mut ui, &mut app, |a| {
            a.color_space_btns
                .iter()
                .find(|(_, c)| *c == cs)
                .unwrap()
                .0
        });
        assert_eq!(app.studio.color_pipeline.canvas_space, cs);
    }

    click_btn(&mut ui, &mut app, |a| a.film_stock_next_btn);
    assert_eq!(app.studio.color_pipeline.film_stock, Some(FilmStock::Kodak2383Print));

    for kind in ScopeKind::ALL {
        click_btn(&mut ui, &mut app, |a| {
            a.scope_kind_btns
                .iter()
                .find(|(_, k)| *k == kind)
                .unwrap()
                .0
        });
        let scope_argb =
            render_scope_argb(&app.studio, &app.last_program_frame, UI_PREVIEW_W, UI_PREVIEW_H);
        assert_eq!(scope_argb.len(), (UI_PREVIEW_W * UI_PREVIEW_H) as usize);
        assert!(
            scope_argb.iter().any(|&p| p != 0xFF0E_1014),
            "Scope {kind:?} rendered empty"
        );
    }
    paint_assert(&mut ui, 1280, 860);
}

#[test]
fn multi_stream_recording_replay_and_shibari_redaction() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);
    app.set_tab(&mut ui, CabaretTab::OutputsMultiStream);

    // EncoderGroupPlanner groups Twitch + SRT + WHIP (H.264 6000k) and separates YouTube AV1 (12000k)
    let groups = app.studio.encoder_groups();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().any(|g| g.shared && g.dest_ids.len() == 3));
    assert!(app.studio.validate_streams().is_empty());

    // Log a message containing raw Shibari secrets and verify redaction
    let raw_twitch_secret = app.studio.active_secrets[0].1.clone();
    app.studio
        .log_event(&format!("Connecting RTMP with key {raw_twitch_secret}"));
    let last_log = app.studio.log_lines.last().unwrap();
    assert!(!last_log.contains(&raw_twitch_secret));
    assert!(last_log.contains("[SHIBARI:REDACTED:slot#1]"));

    // Start streaming + recording + pause + resume + chapter + split + replay + virtual cam
    click_btn(&mut ui, &mut app, |a| a.stream_toggle_btn);
    assert!(app.studio.stream_active);
    click_btn(&mut ui, &mut app, |a| a.record_toggle_btn);
    assert_eq!(app.studio.recording.state, RecordingState::Recording);
    app.studio.step_frame(100);
    click_btn(&mut ui, &mut app, |a| a.record_pause_btn);
    assert_eq!(app.studio.recording.state, RecordingState::Paused);
    click_btn(&mut ui, &mut app, |a| a.record_pause_btn);
    assert_eq!(app.studio.recording.state, RecordingState::Recording);
    click_btn(&mut ui, &mut app, |a| a.chapter_add_btn);
    assert_eq!(app.studio.recording.chapters.len(), 1);
    click_btn(&mut ui, &mut app, |a| a.split_file_btn);
    assert_eq!(app.studio.recording.segment_index, 2);

    click_btn(&mut ui, &mut app, |a| a.replay_save_btn);
    assert_eq!(app.studio.saved_replays_count, 1);
    assert!(app.studio.last_replay_packets > 0);

    click_btn(&mut ui, &mut app, |a| a.vcam_toggle_btn);
    assert!(app.studio.virtual_cam.active);
    assert_eq!(ui.text_of(app.vcam_toggle_btn), "Stop Virtual Cam");
    paint_assert(&mut ui, 1280, 860);
}

#[test]
fn qc_shuttle_splicy_handoff_teleprompter_and_f1_coach() {
    let mut ui = Ui::new(1280, 860, Theme::dark(), Strings::new("en"));
    let mut app = Cabaret::build(&mut ui);

    // 1. F1 Coach 6-step walkthrough
    ui.layout();
    assert!(!app.coach.active);
    let rs = ui.handle(Event::Key(Key::new(code::F1)));
    pump(&mut ui, &mut app, rs);
    assert!(app.coach.active);
    assert_eq!(app.coach.progress_label(), "1 / 6");
    for expected in ["2 / 6", "3 / 6", "4 / 6", "5 / 6", "6 / 6"] {
        click_btn(&mut ui, &mut app, |a| a.coach_controls.next);
        assert_eq!(ui.text_of(app.coach_controls.progress), expected);
    }
    click_btn(&mut ui, &mut app, |a| a.coach_controls.done);
    assert!(!app.coach.active);

    // 2. Teleprompter HIDE_FROM_CAPTURE + voice tracking + mirror
    app.set_tab(&mut ui, CabaretTab::PrompterQc);
    assert!(ui
        .text_of(app.prompter_badge)
        .contains("HIDE_FROM_CAPTURE"));
    assert!(app.studio.teleprompter.hide_from_capture);

    click_btn(&mut ui, &mut app, |a| a.prompter_wpm_up_btn);
    assert_eq!(app.studio.teleprompter.scroll_wpm, 160.0);
    click_btn(&mut ui, &mut app, |a| a.prompter_mirror_h_check);
    assert!(app.studio.teleprompter.mirror_h);
    click_btn(&mut ui, &mut app, |a| a.prompter_voice_track_btn);
    assert!(app.studio.teleprompter.voice_word_index >= 5);

    // 3. QC Shuttle J/K/L + Frame Step + Mark In/Out + 1-click Open in Splicy
    click_btn(&mut ui, &mut app, |a| a.qc_l_btn); // +1x
    assert_eq!(app.studio.qc_player.shuttle_speed, 1.0);
    click_btn(&mut ui, &mut app, |a| a.qc_l_btn); // +2x
    assert_eq!(app.studio.qc_player.shuttle_speed, 2.0);
    click_btn(&mut ui, &mut app, |a| a.qc_mark_in_btn);
    click_btn(&mut ui, &mut app, |a| a.qc_l_btn); // +4x & advance 500ms
    assert_eq!(app.studio.qc_player.shuttle_speed, 4.0);
    click_btn(&mut ui, &mut app, |a| a.qc_k_btn); // Pause
    assert_eq!(app.studio.qc_player.shuttle_speed, 0.0);
    click_btn(&mut ui, &mut app, |a| a.qc_step_fwd_btn);
    click_btn(&mut ui, &mut app, |a| a.qc_mark_out_btn);
    click_btn(&mut ui, &mut app, |a| a.qc_j_btn); // -1x reverse
    assert_eq!(app.studio.qc_player.shuttle_speed, -1.0);

    // Add a chapter marker & click Open Last Recording in Splicy
    app.studio.add_recording_chapter("Highlight Beat");
    click_btn(&mut ui, &mut app, |a| a.open_splicy_btn);
    let handoff = app
        .studio
        .last_splicy_handoff
        .as_ref()
        .expect("Splicy handoff created");
    assert_eq!(handoff.timeline.tracks.len(), 2);
    assert_eq!(handoff.timeline.tracks[0].clips.len(), 1);
    assert_eq!(handoff.timeline.tracks[1].clips.len(), 1);
    assert_eq!(handoff.timeline.markers.len(), 1);
    assert!(handoff.otio_json.contains("Timeline.1"));
    assert!(handoff.bundle.fcpxml.contains("<fcpxml"));
    assert!(ui
        .text_of(app.splicy_summary_label)
        .contains("Splicy Handoff Ready"));
    paint_assert(&mut ui, 1280, 860);
}

#[test]
fn obs_ws_v5_protocol_browser_bridge_streamdeck_script_rpc_and_cli() {
    let mut studio = CabaretStudio::new_seeded();

    // 1. OBS WebSocket v5 Hello + Identify auth
    let mut auth = ObsAuthSession::new("super-secret-obs", "salt123", "chal456");
    assert!(matches!(auth.hello_message(), ObsMessage::Hello { .. }));
    assert!(dispatch_obs_message(
        &mut studio,
        &mut auth,
        &ObsMessage::Identify {
            rpc_version: OBS_WS_RPC_VERSION,
            authentication: Some(String::from("wrong-auth")),
            event_subscriptions: None,
        }
    )
    .is_none());

    let valid_sig = compute_obs_auth("super-secret-obs", "salt123", "chal456");
    let identified = dispatch_obs_message(
        &mut studio,
        &mut auth,
        &ObsMessage::Identify {
            rpc_version: OBS_WS_RPC_VERSION,
            authentication: Some(valid_sig),
            event_subscriptions: None,
        },
    )
    .expect("valid auth succeeds");
    assert!(matches!(identified, ObsMessage::Identified { .. }));

    // Dispatch GetSceneList, SetCurrentPreviewScene, TriggerStudioModeTransition, Mute, Record
    let scenes_resp = dispatch_obs_request(&mut studio, &ObsRequest::GetSceneList);
    assert!(scenes_resp.status.result);
    if let ObsResponseData::SceneList { scenes, .. } = scenes_resp.data {
        assert_eq!(scenes.len(), 4);
    } else {
        panic!("expected SceneList");
    }

    let r_prv = dispatch_obs_request(
        &mut studio,
        &ObsRequest::SetCurrentPreviewScene {
            scene_name: String::from("Coding & Terminal"),
        },
    );
    assert!(r_prv.status.result);
    assert!(
        dispatch_obs_request(&mut studio, &ObsRequest::TriggerStudioModeTransition)
            .status
            .result
    );
    assert_eq!(
        studio.studio_mode.program_studio,
        studio.studios.coding_terminal
    );

    // Mute and Volume
    assert!(dispatch_obs_request(
        &mut studio,
        &ObsRequest::SetInputMute {
            input_name: String::from("Mic / Serenade"),
            input_muted: true,
        }
    )
    .status
    .result);
    assert!(studio.audio.strip(studio.strips.mic_serenade).unwrap().muted);

    // RequestBatch over OBS WS v5
    let batch = dispatch_obs_message(
        &mut studio,
        &mut auth,
        &ObsMessage::RequestBatch {
            request_id: String::from("b1"),
            halt_on_failure: Some(true),
            execution_type: None,
            requests: vec![
                ObsBatchRequest {
                    request_id: None,
                    request: ObsRequest::StartStream,
                },
                ObsBatchRequest {
                    request_id: None,
                    request: ObsRequest::GetVirtualCamStatus,
                },
            ],
        },
    )
    .expect("batch response");
    match batch {
        ObsMessage::RequestBatchResponse { results, .. } => {
            assert_eq!(results.len(), 2);
            assert!(results[0].status.result);
            assert!(studio.stream_active);
        }
        other => panic!("unexpected {other:?}"),
    }

    // 2. Browser bridge (`window.obsstudio`)
    assert!(studio.browser_init_js.contains("window.obsstudio"));
    let req_frame = BrowserToHostMessage::GetCurrentScene {
        call_id: String::from("c1"),
    }
    .format_frame();
    let resp_frame = dispatch_browser_message(&mut studio, &req_frame).expect("bridge reply");
    let parsed_reply = HostToBrowserMessage::parse_frame(&resp_frame).unwrap();
    if let HostToBrowserMessage::Response { payload, .. } = parsed_reply {
        assert_eq!(payload.get_str("name"), Some("Coding & Terminal"));
    } else {
        panic!("expected bridge response");
    }

    // 3. Stream Deck HID input report decoding & dispatch
    let mut dec = StreamDeckInputDecoder::new(StreamDeckModel::Plus);
    // Press Key #4 (Trigger TAKE) on Gen2 8-key report [0x01, 0x00, 0, 0, k0..k7]
    let report = [0x01u8, 0x00, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
    let sd_events = dec.process_report(&report).unwrap();
    assert!(!sd_events.is_empty());
    let sd_out = dispatch_streamdeck_event(&mut studio, sd_events[0]).unwrap();
    assert!(sd_out.contains("TAKE"));

    // 4. Out-of-process ScriptRPC atomic batch rollback on failure
    let pgm_before_fail = studio.studio_mode.program_studio;
    let rolled = dispatch_script_batch(
        &mut studio,
        "batch-fail",
        true,
        &[
            ScriptAction::Obs(ObsRequest::SetCurrentProgramScene {
                scene_name: String::from("BRB / Starting Soon"),
            }),
            ScriptAction::Obs(ObsRequest::SetCurrentProgramScene {
                scene_name: String::from("NonExistent Studio"),
            }),
        ],
    );
    match rolled {
        ScriptRpcMessage::BatchResult {
            committed,
            rolled_back,
            ..
        } => {
            assert!(!committed);
            assert!(rolled_back);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(studio.studio_mode.program_studio, pgm_before_fail);

    // 5. OmaTools Headless CLI (`schema`, `status --json`, `list scenes`, `ctl`, `apply`)
    let out_schema = run_cli_args(&mut studio, &["cabaret", "schema"]);
    assert_eq!(out_schema.exit_code, 0);
    assert!(
        out_schema.stdout.contains("\"tool\": \"cabaret\"")
            || out_schema.stdout.contains("\"tool\":\"cabaret\"")
    );

    let out_status = run_cli_args(&mut studio, &["status", "--json"]);
    assert_eq!(out_status.exit_code, 0);
    let parsed = JsonVal::parse(&out_status.stdout).expect("valid status JSON");
    assert_eq!(
        parsed.get_str("virtualCamName"),
        Some("Cabaret Virtual Camera")
    );
    assert_eq!(parsed.get_bool("hideSelfFromCapture"), Some(true));
    // Ensure raw secrets NEVER appear in CLI status output
    assert!(!out_status.stdout.contains("live_998877_secret_twitch_tok"));

    let out_ctl = run_cli_args(&mut studio, &["ctl", "preview", "BRB / Starting Soon"]);
    assert_eq!(out_ctl.exit_code, 0);
    assert_eq!(
        studio.studio_mode.preview_studio,
        studio.studios.brb_starting_soon
    );

    let out_apply = run_cli_args(
        &mut studio,
        &["apply", r#"{"transition":"Wipe29","take":true}"#],
    );
    assert_eq!(out_apply.exit_code, 0);
    assert_eq!(
        studio.studio_mode.program_studio,
        studio.studios.brb_starting_soon
    );
    assert_eq!(studio.transition_preset, ActiveTransitionPreset::Wipe29);
    let _ = status_code::SUCCESS;
}
