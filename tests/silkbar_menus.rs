// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for interactive SilkBar dropdown menus, right-side status
//! popovers, desktop/window right-click context menus, top-bar hover-switching,
//! dismiss-on-outside-click/Esc, and KDE/Hyprland hotkeys.

#[path = "../crates/sex-pdx/src/silkbar_menus.rs"]
mod silkbar_menus;
#[path = "../servers/silk-shell/src/desktop_keys.rs"]
mod desktop_keys;
#[path = "../servers/silk-shell/src/wm.rs"]
mod wm;

use silkbar_menus::*;

#[test]
fn wire_round_trip_and_reject_invalid() {
    let mut s = MenuState::new(MenuKind::System, 12, 32);
    s.hover = Some(3);
    s.flags = FLAG_MAXIMIZED | FLAG_ON_AC | FLAG_CHARGING;
    s = s.with_power_profile(PROFILE_PERFORMANCE);
    s.volume = 64;
    s.battery = 88;
    s.wifi_phase = 10;
    s.uptime_mins = 145;
    let (w0, w1) = s.wire();
    assert_eq!(MenuState::decode(w0, w1), Some(s));

    // Hidden word decodes to None.
    assert_eq!(MenuState::decode(0, 0), None);
    // Reserved upper bits rejected.
    assert_eq!(MenuState::decode(w0 | (1 << 55), w1), None);
    // Out-of-bounds hover rejected.
    let bad_hover = (w0 & !(0x1F << 36)) | (25u64 << 36);
    assert_eq!(MenuState::decode(bad_hover, w1), None);
}

#[test]
fn top_bar_menu_rects_preserve_virtio_gpu_gate_pixel_300_14() {
    for scale in 1..=3u32 {
        let s = scale as usize;
        let bar_h = 28 * s;
        let rects = top_bar_menu_rects(bar_h, scale, 114 * s, 583 * s);
        assert_eq!(rects[0].0, TopBarTrigger::File);
        assert_eq!(rects[1].0, TopBarTrigger::Edit);
        assert_eq!(rects[2].0, TopBarTrigger::View);
        assert_eq!(rects[3].0, TopBarTrigger::Window);
        for &(_, r) in &rects {
            assert!(r.2 > 0 && r.3 > 0);
            // Strictly left of 290*s so (300*s, 14*s) stays pure frosted bar glass.
            assert!(r.0 + r.2 <= 290 * s, "rect {:?} crossed 290*{}", r, s);
            assert!(!rect_contains(r, 300 * s, 14 * s));
        }
    }
}

#[test]
fn system_and_app_menus_have_expected_items_and_shortcut_hints() {
    let sys = MenuState::new(MenuKind::System, 8, 32);
    assert_eq!(sys.row_count(), 12);
    assert_eq!(sys.row(0).unwrap().action, MenuAction::AboutSexOs);
    assert_eq!(sys.row(1).unwrap().action, MenuAction::OpenSpotlight);
    assert_eq!(sys.row(1).unwrap().shortcut, "Super+Spc");
    assert_eq!(sys.row(2).unwrap().action, MenuAction::LaunchBrowser);
    assert_eq!(sys.row(3).unwrap().action, MenuAction::LaunchTerminal);
    assert_eq!(sys.row(3).unwrap().shortcut, "Super+Ret");
    assert_eq!(sys.row(4).unwrap().action, MenuAction::LaunchTailor);
    assert_eq!(sys.row(5).unwrap().action, MenuAction::LaunchFlash);
    assert_eq!(sys.row(6).unwrap().action, MenuAction::LaunchPdGraph);
    assert_eq!(sys.row(7).unwrap().action, MenuAction::LaunchFiles);
    assert_eq!(sys.row(8).unwrap().action, MenuAction::LaunchSettings);
    assert_eq!(sys.row(9).unwrap().action, MenuAction::RefreshDesktop);
    assert_eq!(sys.row(10).unwrap().action, MenuAction::Reboot);
    assert_eq!(sys.row(11).unwrap().action, MenuAction::Shutdown);

    let win = MenuState::new(MenuKind::AppWindow, 220, 32);
    assert_eq!(win.row(1).unwrap().shortcut, "Super+F");
    assert_eq!(win.row(3).unwrap().action, MenuAction::GroupWindow);
    assert_eq!(win.row(3).unwrap().shortcut, "Super+G");
    assert_eq!(win.row(6).unwrap().action, MenuAction::CloseWindow);
    assert_eq!(win.row(6).unwrap().shortcut, "Alt+F4");
}

