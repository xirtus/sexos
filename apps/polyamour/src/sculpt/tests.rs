// SPDX-License-Identifier: GPL-3.0-only
use crate::app::App;
use crate::doc::{Doc, Mode};
use crate::input::{Button, Input, Mods};
use crate::ui::Op;
use sex_3d::mesh::AttrDomain;
use sex_3d::modifiers::Modifier;
use sex_3d::scene::ObjectData;
use sex_3d::Vec3;

fn run(d: &mut Doc, id: &str, arg: &str) {
    d.run(&Op::with(id, arg));
}

fn mesh_of(d: &Doc, o: usize) -> usize {
    let ObjectData::Mesh(mi) = d.scene.objects[o].data else { panic!("not a mesh") };
    mi
}

/// An ico sphere in place of the startup cube, selected and active.
fn sphere(d: &mut Doc) -> usize {
    if let Some(c) = d.scene.objects.iter().position(|o| o.name == "Cube") {
        d.scene.remove_object(c);
    }
    let i = d.scene.add_mesh_object("Sphere", sex_3d::primitives::ico_sphere(3, 1.0));
    d.select_only(Some(i));
    d.touch();
    i
}

fn view_center(app: &App) -> (i32, i32) {
    let r = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == crate::editors::EditorKind::View3D).unwrap().rect;
    (r.x + r.w / 2, r.y + r.h / 2)
}

fn drag(app: &mut App, (x, y): (i32, i32), dx: i32) {
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    for k in 1..=4 {
        app.frame(Input::Move { x: x + dx * k / 4, y });
    }
    app.frame(Input::Up { x: x + dx, y, button: Button::Left });
}

#[test]
fn workspaces_enter_their_mode() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let c = app.doc.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    app.doc.select_only(Some(c));
    app.doc.want_workspace = Some("Sculpting".into());
    app.frame(Input::Tick);
    assert_eq!(app.doc.mode, Mode::Sculpt);
    app.doc.want_workspace = Some("Texture Paint".into());
    app.frame(Input::Tick);
    assert_eq!(app.doc.mode, Mode::TexturePaint);
    app.doc.want_workspace = Some("Layout".into());
    app.frame(Input::Tick);
    assert_eq!(app.doc.mode, Mode::Object);
}

#[test]
fn sculpt_stroke_in_the_viewport_and_undo() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let o = sphere(&mut app.doc);
    app.doc.want_workspace = Some("Sculpting".into());
    app.frame(Input::Tick);
    assert_eq!(app.doc.mode, Mode::Sculpt);
    run(&mut app.doc, "sculpt.brush_select", "DRAW");
    app.doc.sculpt.set_radius(Mode::Sculpt, 120.0);
    let mi = mesh_of(&app.doc, o);
    let before = app.doc.scene.meshes[mi].positions.clone();
    let c = view_center(&app);
    drag(&mut app, c, 40);
    let after = app.doc.scene.meshes[mi].positions.clone();
    let moved = before.iter().zip(&after).filter(|(a, b)| (**a - **b).length() > 1e-5).count();
    assert!(moved > 0, "the stroke moved vertices");
    // Draw pushes outwards: the moved vertices got further from the centre.
    let out = before.iter().zip(&after).filter(|(a, b)| b.length() > a.length() + 1e-5).count();
    assert!(out * 2 > moved, "{out} of {moved} moved outwards");
    // One undo step restores the exact positions.
    run(&mut app.doc, "ed.undo", "");
    assert_eq!(app.doc.scene.meshes[mi].positions, before);
    run(&mut app.doc, "ed.redo", "");
    assert_eq!(app.doc.scene.meshes[mi].positions, after);
}

