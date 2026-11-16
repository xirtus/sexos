// SPDX-License-Identifier: GPL-3.0-only
//! App tests for Grease Pencil: drawing through the viewport with synthetic
//! input (each drawing plane, primitives, Fill, Erase), Edit Mode keys,
//! interpolation, the properties visitors and the Dope Sheet's key moves.
use crate::app::App;
use crate::doc::Mode;
use crate::editors::view3d::State as View;
use crate::editors::Editor;
use crate::gpencil::ui::{self as gui, GpOwner, PropColor};
use crate::gpencil::{self as g, DrawTool};
use crate::input::{Button, Input, Key, Mods};
use crate::ui::Op;
use sex_3d::grease_pencil::tools::{EraseMode, Primitive};
use sex_3d::grease_pencil::view::DrawPlane;
use sex_3d::grease_pencil::GpStroke;
use sex_3d::Vec3;

fn view(app: &mut App) -> &mut View {
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::View3D(s) = &mut a.editor {
            return s;
        }
    }
    panic!("no 3D viewport");
}

/// An app with a blank Grease Pencil object in Draw Mode, pointer over the view.
fn setup() -> (App, (i32, i32), usize) {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    app.doc.run(&Op::with("object.grease_pencil_add", "EMPTY"));
    app.doc.run(&Op::with("object.mode_set", "PAINT_GREASE_PENCIL"));
    assert_eq!(app.doc.mode, Mode::Draw);
    app.frame(Input::Tick);
    let r = view(&mut app).rect;
    let c = (r.x + r.w / 2, r.y + r.h / 2);
    app.frame(Input::Move { x: c.0, y: c.1 });
    let (_, gi) = g::active_gp(&app.doc).expect("grease pencil active");
    (app, c, gi)
}

fn drag(app: &mut App, a: (i32, i32), b: (i32, i32), mods: Mods) {
    app.frame(Input::Move { x: a.0, y: a.1 });
    app.frame(Input::Down { x: a.0, y: a.1, button: Button::Left, mods });
    for k in 1..=12 {
        let t = k as f32 / 12.0;
        let x = a.0 + ((b.0 - a.0) as f32 * t) as i32;
        let y = a.1 + ((b.1 - a.1) as f32 * t) as i32 + ((t * 6.0).sin() * 8.0) as i32;
        app.frame(Input::Move { x, y });
    }
    app.frame(Input::Up { x: b.0, y: b.1, button: Button::Left });
}

