// SPDX-License-Identifier: GPL-3.0-only
use super::edit::{self, SelectAction};
use super::modifiers::{evaluate, EvalContext, GpModKind, SimplifyMode};
use super::raster::{composite_scene, SceneDraw};
use super::tools::{self, DrawBrush, EraseMode, FillError, FillSettings, Primitive, StrokeBuilder};
use super::view::{DrawPlane, GpView, Placement, Projector};
use super::*;
use crate::math::{Mat4, Quat, Vec3};
use crate::render::viewport::ViewCamera;
use crate::scene::{Camera, Object, ObjectData, Scene};

/// Looking at the origin from -Y (Blender's front view), orthographic.
fn front_view(w: u32, h: u32) -> GpView {
    let cam = ViewCamera { target: Vec3::ZERO, distance: 10.0, yaw: -std::f32::consts::FRAC_PI_2, pitch: 0.0, fov_y: 0.7, ortho: true };
    GpView::from_viewport(&cam, w, h)
}

/// A perspective view from above the front.
fn tilted_view(w: u32, h: u32) -> GpView {
    let cam = ViewCamera { target: Vec3::ZERO, distance: 8.0, yaw: -1.2, pitch: 0.5, fov_y: 0.8, ortho: false };
    GpView::from_viewport(&cam, w, h)
}

fn proj<'a>(view: &'a GpView, plane: DrawPlane, placement: Placement) -> Projector<'a> {
    Projector {
        view,
        plane,
        placement,
        object: Mat4::IDENTITY,
        cursor: Vec3::new(0.5, 0.25, 0.75),
        cursor_rot: Mat4::from_trs(Vec3::ZERO, Quat::from_axis_angle(Vec3::X, 0.6), Vec3::ONE),
        depth_at: None,
        surface_offset: 0.0,
    }
}

fn line(a: Vec3, b: Vec3, n: usize) -> GpStroke {
    let pts: Vec<Vec3> = (0..n).map(|i| a.lerp(b, i as f32 / (n - 1) as f32)).collect();
    GpStroke::from_positions(&pts, 0.05, 0)
}

fn gp_with(strokes: Vec<GpStroke>) -> GreasePencil {
    let mut g = GreasePencil::blank();
    g.layers[0].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes }, key_type: KeyframeType::Keyframe, select: false });
    g
}

#[test]
fn drawing_on_every_plane() {
    let view = tilted_view(400, 300);
    let screen = [(150.0, 120.0), (180.0, 140.0), (220.0, 150.0), (260.0, 140.0), (290.0, 110.0)];
    for plane in DrawPlane::ALL {
        let p = proj(&view, plane, if plane == DrawPlane::Cursor { Placement::Cursor } else { Placement::Origin });
        let mut b = StrokeBuilder::new(DrawBrush { smooth_factor: 0.0, ..DrawBrush::default() }, screen[0]);
        for &s in &screen[1..] {
            b.add(s);
        }
        let st = b.finish(&p, 0).unwrap();
        assert_eq!(st.points.len(), screen.len(), "{plane:?}");
        let (o, n) = match plane {
            DrawPlane::Front => (Vec3::ZERO, Vec3::Y),
            DrawPlane::Side => (Vec3::ZERO, Vec3::X),
            DrawPlane::Top => (Vec3::ZERO, Vec3::Z),
            DrawPlane::View => (Vec3::ZERO, view.back),
            DrawPlane::Cursor => (p.cursor, p.cursor_rot.transform_vector(Vec3::Z).normalize()),
        };
        for q in &st.points {
            assert!((q.position - o).dot(n).abs() < 1e-3, "{plane:?}: {:?} off the plane", q.position);
            // The point is where the pointer was.
            assert!(q.radius > 0.0 && q.opacity == 1.0);
        }
        let (sx, sy, _) = view.project(st.points[0].position).unwrap();
        assert!((sx - 150.0).abs() < 0.01 && (sy - 120.0).abs() < 0.01, "{plane:?}");
    }
}

#[test]
fn stabilizer_and_spacing_drop_points() {
    let b0 = DrawBrush { use_stabilizer: true, stabilizer_radius: 20.0, ..DrawBrush::default() };
    let mut b = StrokeBuilder::new(b0, (0.0, 0.0));
    assert!(!b.add((10.0, 0.0)), "inside the string's length: no point");
    assert!(b.add((40.0, 0.0)));
    let last = *b.screen.last().unwrap();
    assert!(last.0 < 40.0 && last.0 > 10.0, "the pen lags the pointer: {last:?}");
    let mut c = StrokeBuilder::new(DrawBrush { spacing: 5.0, ..DrawBrush::default() }, (0.0, 0.0));
    assert!(!c.add((2.0, 0.0)));
    assert!(c.add((6.0, 0.0)));
}

