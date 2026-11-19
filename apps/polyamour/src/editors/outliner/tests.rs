// SPDX-License-Identifier: GPL-3.0-only
//! Host tests driving the Outliner through the whole window.
use super::{RowKey, State};
use crate::draw::Rect;
use crate::editors::{Editor, EditorKind};
use crate::input::{Button, Input, Key, Mods};
use crate::App;

fn app() -> App {
    let mut app = App::new(1600, 900);
    app.frame(Input::Tick);
    // Debug-build frames are slow; keep double-clicks from timing out.
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::Outliner(s) = &mut a.editor {
            s.double_click = std::time::Duration::from_secs(60);
        }
    }
    app
}

fn area_rect(app: &App, kind: EditorKind) -> Rect {
    app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == kind).expect("area").rect
}

fn outliner(app: &App) -> &State {
    for a in app.workspaces[app.active_ws].areas() {
        if let Editor::Outliner(s) = &a.editor {
            return s;
        }
    }
    panic!("no outliner")
}

/// The body rectangle and the row rectangle showing `label`.
fn row_rect(app: &App, label: &str) -> Rect {
    let r = area_rect(app, EditorKind::Outliner);
    let body_y = r.y + crate::theme::HEADER_H + 1;
    let s = outliner(app);
    let i = s.rows.iter().position(|x| x.label == label).unwrap_or_else(|| panic!("row {label} in {:?}", s.rows.iter().map(|r| &r.label).collect::<Vec<_>>()));
    Rect::new(r.x, body_y + i as i32 * super::ROW_H, r.w, super::ROW_H)
}

fn click_mods(app: &mut App, x: i32, y: i32, button: Button, mods: Mods) {
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button, mods });
    app.frame(Input::Up { x, y, button });
}

fn click(app: &mut App, x: i32, y: i32) {
    click_mods(app, x, y, Button::Left, Mods::NONE);
}

fn type_text(app: &mut App, s: &str) {
    for c in s.chars() {
        app.frame(Input::Key { key: Key::Char(c.to_ascii_lowercase()), mods: Mods::NONE, text: Some(c) });
    }
    app.frame(Input::key(Key::Enter));
}

fn click_row(app: &mut App, label: &str) {
    let rr = row_rect(app, label);
    click(app, rr.x + 80, rr.y + rr.h / 2);
}

#[test]
fn rows_follow_blenders_view_layer_tree() {
    let app = app();
    let labels: Vec<&str> = outliner(&app).rows.iter().map(|r| r.label.as_str()).collect();
    // Alphabetical objects under the default collection.
    assert_eq!(labels, ["Scene Collection", "Collection", "Camera", "Cube", "Light"]);
}

#[test]
fn clicking_a_row_selects_and_activates() {
    let mut app = app();
    click_row(&mut app, "Light");
    let light = app.doc.scene.find_object("Light").unwrap();
    assert_eq!(app.doc.active(), Some(light));
    assert_eq!(app.doc.scene.selected, vec![light]);
    // Ctrl+click extends; on SexOS Ctrl+left arrives as a middle press with Ctrl.
    let rr = row_rect(&app, "Cube");
    click_mods(&mut app, rr.x + 80, rr.y + 10, Button::Middle, Mods::CTRL);
    let cube = app.doc.scene.find_object("Cube").unwrap();
    assert_eq!(app.doc.active(), Some(cube));
    assert!(app.doc.is_selected(light) && app.doc.is_selected(cube));
    // Shift+click selects the range from the active row.
    let rr = row_rect(&app, "Camera");
    click_mods(&mut app, rr.x + 80, rr.y + 10, Button::Left, Mods::SHIFT);
    assert_eq!(app.doc.scene.selected.len(), 3);
}

