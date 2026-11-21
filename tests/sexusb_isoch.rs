// SPDX-License-Identifier: GPL-3.0-only
//! sexusb's isochronous arithmetic and the UVC camera scan, on the host.
#[path = "../servers/sexusb/src/isoch.rs"]
mod isoch;
#[path = "../servers/sexusb/src/uvc_scan.rs"]
mod uvc_scan;

use isoch::*;

#[test]
fn endpoint_parameters() {
    // High speed, 3 x 1024 bytes per microframe.
    let hs = Endpoint::new(HIGH, 0x1400, 1, None);
    assert_eq!((hs.mps, hs.burst, hs.mult, hs.interval), (1024, 2, 0, 0));
    assert_eq!(hs.esit_payload(), 3072);
    // SuperSpeed with a companion: burst 4 (5 packets), mult 1 (2 bursts).
    let ss = Endpoint::new(SUPER, 1024, 1, Some((4, 1)));
    assert_eq!(ss.esit_payload(), 1024 * 5 * 2);
    // Full speed: bInterval 1 = every frame = 8 microframes.
    let fs = Endpoint::new(FULL, 1023, 1, None);
    assert_eq!((fs.interval, fs.microframes(), fs.esit_payload()), (3, 8, 1023));
    assert_eq!(interval(HIGH, 4), 3);
    assert_eq!(interval(HIGH, 0), 0);
}

#[test]
fn trbs() {
    assert_eq!(packets(0, 1024), 1);
    assert_eq!(packets(3072, 1024), 3);
    assert_eq!(packets(3073, 1024), 4);
    assert_eq!(burst_counts(3, 2), (0, 2));
    assert_eq!(burst_counts(4, 2), (1, 0));
    assert_eq!(burst_counts(1, 0), (0, 0));
    let hs = Endpoint::new(HIGH, 0x1400, 1, None);
    let t = trb(0x1234_5678_9000, 3072, &hs, None, true, 1);
    assert_eq!(t[0], 0x5678_9000);
    assert_eq!(t[1], 0x1234);
    assert_eq!(t[2], 3072);
    assert_eq!((t[3] >> 10) & 0x3F, TRB_ISOCH);
    assert_eq!(t[3] & 1, 1);
    assert_ne!(t[3] & (1 << 5), 0, "IOC");
    assert_ne!(t[3] & (1 << 31), 0, "SIA");
    assert_eq!((t[3] >> 16) & 15, 2, "TLBPC");
    let t = trb(0, 1024, &hs, Some(0x7FF), false, 0);
    assert_eq!((t[3] >> 20) & 0x7FF, 0x7FF);
    assert_eq!(t[3] & (1 << 31), 0);
    assert_eq!(pick_alt(&[(1, 128), (2, 512), (3, 3072)], 400), Some(2));
    assert_eq!(pick_alt(&[(1, 128), (2, 512)], 4000), Some(2));
    assert_eq!(pick_alt(&[], 1), None);
}

/// A laptop webcam: RGB camera (interfaces 0/1) and an IR camera (2/3), high speed.
fn webcam() -> Vec<u8> {
    let mut c = vec![9, 2, 0, 0, 4, 1, 0, 0x80, 250];
    c.extend_from_slice(&[8, 0x0B, 0, 2, 0x0E, 3, 0, 0]); // IAD
    c.extend_from_slice(&[9, 4, 0, 0, 1, 0x0E, 1, 0, 0]); // VideoControl
    c.extend_from_slice(&[7, 5, 0x83, 3, 16, 0, 6]);      // its interrupt endpoint (ignored)
    c.extend_from_slice(&[9, 4, 1, 0, 0, 0x0E, 2, 0, 0]); // VideoStreaming alt 0
    c.extend_from_slice(&[11, 0x24, 6, 1, 2, 0, 1, 0, 0, 0, 0]); // MJPEG, 2 frames
    c.extend_from_slice(&[30, 0x24, 7, 1, 0, 0x80, 7, 0x38, 4]); c.extend_from_slice(&[0; 21]); // 1920x1080
    c.extend_from_slice(&[30, 0x24, 7, 2, 0, 0, 5, 0xD0, 2]); c.extend_from_slice(&[0; 21]);    // 1280x720
    let mut yuy2 = vec![27, 0x24, 4, 2, 1];
    yuy2.extend_from_slice(b"YUY2"); yuy2.extend_from_slice(&[0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
    yuy2.extend_from_slice(&[16, 1, 0, 0, 0, 0]);
    c.extend_from_slice(&yuy2);
    c.extend_from_slice(&[30, 0x24, 5, 1, 0, 0x80, 2, 0xE0, 1]); c.extend_from_slice(&[0; 21]); // 640x480
    for (alt, mps) in [(1u8, 0x0080u16), (2, 0x0400), (3, 0x1400)] {
        c.extend_from_slice(&[9, 4, 1, alt, 1, 0x0E, 2, 0, 0]);
        c.extend_from_slice(&[7, 5, 0x81, 5, mps as u8, (mps >> 8) as u8, 1]);
    }
    c.extend_from_slice(&[9, 4, 2, 0, 1, 0x0E, 1, 0, 0]); // IR VideoControl
    c.extend_from_slice(&[9, 4, 3, 0, 0, 0x0E, 2, 0, 0]); // IR VideoStreaming
    c.extend_from_slice(&[9, 4, 3, 1, 1, 0x0E, 2, 0, 0]);
    c.extend_from_slice(&[7, 5, 0x82, 5, 0x00, 0x04, 1]);
    let total = c.len() as u16;
    c[2..4].copy_from_slice(&total.to_le_bytes());
    c
}

#[test]
fn finds_the_webcam() {
    let cam = uvc_scan::scan(&webcam()).expect("camera");
    assert_eq!((cam.control_iface, cam.stream_iface, cam.more_streams), (0, 1, 1));
    assert_eq!(cam.alts().iter().map(|a| (a.alt, a.ep, a.w_max_packet)).collect::<Vec<_>>(), [(1, 0x81, 0x80), (2, 0x81, 0x400), (3, 0x81, 0x1400)]);
    assert_eq!(cam.max_payload(HIGH), 3072);
    assert!(!cam.bulk());
    let f: Vec<_> = cam.formats().map(|f| (f.format, f.frames, f.max_w, f.max_h)).collect();
    assert_eq!(f, [(uvc_scan::Format::Mjpeg, 2, 1920, 1080), (uvc_scan::Format::Yuy2, 1, 640, 480)]);
    // A webcam needs ~ 1920x1080 MJPEG at 30 fps, well within alt 3.
    let alts: Vec<(u8, u32)> = cam.alts().iter().map(|a| (a.alt, a.endpoint(HIGH).esit_payload())).collect();
    assert_eq!(pick_alt(&alts, 2000), Some(3));
}

#[test]
fn not_a_camera_and_truncation() {
    assert!(uvc_scan::scan(&[9, 2, 18, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0]).is_none());
    let w = webcam();
    for cut in 0..w.len() { let _ = uvc_scan::scan(&w[..cut]); }
}
