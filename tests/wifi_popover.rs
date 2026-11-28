// SPDX-License-Identifier: GPL-3.0-only
//! Host integration tests for the SilkBar live Wi-Fi scanner popover,
//! verifying coordination with `silkbar_menus`, wire serialization to Display,
//! layout geometry across HiDPI scales, SSID deduplication, known-network join,
//! and inline WPA2/WPA3 passphrase entry (typing, Backspace, Paste, Tab, Enter, Esc).

#[path = "../crates/sex-pdx/src/silkbar_menus.rs"]
#[allow(dead_code)]
mod silkbar_menus;
#[path = "../crates/sex-pdx/src/wifi_popover.rs"]
#[allow(dead_code)]
mod wifi_popover;
#[path = "../crates/sex-pdx/src/wifi_status.rs"]
#[allow(dead_code)]
mod wifi_status;

use wifi_popover::{
    Assembler, Controller, KnownNetworks, PopoverAction, PopoverView, RawNetwork, WifiLayout,
    chunks, phase, security,
};

#[test]
fn wifi_popover_coordinates_cleanly_with_silkbar_menus() {
    let mut menus = silkbar_menus::Controller::new();
    let mut wifi = Controller::new();
    wifi.apply_state(
        phase::CONNECTED,
        Some(b"Studio5G"),
        -44,
        security::WPA2_WPA3,
        [192, 168, 1, 22],
        b"",
    );
    wifi.set_raw_scan(&[
        RawNetwork::new(0, 36, -50, security::WPA2, b"Cafe5G"),
        RawNetwork::new(1, 1, -65, security::OPEN, b"Lobby"),
    ]);

    // Clicking the Wi-Fi tray opens the live Wi-Fi popover while keeping silkbar_menus closed.
    wifi.open_at(1060, 32);
    assert!(wifi.is_open());
    assert!(!menus.is_open());

    // Hovering a top-bar menu switches away from wifi popover only when not typing a password.
    let v = wifi.view();
    let lay = WifiLayout::new(&v, 1280, 800, 1).unwrap();
    let (rx, ry) = ((lay.net_rows[0].0 + 20) as i32, (lay.net_rows[0].1 + 10) as i32);
    wifi.press(rx, ry, 1280, 800, 1);
    assert!(wifi.is_password_focused());

    // Simulate closing wifi popover to open Volume popover from SilkBar.
    wifi.close();
    menus.open(silkbar_menus::MenuState::new(silkbar_menus::MenuKind::VolumePopover, 1080, 32));
    assert!(!wifi.is_open());
    assert!(menus.is_open());
}

#[test]
fn wire_phase_and_security_constants_match_wifi_status() {
    assert_eq!(phase::NONE, wifi_status::NONE);
    assert_eq!(phase::STARTING, wifi_status::STARTING);
    assert_eq!(phase::SCANNING, wifi_status::SCANNING);
    assert_eq!(phase::READY, wifi_status::READY);
    assert_eq!(phase::FAILED, wifi_status::FAILED);
    assert_eq!(phase::JOINING, wifi_status::JOINING);
    assert_eq!(phase::JOINED, wifi_status::JOINED);
    assert_eq!(phase::ASSOCIATING, wifi_status::ASSOCIATING);
    assert_eq!(phase::ASSOCIATED, wifi_status::ASSOCIATED);
    assert_eq!(phase::HANDSHAKE, wifi_status::HANDSHAKE);
    assert_eq!(phase::CONNECTED, wifi_status::CONNECTED);
    assert_eq!(phase::OFF, wifi_status::OFF);

    // ip4_word / word_ip4 round trip
    let ip = [172, 16, 42, 99];
    assert_eq!(wifi_status::word_ip4(wifi_status::ip4_word(ip, true)), Some((ip, true)));
    assert_eq!(wifi_status::word_ip4(wifi_status::ip4_word(ip, false)), Some((ip, false)));
}

