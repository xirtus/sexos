// SPDX-License-Identifier: GPL-3.0-only
//! Host window tests for Filigree: build, paint, tab switching, live sample editing,
//! smart filters, non-destructive activation, 6-family comparison, F1 Coach, and
//! Spindle monospace handoff.

use filigree::{Filigree, FiligreeTab, SmartCollection, DEFAULT_SAMPLE_TEXT};
use filigree_core::{FontEntry, DEFAULT_WATERFALL_SIZES, MAX_COMPARE_SLOTS};
use weft::input::{code, Button, Event, Key, Pointer, PointerKind};
use weft::widget::Id;
use weft::{Response, Strings, Theme, Ui};

fn pump(ui: &mut Ui, app: &mut Filigree, rs: Vec<Response>) {
    for r in rs {
        app.on(ui, &r);
    }
}

fn click_id(ui: &mut Ui, app: &mut Filigree, id: Id) {
    ui.layout();
    let rect = ui.rect(id);
    assert!(!rect.is_empty(), "widget {id:?} has empty rect");
    let c = rect.center();
    let s = ui.theme().scale;
    click_at(ui, app, c.x * s, c.y * s);
}

fn click_field(ui: &mut Ui, app: &mut Filigree, pick: impl FnOnce(&Filigree) -> Id) {
    let id = pick(app);
    click_id(ui, app, id);
}

fn click_at(ui: &mut Ui, app: &mut Filigree, x: f32, y: f32) {
    for kind in [
        PointerKind::Move,
        PointerKind::Down(Button::Left),
        PointerKind::Up(Button::Left),
    ] {
        let rs = ui.handle(Event::Pointer(Pointer { x, y, kind }));
        pump(ui, app, rs);
    }
}

fn paint_assert(ui: &mut Ui, w: usize, h: usize) {
    let mut px = vec![0u32; w * h];
    assert!(ui.paint(&mut px).is_some(), "expected ui.paint to draw");
}

#[test]
fn build_seeds_embedded_fonts_and_paints_cleanly() {
    let mut ui = Ui::new(1024, 720, Theme::light(), Strings::new("en"));
    let app = Filigree::build(&mut ui);

    let families = app.catalog.families();
    let names: Vec<&str> = families.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Geist"));
    assert!(names.contains(&"Geist Mono"));
    assert!(names.contains(&"JetBrains Mono"));
    assert!(names.contains(&"Noto Sans Mono"));
    assert!(names.contains(&"Noto Sans Arabic"));
    assert!(names.contains(&"Noto Sans Hebrew"));
    assert!(names.contains(&"Noto Sans Thai"));

    assert_eq!(ui.text_of(app.sample_field), DEFAULT_SAMPLE_TEXT);
    assert_eq!(app.waterfall_rows.len(), DEFAULT_WATERFALL_SIZES.len());
    assert!(ui
        .text_of(app.waterfall_rows[0].1)
        .contains(DEFAULT_SAMPLE_TEXT));

    paint_assert(&mut ui, 1024, 720);
}

