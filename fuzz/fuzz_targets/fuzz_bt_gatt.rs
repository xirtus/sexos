// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_bt::controller::Bringup;
use sex_bt::hci::{self, ev, op, Addr};
use sex_bt::host::{Host, Note};
use sex_bt::intel::{self, Loader, Version};
use sex_bt::l2cap::{self, Reassembly};
use sex_bt::pack;
use sex_bt::report;
use sex_bt::smp::{self, Role, Smp};
use sex_bt::ssp::Ssp;

/// `sex_pdx::bt_hci::MAX_PACKET` (`SLOT_BYTES - 4` = 2044).
const BT_HCI_MAX_PACKET: usize = 2044;

fn deterministic_rng(seed: u8) -> Box<dyn FnMut(&mut [u8])> {
    let mut state = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    Box::new(move |out: &mut [u8]| {
        for b in out.iter_mut() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = (state & 0xFF) as u8;
        }
    })
}

/// Complete controller bring-up on `host` with caller-chosen ACL / LE ACL MTUs so
/// post-bring-up ACL, L2CAP signaling, ATT (`CID_ATT`), SMP (`CID_SMP`), and SSP
/// handlers are directly reachable.
fn drive_bringup(host: &mut Host, acl_mtu: u16, le_acl_mtu: u16) {
    for _ in 0..32 {
        if host.ready() {
            break;
        }
        let Some(pkt) = host.poll() else { break };
        let Some((opcode, _params)) = hci::parse_command(&pkt) else { break };
        let reply = match opcode {
            op::RESET
            | op::SET_EVENT_MASK
            | op::WRITE_SIMPLE_PAIRING_MODE
            | op::WRITE_LE_HOST_SUPPORTED
            | op::WRITE_SCAN_ENABLE
            | op::LE_SET_EVENT_MASK => hci::command_complete(opcode, &[0]),
            op::READ_LOCAL_VERSION => {
                hci::command_complete(opcode, &[0, 0x0C, 0x00, 0x10, 0x0C, 0x02, 0x00, 0x34, 0x12])
            }
            op::READ_LOCAL_FEATURES => {
                // Byte 4 bit 6 = LE Supported (Controller); Byte 6 bit 3 = SSP (Controller)
                hci::command_complete(opcode, &[0, 0xFF, 0xFF, 0x8F, 0xFE, 0xDB, 0xFF, 0x5B, 0x87])
            }
            op::READ_BD_ADDR => {
                hci::command_complete(opcode, &[0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66])
            }
            op::READ_BUFFER_SIZE => {
                let mut r = vec![0u8];
                r.extend_from_slice(&acl_mtu.to_le_bytes());
                r.push(64);
                r.extend_from_slice(&8u16.to_le_bytes());
                r.extend_from_slice(&0u16.to_le_bytes());
                hci::command_complete(opcode, &r)
            }
            op::LE_READ_BUFFER_SIZE => {
                let mut r = vec![0u8];
                r.extend_from_slice(&le_acl_mtu.to_le_bytes());
                r.push(8);
                hci::command_complete(opcode, &r)
            }
            op::LE_READ_LOCAL_FEATURES => {
                // Bit 12 (byte 1 bit 4) = LE Extended Advertising
                hci::command_complete(opcode, &[0, 0xFF, 0x1F, 0, 0, 0, 0, 0, 0])
            }
            _ => hci::command_complete(opcode, &[0]),
        };
        host.on_packet(&reply);
    }
    while host.note().is_some() {}
}

fn drain_host(host: &mut Host, strict: bool) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(pkt) = host.poll() {
        if strict {
            assert!(
                pkt.len() <= BT_HCI_MAX_PACKET,
                "Host::poll emitted {} byte packet (> bt_hci::MAX_PACKET {}), causing infinite livelock in cufflinks::send",
                pkt.len(),
                BT_HCI_MAX_PACKET
            );
        }
        out.push(pkt);
    }
    while let Some(note) = host.note() {
        if let Note::Compare { handle, value, .. } = note {
            host.confirm(handle, (value & 1) == 0);
        }
    }
    out
}

