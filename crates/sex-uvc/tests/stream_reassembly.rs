// SPDX-License-Identifier: GPL-3.0-only
//! Tests for multi-packet stream reassembly and SyntheticUvcDevice streaming.

use sex_uvc::stream::UvcStreamAssembler;
use sex_uvc::{SyntheticUvcDevice, WireMessage};

#[test]
fn test_synthetic_device_streaming_reassembly() {
    let mut dev = SyntheticUvcDevice::new(320, 240, 30);
    let mut asm = UvcStreamAssembler::new(2 * 1024 * 1024);

    let expected_frame_bytes = 320 * 240 * 2; // YUY2: 153,600 bytes

    for frame_idx in 0..5 {
        let packets = dev.next_frame_packets();
        assert!(packets.len() > 10, "Should be packetized into multiple chunks");

        let mut completed = None;
        for (pkt_idx, pkt) in packets.iter().enumerate() {
            let res = asm.push_packet(pkt).expect("Packet push failed");
            if pkt_idx == packets.len() - 1 {
                completed = res;
            } else {
                assert!(res.is_none(), "Premature frame completion at packet {}", pkt_idx);
            }
        }

        let frame = completed.expect("Frame should have completed on EOF packet");
        assert_eq!(frame.data.len(), expected_frame_bytes);
        assert_eq!(frame.fid, (frame_idx % 2) != 0);
        assert!(frame.pts.is_some());
    }

    assert_eq!(asm.frames_received, 5);
    assert_eq!(asm.frames_corrupted, 0);
}

#[test]
fn test_wire_message_roundtrip() {
    let dev = SyntheticUvcDevice::new(640, 480, 30);
    let desc = dev.binary_descriptors();

    let msg1 = WireMessage::Descriptors(desc.clone());
    let encoded1 = msg1.encode();
    let (decoded1, len1) = WireMessage::decode_one(&encoded1).unwrap().unwrap();
    assert_eq!(len1, encoded1.len());
    assert_eq!(decoded1, msg1);

    let mut ctrl = sex_uvc::control::VideoProbeCommitControl::default();
    ctrl.set_fps(60);
    let msg2 = WireMessage::ProbeReq(ctrl);
    let encoded2 = msg2.encode();
    let (decoded2, len2) = WireMessage::decode_one(&encoded2).unwrap().unwrap();
    assert_eq!(len2, encoded2.len());
    assert_eq!(decoded2, msg2);

    let msg3 = WireMessage::CommitAck(0);
    let encoded3 = msg3.encode();
    let (decoded3, len3) = WireMessage::decode_one(&encoded3).unwrap().unwrap();
    assert_eq!(len3, encoded3.len());
    assert_eq!(decoded3, msg3);

    let msg4 = WireMessage::StartStream;
    let encoded4 = msg4.encode();
    let (decoded4, len4) = WireMessage::decode_one(&encoded4).unwrap().unwrap();
    assert_eq!(len4, encoded4.len());
    assert_eq!(decoded4, msg4);
}
