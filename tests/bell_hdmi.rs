// SPDX-License-Identifier: GPL-3.0-only
//! Bell's HDMI/DisplayPort codec half: display pins, the monitor's ELD, the InfoFrame.
#[path="../servers/sexbell/src/hdmi.rs"]mod hdmi;
use hdmi::*;

/// An ELD as an Intel display codec returns it for an HDMI monitor named
/// "DELL U2720Q" listing LPCM 2 ch at 32/44.1/48 kHz and 8 ch at 48/96/192 kHz.
fn eld() -> Vec<u8> {
    let name = b"DELL U2720Q";
    let mut b = vec![0u8; 20];
    b[0] = 2 << 3;                       // ELD version 2
    b[4] = name.len() as u8;             // CEA EDID version 0, monitor name length
    b[5] = 2 << 4;                       // two SADs, connection type HDMI
    b.extend_from_slice(name);
    b.extend_from_slice(&[0x09, 0x07, 0x07]); // LPCM, 2 ch, 32/44.1/48, 16/20/24 bit
    b.extend_from_slice(&[0x0f, 0x54, 0x07]); // LPCM, 8 ch, 48/96/192
    b[2] = ((b.len() - 4 + 3) / 4) as u8;
    b.resize(4 + b[2] as usize * 4, 0);
    b
}

#[test] fn eld_names_the_monitor_and_its_pcm() {
    let e = parse_eld(&eld()).unwrap();
    assert_eq!(e.name(), b"DELL U2720Q");
    assert_eq!(e.pcm_channels, 8);
    assert_eq!(e.pcm_rates, 0x57);
    assert!(e.plays_48k() && !e.displayport);
}
#[test] fn displayport_and_bad_elds() {
    let mut b = eld(); b[5] |= 1 << 2;
    assert!(parse_eld(&b).unwrap().displayport);
    assert!(parse_eld(&b[..10]).is_none());
    let mut v = eld(); v[0] = 1 << 3; assert!(parse_eld(&v).is_none());
    let mut long = eld(); long[5] = 9 << 4; assert!(parse_eld(&long).is_none());
}
#[test] fn eld_without_sads_means_basic_stereo() {
    let mut b = vec![0u8; 24]; b[0] = 2 << 3; b[2] = 5;
    let e = parse_eld(&b).unwrap();
    assert_eq!((e.pcm_channels, e.plays_48k(), e.name_len), (2, true, 0));
}
#[test] fn infoframe_checksums_to_zero() {
    for ch in [2u8, 8] {
        let f = audio_infoframe(ch);
        assert_eq!((f[0], f[1], f[2], f[4]), (0x84, 1, 10, ch - 1));
        assert_eq!(f.iter().fold(0u8, |s, b| s.wrapping_add(*b)), 0);
    }
}
#[test] fn intel_codecs_are_told_apart() {
    assert!(is_intel_display_codec(0x8086_2812));
    assert!(!is_intel_display_codec(0x10ec_0295));
}