fn exercise_unit_parsers(data: &[u8]) {
    let _ = hci::parse_command(data);
    let _ = hci::parse_acl(data);
    if let Some(ev_parsed) = hci::parse_event(data) {
        let mut b = Bringup::new();
        let _ = b.on_event(&ev_parsed);

        let mut ssp = Ssp::new(smp::io::DISPLAY_YES_NO, 0x03);
        let _ = ssp.on_event(&ev_parsed);

        let mut loader = Loader::new();
        let _ = loader.next();
        let _ = loader.on_event(&ev_parsed);
    }

    for (ad_type, ad_body) in report::structures(data).take(64) {
        if ad_body.len() < 255 {
            let _ = report::encode(&[(ad_type, ad_body)]);
        }
    }

    if data.len() >= 2 {
        let _ = report::reports(data[0], data[1], &data[2..]);
    }

    if let Some((cid, payload)) = l2cap::parse_frame(data) {
        let _ = l2cap::signal(cid, payload);
    }
    let _ = l2cap::signal(l2cap::CID_SIGNALING, data);
    let _ = l2cap::signal(l2cap::CID_LE_SIGNALING, data);

    // Fragmented L2CAP reassembly across variable-sized ACL chunks.
    let mut rx = Reassembly::default();
    let mut pos = 0usize;
    while pos < data.len() {
        let hdr = data[pos];
        let pb = hdr & 0x03;
        let step = ((hdr >> 2) as usize + 1).min(data.len() - pos);
        let chunk = &data[pos + 1..pos + step];
        if let Some(frame) = rx.push(pb, chunk) {
            if let Some((cid, payload)) = l2cap::parse_frame(&frame) {
                let _ = l2cap::signal(cid, payload);
            }
        }
        pos += step;
    }

    // Intel version, firmware `.sfi`, `.ddc`, bootloader bulk-event, and pack lookup.
    if let Ok(ver) = Version::parse(data) {
        let _ = ver.hw_variant();
        let _ = ver.bootloader();
        let _ = ver.firmware_base();
        let _ = intel::secure_send_plan(data, &ver);
    }
    let legacy_boot = Version::Legacy {
        hw_platform: 0x37,
        hw_variant: 0x12,
        hw_revision: 1,
        fw_variant: 0x06,
        fw_revision: 1,
        fw_build_num: 1,
        fw_build_ww: 1,
        fw_build_yy: 24,
        fw_patch_num: 0,
    };
    let tlv_boot = Version::Tlv {
        cnvi_top: 0x17,
        cnvr_top: 0x01,
        cnvi_bt: 0x17,
        image_type: 0x01,
        sbe_type: data.first().copied().unwrap_or(0) & 1,
        build_num: 0,
    };
    let _ = intel::secure_send_plan(data, &legacy_boot);
    let _ = intel::secure_send_plan(data, &tlv_boot);
    let _ = intel::ddc_commands(data);
    let _ = intel::bootloader_event(data);
    let _ = pack::find(data, "ibt-18-16-1.sfi");
    if let Ok(s) = core::str::from_utf8(&data[..data.len().min(48)]) {
        let _ = pack::find(data, s);
    }

    // Direct SMP Initiator & Responder PDU sequences.
    let mut secret = [0x42u8; 32];
    for (i, b) in data.iter().take(32).enumerate() {
        secret[i] ^= *b;
    }
    secret[0] |= 1;
    secret[31] &= 0x7F;
    let nonce = [0xA5u8; 16];
    let local = [1, 2, 3, 4, 5, 6, 0];
    let remote = [9, 8, 7, 6, 5, 4, 1];
    for role in [Role::Initiator, Role::Responder] {
        if let Some(mut smp_sm) = Smp::new(
            role,
            smp::io::DISPLAY_YES_NO,
            smp::AUTH_BONDING | smp::AUTH_MITM,
            local,
            remote,
            &secret,
            nonce,
        ) {
            if role == Role::Initiator {
                let _ = smp_sm.start();
            }
            for pdu in data.split(|&b| b == 0xFF).take(16) {
                if !pdu.is_empty() {
                    let actions = smp_sm.on_pdu(pdu);
                    for a in actions {
                        if let smp::Action::Compare(v) = a {
                            let _ = smp_sm.confirm((v & 1) == 0);
                        }
                    }
                }
            }
        }
    }
}

