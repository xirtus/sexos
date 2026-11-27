// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for Carbon-1 Background App Lifecycle, Top-Right Tray & Switcher,
//! Noir Close Policy Sheet, Force Quit, Crimson Lips emblem, and Ctrl+Left-Click translation.

#[path = "../crates/sex-pdx/src/bg_apps.rs"]
mod bg_apps;
#[path = "../crates/sex-pdx/src/brand.rs"]
mod brand;
#[path = "../servers/silk-shell/src/wm.rs"]
#[allow(dead_code)]
mod wm;

use bg_apps::*;

#[test]
fn close_policy_defaults_and_persisted_masks() {
    // Background worker apps pre-default to KeepInSilkBar.
    for name in [b"tether".as_slice(), b"tailor", b"swingers", b"qupid"] {
        let kind = classify_name(name);
        assert_eq!(kind.default_policy(), ClosePolicy::KeepInSilkBar);
        assert_eq!(resolve_policy(name, kind, 0, 0), ClosePolicy::KeepInSilkBar);
    }
    // Ordinary interactive apps default to Ask.
    for name in [b"abacus".as_slice(), b"loom", b"tatami", b"sexshop", b"spindle"] {
        let kind = classify_name(name);
        assert_eq!(kind.default_policy(), ClosePolicy::Ask);
        assert_eq!(resolve_policy(name, kind, 0, 0), ClosePolicy::Ask);
    }

    // Remember choice overrides default via persisted bitmasks.
    let mut keep_mask = 0i32;
    let mut quit_mask = 0i32;
    let abacus_bit = policy_bit_for_name(b"abacus");
    assert!(abacus_bit.is_some());
    apply_remember_policy(abacus_bit, ClosePolicy::KeepInSilkBar, &mut keep_mask, &mut quit_mask);
    assert_eq!(resolve_policy(b"abacus", AppKind::Generic, keep_mask, quit_mask), ClosePolicy::KeepInSilkBar);

    // Explicitly switching tether to QuitApp overrides KeepInSilkBar default.
    let tether_bit = policy_bit_for_name(b"tether");
    apply_remember_policy(tether_bit, ClosePolicy::QuitApp, &mut keep_mask, &mut quit_mask);
    assert_eq!(resolve_policy(b"tether", AppKind::Tether, keep_mask, quit_mask), ClosePolicy::QuitApp);
    // Abacus preference stayed intact.
    assert_eq!(resolve_policy(b"abacus", AppKind::Generic, keep_mask, quit_mask), ClosePolicy::KeepInSilkBar);
}

#[test]
fn wire_round_trip_open_parked_and_confirm_sheet() {
    let mut s = State::EMPTY;
    s.open_count = 2;
    s.open[0] = OpenWindow {
        sid: 358,
        kind: AppKind::Settings,
        scene: 0,
        focused: true,
        name: pack_fixed_name(b"Settings").0,
        name_len: 8,
    };
    s.open[1] = OpenWindow {
        sid: 153,
        kind: AppKind::Spindle,
        scene: 0,
        focused: false,
        name: pack_fixed_name(b"Spindle").0,
        name_len: 7,
    };
    s.parked_count = 2;
    s.parked[0] = ParkedApp {
        target_pd: 41,
        sid: 359,
        kind: AppKind::Tether,
        status: 1,
        name: pack_fixed_name(b"Tether").0,
        name_len: 6,
    };
    s.parked[1] = ParkedApp {
        target_pd: 42,
        sid: 360,
        kind: AppKind::Tailor,
        status: 1,
        name: pack_fixed_name(b"Tailor").0,
        name_len: 6,
    };
    s.overlay = OverlayKind::CloseConfirm;
    s.remember = true;
    s.confirm_sid = 358;
    s.confirm_target = 43;
    s.confirm_kind = AppKind::Settings;
    let (cname, clen) = pack_fixed_name(b"System Settings");
    s.confirm_name = cname;
    s.confirm_name_len = clen;
    s.bar_right_edge = 1018;
    s.hover = HoverTarget::CloseKeep;

    let mut rx = Receiver::new();
    let ok = send_calls(&s, 42, |w| {
        let _ = rx.push(w[0], w[1], w[2]);
        true
    });
    assert!(ok);
    assert_eq!(rx.shown, s);
}

