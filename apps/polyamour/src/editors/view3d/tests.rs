// SPDX-License-Identifier: GPL-3.0-only
//! Host tests that drive the whole App with synthetic input, the way a user
//! would: pointer over the viewport, then Blender's keys.
use super::{Modal, State};
use crate::app::App;
use crate::doc::{Mode, SelectMode};
use crate::editors::Editor;
use crate::input::{Button, Input, Key, Mods};
use std::f32::consts::FRAC_PI_2;

fn view(app: &mut App) -> &mut State {
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::View3D(s) = &mut a.editor {
            return s;
        }
    }
    panic!("no 3D viewport in the active workspace");
}

/// A fresh app with the pointer resting over the middle of the viewport.
fn setup() -> (App, (i32, i32)) {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let r = view(&mut app).rect;
    let c = (r.x + r.w / 2, r.y + r.h / 2);
    app.frame(Input::Move { x: c.0, y: c.1 });
    (app, c)
}

fn key(app: &mut App, k: Key, mods: Mods) {
    let text = match k {
        Key::Char(c) => Some(if mods.shift { c.to_ascii_uppercase() } else { c }),
        _ => None,
    };
    app.frame(Input::Key { key: k, mods, text });
}

fn typed(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, Key::Char(c), Mods::NONE);
    }
}

fn click(app: &mut App, x: i32, y: i32) {
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Up { x, y, button: Button::Left });
}

fn cube(app: &App) -> usize {
    app.doc.scene.objects.iter().position(|o| o.name == "Cube").expect("default cube")
}

