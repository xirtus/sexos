// SPDX-License-Identifier: GPL-3.0-only
//! Integration Test Suite for Moan Tapestry & Pro Studio Workstation:
//! 1. Workspace::Tapestry Channel Rack, 4-beat colors, Song Starters, pointer toggles,
//!    linked PatternInstances, Make Unique divergence, SceneMatrix, and Undo/Redo.
//! 2. Cut/Choke hi-hat groups, Sub 808 legato pitch slides, Piano Roll visual slide
//!    connector pixels, and ComposeTool transforms (Chop, Strum, Legato, Flam) via Weft UI.
//! 3. All 7 new instruments (Sub 808, Studio Drums, Tine Keys, Grand Piano, Electric Guitar,
//!    Mallet Bells, Cross Synth) baking distinct non-silent audio.
//! 4. Pro Console Mixing: sidechain ducking + GR dB, 6-band ParametricEQ + Auto Cut Resonance,
//!    Tape/Gate/Transient/BitCrush/Chorus FX, Sends/Buses, and -1.0 dB Lookahead Mastering.
//! 5. Standard MIDI File (.mid) Format 1 960 PPQ export/re-import round-trip & Retrospective
//!    Keyboard Typing -> Recover Riff workflow.
//! 6. MOANPRJ EXT5 persistence, byte-exact v1/v2/v3/v4 backward compatibility, window fit,
//!    and UI hygiene.

extern crate alloc;

use alloc::{vec, vec::Vec};
use moan::{
    piano,
    tapestry_ui::{self, TapestryUi},
    Session, Studio, Workspace,
};
use moan_core::{
    device_presets::{GATE_PRESETS, MASTERING_PRESETS, TAPE_PRESETS},
    dsp_math::sin_f32,
    kits::{ALL_KITS, ALL_SONG_STARTERS, STEP_BEAT_COLORS},
    midi::{Instrument, Note, Pattern},
    pattern::TransportContext,
    time::Time,
    RATE,
};
use weft::{Button, Event, Key, Pointer, PointerKind, Response, Strings, Theme, Ui};

fn studio_sized(w: u32, h: u32) -> (Ui, Studio) {
    let mut ui = Ui::new(
        w,
        h,
        Theme::dark().with_density(weft::Density::Compact),
        Strings::new("en"),
    );
    let s = Studio::build(&mut ui);
    (ui, s)
}

fn studio() -> (Ui, Studio) {
    studio_sized(1264, 730)
}

fn click_button(ui: &mut Ui, s: &mut Studio, name: &str) {
    ui.layout();
    let id = s
        .buttons
        .iter()
        .find(|(_, n)| *n == name)
        .unwrap_or_else(|| panic!("missing button {name}"))
        .0;
    let p = ui.rect(id).center();
    assert!(p.x > 0.0 && p.y > 0.0, "button {name} not laid out: {p:?}");
    s.on(ui, &Response::Clicked(id)).unwrap();
}

fn click_point(ui: &mut Ui, s: &mut Studio, x: f32, y: f32) {
    for kind in [
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        s.handle_event(ui, Event::Pointer(Pointer { x, y, kind }));
    }
}

fn rms_range(frames: &[[i16; 2]]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = frames
        .iter()
        .map(|f| (f[0] as f64) * (f[0] as f64) + (f[1] as f64) * (f[1] as f64))
        .sum();
    (sum_sq / (frames.len() as f64 * 2.0)).sqrt()
}

