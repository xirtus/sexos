// SPDX-License-Identifier: GPL-3.0-only
use moan::{
    waveform::{self, View},
    Action, Studio,
};
use moan_core::RATE;
use weft::{
    input::{Button, Event, Pointer, PointerKind},
    Response, Strings, Theme, Ui,
};
fn studio() -> (Ui, Studio) {
    let mut ui = Ui::new(640, 400, Theme::dark(), Strings::new("en"));
    let mut s = Studio::build(&mut ui);
    s.session
        .add(
            (0..RATE * 2)
                .map(|i| [if i % 120 < 60 { 10000 } else { -10000 }; 2])
                .collect(),
        )
        .unwrap();
    s.refresh(&mut ui);
    ui.layout();
    (ui, s)
}
fn reveal(ui: &mut Ui, s: &Studio) {
    ui.focus(Some(s.waveform));
    ui.layout();
}
fn pointer(ui: &mut Ui, s: &mut Studio, x: f32, y: f32, kind: PointerKind) {
    s.handle_event(ui, Event::Pointer(Pointer { x, y, kind }));
}
#[test]
fn waveform_contains_actual_audio_selection_and_playhead_pixels() {
    let (ui, s) = studio();
    let (height, pixels) = waveform::render(&s.session, View::default());
    assert_eq!(height, waveform::HEADER + waveform::ROW);
    assert!(pixels.contains(&(((0x63D4CD & 0xFEFEFE) >> 1) + 0x263A3C)));
    assert!(pixels.contains(&0xFFCC66));
    assert!(pixels.iter().any(|c| *c != pixels[0]));
    drop(ui);
}
#[test]
fn pointer_selects_time_range_and_cut_undo_edit_actual_pcm() {
    let (mut ui, mut s) = studio();
    reveal(&mut ui, &s);
    let r = ui.rect(s.waveform);
    let y = r.y + waveform::HEADER as f32 + 10.;
    pointer(
        &mut ui,
        &mut s,
        r.x + 70.,
        y,
        PointerKind::Down(Button::Left),
    );
    pointer(&mut ui, &mut s, r.x + 210., y, PointerKind::Move);
    pointer(
        &mut ui,
        &mut s,
        r.x + 210.,
        y,
        PointerKind::Up(Button::Left),
    );
    assert_eq!(s.session.from, s.view.frame(70));
    assert_eq!(s.session.to, s.view.frame(210));
    let before = s.session.duration();
    let id = s
        .buttons
        .iter()
        .find(|(_, name)| *name == "Cut range")
        .unwrap()
        .0;
    s.on(&mut ui, &Response::Clicked(id)).unwrap();
    assert_eq!(s.session.tracks[0].clips.len(), 2);
    assert_eq!(s.session.duration(), before);
    s.session.undo();
    assert_eq!(s.session.tracks[0].clips.len(), 1);
}
#[test]
fn drag_range_clamps_to_view_and_busy_transport_rejects_pointer_edits() {
    let (mut ui, mut s) = studio();
    reveal(&mut ui, &s);
    let r = ui.rect(s.waveform);
    let y = r.y + waveform::HEADER as f32 + 10.;
    pointer(
        &mut ui,
        &mut s,
        r.x + 70.,
        y,
        PointerKind::Down(Button::Left),
    );
    pointer(
        &mut ui,
        &mut s,
        r.x - 1000.,
        y,
        PointerKind::Up(Button::Left),
    );
    assert_eq!(s.session.from, 0);
    assert!(s.session.to > 0);
    s.set_busy(&mut ui, true);
    let old = (s.session.cursor, s.session.from, s.session.to);
    pointer(
        &mut ui,
        &mut s,
        r.x + 200.,
        y,
        PointerKind::Down(Button::Left),
    );
    assert_eq!((s.session.cursor, s.session.from, s.session.to), old);
}
#[test]
fn file_actions_require_standard_discard_confirmation_and_cancel_preserves_work() {
    let (mut ui, mut s) = studio();
    assert_eq!(
        s.on(&mut ui, &Response::Command("moan.new-project".into()))
            .unwrap(),
        None
    );
    assert_eq!(s.session.tracks.len(), 1);
    // The UI presents a real modal instead of destroying the dirty project.
    assert!(!ui.find_kind("button").is_empty());
    let result = s.request_close(&mut ui);
    assert!(result.is_none());
    assert_eq!(s.session.tracks.len(), 1);
    for response in ui.handle(Event::Key(weft::Key::new(weft::input::code::ESCAPE))) {
        s.on(&mut ui, &response).unwrap();
    }
    s.session.mark_saved();
    assert_eq!(s.request_close(&mut ui), Some(Action::Quit));
}
#[test]
fn fit_zoom_and_export_format_controls_use_standard_weft_buttons() {
    let (mut ui, mut s) = studio();
    for name in [
        "Fit audio",
        "Zoom in",
        "Zoom out",
        "Later",
        "Earlier",
        "FLAC",
        "MP3",
        "WAV",
    ] {
        let id = s.buttons.iter().find(|(_, n)| *n == name).unwrap().0;
        s.on(&mut ui, &Response::Clicked(id)).unwrap();
    }
    assert_eq!(s.export_format, 0);
    assert!(s.view.span >= RATE as usize / 10);
    let mut pixels = vec![0; 640 * 400];
    assert!(ui.paint(&mut pixels).is_some());
}