#[test]
fn mask_keys_and_ops() {
    let mut d = Doc::default();
    let o = sphere(&mut d);
    run(&mut d, "object.mode_set", "SCULPT");
    let mi = mesh_of(&d, o);
    run(&mut d, "paint.mask_flood_fill", "VALUE 1");
    assert!(d.scene.meshes[mi].mask.iter().all(|&m| m == 1.0));
    run(&mut d, "paint.mask_flood_fill", "INVERT");
    assert!(d.scene.meshes[mi].mask.iter().all(|&m| m == 0.0));
    run(&mut d, "ed.undo", "");
    assert!(d.scene.meshes[mi].mask.iter().all(|&m| m == 1.0));
    // Face sets from the loose parts give one set; hiding then showing.
    run(&mut d, "sculpt.face_sets_init", "LOOSE_PARTS");
    run(&mut d, "sculpt.face_set_change_visibility", "INVERT");
    run(&mut d, "sculpt.face_set_change_visibility", "SHOW");
    assert!(d.scene.meshes[mi].hidden_faces.iter().all(|h| !h));
}

#[test]
fn vertex_paint_fill_and_undo() {
    let mut d = Doc::default();
    let c = d.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    d.select_only(Some(c));
    run(&mut d, "object.mode_set", "VERTEX_PAINT");
    assert_eq!(d.mode, Mode::VertexPaint);
    run(&mut d, "paint.color_set", "1 0 0");
    run(&mut d, "paint.vertex_color_set", "");
    let mi = mesh_of(&d, c);
    let m = &d.scene.meshes[mi];
    assert_eq!(m.color_attributes.len(), 1);
    assert!(m.color_attributes[0].data.iter().all(|p| p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01));
    run(&mut d, "ed.undo", "");
    assert!(d.scene.meshes[mi].color_attributes[0].data.iter().all(|p| p[1] > 0.99));
    // Attributes: add a point one, make it active and the render one.
    run(&mut d, "geometry.color_attribute_add", "Paint\tPOINT");
    let m = &d.scene.meshes[mi];
    assert_eq!(m.color_attributes[1].domain, AttrDomain::Point);
    assert_eq!(m.color_attributes[1].data.len(), m.positions.len());
    assert_eq!(m.active_color, 1);
    run(&mut d, "geometry.color_attribute_render_set", "1");
    assert_eq!(d.scene.meshes[mi].render_color, 1);
    run(&mut d, "geometry.color_attribute_remove", "0");
    assert_eq!(d.scene.meshes[mi].color_attributes.len(), 1);
}

#[test]
fn texture_paint_slot_and_viewport_stroke() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let c = app.doc.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    app.doc.select_only(Some(c));
    run(&mut app.doc, "object.mode_set", "TEXTURE_PAINT");
    run(&mut app.doc, "paint.add_texture_paint_slot", "Base Color");
    let slots = crate::sculpt::texture_slots(&app.doc, c);
    assert_eq!(slots.len(), 1);
    let img = slots[0].image;
    assert!(app.doc.sculpt.dirty_images.contains(&img));
    app.doc.sculpt.dirty_images.clear();
    run(&mut app.doc, "paint.color_set", "0 0 1");
    app.doc.sculpt.tp.radius = 80.0;
    app.doc.sculpt.tp.strength = 1.0;
    let before = app.doc.scene.images[img].pixels.clone();
    app.frame(Input::Tick);
    let center = view_center(&app);
    drag(&mut app, center, 30);
    let px = &app.doc.scene.images[img].pixels;
    let blue = px.iter().filter(|p| p[2] > 0.9 && p[0] < 0.1).count();
    assert!(blue > 100, "{blue} texels painted blue");
    assert!(app.doc.sculpt.dirty_images.contains(&img), "painting dirties the image");
    run(&mut app.doc, "ed.undo", "");
    assert_eq!(app.doc.scene.images[img].pixels, before);
}

