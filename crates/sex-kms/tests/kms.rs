// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
use sex_kms::commit::{self, Change, Commit, CommitError, FrameCounter, ALLOW_MODESET};
use sex_kms::edid::{self, Edid, EdidError};
use sex_kms::layout::{Desktop, Output};
use sex_kms::mode::{cta_vic, cvt_rb, Mode, FLAG_PREFERRED};
use sex_kms::model::*;
use sex_kms::wire;

#[path = "fixtures/edid.rs"]
mod fixtures;
use fixtures::{panel_edid, tv_edid};

#[test]
fn panel_edid_parses() {
    let e = Edid::parse(&panel_edid()).unwrap();
    assert_eq!(e.vendor_str(), "AUO");
    assert_eq!(e.product, 0x409d);
    assert_eq!(e.year, 2020);
    assert_eq!(e.bpc, Some(8));
    assert_eq!((e.width_mm, e.height_mm), (382, 215));
    assert_eq!(e.name, "AUO B173HAN");
    assert_eq!(e.range.unwrap().max_hz, 165);
    let p = e.preferred().unwrap();
    assert_eq!((p.hdisplay, p.vdisplay, p.refresh_hz()), (1920, 1080, 165));
    assert!(p.flags & FLAG_PREFERRED != 0);
    assert!(e.modes.iter().any(|m| m.hdisplay == 640 && m.refresh_hz() == 60));
    // Standard 1920x1080@60 CVT-RB equals the second DTD, so it is not listed twice.
    assert_eq!(e.modes.iter().filter(|m| m.hdisplay == 1920 && m.refresh_hz() == 60).count(), 1);
    assert_eq!(e.modes.len(), 3);
}

#[test]
fn tv_edid_has_cta_modes_and_hdmi() {
    let e = Edid::parse(&tv_edid()).unwrap();
    assert!(e.hdmi && e.audio);
    assert_eq!(e.vendor_str(), "DEL");
    assert!(e.modes.iter().any(|m| m.hdisplay == 3840 && m.refresh_hz() == 60));
    assert!(e.modes.iter().any(|m| m.hdisplay == 1280 && m.refresh_hz() == 50));
    assert_eq!(e.modes.iter().filter(|m| m.hdisplay == 1920 && m.vdisplay == 1080 && m.refresh_hz() == 60).count(), 1);
    assert_eq!(edid::total_len(&tv_edid()), 256);
}

#[test]
fn edid_errors_and_bad_extension() {
    assert_eq!(Edid::parse(&[0u8; 10]), Err(EdidError::TooShort));
    let mut b = panel_edid(); b[0] = 1;
    assert_eq!(Edid::parse(&b), Err(EdidError::BadHeader));
    let mut b = panel_edid(); b[100] ^= 1;
    assert_eq!(Edid::parse(&b), Err(EdidError::BadChecksum));
    let mut t = tv_edid(); t[140] ^= 0x55;
    let e = Edid::parse(&t).unwrap();
    assert_eq!(e.bad_extensions, 1);
    assert!(!e.hdmi);
}

#[test]
fn mode_math() {
    let m = cvt_rb(1920, 1080, 60).unwrap();
    assert_eq!((m.clock_khz, m.htotal, m.vtotal), (138_500, 2080, 1111));
    assert_eq!(m.refresh_hz(), 60);
    let m4 = cvt_rb(2560, 1600, 60).unwrap();
    assert_eq!((m4.clock_khz, m4.vtotal), (268_500, 1646));
    let v = cta_vic(16).unwrap();
    assert_eq!(v.refresh_mhz(), 60_000);
    assert_eq!(v.frame_us(), 16_666);
    assert!(v.vblank_us() > 600 && v.vblank_us() < 700);
    let meas = Mode::from_measured([1920, 2008, 2052, 2200], [1080, 1084, 1089, 1125], 0, 60_000);
    assert_eq!(meas.clock_khz, 148_500);
    assert!(!Mode::new(1, [10, 5, 6, 7], [1, 2, 3, 4], 0).valid());
}

