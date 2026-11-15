use moan::{piano, Studio};
use moan_core::midi::Instrument;
use weft::{Button, Event, Pointer, PointerKind, Response, Strings, Theme, Ui};
fn studio() -> (Ui, Studio) {
    let mut ui = Ui::new(1264, 730, Theme::dark(), Strings::new("en"));
    let s = Studio::build(&mut ui);
    (ui, s)
}
#[test]
fn original_library_builds_editable_regions_and_piano_pointer_changes_pcm() {
    let (mut ui, mut s) = studio();
    s.on(&mut ui, &Response::Command("moan.library.keys".into()))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    assert_eq!(
        s.session
            .midi_clip()
            .unwrap()
            .midi_pattern()
            .unwrap()
            .instrument,
        Instrument::SilkTriangle
    );
    s.on(&mut ui, &Response::Command("moan.clear-notes".into()))
        .unwrap();
    s.on(&mut ui, &Response::Command("moan.piano".into()))
        .unwrap();
    ui.layout();
    let r = ui.rect(s.piano);
    let key = 69;
    let y = (piano::LOW + piano::ROWS as u8 - 1 - key) as usize * piano::ROW + 3;
    let point = (
        r.x + (piano::KEYBOARD + 10) as f32 * r.w / piano::WIDTH as f32,
        r.y + y as f32,
    );
    for kind in [
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        s.handle_event(
            &mut ui,
            Event::Pointer(Pointer {
                x: point.0,
                y: point.1,
                kind,
            }),
        );
    }
    let p = s.session.midi_clip().unwrap();
    assert_eq!(p.midi_pattern().unwrap().notes.len(), 1);
    assert_eq!(p.midi_pattern().unwrap().notes[0].key, key);
    assert!(p.source_frames().iter().any(|f| f[0] != 0));
    let edited = s.session.encode_project().unwrap();
    assert!(s.session.undo());
    assert!(s
        .session
        .midi_clip()
        .unwrap()
        .midi_pattern()
        .unwrap()
        .notes
        .is_empty());
    assert!(s.session.redo());
    assert_eq!(s.session.encode_project().unwrap(), edited);
    let pixels = piano::render(s.session.midi_clip().and_then(|c| c.midi_pattern()));
    assert!(pixels.contains(&0xaff4cf));
}
#[test]
fn busy_library_and_piano_cannot_mutate_the_transport_project() {
    let (mut ui, mut s) = studio();
    s.set_busy(&mut ui, true);
    s.on(&mut ui, &Response::Command("moan.library.kit".into()))
        .unwrap();
    assert!(s.session.tracks.is_empty());
    s.set_busy(&mut ui, false);
    s.on(&mut ui, &Response::Command("moan.library.kit".into()))
        .unwrap();
    let before = s.session.encode_project().unwrap();
    s.set_busy(&mut ui, true);
    s.on(&mut ui, &Response::Command("moan.clear-notes".into()))
        .unwrap();
    assert_eq!(s.session.encode_project().unwrap(), before);
}
#[test]
fn midi_shortcuts_use_real_weft_key_dispatch() {
    let (mut ui, mut s) = studio();
    for response in ui.handle(Event::Key(weft::Key::ctrl(0x17).shift())) {
        s.on(&mut ui, &response).unwrap();
    }
    assert_eq!(s.session.tracks.len(), 1);
    for response in ui.handle(Event::Key(
        weft::Key::ctrl(weft::input::code::BACKSPACE).shift(),
    )) {
        s.on(&mut ui, &response).unwrap();
    }
    assert!(s
        .session
        .midi_clip()
        .unwrap()
        .midi_pattern()
        .unwrap()
        .notes
        .is_empty());
    for response in ui.handle(Event::Key(weft::Key::ctrl(0x19))) {
        s.on(&mut ui, &response).unwrap();
    }
    assert_eq!(ui.focused(), Some(s.piano));
    s.session
        .edit_midi(|p| {
            p.notes.push(moan_core::midi::Note {
                start: 0,
                frames: 12000,
                key: 57,
                velocity: 96,
            })
        })
        .unwrap();
    for code in [0x22, weft::input::code::UP, 0x2f] {
        for response in ui.handle(Event::Key(weft::Key::ctrl(code).shift())) {
            s.on(&mut ui, &response).unwrap();
        }
    }
    let note = &s.session.midi_clip().unwrap().midi_pattern().unwrap().notes[0];
    assert_eq!((note.frames, note.key, note.velocity), (24000, 69, 48));
}
#[test]
fn workspace_switches_preserve_project_selection_and_undo() {
    let (mut ui, mut s) = studio();
    s.on(&mut ui, &Response::Command("moan.library.keys".into()))
        .unwrap();
    let original = s.session.encode_project().unwrap();
    for name in [
        "Mix",
        "Piano Roll",
        "Instruments",
        "Arrange",
        "Library",
        "Library",
    ] {
        let id = s.buttons.iter().find(|(_, n)| *n == name).unwrap().0;
        s.on(&mut ui, &Response::Clicked(id)).unwrap();
        assert_eq!(s.session.encode_project().unwrap(), original);
    }
    assert!(s.session.undo());
    assert!(
        s.session.tracks.is_empty(),
        "view switches must not add musical undo entries"
    );
}
#[test]
fn hidden_panels_take_no_space_and_real_track_headers_align_at_both_sizes() {
    for (width, height) in [(1264, 730), (640, 400)] {
        let mut ui = Ui::new(
            width,
            height,
            Theme::dark().with_density(weft::Density::Compact),
            Strings::new("en"),
        );
        let mut s = Studio::build(&mut ui);
        s.on(&mut ui, &Response::Command("moan.library.keys".into()))
            .unwrap();
        ui.layout();
        let r = ui.rect(s.waveform);
        assert!(r.right() <= width as f32 && r.x >= 142.0, "{r:?}");
        let header = ui.find_text("1 Silk Keys").unwrap();
        let h = ui.rect(header);
        assert!(
            (h.y - r.y - moan::waveform::HEADER as f32).abs() < 0.1,
            "header {h:?}, lane {r:?}"
        );
        assert!((h.h - moan::waveform::ROW as f32).abs() < 0.1);
        if width >= 1000 {
            let library = s.buttons.iter().find(|(_, n)| *n == "Library").unwrap().0;
            s.on(&mut ui, &Response::Clicked(library)).unwrap();
            ui.layout();
            assert!((r.x - ui.rect(s.waveform).x - 156.0).abs() < 0.1);
        }
        for name in ["Play", "Pause", "Stop", "Record"] {
            let id = s.buttons.iter().find(|(_, n)| *n == name).unwrap().0;
            let rect = ui.rect(id);
            assert!(
                rect.w > 0.0
                    && rect.h > 0.0
                    && rect.right() <= width as f32
                    && rect.bottom() <= height as f32,
                "{name} {rect:?}"
            );
        }
    }
}
#[test]
fn pitch_names_are_visible_musical_octaves() {
    assert_eq!(piano::note_name(57), "A3");
    assert_eq!(piano::note_name(69), "A4");
    assert_eq!(piano::note_name(0), "C-1");
    assert_eq!(piano::note_name(127), "G9");
}
#[test]
fn sequencer_pointer_edits_same_notes_velocity_gate_cache_and_persistence() {
    let (mut ui, mut s) = studio();
    s.on(&mut ui, &Response::Command("moan.library.keys".into()))
        .unwrap();
    s.on(&mut ui, &Response::Command("moan.clear-notes".into()))
        .unwrap();
    s.on(&mut ui, &Response::Command("moan.steps".into()))
        .unwrap();
    ui.layout();
    let rect = ui.rect(s.steps);
    assert!(
        rect.x >= 0.0 && rect.right() <= 1264.0 && rect.bottom() <= 730.0,
        "{rect:?}"
    );
    for index in [1, 3, 7, 13] {
        let x = moan::step::LABEL + (index * 2 + 1) * moan::step::GRID / 32;
        let y = moan::step::HEADER + 3 * moan::step::ROW + moan::step::ROW / 2;
        for kind in [
            PointerKind::Down(Button::Left),
            PointerKind::Up(Button::Left),
        ] {
            s.handle_event(
                &mut ui,
                Event::Pointer(Pointer {
                    x: rect.x + x as f32 * rect.w / moan::step::WIDTH as f32,
                    y: rect.y + y as f32,
                    kind,
                }),
            );
        }
    }
    let pattern = s.session.midi_clip().unwrap().midi_pattern().unwrap();
    assert_eq!(pattern.notes.len(), 4);
    for (note, index) in pattern.notes.iter().zip([1, 3, 7, 13]) {
        assert_eq!(
            (note.start, note.frames, note.key, note.velocity),
            (index * 12000, 9000, 60, 96)
        );
    }
    let bytes = s.session.encode_project().unwrap();
    let loaded = moan::Session::decode_project(&bytes).unwrap();
    assert_eq!(loaded.encode_project().unwrap(), bytes);
    let original = bytes.clone();
    for command in ["moan.piano", "moan.steps", "moan.mixer", "moan.steps"] {
        s.on(&mut ui, &Response::Command(command.into())).unwrap();
    }
    assert_eq!(s.session.encode_project().unwrap(), original);
    assert!(s.session.undo());
    assert_eq!(
        s.session
            .midi_clip()
            .unwrap()
            .midi_pattern()
            .unwrap()
            .notes
            .len(),
        3
    );
    assert!(s.session.redo());
    assert_eq!(s.session.encode_project().unwrap(), original);
    s.set_busy(&mut ui, true);
    let before = s.session.encode_project().unwrap();
    s.handle_event(
        &mut ui,
        Event::Pointer(Pointer {
            x: rect.x + 100.0,
            y: rect.y + 100.0,
            kind: PointerKind::Down(Button::Left),
        }),
    );
    assert_eq!(s.session.encode_project().unwrap(), before);
}