#[test]
fn hover_switching_and_dismiss_on_outside_click_or_escape() {
    let mut ctrl = Controller::new();
    let mut st = MenuState::new(MenuKind::System, 10, 32);
    st.volume = 42;
    st.battery = 77;
    ctrl.open(st);
    assert!(ctrl.is_open());

    // Hovering across top-bar menu items switches smoothly while preserving state.
    assert!(ctrl.hover_top_bar(TopBarTrigger::File, 120, 28, 1));
    assert_eq!(ctrl.state().unwrap().kind, MenuKind::AppFile);
    assert_eq!(ctrl.state().unwrap().volume, 42);
    assert_eq!(ctrl.state().unwrap().battery, 77);

    assert!(ctrl.hover_top_bar(TopBarTrigger::View, 190, 28, 1));
    assert_eq!(ctrl.state().unwrap().kind, MenuKind::AppView);
    // Hovering the same trigger is a no-op.
    assert!(!ctrl.hover_top_bar(TopBarTrigger::View, 190, 28, 1));

    // Clicking outside dismisses cleanly.
    assert_eq!(ctrl.press(5, 700, 1280, 800, 1), PressResult::Dismiss);
    assert!(!ctrl.is_open());

    // Escape dismisses cleanly.
    ctrl.open(MenuState::new(MenuKind::AppEdit, 150, 32));
    assert!(ctrl.key_escape());
    assert!(!ctrl.is_open());
    assert!(!ctrl.key_escape());
}

#[test]
fn volume_and_battery_popovers_hit_controls_and_stay_open() {
    let mut ctrl = Controller::new();
    let st = MenuState::new(MenuKind::VolumePopover, 1060, 32);
    let l = Layout::new(&st, 1280, 800, 1).unwrap();
    assert!(l.panel.0 + l.panel.2 <= 1280);
    ctrl.open(st);

    // Click Vol -, Mute, Vol +, and slider bar: all keep popover open.
    let click_center = |r: Rect| ((r.0 + r.2 / 2) as i32, (r.1 + r.3 / 2) as i32);
    let (dx, dy) = click_center(l.vol_down);
    assert_eq!(
        ctrl.press(dx, dy, 1280, 800, 1),
        PressResult::Action { action: MenuAction::VolumeDown, closed: false }
    );
    let (mx, my) = click_center(l.vol_mute);
    assert_eq!(
        ctrl.press(mx, my, 1280, 800, 1),
        PressResult::Action { action: MenuAction::VolumeMuteToggle, closed: false }
    );
    let (ux, uy) = click_center(l.vol_up);
    assert_eq!(
        ctrl.press(ux, uy, 1280, 800, 1),
        PressResult::Action { action: MenuAction::VolumeUp, closed: false }
    );
    let (bx, by) = click_center(l.vol_bar);
    assert!(matches!(
        ctrl.press(bx, by, 1280, 800, 1),
        PressResult::Action { action: MenuAction::VolumeSet(48..=52), closed: false }
    ));
    assert!(ctrl.is_open());

    // Battery popover: clicking power profile keeps open, clicking header does nothing.
    let bat = MenuState::new(MenuKind::BatteryPopover, 1090, 32);
    let bl = Layout::new(&bat, 1280, 800, 1).unwrap();
    ctrl.open(bat);
    let (hx, hy) = click_center(bl.row_rect(0));
    assert_eq!(ctrl.press(hx, hy, 1280, 800, 1), PressResult::Inside);
    let (px, py) = click_center(bl.row_rect(4));
    assert_eq!(
        ctrl.press(px, py, 1280, 800, 1),
        PressResult::Action { action: MenuAction::PowerProfilePerformance, closed: false }
    );
}