#[test]
fn eye_toggle_hides_and_undoes() {
    let mut app = app();
    let r = area_rect(&app, EditorKind::Outliner);
    let rr = row_rect(&app, "Cube");
    // Columns: exclude checkbox, then the eye at the far right.
    let eye_x = r.right() - 6 - super::COL_W / 2;
    click(&mut app, eye_x, rr.y + rr.h / 2);
    let cube = app.doc.scene.find_object("Cube").unwrap();
    assert!(!app.doc.scene.objects[cube].visible);
    assert!(!app.doc.scene.is_visible(cube));
    app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
    assert!(app.doc.scene.objects[cube].visible);
    // The collection's eye hides everything in it; its checkbox excludes it.
    let cr = row_rect(&app, "Collection");
    click(&mut app, eye_x, cr.y + 10);
    assert!(!app.doc.scene.is_visible(cube));
    click(&mut app, eye_x, cr.y + 10);
    assert!(app.doc.scene.is_visible(cube));
    click(&mut app, eye_x - super::COL_W, cr.y + 10);
    assert!(app.doc.settings.excluded.contains(&0));
    assert!(!app.doc.scene.is_visible(cube));
}

#[test]
fn double_click_renames_inline() {
    let mut app = app();
    let rr = row_rect(&app, "Cube");
    click(&mut app, rr.x + 80, rr.y + 10);
    click(&mut app, rr.x + 80, rr.y + 10);
    assert!(outliner(&app).renaming.is_some(), "double-click starts renaming: {:?} {}", outliner(&app).rows.iter().map(|r| &r.label).collect::<Vec<_>>(), app.doc.report);
    type_text(&mut app, "Box");
    assert!(app.doc.scene.find_object("Box").is_some(), "{:?}", app.doc.scene.objects.iter().map(|o| &o.name).collect::<Vec<_>>());
    assert!(outliner(&app).renaming.is_none());
    // F2 renames the active object too, and names stay unique.
    app.frame(Input::Move { x: rr.x + 80, y: rr.y + 10 });
    app.frame(Input::key(Key::F(2)));
    type_text(&mut app, "Light");
    let names: Vec<&str> = app.doc.scene.objects.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"Light.001"), "{names:?}");
}

#[test]
fn keys_select_all_hide_and_delete() {
    let mut app = app();
    let rr = row_rect(&app, "Cube");
    app.frame(Input::Move { x: rr.x + 80, y: rr.y + 10 });
    app.frame(Input::key(Key::Char('a')));
    assert_eq!(app.doc.scene.selected.len(), 3);
    // A again with everything selected deselects (Alt+A stand-in).
    app.frame(Input::key(Key::Char('a')));
    assert!(app.doc.scene.selected.is_empty());
    click_row(&mut app, "Light");
    app.frame(Input::key(Key::Char('h')));
    let light = app.doc.scene.find_object("Light").unwrap();
    assert!(!app.doc.scene.objects[light].visible);
    app.frame(Input::key_mods(Key::Char('h'), Mods::SHIFT));
    assert!(app.doc.scene.objects[light].visible);
    // (Another row in between, so this is not a double-click.)
    click_row(&mut app, "Cube");
    click_row(&mut app, "Light");
    app.frame(Input::key(Key::Char('x')));
    assert!(app.doc.scene.find_object("Light").is_none());
    assert_eq!(outliner(&app).rows.len(), 4);
}

#[test]
fn new_collection_and_drag_object_into_it() {
    let mut app = app();
    let r = area_rect(&app, EditorKind::Outliner);
    // Header's New Collection button at the right.
    click(&mut app, r.right() - 15, r.y + 13);
    assert_eq!(app.doc.scene.collections.len(), 2);
    let new = 1;
    assert_eq!(app.doc.scene.collections[new].name, "Collection.001");
    // Drag the Cube row onto the new collection.
    let from = row_rect(&app, "Cube");
    let to = row_rect(&app, "Collection.001");
    app.frame(Input::Move { x: from.x + 80, y: from.y + 10 });
    app.frame(Input::Down { x: from.x + 80, y: from.y + 10, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: from.x + 80, y: from.y + 20 });
    app.frame(Input::Move { x: to.x + 80, y: to.y + 10 });
    app.frame(Input::Up { x: to.x + 80, y: to.y + 10, button: Button::Left });
    let cube = app.doc.scene.find_object("Cube").unwrap();
    assert_eq!(app.doc.scene.objects[cube].collection, new, "{}", app.doc.report);
    // Drag Camera onto Cube: parented, keeping its world position.
    let cam = app.doc.scene.find_object("Camera").unwrap();
    let before = app.doc.scene.world_matrix(cam).transform_point(sex_3d::Vec3::ZERO);
    let from = row_rect(&app, "Camera");
    let to = row_rect(&app, "Cube");
    app.frame(Input::Move { x: from.x + 80, y: from.y + 10 });
    app.frame(Input::Down { x: from.x + 80, y: from.y + 10, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: to.x + 80, y: to.y + 10 });
    app.frame(Input::Up { x: to.x + 80, y: to.y + 10, button: Button::Left });
    assert_eq!(app.doc.scene.objects[cam].parent, Some(cube));
    let after = app.doc.scene.world_matrix(cam).transform_point(sex_3d::Vec3::ZERO);
    assert!(before.approx_eq(after, 1e-3));
}

