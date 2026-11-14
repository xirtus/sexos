// SPDX-License-Identifier: GPL-3.0-only
//! Whole-window behaviour: layout, focus, pointer, lists, tabs, menus, the palette,
//! dialogs, translations, painting and the accessibility tree.
use weft::input::{code, Button, Event, Key, Pointer, PointerKind};
use weft::menu::{Command, Menu};
use weft::widget::{Column, Id, Layout, ListModel, Tab};
use weft::{a11y, DialogValue, Response, Strings, TabEvent, Theme, Ui};

fn ui() -> Ui { Ui::new(800, 600, Theme::light(), Strings::new("en")) }

fn press(ui: &mut Ui, k: Key) -> Vec<Response> { ui.handle(Event::Key(k)) }
fn typed(ui: &mut Ui, s: &str) -> Vec<Response> { s.chars().flat_map(|c| ui.handle(Event::Key(Key::typed(c)))).collect() }
fn pointer(ui: &mut Ui, x: f32, y: f32, kind: PointerKind) -> Vec<Response> { ui.handle(Event::Pointer(Pointer { x, y, kind })) }
fn click_at(ui: &mut Ui, x: f32, y: f32) -> Vec<Response> {
    let mut r = pointer(ui, x, y, PointerKind::Move);
    r.extend(pointer(ui, x, y, PointerKind::Down(Button::Left)));
    r.extend(pointer(ui, x, y, PointerKind::Up(Button::Left)));
    r
}
fn click(ui: &mut Ui, id: Id) -> Vec<Response> {
    ui.layout();
    let c = ui.rect(id).center();
    let s = ui.theme().scale;
    click_at(ui, c.x * s, c.y * s)
}

#[test]
fn a_column_lays_out_top_to_bottom_and_rows_side_by_side() {
    let mut ui = ui();
    let root = ui.root();
    let col = ui.column(root);
    let a = ui.button(col, "First");
    let row = ui.row(col);
    let b = ui.button(row, "Left");
    let c = ui.button(row, "Right");
    ui.layout();
    let (ra, rb, rc) = (ui.rect(a), ui.rect(b), ui.rect(c));
    assert!(ra.w > 40.0 && ra.h >= 28.0, "{ra:?}");
    assert!(rb.y > ra.bottom() - 0.5, "row below the button: {ra:?} {rb:?}");
    assert!(rc.x > rb.right() - 0.5 && (rc.y - rb.y).abs() < 0.5, "{rb:?} {rc:?}");
}

#[test]
fn tab_moves_focus_in_order_and_space_presses() {
    let mut ui = ui();
    let root = ui.root();
    let col = ui.column(root);
    let a = ui.button(col, "A");
    let f = ui.field(col, "", "Name");
    let off = ui.button(col, "Off");
    ui.set_enabled(off, false);
    let ch = ui.checkbox(col, "Check", false);
    press(&mut ui, Key::new(code::TAB));
    assert_eq!(ui.focused(), Some(a));
    assert_eq!(press(&mut ui, Key::new(code::SPACE)), vec![Response::Clicked(a)]);
    press(&mut ui, Key::new(code::TAB));
    assert_eq!(ui.focused(), Some(f));
    press(&mut ui, Key::new(code::TAB));
    assert_eq!(ui.focused(), Some(ch), "disabled widgets are skipped");
    assert_eq!(press(&mut ui, Key::new(code::SPACE)), vec![Response::Toggled(ch, true)]);
    press(&mut ui, Key::new(code::TAB).shift());
    assert_eq!(ui.focused(), Some(f));
}

#[test]
fn clicks_hit_widgets_and_fields_edit() {
    let mut ui = ui();
    let root = ui.root();
    let col = ui.column(root);
    let b = ui.button(col, "Press");
    let f = ui.field(col, "", "");
    assert!(click(&mut ui, b).contains(&Response::Clicked(b)));
    click(&mut ui, f);
    assert_eq!(ui.focused(), Some(f));
    let r = typed(&mut ui, "Grüße");
    assert!(r.contains(&Response::Changed(f)));
    assert_eq!(ui.text_of(f), "Grüße");
    press(&mut ui, Key::new(code::BACKSPACE));
    press(&mut ui, Key::ctrl(0x1E)); // Ctrl+A
    let r = press(&mut ui, Key::ctrl(0x2E)); // Ctrl+C
    assert_eq!(r, vec![Response::Copy("Grüß".into())]);
    press(&mut ui, Key::new(code::END));
    press(&mut ui, Key::ctrl(0x2F)); // Ctrl+V
    assert_eq!(ui.text_of(f), "GrüßGrüß");
    press(&mut ui, Key::ctrl(0x2C)); // Ctrl+Z
    assert_eq!(ui.text_of(f), "Grüß");
    assert_eq!(press(&mut ui, Key::new(code::ENTER)), vec![Response::Submitted(f)]);
}

