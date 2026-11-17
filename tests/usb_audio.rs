// SPDX-License-Identifier: GPL-3.0-only
//! USB sound cards on the host: the UAC descriptor parser and the Bell-SexUSB ring.
#[path = "../servers/sexusb/src/uac.rs"]
mod uac;
#[allow(dead_code)]
#[path = "../crates/sex-pdx/src/usb_audio.rs"]
mod usb_audio;

/// QEMU's usb-audio (UAC1, full speed): AC interface 0, AS interface 1 with alt 1
/// stereo 16-bit 48 kHz on an adaptive OUT endpoint of 192 bytes, every 1 ms.
fn qemu_usb_audio() -> Vec<u8> {
    let mut c = vec![9, 2, 0, 0, 2, 1, 0, 0xc0, 50];
    c.extend([9, 4, 0, 0, 0, 1, 1, 0, 0]);
    c.extend([9, 0x24, 1, 0, 1, 0x1e, 0, 1, 1]);
    c.extend([9, 4, 1, 0, 0, 1, 2, 0, 0]);
    c.extend([9, 4, 1, 1, 1, 1, 2, 0, 0]);
    c.extend([7, 0x24, 1, 1, 0, 1, 0]);
    c.extend([11, 0x24, 2, 1, 2, 2, 16, 1, 0x80, 0xbb, 0x00]);
    c.extend([9, 5, 0x01, 0x0d, 192, 0, 1, 0, 0]);
    c.extend([7, 0x25, 1, 0, 0, 0, 0]);
    let n = c.len() as u16;
    c[2..4].copy_from_slice(&n.to_le_bytes());
    c
}

#[test]
fn finds_uac1_playback() {
    let p = uac::playback(&qemu_usb_audio()).unwrap();
    assert_eq!((p.uac2, p.config, p.control, p.iface, p.alt, p.ep, p.mps, p.interval, p.sync), (false, 1, 0, 1, 1, 1, 192, 1, 3));
    assert_eq!(uac::set_rate(&p), (0x22, 0x0100, 1, [0x80, 0xbb, 0, 0], 3));
    assert_eq!(uac::service_us(false, 1), 1000);
    assert_eq!(uac::xhci_interval(false, 1), 3);
}

#[test]
fn skips_settings_it_cannot_play() {
    let mut c = qemu_usb_audio();
    // 44.1 kHz only.
    let f = c.windows(3).position(|w| w == [0x80, 0xbb, 0x00]).unwrap();
    c[f..f + 3].copy_from_slice(&[0x44, 0xac, 0x00]);
    assert_eq!(uac::playback(&c), None);
    // Mono.
    let mut c = qemu_usb_audio();
    let f = c.windows(3).position(|w| w == [0x24, 2, 1]).unwrap();
    c[f + 3] = 1;
    assert_eq!(uac::playback(&c), None);
}

#[test]
fn finds_uac2_playback() {
    let mut c = vec![9, 2, 0, 0, 2, 1, 0, 0x80, 50];
    c.extend([8, 0x0b, 0, 2, 1, 0, 0x20, 0]);
    c.extend([9, 4, 0, 0, 0, 1, 1, 0x20, 0]);
    c.extend([9, 0x24, 1, 0, 2, 8, 0x40, 0, 0]);
    c.extend([8, 0x24, 0x0a, 0x29, 3, 7, 0, 0]);
    c.extend([9, 4, 1, 0, 0, 1, 2, 0x20, 0]);
    c.extend([9, 4, 1, 1, 2, 1, 2, 0x20, 0]);
    c.extend([16, 0x24, 1, 2, 0, 1, 1, 0, 0, 0, 2, 3, 0, 0, 0, 0]);
    c.extend([6, 0x24, 2, 1, 2, 16]);
    c.extend([7, 5, 0x01, 0x05, 0x88, 0x00, 1]);
    c.extend([7, 5, 0x81, 0x11, 4, 0, 4]);
    let p = uac::playback(&c).unwrap();
    assert_eq!((p.uac2, p.control, p.iface, p.alt, p.ep, p.mps, p.sync, p.clock), (true, 0, 1, 1, 1, 0x88, 1, 0x29));
    assert_eq!(uac::set_rate(&p), (0x21, 0x0100, 0x2900, [0x80, 0xbb, 0, 0], 4));
    assert_eq!(uac::service_us(true, 1), 125);
    assert_eq!(uac::xhci_interval(true, 1), 0);
}