#[test]
fn tab_switching_and_repertoire_inspector_work() {
    let mut ui = Ui::new(1024, 720, Theme::light(), Strings::new("en"));
    let mut app = Filigree::build(&mut ui);
    paint_assert(&mut ui, 1024, 720);

    // Click the Compare tab in the tab strip (tab_geom populated by paint).
    let tab_rect = ui.rect(app.tabs);
    let strip_y = tab_rect.y + 16.0;
    click_at(&mut ui, &mut app, tab_rect.x + 190.0, strip_y);
    assert_eq!(app.active_tab, FiligreeTab::Compare);
    paint_assert(&mut ui, 1024, 720);

    // Switch via Ctrl+3 command shortcut to Repertoire & Inspector.
    let rs = ui.handle(Event::Key(Key::ctrl(0x04))); // '3'
    pump(&mut ui, &mut app, rs);
    assert_eq!(app.active_tab, FiligreeTab::Repertoire);

    // Select Noto Sans Arabic to verify script coverage & ScalarRange inspection.
    app.select_family(&mut ui, "Noto Sans Arabic");
    assert!(ui.text_of(app.inspector_scripts).contains("Arabic"));
    assert!(ui.text_of(app.inspector_ranges).contains("U+"));
    assert!(ui.text_of(app.inspector_metrics).contains("OS/2 Weight:"));

    // Step through codepoint blocks if multiple non-empty blocks exist.
    let initial_page = String::from(ui.text_of(app.inspector_page_label));
    if ui.is_enabled(app.inspector_next_page_btn) {
        click_field(&mut ui, &mut app, |a| a.inspector_next_page_btn);
        assert_ne!(ui.text_of(app.inspector_page_label), initial_page);
        click_field(&mut ui, &mut app, |a| a.inspector_prev_page_btn);
        assert_eq!(ui.text_of(app.inspector_page_label), initial_page);
    }

    // Switch to Tab 4 via command and back to Tab 1.
    let rs = ui.handle(Event::Key(Key::ctrl(0x05))); // '4'
    pump(&mut ui, &mut app, rs);
    assert_eq!(app.active_tab, FiligreeTab::Duplicates);
    paint_assert(&mut ui, 1024, 720);

    let rs = ui.handle(Event::Key(Key::ctrl(0x02))); // '1'
    pump(&mut ui, &mut app, rs);
    assert_eq!(app.active_tab, FiligreeTab::Families);
}

#[test]
fn sample_text_editing_updates_waterfall_and_compare_live() {
    let mut ui = Ui::new(1024, 720, Theme::light(), Strings::new("en"));
    let mut app = Filigree::build(&mut ui);

    click_field(&mut ui, &mut app, |a| a.sample_field);
    let rs = ui.handle(Event::Key(Key::ctrl(0x1E))); // Ctrl+A
    pump(&mut ui, &mut app, rs);
    for ch in "Sphinx of black quartz".chars() {
        let rs = ui.handle(Event::Key(Key::typed(ch)));
        pump(&mut ui, &mut app, rs);
    }

    assert_eq!(app.sample_text, "Sphinx of black quartz");
    for &(_, row_id) in &app.waterfall_rows {
        assert!(ui.text_of(row_id).contains("Sphinx of black quartz"));
    }
    assert!(ui
        .text_of(app.compare_slots[0].sample)
        .contains("Sphinx of black quartz"));

    // Reset restores the default pangram.
    click_field(&mut ui, &mut app, |a| a.reset_sample_btn);
    assert_eq!(app.sample_text, DEFAULT_SAMPLE_TEXT);
    assert!(ui
        .text_of(app.waterfall_rows[0].1)
        .contains(DEFAULT_SAMPLE_TEXT));
}

#[test]
fn smart_filters_star_and_non_destructive_deactivation_reactivation() {
    let mut ui = Ui::new(1024, 720, Theme::light(), Strings::new("en"));
    let mut app = Filigree::build(&mut ui);

    // Filter to Monospace.
    let mono_btn = app
        .filter_buttons
        .iter()
        .find(|(_, sc)| *sc == SmartCollection::Monospace)
        .unwrap()
        .0;
    click_id(&mut ui, &mut app, mono_btn);
    let mono_names: Vec<String> = app.filtered_families().into_iter().map(|f| f.name).collect();
    assert!(mono_names.contains(&"Geist Mono".to_string()));
    assert!(mono_names.contains(&"JetBrains Mono".to_string()));
    assert!(!mono_names.contains(&"Geist".to_string()));

    // Back to All, select Geist, star it, and deactivate it.
    let all_btn = app
        .filter_buttons
        .iter()
        .find(|(_, sc)| *sc == SmartCollection::All)
        .unwrap()
        .0;
    click_id(&mut ui, &mut app, all_btn);
    app.select_family(&mut ui, "Geist");

    click_field(&mut ui, &mut app, |a| a.star_btn);
    assert!(ui.text_of(app.star_btn).contains("1/5"));

    click_field(&mut ui, &mut app, |a| a.toggle_active_btn);
    assert_eq!(ui.text_of(app.toggle_active_btn), "Activate");
    assert!(ui
        .text_of(app.manifest_preview)
        .contains("deactivated_families = [\n  \"Geist\","));
    assert!(!app.current_family().unwrap().is_any_active());

    // Smart Collection: Deactivated shows only Geist.
    let deact_btn = app
        .filter_buttons
        .iter()
        .find(|(_, sc)| *sc == SmartCollection::Deactivated)
        .unwrap()
        .0;
    click_id(&mut ui, &mut app, deact_btn);
    let deact_names: Vec<String> = app.filtered_families().into_iter().map(|f| f.name).collect();
    assert_eq!(deact_names, vec!["Geist".to_string()]);

    // Reactivate Geist non-destructively.
    click_field(&mut ui, &mut app, |a| a.toggle_active_btn);
    assert!(app.filtered_families().is_empty());

    click_id(&mut ui, &mut app, all_btn);
    app.select_family(&mut ui, "Geist");
    assert!(app.current_family().unwrap().is_any_active());
    assert_eq!(ui.text_of(app.toggle_active_btn), "Deactivate");
    assert!(ui
        .text_of(app.manifest_preview)
        .contains("deactivated_families = []"));

    // Starred filter includes Geist.
    let star_filter_btn = app
        .filter_buttons
        .iter()
        .find(|(_, sc)| *sc == SmartCollection::Starred)
        .unwrap()
        .0;
    click_id(&mut ui, &mut app, star_filter_btn);
    assert_eq!(
        app.filtered_families()
            .into_iter()
            .map(|f| f.name)
            .collect::<Vec<_>>(),
        vec!["Geist".to_string()]
    );
}