#[test]
fn right_click_desktop_and_window_context_menus_dispatch_and_clamp() {
    let mut ctrl = Controller::new();
    // Opened near bottom-right corner: layout clamps inside screen.
    let desk = MenuState::new(MenuKind::DesktopContext, 1260, 780);
    let l = Layout::new(&desk, 1280, 800, 1).unwrap();
    assert!(l.panel.0 + l.panel.2 <= 1280);
    assert!(l.panel.1 + l.panel.3 <= 800);
    ctrl.open(desk);

    // Context menu does not hover-switch to top bar menus.
    assert!(!ctrl.hover_top_bar(TopBarTrigger::File, 120, 28, 1));

    // Row 0 = New Terminal, Row 2 = Open Tailor AI, Row 5 = Display Settings.
    let r2 = l.row_rect(2);
    assert!(ctrl.hover((r2.0 + 4) as i32, (r2.1 + 4) as i32, 1280, 800, 1));
    assert_eq!(ctrl.state().unwrap().hover, Some(2));
    assert_eq!(
        ctrl.press((r2.0 + 4) as i32, (r2.1 + 4) as i32, 1280, 800, 1),
        PressResult::Action { action: MenuAction::LaunchTailor, closed: true }
    );
    assert!(!ctrl.is_open());

    // Window context menu: Minimize, Maximize/Restore, Bring to Front, Close.
    let win = MenuState::new(MenuKind::WindowContext, 300, 100);
    let wl = Layout::new(&win, 1280, 800, 1).unwrap();
    ctrl.open(win);
    let r3 = wl.row_rect(3);
    assert_eq!(
        ctrl.press((r3.0 + 4) as i32, (r3.1 + 4) as i32, 1280, 800, 1),
        PressResult::Action { action: MenuAction::CloseWindow, closed: true }
    );
}

#[test]
fn kde_and_hyprland_hotkeys_work_cleanly() {
    // Alt+F4 closes window; Alt+F10 toggles maximize.
    let mut k = desktop_keys::Keys::new();
    assert_eq!(k.event(0x38, 1), desktop_keys::Action::Consume); // Alt down
    assert_eq!(k.event(desktop_keys::KEY_F4, 1), desktop_keys::Action::Close);
    assert_eq!(k.event(desktop_keys::KEY_F4, 0), desktop_keys::Action::Consume);
    assert_eq!(k.event(desktop_keys::KEY_F10, 1), desktop_keys::Action::ToggleMaximize);
    assert_eq!(k.event(desktop_keys::KEY_F10, 0), desktop_keys::Action::Consume);
    assert_eq!(k.event(0x38, 0), desktop_keys::Action::Consume); // Alt up

    // Hyprland/KDE tabbing & grouping chords.
    let plain = wm::Mods { shift: false, ctrl: false };
    let shift = wm::Mods { shift: true, ctrl: false };
    assert_eq!(wm::binding(wm::code::G, plain), Some(wm::WmAction::Group));
    assert_eq!(wm::binding(wm::code::G, shift), Some(wm::WmAction::Ungroup));
    assert_eq!(wm::binding(wm::code::PAGE_DOWN, plain), Some(wm::WmAction::NextTab));
    assert_eq!(wm::binding(wm::code::PAGE_UP, plain), Some(wm::WmAction::PrevTab));
    assert_eq!(wm::binding(wm::code::GRAVE, plain), Some(wm::WmAction::NextTab));
    assert_eq!(wm::binding(wm::code::GRAVE, shift), Some(wm::WmAction::PrevTab));
}
