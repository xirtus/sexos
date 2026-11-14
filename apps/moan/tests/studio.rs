// SPDX-License-Identifier: GPL-3.0-only
use moan::{read_wav, wav_header, Action, Session, Studio};
use moan_core::{render, RATE};
use weft::{
    input::{Button, Event, Pointer, PointerKind},
    Response, Strings, Theme, Ui,
};
#[test]
fn cut_split_undo_redo_preserve_pcm_and_positions() {
    let mut s = Session::default();
    s.add(vec![[100; 2]; 100]).unwrap();
    s.cursor = 50;
    s.split().unwrap();
    assert_eq!(s.tracks[0].clips.len(), 2);
    s.from = 25;
    s.to = 75;
    s.cut().unwrap();
    let mut out = [[0; 2]; 100];
    render(&s.tracks, 0, &mut out);
    assert!(out[..25].iter().all(|f| *f == [100; 2]));
    assert!(out[25..75].iter().all(|f| *f == [0; 2]));
    assert!(out[75..].iter().all(|f| *f == [100; 2]));
    assert!(s.undo());
    render(&s.tracks, 0, &mut out);
    assert_eq!(out, [[100; 2]; 100]);
    assert!(s.redo());
    render(&s.tracks, 0, &mut out);
    assert!(out[25..75].iter().all(|f| *f == [0; 2]));
    s.gain(50).unwrap();
    assert!(!s.redo());
    assert!(s.undo());
    assert_eq!(s.tracks[0].clips[0].gain(), 100);
}
#[test]
fn owned_clips_survive_edit_history_and_source_scope() {
    let mut s = Session::default();
    {
        let pcm = vec![[123, -456]; 10];
        s.add(pcm).unwrap();
    }
    s.cursor = 5;
    s.split().unwrap();
    s.undo();
    s.redo();
    let mut out = [[0; 2]; 10];
    render(&s.tracks, 0, &mut out);
    assert_eq!(out, [[123, -456]; 10]);
}
#[test]
fn invalid_edits_and_track_limits_do_not_change_project() {
    let mut s = Session::default();
    assert!(s.split().is_err());
    assert!(s.add(vec![]).is_err());
    for _ in 0..8 {
        s.add(vec![[1; 2]; 3]).unwrap();
    }
    assert!(s.add(vec![[1; 2]; 3]).is_err());
    assert_eq!(s.tracks.len(), 8);
    s.cursor = 100;
    assert!(s.split().is_err());
    s.from = 3;
    s.to = 2;
    assert!(s.cut().is_err());
    assert!(s.gain(401).is_err());
    assert_eq!(s.tracks[7].clips[0].gain(), 100);
}
#[test]
fn wav_roundtrip_and_all_truncations_fail_safely() {
    let mut data = wav_header(3).unwrap().to_vec();
    for sample in [0i16, 1, -32768, 32767, -123, 456] {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    assert_eq!(
        read_wav(&data).unwrap(),
        vec![[0, 1], [-32768, 32767], [-123, 456]]
    );
    for n in 0..data.len() {
        assert!(
            read_wav(&data[..n]).is_err(),
            "accepted truncated WAV at {n}"
        );
    }
    let mut corrupt = data.clone();
    corrupt[24..28].copy_from_slice(&44100u32.to_le_bytes());
    assert!(read_wav(&corrupt).is_err());
    corrupt = data.clone();
    corrupt[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_wav(&corrupt).is_err());
    assert!(wav_header(usize::MAX).is_err());
}
fn click(ui: &mut Ui, s: &mut Studio, name: &str) {
    ui.layout();
    let id = s.buttons.iter().find(|(_, n)| *n == name).unwrap().0;
    let p = ui.rect(id).center();
    for kind in [
        PointerKind::Move,
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        for r in ui.handle(Event::Pointer(Pointer {
            x: p.x,
            y: p.y,
            kind,
        })) {
            s.on(ui, &r).unwrap();
        }
    }
}
#[test]
fn real_weft_pointer_edits_and_commands_build_a_project() {
    let mut ui = Ui::new(720, 600, Theme::dark(), Strings::new("en"));
    let mut s = Studio::build(&mut ui);
    click(&mut ui, &mut s, "Tone");
    click(&mut ui, &mut s, "Tone");
    assert_eq!(s.session.tracks.len(), 2);
    ui.set_text(s.position, "1");
    click(&mut ui, &mut s, "Split");
    assert_eq!(s.session.tracks[1].clips.len(), 2);
    ui.set_text(s.from, "0.25");
    ui.set_text(s.to, "0.75");
    click(&mut ui, &mut s, "Cut range");
    assert_eq!(s.session.tracks[1].clips.len(), 3);
    s.on(&mut ui, &Response::Command("moan.undo".into()))
        .unwrap();
    assert_eq!(s.session.tracks[1].clips.len(), 2);
    assert_eq!(
        s.on(&mut ui, &Response::Command("moan.play".into()))
            .unwrap(),
        Some(Action::Play)
    );
    let mut pixels = vec![0; 720 * 600];
    assert!(ui.paint(&mut pixels).is_some());
    assert!(pixels.iter().any(|p| *p != pixels[0]));
}
#[test]
fn busy_transport_blocks_edits_and_invalid_positions_are_reported() {
    let mut ui = Ui::new(640, 480, Theme::dark(), Strings::new("en"));
    let mut s = Studio::build(&mut ui);
    s.session.add(vec![[100; 2]; RATE as usize]).unwrap();
    s.set_busy(&mut ui, true);
    s.on(&mut ui, &Response::Command("moan.undo".into()))
        .unwrap();
    assert_eq!(s.session.tracks.len(), 1);
    s.set_busy(&mut ui, false);
    ui.set_text(s.position, "NaN");
    assert!(s
        .on(&mut ui, &Response::Command("moan.play".into()))
        .is_err());
}

#[test]
fn edit_controls_fit_the_initial_guest_window() {
    let mut ui = Ui::new(640, 400, Theme::dark(), Strings::new("en"));
    let s = Studio::build(&mut ui);
    ui.layout();
    for name in ["Play", "Tone", "Split", "Undo"] {
        let id = s.buttons.iter().find(|(_, n)| *n == name).unwrap().0;
        let p = ui.rect(id).center();
        assert!(
            p.x > 0.0 && p.x < 640.0 && p.y > 0.0 && p.y < 400.0,
            "{name} at {p:?}"
        );
    }
    assert!(ui.rect(s.to).right() <= 640.0);
}

#[test]
fn empty_track_has_its_own_selectable_timeline_row() {
    let mut ui = Ui::new(640, 400, Theme::dark(), Strings::new("en"));
    let mut s = Studio::build(&mut ui);
    s.session.add(vec![[1; 2]; 100]).unwrap();
    s.session.add(vec![[2; 2]; 100]).unwrap();
    s.session.selected = 0;
    s.session.from = 0;
    s.session.to = 100;
    s.session.cut().unwrap();
    s.refresh(&mut ui);
    assert_eq!(ui.selected_row(s.timeline), Some(0));
    ui.select_row(s.timeline, Some(1));
    s.on(&mut ui, &Response::Selected(s.timeline)).unwrap();
    assert_eq!(s.session.selected, 1);
}
