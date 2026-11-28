// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for the OLauncher / macOS Spotlight-style Frosted Candy
//! Lavender App Launcher & Search overlay.

#[path = "../crates/sex-pdx/src/spotlight.rs"]
mod spotlight;
#[path = "../servers/silk-shell/src/wm.rs"]
mod wm;
#[path = "../servers/silk-shell/src/desktop_keys.rs"]
mod desktop_keys;

use spotlight::*;

#[test]
fn wire_round_trip_and_invalid_rejection() {
    let mut ctrl = Controller::new();
    ctrl.open();
    // Type "tail"
    for &code in &[0x14u8, 0x1E, 0x17, 0x26] {
        assert_eq!(ctrl.handle_key(code, 1, false, false), KeyOutcome::Redraw);
    }
    let st = ctrl.state();
    assert_eq!(st.query.as_str(), "tail");
    let (w0, _, _) = ctrl.wire();
    let (q0, q1, q2) = ctrl.wire_query();
    assert_eq!(SpotlightState::decode(w0, q0, q1, q2), Some(st));

    // Hidden word decodes to closed state.
    assert_eq!(SpotlightState::decode(0, 0, 0, 0), Some(SpotlightState::closed()));
    // Reserved upper bits rejected.
    assert_eq!(SpotlightState::decode(w0 | (1 << 50), q0, q1, q2), None);
    // Query length > QUERY_MAX rejected.
    let bad_qlen = (w0 & !(0xFF << 24)) | (40u64 << 24);
    assert_eq!(SpotlightState::decode(bad_qlen, q0, q1, q2), None);
}