#[test]
fn primitives_with_constraints() {
    let view = front_view(200, 200);
    let p = proj(&view, DrawPlane::Front, Placement::Origin);
    for kind in Primitive::ALL {
        let ctrl: Vec<(f32, f32)> = match kind {
            Primitive::Polyline => vec![(20.0, 20.0), (80.0, 30.0), (120.0, 90.0)],
            _ => vec![(40.0, 40.0), (160.0, 120.0)],
        };
        let s = tools::primitive_stroke(kind, &ctrl, 4, &DrawBrush::default(), &p, 0).unwrap();
        assert!(s.points.len() >= 3, "{kind:?}");
        assert_eq!(s.cyclic, kind.cyclic(), "{kind:?}");
        assert!(s.points.iter().all(|q| q.position.y.abs() < 1e-4));
    }
    // Shift: 45 degree steps for lines, squares for boxes.
    let e = tools::snap_angle((0.0, 0.0), (10.0, 1.0));
    assert!((e.1).abs() < 1e-4 && (e.0 - 101f32.sqrt()).abs() < 1e-3);
    let e = tools::snap_angle((0.0, 0.0), (10.0, 9.0));
    assert!((e.0 - e.1).abs() < 1e-3);
    assert_eq!(tools::snap_square((0.0, 0.0), (10.0, -4.0)), (10.0, -10.0));
    let circle = tools::primitive_points(Primitive::Circle, &[(0.0, 0.0), (100.0, 100.0)], 2);
    assert!(circle.iter().all(|&(x, y)| (((x - 50.0).powi(2) + (y - 50.0).powi(2)).sqrt() - 50.0).abs() < 1e-3));
}

#[test]
fn fill_closes_a_region_and_reports_leaks() {
    let view = front_view(200, 200);
    let p = proj(&view, DrawPlane::Front, Placement::Origin);
    // A closed square (cyclic) around the middle, and an open "U".
    let sq = [Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, 2.0)];
    let mut square = GpStroke::from_positions(&sq, 0.03, 0);
    square.cyclic = true;
    let d = Drawing { strokes: vec![square] };
    let bounds = tools::screen_strokes(&view, &Mat4::IDENTITY, &d);
    let (cx, cy, _) = view.project(Vec3::ZERO).unwrap();
    let st = tools::fill(&view, &bounds, (cx, cy), &FillSettings::default(), &DrawBrush::default(), &p, 1).unwrap();
    assert!(st.cyclic && st.material == 1);
    assert!(st.points.len() >= 4);
    // The fill outline hugs the square (within the line width).
    for q in &st.points {
        let m = q.position.x.abs().max(q.position.z.abs());
        assert!((m - 2.0).abs() < 0.2, "{:?}", q.position);
    }
    let mut g = gp_with(vec![]);
    g.layers[0].frames[0].drawing.strokes.push(st);
    assert_eq!(g.point_count() > 0, true);
    // Open shape: the flood escapes.
    let u = GpStroke::from_positions(&[Vec3::new(-2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0)], 0.03, 0);
    let bounds = tools::screen_strokes(&view, &Mat4::IDENTITY, &Drawing { strokes: vec![u] });
    assert_eq!(tools::fill(&view, &bounds, (cx, cy), &FillSettings::default(), &DrawBrush::default(), &p, 1).unwrap_err(), FillError::Leak);
}

#[test]
fn erase_modes() {
    let view = front_view(200, 200);
    let base = Drawing { strokes: vec![line(Vec3::new(-3.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0), 31)] };
    let (cx, cy, _) = view.project(Vec3::ZERO).unwrap();
    let mut d = base.clone();
    assert!(tools::erase(&mut d, &view, &Mat4::IDENTITY, (cx, cy), 6.0, EraseMode::Point, 1.0));
    assert_eq!(d.strokes.len(), 2, "point erase splits the stroke");
    assert!(d.strokes.iter().all(|s| s.points.len() < 31));
    let mut d = base.clone();
    tools::erase(&mut d, &view, &Mat4::IDENTITY, (cx, cy), 6.0, EraseMode::Stroke, 1.0);
    assert!(d.strokes.is_empty());
    let mut d = base.clone();
    tools::erase(&mut d, &view, &Mat4::IDENTITY, (cx, cy), 6.0, EraseMode::Dissolve, 1.0);
    assert_eq!(d.strokes.len(), 1);
    assert!(d.strokes[0].points.len() < 31);
    let mut d = base.clone();
    tools::erase(&mut d, &view, &Mat4::IDENTITY, (cx, cy), 10.0, EraseMode::Soft, 0.3);
    assert_eq!(d.strokes.len(), 1);
    let mid = &d.strokes[0].points[15];
    assert!(mid.opacity < 1.0 && mid.opacity > 0.0);
    // Missing the stroke changes nothing.
    let mut d = base.clone();
    assert!(!tools::erase(&mut d, &view, &Mat4::IDENTITY, (cx, cy - 60.0), 5.0, EraseMode::Point, 1.0));
    assert_eq!(d, base);
}

#[test]
fn tint_colours_points() {
    let view = front_view(200, 200);
    let mut d = Drawing { strokes: vec![line(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 11)] };
    let (cx, cy, _) = view.project(Vec3::ZERO).unwrap();
    assert!(tools::tint(&mut d, &view, &Mat4::IDENTITY, (cx, cy), 20.0, 1.0, [1.0, 0.0, 0.0], true, false));
    let c = d.strokes[0].points[5].vertex_color;
    assert!(c[0] > 0.9 && c[3] > 0.9);
    assert_eq!(d.strokes[0].points[0].vertex_color, [0.0; 4]);
}