fn select_only_cube(app: &mut App) -> usize {
    let c = cube(app);
    app.doc.scene.selected = vec![c];
    app.doc.scene.active_object = Some(c);
    c
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn grab_x_2_enter_moves_two_along_x_and_undoes() {
    let (mut app, _) = setup();
    let c = select_only_cube(&mut app);
    let before = app.doc.scene.objects[c].transform.location;
    key(&mut app, Key::Char('g'), Mods::NONE);
    assert!(matches!(view(&mut app).modal, Modal::Transform(_)), "G starts a modal move");
    typed(&mut app, "x2");
    key(&mut app, Key::Enter, Mods::NONE);
    assert!(matches!(view(&mut app).modal, Modal::None));
    let after = app.doc.scene.objects[c].transform.location;
    assert!(close(after.x, before.x + 2.0), "{before:?} -> {after:?}");
    assert!(close(after.y, before.y) && close(after.z, before.z));
    key(&mut app, Key::Char('z'), Mods::CTRL);
    let undone = app.doc.scene.objects[c].transform.location;
    assert!(close(undone.x, before.x), "Ctrl+Z restores the location");
}

#[test]
fn rotate_z_90() {
    let (mut app, _) = setup();
    let c = select_only_cube(&mut app);
    key(&mut app, Key::Char('r'), Mods::NONE);
    typed(&mut app, "z90");
    key(&mut app, Key::Enter, Mods::NONE);
    let e = super::sidebar::quat_to_euler(app.doc.scene.objects[c].transform.rotation);
    assert!(close(e.z, FRAC_PI_2), "{e:?}");
    assert!(close(e.x, 0.0) && close(e.y, 0.0), "{e:?}");
}

#[test]
fn scale_2() {
    let (mut app, _) = setup();
    let c = select_only_cube(&mut app);
    let before = app.doc.scene.objects[c].transform.scale;
    key(&mut app, Key::Char('s'), Mods::NONE);
    typed(&mut app, "2");
    key(&mut app, Key::Enter, Mods::NONE);
    let after = app.doc.scene.objects[c].transform.scale;
    assert!(close(after.x, before.x * 2.0) && close(after.y, before.y * 2.0) && close(after.z, before.z * 2.0), "{after:?}");
}

#[test]
fn escape_cancels_a_modal_move() {
    let (mut app, (cx, cy)) = setup();
    let c = select_only_cube(&mut app);
    let before = app.doc.scene.objects[c].transform.location;
    let undo = app.doc.undo.undo_label().map(str::to_string);
    key(&mut app, Key::Char('g'), Mods::NONE);
    app.frame(Input::Move { x: cx + 120, y: cy - 40 });
    let moving = app.doc.scene.objects[c].transform.location;
    assert!((moving - before).length() > 0.01, "the cube follows the mouse");
    key(&mut app, Key::Esc, Mods::NONE);
    let after = app.doc.scene.objects[c].transform.location;
    assert!((after - before).length() < 1e-5, "Esc puts it back");
    assert_eq!(app.doc.undo.undo_label().map(str::to_string), undo, "a cancelled move leaves no undo step");
}

#[test]
fn edit_mode_face_extrude_adds_faces() {
    let (mut app, (cx, cy)) = setup();
    let c = select_only_cube(&mut app);
    key(&mut app, Key::Tab, Mods::NONE);
    assert_eq!(app.doc.mode, Mode::Edit);
    key(&mut app, Key::Char('3'), Mods::NONE);
    assert_eq!(app.doc.select_mode, SelectMode::Face);
    // Click the face under the middle of the view (the cube sits at the origin).
    click(&mut app, cx, cy);
    let sel = crate::edit::selected_faces(&app.doc);
    assert_eq!(sel.len(), 1, "one face picked");
    let mesh_faces = |app: &App| match &app.doc.scene.objects[c].data {
        sex_3d::scene::ObjectData::Mesh(m) => app.doc.scene.meshes[*m].faces.len(),
        _ => 0,
    };
    assert_eq!(mesh_faces(&app), 6);
    key(&mut app, Key::Char('e'), Mods::NONE);
    assert!(matches!(view(&mut app).modal, Modal::Transform(_)), "E moves the new faces");
    typed(&mut app, "1");
    key(&mut app, Key::Enter, Mods::NONE);
    assert_eq!(mesh_faces(&app), 10, "one face extruded: 4 side faces added");
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert_eq!(mesh_faces(&app), 6, "undo removes the extrusion");
    key(&mut app, Key::Tab, Mods::NONE);
    assert_eq!(app.doc.mode, Mode::Object);
}

#[test]
fn box_select_selects_objects_inside() {
    let (mut app, (cx, cy)) = setup();
    let c = cube(&app);
    app.doc.scene.selected.clear();
    // A box around the middle of the view, where the cube is.
    let (x0, y0, x1, y1) = (cx - 60, cy - 60, cx + 60, cy + 60);
    app.frame(Input::Move { x: x0, y: y0 });
    app.frame(Input::Down { x: x0, y: y0, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: (x0 + x1) / 2, y: (y0 + y1) / 2 });
    app.frame(Input::Move { x: x1, y: y1 });
    app.frame(Input::Up { x: x1, y: y1, button: Button::Left });
    assert!(app.doc.scene.selected.contains(&c), "{:?}", app.doc.scene.selected);
    // An empty box in a corner deselects (Set mode).
    let r = view(&mut app).rect;
    let (x0, y0) = (r.x + r.w / 2 - 20, r.bottom() - 40);
    app.frame(Input::Move { x: x0, y: y0 });
    app.frame(Input::Down { x: x0, y: y0, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: x0 + 10, y: y0 + 10 });
    app.frame(Input::Move { x: x0 + 20, y: y0 + 20 });
    app.frame(Input::Up { x: x0 + 20, y: y0 + 20, button: Button::Left });
    assert!(app.doc.scene.selected.is_empty(), "{:?}", app.doc.scene.selected);
}

#[test]
fn numpad_7_gives_top_ortho() {
    let (mut app, _) = setup();
    key(&mut app, Key::Num(7), Mods::NONE);
    let s = view(&mut app);
    assert!(close(s.cam.pitch, FRAC_PI_2), "pitch {}", s.cam.pitch);
    assert!(s.cam.ortho);
    assert_eq!(s.axis_view, Some("Top"));
    // Orbiting leaves the axis view and goes back to perspective.
    key(&mut app, Key::Num(4), Mods::NONE);
    let s = view(&mut app);
    assert_eq!(s.axis_view, None);
    assert!(!s.cam.ortho);
    // Numpad 1 / Ctrl+1: front and back.
    key(&mut app, Key::Num(1), Mods::NONE);
    assert_eq!(view(&mut app).axis_view, Some("Front"));
    key(&mut app, Key::Num(1), Mods::CTRL);
    assert_eq!(view(&mut app).axis_view, Some("Back"));
}

#[test]
fn sidebar_location_field_sets_location() {
    let (mut app, _) = setup();
    let c = select_only_cube(&mut app);
    key(&mut app, Key::Char('n'), Mods::NONE);
    let s = view(&mut app);
    assert!(s.show_sidebar);
    let r = s.rect;
    let sr = crate::draw::Rect::new(r.right() - s.sidebar_w, r.y, s.sidebar_w, r.h);
    // Transform panel header (28), "Location:" (20), then the X field.
    let (fx, fy) = (sr.x + 80, sr.y + 6 + 28 + 20 + 10);
    click(&mut app, fx, fy);
    assert!(app.ui.editing.is_some(), "a click starts typing");
    for _ in 0..8 {
        key(&mut app, Key::Backspace, Mods::NONE);
    }
    typed(&mut app, "3.5");
    key(&mut app, Key::Enter, Mods::NONE);
    let loc = app.doc.scene.objects[c].transform.location;
    assert!(close(loc.x, 3.5), "{loc:?}");
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert!(close(app.doc.scene.objects[c].transform.location.x, 0.0));
}

#[test]
fn shift_d_duplicates_and_moves() {
    let (mut app, _) = setup();
    let c = select_only_cube(&mut app);
    let n = app.doc.scene.objects.len();
    key(&mut app, Key::Char('d'), Mods::SHIFT);
    assert_eq!(app.doc.scene.objects.len(), n + 1);
    assert!(matches!(view(&mut app).modal, Modal::Transform(_)), "the copy follows the mouse");
    typed(&mut app, "x3");
    key(&mut app, Key::Enter, Mods::NONE);
    let new = app.doc.active().unwrap();
    assert_ne!(new, c);
    assert_eq!(app.doc.scene.selected, vec![new]);
    let a = app.doc.scene.objects[c].transform.location;
    let b = app.doc.scene.objects[new].transform.location;
    assert!(close(b.x - a.x, 3.0), "{a:?} {b:?}");
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert_eq!(app.doc.scene.objects.len(), n, "one undo step removes the copy");
}

#[test]
fn x_asks_then_deletes_and_i_keys() {
    let (mut app, (cx, cy)) = setup();
    select_only_cube(&mut app);
    let n = app.doc.scene.objects.len();
    key(&mut app, Key::Char('x'), Mods::NONE);
    let pop = app.popup.clone().expect("delete confirmation");
    let r = pop.rect(&mut app.painter);
    click(&mut app, r.x + 30, r.y + 2 + 24 + 11);
    assert_eq!(app.doc.scene.objects.len(), n - 1, "{}", app.doc.report);
    app.frame(Input::Move { x: cx, y: cy });
    key(&mut app, Key::Char('i'), Mods::NONE);
    assert!(app.doc.report.to_lowercase().contains("key") || app.doc.scene.objects.is_empty() || app.doc.active().is_none(), "{}", app.doc.report);
}

#[test]
fn rendered_shading_refines_on_tick() {
    let (mut app, _) = setup();
    view(&mut app).shading = super::Shading::Rendered;
    app.frame(Input::Tick);
    assert!(app.wants_tick(), "a rendered view keeps asking for ticks");
    for _ in 0..3 {
        app.frame(Input::Tick);
    }
    let s = view(&mut app);
    assert!(s.render.as_ref().is_some_and(|r| r.samples() >= 2));
}

#[test]
fn material_preview_and_rendered_engines() {
    use crate::settings::Engine;
    let (mut app, _) = setup();
    view(&mut app).shading = super::Shading::Material;
    app.frame(Input::Tick);
    app.frame(Input::Tick);
    let s = view(&mut app);
    let rv = s.render.as_ref().expect("material preview renders");
    assert!(rv.samples() >= 1 && rv.activity() == "Rendering");
    // At the viewport's resolution, not a fraction of it.
    assert_eq!((rv.w as i32, rv.h as i32), (s.rect.w, s.rect.h));
    assert!(rv.rgba.chunks_exact(4).any(|p| p[0] != p[2]), "shaded, not blank");
    for engine in [Engine::Eevee, Engine::Workbench] {
        app.doc.settings.engine = engine;
        view(&mut app).shading = super::Shading::Rendered;
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let rv = view(&mut app).render.as_ref().expect("rendered view");
        assert!(rv.samples() >= 1, "{engine:?}");
    }
}

#[test]
fn solid_shading_options_from_the_menu() {
    use sex_3d::render::workbench::{ColorType, Lighting};
    let (mut app, _) = setup();
    app.doc.queue.push(crate::ui::Op::with("view3d.solid", "lighting:Flat"));
    app.doc.queue.push(crate::ui::Op::with("view3d.solid", "color:Random"));
    app.doc.queue.push(crate::ui::Op::with("view3d.solid", "cavity"));
    app.frame(Input::Tick);
    let w = view(&mut app).solid;
    assert_eq!((w.lighting, w.color, w.cavity), (Lighting::Flat, ColorType::Random, true));
    app.doc.queue.push(crate::ui::Op::with("view3d.solid", "matcap:2"));
    app.frame(Input::Tick);
    let w = view(&mut app).solid;
    assert_eq!((w.lighting, w.matcap), (Lighting::MatCap, 2));
}

/// Writes screenshots of a few states when POLYAMOUR_SHOTS names a folder.
#[test]
fn screenshots_on_request() {
    let Ok(dir) = std::env::var("POLYAMOUR_SHOTS") else { return };
    let save = |app: &App, name: &str| {
        let f = std::fs::File::create(format!("{dir}/{name}.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), app.painter.width as u32, app.painter.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().unwrap().write_image_data(&app.painter.to_rgba()).unwrap();
    };
    let (mut app, (cx, cy)) = setup();
    save(&app, "object");
    key(&mut app, Key::Char('n'), Mods::NONE);
    app.frame(Input::Move { x: cx, y: cy });
    key(&mut app, Key::Char('g'), Mods::NONE);
    app.frame(Input::Move { x: cx + 80, y: cy - 30 });
    save(&app, "grab");
    key(&mut app, Key::Esc, Mods::NONE);
    key(&mut app, Key::Tab, Mods::NONE);
    save(&app, "edit");
    key(&mut app, Key::Char('3'), Mods::NONE);
    click(&mut app, cx, cy);
    key(&mut app, Key::Char('e'), Mods::NONE);
    typed(&mut app, "1");
    key(&mut app, Key::Enter, Mods::NONE);
    save(&app, "extruded");
    app.frame(Input::Down { x: cx, y: cy, button: Button::Right, mods: Mods::NONE });
    save(&app, "edit_context");
    key(&mut app, Key::Esc, Mods::NONE);
    key(&mut app, Key::Tab, Mods::NONE);
    key(&mut app, Key::Num(0), Mods::NONE);
    save(&app, "camera");
    key(&mut app, Key::Num(7), Mods::NONE);
    view(&mut app).shading = super::Shading::Wireframe;
    app.frame(Input::Tick);
    save(&app, "top_wire");
    view(&mut app).shading = super::Shading::Rendered;
    key(&mut app, Key::Num(1), Mods::NONE);
    for _ in 0..16 {
        app.frame(Input::Tick);
    }
    save(&app, "rendered");
    app.frame(Input::Move { x: cx, y: cy });
    key(&mut app, Key::Char('`'), Mods::NONE);
    save(&app, "pie");
}