#[test]
fn six_family_side_by_side_comparison_enforces_limit_and_renders() {
    let mut ui = Ui::new(1080, 760, Theme::dark(), Strings::new("en"));
    let mut app = Filigree::build(&mut ui);

    app.set_tab(&mut ui, FiligreeTab::Compare);
    assert_eq!(app.compare_slots.len(), 3);

    // Fill up to 6 families.
    click_field(&mut ui, &mut app, |a| a.compare_fill_btn);
    assert_eq!(app.compare_families.len(), MAX_COMPARE_SLOTS);
    assert_eq!(app.compare_slots.len(), 6);
    assert!(ui.text_of(app.compare_summary).contains("Comparing 6 / 6"));

    for slot in &app.compare_slots {
        assert!(ui.text_of(slot.metrics).contains("x-height:"));
        assert!(ui.text_of(slot.metrics).contains("cap-height:"));
        assert!(ui.text_of(slot.class_glyphs).contains("PANOSE"));
        assert!(ui.text_of(slot.class_glyphs).contains("Glyphs:"));
    }

    // Adding a 7th family is rejected cleanly.
    let seventh = app
        .catalog
        .families()
        .into_iter()
        .find(|f| !app.compare_families.iter().any(|c| c == &f.name))
        .expect("7th embedded family exists")
        .name;
    assert!(!app.add_to_compare(&mut ui, &seventh));
    assert_eq!(app.compare_families.len(), 6);

    paint_assert(&mut ui, 1080, 760);

    // Remove the first slot via its Remove button.
    let remove0 = app.compare_slots[0].remove_btn;
    click_id(&mut ui, &mut app, remove0);
    assert_eq!(app.compare_slots.len(), 5);
    assert!(ui.text_of(app.compare_summary).contains("Comparing 5 / 6"));

    paint_assert(&mut ui, 1080, 760);
}