#[test]
fn edit_operators() {
    let mut d = Drawing { strokes: vec![line(Vec3::ZERO, Vec3::X, 5), line(Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0), 5)] };
    edit::select_all(&mut d, SelectAction::Select);
    assert!(edit::subdivide(&mut d, 1));
    assert_eq!(d.strokes[0].points.len(), 9);
    assert!(edit::join(&mut d, false));
    assert_eq!(d.strokes.len(), 1);
    assert_eq!(d.strokes[0].points.len(), 18);
    // Select the last point and extrude.
    edit::select_all(&mut d, SelectAction::Deselect);
    d.strokes[0].points.last_mut().unwrap().select = true;
    assert_eq!(edit::extrude(&mut d), 1);
    assert_eq!(d.strokes[0].points.len(), 19);
    edit::transform_selected(&mut d, &|p| p + Vec3::Z);
    assert_eq!(d.strokes[0].points.last().unwrap().position, Vec3::new(3.0, 0.0, 1.0));
    // Delete a middle point: two strokes.
    edit::select_all(&mut d, SelectAction::Deselect);
    d.strokes[0].points[8].select = true;
    assert!(edit::delete_points(&mut d));
    assert_eq!(d.strokes.len(), 2);
    edit::select_all(&mut d, SelectAction::Select);
    assert!(edit::toggle_cyclic(&mut d));
    assert!(d.strokes.iter().all(|s| s.cyclic));
    edit::set_caps(&mut d, Some(CapType::Flat), None);
    assert!(d.strokes.iter().all(|s| s.start_cap == CapType::Flat && s.end_cap == CapType::Round));
    edit::set_material(&mut d, 2);
    assert!(d.strokes.iter().all(|s| s.material == 2));
    let n = d.point_count();
    assert_eq!(edit::duplicate(&mut d), 2);
    assert_eq!(d.point_count(), n * 2);
    let taken = edit::extract_selected(&mut d);
    assert_eq!(taken.len(), 2);
    assert!(edit::delete_strokes(&mut { let mut x = d.clone(); edit::select_all(&mut x, SelectAction::Select); x }));
    // Simplify a straight line down to its ends.
    let mut d = Drawing { strokes: vec![line(Vec3::ZERO, Vec3::X, 20)] };
    edit::select_all(&mut d, SelectAction::Select);
    edit::simplify(&mut d, 0.01);
    assert_eq!(d.strokes[0].points.len(), 2);
}

#[test]
fn sculpt_and_paint_brushes() {
    use super::sculpt::*;
    let view = front_view(200, 200);
    let (cx, cy, _) = view.project(Vec3::ZERO).unwrap();
    let base = Drawing { strokes: vec![line(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 11)] };
    let dab = Dab { at: (cx, cy), prev: (cx, cy), radius: 30.0, strength: 1.0, invert: false };
    let mut d = base.clone();
    let mut st = SculptStroke::default();
    assert!(sculpt_dab(&mut d, &view, &Mat4::IDENTITY, SculptTool::Thickness, &dab, &mut st, &[]));
    assert!(d.strokes[0].points[5].radius > base.strokes[0].points[5].radius);
    let mut d = base.clone();
    let grab = Dab { at: (cx, cy - 20.0), ..dab };
    sculpt_dab(&mut d, &view, &Mat4::IDENTITY, SculptTool::Grab, &grab, &mut SculptStroke::default(), &[]);
    assert!(d.strokes[0].points[5].position.z > 0.1, "grabbed upwards: {:?}", d.strokes[0].points[5].position);
    let mut d = base.clone();
    sculpt_dab(&mut d, &view, &Mat4::IDENTITY, SculptTool::Pinch, &dab, &mut SculptStroke::default(), &[]);
    assert!(d.strokes[0].points[3].position.x > base.strokes[0].points[3].position.x);
    let mut d = base.clone();
    sculpt_dab(&mut d, &view, &Mat4::IDENTITY, SculptTool::Clone, &dab, &mut SculptStroke::default(), &base.strokes);
    assert_eq!(d.strokes.len(), 2);
    let mut d = base.clone();
    assert!(vertex_dab(&mut d, &view, &Mat4::IDENTITY, VertexTool::Draw, &dab, [0.0, 1.0, 0.0], false));
    assert!(d.strokes[0].points[5].vertex_color[1] > 0.9);
    let mut d = base.clone();
    assert!(weight_dab(&mut d, &view, &Mat4::IDENTITY, WeightTool::Draw, &dab, 0, 1.0));
    assert!(d.strokes[0].points[5].weight(0) > 0.9);
    assert!(d.strokes[0].points[0].weight(0) < d.strokes[0].points[5].weight(0), "falloff towards the brush edge");
}

fn eval_one(g: &GreasePencil, m: GpModifier, scene: Option<&Scene>, owner: usize, frame: i32) -> Vec<GpStroke> {
    evaluate(g, &[m], &EvalContext { scene, owner, frame, render: false }).remove(0).strokes
}

fn modifier(id: &str, f: impl FnOnce(&mut GpModKind)) -> GpModifier {
    let mut m = GpModifier::new(id).unwrap();
    f(&mut m.kind);
    m
}

