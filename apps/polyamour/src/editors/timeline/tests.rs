// SPDX-License-Identifier: GPL-3.0-only
use crate::input::{Button, Input, Key, Mods};
use crate::App;

fn click(app: &mut App, x: i32, y: i32) {
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Up { x, y, button: Button::Left });
}

fn timeline_rect(app: &App) -> crate::draw::Rect {
    let ws = &app.workspaces[app.active_ws];
    ws.areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::Timeline).unwrap().rect
}

#[test]
fn i_keys_and_frames_interpolate() {
    let mut app = App::new(1280, 720);
    app.frame(Input::Tick);
    let cube = app.doc.active().unwrap();
    let name = app.doc.scene.objects[cube].name.clone();
    let tr = timeline_rect(&app);
    app.frame(Input::Move { x: tr.x + 300, y: tr.y + 60 });
    app.frame(Input::key(Key::Char('i')));
    app.doc.frame = 20;
    app.frame(Input::Tick);
    app.doc.scene.objects[cube].transform.location.x = 6.0;
    app.frame(Input::key(Key::Char('i')));
    assert_eq!(app.doc.anim.key_frames(&name), vec![1.0, 20.0]);
    app.doc.frame = 10;
    app.frame(Input::Tick);
    let x = app.doc.scene.objects[cube].transform.location.x;
    assert!(x > 1.0 && x < 5.0, "interpolated x {x}");
    // Up/Down jump between keys.
    app.frame(Input::key(Key::Up));
    assert_eq!(app.doc.frame, 20);
    app.frame(Input::key(Key::Down));
    app.frame(Input::key(Key::Down));
    assert_eq!(app.doc.frame, 1);
}

#[test]
fn ruler_click_sets_frame_and_keys_move() {
    let mut app = App::new(1280, 720);
    app.frame(Input::Tick);
    let cube = app.doc.active().unwrap();
    let name = app.doc.scene.objects[cube].name.clone();
    let tr = timeline_rect(&app);
    // Ruler sits right under the header.
    let ry = tr.y + crate::theme::HEADER_H + 1 + 8;
    let s = match &app.workspaces[0].areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::Timeline).unwrap().editor {
        crate::editors::Editor::Timeline(s) => (s.x0, s.ppf, s.keys_rect),
        _ => unreachable!(),
    };
    let fx = |f: f32| (s.2.x as f32 + (f - s.0) * s.1).round() as i32;
    click(&mut app, fx(100.0), ry);
    assert_eq!(app.doc.frame, 100);
    // Key at 100, select it by clicking its diamond, G and move by 10 frames.
    app.frame(Input::key(Key::Char('i')));
    let ky = s.2.y + 6 + 10;
    click(&mut app, fx(100.0), ky);
    assert!(app.doc.anim.any_selected_key());
    app.frame(Input::key(Key::Char('g')));
    app.frame(Input::Move { x: fx(110.0), y: ky });
    app.frame(Input::Down { x: fx(110.0), y: ky, button: Button::Left, mods: Mods::NONE });
    assert_eq!(app.doc.anim.key_frames(&name), vec![110.0]);
    app.frame(Input::key(Key::Char('x')));
    assert!(app.doc.anim.key_frames(&name).is_empty());
}
