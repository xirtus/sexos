// SPDX-License-Identifier: GPL-3.0-only
//! Tests for descriptor parsing and Probe/Commit negotiation.

use sex_uvc::control::{VideoProbeCommitControl, UsbSetupPacket, GET_CUR, SET_CUR, VS_PROBE_CONTROL, VS_COMMIT_CONTROL};
use sex_uvc::descriptors::{parse_descriptors, FormatKind};
use sex_uvc::SyntheticUvcDevice;

#[test]
fn test_descriptors_parse_synthetic_device() {
    let dev = SyntheticUvcDevice::new(640, 480, 30);
    let binary = dev.binary_descriptors();

    let parsed = parse_descriptors(&binary).expect("Failed to parse synthetic descriptors");

    // IAD
    let iad = parsed.iad.expect("Missing IAD");
    assert_eq!(iad.first_interface, 0);
    assert_eq!(iad.interface_count, 2);

    // VideoControl
    assert_eq!(parsed.control.interface_number, 0);
    assert_eq!(parsed.control.camera_terminals.len(), 1);
    assert_eq!(parsed.control.processing_units.len(), 1);
    assert_eq!(parsed.control.output_terminals.len(), 1);

    // VideoStreaming
    assert_eq!(parsed.streaming.len(), 1);
    let stream = &parsed.streaming[0];
    assert_eq!(stream.interface_number, 1);
    assert_eq!(stream.endpoint_address, 0x81);
    assert_eq!(stream.formats.len(), 1);

    let fmt = &stream.formats[0];
    assert_eq!(fmt.kind, FormatKind::Yuy2);
    assert_eq!(fmt.bits_per_pixel, 16);
    assert_eq!(fmt.frames.len(), 1);

    let frame = &fmt.frames[0];
    assert_eq!(frame.width, 640);
    assert_eq!(frame.height, 480);
    assert_eq!(frame.default_fps(), 30);

    // Endpoint
    assert_eq!(stream.endpoints.len(), 1);
    let ep = &stream.endpoints[0];
    assert!(ep.is_in());
    assert!(ep.is_bulk());
    assert_eq!(ep.max_packet_size, 1024);
}

#[test]
fn test_probe_commit_negotiation() {
    let mut dev = SyntheticUvcDevice::new(320, 240, 15);
    let mut req = VideoProbeCommitControl::default();
    req.set_fps(15);

    let resp = dev.handle_probe(&req);
    assert_eq!(resp.b_format_index, 1);
    assert_eq!(resp.b_frame_index, 1);
    assert_eq!(resp.fps(), 15);
    assert_eq!(resp.dw_max_video_frame_size, 320 * 240 * 2);

    dev.handle_commit(&resp);
    assert_eq!(dev.committed, Some(resp));
}

#[test]
fn test_usb_setup_packet_helpers() {
    let probe_get = UsbSetupPacket::probe_get_cur(1, 26);
    let encoded = probe_get.encode();
    let decoded = UsbSetupPacket::decode(&encoded).expect("Decode failed");
    assert_eq!(decoded.b_request, GET_CUR);
    assert_eq!(decoded.w_value, VS_PROBE_CONTROL);
    assert_eq!(decoded.w_index, 1);
    assert_eq!(decoded.w_length, 26);

    let commit_set = UsbSetupPacket::commit_set_cur(1, 26);
    let encoded = commit_set.encode();
    let decoded = UsbSetupPacket::decode(&encoded).expect("Decode failed");
    assert_eq!(decoded.b_request, SET_CUR);
    assert_eq!(decoded.w_value, VS_COMMIT_CONTROL);
    assert_eq!(decoded.w_index, 1);
    assert_eq!(decoded.w_length, 26);
}