#[test]
fn every_modifier_has_its_effect() {
    let base = line(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 11);
    let g = gp_with(vec![base.clone()]);
    let n0 = base.points.len();
    let mut checked = Vec::new();
    for (id, _) in modifiers::TYPES {
        checked.push(id);
        let m = GpModifier::new(id).unwrap();
        assert_eq!(m.type_id(), id);
        let out = match id {
            "ARRAY" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert_eq!(out.len(), 2);
                assert!((out[1].points[0].position.x - 1.0).abs() < 1e-4, "relative offset by the width");
                out
            }
            "BUILD" => {
                let m = modifier(id, |k| if let GpModKind::Build { length, .. } = k { *length = 10.0 });
                let half = eval_one(&g, m.clone(), None, 0, 6);
                assert!(half[0].points.len() < n0 && !half[0].points.is_empty());
                let full = eval_one(&g, m, None, 0, 20);
                assert_eq!(full[0].points.len(), n0);
                half
            }
            "ENVELOPE" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert!(out[0].points[5].radius > base.points[5].radius);
                out
            }
            "LENGTH" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert!(out[0].length() > base.length() * 1.1);
                out
            }
            "LINEART" => continue, // needs a scene: line_art_sees_a_cube
            "MIRROR" => {
                let shifted = gp_with(vec![line(Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), 3)]);
                let out = eval_one(&shifted, m, None, 0, 1);
                assert_eq!(out.len(), 2);
                assert!(out[1].points[0].position.x < 0.0);
                out
            }
            "MULTIPLY" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert_eq!(out.len(), 5);
                assert!(out.iter().any(|s| s.points[5].position.z.abs() > 0.05));
                out
            }
            "OUTLINE" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert!(out[0].cyclic && out[0].points.len() > 2 * n0);
                out
            }
            "SIMPLIFY" => {
                let m = modifier(id, |k| *k = GpModKind::Simplify(SimplifyMode::Adaptive { factor: 0.01 }));
                let out = eval_one(&g, m, None, 0, 1);
                assert_eq!(out[0].points.len(), 2);
                out
            }
            "SMOOTH" => {
                let mut zig = base.clone();
                zig.points[5].position.z = 0.5;
                let out = eval_one(&gp_with(vec![zig]), m, None, 0, 1);
                assert!(out[0].points[5].position.z < 0.5);
                out
            }
            "SUBDIV" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert_eq!(out[0].points.len(), 2 * n0 - 1);
                out
            }
            "THICKNESS" => {
                let m = modifier(id, |k| if let GpModKind::Thickness { factor, .. } = k { *factor = 2.0 });
                let out = eval_one(&g, m, None, 0, 1);
                assert!((out[0].points[3].radius - 0.1).abs() < 1e-5);
                out
            }
            "NOISE" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert!(out[0].points.iter().zip(&base.points).any(|(a, b)| a.position != b.position));
                out
            }
            "OFFSET" => {
                let m = modifier(id, |k| if let GpModKind::Offset { location, .. } = k { *location = [0.0, 0.0, 1.0] });
                let out = eval_one(&g, m, None, 0, 1);
                assert!((out[0].points[0].position.z - 1.0).abs() < 1e-5);
                out
            }
            "OPACITY" => {
                let m = modifier(id, |k| if let GpModKind::Opacity { factor, .. } = k { *factor = 0.25 });
                let out = eval_one(&g, m, None, 0, 1);
                assert!((out[0].points[0].opacity - 0.25).abs() < 1e-5);
                out
            }
            "TINT" => {
                let m = modifier(id, |k| if let GpModKind::Tint { color, factor, .. } = k {
                    *color = [1.0, 0.0, 0.0];
                    *factor = 1.0;
                });
                let out = eval_one(&g, m, None, 0, 1);
                assert_eq!(out[0].points[0].vertex_color, [1.0, 0.0, 0.0, 1.0]);
                out
            }
            "HOOK" | "LATTICE" | "ARMATURE" => continue, // need objects: deform_modifiers_move_points
            "TIME" => {
                let mut two = g.clone();
                two.layers[0].frames.push(GpFrame { frame: 5, drawing: Drawing::default(), key_type: KeyframeType::Keyframe, select: false });
                let m = modifier(id, |k| if let GpModKind::TimeOffset { offset, .. } = k { *offset = 5 });
                // At frame 1 the layer shows frame 5's (empty) drawing.
                let out = eval_one(&two, m, None, 0, 1);
                assert!(out.is_empty());
                out
            }
            "DASH" => {
                let out = eval_one(&g, m, None, 0, 1);
                assert!(out.len() >= 3, "dashes: {}", out.len());
                assert!(out.iter().all(|s| s.points.len() == 3));
                out
            }
            "TEXTURE" => {
                let m = modifier(id, |k| if let GpModKind::TextureMapping { uv_scale, .. } = k { *uv_scale = 3.0 });
                let out = eval_one(&g, m, None, 0, 1);
                assert_eq!(out[0].uv_scale, 3.0);
                out
            }
            other => panic!("unchecked modifier {other}"),
        };
        let _ = out;
    }
    assert_eq!(checked.len(), modifiers::TYPES.len());
}