#[test]
fn f1_coach_toggle_and_five_step_walkthrough() {
    let mut ui = Ui::new(1024, 720, Theme::light(), Strings::new("en"));
    let mut app = Filigree::build(&mut ui);
    ui.layout();

    assert!(!app.coach.active);
    assert_eq!(ui.rect(app.coach_controls.card).h, 0.0);

    // F1 opens the Coach.
    let rs = ui.handle(Event::Key(Key::new(code::F1)));
    pump(&mut ui, &mut app, rs);
    ui.layout();
    assert!(app.coach.active);
    assert_eq!(app.coach.progress_label(), "1 / 5");
    assert_eq!(ui.text_of(app.coach_controls.progress), "1 / 5");
    assert!(ui.rect(app.coach_controls.card).h > 40.0);
    assert!(!ui.is_enabled(app.coach_controls.prev));
    assert!(ui.is_enabled(app.coach_controls.next));

    // Step through all 5 steps via Next button.
    for expected in ["2 / 5", "3 / 5", "4 / 5", "5 / 5"] {
        click_field(&mut ui, &mut app, |a| a.coach_controls.next);
        assert_eq!(ui.text_of(app.coach_controls.progress), expected);
    }
    assert!(!ui.is_enabled(app.coach_controls.next));

    // Step back once with Previous.
    click_field(&mut ui, &mut app, |a| a.coach_controls.prev);
    assert_eq!(ui.text_of(app.coach_controls.progress), "4 / 5");

    // Done closes the Coach and collapses its banner.
    click_field(&mut ui, &mut app, |a| a.coach_controls.done);
    ui.layout();
    assert!(!app.coach.active);
    assert_eq!(ui.rect(app.coach_controls.card).h, 0.0);

    // Toolbar Guide (F1) button reopens it.
    click_field(&mut ui, &mut app, |a| a.guide_btn);
    ui.layout();
    assert!(app.coach.active);
    assert!(ui.rect(app.coach_controls.card).h > 40.0);
    paint_assert(&mut ui, 1024, 720);

    // F1 toggles it closed again.
    let rs = ui.handle(Event::Key(Key::new(code::F1)));
    pump(&mut ui, &mut app, rs);
    assert!(!app.coach.active);
}

#[test]
fn duplicates_install_check_and_spindle_monospace_handoff() {
    let mut ui = Ui::new(1024, 720, Theme::light(), Strings::new("en"));
    let mut app = Filigree::build(&mut ui);

    app.set_tab(&mut ui, FiligreeTab::Duplicates);
    assert!(ui.text_of(app.dupes_summary).contains("0 duplicate"));

    // Inject an exact duplicate and recheck.
    let dup = FontEntry::from_bytes("/home/user/GeistMono-Copy.ttf", 0, sex_font::GEIST_MONO)
        .unwrap();
    app.catalog.add_entry(dup);
    click_field(&mut ui, &mut app, |a| a.dupes_recheck_btn);
    assert!(ui.text_of(app.dupes_summary).contains("Exact SHA-256"));
    assert!(ui.text_of(app.dupes_summary).contains("remove_duplicate"));

    // Pre-install validation: valid TTF vs truncated bytes.
    assert!(app
        .validate_install_bytes(&mut ui, sex_font::JETBRAINS_MONO, "JetBrainsMono-Regular.ttf")
        .is_ok());
    assert!(ui.text_of(app.install_status).contains("Validated 1 asset(s)"));
    assert!(app
        .validate_install_bytes(&mut ui, &sex_font::GEIST_SANS[..128], "broken.ttf")
        .is_err());
    assert!(ui.text_of(app.install_status).contains("Validation failed"));

    // Proportional font ("Geist") is blocked from Spindle monospace handoff.
    app.select_family(&mut ui, "Geist");
    click_field(&mut ui, &mut app, |a| a.spindle_handoff_btn);
    assert!(app.last_spindle_toml.is_none());
    assert!(ui.text_of(app.spindle_status).contains("Handoff blocked"));

    // Quick-pick JetBrains Mono and execute 1-click Spindle handoff.
    let jb_btn = app
        .spindle_mono_buttons
        .iter()
        .find(|(_, fam)| fam == "JetBrains Mono")
        .unwrap()
        .0;
    click_id(&mut ui, &mut app, jb_btn);
    click_field(&mut ui, &mut app, |a| a.spindle_handoff_btn);

    let staged = app
        .last_spindle_toml
        .as_ref()
        .expect("Spindle TOML staged");
    assert!(staged.contains("[appearance]"));
    assert!(staged.contains("[spindle]"));
    assert!(staged.contains("monospace_font = \"JetBrains Mono\""));
    assert!(ui
        .text_of(app.spindle_preview)
        .contains("monospace_font = \"JetBrains Mono\""));
    assert!(ui
        .text_of(app.spindle_status)
        .contains("/system/settings.toml"));

    paint_assert(&mut ui, 1024, 720);
}
