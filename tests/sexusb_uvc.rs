// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for UVC payload header stripping, frame reassembly, and
//! `sexusb <-> sexuvc` shared-ring handoff (`crates/sex-pdx/src/usb_uvc.rs` +
//! `servers/sexusb/src/uvc_stream.rs`).

#[path = "../servers/sexusb/src/uvc_stream.rs"]
mod uvc_stream;

#[path = "../crates/sex-pdx/src/usb_uvc.rs"]
mod usb_uvc;

use uvc_stream::*;

fn make_packet(header_len: u8, info: u8, payload: &[u8]) -> Vec<u8> {
    let mut pkt = vec![0u8; header_len as usize + payload.len()];
    pkt[0] = header_len;
    pkt[1] = info;
    pkt[header_len as usize..].copy_from_slice(payload);
    pkt
}

#[test]
fn payload_header_stripping_2_to_12_bytes() {
    // 2-byte minimal header
    let p2 = make_packet(2, HDR_FID | HDR_EOF, &[0xFF, 0xD8, 0xFF, 0xD9]);
    let (h2, body2) = parse_header(&p2).expect("2-byte header");
    assert_eq!((h2.len, h2.fid, h2.eof, h2.err, h2.pts, h2.scr_stc), (2, true, true, false, None, None));
    assert_eq!(body2, &[0xFF, 0xD8, 0xFF, 0xD9]);

    // 12-byte PTS + SCR header (typical UVC 1.1 webcam like 0c45:6d18)
    let mut p12 = vec![12, HDR_PTS | HDR_SCR | HDR_FID, 0x78, 0x56, 0x34, 0x12, 0xEF, 0xBE, 0xAD, 0xDE, 0x34, 0x12];
    p12.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
    let (h12, body12) = parse_header(&p12).expect("12-byte header");
    assert_eq!(h12.len, 12);
    assert_eq!(h12.pts, Some(0x1234_5678));
    assert_eq!(h12.scr_stc, Some(0xDEAD_BEEF));
    assert_eq!(h12.scr_sof, Some(0x1234));
    assert_eq!(body12, &[0xAA, 0xBB, 0xCC]);

    // Invalid / truncated headers rejected
    assert!(parse_header(&[]).is_none());
    assert!(parse_header(&[1]).is_none());
    assert!(parse_header(&[1, 0]).is_none());
    assert!(parse_header(&[12, HDR_PTS, 1, 2, 3]).is_none());
    assert!(parse_header(&[4, HDR_PTS, 1, 2]).is_none());
    assert!(parse_header(&[6, HDR_SCR, 1, 2, 3, 4]).is_none());
}

#[test]
fn reassembly_eof_and_fid_toggle_and_multichunk() {
    let mut reasm = UvcReassembler::new();
    let mut frames: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut cur = Vec::new();
    let mut cur_seq = 0u32;

    let mut feed_pkt = |r: &mut UvcReassembler, pkt: &[u8]| {
        r.feed(pkt, |flags, seq, data| {
            if (flags & FLAG_SOF) != 0 {
                cur.clear();
                cur_seq = seq;
            }
            cur.extend_from_slice(data);
            if (flags & FLAG_EOF) != 0 {
                frames.push((cur_seq, core::mem::take(&mut cur)));
            }
            true
        });
    };

    // Frame 1: multi-packet (> MAX_CHUNK = 4088) terminated by EOF
    let big_a = vec![0x11u8; 3000];
    let big_b = vec![0x22u8; 2500];
    feed_pkt(&mut reasm, &make_packet(12, 0, &big_a));
    feed_pkt(&mut reasm, &make_packet(12, HDR_EOF, &big_b));
    drop(feed_pkt);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1.len(), 5500);
    assert!(frames[0].1[..3000].iter().all(|&b| b == 0x11));
    assert!(frames[0].1[3000..].iter().all(|&b| b == 0x22));

    // Frame 2: NO EOF bit, closed implicitly when Frame 3 toggles FID (0 -> 1)
    let mut feed2 = |r: &mut UvcReassembler, pkt: &[u8]| {
        r.feed(pkt, |flags, seq, data| {
            if (flags & FLAG_SOF) != 0 {
                cur.clear();
                cur_seq = seq;
            }
            cur.extend_from_slice(data);
            if (flags & FLAG_EOF) != 0 {
                frames.push((cur_seq, core::mem::take(&mut cur)));
            }
            true
        });
    };
    feed2(&mut reasm, &make_packet(2, 0, b"FRAME2_A"));
    feed2(&mut reasm, &make_packet(2, 0, b"_B"));
    drop(feed2);
    assert_eq!(frames.len(), 1);
    reasm.feed(&make_packet(2, HDR_FID | HDR_EOF, b"FRAME3"), |flags, seq, data| {
        if (flags & FLAG_SOF) != 0 {
            cur.clear();
            cur_seq = seq;
        }
        cur.extend_from_slice(data);
        if (flags & FLAG_EOF) != 0 {
            frames.push((cur_seq, core::mem::take(&mut cur)));
        }
        true
    });
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[1].1, b"FRAME2_A_B");
    assert_eq!(frames[2].1, b"FRAME3");
    assert_eq!(reasm.frames_completed, 3);
    assert_eq!(reasm.frames_dropped, 0);
}