#[test]
fn influence_filters_layers_and_materials() {
    let mut g = gp_with(vec![line(Vec3::ZERO, Vec3::X, 3)]);
    g.add_layer("Other");
    g.layers[1].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: vec![line(Vec3::ZERO, Vec3::X, 3)] }, key_type: KeyframeType::Keyframe, select: false });
    let mut m = modifier("OFFSET", |k| if let GpModKind::Offset { location, .. } = k { *location = [0.0, 0.0, 1.0] });
    m.influence.layer = "Other".into();
    let out = evaluate(&g, &[m.clone()], &EvalContext { scene: None, owner: 0, frame: 1, render: false });
    assert_eq!(out[0].strokes[0].points[0].position.z, 0.0);
    assert_eq!(out[1].strokes[0].points[0].position.z, 1.0);
    m.influence.invert_layer = true;
    let out = evaluate(&g, &[m.clone()], &EvalContext { scene: None, owner: 0, frame: 1, render: false });
    assert_eq!(out[0].strokes[0].points[0].position.z, 1.0);
    m.influence = Default::default();
    m.influence.material = "Nope".into();
    let out = evaluate(&g, &[m], &EvalContext { scene: None, owner: 0, frame: 1, render: false });
    assert_eq!(out[0].strokes[0].points[0].position.z, 0.0);
}

fn scene_with_gp(g: GreasePencil) -> (Scene, usize) {
    let mut s = Scene::new();
    let oi = s.add_grease_pencil_object("Stroke", g);
    (s, oi)
}

#[test]
fn deform_modifiers_move_points() {
    let (mut s, oi) = scene_with_gp(gp_with(vec![line(Vec3::ZERO, Vec3::X, 3)]));
    let e = s.add_object(Object::new("Hooker", ObjectData::Empty));
    let g = s.grease_pencils[0].clone();
    let m = modifier("HOOK", |k| if let GpModKind::Hook { object, .. } = k { *object = "Hooker".into() });
    // The hook was set where the empty is; moving the empty drags the points.
    let mut m = m;
    s.objects[e].transform.location = Vec3::ZERO;
    let base = eval_one(&g, m.clone(), Some(&s), oi, 1);
    assert_eq!(base[0].points[1].position, Vec3::new(0.5, 0.0, 0.0));
    if let GpModKind::Hook { .. } = &mut m.kind {}
    // Hook matrix_inverse is computed at evaluation time from the current
    // positions, so instead check a lattice with a moved point.
    let mut lat = crate::lattice::Lattice::new(2, 2, 2);
    for p in lat.points.iter_mut() {
        p.z += 1.0;
    }
    let li = s.add_lattice_object("Lat", lat);
    s.objects[li].transform.scale = Vec3::splat(4.0);
    let m = modifier("LATTICE", |k| if let GpModKind::Lattice { object, .. } = k { *object = "Lat".into() });
    let out = eval_one(&g, m, Some(&s), oi, 1);
    assert!(out[0].points[1].position.z > 0.5, "lattice lifted the points: {:?}", out[0].points[1].position);
}

#[test]
fn interpolation_between_keyframes() {
    let mut l = GpLayer::new("Layer");
    l.frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: vec![line(Vec3::ZERO, Vec3::X, 5)] }, key_type: KeyframeType::Keyframe, select: false });
    l.frames.push(GpFrame { frame: 11, drawing: Drawing { strokes: vec![line(Vec3::Z, Vec3::X + Vec3::Z, 9)] }, key_type: KeyframeType::Keyframe, select: false });
    let mut a = l.clone();
    assert!(interp::interpolate(&mut a, 6, interp::Easing::Linear));
    let k = a.key_at(6).unwrap();
    assert_eq!(a.frames[k].key_type, KeyframeType::Breakdown);
    let s = &a.frames[k].drawing.strokes[0];
    assert_eq!(s.points.len(), 9);
    assert!(s.points.iter().all(|p| (p.position.z - 0.5).abs() < 1e-5));
    assert!(!interp::interpolate(&mut a, 6, interp::Easing::Linear), "already a key");
    let mut b = l.clone();
    assert_eq!(interp::interpolate_sequence(&mut b, 1, 1, interp::Easing::EaseInOut), 9);
    assert_eq!(b.frames.len(), 11);
    let z3 = b.drawing_at(3).unwrap().strokes[0].points[0].position.z;
    assert!(z3 < 0.2, "ease in: {z3}");
    assert!(!interp::interpolate(&mut l.clone(), 20, interp::Easing::Linear));
}

#[test]
fn onion_skins_neighbouring_keys() {
    let mut g = gp_with(vec![line(Vec3::ZERO, Vec3::X, 3)]);
    for f in [5, 9] {
        g.layers[0].frames.push(GpFrame { frame: f, drawing: Drawing { strokes: vec![line(Vec3::ZERO, Vec3::Z, 3)] }, key_type: KeyframeType::Keyframe, select: false });
    }
    assert!(raster::onion_ghosts(&g, 5).is_empty(), "off by default");
    g.onion.enabled = true;
    let ghosts = raster::onion_ghosts(&g, 5);
    assert_eq!(ghosts.len(), 2);
    assert_eq!(ghosts[0].2[..3], g.onion.color_before);
}

fn pixel(buf: &[[f32; 4]], w: usize, x: f32, y: f32) -> [f32; 4] {
    buf[y as usize * w + x as usize]
}