fn exercise_host_session(data: &[u8], strict: bool) {
    if data.len() < 4 {
        return;
    }
    let acl_mtu = u16::from_le_bytes([data[0], data[1]]).max(27);
    let le_acl_mtu = u16::from_le_bytes([data[2], data[3]]).max(23);
    let stream = &data[4..];

    let mut host = Host::new(deterministic_rng(data[0]));
    drive_bringup(&mut host, acl_mtu, le_acl_mtu);

    // Open three connections:
    // - 0x0040: LE Central (role = 0)
    // - 0x0041: LE Peripheral (role = 1)
    // - 0x0042: Classic BR/EDR
    let le_central = hci::event(
        ev::LE_META,
        &[
            ev::LE_CONNECTION_COMPLETE,
            0x00,
            0x40,
            0x00,
            0x00, // central
            0x01, // random peer addr
            0xAA,
            0xBB,
            0xCC,
            0xDD,
            0xEE,
            0x01,
            0x18,
            0x00,
            0x00,
            0x00,
            0xF4,
            0x01,
            0x00,
        ],
    );
    let le_periph = hci::event(
        ev::LE_META,
        &[
            ev::LE_CONNECTION_COMPLETE,
            0x00,
            0x41,
            0x00,
            0x01, // peripheral
            0x00, // public peer addr
            0xAA,
            0xBB,
            0xCC,
            0xDD,
            0xEE,
            0x02,
            0x18,
            0x00,
            0x00,
            0x00,
            0xF4,
            0x01,
            0x00,
        ],
    );
    let classic_conn = hci::event(
        ev::CONNECTION_COMPLETE,
        &[0x00, 0x42, 0x00, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x03, 0x01, 0x00],
    );
    host.on_packet(&le_central);
    host.on_packet(&le_periph);
    host.on_packet(&classic_conn);
    let _ = drain_host(&mut host, strict);

    let handles = [0x0040u16, 0x0041u16, 0x0042u16];
    let cids = [
        l2cap::CID_ATT,
        l2cap::CID_SMP,
        l2cap::CID_SIGNALING,
        l2cap::CID_LE_SIGNALING,
        l2cap::CID_SMP_BREDR,
        0x0040,
    ];

    let mut idx = 0usize;
    let mut steps = 0usize;
    while idx < stream.len() && steps < 64 {
        steps += 1;
        let tag = stream[idx];
        idx += 1;
        if idx >= stream.len() {
            break;
        }
        let len = (stream[idx] as usize).min(stream.len() - (idx + 1));
        idx += 1;
        let body = &stream[idx..idx + len];
        idx += len;

        match tag & 0x07 {
            // Structured L2CAP frame over chosen connection & fixed CID (ATT / SMP / Signaling)
            0..=3 => {
                let handle = handles[(tag as usize >> 3) % handles.len()];
                let cid = cids[(tag as usize >> 5) % cids.len()];
                let frag_mtu = ((tag as usize >> 4) & 0x07) * 12 + 7;
                let frame = l2cap::frame(cid, body);
                for pkt in l2cap::fragment(handle, &frame, frag_mtu) {
                    host.on_packet(&pkt);
                }
                let emitted = drain_host(&mut host, strict);
                if strict && cid == l2cap::CID_LE_SIGNALING {
                    // Check whether Host sent CONNECTION_PARAMETER_UPDATE_RESPONSE (0x13) with Accepted (0x0000)
                    let mut accepted_update = false;
                    for pkt in &emitted {
                        if let Some((_h, _pb, acl_data)) = hci::parse_acl(pkt) {
                            if let Some((l2cap::CID_LE_SIGNALING, sig_pay)) = l2cap::parse_frame(acl_data) {
                                if sig_pay.len() >= 6
                                    && sig_pay[0] == l2cap::sig::CONNECTION_PARAMETER_UPDATE_RESPONSE
                                    && sig_pay[4..6] == [0x00, 0x00]
                                {
                                    accepted_update = true;
                                }
                            }
                        }
                    }
                    if accepted_update {
                        let sent_hci_update = emitted.iter().any(|pkt| {
                            hci::parse_command(pkt).is_some_and(|(opc, _)| opc == 0x2013)
                        });
                        assert!(
                            handle == 0x0040 && sent_hci_update,
                            "Host accepted L2CAP LE Connection Parameter Update on handle {:#06x} without issuing HCI_LE_Connection_Update",
                            handle
                        );
                    }
                }
            }
            // Large L2CAP Echo Request (multi-fragment ACL -> single reassembled L2CAP echo)
            4 => {
                let handle = handles[(tag as usize >> 3) % handles.len()];
                let repeat = ((tag >> 4) as usize + 1) * 16;
                let mut echo_data = Vec::with_capacity(body.len().saturating_mul(repeat).min(4096));
                for _ in 0..repeat {
                    if echo_data.len() + body.len() > 4096 {
                        break;
                    }
                    echo_data.extend_from_slice(body);
                }
                let mut sig_payload = vec![l2cap::sig::ECHO_REQUEST, 0x01];
                sig_payload.extend_from_slice(&(echo_data.len() as u16).to_le_bytes());
                sig_payload.extend_from_slice(&echo_data);
                let frame = l2cap::frame(l2cap::CID_SIGNALING, &sig_payload);
                for pkt in l2cap::fragment(handle, &frame, 251) {
                    host.on_packet(&pkt);
                }
                let _ = drain_host(&mut host, strict);
            }
            // Raw HCI packet (event or ACL) directly from controller
            5 | 6 => {
                host.on_packet(body);
                let _ = drain_host(&mut host, strict);
            }
            // Host-initiated actions (scan, connect_le, pair)
            _ => {
                let handle = handles[(tag as usize >> 3) % handles.len()];
                if tag & 0x10 != 0 {
                    host.scan(4);
                } else if tag & 0x20 != 0 {
                    host.connect_le(Addr([1, 2, 3, 4, 5, 6]), tag & 1);
                } else {
                    host.pair(handle);
                }
                // Feed CommandComplete for any queued command so credits stay unlocked
                let emitted = drain_host(&mut host, strict);
                for pkt in emitted {
                    if let Some((opc, _)) = hci::parse_command(&pkt) {
                        host.on_packet(&hci::command_complete(opc, &[0]));
                    }
                }
                let _ = drain_host(&mut host, strict);
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let strict = std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some();

    exercise_unit_parsers(data);
    exercise_host_session(data, strict);

    // Also feed raw packet stream into a fresh Host from cold reset.
    let mut cold = Host::new(deterministic_rng(0x5A));
    for pkt in data.split(|&b| b == 0x7E).take(32) {
        if !pkt.is_empty() {
            cold.on_packet(pkt);
            let _ = drain_host(&mut cold, strict);
        }
    }
});