#[test]
fn new_note_velocity_slider_preserves_earlier_note_velocities() {
    let (mut ui, mut s) = studio();
    s.on(&mut ui, &Response::Command("moan.library.keys".into()))
        .unwrap();
    s.on(&mut ui, &Response::Command("moan.clear-notes".into()))
        .unwrap();
    s.on(&mut ui, &Response::Command("moan.steps".into()))
        .unwrap();
    ui.layout();
    let rect = ui.rect(s.steps);
    let tap = |ui: &mut Ui, s: &mut Studio, index: usize| {
        let x = moan::step::LABEL + (index * 2 + 1) * moan::step::GRID / 32;
        let y = moan::step::HEADER + 3 * moan::step::ROW + moan::step::ROW / 2;
        for kind in [
            PointerKind::Down(Button::Left),
            PointerKind::Up(Button::Left),
        ] {
            s.handle_event(
                ui,
                Event::Pointer(Pointer {
                    x: rect.x + x as f32 * rect.w / moan::step::WIDTH as f32,
                    y: rect.y + y as f32,
                    kind,
                }),
            );
        }
    };
    tap(&mut ui, &mut s, 1);
    // The slider beside the Step editor's Velocity label; use real pointer input
    // and native Home/PageUp/Right keyboard dispatch, not a private model setter.
    let panel = ui.parent(s.steps).unwrap();
    let slider = ui
        .find_kind("slider")
        .into_iter()
        .filter(|id| ui.parent(*id).and_then(|p| ui.parent(p)) == Some(panel))
        .last()
        .unwrap();
    let point = ui.rect(slider).center();
    for kind in [
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        for response in ui.handle(Event::Pointer(Pointer {
            x: point.x,
            y: point.y,
            kind,
        })) {
            s.on(&mut ui, &response).unwrap();
        }
    }
    let mut keys = vec![weft::input::code::HOME];
    keys.extend([weft::input::code::PAGE_UP; 4]);
    keys.extend([weft::input::code::RIGHT; 7]);
    for code in keys {
        for response in ui.handle(Event::Key(weft::Key::new(code))) {
            s.on(&mut ui, &response).unwrap();
        }
    }
    assert_eq!(ui.value(slider), 48.0);
    tap(&mut ui, &mut s, 3);
    let notes = &s.session.midi_clip().unwrap().midi_pattern().unwrap().notes;
    assert_eq!(
        notes.iter().map(|n| n.velocity).collect::<Vec<_>>(),
        vec![96, 48]
    );
}
#[test]
fn offline_job_locks_project_replacement_until_retirement_or_explicit_memory_retry() {
    let (mut ui, mut studio) = studio();
    studio
        .on(&mut ui, &Response::Command("moan.library.keys".into()))
        .unwrap();
    let before = studio.session.encode_project().unwrap();
    studio.plugin_status(&mut ui, moan::plugins::Status::Rendering);
    studio.set_busy(&mut ui, true);
    for command in [
        "moan.new-project",
        "moan.open-project",
        "moan.save-project",
        "moan.undo",
        "moan.play",
    ] {
        assert_eq!(
            studio
                .on(&mut ui, &Response::Command(command.into()))
                .unwrap(),
            None
        );
    }
    assert_eq!(studio.session.encode_project().unwrap(), before);
    let render = studio
        .buttons
        .iter()
        .find(|(_, name)| *name == "Offline render")
        .unwrap()
        .0;
    let cancel = studio
        .buttons
        .iter()
        .find(|(_, name)| *name == "Cancel render")
        .unwrap()
        .0;
    assert!(!ui.is_enabled(render));
    assert!(ui.is_enabled(cancel));
    assert_eq!(
        studio.on(&mut ui, &Response::Clicked(cancel)).unwrap(),
        Some(moan::Action::CancelPlugin)
    );
    studio.plugin_status(&mut ui, moan::plugins::Status::WaitingForMemory);
    assert!(ui.is_enabled(render));
    assert_eq!(ui.text_of(render), "Retry result");
    assert_eq!(
        studio.on(&mut ui, &Response::Clicked(render)).unwrap(),
        Some(moan::Action::RenderPlugin)
    );
    studio.plugin_status(&mut ui, moan::plugins::Status::Idle);
    studio.set_busy(&mut ui, false);
    assert!(!ui.is_enabled(cancel));
    assert_eq!(studio.session.encode_project().unwrap(), before);
}
#[test]
fn eight_real_strips_and_master_are_reachable_in_both_mixer_sizes() {
    for (width, height) in [(1264, 730), (624, 410)] {
        let mut ui = Ui::new(
            width,
            height,
            Theme::dark().with_density(weft::Density::Compact),
            Strings::new("en"),
        );
        let mut s = Studio::build(&mut ui);
        for _ in 0..8 {
            s.session.add(vec![[1000, -1000]; 100]).unwrap();
        }
        s.refresh(&mut ui);
        let mix = s.buttons.iter().find(|(_, name)| *name == "Mix").unwrap().0;
        s.on(&mut ui, &Response::Clicked(mix)).unwrap();
        ui.layout();
        let label = ui.find_text("MASTER").unwrap();
        let strip = ui.parent(label).unwrap();
        let level = ui
            .find_kind("slider")
            .into_iter()
            .find(|id| ui.parent(*id) == Some(strip))
            .unwrap();
        let master = s
            .buttons
            .iter()
            .find(|(_, name)| *name == "Master")
            .unwrap()
            .0;
        s.on(&mut ui, &Response::Clicked(master)).unwrap();
        ui.layout();
        let rect = ui.rect(level);
        assert!(
            rect.w > 0.0
                && rect.x >= 0.0
                && rect.right() <= width as f32
                && rect.y >= 0.0
                && rect.bottom() <= height as f32,
            "{width}x{height} {rect:?}"
        );
        assert_eq!(ui.focused(), Some(level));
        assert_eq!(s.session.tracks.len(), 8);
        s.set_busy(&mut ui, true);
        s.on(&mut ui, &Response::Clicked(master)).unwrap();
        assert_ne!(ui.focused(), Some(level));
    }
}
#[test]
fn playback_allows_live_mix_but_blocks_source_edits_and_groups_one_pass_undo() {
    let (mut ui, mut s) = studio();
    s.session.add(vec![[12000, -24000]; 100]).unwrap();
    s.session.mark_saved();
    s.refresh(&mut ui);
    let original = s.session.encode_project().unwrap();
    s.set_activity(&mut ui, true, true);
    let master_label = ui.find_text("MASTER").unwrap();
    let master_strip = ui.parent(master_label).unwrap();
    let master = ui
        .find_kind("slider")
        .into_iter()
        .find(|id| ui.parent(*id) == Some(master_strip))
        .unwrap();
    assert!(ui.is_enabled(master));
    ui.set_value(master, 50.0);
    s.on(&mut ui, &Response::Changed(master)).unwrap();
    assert_eq!(s.session.master_volume(), 50);
    ui.set_value(master, 60.0);
    s.on(&mut ui, &Response::Changed(master)).unwrap();
    assert_eq!(s.session.master_volume(), 60);
    for cmd in ["moan.library.keys", "moan.new-project", "moan.undo"] {
        assert_eq!(s.on(&mut ui, &Response::Command(cmd.into())).unwrap(), None);
    }
    assert_eq!(s.session.tracks.len(), 1);
    assert!(s.session.is_dirty());
    s.set_activity(&mut ui, false, false);
    assert!(s.session.undo());
    assert_eq!(
        s.session.encode_project().unwrap(),
        original,
        "entire live pass is one bounded mix checkpoint"
    );
    s.set_activity(&mut ui, true, false);
    assert!(!ui.is_enabled(master));
}