#[test]
fn render_compositing_order_and_depth() {
    let (w, h) = (120u32, 90u32);
    let red = {
        let mut g = gp_with(vec![line(Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), 9)]);
        g.materials[0].color = [1.0, 0.0, 0.0, 1.0];
        g.layers[0].frames[0].drawing.strokes[0].points.iter_mut().for_each(|p| p.radius = 0.3);
        g
    };
    let mut blue = red.clone();
    blue.materials[0].color = [0.0, 0.0, 1.0, 1.0];
    let mut s = Scene::new();
    let a = s.add_grease_pencil_object("Red", red);
    let b = s.add_grease_pencil_object("Blue", blue);
    // Red in front (towards the camera at -Y), blue behind.
    s.objects[a].transform.location = Vec3::new(0.0, -1.0, 0.0);
    s.objects[b].transform.location = Vec3::new(0.0, 1.0, 0.0);
    let cam = s.add_object(Object::new("Camera", ObjectData::Camera(Camera { fov_y: 0.8, ..Camera::default() })));
    s.objects[cam].transform = crate::scene::Transform::from_matrix(&Mat4::look_at(Vec3::new(0.0, -10.0, 0.0), Vec3::ZERO, Vec3::Z).inverse());
    s.active_camera = Some(cam);
    let view = GpView::from_camera(&s.world_matrix(cam), match &s.objects[cam].data {
        ObjectData::Camera(c) => c,
        _ => unreachable!(),
    }, w, h);
    let (cx, cy, _) = view.project(Vec3::ZERO).unwrap();
    let mut buf = vec![[0.0f32; 4]; (w * h) as usize];
    let n = composite_scene(&s, &SceneDraw { view: &view, depth: None, frame: 1, render: true, onion: false, evaluated: None }, buf.as_mut_slice());
    assert_eq!(n, 2);
    let c = pixel(&buf, w as usize, cx, cy);
    assert!(c[0] > 0.9 && c[2] < 0.1, "the nearer (red) object is on top: {c:?}");
    // Swap depths: blue now in front.
    s.objects[a].transform.location.y = 1.0;
    s.objects[b].transform.location.y = -1.0;
    let mut buf = vec![[0.0f32; 4]; (w * h) as usize];
    composite_scene(&s, &SceneDraw { view: &view, depth: None, frame: 1, render: true, onion: false, evaluated: None }, buf.as_mut_slice());
    let c = pixel(&buf, w as usize, cx, cy);
    assert!(c[2] > 0.9 && c[0] < 0.1, "{c:?}");
    // A surface 5 units from the camera hides blue (at 9) but not red... both are behind it.
    let depth = vec![5.0f32; (w * h) as usize];
    let mut buf = vec![[0.0f32; 4]; (w * h) as usize];
    composite_scene(&s, &SceneDraw { view: &view, depth: Some(&depth), frame: 1, render: true, onion: false, evaluated: None }, buf.as_mut_slice());
    assert_eq!(pixel(&buf, w as usize, cx, cy), [0.0; 4], "strokes behind the surface are hidden");
    // Through composite_render with a depth pass image: anti-aliased edges.
    let mut img = crate::scene::Image::new("r", w, h);
    let dimg = crate::scene::Image { name: "d".into(), width: w, height: h, pixels: vec![[1e10, 1e10, 1e10, 1.0]; (w * h) as usize] };
    assert_eq!(raster::composite_render(&s, &mut img, Some(&dimg), 1), 2);
    let edge_values: Vec<f32> = (0..h as usize).map(|y| img.pixels[y * w as usize + cx as usize][2]).collect();
    assert!(edge_values.iter().any(|&v| v > 0.05 && v < 0.95), "anti-aliased edge: {edge_values:?}");
}