struct Million;
impl ListModel for Million {
    fn len(&self) -> usize { 1_000_000 }
    fn cell(&self, row: usize, column: usize, out: &mut String) { use core::fmt::Write; let _ = write!(out, "{}-{}", ["row", "size"][column.min(1)], row); }
}

#[test]
fn a_million_row_table_scrolls_and_paints_only_what_shows() {
    let mut ui = ui();
    let root = ui.root();
    let list = ui.list_with(root, Box::new(Million), vec![Column::new("Name", 200.0), Column::new("Size", 100.0).right()]);
    ui.set_multi_select(list, true);
    ui.focus(Some(list));
    let mut px = vec![0u32; 800 * 600];
    let t = std::time::Instant::now();
    ui.paint(&mut px);
    press(&mut ui, Key::new(code::END));
    assert_eq!(ui.selected_row(list), Some(999_999));
    ui.paint(&mut px);
    press(&mut ui, Key::new(code::UP).shift());
    press(&mut ui, Key::new(code::UP).shift());
    assert_eq!(ui.selection(list).unwrap().count(), 3);
    press(&mut ui, Key::ctrl(0x1E));
    assert_eq!(ui.selection(list).unwrap().count(), 1_000_000);
    ui.paint(&mut px);
    assert!(t.elapsed().as_millis() < 3000, "slow: {:?}", t.elapsed());
    // Type to find jumps to a row starting with what was typed.
    press(&mut ui, Key::new(code::HOME));
    let tree = a11y::describe(&ui.a11y_tree());
    assert!(tree.contains("Row \"row-0, size-0\" [selected] [focus]"), "{tree}");
}

#[test]
fn document_tabs_switch_close_reorder_pin_and_tear_off() {
    let mut ui = ui();
    let root = ui.root();
    let tabs = ui.document_tabs(root);
    for name in ["One", "Two", "Three"] { let p = ui.tab_page(tabs, Tab::new(name)); ui.label(p, name); }
    ui.layout();
    assert_eq!(ui.active_tab(tabs), 0);
    assert!(ui.find_text("One").is_some() && ui.find_text("Two").is_none(), "only the active page shows");
    assert_eq!(press(&mut ui, Key::ctrl(code::TAB)), vec![Response::Tab(tabs, TabEvent::Selected(1))]);
    assert_eq!(press(&mut ui, Key::ctrl(0x04)), vec![Response::Tab(tabs, TabEvent::Selected(2))]); // Ctrl+3
    assert_eq!(press(&mut ui, Key::ctrl(0x11)), vec![Response::Tab(tabs, TabEvent::Closed(2))]); // Ctrl+W
    assert_eq!(press(&mut ui, Key::ctrl(0x14)), vec![Response::Tab(tabs, TabEvent::New)]); // Ctrl+T
    ui.remove_tab(tabs, 2);
    assert_eq!(ui.tab_list(tabs).len(), 2);
    assert_eq!(ui.active_tab(tabs), 1);
    // Drag tab 0 to the right of tab 1.
    ui.layout();
    let r = ui.rect(tabs);
    let y = r.y + 15.0;
    pointer(&mut ui, r.x + 30.0, y, PointerKind::Move);
    pointer(&mut ui, r.x + 30.0, y, PointerKind::Down(Button::Left));
    pointer(&mut ui, r.x + 120.0, y, PointerKind::Move);
    pointer(&mut ui, r.x + 260.0, y, PointerKind::Move);
    let out = pointer(&mut ui, r.x + 260.0, y, PointerKind::Up(Button::Left));
    assert!(out.contains(&Response::Tab(tabs, TabEvent::Moved { from: 0, to: 1 })), "{out:?}");
    assert_eq!(ui.tab_list(tabs)[1].title, "One");
    // Pin the active tab ("One"): it moves first.
    let out = ui.run("weft.tab-pin");
    assert!(out.iter().any(|r| matches!(r, Response::Tab(_, TabEvent::Pinned(0, true)))), "{out:?}");
    assert!(ui.tab_list(tabs)[0].pinned);
    // Drag a tab far below the strip: it comes off.
    ui.layout();
    pointer(&mut ui, r.x + 80.0, y, PointerKind::Move);
    pointer(&mut ui, r.x + 80.0, y, PointerKind::Down(Button::Left));
    pointer(&mut ui, r.x + 160.0, y + 30.0, PointerKind::Move);
    pointer(&mut ui, r.x + 170.0, y + 200.0, PointerKind::Move);
    let out = pointer(&mut ui, r.x + 170.0, y + 200.0, PointerKind::Up(Button::Left));
    assert!(out.iter().any(|r| matches!(r, Response::Tab(_, TabEvent::TornOff(1, _)))), "{out:?}");
}

