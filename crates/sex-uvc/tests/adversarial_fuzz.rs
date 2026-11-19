// SPDX-License-Identifier: GPL-3.0-only
//! Adversarial fuzz and malformed packet tests for sex-uvc.

use sex_uvc::descriptors::parse_descriptors;
use sex_uvc::stream::{StreamError, UvcStreamAssembler};
use sex_uvc::SyntheticUvcDevice;

#[test]
fn test_descriptor_truncation_fuzz() {
    let dev = SyntheticUvcDevice::new(640, 480, 30);
    let valid_desc = dev.binary_descriptors();

    // Verify valid descriptors parse cleanly
    assert!(parse_descriptors(&valid_desc).is_ok());

    // Truncate at every single byte index from 0 to len - 1
    for i in 0..valid_desc.len() {
        let truncated = &valid_desc[..i];
        let res = parse_descriptors(truncated);
        // It must safely return an Err (never panic!)
        assert!(res.is_err(), "Truncated at index {} unexpectedly succeeded", i);
    }
}

#[test]
fn test_descriptor_corrupt_lengths() {
    let dev = SyntheticUvcDevice::new(320, 240, 15);
    let valid_desc = dev.binary_descriptors();

    let mut offset = 0;
    while offset < valid_desc.len() {
        let len = valid_desc[offset] as usize;

        let mut corrupt = valid_desc.clone();
        corrupt[offset] = 255; // Oversized length
        assert!(parse_descriptors(&corrupt).is_err());

        let mut corrupt_zero = valid_desc.clone();
        corrupt_zero[offset] = 0; // Illegal zero length
        assert!(parse_descriptors(&corrupt_zero).is_err());

        let mut corrupt_one = valid_desc.clone();
        corrupt_one[offset] = 1; // Illegal short length
        assert!(parse_descriptors(&corrupt_one).is_err());

        if len < 2 { break; }
        offset += len;
    }
}

#[test]
fn test_stream_assembler_header_fuzz() {
    let mut asm = UvcStreamAssembler::new(1024 * 1024);

    // Empty packet
    assert_eq!(asm.push_packet(&[]), Err(StreamError::PacketTooShort));

    // 1-byte packet
    assert_eq!(asm.push_packet(&[0x01]), Err(StreamError::PacketTooShort));

    // Header length 0 or 1
    assert_eq!(asm.push_packet(&[0x00, 0x00]), Err(StreamError::HeaderLengthInvalid));
    assert_eq!(asm.push_packet(&[0x01, 0x00]), Err(StreamError::HeaderLengthInvalid));

    // Header length greater than packet length
    assert_eq!(asm.push_packet(&[0x05, 0x00, 0x01]), Err(StreamError::HeaderLengthInvalid));

    // PTS flag set but header too short for PTS
    assert_eq!(asm.push_packet(&[0x04, 0x04, 0x01, 0x02]), Err(StreamError::HeaderLengthInvalid));

    // SCR flag set but header too short for SCR
    assert_eq!(asm.push_packet(&[0x06, 0x08, 0x01, 0x02, 0x03, 0x04]), Err(StreamError::HeaderLengthInvalid));
}

#[test]
fn test_stream_assembler_quota_exhaustion() {
    // 500 byte limit
    let mut asm = UvcStreamAssembler::new(500);

    // Send packet of 400 bytes payload
    let mut pkt1 = vec![0u8; 402];
    pkt1[0] = 2; // header len
    pkt1[1] = 0; // FID = 0, EOF = 0
    assert!(asm.push_packet(&pkt1).unwrap().is_none());

    // Send another 400 bytes payload (total 800 > 500)
    let mut pkt2 = vec![0u8; 402];
    pkt2[0] = 2;
    pkt2[1] = 0;
    assert_eq!(asm.push_packet(&pkt2), Err(StreamError::FrameTooLarge));
    assert_eq!(asm.frames_corrupted, 1);
}

#[test]
fn test_stream_assembler_err_bit_drops_frame() {
    let mut asm = UvcStreamAssembler::new(1024 * 1024);

    // Send valid first packet
    let mut pkt1 = vec![0u8; 100];
    pkt1[0] = 2;
    pkt1[1] = 0;
    assert!(asm.push_packet(&pkt1).unwrap().is_none());

    // Send second packet with ERR bit (0x40)
    let mut pkt2 = vec![0u8; 100];
    pkt2[0] = 2;
    pkt2[1] = 0x40;
    assert_eq!(asm.push_packet(&pkt2), Err(StreamError::CorruptFrame));
    assert_eq!(asm.frames_corrupted, 1);

    // New valid frame starts
    let mut pkt3 = vec![0u8; 100];
    pkt3[0] = 2;
    pkt3[1] = 0x03; // FID = 1, EOF = 1
    let completed = asm.push_packet(&pkt3).unwrap();
    assert!(completed.is_some());
    assert_eq!(completed.unwrap().data.len(), 98);
    assert_eq!(asm.frames_received, 1);
}