#[test]
fn context_menu_and_collection_delete() {
    let mut app = app();
    let rr = row_rect(&app, "Cube");
    click_mods(&mut app, rr.x + 80, rr.y + 10, Button::Right, Mods::NONE);
    let pop = app.popup.clone().expect("context menu");
    assert_eq!(pop.title, "Object");
    let cube = app.doc.scene.find_object("Cube").unwrap();
    assert_eq!(app.doc.active(), Some(cube), "right-click selects the row");
    // "Duplicate" is the 7th entry (after a separator of 7 px).
    let pr = pop.rect(&mut app.painter);
    let idx = pop.items.iter().position(|i| matches!(i, crate::ui::MenuItem::Op(l, _, _) if l == "Duplicate")).unwrap();
    let mut y = pr.y + 2 + 24;
    for it in &pop.items[..idx] {
        y += if matches!(it, crate::ui::MenuItem::Separator) { 7 } else { 22 };
    }
    click(&mut app, pr.x + 40, y + 11);
    assert_eq!(app.doc.scene.objects.len(), 4, "{} {:?} {:?}", app.doc.report, app.popup.as_ref().map(|p| &p.title), pr);
    // Deleting the only collection is refused; a new empty one can go.
    let r = area_rect(&app, EditorKind::Outliner);
    click(&mut app, r.right() - 15, r.y + 13);
    assert_eq!(app.doc.scene.collections.len(), 2);
    // Clicking the collection row deselects the objects; Delete removes it.
    click_row(&mut app, "Collection.001");
    assert!(app.doc.scene.selected.is_empty());
    app.frame(Input::key(Key::Delete));
    assert_eq!(app.doc.scene.collections.len(), 1);
    assert_eq!(app.doc.scene.objects.len(), 4);
}

#[test]
fn search_filters_rows_and_period_shows_active() {
    let mut app = app();
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::Outliner(s) = &mut a.editor {
            s.search = "lig".into();
        }
    }
    app.frame(Input::Tick);
    let labels: Vec<&str> = outliner(&app).rows.iter().map(|r| r.label.as_str()).collect();
    // The light object and its light data both match.
    assert_eq!(labels, ["Scene Collection", "Collection", "Light", "Light"]);
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::Outliner(s) = &mut a.editor {
            s.search.clear();
        }
    }
    // Collapse everything, then period reveals the active object.
    let cr = row_rect(&app, "Collection");
    app.frame(Input::Move { x: cr.x + 80, y: cr.y + 10 });
    let r = area_rect(&app, EditorKind::Outliner);
    let arrow_x = r.x + 6 + super::INDENT + 6;
    click(&mut app, arrow_x, cr.y + 10);
    assert_eq!(outliner(&app).rows.len(), 2);
    app.frame(Input::Key { key: Key::Char('.'), mods: Mods::NONE, text: Some('.') });
    app.frame(Input::Tick);
    assert_eq!(outliner(&app).rows.len(), 5);
}

#[test]
fn expanding_an_object_shows_data_modifiers_and_materials() {
    let mut app = app();
    app.doc.run(&crate::ui::Op::with("object.modifier_add", "SUBSURF"));
    app.frame(Input::Tick);
    let rr = row_rect(&app, "Cube");
    let r = area_rect(&app, EditorKind::Outliner);
    // Shift+click on the triangle opens the whole branch.
    click_mods(&mut app, r.x + 6 + 2 * super::INDENT + 6, rr.y + 10, Button::Left, Mods::SHIFT);
    let labels: Vec<&str> = outliner(&app).rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Scene Collection", "Collection", "Camera", "Cube", "Cube", "Material", "Modifiers", "Subdivision", "Light"]);
    assert!(matches!(outliner(&app).rows[5].key, RowKey::Mat(_, 0)));
}