#[test]
fn menus_open_from_the_bar_and_run_commands_by_key_and_pointer() {
    let mut ui = ui();
    ui.commands.add(Command::new("app.open", "Open…").shortcut("Ctrl+O").icon("folder-open"));
    ui.commands.add(Command::new("app.save", "Save").shortcut("Ctrl+S"));
    ui.commands.add(Command::new("app.wrap", "Word Wrap").checked(true));
    let root = ui.root();
    let file = Menu::new("File").command("app.open").command("app.save").separator().submenu(Menu::new("View").command("app.wrap"));
    ui.menubar(root, vec![file, Menu::new("Help").command("app.about")]);
    assert_eq!(press(&mut ui, Key::ctrl(0x1F)), vec![Response::Command("app.save".into())]);
    press(&mut ui, Key::new(code::F10));
    press(&mut ui, Key::new(code::DOWN));
    assert_eq!(press(&mut ui, Key::new(code::ENTER)), vec![Response::Command("app.save".into())]);
    // Into the submenu by keys.
    press(&mut ui, Key::new(code::F10));
    press(&mut ui, Key::new(code::UP));
    press(&mut ui, Key::new(code::RIGHT));
    assert_eq!(press(&mut ui, Key::new(code::ENTER)), vec![Response::Command("app.wrap".into())]);
    // By pointer: press on the title, release on the first item.
    ui.layout();
    let bar = ui.find_kind("menubar")[0];
    let r = ui.rect(bar);
    pointer(&mut ui, r.x + 10.0, r.y + 10.0, PointerKind::Move);
    pointer(&mut ui, r.x + 10.0, r.y + 10.0, PointerKind::Down(Button::Left));
    ui.layout();
    let open = ui.find_text("Open…");
    assert!(open.is_none(), "menu rows are not labels");
    let row_y = r.bottom() + 4.0 + ui.theme().row_height() / 2.0;
    pointer(&mut ui, r.x + 40.0, row_y, PointerKind::Move);
    let out = pointer(&mut ui, r.x + 40.0, row_y, PointerKind::Up(Button::Left));
    assert_eq!(out, vec![Response::Command("app.open".into())]);
    // Escape closes an open menu without running anything.
    press(&mut ui, Key::new(code::F10));
    assert!(press(&mut ui, Key::new(code::ESCAPE)).is_empty());
    ui.layout();
    assert_eq!(ui.find_kind("menurow").len(), 0);
}

#[test]
fn the_command_palette_finds_and_runs_commands() {
    let mut ui = ui();
    ui.commands.add(Command::new("app.settings", "Settings").keywords("preferences options"));
    ui.commands.add(Command::new("app.close", "Close Window"));
    press(&mut ui, Key::ctrl(0x19).shift()); // Ctrl+Shift+P
    typed(&mut ui, "pref");
    assert_eq!(press(&mut ui, Key::new(code::ENTER)), vec![Response::Command("app.settings".into())]);
    press(&mut ui, Key::ctrl(0x19).shift());
    assert!(press(&mut ui, Key::new(code::ESCAPE)).is_empty());
}

#[test]
fn dialogs_answer_with_buttons_text_and_colours() {
    let mut ui = ui();
    let d = ui.ask_save("Save changes?", "Your notes have unsaved changes.");
    assert_eq!(press(&mut ui, Key::new(code::ENTER)), vec![Response::Dialog(d, DialogValue::Button(0))]);
    let d = ui.confirm("Delete?", "");
    assert_eq!(press(&mut ui, Key::new(code::ESCAPE)), vec![Response::Dialog(d, DialogValue::Cancelled)]);
    let d = ui.prompt("Rename", "New name:", "old");
    typed(&mut ui, "new");
    assert_eq!(press(&mut ui, Key::new(code::ENTER)), vec![Response::Dialog(d, DialogValue::Text("new".into()))]);
    let d = ui.pick_color("Colour", weft::Color::rgb(10, 20, 30));
    press(&mut ui, Key::new(code::RIGHT)); // red slider +1
    let out = press(&mut ui, Key::new(code::ENTER));
    assert_eq!(out, vec![Response::Dialog(d, DialogValue::Color(weft::Color::rgb(11, 20, 30)))]);
}