#[test]
fn test_1_tapestry_rack_starters_linked_instances_make_unique_and_undo() {
    let (mut ui, mut s) = studio();

    // Switch to Workspace::Tapestry via command and verify canvas rect & visibility
    s.on(&mut ui, &Response::Command("moan.tapestry".into()))
        .unwrap();
    assert_eq!(s.workspace, Workspace::Tapestry);
    ui.layout();
    let rack_rect = ui.rect(s.tapestry_ui.canvas);
    assert!(rack_rect.w >= 300.0 && rack_rect.h >= 150.0, "{rack_rect:?}");

    // Cycle to "Night Slide (Trap 140)" starter and load it
    while s.tapestry_ui.active_starter().name != "Night Slide (Trap 140)" {
        click_button(&mut ui, &mut s, "Tapestry Cycle Starter");
    }
    click_button(&mut ui, &mut s, "Load Starter");

    assert_eq!(
        s.session.tapestry.tempo_map.bpm_x100_at(Time::ZERO),
        14_000
    );
    let pat0_id = s.session.tapestry.active_pattern;
    let pat0 = s.session.tapestry.pattern(pat0_id).unwrap();
    assert_eq!(pat0.steps_per_beat, 8);
    assert!(!s.session.tapestry.channels.is_empty());
    assert!(!s.session.tracks.is_empty());

    // Verify rendered Channel Rack image includes 4-beat step colors and mini-roll pixels
    let rack_px = TapestryUi::render_rack(
        &s.session.tapestry,
        s.tapestry_ui.selected_channel_idx,
        &s.tapestry_ui.channel_colors,
    );
    assert_eq!(
        rack_px.len(),
        tapestry_ui::RACK_WIDTH * tapestry_ui::RACK_HEIGHT
    );
    for col in STEP_BEAT_COLORS {
        assert!(rack_px.contains(&col), "missing beat color {col:#x}");
    }

    // Real pointer toggle on channel 0 (Punch Kick), step 2
    ui.layout();
    let canvas_rect = ui.rect(s.tapestry_ui.canvas);
    let (pt_x, pt_y) = TapestryUi::step_point(canvas_rect, 0, 2, 64);
    let ch0 = s.session.tapestry.channels[0].id;
    let sig = s.session.tapestry.tempo_map.time_signature_at(Time::ZERO);
    let was_active = s
        .session
        .tapestry
        .pattern(pat0_id)
        .unwrap()
        .step_grid(ch0, s.session.tapestry.channels[0].root_key, sig)
        .unwrap()[2]
        .active;
    click_point(&mut ui, &mut s, pt_x, pt_y);
    let now_active = s
        .session
        .tapestry
        .pattern(pat0_id)
        .unwrap()
        .step_grid(ch0, s.session.tapestry.channels[0].root_key, sig)
        .unwrap()[2]
        .active;
    assert_ne!(was_active, now_active, "pointer click must toggle step");
    // Toggle back cleanly
    click_point(&mut ui, &mut s, pt_x, pt_y);

    // Paint TWO linked pattern instances onto the Song timeline
    click_button(&mut ui, &mut s, "Paint Pattern to Arrange");
    click_button(&mut ui, &mut s, "Paint Pattern to Arrange");
    assert_eq!(
        s.session.tapestry.transport_mode,
        TransportContext::SongMode
    );
    assert_eq!(s.session.tapestry.instances.len(), 2);
    assert_eq!(s.session.tapestry.instances[0].pattern, pat0_id);
    assert_eq!(s.session.tapestry.instances[1].pattern, pat0_id);

    let inst1_start = s
        .session
        .tapestry
        .tempo_map
        .beat_to_frame(s.session.tapestry.instances[1].start_beat, 0)
        as usize;
    let mut before_0 = vec![[0i16; 2]; 12_000];
    let mut before_1 = vec![[0i16; 2]; 12_000];
    moan_core::render(&s.session.tracks, 0, &mut before_0);
    moan_core::render(&s.session.tracks, inst1_start, &mut before_1);
    assert!(rms_range(&before_0) > 10.0);
    assert_eq!(
        before_0, before_1,
        "linked instances initially share identical audio"
    );

    // Edit shared pat0 at step 1 via pointer -> BOTH linked instances must update
    let (s1_x, s1_y) = TapestryUi::step_point(ui.rect(s.tapestry_ui.canvas), 0, 1, 64);
    click_point(&mut ui, &mut s, s1_x, s1_y);
    let mut linked_0 = vec![[0i16; 2]; 12_000];
    let mut linked_1 = vec![[0i16; 2]; 12_000];
    moan_core::render(&s.session.tracks, 0, &mut linked_0);
    moan_core::render(&s.session.tracks, inst1_start, &mut linked_1);
    assert_ne!(linked_0, before_0);
    assert_eq!(
        linked_0, linked_1,
        "shared pattern edit propagates to all linked instances"
    );

    // Select Instance 2 (`selected_instance_idx == 1`), Make Unique, and edit ONLY Unique instance
    s.tapestry_ui.selected_instance_idx = 1;
    click_button(&mut ui, &mut s, "Make Unique");
    let unique_pat_id = s.session.tapestry.instances[1].pattern;
    assert_eq!(s.session.tapestry.instances[0].pattern, pat0_id);
    assert_ne!(unique_pat_id, pat0_id);
    assert_eq!(s.session.tapestry.active_pattern, unique_pat_id);

    // Toggle step 5 on channel 0 ONLY in the new unique pattern
    let (s5_x, s5_y) = TapestryUi::step_point(ui.rect(s.tapestry_ui.canvas), 0, 5, 64);
    click_point(&mut ui, &mut s, s5_x, s5_y);

    let mut uniq_0 = vec![[0i16; 2]; 16_000];
    let mut uniq_1 = vec![[0i16; 2]; 16_000];
    moan_core::render(&s.session.tracks, 0, &mut uniq_0);
    moan_core::render(&s.session.tracks, inst1_start, &mut uniq_1);
    assert_ne!(
        uniq_0, uniq_1,
        "editing a Make Unique instance must diverge from the original sibling"
    );

    // Undo restores prior state without divergence at step 5
    assert!(s.session.undo());
    let mut undone_0 = vec![[0i16; 2]; 12_000];
    let mut undone_1 = vec![[0i16; 2]; 12_000];
    moan_core::render(&s.session.tracks, 0, &mut undone_0);
    moan_core::render(&s.session.tracks, inst1_start, &mut undone_1);
    assert_eq!(undone_0, linked_0);
    assert_eq!(undone_1, linked_1);

    // SceneMatrix Queue & Advance Bar places quantized pattern instances
    click_button(&mut ui, &mut s, "Queue Scene");
    assert!(s.session.scenes.queued.is_some());
    click_button(&mut ui, &mut s, "Advance Bar");
    assert!(s.session.scenes.queued.is_none());
    assert!(s.session.tapestry.instances.len() >= 3);

    // Swing slider updates TapestryBank::swing_pct and re-bakes delayed swing steps
    ui.set_value(s.tapestry_ui.swing_slider, 28.0);
    s.on(&mut ui, &Response::Changed(s.tapestry_ui.swing_slider))
        .unwrap();
    assert_eq!(s.session.tapestry.swing_pct, 28);

    // Clone Pattern & Split Channels create real patterns inside TapestryBank
    let pat_count_before = s.session.tapestry.patterns.len();
    click_button(&mut ui, &mut s, "Clone Pattern");
    assert_eq!(s.session.tapestry.patterns.len(), pat_count_before + 1);
    click_button(&mut ui, &mut s, "Split Channels");
    assert!(s.session.tapestry.patterns.len() > pat_count_before + 1);

    // Pointer click on Channel 0 Mute dot toggles muted state
    let r = ui.rect(s.tapestry_ui.canvas);
    let mute_x = r.x + 14.0 * r.w / tapestry_ui::RACK_WIDTH as f32;
    let mute_y =
        r.y + (tapestry_ui::HEADER_H as f32 + 11.0) * r.h / tapestry_ui::RACK_HEIGHT as f32;
    click_point(&mut ui, &mut s, mute_x, mute_y);
    assert!(s.session.tapestry.channels[0].muted);
    click_point(&mut ui, &mut s, mute_x, mute_y);
    assert!(!s.session.tapestry.channels[0].muted);

    // Busy lock permits switching to Tapestry workspace while blocking mutating actions
    let locked_project = s.session.encode_project().unwrap();
    s.set_busy(&mut ui, true);
    s.on(&mut ui, &Response::Command("moan.arrange".into()))
        .ok();
    s.on(&mut ui, &Response::Command("moan.tapestry".into()))
        .unwrap();
    assert_eq!(s.workspace, Workspace::Tapestry);
    s.on(&mut ui, &Response::Command("moan.library.sub808".into()))
        .unwrap();
    assert_eq!(s.session.encode_project().unwrap(), locked_project);
    s.set_busy(&mut ui, false);
}

