// SPDX-License-Identifier: GPL-3.0-only
//! Host tests driving the Properties editor through the whole window.
use super::{State, Tab};
use crate::draw::Rect;
use crate::editors::Editor;
use crate::input::{Button, Input, Key, Mods};
use crate::ui::MenuItem;
use crate::App;
use sex_3d::modifiers::Modifier;
use sex_3d::scene::ObjectData;

fn app() -> App {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    app
}

fn props(app: &App) -> &State {
    for a in app.workspaces[app.active_ws].areas() {
        if let Editor::Properties(s) = &a.editor {
            return s;
        }
    }
    panic!("no properties editor")
}

fn props_mut(app: &mut App) -> &mut State {
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::Properties(s) = &mut a.editor {
            return s;
        }
    }
    panic!("no properties editor")
}

fn click(app: &mut App, x: i32, y: i32) {
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Up { x, y, button: Button::Left });
}

fn click_rect(app: &mut App, r: Rect) {
    let (x, y) = r.center();
    click(app, x, y);
}

fn type_text(app: &mut App, s: &str) {
    for c in s.chars() {
        app.frame(Input::Key { key: Key::Char(c), mods: Mods::NONE, text: Some(c) });
    }
    app.frame(Input::key(Key::Enter));
}

fn field(app: &App, key: &str) -> Rect {
    props(app).field(key).unwrap_or_else(|| panic!("field {key} in {:?}", props(app).fields.iter().map(|f| &f.0).collect::<Vec<_>>()))
}

fn open_tab(app: &mut App, tab: Tab) {
    let r = props(app).tab_rects.iter().find(|t| t.0 == tab).unwrap_or_else(|| panic!("tab {tab:?}")).1;
    click_rect(app, r);
    assert_eq!(props(app).tab, tab);
}

/// Type `text` into the number field `key` (click, type, Enter).
fn set_field(app: &mut App, key: &str, text: &str) {
    let r = field(app, key);
    click_rect(app, r);
    type_text(app, text);
}

fn menu_item_y(app: &mut App, pop: &crate::ui::Popup, label: &str) -> (i32, i32) {
    let pr = pop.rect(&mut app.painter);
    let mut y = pr.y + 2 + if pop.title.is_empty() { 0 } else { 24 };
    for it in &pop.items {
        let h = if matches!(it, MenuItem::Separator) { 7 } else { 22 };
        let l = match it {
            MenuItem::Op(l, _, _) | MenuItem::Sub(l, _) | MenuItem::Check(l, _, _) | MenuItem::Heading(l) => l.as_str(),
            MenuItem::Separator => "",
        };
        if l == label {
            return (pr.x + 40, y + h / 2);
        }
        y += h;
    }
    panic!("no menu item {label}")
}

#[test]
fn tabs_follow_the_active_object_type() {
    let mut app = app();
    let tabs: Vec<Tab> = props(&app).tab_rects.iter().map(|t| t.0).collect();
    assert_eq!(
        tabs,
        [Tab::Tool, Tab::Render, Tab::Output, Tab::ViewLayer, Tab::Scene, Tab::World, Tab::Collection, Tab::Object, Tab::Modifiers, Tab::Particles, Tab::Physics, Tab::Constraints, Tab::Data, Tab::Material, Tab::Texture]
    );
    let light = app.doc.scene.find_object("Light").unwrap();
    app.doc.select_only(Some(light));
    app.frame(Input::Tick);
    let tabs: Vec<Tab> = props(&app).tab_rects.iter().map(|t| t.0).collect();
    assert!(!tabs.contains(&Tab::Modifiers) && !tabs.contains(&Tab::Material) && tabs.contains(&Tab::Data));
}