fn laptop() -> (Device, State) {
    let mut edp = Connector::new(ConnectorKind::Edp, 1, 0b1111);
    edp.apply_edid(&Edid::parse(&panel_edid()).unwrap());
    let mut hdmi = Connector::new(ConnectorKind::Hdmi, 1, 0b1110);
    hdmi.status = Status::Disconnected;
    let mut planes = Vec::new();
    for c in 0..4 {
        planes.push(Plane { kind: PlaneKind::Primary, possible_crtcs: 1 << c, formats: Format::Xrgb8888.bit() | Format::Argb8888.bit(), max_width: 5120, max_height: 4096, scaling: false });
        planes.push(Plane { kind: PlaneKind::Cursor, possible_crtcs: 1 << c, formats: Format::Argb8888.bit(), max_width: 256, max_height: 256, scaling: false });
        planes.push(Plane { kind: PlaneKind::Overlay, possible_crtcs: 1 << c, formats: Format::Nv12.bit() | Format::Xrgb8888.bit(), max_width: 5120, max_height: 4096, scaling: true });
    }
    let dev = Device { connectors: vec![edp, hdmi], crtcs: 4, planes };
    let mut st = State::off(&dev);
    st.add_fb(Framebuffer { width: 1920, height: 1080, stride: 7680, format: Format::Xrgb8888 }).unwrap();
    st.add_fb(Framebuffer { width: 1920, height: 1080, stride: 7680, format: Format::Xrgb8888 }).unwrap();
    st.add_fb(Framebuffer { width: 64, height: 64, stride: 256, format: Format::Argb8888 }).unwrap();
    (dev, st)
}

#[test]
fn light_panel_then_flip_without_modeset() {
    let (dev, st) = laptop();
    let changes = commit::light(&dev, 0, 0, 0, 1920, 1080).unwrap();
    assert_eq!(commit::check(&dev, &st, Commit { changes: &changes, flags: 0 }), Err(CommitError::ModesetNotAllowed));
    let lit = commit::check(&dev, &st, Commit { changes: &changes, flags: ALLOW_MODESET }).unwrap();
    assert_eq!(lit.modeset, 1);
    let st = lit.state;
    assert!(!st.clone().remove_fb(0));
    let flip = [Change::Flip { plane: 0, fb: 1 }, Change::Plane { plane: 1, crtc: Some(0), fb: Some(2), src: Rect::new(0, 0, 64, 64), dst: Rect::new(100, 100, 64, 64) }];
    let f = commit::check(&dev, &st, Commit { changes: &flip, flags: 0 }).unwrap();
    assert_eq!((f.modeset, f.touched), (0, 1));
    // Cursor partly off the edge is fine; fully off is not.
    let mv = [Change::Move { plane: 1, x: 1900, y: 1070 }];
    assert!(commit::check(&dev, &f.state, Commit { changes: &mv, flags: 0 }).is_ok());
    let mv = [Change::Move { plane: 1, x: 5000, y: 10 }];
    assert_eq!(commit::check(&dev, &f.state, Commit { changes: &mv, flags: 0 }), Err(CommitError::Offscreen(1)));
}