#[test]
fn test_2_choke_groups_808_slides_piano_slide_lines_and_compose_tools() {
    // 1. Cut/Choke group verification: Open Hat chokes immediately when Closed Hat hits
    let mut bank = moan_core::pattern::TapestryBank::new();
    let kit = ALL_KITS.iter().find(|k| k.name == "808 Glide Kit").unwrap();
    let (mut ui, mut s) = studio();
    s.on(&mut ui, &Response::Command("moan.tapestry".into()))
        .unwrap();
    s.tapestry_ui.select_kit_by_name(&mut ui, kit.name);
    s.tapestry_ui.apply_kit_to_bank(kit, &mut bank).unwrap();
    let pat_id = bank.active_pattern;
    let open_ch = bank
        .channels
        .iter()
        .find(|c| c.name == "Sizzle Open Hat")
        .unwrap()
        .clone();
    let closed_ch = bank
        .channels
        .iter()
        .find(|c| c.name == "Tight Closed Hat")
        .unwrap()
        .clone();
    assert_eq!(open_ch.cut_group, 1);
    assert_eq!(closed_ch.cut_group, 1);

    // Schedule a 2-beat Open Hat at beat 0, and Closed Hat choking it at beat 0.5 (frame 24_000 at 120 BPM)
    {
        let pat = bank.pattern_mut(pat_id).unwrap();
        pat.toggle_step(open_ch.id, 0, open_ch.root_key, 110)
            .unwrap();
        if let Some(n) = pat.channel_notes_mut(open_ch.id).unwrap().first_mut() {
            n.length = Time::from_beats(2);
        }
        // step 2 at steps_per_beat=4 is beat 0.5
        pat.toggle_step(closed_ch.id, 2, closed_ch.root_key, 110)
            .unwrap();
    }

    let mut tracks = Vec::new();
    TapestryUi::bake_bank_to_tracks(&bank, &mut tracks).unwrap();
    let open_track = open_ch.target_track as usize;
    let mut pre_choke = vec![[0i16; 2]; 12_000];
    let mut post_choke = vec![[0i16; 2]; 12_000];
    moan_core::render(
        core::slice::from_ref(&tracks[open_track]),
        1_000,
        &mut pre_choke,
    );
    moan_core::render(
        core::slice::from_ref(&tracks[open_track]),
        24_000 + 256,
        &mut post_choke,
    );
    assert!(
        rms_range(&pre_choke) > 20.0,
        "open hat must sound before choke"
    );
    assert_eq!(
        post_choke.iter().map(|f| f[0].abs().max(f[1].abs())).max().unwrap(),
        0,
        "open hat must be completely silenced after 144-frame choke fade"
    );

    // 2. Piano Roll visual slide connector lines (0xffcc66) + 0xaff4cf notes + Sub 808 slide PCM
    let slide_pat = Pattern {
        frames: RATE * 2,
        bpm: 120,
        instrument: Instrument::Sub808,
        notes: vec![
            Note {
                start: 0,
                frames: RATE,
                key: 52,
                velocity: 110,
            },
            Note {
                start: RATE / 2,
                frames: RATE / 2,
                key: 64,
                velocity: 110,
            },
        ],
    };
    let piano_px = piano::render(Some(&slide_pat));
    assert!(piano_px.contains(&0xaff4cf), "note body pixels missing");
    assert!(
        piano_px.contains(&piano::SLIDE_COLOR),
        "diagonal slide connector pixels missing"
    );

    // Compare overlapping Sub 808 pattern vs non-overlapping Sub 808 pattern at overlap window
    let non_overlap_pat = Pattern {
        frames: RATE * 2,
        bpm: 120,
        instrument: Instrument::Sub808,
        notes: vec![
            Note {
                start: 0,
                frames: RATE / 2 - 200,
                key: 52,
                velocity: 110,
            },
            Note {
                start: RATE / 2,
                frames: RATE / 2,
                key: 64,
                velocity: 110,
            },
        ],
    };
    let pcm_slide = slide_pat.bake().unwrap();
    let pcm_step = non_overlap_pat.bake().unwrap();
    assert_ne!(
        &pcm_slide[RATE as usize / 2..RATE as usize / 2 + 1200],
        &pcm_step[RATE as usize / 2..RATE as usize / 2 + 1200],
        "overlapping Sub 808 glide must alter onset audio"
    );

    // 3. ComposeTool transformations via Weft UI ("Cycle Note Tool" + "Apply Note Tool")
    s.session = Session::default();
    let chord_pat = Pattern {
        frames: RATE * 2,
        bpm: 120,
        instrument: Instrument::TineKeys,
        notes: vec![
            Note {
                start: 0,
                frames: RATE / 2,
                key: 60,
                velocity: 100,
            },
            Note {
                start: 0,
                frames: RATE / 2,
                key: 64,
                velocity: 100,
            },
        ],
    };
    s.session.add_midi(chord_pat).unwrap();
    s.on(&mut ui, &Response::Command("moan.piano".into()))
        .unwrap();

    // Select Chop ("Legato" -> "Articulate" -> "Chop")
    while s.studio_rack_ui.compose_tool_name() != "Chop" {
        click_button(&mut ui, &mut s, "Cycle Note Tool");
    }
    click_button(&mut ui, &mut s, "Apply Note Tool");
    assert!(
        s.session
            .midi_clip()
            .unwrap()
            .midi_pattern()
            .unwrap()
            .notes
            .len()
            > 2,
        "Chop must slice long chord notes"
    );
    assert!(s.session.undo());

    // Select Strum -> chord onsets stagger
    while s.studio_rack_ui.compose_tool_name() != "Strum" {
        click_button(&mut ui, &mut s, "Cycle Note Tool");
    }
    s.on(&mut ui, &Response::Command("moan.compose.apply".into()))
        .unwrap();
    let strum_notes = &s.session.midi_clip().unwrap().midi_pattern().unwrap().notes;
    assert_eq!(strum_notes.len(), 2);
    assert!(strum_notes[1].start > strum_notes[0].start);
    assert!(s.session.undo());

    // Select Flam -> adds grace hits
    while s.studio_rack_ui.compose_tool_name() != "Flam" {
        click_button(&mut ui, &mut s, "Cycle Note Tool");
    }
    click_button(&mut ui, &mut s, "Apply Note Tool");
    assert!(
        s.session
            .midi_clip()
            .unwrap()
            .midi_pattern()
            .unwrap()
            .notes
            .len()
            >= 4
    );
    assert!(s.session.undo());

    // Select Legato -> extends short note to meet later onset
    s.session = Session::default();
    s.session
        .add_midi(Pattern {
            frames: RATE * 2,
            bpm: 120,
            instrument: Instrument::TineKeys,
            notes: vec![
                Note {
                    start: 0,
                    frames: 4000,
                    key: 60,
                    velocity: 96,
                },
                Note {
                    start: 24_000,
                    frames: 8000,
                    key: 64,
                    velocity: 96,
                },
            ],
        })
        .unwrap();
    while s.studio_rack_ui.compose_tool_name() != "Legato" {
        click_button(&mut ui, &mut s, "Cycle Note Tool");
    }
    click_button(&mut ui, &mut s, "Apply Note Tool");
    let legato_notes = &s.session.midi_clip().unwrap().midi_pattern().unwrap().notes;
    assert!(legato_notes[0].frames >= 23_500);
}