#[test]
fn location_x_typed_moves_the_object() {
    let mut app = app();
    let cube = app.doc.active().unwrap();
    let gen = app.doc.generation;
    set_field(&mut app, "loc0", "3");
    assert_eq!(app.doc.scene.objects[cube].transform.location.x, 3.0);
    assert!(app.doc.generation > gen, "viewports refresh");
    // One undo step.
    app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
    assert_eq!(app.doc.scene.objects[cube].transform.location.x, 0.0);
    // Rotation in degrees, XYZ Euler.
    set_field(&mut app, "rot2", "90");
    let e = sex_3d::script::quat_to_euler_xyz(app.doc.scene.objects[cube].transform.rotation);
    assert!((e[2].to_degrees() - 90.0).abs() < 1e-3, "{e:?}");
    // Dragging a field is one undo step too.
    let r = field(&app, "scale0");
    let (x, y) = r.center();
    let undo_before = app.doc.undo.undo_label().map(String::from);
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    for dx in [5, 10, 20, 30] {
        app.frame(Input::Move { x: x + dx, y });
    }
    app.frame(Input::Up { x: x + 30, y, button: Button::Left });
    assert!((app.doc.scene.objects[cube].transform.scale.x - 1.15).abs() < 1e-4);
    app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
    assert_eq!(app.doc.scene.objects[cube].transform.scale.x, 1.0);
    assert_eq!(app.doc.undo.undo_label().map(String::from), undo_before);
}

#[test]
fn add_modifier_menu_adds_subdivision_and_levels_edit_it() {
    let mut app = app();
    open_tab(&mut app, Tab::Modifiers);
    let r = field(&app, "add_modifier");
    click_rect(&mut app, r);
    let pop = app.popup.clone().expect("Add Modifier menu");
    let (x, y) = menu_item_y(&mut app, &pop, "Generate");
    app.frame(Input::Move { x, y });
    let sub = app.popup.as_ref().unwrap().sub.as_ref().expect("Generate submenu").1.as_ref().clone();
    let (x, y) = menu_item_y(&mut app, &sub, "Subdivision Surface");
    click(&mut app, x, y);
    let cube = app.doc.active().unwrap();
    assert_eq!(app.doc.scene.objects[cube].modifiers, vec![Modifier::Subdivision { levels: 1, simple: false }]);
    set_field(&mut app, "m0/levels", "3");
    assert_eq!(app.doc.scene.objects[cube].modifiers, vec![Modifier::Subdivision { levels: 3, simple: false }]);
    // Unavailable modifiers report instead of doing nothing.
    let r = field(&app, "add_modifier");
    click_rect(&mut app, r);
    let pop = app.popup.clone().unwrap();
    let (x, y) = menu_item_y(&mut app, &pop, "Generate");
    app.frame(Input::Move { x, y });
    let sub = app.popup.as_ref().unwrap().sub.as_ref().unwrap().1.as_ref().clone();
    let (x, y) = menu_item_y(&mut app, &sub, "Skin");
    click(&mut app, x, y);
    assert!(app.doc.report.contains("Skin modifier is not available"), "{}", app.doc.report);
    assert_eq!(app.doc.scene.objects[cube].modifiers.len(), 1);
}