#[test]
fn multires_operators() {
    let mut d = Doc::default();
    let c = d.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    d.select_only(Some(c));
    run(&mut d, "object.modifier_add", "MULTIRES");
    let levels = |d: &Doc| match &d.scene.objects[c].modifiers[0] {
        Modifier::Multires { levels, sculpt_levels, render_levels, data, .. } => (*levels, *sculpt_levels, *render_levels, data.total_levels()),
        _ => panic!(),
    };
    assert_eq!(levels(&d), (0, 0, 0, 0));
    run(&mut d, "object.multires_subdivide", "CATMULL_CLARK");
    run(&mut d, "object.multires_subdivide", "SIMPLE");
    assert_eq!(levels(&d), (2, 2, 2, 2));
    let ev = d.scene.evaluated_mesh(c).unwrap();
    assert_eq!(ev.faces.len(), 6 * 16);
    run(&mut d, "ed.undo", "");
    assert_eq!(levels(&d).3, 1);
    run(&mut d, "ed.redo", "");
    // Delete Higher keeps the sculpt level and below.
    if let Modifier::Multires { sculpt_levels, .. } = &mut d.scene.objects[c].modifiers[0] {
        *sculpt_levels = 1;
    }
    run(&mut d, "object.multires_higher_levels_delete", "");
    assert_eq!(levels(&d).3, 1);
    // Apply Base on an undisplaced level leaves the base where the limit
    // surface asks: the shape stays a cube-ish blob of 8 verts.
    let mi = mesh_of(&d, c);
    run(&mut d, "object.multires_base_apply", "");
    assert_eq!(d.scene.meshes[mi].positions.len(), 8);
    // Unsubdivide a mesh that is a subdivided cube: a level appears below.
    let mut d = Doc::default();
    let c = d.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    let mi = mesh_of(&d, c);
    d.scene.meshes[mi] = sex_3d::modifiers::subdivide_simple(&d.scene.meshes[mi], 1);
    d.select_only(Some(c));
    run(&mut d, "object.modifier_add", "MULTIRES");
    run(&mut d, "object.multires_unsubdivide", "");
    assert_eq!(d.scene.meshes[mi].positions.len(), 8, "{}", d.report);
    assert_eq!(levels(&d).3, 1);
}

#[test]
fn poly_round_trip_keeps_sculpt_and_paint_data() {
    let mut d = Doc::default();
    let o = sphere(&mut d);
    let mi = mesh_of(&d, o);
    {
        let m = &mut d.scene.meshes[mi];
        m.mask = (0..m.positions.len()).map(|v| (v % 7) as f32 / 6.0).collect();
        m.face_sets = (0..m.faces.len()).map(|f| 1 + (f % 3) as i32).collect();
        m.hidden_faces = (0..m.faces.len()).map(|f| f % 5 == 0).collect();
        let a = m.add_color_attribute("Col", AttrDomain::Point, [0.25, 0.5, 0.75, 1.0]);
        m.color_attributes[a].data[3] = [1.0, 0.0, 0.0, 1.0];
        m.add_color_attribute("Corner", AttrDomain::Corner, [0.1, 0.2, 0.3, 1.0]);
        m.active_color = 1;
        m.render_color = 0;
    }
    run(&mut d, "object.modifier_add", "MULTIRES");
    run(&mut d, "object.multires_subdivide", "LINEAR");
    if let Modifier::Multires { data, .. } = &mut d.scene.objects[o].modifiers[0] {
        let base = d.scene.meshes[mi].clone();
        let mut lv = data.level_mesh(&base, 1);
        lv.positions[5] += Vec3::new(0.0, 0.0, 0.3);
        assert!(data.store(&base, 1, &lv.positions));
    }
    let mut img = sex_3d::scene::Image::new("Painted", 4, 2);
    img.pixels[3] = [1.0, 0.0, 0.0, 1.0];
    img.pixels[6] = [0.0, 1.0, 0.0, 1.0];
    d.scene.images.push(img);

    let text = crate::persist::save(&d);
    let e = crate::persist::load(&text).unwrap();
    let a = &d.scene.meshes[mi];
    let emi = e.scene.mesh_names.iter().position(|n| *n == d.scene.mesh_names[mi]).unwrap();
    let b = &e.scene.meshes[emi];
    assert_eq!(a.mask, b.mask);
    assert_eq!(a.face_sets, b.face_sets);
    assert_eq!(a.hidden_faces, b.hidden_faces);
    assert_eq!(a.color_attributes, b.color_attributes);
    assert_eq!((b.active_color, b.render_color), (1, 0));
    let eo = e.scene.objects.iter().position(|x| x.name == "Sphere").unwrap();
    assert_eq!(e.scene.objects[eo].modifiers, d.scene.objects[o].modifiers);
    assert_eq!(e.scene.images.len(), 1);
    let ei = &e.scene.images[0];
    assert_eq!((ei.name.as_str(), ei.width, ei.height), ("Painted", 4, 2));
    for (p, q) in d.scene.images[0].pixels.iter().zip(&ei.pixels) {
        for k in 0..4 {
            assert!((p[k] - q[k]).abs() < 0.01, "{p:?} {q:?}");
        }
    }
    // Saving again gives the same text.
    assert_eq!(crate::persist::save(&e), text);
}