#[test]
fn test_3_all_seven_new_instruments_bake_distinct_audio() {
    let specs = [
        ("moan.library.sub808", "Sub 808", Instrument::Sub808),
        ("moan.library.drums", "Studio Drums", Instrument::DrumKit),
        ("moan.library.tine", "Tine Keys", Instrument::TineKeys),
        ("moan.library.grand", "Grand Piano", Instrument::ModalGrand),
        ("moan.library.guitar", "Electric Guitar", Instrument::PluckGuitar),
        ("moan.library.mallet", "Mallet Bells", Instrument::ModalMallet),
        ("moan.library.cross", "Cross Synth", Instrument::CrossSynth),
    ];

    let (mut ui, mut s) = studio();
    let mut slices: Vec<(Instrument, Vec<[i16; 2]>)> = Vec::new();

    for (cmd, btn_name, inst) in specs {
        s.on(&mut ui, &Response::Command(cmd.into())).unwrap();
        let trk = s.session.tracks.last().unwrap();
        let clip = &trk.clips[0];
        assert_eq!(clip.midi_pattern().unwrap().instrument, inst);
        let slice = clip.source_frames()[..8000].to_vec();
        assert!(
            rms_range(&slice) > 25.0,
            "{btn_name} rendered unexpectedly quiet audio"
        );
        // Verify button exists in Library UI too
        assert!(s.buttons.iter().any(|(_, n)| *n == btn_name));
        slices.push((inst, slice));
    }

    // All 7 instruments produce distinct timbres
    for i in 0..slices.len() {
        for j in (i + 1)..slices.len() {
            assert_ne!(
                slices[i].1, slices[j].1,
                "{:?} and {:?} rendered identical waveforms",
                slices[i].0, slices[j].0
            );
        }
    }
}