#[test]
fn german_strings_name_weft_controls() {
    let mut ui = Ui::new(800, 600, Theme::dark(), Strings::new("de"));
    assert_eq!(ui.tr("weft-cancel"), "Abbrechen");
    let d = ui.confirm("Löschen?", "");
    ui.layout();
    assert!(ui.find_text("Abbrechen").is_some());
    ui.close_dialog(d);
}

#[test]
fn painting_draws_and_repaints_only_damage() {
    let mut ui = ui();
    let root = ui.root();
    let col = ui.column(root);
    let b = ui.button(col, "Paint me");
    ui.slider(col, 0.0, 1.0, 0.5);
    ui.progress(col, Some(0.3));
    let mut px = vec![0u32; 800 * 600];
    let full = ui.paint(&mut px).unwrap();
    assert_eq!((full.w, full.h), (800.0, 600.0));
    let colours: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert!(colours.len() > 10, "{} colours", colours.len());
    assert!(ui.paint(&mut px).is_none(), "nothing changed");
    ui.set_text(b, "Painted");
    let part = ui.paint(&mut px).unwrap();
    assert!(part.w < 800.0 || part.h < 600.0, "{part:?}");
}

#[test]
fn the_accessibility_tree_names_roles_states_and_round_trips() {
    let mut ui = ui();
    let root = ui.root();
    let col = ui.column(root);
    ui.text(col, "Settings", weft::TextStyle::Heading);
    let b = ui.icon_button(col, "save", "");
    ui.set_label(b, "Save");
    let s = ui.switch(col, "Wi-Fi", true);
    ui.slider(col, 0.0, 10.0, 3.0);
    ui.field(col, "hello", "Name");
    ui.focus(Some(s));
    let full = ui.a11y_tree();
    let text = a11y::describe(&full);
    let want = "Window\n  GenericContainer\n    Label = \"Settings\"\n    Button \"Save\"\n    Switch \"Wi-Fi\" [on] [focus]\n    Slider = 3\n    TextInput = \"hello\"\n";
    assert_eq!(text, want);
    let bytes = a11y::encode(&full);
    let wire = a11y::decode(&bytes).unwrap();
    assert_eq!(wire.root, Some(1));
    assert_eq!(wire.nodes.len(), full.nodes.len());
    assert!(wire.nodes.iter().any(|n| n.label == "Wi-Fi" && n.toggled == Some(1)));
    assert!(ui.a11y_update().is_none(), "nothing changed");
    // A screen reader's click toggles the switch; the next update carries only it.
    let req = accesskit::ActionRequest { action: accesskit::Action::Click, target_tree: accesskit::TreeId::ROOT, target_node: a11y::node_id(s), data: None };
    assert_eq!(ui.a11y_action(&req), vec![Response::Toggled(s, false)]);
    let upd = ui.a11y_update().unwrap();
    assert!(upd.tree.is_none() && upd.nodes.len() <= 2, "{} nodes", upd.nodes.len());
    assert!(a11y::decode(&bytes[..bytes.len() - 1]).is_none());
}

#[test]
fn layout_hides_invisible_widgets_and_scroll_views_scroll() {
    let mut ui = ui();
    let root = ui.root();
    let sc = ui.scroll(root);
    ui.set_layout(sc, Layout::column().height(100.0));
    let mut last = None;
    for i in 0..40 { last = Some(ui.button(sc, &format!("Item {i}"))); }
    let last = last.unwrap();
    ui.layout();
    let before = ui.rect(last).y;
    let r = ui.rect(sc);
    let s = ui.theme().scale;
    pointer(&mut ui, (r.x + 20.0) * s, (r.y + 20.0) * s, PointerKind::Wheel(-2));
    ui.layout();
    assert!(ui.rect(last).y < before, "{} !< {}", ui.rect(last).y, before);
    ui.focus(Some(last));
    ui.layout();
    assert!(ui.rect(last).bottom() <= ui.rect(sc).bottom() + 5.0, "focused widget scrolled into view");
}