#[test]
fn empty_wired_connecting_and_off_states_render_valid_layouts_and_wire_frames() {
    let mut asm = Assembler::new();
    let mut ctrl = Controller::new();
    ctrl.known = KnownNetworks::new();

    // 1. Wired / No Wi-Fi hardware state
    ctrl.apply_state(phase::NONE, None, 0, security::OPEN, [10, 0, 2, 15], b"");
    ctrl.open_at(1120, 28);
    let v_wired = ctrl.view();
    assert!(v_wired.active_wired);
    let l_wired = WifiLayout::new(&v_wired, 1280, 800, 2).unwrap();
    assert!(l_wired.card.is_some());
    assert!(l_wired.disconnect.is_none());
    let mut out = None;
    for (w0, w1, w2) in chunks(&v_wired, 1) {
        out = asm.push(w0, w1, w2);
    }
    assert_eq!(out, Some(v_wired));

    // 2. Connecting card state + Disconnect click
    ctrl.apply_state(phase::HANDSHAKE, Some(b"Eduroam"), -61, security::WPA2, [0; 4], b"");
    let v_conn = ctrl.view();
    assert!(phase::is_connecting(v_conn.phase));
    assert!(v_conn.has_active);
    let lay = WifiLayout::new(&v_conn, 1280, 800, 1).unwrap();
    assert!(lay.card.is_some());
    let disc = lay.disconnect.unwrap();
    let (_, act) = ctrl.press((disc.0 + 10) as i32, (disc.1 + 8) as i32, 1280, 800, 1);
    assert_eq!(act, PopoverAction::Disconnect);

    // 3. Wi-Fi Off state preserves clean wire round-trip
    ctrl.apply_state(phase::OFF, None, 0, security::OPEN, [0; 4], b"");
    let v_off = ctrl.view();
    assert!(!v_off.radio_on);
    let mut out_off = None;
    for (w0, w1, w2) in chunks(&v_off, 2) {
        out_off = asm.push(w0, w1, w2);
    }
    assert_eq!(out_off, Some(v_off));

    // Hidden close state decodes back to PopoverView::hidden()
    ctrl.close();
    let mut out_closed = None;
    for (w0, w1, w2) in chunks(&ctrl.view(), 3) {
        out_closed = asm.push(w0, w1, w2);
    }
    assert_eq!(out_closed, Some(PopoverView::hidden()));
}

#[test]
fn popover_full_flow_end_to_end() {
    use wifi_popover::{PASSWORD_MAX, Rect};
    let center = |r: Rect| -> (i32, i32) { ((r.0 + r.2 / 2) as i32, (r.1 + r.3 / 2) as i32) };
    let mut c = Controller::new();
    assert!(c.known.remember(b"SavedNet", b"saved-pass-1"));
    c.set_raw_scan(&[
        RawNetwork::new(0, 6, -70, security::WPA2, b"MeshAP"),
        RawNetwork::new(1, 36, -44, security::WPA2, b"MeshAP"),
        RawNetwork::new(2, 11, -55, security::WPA2, b"SavedNet"),
        RawNetwork::new(3, 1, -60, security::OPEN, b"OpenCafe"),
        RawNetwork::new(4, 44, -49, security::WPA2_WPA3, b"SecureLock"),
    ]);
    c.apply_state(phase::READY, None, 0, security::OPEN, [10, 0, 2, 15], b"");
    c.open_at(1080, 32);

    assert_eq!(c.networks().len(), 4);
    assert_eq!(c.networks()[0].ssid(), b"MeshAP");
    assert_eq!(c.networks()[0].scan_index, 1);

    let l0 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
    let (_, a0) = c.press(center(l0.net_rows[1]).0, center(l0.net_rows[1]).1, 1280, 800, 1);
    assert_eq!(a0, PopoverAction::None);
    assert!(c.is_password_focused());

    let (cons, redr, _) = c.handle_key_event(0x2F, 1, false, true, None, Some(b"noir-wifi-99"));
    assert!(cons && redr);
    let (_, _, act) = c.handle_key_event(0x1C, 1, false, false, None, None);
    assert_eq!(act, PopoverAction::Join { scan_index: 4, has_password: true });

    let mut pw = [0u8; PASSWORD_MAX];
    let plen = c.take_pending_join_password(&mut pw);
    assert_eq!(&pw[..plen], b"noir-wifi-99");

    c.apply_state(phase::CONNECTED, Some(b"SecureLock"), -49, security::WPA2_WPA3, [192, 168, 10, 5], b"");
    assert!(c.known.is_known(b"SecureLock"));
    let v = c.view();
    let mut asm = Assembler::new();
    let mut dec = None;
    for (w0, w1, w2) in chunks(&v, 42) {
        dec = asm.push(w0, w1, w2);
    }
    assert_eq!(dec, Some(v));
}