#[test]
fn test_4_pro_console_mixing_sidechain_eq_auto_cut_tape_and_mastering() {
    let (mut ui, mut s) = studio();
    click_button(&mut ui, &mut s, "Mix");

    // Track 0: loud key signal; Track 1: sustained pad; Track 2: silent bus catcher for Track 0
    let kick_key: Vec<[i16; 2]> = (0..24_000)
        .map(|i| {
            let v = if (i % 120) < 60 { 22_000 } else { -22_000 };
            [v, v]
        })
        .collect();
    let pad_bed: Vec<[i16; 2]> = (0..24_000)
        .map(|i| {
            let v = (sin_f32(i as f32 * 0.03) * 9000.0) as i16;
            [v, v]
        })
        .collect();
    let bus_holder = vec![[0i16; 2]; 24_000];

    s.session.add(kick_key).unwrap();
    s.session.add(pad_bed).unwrap();
    s.session.add(bus_holder).unwrap();

    // Mute bus output so Track 0 drives Sidechain Key while Master outputs ONLY Track 1!
    s.session.tracks[2].set_volume(0).unwrap();
    s.session.tracks[0].output_bus = Some(2);

    // Baseline Track 1 level before sidechain
    let mut dry_pad = vec![[0i16; 2]; 8192];
    s.session.render_processed(0, &mut dry_pad);
    let dry_rms = rms_range(&dry_pad);
    assert_eq!(s.session.last_gr_db_q8[1], 0);

    // Enable Sidechain Duck on Track 1 keyed from Track 0
    s.session.selected = 1;
    click_button(&mut ui, &mut s, "Enable Sidechain Duck");
    // Sidechain defaulted to track 0
    assert_eq!(s.session.tracks[1].sidechain_from, Some(0));
    assert!(s.session.tracks[1].sc_compressor.is_some());

    let mut ducked_pad = vec![[0i16; 2]; 8192];
    s.session.render_processed(0, &mut ducked_pad);
    s.refresh(&mut ui);
    let ducked_rms = rms_range(&ducked_pad);
    assert!(
        ducked_rms < dry_rms * 0.8,
        "sidechain compression did not duck Track 1: dry={dry_rms}, ducked={ducked_rms}"
    );
    assert!(
        s.session.last_gr_db_q8[1] > 0,
        "live GR dB should be positive under sidechain gain reduction"
    );
    assert_ne!(ui.text_of(s.console_fx_ui.gr_readout), "GR: 0.0 dB (M 0.0)");

    // Undo restores non-sidechained Track 1
    assert!(s.session.undo());
    assert!(s.session.tracks[1].sc_compressor.is_none());

    // 2. 6-Band Parametric EQ & "Auto Cut Resonance" on a resonant 2000 Hz tone
    s.session = Session::default();
    let resonant_sig: Vec<[i16; 2]> = (0..12_000)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let base = sin_f32(2.0 * core::f32::consts::PI * 150.0 * t) * 2500.0;
            let ring = sin_f32(2.0 * core::f32::consts::PI * 2000.0 * t) * 18_000.0;
            let sample = (base + ring).clamp(-32767.0, 32767.0) as i16;
            [sample, sample]
        })
        .collect();
    s.session.add(resonant_sig).unwrap();
    s.session.selected = 0;

    let mut raw_res = vec![[0i16; 2]; 4096];
    s.session.render_processed(0, &mut raw_res);
    let raw_rms = rms_range(&raw_res);

    click_button(&mut ui, &mut s, "Auto Cut Resonance");
    let eq = s.session.tracks[0].parametric_eq.as_ref().unwrap();
    let bell1 = eq.band(2).unwrap();
    assert!(bell1.enabled);
    assert!(
        (bell1.freq_hz - 2000.0).abs() < 350.0,
        "detected freq {} not near 2000Hz",
        bell1.freq_hz
    );
    assert!(bell1.gain_db < -1.0, "expected negative resonance cut");

    let mut cut_res = vec![[0i16; 2]; 4096];
    s.session.render_processed(0, &mut cut_res);
    assert!(
        rms_range(&cut_res) < raw_rms * 0.92,
        "Auto Cut Resonance did not attenuate resonant peak"
    );

    // Exercise HPF 80Hz, Gate ("Gated Snare Reverb"), Tape, BitCrush, Chorus, Mastering
    click_button(&mut ui, &mut s, "HPF 80Hz");
    assert!(
        s.session.tracks[0]
            .parametric_eq
            .as_ref()
            .unwrap()
            .band(0)
            .unwrap()
            .enabled
    );
    while !GATE_PRESETS[s.console_fx_ui.gate_preset_idx]
        .name
        .eq_ignore_ascii_case("Gated Snare Reverb")
    {
        click_button(&mut ui, &mut s, "Cycle Gate Preset");
    }
    click_button(&mut ui, &mut s, "Apply Gate Preset");
    assert!(s.session.tracks[0].gate.is_some());

    click_button(&mut ui, &mut s, "Apply Tape Preset");
    assert!(s.session.tracks[0].tape.is_some());
    click_button(&mut ui, &mut s, "Toggle Transient");
    assert!(s.session.tracks[0].transient.is_some());
    click_button(&mut ui, &mut s, "Toggle BitCrush");
    assert!(s.session.tracks[0].crusher.is_some());
    click_button(&mut ui, &mut s, "Toggle Chorus");
    assert!(s.session.tracks[0].chorus.is_some());

    // 3. Mastering Lookahead Limiter Ceiling (-1.0 dB = ~29204 sample magnitude)
    s.session = Session::default();
    let hot_wave: Vec<[i16; 2]> = (0..8192)
        .map(|i| {
            let v = (sin_f32(i as f32 * 0.08) * 30_000.0) as i16;
            [v, v]
        })
        .collect();
    for _ in 0..3 {
        s.session.add(hot_wave.clone()).unwrap();
    }
    s.session.set_master_volume(150).unwrap();

    // Balanced Clear preset uses -1.0 dB ceiling (~29,205 linear peak)
    assert_eq!(
        MASTERING_PRESETS[s.console_fx_ui.master_preset_idx].name,
        "Balanced Clear"
    );
    click_button(&mut ui, &mut s, "Apply Mastering Preset");
    assert!(s.session.mastering.is_some());

    let mut mastered = vec![[0i16; 2]; 4096];
    s.session.render_processed(0, &mut mastered);
    // Steady-state frames after 4 ms (192 frames) lookahead ramp must strictly honor -1.0 dB ceiling
    let max_sample = mastered[512..]
        .iter()
        .map(|f| f[0].abs().max(f[1].abs()))
        .max()
        .unwrap();
    assert!(
        max_sample <= 29_350,
        "lookahead limiter allowed peak {max_sample} above -1.0 dB ceiling"
    );
}