#[test]
fn top_bar_tray_and_overlays_hit_test_and_clamp() {
    let ws_last_right = 748usize;
    let right_edge = 1018usize;
    for scale in 1..=3u32 {
        let sc = scale as usize;
        let bar = BarLayout::compute(28 * sc, scale, ws_last_right * sc, right_edge * sc, 2);
        assert_eq!(bar.parked_count, 2);
        // Switcher pill sits right-most inside [ws_last_right .. right_edge].
        assert!(bar.switcher_pill.0 >= ws_last_right * sc);
        assert_eq!(bar.switcher_pill.0 + bar.switcher_pill.2, right_edge * sc);
        // Parked badges sit immediately to the left of the Switcher pill.
        assert!(bar.parked_badges[0].0 + bar.parked_badges[0].2 < bar.switcher_pill.0);
        assert!(bar.parked_badges[1].0 + bar.parked_badges[1].2 < bar.parked_badges[0].0);
        assert!(bar.tray_right_edge < bar.parked_badges[1].0);

        let mid = |r: Rect| (r.0 + r.2 / 2, r.1 + r.3 / 2);
        let (sx, sy) = mid(bar.switcher_pill);
        assert_eq!(bar.hit(sx, sy), BarHit::SwitcherPill);
        let (px, py) = mid(bar.parked_badges[1]);
        assert_eq!(bar.hit(px, py), BarHit::ParkedBadge(1));
    }

    // Two-Section Switcher layout hit-tests Open focus/close and Parked Reopen/Quit.
    let mut state = State::EMPTY;
    state.open_count = 1;
    state.parked_count = 1;
    state.overlay = OverlayKind::Switcher;
    state.bar_right_edge = 1018;
    let bar = BarLayout::compute(28, 1, 748, 1018, 1);
    let ol = OverlayLayout::compute(&state, &bar, 1280, 800, 28, 1).unwrap();
    let OverlayLayout::Switcher(sw) = ol else { panic!("expected Switcher") };
    assert!(sw.panel.0 + sw.panel.2 <= 1280);
    let mid = |r: Rect| (r.0 + r.2 / 2, r.1 + r.3 / 2);
    assert_eq!(ol.hit(mid(sw.open_rows[0].0).0, mid(sw.open_rows[0].0).1), OverlayHit::SwitcherOpenFocus(0));
    assert_eq!(ol.hit(mid(sw.open_rows[0].1).0, mid(sw.open_rows[0].1).1), OverlayHit::SwitcherOpenClose(0));
    assert_eq!(ol.hit(mid(sw.parked_rows[0].1).0, mid(sw.parked_rows[0].1).1), OverlayHit::SwitcherParkedReopen(0));
    assert_eq!(ol.hit(mid(sw.parked_rows[0].2).0, mid(sw.parked_rows[0].2).1), OverlayHit::SwitcherParkedQuit(0));

    // Close Sheet layout hits checkbox + 3 action buttons.
    state.overlay = OverlayKind::CloseConfirm;
    let olc = OverlayLayout::compute(&state, &bar, 1280, 800, 28, 1).unwrap();
    let OverlayLayout::CloseConfirm(cl) = olc else { panic!("expected CloseConfirm") };
    assert_eq!(olc.hit(mid(cl.remember_row).0, mid(cl.remember_row).1), OverlayHit::CloseRememberToggle);
    assert_eq!(olc.hit(mid(cl.keep_btn).0, mid(cl.keep_btn).1), OverlayHit::CloseKeepInSilkBar);
    assert_eq!(olc.hit(mid(cl.quit_btn).0, mid(cl.quit_btn).1), OverlayHit::CloseQuitApp);
    assert_eq!(olc.hit(mid(cl.cancel_btn).0, mid(cl.cancel_btn).1), OverlayHit::CloseCancel);
}

#[test]
fn ctrl_left_click_translates_with_release_symmetry_and_spindle_exemption() {
    let mut tr = CtrlClickTranslator::new();

    // Plain left click stays button 1.
    assert_eq!(tr.translate(1, true, false, false), 1);
    assert_eq!(tr.translate(1, false, false, false), 1);

    // Ctrl + Left-Click becomes Right-Click (button 2), even if Ctrl releases before mouse-up.
    assert_eq!(tr.translate(1, true, true, false), 2);
    assert_eq!(tr.translate(1, false, false, false), 2);

    // Next normal left click after symmetry release is back to button 1.
    assert_eq!(tr.translate(1, true, false, false), 1);
    assert_eq!(tr.translate(1, false, true, false), 1);

    // Inside Spindle terminal content, raw Ctrl+Left-Click passes through as button 1.
    assert_eq!(tr.translate(1, true, true, true), 1);
    assert_eq!(tr.translate(1, false, true, true), 1);
}