fn click(app: &mut App, p: (i32, i32)) {
    app.frame(Input::Move { x: p.0, y: p.1 });
    app.frame(Input::Down { x: p.0, y: p.1, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Up { x: p.0, y: p.1, button: Button::Left });
}

fn key(app: &mut App, k: Key, mods: Mods) {
    let text = match k {
        Key::Char(c) => Some(c),
        _ => None,
    };
    app.frame(Input::Key { key: k, mods, text });
}

/// Strokes of the active layer's drawing at the current frame.
fn strokes(app: &App, gi: usize) -> Vec<GpStroke> {
    let gp = &app.doc.scene.grease_pencils[gi];
    let l = &gp.layers[gp.active_layer];
    l.frame_index_at(app.doc.frame).map(|fi| l.frames[fi].drawing.strokes.clone()).unwrap_or_default()
}

fn spread(s: &GpStroke, f: impl Fn(Vec3) -> f32) -> f32 {
    let v: Vec<f32> = s.points.iter().map(|p| f(p.position)).collect();
    v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
}

#[test]
fn draw_strokes_on_each_plane_and_undo() {
    let (mut app, c, gi) = setup();
    for (plane, axis) in [(DrawPlane::Front, 1usize), (DrawPlane::Side, 0), (DrawPlane::Top, 2), (DrawPlane::View, 9), (DrawPlane::Cursor, 2)] {
        app.doc.gp.plane = plane;
        let n = strokes(&app, gi).len();
        drag(&mut app, (c.0 - 80, c.1), (c.0 + 90, c.1 - 40), Mods::NONE);
        let ss = strokes(&app, gi);
        assert_eq!(ss.len(), n + 1, "{plane:?}: one stroke drawn");
        let s = ss.last().unwrap();
        assert!(s.points.len() >= 4, "{plane:?}: {} points", s.points.len());
        assert!(s.points.iter().all(|p| p.radius > 0.0 && p.position.to_array().iter().all(|v| v.is_finite())));
        if axis < 3 {
            let a = [Vec3::X, Vec3::Y, Vec3::Z][axis];
            assert!(spread(s, |p| p.dot(a)) < 1e-3, "{plane:?}: points lie on the plane");
            // ...and the stroke has extent on the plane.
            let other = [Vec3::X, Vec3::Y, Vec3::Z][(axis + 1) % 3];
            let third = [Vec3::X, Vec3::Y, Vec3::Z][(axis + 2) % 3];
            assert!(spread(s, |p| p.dot(other)).max(spread(s, |p| p.dot(third))) > 0.1);
        } else {
            let back = sex_3d::grease_pencil::view::GpView::from_viewport(&view(&mut app).cam, 10, 10).back;
            assert!(spread(s, |p| p.dot(back)) < 1e-3, "View plane faces the viewer");
        }
    }
    let n = strokes(&app, gi).len();
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert_eq!(strokes(&app, gi).len(), n - 1, "Ctrl+Z removes the last stroke");
}

#[test]
fn primitives_fill_and_erase() {
    let (mut app, c, gi) = setup();
    // A box primitive closes a region.
    app.doc.gp.tool = DrawTool::Primitive(Primitive::Box);
    app.frame(Input::Move { x: c.0 - 100, y: c.1 - 80 });
    app.frame(Input::Down { x: c.0 - 100, y: c.1 - 80, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: c.0 + 100, y: c.1 + 80 });
    app.frame(Input::Up { x: c.0 + 100, y: c.1 + 80, button: Button::Left });
    let ss = strokes(&app, gi);
    assert_eq!(ss.len(), 1);
    assert!(ss[0].cyclic, "a box is cyclic");
    // Fill inside it.
    app.doc.gp.tool = DrawTool::Fill;
    click(&mut app, c);
    let ss = strokes(&app, gi);
    assert_eq!(ss.len(), 2, "fill added: {}", app.doc.report);
    let fill = &ss[0];
    let m = &app.doc.scene.grease_pencils[gi].materials[fill.material];
    assert!(m.show_fill, "the fill uses a fill material");
    assert!(fill.points.len() >= 4);
    // Outside the box the flood reaches the border: no fill.
    click(&mut app, (c.0 + 300, c.1));
    assert_eq!(strokes(&app, gi).len(), 2);
    assert!(app.doc.report.contains("unclosed"), "{}", app.doc.report);

    // A long line, then erase its middle: it splits in two.
    let (mut app, c, gi) = setup();
    app.doc.gp.tool = DrawTool::Primitive(Primitive::Line);
    app.doc.gp.primitive_subdiv = 40;
    app.frame(Input::Down { x: c.0 - 150, y: c.1, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: c.0 + 150, y: c.1 });
    app.frame(Input::Up { x: c.0 + 150, y: c.1, button: Button::Left });
    assert_eq!(strokes(&app, gi).len(), 1);
    app.doc.gp.tool = DrawTool::Erase;
    app.doc.gp.erase_mode = EraseMode::Point;
    app.doc.gp.erase.radius = 12.0;
    click(&mut app, c);
    assert_eq!(strokes(&app, gi).len(), 2, "erasing the middle splits the stroke");
    // Stroke mode removes a whole stroke.
    app.doc.gp.erase_mode = EraseMode::Stroke;
    click(&mut app, (c.0 - 100, c.1));
    assert_eq!(strokes(&app, gi).len(), 1);
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert_eq!(strokes(&app, gi).len(), 2, "erase is one undo step");
}

#[test]
fn edit_mode_select_move_delete() {
    let (mut app, c, gi) = setup();
    drag(&mut app, (c.0 - 80, c.1), (c.0 + 80, c.1), Mods::NONE);
    drag(&mut app, (c.0 - 80, c.1 + 60), (c.0 + 80, c.1 + 60), Mods::NONE);
    app.doc.run(&Op::with("object.mode_set", "EDIT"));
    assert_eq!(app.doc.mode, Mode::Edit);
    assert!(g::in_mode(&app.doc));
    key(&mut app, Key::Char('a'), Mods::NONE);
    app.frame(Input::Tick);
    assert!(strokes(&app, gi).iter().all(|s| s.points.iter().all(|p| p.select)));
    let before = strokes(&app, gi)[0].points[0].position;
    app.frame(Input::Move { x: c.0, y: c.1 });
    key(&mut app, Key::Char('g'), Mods::NONE);
    app.frame(Input::Move { x: c.0 + 60, y: c.1 });
    key(&mut app, Key::Enter, Mods::NONE);
    let after = strokes(&app, gi)[0].points[0].position;
    assert!((after - before).length() > 0.05, "G moved the points: {before:?} -> {after:?}");
    key(&mut app, Key::Char('z'), Mods::CTRL);
    let undone = strokes(&app, gi)[0].points[0].position;
    assert!((undone - before).length() < 1e-4);
    // Click on empty space deselects; box select the upper stroke.
    click(&mut app, (c.0 + 400, c.1 - 200));
    assert!(strokes(&app, gi).iter().all(|s| s.points.iter().all(|p| !p.select)));
    app.frame(Input::Down { x: c.0 - 120, y: c.1 - 30, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: c.0 + 120, y: c.1 + 30 });
    app.frame(Input::Up { x: c.0 + 120, y: c.1 + 30, button: Button::Left });
    let ss = strokes(&app, gi);
    assert!(ss[0].points.iter().any(|p| p.select) && ss[1].points.iter().all(|p| !p.select));
    app.doc.run(&Op::with("grease_pencil.delete", "STROKES"));
    assert_eq!(strokes(&app, gi).len(), 1);
    // F toggles cyclic on the selection.
    key(&mut app, Key::Char('a'), Mods::NONE);
    key(&mut app, Key::Char('f'), Mods::NONE);
    app.frame(Input::Tick);
    assert!(strokes(&app, gi)[0].cyclic);
}

#[test]
fn interpolate_between_keys() {
    let (mut app, c, gi) = setup();
    app.doc.frame = 1;
    drag(&mut app, (c.0 - 80, c.1), (c.0 + 80, c.1), Mods::NONE);
    app.doc.frame = 11;
    app.doc.run(&Op::new("grease_pencil.insert_blank_frame"));
    drag(&mut app, (c.0 - 80, c.1 - 100), (c.0 + 80, c.1 - 100), Mods::NONE);
    app.doc.frame = 6;
    key(&mut app, Key::Char('e'), Mods::CTRL);
    app.frame(Input::Tick);
    let l = &app.doc.scene.grease_pencils[gi].layers[0];
    let k = l.key_at(6).expect("Ctrl+E made a key at frame 6");
    let mid = &l.frames[k].drawing.strokes[0];
    let a = l.frames[l.key_at(1).unwrap()].drawing.strokes[0].points[0].position;
    let b = l.frames[l.key_at(11).unwrap()].drawing.strokes[0].points[0].position;
    let p = mid.points[0].position;
    assert!((p - (a + b) * 0.5).length() < 0.15 * (a - b).length(), "{a:?} {p:?} {b:?}");
    // The sequence fills the frames between the keys around the playhead.
    app.doc.frame = 3;
    app.doc.run(&Op::new("grease_pencil.interpolate_sequence"));
    app.doc.frame = 8;
    app.doc.run(&Op::new("grease_pencil.interpolate_sequence"));
    let l = &app.doc.scene.grease_pencils[gi].layers[0];
    assert!((2..=10).all(|f| l.key_at(f).is_some()), "{:?}", l.frames.iter().map(|f| f.frame).collect::<Vec<_>>());
}

#[test]
fn properties_visitors_and_colors() {
    let (mut app, _, gi) = setup();
    let oi = app.doc.active().unwrap();
    let tabs = crate::editors::properties::tabs_for(&app.doc, Some(oi));
    use crate::editors::properties::Tab;
    for t in [Tab::Modifiers, Tab::Effects, Tab::Data, Tab::Material] {
        assert!(tabs.contains(&Some(t)), "{t:?}");
    }
    gui::prop_set(&mut app.doc, &format!("{}\topacity\t0.25", GpOwner::Layer(gi, 0).to_id()));
    assert!((app.doc.scene.grease_pencils[gi].layers[0].opacity - 0.25).abs() < 1e-6);
    app.doc.run(&Op::new("ed.undo"));
    assert!((app.doc.scene.grease_pencils[gi].layers[0].opacity - 1.0).abs() < 1e-6);
    let pc = PropColor::new(GpOwner::Material(gi, 0), "color");
    gui::set_color(&mut app.doc, pc, [0.1, 0.2, 0.3]);
    let got = gui::get_color(&app.doc, pc).unwrap();
    assert!((got[2] - 0.3).abs() < 1e-6);
    // Modifiers and effects through operators.
    app.doc.run(&Op::with("object.gpencil_modifier_add", "THICKNESS"));
    app.doc.run(&Op::with("object.shaderfx_add", "FX_GLOW"));
    assert_eq!(app.doc.scene.objects[oi].gp_modifiers.len(), 1);
    assert_eq!(app.doc.scene.objects[oi].shader_fx.len(), 1);
    gui::prop_set(&mut app.doc, &format!("{}\tshow_viewport\t0", GpOwner::Modifier(oi, 0).to_id()));
    assert!(!app.doc.scene.objects[oi].gp_modifiers[0].show_viewport);
    // Every tab draws.
    for t in [Tab::Modifiers, Tab::Effects, Tab::Data, Tab::Material] {
        for a in app.workspaces[app.active_ws].areas_mut() {
            if let Editor::Properties(p) = &mut a.editor {
                p.tab = t;
            }
        }
        app.frame(Input::Tick);
    }
}

#[test]
fn modes_and_dope_sheet_keys() {
    let (mut app, c, gi) = setup();
    drag(&mut app, (c.0 - 40, c.1), (c.0 + 40, c.1), Mods::NONE);
    for (id, m) in [("SCULPT_GREASE_PENCIL", Mode::Sculpt), ("WEIGHT_GREASE_PENCIL", Mode::WeightPaint), ("VERTEX_GREASE_PENCIL", Mode::VertexPaint), ("OBJECT", Mode::Object)] {
        app.doc.run(&Op::with("object.mode_set", id));
        assert_eq!(app.doc.mode, m);
        app.frame(Input::Tick);
    }
    // Vertex paint a dab: the colour changes.
    app.doc.run(&Op::with("object.mode_set", "VERTEX_GREASE_PENCIL"));
    app.doc.gp.vertex_color = [1.0, 0.0, 0.0];
    app.doc.gp.vertex.radius = 60.0;
    click(&mut app, c);
    assert!(strokes(&app, gi)[0].points.iter().any(|p| p.vertex_color[3] > 0.0), "{}", app.doc.report);
    // Dope Sheet key moves.
    let l = &mut app.doc.scene.grease_pencils[gi].layers[0];
    for f in &mut l.frames {
        f.select = true;
    }
    let f0 = l.frames[0].frame;
    g::move_keys(&mut app.doc, 5);
    assert_eq!(app.doc.scene.grease_pencils[gi].layers[0].frames[0].frame, f0 + 5);
    let rows = g::dopesheet_rows(&app.doc);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].3[0].0, f0 + 5);
}

#[test]
fn f12_composites_grease_pencil_over_the_render() {
    let mut d = crate::doc::Doc::default();
    // A small render of the startup scene, then the cat in front of the camera.
    d.cursor = Vec3::ZERO;
    g::add_object(&mut d, "MONKEY");
    let cam = d.scene.active_camera.unwrap();
    let m = d.scene.world_matrix(cam);
    let oi = d.active().unwrap();
    // Halfway between the camera and the origin, facing it.
    d.scene.objects[oi].transform.location = m.transform_point(Vec3::ZERO) * 0.5;
    let scene = d.scene.clone();
    let st = sex_3d::render::RenderSettings { width: 48, height: 32, samples: 1, denoise: false, ..Default::default() };
    let st = crate::seqedit::with_depth(&scene, st);
    let r = sex_3d::render::render(&scene, &st);
    let before = r.color.clone();
    let r = crate::seqedit::post_render(&scene, 1, &st, r, &|_| scene.clone());
    let changed = before.pixels.iter().zip(&r.color.pixels).filter(|(a, b)| a != b).count();
    assert!(changed > 20, "the drawing covers part of the frame ({changed} px)");
}