#[test]
fn test_5_smf_roundtrip_and_retrospective_riff_recovery() {
    // 1. Multi-track Format 1 960 PPQ SMF Export -> Re-Import Roundtrip
    let mut s = Session::default();
    let pat_bass = Pattern {
        frames: RATE * 2,
        bpm: 132,
        instrument: Instrument::Sub808,
        notes: vec![
            Note {
                start: 0,
                frames: 18_000,
                key: 36,
                velocity: 112,
            },
            Note {
                start: 24_000,
                frames: 18_000,
                key: 43,
                velocity: 108,
            },
        ],
    };
    let pat_keys = Pattern {
        frames: RATE * 2,
        bpm: 132,
        instrument: Instrument::TineKeys,
        notes: vec![Note {
            start: 12_000,
            frames: 20_000,
            key: 64,
            velocity: 96,
        }],
    };
    s.add_midi(pat_bass).unwrap();
    s.add_midi(pat_keys).unwrap();

    let mid_bytes = s.export_smf_bytes().unwrap();
    assert_eq!(&mid_bytes[0..4], b"MThd");
    assert_eq!(u16::from_be_bytes([mid_bytes[8], mid_bytes[9]]), 1);
    assert_eq!(u16::from_be_bytes([mid_bytes[12], mid_bytes[13]]), 960);

    let mut reimported = Session::default();
    reimported.import_smf_bytes(&mid_bytes).unwrap();
    assert_eq!(reimported.tracks.len(), 2);
    assert_eq!(
        reimported.tracks[0].clips[0]
            .midi_pattern()
            .unwrap()
            .instrument,
        Instrument::Sub808
    );
    assert_eq!(
        reimported.tracks[1].clips[0]
            .midi_pattern()
            .unwrap()
            .instrument,
        Instrument::TineKeys
    );
    assert_eq!(
        reimported.tracks[0].clips[0].midi_pattern().unwrap().bpm,
        132
    );
    assert_eq!(
        reimported.tracks[0].clips[0]
            .midi_pattern()
            .unwrap()
            .notes
            .len(),
        2
    );
    assert_eq!(
        reimported.tracks[0].clips[0]
            .midi_pattern()
            .unwrap()
            .notes[1]
            .key,
        43
    );

    let mut out = vec![[0i16; 2]; 4096];
    reimported.render_processed(0, &mut out);
    assert!(rms_range(&out) > 20.0);

    // 1-step Undo removes the imported arrangement cleanly
    assert!(reimported.undo());
    assert!(reimported.tracks.is_empty());

    // 2. Retrospective Keyboard Typing -> Recover Riff
    let (mut ui, mut st) = studio();
    st.on(&mut ui, &Response::Command("moan.tapestry".into()))
        .unwrap();
    ui.focus(None);

    // Shift up one octave ('x') and play 4 notes via real Weft keyboard events
    st.handle_event(&mut ui, Event::Key(Key::typed('x')));
    for ch in ['a', 'w', 's', 'e'] {
        st.handle_event(&mut ui, Event::Key(Key::typed(ch)));
    }
    assert_eq!(st.session.retrospective.len(), 4);

    click_button(&mut ui, &mut st, "Recover Riff");
    assert_eq!(st.session.tapestry.patterns.len(), 1);
    assert_eq!(st.session.tracks.len(), 1);
    let rec_pat = st.session.tracks[0].clips[0].midi_pattern().unwrap();
    assert_eq!(rec_pat.notes.len(), 4);
    assert!(st.session.undo());
    assert!(st.session.tracks.is_empty());
}