#[test]
fn live_search_filtering_and_backspace() {
    let mut ctrl = Controller::new();
    ctrl.open();
    let (_, all_count) = ctrl.filtered();
    assert_eq!(all_count, CATALOG_LEN);

    // Search "tail" -> Tailor AI first.
    let (order, n) = filter_catalog(Category::All, "tail");
    assert!(n >= 1);
    assert_eq!(CATALOG[order[0]].id, "tailor");

    // Search by keyword "webcam" -> Flash.
    let (order, n) = filter_catalog(Category::All, "webcam");
    assert_eq!(n, 1);
    assert_eq!(CATALOG[order[0]].id, "flash");

    // Fuzzy subsequence "klds" -> Kaleidoscope.
    let (order, n) = filter_catalog(Category::All, "klds");
    assert!(n >= 1);
    assert_eq!(CATALOG[order[0]].id, "kaleidoscope");

    // Search "loom" -> Files.
    let (order, n) = filter_catalog(Category::All, "loom");
    assert_eq!(n, 1);
    assert_eq!(CATALOG[order[0]].id, "files");

    // Search "moan" -> Moan Studio DAW.
    let (order, n) = filter_catalog(Category::All, "moan");
    assert_eq!(n, 1);
    assert_eq!(CATALOG[order[0]].id, "moan");
    assert_eq!(CATALOG[order[0]].name, "Moan Studio");
    assert_eq!(CATALOG[order[0]].target, LaunchTarget::Program("moan-studio"));

    // Search "splicy", "fcp", and "video editor" -> Splicy video editor.
    for query in ["splicy", "fcp", "video editor"] {
        let (order, n) = filter_catalog(Category::All, query);
        assert!(n >= 1, "expected match for '{}'", query);
        assert_eq!(CATALOG[order[0]].id, "splicy", "query '{}'", query);
        assert_eq!(CATALOG[order[0]].category, Category::Media);
        assert_eq!(CATALOG[order[0]].target, LaunchTarget::Program("splicy"));
    }

    // Search "polyamour" and "3d" -> Polyamour.
    let (order, n) = filter_catalog(Category::All, "polyamour");
    assert_eq!(n, 1);
    assert_eq!(CATALOG[order[0]].id, "polyamour");
    assert_eq!(CATALOG[order[0]].target, LaunchTarget::Program("polyamour"));
    let (order, n) = filter_catalog(Category::All, "3d");
    assert_eq!(n, 1);
    assert_eq!(CATALOG[order[0]].id, "polyamour");

    // Live typing + Backspace in Controller.
    assert_eq!(ctrl.handle_key(0x21, 1, false, false), KeyOutcome::Redraw); // 'f'
    assert_eq!(ctrl.handle_key(0x26, 1, false, false), KeyOutcome::Redraw); // 'l'
    assert_eq!(ctrl.state().query.as_str(), "fl");
    let (order, n) = ctrl.filtered();
    assert!(n >= 1);
    assert_eq!(CATALOG[order[0]].id, "flash");

    // Backspace pops 'l'.
    assert_eq!(ctrl.handle_key(0x0E, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().query.as_str(), "f");
}

#[test]
fn every_catalog_app_maps_to_real_sex_icons_app_icon() {
    let icons_txt = include_str!("../assets/icons/icons.txt");
    for app in &CATALOG {
        let found = icons_txt.lines().any(|line| {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("app ") {
                rest.split_once(' ').map(|(name, _)| name == app.icon).unwrap_or(false)
            } else {
                false
            }
        });
        assert!(
            found,
            "Spotlight app '{}' icon '{}' missing from assets/icons/icons.txt",
            app.id, app.icon
        );
    }
    assert!(CATALOG_LEN >= GRID_COLS * GRID_ROWS);
}

#[test]
fn inline_calculator_evaluates_arithmetic_safely() {
    assert_eq!(eval_calc("2+2").unwrap().as_str(), "= 4");
    assert_eq!(eval_calc("10 * (3 + 4)").unwrap().as_str(), "= 70");
    assert_eq!(eval_calc("100 / 4").unwrap().as_str(), "= 25");
    assert_eq!(eval_calc("7 / 2").unwrap().as_str(), "= 3.5");
    assert_eq!(eval_calc("-5 + 12").unwrap().as_str(), "= 7");
    assert_eq!(eval_calc("10 % 3").unwrap().as_str(), "= 1");
    assert_eq!(eval_calc("1.5 + 2.25").unwrap().as_str(), "= 3.75");

    // Division/modulo by zero, plain numbers without operators, and app names return None.
    assert_eq!(eval_calc("10 / 0"), None);
    assert_eq!(eval_calc("10 % 0"), None);
    assert_eq!(eval_calc("42"), None);
    assert_eq!(eval_calc("tailor"), None);
    assert_eq!(eval_calc("2 +"), None);

    // Typing "2+2" via Controller produces "= 4".
    let mut ctrl = Controller::new();
    ctrl.open();
    assert_eq!(ctrl.handle_key(0x03, 1, false, false), KeyOutcome::Redraw); // '2'
    assert_eq!(ctrl.handle_key(0x0D, 1, false, true), KeyOutcome::Redraw);  // '+'
    assert_eq!(ctrl.handle_key(0x03, 1, false, false), KeyOutcome::Redraw); // '2'
    assert_eq!(ctrl.calc_result().unwrap().as_str(), "= 4");
}

#[test]
fn category_filtering_via_tab_ctrl_arrows_and_click() {
    let mut ctrl = Controller::new();
    ctrl.open();
    assert_eq!(ctrl.state().category, Category::All);

    // Tab cycles All -> Apps -> Media -> AI & Dev -> System -> All.
    assert_eq!(ctrl.handle_key(0x0F, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().category, Category::Apps);
    let (order, n) = ctrl.filtered();
    assert!(n >= 3);
    for &idx in &order[..n] {
        assert_eq!(CATALOG[idx].category, Category::Apps);
    }

    // Ctrl+Right moves to Media; Ctrl+Left returns to Apps; Shift+Tab returns to All.
    assert_eq!(ctrl.handle_key(0x4D, 1, true, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().category, Category::Media);
    assert_eq!(ctrl.handle_key(0x4B, 1, true, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().category, Category::Apps);
    assert_eq!(ctrl.handle_key(0x0F, 1, false, true), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().category, Category::All);

    // Clicking AI & Dev pill switches category.
    let layout = Layout::new(1280, 800, 28, 1).unwrap();
    let ai_pill = layout.category_pills[Category::AiDev as usize];
    let cx = (ai_pill.0 + ai_pill.2 / 2) as i32;
    let cy = (ai_pill.1 + ai_pill.3 / 2) as i32;
    assert_eq!(ctrl.handle_press(cx, cy, 1280, 800, 28, 1), ClickOutcome::Redraw);
    assert_eq!(ctrl.state().category, Category::AiDev);
}

#[test]
fn grid_arrow_navigation_and_enter_launch_dispatch() {
    let mut ctrl = Controller::new();
    ctrl.open();
    assert_eq!(ctrl.state().selected, 0);

    // Right moves 0 -> 1.
    assert_eq!(ctrl.handle_key(0x4D, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().selected, 1);

    // Down moves 1 -> 1 + GRID_COLS.
    assert_eq!(ctrl.handle_key(0x50, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().selected as usize, 1 + GRID_COLS);

    // Up moves back to 1 (Kaleidoscope).
    assert_eq!(ctrl.handle_key(0x48, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().selected, 1);

    // Left wraps 0 -> last visible grid slot.
    assert_eq!(ctrl.handle_key(0x4B, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().selected, 0);
    assert_eq!(ctrl.handle_key(0x4B, 1, false, false), KeyOutcome::Redraw);
    assert_eq!(ctrl.state().selected as usize, (GRID_COLS * GRID_ROWS).min(CATALOG_LEN) - 1);

    // Move back to index 2 (Kaleidoscope) and press Enter -> Launch(Browser) and close.
    assert_eq!(ctrl.handle_key(0x4D, 1, false, false), KeyOutcome::Redraw); // 0
    assert_eq!(ctrl.handle_key(0x4D, 1, false, false), KeyOutcome::Redraw); // 1
    assert_eq!(ctrl.handle_key(0x4D, 1, false, false), KeyOutcome::Redraw); // 2
    assert_eq!(
        ctrl.handle_key(0x1C, 1, false, false),
        KeyOutcome::Launch(LaunchTarget::Browser)
    );
    assert!(!ctrl.is_open());
}

#[test]
fn tile_click_launch_esc_and_outside_click_dismissal() {
    let mut ctrl = Controller::new();
    ctrl.open();

    // Layout is centered (~640x440) and clamped inside screen bounds across scales.
    for scale in 1..=3u32 {
        let s = scale as usize;
        let l = Layout::new(1280 * s, 800 * s, 28 * s, scale).unwrap();
        assert_eq!(l.panel.2, PANEL_W * s);
        assert_eq!(l.panel.3, PANEL_H * s);
        assert!(l.panel.0 + l.panel.2 <= 1280 * s);
        assert!(l.panel.1 >= 28 * s);
        assert!(l.panel.1 + l.panel.3 <= 800 * s);
    }

    // Clicking tile 0 (Tailor AI) launches it immediately and closes Spotlight.
    let l = Layout::new(1280, 800, 28, 1).unwrap();
    let t0 = l.tile_rect(0).unwrap();
    assert_eq!(
        ctrl.handle_press((t0.0 + 10) as i32, (t0.1 + 10) as i32, 1280, 800, 28, 1),
        ClickOutcome::Launch(LaunchTarget::Program("tailor"))
    );
    assert!(!ctrl.is_open());

    // Clicking outside dismisses cleanly.
    ctrl.open();
    assert_eq!(ctrl.handle_press(10, 40, 1280, 800, 28, 1), ClickOutcome::Dismiss);
    assert!(!ctrl.is_open());

    // Pressing Esc dismisses cleanly.
    ctrl.open();
    assert_eq!(ctrl.handle_key(0x01, 1, false, false), KeyOutcome::Close);
    assert!(!ctrl.is_open());
}

#[test]
fn spotlight_hotkeys_super_space_super_d_and_alt_space() {
    let plain = wm::Mods { shift: false, ctrl: false };
    let shift = wm::Mods { shift: true, ctrl: false };
    assert_eq!(wm::binding(wm::code::SPACE, plain), Some(wm::WmAction::CycleLayout));
    assert_eq!(wm::binding(wm::code::D, plain), Some(wm::WmAction::Spotlight));
    assert_eq!(wm::binding(wm::code::D, shift), Some(wm::WmAction::Launcher));
    assert_eq!(wm::binding(wm::code::SPACE, shift), Some(wm::WmAction::ToggleFloating));

    let mut k = desktop_keys::Keys::new();
    assert_eq!(k.event(0x38, 1), desktop_keys::Action::Consume);
    assert_eq!(k.event(desktop_keys::KEY_SPACE, 1), desktop_keys::Action::Spotlight);
    assert_eq!(k.event(desktop_keys::KEY_SPACE, 0), desktop_keys::Action::Consume);
    assert_eq!(k.event(0x38, 0), desktop_keys::Action::Consume);
}

#[test]
fn human_aliases_and_keyword_priority_rank_exact_tile_first() {
    assert_eq!(resolve_program_alias("settings"), "tatami");
    assert_eq!(resolve_program_alias("preferences"), "tatami");
    assert_eq!(resolve_program_alias("prefs"), "tatami");
    assert_eq!(resolve_program_alias("system-settings"), "tatami");
    assert_eq!(resolve_program_alias("store"), "sexshop");
    assert_eq!(resolve_program_alias("appstore"), "sexshop");
    assert_eq!(resolve_program_alias("app-store"), "sexshop");
    assert_eq!(resolve_program_alias("packages"), "sexshop");
    assert_eq!(resolve_program_alias("pkg"), "sexshop");
    assert_eq!(resolve_program_alias("files"), "loom");
    assert_eq!(resolve_program_alias("finder"), "loom");
    assert_eq!(resolve_program_alias("explorer"), "loom");
    assert_eq!(resolve_program_alias("browser"), "kaleidoscope");
    assert_eq!(resolve_program_alias("web"), "kaleidoscope");
    assert_eq!(resolve_program_alias("internet"), "kaleidoscope");
    assert_eq!(resolve_program_alias("monitor"), "mesh");
    assert_eq!(resolve_program_alias("activity"), "mesh");
    assert_eq!(resolve_program_alias("tasks"), "mesh");
    assert_eq!(resolve_program_alias("top"), "mesh");
    assert_eq!(resolve_program_alias("htop"), "mesh");
    assert_eq!(resolve_program_alias("calendar"), "date");
    assert_eq!(resolve_program_alias("schedule"), "date");
    assert_eq!(resolve_program_alias("music"), "qupid");
    assert_eq!(resolve_program_alias("audio"), "qupid");
    assert_eq!(resolve_program_alias("player"), "qupid");
    assert_eq!(resolve_program_alias("vlc"), "qupid");
    assert_eq!(resolve_program_alias("chat"), "flirt");
    assert_eq!(resolve_program_alias("messages"), "flirt");
    assert_eq!(resolve_program_alias("messaging"), "flirt");
    assert_eq!(resolve_program_alias("daw"), "moan-studio");
    assert_eq!(resolve_program_alias("moan"), "moan-studio");
    assert_eq!(resolve_program_alias("synth"), "moan-studio");
    assert_eq!(resolve_program_alias("notes"), "jot");
    assert_eq!(resolve_program_alias("notepad"), "jot");
    assert_eq!(resolve_program_alias("editor"), "vellum");
    assert_eq!(resolve_program_alias("textedit"), "vellum");
    assert_eq!(resolve_program_alias("calc"), "abacus");
    assert_eq!(resolve_program_alias("calculator"), "abacus");
    assert_eq!(resolve_program_alias("ai"), "tailor");
    assert_eq!(resolve_program_alias("assistant"), "tailor");
    assert_eq!(resolve_program_alias("claude"), "tailor");
    assert_eq!(resolve_program_alias("codex"), "tailor");
    assert_eq!(resolve_program_alias("bridge"), "tether");
    assert_eq!(resolve_program_alias("remote"), "tether");
    assert_eq!(resolve_program_alias("mcp"), "tether");
    assert_eq!(resolve_program_alias("torrent"), "swingers");
    assert_eq!(resolve_program_alias("p2p"), "swingers");
    assert_eq!(resolve_program_alias("webcam"), "flash");
    assert_eq!(resolve_program_alias("camera"), "flash");

    let expected_top = [
        ("store", "sexshop"),
        ("preferences", "settings"),
        ("calculator", "abacus"),
        ("music", "qupid"),
        ("wifi", "settings"),
        ("bluetooth", "hookup"),
        ("firewall", "corset"),
        ("backup", "aftercare"),
    ];
    for (query, expected_id) in expected_top {
        let (order, n) = filter_catalog(Category::All, query);
        assert!(n >= 1, "expected match for {}", query);
        assert_eq!(CATALOG[order[0]].id, expected_id, "query={}", query);
    }
}