#[test]
fn fills_holdouts_layers_and_effects() {
    let (w, h) = (100u32, 100u32);
    let view = front_view(w, h);
    let mut g = GreasePencil::blank();
    g.materials[0] = GpMaterial::fill("Fill", [0.0, 1.0, 0.0, 1.0]);
    let sq = [Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, 2.0)];
    let mut fill = GpStroke::from_positions(&sq, 0.01, 0);
    fill.cyclic = true;
    g.layers[0].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: vec![fill] }, key_type: KeyframeType::Keyframe, select: false });
    let (mut s, oi) = scene_with_gp(g);
    let draw = |s: &Scene| {
        let mut buf = vec![[0.0f32; 4]; (w * h) as usize];
        composite_scene(s, &SceneDraw { view: &view, depth: None, frame: 1, render: false, onion: false, evaluated: None }, buf.as_mut_slice());
        buf
    };
    let (cx, cy, _) = view.project(Vec3::ZERO).unwrap();
    let (rx, _, _) = view.project(Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let buf = draw(&s);
    assert!(pixel(&buf, w as usize, cx, cy)[1] > 0.99, "filled inside");
    assert_eq!(pixel(&buf, w as usize, 2.0, 2.0), [0.0; 4]);
    // A holdout stroke on a layer above cuts through.
    {
        let g = &mut s.grease_pencils[0];
        let mut cutter = GpMaterial::new("Cut", [1.0; 4]);
        cutter.use_stroke_holdout = true;
        g.materials.push(cutter);
        let l = g.add_layer("Top");
        let mut st = line(Vec3::new(-0.5, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0), 3);
        st.material = 1;
        st.points.iter_mut().for_each(|p| p.radius = 0.4);
        g.layers[l].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: vec![st] }, key_type: KeyframeType::Keyframe, select: false });
    }
    let buf = draw(&s);
    assert!(pixel(&buf, w as usize, cx, cy)[3] < 0.05, "holdout");
    assert!(pixel(&buf, w as usize, cx, cy - 20.0)[1] > 0.99);
    // Layer opacity and tint.
    s.grease_pencils[0].layers[1].hide = true;
    s.grease_pencils[0].layers[0].opacity = 0.5;
    s.grease_pencils[0].layers[0].tint_factor = 1.0;
    s.grease_pencils[0].layers[0].tint_color = [1.0, 0.0, 0.0];
    let buf = draw(&s);
    let c = pixel(&buf, w as usize, cx, cy);
    assert!((c[3] - 0.5).abs() < 0.01 && c[0] > 0.45 && c[1] < 0.05, "{c:?}");
    s.grease_pencils[0].layers[0].opacity = 1.0;
    s.grease_pencils[0].layers[0].tint_factor = 0.0;
    // Visual effects: Flip moves the right half to the left; Colorize greys it.
    let asym = |s: &mut Scene| {
        let g = &mut s.grease_pencils[0];
        let d = &mut g.layers[0].frames[0].drawing;
        d.strokes[0].points = [Vec3::new(0.5, 0.0, -1.0), Vec3::new(3.0, 0.0, -1.0), Vec3::new(3.0, 0.0, 1.0), Vec3::new(0.5, 0.0, 1.0)].iter().map(|&p| GpPoint::new(p, 0.01)).collect();
    };
    asym(&mut s);
    let buf = draw(&s);
    let lx = 2.0 * cx - rx;
    assert!(pixel(&buf, w as usize, rx, cy)[1] > 0.9 && pixel(&buf, w as usize, lx, cy)[3] < 0.01);
    s.objects[oi].shader_fx.push(GpEffect::new("FX_FLIP").unwrap());
    let buf = draw(&s);
    assert!(pixel(&buf, w as usize, lx, cy)[1] > 0.9 && pixel(&buf, w as usize, rx, cy)[3] < 0.01, "flipped");
    s.objects[oi].shader_fx[0] = GpEffect::new("FX_COLORIZE").unwrap();
    if let effects::EffectKind::Colorize { factor, .. } = &mut s.objects[oi].shader_fx[0].kind {
        *factor = 1.0;
    }
    let c = pixel(&draw(&s), w as usize, rx, cy);
    assert!((c[0] - c[1]).abs() < 1e-4 && (c[1] - c[2]).abs() < 1e-4, "grey: {c:?}");
    for (id, _) in effects::TYPES {
        s.objects[oi].shader_fx = vec![GpEffect::new(id).unwrap()];
        let buf = draw(&s);
        assert!(buf.iter().any(|p| p[3] > 0.0), "{id} keeps something visible");
    }
    s.objects[oi].shader_fx = vec![GpEffect::new("FX_PIXEL").unwrap()];
    if let effects::EffectKind::Pixelate { size } = &mut s.objects[oi].shader_fx[0].kind {
        *size = [10, 10];
    }
    let buf = draw(&s);
    let block = |x: usize, y: usize| buf[y * w as usize + x];
    assert_eq!(block(60, 40), block(69, 49), "one block, one colour");
}

#[test]
fn line_art_sees_a_cube() {
    let mut s = Scene::new();
    s.add_mesh_object("Cube", crate::primitives::cube(2.0));
    let cam = s.add_object(Object::new("Camera", ObjectData::Camera(Camera::default())));
    s.objects[cam].transform = crate::scene::Transform::from_matrix(&Mat4::look_at(Vec3::new(6.0, -7.0, 5.0), Vec3::ZERO, Vec3::Z).inverse());
    s.active_camera = Some(cam);
    let mut g = GreasePencil::blank();
    g.layers[0].name = "Lines".into();
    let oi = s.add_grease_pencil_object("LineArt", g);
    let m = GpModifier::new("LINEART").unwrap();
    let out = evaluate(&s.grease_pencils[0], &[m.clone()], &EvalContext { scene: Some(&s), owner: oi, frame: 1, render: true });
    let pts: Vec<Vec3> = out[0].strokes.iter().flat_map(|st| st.points.iter().map(|p| p.position)).collect();
    assert!(!out[0].strokes.is_empty());
    // A cube seen from a corner shows 9 edges: 3 meeting at the near corner
    // and the 6 of the silhouette. The far corner (-1,1,-1) is hidden.
    let near = pts.iter().filter(|p| p.distance(Vec3::new(1.0, -1.0, 1.0)) < 0.05).count();
    let far = pts.iter().filter(|p| p.distance(Vec3::new(-1.0, 1.0, -1.0)) < 0.05).count();
    assert!(near > 0, "the nearest corner has lines");
    assert_eq!(far, 0, "the far corner is occluded");
    let total: f32 = out[0].strokes.iter().map(|s| s.length()).sum();
    assert!((total - 18.0).abs() < 1.5, "nine visible edges of length 2: {total}");
    // Creases off and contours off: nothing.
    let mut quiet = m;
    if let GpModKind::LineArt(ls) = &mut quiet.kind {
        ls.use_crease = false;
        ls.use_contour = false;
        ls.use_border = false;
    }
    let out = evaluate(&s.grease_pencils[0], &[quiet], &EvalContext { scene: Some(&s), owner: oi, frame: 1, render: true });
    assert!(out[0].strokes.is_empty());
}

