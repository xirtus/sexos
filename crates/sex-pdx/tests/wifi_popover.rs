// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Live Wi-Fi Scanner popover tests: `cargo test -p sex-pdx --test wifi_popover`.
use sex_pdx::wifi_popover::*;
use sex_pdx::wifi_status;

fn center(r: Rect) -> (i32, i32) {
    ((r.0 + r.2 / 2) as i32, (r.1 + r.3 / 2) as i32)
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

    let ip = [172, 16, 42, 99];
    assert_eq!(wifi_status::word_ip4(wifi_status::ip4_word(ip, true)), Some((ip, true)));
    assert_eq!(wifi_status::word_ip4(wifi_status::ip4_word(ip, false)), Some((ip, false)));
}

#[test]
fn popover_full_flow_end_to_end() {
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

    // Dedup keeps strongest MeshAP (-44, scan_index 1) first.
    assert_eq!(c.networks().len(), 4);
    assert_eq!(c.networks()[0].ssid(), b"MeshAP");
    assert_eq!(c.networks()[0].scan_index, 1);

    // Click unknown secured row SecureLock (index 1 in sorted list).
    let l0 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
    let (_, a0) = c.press(center(l0.net_rows[1]).0, center(l0.net_rows[1]).1, 1280, 800, 1);
    assert_eq!(a0, PopoverAction::None);
    assert!(c.is_password_focused());

    // Paste passphrase with Super+V and submit via Enter.
    let (cons, redr, _) = c.handle_key_event(0x2F, 1, false, true, None, Some(b"noir-wifi-99"));
    assert!(cons && redr);
    let (_, _, act) = c.handle_key_event(0x1C, 1, false, false, None, None);
    assert_eq!(act, PopoverAction::Join { scan_index: 4, has_password: true });

    let mut pw = [0u8; PASSWORD_MAX];
    let plen = c.take_pending_join_password(&mut pw);
    assert_eq!(&pw[..plen], b"noir-wifi-99");

    // Round-trip Display snapshot cleanly through wire chunks/Assembler.
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