#[test]
fn ported_modifiers_menu_panels_and_object_fields() {
    use sex_3d::modifiers::{BooleanOp, DecimateMode, SimpleDeformMode};
    let mut app = app();
    let cube = app.doc.active().unwrap();
    app.doc.scene.add_mesh_object("Cutter", sex_3d::primitives::cube(1.0));
    open_tab(&mut app, Tab::Modifiers);
    // Deform > Simple Deform from the Add Modifier menu.
    let r = field(&app, "add_modifier");
    click_rect(&mut app, r);
    let pop = app.popup.clone().unwrap();
    let (x, y) = menu_item_y(&mut app, &pop, "Deform");
    app.frame(Input::Move { x, y });
    let sub = app.popup.as_ref().unwrap().sub.as_ref().expect("Deform submenu").1.as_ref().clone();
    let (x, y) = menu_item_y(&mut app, &sub, "Simple Deform");
    click(&mut app, x, y);
    assert!(matches!(app.doc.scene.objects[cube].modifiers[0], Modifier::SimpleDeform { mode: SimpleDeformMode::Twist, axis: 2, .. }));
    super::object::modifier_op(&mut app.doc, cube, "object.modifier_remove", 0);
    // Decimate's ratio and Boolean's object dropdown.
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "DECIMATE"));
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "BOOLEAN"));
    app.frame(Input::Tick);
    set_field(&mut app, "m0/ratio", "0.5");
    assert!(matches!(app.doc.scene.objects[cube].modifiers[0], Modifier::Decimate { mode: DecimateMode::Collapse, ratio, .. } if ratio == 0.5));
    let r = field(&app, "modobj/1");
    click_rect(&mut app, r);
    let pop = app.popup.clone().expect("object dropdown");
    let (x, y) = menu_item_y(&mut app, &pop, "Cutter");
    click(&mut app, x, y);
    app.frame(Input::Tick);
    assert_eq!(app.doc.scene.objects[cube].modifiers[1], Modifier::Boolean { object: "Cutter".into(), operation: BooleanOp::Difference });
    // Renaming the operand keeps the modifier pointing at it.
    let cutter = app.doc.scene.find_object("Cutter").unwrap();
    super::rename_object(&mut app.doc, cutter, "Knife");
    assert_eq!(app.doc.scene.objects[cube].modifiers[1].object_ref(), Some("Knife"));
    // Every ported modifier adds and draws (one at a time: stacked, they
    // multiply the geometry).
    for id in ["REMESH", "SCREW", "WIREFRAME", "EDGE_SPLIT", "BUILD", "CAST", "WAVE", "SHRINKWRAP", "LAPLACIANSMOOTH", "CORRECTIVE_SMOOTH", "WEIGHTED_NORMAL"] {
        app.doc.run(&crate::ui::Op::with("object.modifier_add", id));
        app.frame(Input::Tick);
        assert_eq!(app.doc.scene.objects[cube].modifiers.len(), 3, "{id}");
        super::object::modifier_op(&mut app.doc, cube, "object.modifier_remove", 2);
    }
    // Wave follows the timeline's frame.
    app.doc.frame = 40;
    app.frame(Input::Tick);
    assert_eq!(app.doc.scene.frame_current, 40);
}

#[test]
fn modifier_card_toggle_and_menu_ops() {
    let mut app = app();
    let cube = app.doc.active().unwrap();
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "SUBSURF"));
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "TRIANGULATE"));
    open_tab(&mut app, Tab::Modifiers);
    // Show in Viewport off parks the modifier; the stack shown keeps it.
    super::object::modifier_op(&mut app.doc, cube, "object.modifier_move_up", 1);
    assert!(matches!(app.doc.scene.objects[cube].modifiers[0], Modifier::Triangulate));
    app.doc.settings.toggle_modifier(&mut app.doc.scene, cube, 0);
    assert_eq!(app.doc.scene.objects[cube].modifiers.len(), 1);
    assert_eq!(app.doc.settings.shown_stack(&app.doc.scene, cube).len(), 2);
    super::object::modifier_op(&mut app.doc, cube, "object.modifier_remove", 1);
    assert!(app.doc.scene.objects[cube].modifiers.is_empty());
    assert_eq!(app.doc.settings.shown_stack(&app.doc.scene, cube).len(), 1);
    // Apply bakes into the mesh.
    app.doc.settings.toggle_modifier(&mut app.doc.scene, cube, 0);
    super::object::modifier_op(&mut app.doc, cube, "object.modifier_apply", 0);
    let ObjectData::Mesh(mi) = app.doc.scene.objects[cube].data else { panic!() };
    assert_eq!(app.doc.scene.meshes[mi].face_count(), 12);
    assert!(app.doc.scene.objects[cube].modifiers.is_empty());
}