#[test]
fn error_bit_drops_corrupted_frame_and_recovers() {
    let mut reasm = UvcReassembler::new();
    let mut events: Vec<(u16, u32, usize)> = Vec::new();

    // Start a large frame so one chunk is emitted, then hit ERR bit
    reasm.feed(&make_packet(2, 0, &vec![0x55u8; MAX_CHUNK]), |f, s, d| { events.push((f, s, d.len())); true });
    reasm.feed(&make_packet(2, 0, &[0x66; 16]), |f, s, d| { events.push((f, s, d.len())); true });
    reasm.feed(&make_packet(2, HDR_ERR | HDR_EOF, &[0x77; 8]), |f, s, d| { events.push((f, s, d.len())); true });
    assert_eq!(reasm.frames_dropped, 1);
    assert_eq!(reasm.frames_completed, 0);
    assert!(events.iter().any(|(f, _, _)| (*f & FLAG_ERR) != 0));

    // Next frame with toggled FID succeeds cleanly
    events.clear();
    reasm.feed(&make_packet(2, HDR_FID | HDR_EOF, b"GOOD"), |f, s, d| { events.push((f, s, d.len())); true });
    assert_eq!(reasm.frames_completed, 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0 & (FLAG_SOF | FLAG_EOF | FLAG_ERR), FLAG_SOF | FLAG_EOF);
    assert_eq!(events[0].2, 4);
}

#[test]
fn probe_commit_wire_format_and_shared_ring_handoff() {
    let probe = build_probe_commit(1, 1, 30);
    assert_eq!(probe[2], 1);
    assert_eq!(probe[3], 1);
    assert_eq!(u32::from_le_bytes([probe[4], probe[5], probe[6], probe[7]]), 333_333);
    let mut reply = probe;
    reply[18..22].copy_from_slice(&(1280u32 * 720 * 2).to_le_bytes());
    reply[22..26].copy_from_slice(&3072u32.to_le_bytes());
    assert_eq!(parse_probe_response(&reply), Some((1280 * 720 * 2, 3072)));

    // End-to-end SexUSB -> Shared -> SexUVC handoff for Alienware x17 R1 0c45:6d18
    let mut mem = vec![0u32; usb_uvc::BYTES as usize / 4];
    let shared = unsafe { usb_uvc::Shared::at(mem.as_mut_ptr() as *mut u8) };

    let mut cam = usb_uvc::CameraInfo {
        vendor: 0x0c45,
        product: 0x6d18,
        generation: 0,
        control_iface: 0,
        stream_iface: 1,
        alt_count: 6,
        bulk: false,
        max_payload: 3072,
        format_count: 2,
        formats: [usb_uvc::FormatMeta::default(); usb_uvc::MAX_FORMATS],
    };
    cam.formats[0] = usb_uvc::FormatMeta { index: 1, kind: usb_uvc::FMT_MJPEG, frames: 5, max_w: 1280, max_h: 720 };
    cam.formats[1] = usb_uvc::FormatMeta { index: 2, kind: usb_uvc::FMT_YUY2, frames: 4, max_w: 1280, max_h: 720 };
    shared.set_camera(Some(&cam));

    // SexUVC sees camera and requests start
    let seen = shared.camera().expect("sexuvc sees camera");
    assert_eq!((seen.vendor, seen.product, seen.max_dimensions()), (0x0c45, 0x6d18, (1280, 720)));
    let seq = shared.request_start(1, 1, 30, 1280, 720);

    // SexUSB polls command, acks streaming, feeds 2 frames via UvcReassembler
    let cmd = shared.poll_command().expect("sexusb polls CMD_START");
    assert_eq!((cmd.seq, cmd.op), (seq, usb_uvc::CMD_START));
    shared.ack_command(cmd.seq, usb_uvc::STATE_STREAMING, 6, 0x81, 3072);

    let mut reasm = UvcReassembler::new();
    let f1 = [0xFFu8, 0xD8, 1, 2, 3, 4, 0xFF, 0xD9];
    let f2 = [0xFFu8, 0xD8, 9, 8, 7, 6, 0xFF, 0xD9];
    reasm.feed(&make_packet(12, 0, &f1[..4]), |fl, sq, d| shared.push_chunk(fl, sq, d));
    reasm.feed(&make_packet(12, HDR_EOF, &f1[4..]), |fl, sq, d| shared.push_chunk(fl, sq, d));
    reasm.feed(&make_packet(12, HDR_FID | HDR_EOF, &f2), |fl, sq, d| shared.push_chunk(fl, sq, d));
    assert_eq!(shared.frames_done(), 2);

    // SexUVC pops both frames cleanly
    let mut out = [0u8; usb_uvc::MAX_CHUNK];
    let c1 = shared.pop_chunk(&mut out).unwrap();
    assert_eq!((c1.flags & (usb_uvc::FLAG_SOF | usb_uvc::FLAG_EOF), &out[..c1.len]),
               (usb_uvc::FLAG_SOF | usb_uvc::FLAG_EOF, &f1[..]));
    let c2 = shared.pop_chunk(&mut out).unwrap();
    assert_eq!((c2.flags & (usb_uvc::FLAG_SOF | usb_uvc::FLAG_EOF), &out[..c2.len]),
               (usb_uvc::FLAG_SOF | usb_uvc::FLAG_EOF, &f2[..]));
    assert!(shared.pop_chunk(&mut out).is_none());
}