#[test]
fn samples_and_layers() {
    let cat = sample_cat();
    assert_eq!(cat.layers.len(), 2);
    assert!(cat.point_count() > 100);
    assert!(cat.materials.iter().any(|m| m.show_fill));
    let st = sample_stroke();
    assert_eq!(st.layers[0].frames[0].drawing.strokes.len(), 1);
    let mut g = GreasePencil::blank();
    let i = g.add_layer("Layer");
    assert_eq!(g.layers[i].name, "Layer.001");
    let k = g.layers[0].ensure_key(5, false);
    assert_eq!(g.layers[0].frames[k].frame, 5);
    assert_eq!(g.layers[0].frame_index_at(4), None);
    assert_eq!(g.layers[0].frame_index_at(7), Some(0));
    g.groups.push(LayerGroup { name: "G".into(), parent: None, hide: true, lock: false, expanded: true, color_tag: 0 });
    g.layers[0].group = Some(0);
    assert!(!g.layer_visible(0));
}

#[test]
fn polyops_round_trip() {
    use crate::sequencer::{Effect, Sequencer, Strip, StripKind, StripModifier};
    let mut scene = Scene::new();
    let mut g = sample_cat();
    g.vertex_groups.push("Ears".into());
    g.groups.push(LayerGroup { name: "Head".into(), parent: None, hide: false, lock: true, expanded: true, color_tag: 3 });
    g.layers[0].group = Some(0);
    g.layers[1].opacity = 0.5;
    g.layers[1].blend = LayerBlend::Multiply;
    g.layers[1].transform.location = Vec3::new(0.0, 0.5, 0.0);
    g.layers[1].masks = vec!["Fills".into()];
    g.layers[1].frames[0].key_type = KeyframeType::Breakdown;
    g.layers[1].ensure_key(10, true);
    g.layers[1].frames[1].drawing.strokes[0].points[0].set_weight(0, 0.25);
    g.layers[1].frames[1].drawing.strokes[0].cyclic = true;
    g.layers[1].frames[1].drawing.strokes[0].end_cap = CapType::Flat;
    g.materials[0].mode = LineMode::Dots;
    g.onion.enabled = true;
    g.onion.before = 3;
    g.depth_order = DepthOrder::Location3D;
    let oi = scene.add_grease_pencil_object("Cat", g);
    scene.objects[oi].transform.location = Vec3::new(1.0, 2.0, 3.0);
    for (id, _) in modifiers::TYPES {
        let mut m = GpModifier::new(id).unwrap();
        m.influence.layer = "Lines".into();
        if let GpModKind::Dash { segments, .. } = &mut m.kind {
            segments.push(Default::default());
            segments[1].gap = 4;
        }
        if let GpModKind::Simplify(s) = &mut m.kind {
            *s = SimplifyMode::Sample { length: 0.25 };
        }
        scene.objects[oi].gp_modifiers.push(m);
    }
    for (id, _) in effects::TYPES {
        scene.objects[oi].shader_fx.push(effects::GpEffect::new(id).unwrap());
    }
    let mut q = Sequencer::default();
    let a = q.add(Strip::new("Red", StripKind::Color { color: [1.0, 0.0, 0.0] }, 1, 1.0, 50.0));
    let b = q.add(Strip::new("Pics", StripKind::Image { directory: "//img".into(), files: vec!["a.png".into(), "b c.png".into()] }, 2, 10.0, 2.0));
    q.add_effect(Effect::new("WIPE").unwrap(), &[a, b], 3, 1.0);
    let mut t = Strip::new("Title", StripKind::Text(Default::default()), 4, 1.0, 20.0);
    t.modifiers.push(StripModifier::new("CURVES").unwrap());
    t.opacity = 0.75;
    q.add(t);
    q.add(Strip::new("Snd", StripKind::Sound { path: "a.wav".into(), volume: 0.5, pan: 0.0, waveform: Vec::new() }, 5, 1.0, 30.0));
    q.add(Strip::new("Inner", StripKind::Adjustment, 6, 1.0, 5.0));
    q.select_all(Some(false));
    let inner = q.find("Inner").unwrap();
    q.strips[inner].select = true;
    q.make_meta();
    q.channel_mut(2).lock = true;
    q.use_sequencer = false;
    scene.sequence_editor = Some(q);

    let text = crate::script::scene_to_polyops(&scene);
    let mut back = Scene::new();
    crate::script::run(&mut back, &text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(back.grease_pencil_names, scene.grease_pencil_names);
    assert_eq!(back.grease_pencils, scene.grease_pencils);
    let bo = back.find_object("Cat").unwrap();
    assert_eq!(back.objects[bo].gp_modifiers, scene.objects[oi].gp_modifiers);
    assert_eq!(back.objects[bo].shader_fx, scene.objects[oi].shader_fx);
    assert_eq!(back.objects[bo].transform.location, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(back.sequence_editor, scene.sequence_editor);
    // And again: the text is stable.
    assert_eq!(crate::script::scene_to_polyops(&back), text);
}
