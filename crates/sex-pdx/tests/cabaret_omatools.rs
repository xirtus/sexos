// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Protocol tests for Broadcast Studio Capture v2, Per-PD Audio Tap / Private Sink,
//! and Collar FileChooser Portal: `cargo test -p sex-pdx --test cabaret_omatools`.

use sex_pdx::desktop_recording::{
    CapturePixelFmt, CaptureSourceTarget, SessionSequence, StudioSessionSpec, StudioSpecError,
    WindowCaptureFlagsReq, WindowCapturePolicy, MAX_STUDIO_FPS, MAX_STUDIO_HEIGHT,
    MAX_STUDIO_WIDTH, OP_STUDIO_CAPTURE, OP_STUDIO_WINDOW_FLAGS,
};
use sex_pdx::file_chooser::{
    ChooserMode, ChooserReply, ChooserRequest, ChooserStatus, CollarGrantToken, FileFilter,
    OP_FILE_CHOOSER, OP_FILE_CHOOSER_CANCEL, OP_FILE_CHOOSER_REPLY,
};
use sex_pdx::sound_mixer::{
    decode_private_sink_reply, is_reply, private_sink_reply, AppTapAction, AppTapReply,
    AppTapRequest, AppTapState, PrivateSinkRoute, PrivateSinkSpec, OP_MIXER_APP_TAP,
    OP_MIXER_PRIVATE_SINK, REFUSED,
};

#[test]
fn studio_capture_v2_enforces_tally_and_geometry_bounds() {
    assert_eq!(OP_STUDIO_CAPTURE, 0x2e7);
    assert_eq!(OP_STUDIO_WINDOW_FLAGS, 0x2e8);
    assert_eq!(MAX_STUDIO_WIDTH, 3840);
    assert_eq!(MAX_STUDIO_HEIGHT, 2160);
    assert_eq!(MAX_STUDIO_FPS, 120);

    let seq = SessionSequence::new(2, 42).unwrap();
    let region = CaptureSourceTarget::Region {
        x: 100,
        y: 200,
        w: 1920,
        h: 1080,
    };

    // Without SilkBar red tally active, frame rings cannot arm.
    let unlit = StudioSessionSpec::new(
        seq,
        region,
        3840,
        2160,
        120,
        CapturePixelFmt::P010,
        false,
        true,
    );
    assert!(!unlit.can_arm_rings());
    assert_eq!(unlit.arm(), Err(StudioSpecError::TallyInactive));

    // With tally active, arming succeeds and wire round-trips cleanly.
    let lit = StudioSessionSpec::new(
        seq,
        region,
        3840,
        2160,
        120,
        CapturePixelFmt::P010,
        true,
        true,
    );
    assert!(lit.can_arm_rings());
    let armed = lit.arm().unwrap();
    let args = armed.args();
    assert_eq!(StudioSessionSpec::decode_armed(args), Some(armed));

    // Tampering tally bit off on the wire is rejected.
    let mut forged = args;
    forged[2] &= !(1 << 8);
    assert!(StudioSessionSpec::decode_armed(forged).is_none());

    // Subsampled 4:2:0 formats reject odd dimensions; bounds reject >4K or >120fps.
    let odd_nv12 = StudioSessionSpec::new(
        seq,
        CaptureSourceTarget::FollowFocusedWindow,
        1921,
        1080,
        60,
        CapturePixelFmt::Nv12,
        true,
        false,
    );
    assert_eq!(
        odd_nv12.arm(),
        Err(StudioSpecError::UnalignedSubsampledDimensions)
    );
    assert!(CapturePixelFmt::Xrgb8888.frame_bytes(3840, 2160).is_some());
    assert!(CapturePixelFmt::Nv12.frame_bytes(1921, 1080).is_none());
}

#[test]
fn window_capture_policy_hide_from_capture_wire_round_trip() {
    let policy = WindowCapturePolicy::sensitive_overlay() | WindowCapturePolicy::OMIT_DECORATIONS;
    assert!(policy.is_hidden_from_capture());
    assert!(policy.contains(WindowCapturePolicy::HIDE_FROM_CAPTURE));

    let req = WindowCaptureFlagsReq::new(305, policy).unwrap();
    let args = req.args();
    assert_eq!(WindowCaptureFlagsReq::decode(args), Some(req));

    // Unknown flag bits or zero frame_id are rejected.
    assert!(WindowCapturePolicy::from_bits(1 << 30).is_none());
    assert!(WindowCaptureFlagsReq::new(0, policy).is_none());
    assert!(WindowCaptureFlagsReq::decode([305, 1 << 30, 0]).is_none());
}