#[test]
fn material_base_color_edit_changes_the_material() {
    let mut app = app();
    open_tab(&mut app, Tab::Material);
    let r = field(&app, "Base Color");
    click_rect(&mut app, r);
    let pr = props(&app).picker.as_ref().expect("colour picker open").rect;
    // R slider below the wheel and the RGB/HSV switch.
    let ry = pr.y + 12 + 124 + 10 + 26 + 10;
    click(&mut app, pr.x + pr.w / 2, ry);
    type_text(&mut app, "0.25");
    let mi = app.doc.scene.objects[app.doc.active().unwrap()].materials[0];
    assert_eq!(app.doc.scene.materials[mi].base_color[0], 0.25);
    assert_eq!(app.doc.scene.materials[mi].base_color[1], 0.8);
    // Clicking the wheel's centre makes it grey at the current value.
    click(&mut app, pr.x + 14 + 62, pr.y + 12 + 62);
    let c = app.doc.scene.materials[mi].base_color;
    assert!((c[0] - c[1]).abs() < 0.02 && (c[1] - c[2]).abs() < 0.02, "{c:?}");
    // Esc closes the picker; then Roughness typed.
    app.frame(Input::key(Key::Esc));
    assert!(props(&app).picker.is_none());
    set_field(&mut app, "roughness", "0.1");
    assert!((app.doc.scene.materials[mi].roughness - 0.1).abs() < 1e-6);
    app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
    assert_eq!(app.doc.scene.materials[mi].roughness, 0.5);
}

#[test]
fn render_samples_drive_f12() {
    let mut app = app();
    open_tab(&mut app, Tab::Render);
    set_field(&mut app, "render_samples", "2");
    assert_eq!(app.doc.settings.render_samples, 2);
    open_tab(&mut app, Tab::Output);
    set_field(&mut app, "res_pct", "5");
    assert_eq!(app.doc.settings.output_size(), (96, 54));
    app.frame(Input::key(Key::F(12)));
    let img = app.doc.render.as_ref().expect("rendered");
    assert_eq!((img.width, img.height), (96, 54));
    assert!(app.doc.report.contains("2 samples"), "{}", app.doc.report);
}

#[test]
fn light_and_camera_data() {
    let mut app = app();
    let light = app.doc.scene.find_object("Light").unwrap();
    app.doc.select_only(Some(light));
    app.frame(Input::Tick);
    open_tab(&mut app, Tab::Data);
    set_field(&mut app, "power", "500");
    let ObjectData::Light(l) = app.doc.scene.objects[light].data else { panic!() };
    assert_eq!(l.power, 500.0);
    let cam = app.doc.scene.find_object("Camera").unwrap();
    app.doc.select_only(Some(cam));
    app.frame(Input::Tick);
    set_field(&mut app, "focal", "35");
    let ObjectData::Camera(c) = app.doc.scene.objects[cam].data else { panic!() };
    let f = super::data::focal_from_fov_y(c.fov_y, 36.0, 16.0 / 9.0);
    assert!((f - 35.0).abs() < 1e-3, "{f}");
}

/// Pick `value` for the dropdown `key` as its menu would.
fn pick(app: &mut App, key: &str, value: &str) {
    app.doc.queue.push(crate::ui::Op::with("properties.set", &format!("{key}={value}")));
    app.frame(Input::Tick);
}

#[test]
fn render_tab_sampling_and_color_management() {
    use sex_3d::render::{Look, ViewTransform};
    let mut app = app();
    open_tab(&mut app, Tab::Render);
    set_field(&mut app, "min_samples", "8");
    set_field(&mut app, "time_limit", "3");
    set_field(&mut app, "threshold", "0.05");
    let st = &app.doc.settings;
    assert_eq!((st.adaptive_min_samples, st.time_limit), (8, 3.0));
    let rs = st.render_settings();
    assert_eq!((rs.min_samples, rs.time_limit, rs.noise_threshold), (8, 3.0, 0.05));
    pick(&mut app, "view_transform", "Khronos PBR Neutral");
    pick(&mut app, "look", "Punchy");
    let cm = app.doc.settings.color_management();
    assert_eq!((cm.view, cm.look), (ViewTransform::KhronosPbrNeutral, Look::Punchy));
    app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
    assert_eq!(app.doc.settings.look, "None", "look change is one undo step");
    assert_eq!(app.doc.settings.view_transform, ViewTransform::KhronosPbrNeutral);
}