#[test]
fn test_6_moanprj_ext5_persistence_byte_exact_legacy_layout_and_zero_brands() {
    // 1. Legacy v1, v2, v3 remain 100% byte-identical and contain zero "EXT5" tags
    let mut s1 = Session::default();
    s1.add(vec![[100, -100]; 64]).unwrap();
    let v1 = s1.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes([v1[8], v1[9]]), 1);
    assert!(!v1.windows(4).any(|w| w == b"EXT5"));
    assert_eq!(Session::decode_project(&v1).unwrap().encode_project().unwrap(), v1);

    let mut s2 = Session::default();
    s2.add_midi(Pattern::preset(Instrument::SilkTriangle))
        .unwrap();
    let v2 = s2.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes([v2[8], v2[9]]), 2);
    assert!(!v2.windows(4).any(|w| w == b"EXT5"));
    assert_eq!(Session::decode_project(&v2).unwrap().encode_project().unwrap(), v2);

    let mut s3 = s2;
    s3.set_master_volume(80).unwrap();
    let v3 = s3.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes([v3[8], v3[9]]), 3);
    assert!(!v3.windows(4).any(|w| w == b"EXT5"));
    assert_eq!(Session::decode_project(&v3).unwrap().encode_project().unwrap(), v3);

    // 2. Full Phase 2 project with Tapestry, Sub 808, Pro FX, Sidechain, Mastering -> EXT5 exact roundtrip
    let (mut ui, mut s) = studio();
    s.on(&mut ui, &Response::Command("moan.tapestry".into()))
        .unwrap();
    click_button(&mut ui, &mut s, "Load Starter");
    click_button(&mut ui, &mut s, "Paint Pattern to Arrange");
    click_button(&mut ui, &mut s, "Queue Scene");

    // Also add a Sub 808 MIDI clip & configure Pro Mixing + Mastering on track 0/1
    if s.session.tracks.len() < 2 {
        s.session
            .add_midi(Pattern::preset(Instrument::Sub808))
            .unwrap();
    }
    s.session.selected = 0;
    click_button(&mut ui, &mut s, "Mix");
    click_button(&mut ui, &mut s, "Auto Cut Resonance");
    click_button(&mut ui, &mut s, "HPF 80Hz");
    click_button(&mut ui, &mut s, "Apply Gate Preset");
    click_button(&mut ui, &mut s, "Apply Tape Preset");
    click_button(&mut ui, &mut s, "Toggle Transient");
    click_button(&mut ui, &mut s, "Toggle BitCrush");
    click_button(&mut ui, &mut s, "Toggle Chorus");
    s.session.selected = 1;
    click_button(&mut ui, &mut s, "Enable Sidechain Duck");
    click_button(&mut ui, &mut s, "Cycle Post Send");
    click_button(&mut ui, &mut s, "Apply Mastering Preset");

    let ext5_bytes = s.session.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes([ext5_bytes[8], ext5_bytes[9]]), 4);
    assert!(ext5_bytes.windows(4).any(|w| w == b"EXT5"));

    let decoded = Session::decode_project(&ext5_bytes).unwrap();
    assert_eq!(decoded.encode_project().unwrap(), ext5_bytes);
    assert_eq!(
        decoded.tapestry.channels.len(),
        s.session.tapestry.channels.len()
    );
    assert_eq!(
        decoded.tapestry.patterns.len(),
        s.session.tapestry.patterns.len()
    );
    assert_eq!(
        decoded.tapestry.instances.len(),
        s.session.tapestry.instances.len()
    );
    assert_eq!(decoded.scenes.queued, s.session.scenes.queued);
    assert!(decoded.mastering.is_some());
    assert!(decoded.tracks[0].parametric_eq.is_some());
    assert!(decoded.tracks[0].gate.is_some());
    assert!(decoded.tracks[0].tape.is_some());
    assert!(decoded.tracks[0].transient.is_some());
    assert!(decoded.tracks[0].crusher.is_some());
    assert!(decoded.tracks[0].chorus.is_some());
    assert!(decoded.tracks[1].sc_compressor.is_some());

    // Render equivalence across encode/decode
    let mut a = vec![[0i16; 2]; 4096];
    let mut b = vec![[0i16; 2]; 4096];
    let mut s_orig = Session::decode_project(&ext5_bytes).unwrap();
    let mut s_copy = Session::decode_project(&ext5_bytes).unwrap();
    s_orig.render_processed(0, &mut a);
    s_copy.render_processed(0, &mut b);
    assert_eq!(a, b);

    // Truncated / corrupt EXT5 never panics and rejects cleanly
    assert!(Session::decode_project(&ext5_bytes[..ext5_bytes.len() - 3]).is_err());

    // 3. Window geometry sanity at (1264, 730) and (624, 410) + zero forbidden brand names
    for (w, h) in [(1264, 730), (624, 410)] {
        let (mut ui, mut win_s) = studio_sized(w, h);
        win_s.on(&mut ui, &Response::Command("moan.tapestry".into()))
            .unwrap();
        ui.layout();
        let mut px = vec![0u32; (w * h) as usize];
        assert!(ui.paint(&mut px).is_some());
        win_s.on(&mut ui, &Response::Command("moan.mixer".into()))
            .unwrap();
        ui.layout();
        assert!(ui.paint(&mut px).is_some());
    }

    let forbidden = [
        "FL Studio",
        "Peel",
        "Suno",
        "Ableton",
        "Logic",
        "VCV",
        "ACE-Step",
    ];
    for (_, name) in &s.buttons {
        for brand in forbidden {
            assert!(
                !name.to_ascii_lowercase().contains(&brand.to_ascii_lowercase()),
                "forbidden brand {brand} in button {name}"
            );
        }
    }
    for k in ALL_KITS {
        for brand in forbidden {
            assert!(!k.name.contains(brand));
        }
    }
    for st in ALL_SONG_STARTERS {
        for brand in forbidden {
            assert!(!st.name.contains(brand));
        }
    }
    for p in TAPE_PRESETS {
        for brand in forbidden {
            assert!(!p.name.contains(brand));
        }
    }
}