#[test]
fn app_tap_consent_and_private_browser_sink_wire_round_trip() {
    assert_eq!(OP_MIXER_APP_TAP, 0xCD);
    assert_eq!(OP_MIXER_PRIVATE_SINK, 0xCE);

    let tap_req = AppTapRequest::new(AppTapAction::RequestStart, 15, 7, 42).unwrap();
    assert_eq!(AppTapRequest::decode(tap_req.args()), Some(tap_req));
    assert!(AppTapRequest::decode([tap_req.args()[0], tap_req.args()[1], 1]).is_none());

    for state in [
        AppTapState::Off,
        AppTapState::WaitingConsent,
        AppTapState::Active,
    ] {
        let reply = AppTapReply::new(state, 15, 7).unwrap();
        let word = reply.word();
        assert!(is_reply(word));
        assert_ne!(word, REFUSED);
        assert_eq!(AppTapReply::decode(word), Some(reply));
    }

    let sink = PrivateSinkSpec::new(3, 15, 42, PrivateSinkRoute::CaptureOnly, 85, false).unwrap();
    assert_eq!(PrivateSinkSpec::decode(sink.args()), Some(sink));
    assert!(PrivateSinkSpec::new(3, 15, 42, PrivateSinkRoute::CaptureOnly, 101, false).is_none());

    let reply_word = private_sink_reply(3, PrivateSinkRoute::CaptureOnly, true).unwrap();
    assert!(is_reply(reply_word));
    assert!(AppTapReply::decode(reply_word).is_none());
    assert_eq!(
        decode_private_sink_reply(reply_word),
        Some((3, PrivateSinkRoute::CaptureOnly, true))
    );
}

#[test]
fn collar_file_chooser_portal_round_trip_and_grant_validation() {
    assert_eq!(OP_FILE_CHOOSER, 0x2F3);
    assert_eq!(OP_FILE_CHOOSER_REPLY, 0x2F4);
    assert_eq!(OP_FILE_CHOOSER_CANCEL, 0x2F5);

    let filter = FileFilter::new(b"Video", &[b".MKV", b"mp4"]).unwrap();
    assert!(filter.matches_name(b"/recordings/take1.mkv"));
    assert!(filter.matches_name(b"CLIP.MP4"));
    assert!(!filter.matches_name(b"notes.txt"));

    let req = ChooserRequest::new(19, 41, ChooserMode::OpenMultiple)
        .unwrap()
        .with_title(b"Import Media")
        .unwrap()
        .with_suggested_name(b"take.mkv")
        .unwrap()
        .with_filter(filter)
        .unwrap();

    let mut req_buf = [0u8; 512];
    let n = req.encode_bytes(&mut req_buf).unwrap();
    assert_eq!(ChooserRequest::decode_bytes(&req_buf[..n]), Some(req));
    assert!(ChooserRequest::decode_open(req.wire_open()).is_some());

    // Root ambient grant and traversal tokens must always be rejected.
    assert!(CollarGrantToken::new(b"@/", 1, false, true).is_none());
    assert!(CollarGrantToken::new(b"@/home/../etc/shadow", 1, false, false).is_none());
    assert!(CollarGrantToken::new(b"/home/user/a.mkv", 1, false, false).is_none());

    let tok1 = CollarGrantToken::new(b"@/home/user/a.mkv", 1001, false, false).unwrap();
    let tok2 = CollarGrantToken::new(b"@/home/user/b.mp4", 1002, false, false).unwrap();
    let reply = ChooserReply::granted(&req, 5, &[tok1, tok2]).unwrap();

    let mut rep_buf = [0u8; 1024];
    let rn = reply.encode_bytes(&mut rep_buf).unwrap();
    let decoded = ChooserReply::decode_bytes(&rep_buf[..rn]).unwrap();
    assert_eq!(decoded, reply);
    assert_eq!(decoded.grants()[0].as_bytes(), b"@/home/user/a.mkv");
    assert_eq!(decoded.grants()[1].path_bytes(), b"/home/user/b.mp4");
    assert_eq!(
        ChooserReply::decode_summary(reply.wire_summary()),
        Some((19, ChooserMode::OpenMultiple, ChooserStatus::Granted, 2, 41, 5))
    );

    // Read-only open request rejects writable grant token.
    let bad_writable = CollarGrantToken::new(b"@/home/user/a.mkv", 1003, true, false).unwrap();
    assert!(ChooserReply::granted(&req, 5, &[bad_writable]).is_none());

    // Single-file request rejects multiple grants; PickFolder requires is_dir=true.
    let single_req = ChooserRequest::new(20, 41, ChooserMode::OpenFile).unwrap();
    assert!(ChooserReply::granted(&single_req, 5, &[tok1, tok2]).is_none());
    let folder_req = ChooserRequest::new(21, 41, ChooserMode::PickFolder).unwrap();
    assert!(ChooserReply::granted(&folder_req, 5, &[tok1]).is_none());
    let dir_tok = CollarGrantToken::new(b"@/home/user/clips", 1004, false, true).unwrap();
    assert!(ChooserReply::granted(&folder_req, 5, &[dir_tok]).is_some());
}