#[test]
fn texture_slot_image_survives_a_poly_round_trip() {
    let mut d = Doc::default();
    let c = d.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    d.select_only(Some(c));
    run(&mut d, "paint.add_texture_paint_slot", "Base Color");
    let text = crate::persist::save(&d);
    let e = crate::persist::load(&text).unwrap();
    let slots = crate::sculpt::texture_slots(&e, c);
    assert_eq!(slots.len(), 1, "the material's image node still names the image");
    assert_eq!(e.scene.images[slots[0].image].width, 1024);
}

#[test]
fn image_editor_paints_in_2d_and_saves() {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    let c = app.doc.scene.objects.iter().position(|o| o.name == "Cube").unwrap();
    app.doc.select_only(Some(c));
    run(&mut app.doc, "paint.add_texture_paint_slot", "Base Color");
    let img = crate::sculpt::texture_slots(&app.doc, c)[0].image;
    app.doc.want_workspace = Some("Texture Paint".into());
    app.frame(Input::Tick);
    assert_eq!(app.doc.mode, Mode::TexturePaint);
    // The Image Editor shows the slot's texture; switch it to Paint.
    let mut rect = None;
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let crate::editors::Editor::Image(s) = &mut a.editor {
            if s.kind == crate::editors::EditorKind::ImageEditor {
                s.paint = true;
                rect = Some(a.rect);
            }
        }
    }
    let r = rect.expect("an Image Editor");
    run(&mut app.doc, "paint.color_set", "1 0 0");
    app.doc.sculpt.tp.radius = 30.0;
    app.doc.sculpt.tp.strength = 1.0;
    app.doc.sculpt.dirty_images.clear();
    app.frame(Input::Tick);
    let before = app.doc.scene.images[img].pixels.clone();
    drag(&mut app, (r.x + r.w / 2, r.y + r.h / 2), 40);
    let px = &app.doc.scene.images[img].pixels;
    let red = px.iter().filter(|p| p[0] > 0.9 && p[1] < 0.1).count();
    assert!(red > 100, "{red} texels painted red");
    // The middle of the image, under the press, is red.
    let (w, h) = (app.doc.scene.images[img].width as usize, app.doc.scene.images[img].height as usize);
    assert!(px[(h / 2) * w + w / 2][0] > 0.9);
    assert!(app.doc.sculpt.dirty_images.contains(&img));
    run(&mut app.doc, "ed.undo", "");
    assert_eq!(app.doc.scene.images[img].pixels, before);
    // Image > Save writes a PNG and clears the dirty mark.
    let dir = std::env::temp_dir().join(format!("poly-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tex.png");
    app.doc.sculpt.dirty_images.insert(img);
    run(&mut app.doc, "image.save", &format!("{img}\t{}", path.display()));
    let bytes = std::fs::read(&path).unwrap();
    let back = sex_3d::io::decode_image("tex.png", &bytes).unwrap();
    assert_eq!((back.width as usize, back.height as usize), (w, h));
    assert!(!app.doc.sculpt.dirty_images.contains(&img));
    let _ = std::fs::remove_dir_all(&dir);
}