#[test]
fn ctrl_f12_renders_the_frame_range_to_files() {
    let mut app = app();
    let dir = std::env::temp_dir().join(format!("polyamour-anim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let st = &mut app.doc.settings;
    st.output_path = format!("{}/f", dir.display());
    st.resolution_pct = 2;
    st.render_samples = 1;
    st.motion_blur = true;
    app.doc.scene.frame_start = 3;
    app.doc.scene.frame_end = 4;
    app.frame(Input::key_mods(Key::F(12), Mods::CTRL));
    assert!(app.doc.report.contains("Rendered 2 frames"), "{}", app.doc.report);
    for f in ["f0003.png", "f0004.png"] {
        let bytes = std::fs::read(dir.join(f)).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
    }
    assert!(app.doc.render.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn camera_lens_types_and_area_light_shape() {
    use sex_3d::scene::{AreaShape, LightKind, PanoramaType};
    let mut app = app();
    let cam = app.doc.scene.find_object("Camera").unwrap();
    app.doc.select_only(Some(cam));
    app.frame(Input::Tick);
    open_tab(&mut app, Tab::Data);
    set_field(&mut app, "shift_x", "0.1");
    pick(&mut app, "lens_type", "Panoramic");
    pick(&mut app, "panorama_type", "Fisheye Equidistant");
    pick(&mut app, "sensor_fit", "Vertical");
    let ObjectData::Camera(c) = app.doc.scene.objects[cam].data else { panic!() };
    assert_eq!(c.panorama, Some(PanoramaType::FisheyeEquidistant));
    assert_eq!(c.shift_x, 0.1);
    assert_eq!(c.sensor_fit, sex_3d::scene::SensorFit::Vertical);
    pick(&mut app, "lens_type", "Orthographic");
    let ObjectData::Camera(c) = app.doc.scene.objects[cam].data else { panic!() };
    assert!(c.panorama.is_none() && c.ortho_scale.is_some());

    let light = app.doc.scene.find_object("Light").unwrap();
    if let ObjectData::Light(l) = &mut app.doc.scene.objects[light].data {
        l.kind = LightKind::Area { size: 1.0 };
    }
    app.doc.select_only(Some(light));
    app.frame(Input::Tick);
    pick(&mut app, "area_shape", "Rectangle");
    set_field(&mut app, "area_size_y", "0.25");
    let ObjectData::Light(l) = app.doc.scene.objects[light].data else { panic!() };
    assert_eq!((l.shape, l.size_y), (AreaShape::Rectangle, 0.25));
}

#[test]
fn search_hides_panels_without_matches() {
    let mut app = app();
    props_mut(&mut app).search = "scale".into();
    app.frame(Input::Tick);
    let keys: Vec<String> = props(&app).fields.iter().map(|f| f.0.clone()).collect();
    assert!(keys.contains(&"scale0".to_string()));
    assert!(!keys.contains(&"Parent".to_string()), "{keys:?}");
}

/// Screenshots of every tab for visual review:
/// `SHOT_DIR=/some/dir cargo test -p polyamour shots -- --ignored`.
#[test]
#[ignore]
fn shots() {
    let Ok(dir) = std::env::var("SHOT_DIR") else { return };
    let save = |app: &App, name: &str| {
        let f = std::fs::File::create(format!("{dir}/{name}.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), app.painter.width as u32, app.painter.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().unwrap().write_image_data(&app.painter.to_rgba()).unwrap();
    };
    let mut app = app();
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "SUBSURF"));
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "MIRROR"));
    app.frame(Input::Tick);
    let tabs: Vec<Tab> = props(&app).tab_rects.iter().map(|t| t.0).collect();
    for t in tabs {
        props_mut(&mut app).tab = t;
        app.frame(Input::Tick);
        save(&app, &format!("tab_{}", t.label().replace(' ', "_")));
    }
    props_mut(&mut app).tab = Tab::Material;
    app.frame(Input::Tick);
    let r = field(&app, "Base Color");
    click_rect(&mut app, r);
    save(&app, "picker");
    app.frame(Input::key(Key::Esc));
    for name in ["Light", "Camera"] {
        let o = app.doc.scene.find_object(name).unwrap();
        app.doc.select_only(Some(o));
        props_mut(&mut app).tab = Tab::Data;
        app.frame(Input::Tick);
        save(&app, &format!("data_{name}"));
    }
}