#[test]
fn commit_refusals() {
    let (dev, st) = laptop();
    let base = commit::light(&dev, 0, 0, 0, 1920, 1080).unwrap();
    let go = |extra: &[Change]| {
        let mut c = base.clone(); c.extend_from_slice(extra);
        commit::check(&dev, &st, Commit { changes: &c, flags: ALLOW_MODESET })
    };
    assert_eq!(go(&[Change::Route { connector: 1, crtc: Some(0) }]), Err(CommitError::CrtcNotPossible { connector: 1, crtc: 0 }));
    assert_eq!(go(&[Change::Route { connector: 1, crtc: Some(1) }]), Err(CommitError::ConnectorDisconnected(1)));
    let odd = Mode::new(100_000, [1000, 1010, 1020, 1100], [700, 701, 702, 710], 0);
    assert_eq!(go(&[Change::Mode { crtc: 0, mode: Some(odd) }]), Err(CommitError::ModeRejected { connector: 0, crtc: 0 }));
    assert_eq!(go(&[Change::Plane { plane: 0, crtc: Some(0), fb: None, src: Rect::default(), dst: Rect::default() }]), Err(CommitError::NoPrimary(0)));
    assert_eq!(go(&[Change::Plane { plane: 1, crtc: Some(0), fb: Some(0), src: Rect::new(0, 0, 64, 64), dst: Rect::new(0, 0, 64, 64) }]), Err(CommitError::FormatUnsupported(1)));
    assert_eq!(go(&[Change::Plane { plane: 1, crtc: Some(0), fb: Some(2), src: Rect::new(0, 0, 65, 64), dst: Rect::new(0, 0, 65, 64) }]), Err(CommitError::BadSource(1)));
    assert_eq!(go(&[Change::Plane { plane: 1, crtc: Some(0), fb: Some(2), src: Rect::new(0, 0, 32, 32), dst: Rect::new(0, 0, 64, 64) }]), Err(CommitError::BadSize(1)));
    assert_eq!(go(&[Change::Plane { plane: 2, crtc: Some(0), fb: Some(0), src: Rect::new(0, 0, 1920, 1080), dst: Rect::new(0, 0, 960, 540) }]).map(|c| c.modeset), Ok(1));
    assert_eq!(go(&[Change::Plane { plane: 3, crtc: Some(1), fb: Some(1), src: Rect::new(0, 0, 10, 10), dst: Rect::new(0, 0, 10, 10) }]), Err(CommitError::PlaneOnInactiveCrtc(3)));
    assert_eq!(go(&[Change::Active { crtc: 2, on: true }]), Err(CommitError::CrtcIncomplete(2)));
    assert_eq!(go(&[Change::Flip { plane: 4, fb: 0 }]), Err(CommitError::PlaneDisabled(4)));
    assert_eq!(go(&[Change::Active { crtc: 9, on: true }]), Err(CommitError::NoSuchCrtc(9)));
    assert_eq!(go(&[Change::Plane { plane: 0, crtc: Some(0), fb: Some(7), src: Rect::new(0, 0, 1, 1), dst: Rect::new(0, 0, 1, 1) }]), Err(CommitError::NoSuchFramebuffer(7)));
}

#[test]
fn virtual_connector_takes_any_mode() {
    let mut c = Connector::new(ConnectorKind::Virtual, 1, 1);
    c.status = Status::Connected;
    assert!(c.accepts(&cvt_rb(1366, 768, 60).unwrap()));
    assert!(!c.accepts(&Mode::default()));
}

#[test]
fn frame_counter_wraps() {
    let mut f = FrameCounter::new(32);
    assert_eq!(f.update(u32::MAX - 1), 0);
    assert_eq!(f.update(1), 3);
    let mut g = FrameCounter::new(24);
    g.update(0xff_fffe); assert_eq!(g.update(0x00_0002), 4);
}

#[test]
fn desktop_layout() {
    let o = |connector, width, height, scale, internal| Output { connector, width, height, scale, x: 0, y: 0, internal };
    let d = Desktop::arrange(vec![o(1, 3840, 2160, 2, false), o(0, 1920, 1080, 1, true)]);
    assert_eq!(d.outputs[0].connector, 0);
    assert_eq!(d.outputs[1].x, 1920);
    assert_eq!(d.bounds(), Rect::new(0, 0, 3840, 1080));
    assert_eq!(d.output_at(2000, 10), Some(1));
    assert_eq!(d.output_at(10, 2000), None);
    let w = Rect::new(960, 540, 800, 600);
    let moved = d.move_to(&w, 1).unwrap();
    assert_eq!(moved, Rect::new(1920 + 960, 480, 800, 600));
    assert_eq!(d.output_of(&moved), Some(1));
}

#[test]
fn wire_round_trip() {
    let m = cta_vic(97).unwrap();
    assert_eq!(wire::unpack_mode(wire::pack_mode(&m)), m);
    let changes = [
        Change::Route { connector: 3, crtc: Some(1) }, Change::Route { connector: 0, crtc: None },
        Change::Active { crtc: 2, on: true }, Change::Mode { crtc: 1, mode: Some(m) }, Change::Mode { crtc: 0, mode: None },
        Change::Plane { plane: 5, crtc: Some(1), fb: Some(300), src: Rect::new(0, 0, 3840, 2160), dst: Rect::new(-20, -5, 64, 64) },
        Change::Plane { plane: 1, crtc: None, fb: None, src: Rect::default(), dst: Rect::default() },
        Change::Flip { plane: 2, fb: 9 }, Change::Move { plane: 1, x: -63, y: 2000 },
    ];
    for c in changes { assert_eq!(wire::unpack_change(wire::pack_change(&c)), Some(c)); }
}