#[test]
fn crimson_lips_emblem_color_and_sculpted_highlight() {
    let edge = brand::color_at(brand::Tone::Crimson, 0, 100);
    let mid = brand::color_at(brand::Tone::Crimson, 50, 100);
    assert_eq!(edge, 0xE11D48);
    assert_ne!(mid, edge);
    let rest = brand::color_at_sculpted(brand::Tone::Crimson, 10, 4, 20, 14, 0);
    let hovered = brand::color_at_sculpted(brand::Tone::Crimson, 10, 4, 20, 14, 255);
    assert_ne!(rest, hovered);
}

#[test]
fn quick_action_force_quit_about_layouts_and_wm_close_window_binding() {
    let mut state = State::EMPTY;
    state.parked_count = 1;
    state.parked[0] = ParkedApp {
        target_pd: 41,
        sid: 359,
        kind: AppKind::Tether,
        status: 1,
        name: pack_fixed_name(b"Tether").0,
        name_len: 6,
    };
    state.bar_right_edge = 1018;
    let bar = BarLayout::compute(28, 1, 748, 1018, 1);
    let mid = |r: Rect| (r.0 + r.2 / 2, r.1 + r.3 / 2);

    // QuickAction popover hit-testing + contextual quick actions.
    state.overlay = OverlayKind::QuickAction;
    state.overlay_idx = 0;
    let qa = OverlayLayout::compute(&state, &bar, 1280, 800, 28, 1).unwrap();
    let OverlayLayout::QuickAction(ql) = qa else { panic!("expected QuickAction") };
    assert_eq!(AppKind::Tether.quick_actions()[0], "Reconnect Host Link");
    assert_eq!(qa.hit(mid(ql.quick_rows[0]).0, mid(ql.quick_rows[0]).1), OverlayHit::QuickAction(0));
    assert_eq!(qa.hit(mid(ql.quick_rows[1]).0, mid(ql.quick_rows[1]).1), OverlayHit::QuickAction(1));
    assert_eq!(qa.hit(mid(ql.reopen_btn).0, mid(ql.reopen_btn).1), OverlayHit::QuickReopen);
    assert_eq!(qa.hit(mid(ql.quit_btn).0, mid(ql.quit_btn).1), OverlayHit::QuickQuit);

    // ForceQuit dialog hit-testing.
    state.open_count = 1;
    state.overlay = OverlayKind::ForceQuit;
    let fq = OverlayLayout::compute(&state, &bar, 1280, 800, 28, 1).unwrap();
    let OverlayLayout::ForceQuit(fl) = fq else { panic!("expected ForceQuit") };
    assert_eq!(fq.hit(mid(fl.rows[0]).0, mid(fl.rows[0]).1), OverlayHit::ForceSelect(0));
    assert_eq!(fq.hit(mid(fl.force_quit_btn).0, mid(fl.force_quit_btn).1), OverlayHit::ForceQuitSelected);
    assert_eq!(fq.hit(mid(fl.close_btn).0, mid(fl.close_btn).1), OverlayHit::ForceClose);

    // AboutSexOs card hit-testing.
    state.overlay = OverlayKind::AboutSexOs;
    let ab = OverlayLayout::compute(&state, &bar, 1280, 800, 28, 1).unwrap();
    let OverlayLayout::AboutSexOs(al) = ab else { panic!("expected AboutSexOs") };
    assert_eq!(ab.hit(mid(al.settings_btn).0, mid(al.settings_btn).1), OverlayHit::AboutOpenSettings);
    assert_eq!(ab.hit(mid(al.close_btn).0, mid(al.close_btn).1), OverlayHit::AboutClose);

    // Super+W remains Overview while Super+Ctrl+W binds CloseWindow and Super+Q binds Close (Quit).
    let plain = wm::Mods::default();
    let ctrl = wm::Mods { shift: false, ctrl: true };
    assert_eq!(wm::binding(wm::code::W, plain), Some(wm::WmAction::Overview));
    assert_eq!(wm::binding(wm::code::W, ctrl), Some(wm::WmAction::CloseWindow));
    assert_eq!(wm::binding(wm::code::Q, plain), Some(wm::WmAction::Close));
}
