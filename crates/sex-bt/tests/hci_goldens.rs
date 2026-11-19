//! Byte-exact HCI goldens: commands we build, events we decode, ACL and L2CAP framing,
//! and the advertising/inquiry report layouts.
use sex_bt::hci::{self, op, Addr, Event};
use sex_bt::report::{self, Transport};
use sex_bt::{controller, l2cap};

#[test]
fn command_goldens() {
    assert_eq!(hci::command(op::RESET, &[]), [0x01, 0x03, 0x0c, 0x00]);
    assert_eq!(hci::command(op::SET_EVENT_MASK, &controller::EVENT_MASK.to_le_bytes()),
        [0x01, 0x01, 0x0c, 0x08, 0xff, 0xff, 0xfb, 0xff, 0x07, 0xf8, 0xbf, 0x3d]);
    assert_eq!(hci::command(op::LE_SET_EVENT_MASK, &controller::LE_EVENT_MASK.to_le_bytes()),
        [0x01, 0x01, 0x20, 0x08, 0x1f, 0x12, 0, 0, 0, 0, 0, 0]);
    let mut info = controller::Info::default();
    info.features[4] = 0x40;
    info.le_features[1] = 0x10;
    let scan = controller::start_scan(&info, 8);
    assert_eq!(scan[0], [0x01, 0x01, 0x04, 0x05, 0x33, 0x8b, 0x9e, 0x08, 0x00]);
    assert_eq!(scan[1], [0x01, 0x41, 0x20, 0x08, 0x00, 0x00, 0x01, 0x01, 0x60, 0x00, 0x30, 0x00]);
    assert_eq!(scan[2], [0x01, 0x42, 0x20, 0x06, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00]);
    info.le_features[1] = 0;
    let scan = controller::start_scan(&info, 8);
    assert_eq!(scan[1], [0x01, 0x0b, 0x20, 0x07, 0x01, 0x60, 0x00, 0x30, 0x00, 0x00, 0x00]);
    assert_eq!(scan[2], [0x01, 0x0c, 0x20, 0x02, 0x01, 0x01]);
    assert_eq!(hci::parse_command(&scan[2]), Some((op::LE_SET_SCAN_ENABLE, &[1u8, 1][..])));
    assert_eq!(hci::parse_command(&[0x01, 0x03, 0x0c, 0x01]), None);
}

#[test]
fn event_goldens() {
    let p = [0x04, 0x0e, 0x0a, 0x01, 0x09, 0x10, 0x00, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11];
    let e = hci::parse_event(&p).unwrap();
    assert_eq!(e, Event::CommandComplete { ncmd: 1, opcode: op::READ_BD_ADDR, ret: &[0, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11] });
    assert_eq!(hci::command_complete(op::READ_BD_ADDR, &[0, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]), p);
    assert_eq!(hci::command_status(op::INQUIRY, 0), [0x04, 0x0f, 0x04, 0x00, 0x01, 0x01, 0x04]);
    assert_eq!(hci::parse_event(&[0x04, 0x0f, 0x04, 0x0c, 0x01, 0x01, 0x04]),
        Some(Event::CommandStatus { status: 0x0c, ncmd: 1, opcode: op::INQUIRY }));
    // Wrong length is refused, not half-read.
    assert_eq!(hci::parse_event(&[0x04, 0x0e, 0x04, 0x01, 0x03, 0x0c]), None);
    // User Confirmation Request: address then value (le32).
    let e = hci::parse_event(&[0x04, 0x33, 0x0a, 1, 2, 3, 4, 5, 6, 0x40, 0xe2, 0x01, 0x00]).unwrap();
    assert_eq!(e, Event::UserConfirmationRequest { addr: Addr([1, 2, 3, 4, 5, 6]), value: 123456 });
    assert_eq!(format!("{}", Addr([1, 2, 3, 4, 5, 6])), "06:05:04:03:02:01");
    // LE Connection Complete.
    let e = hci::parse_event(&[0x04, 0x3e, 0x13, 0x01, 0x00, 0x40, 0x00, 0x00, 0x01, 1, 2, 3, 4, 5, 6, 0x28, 0, 0, 0, 0xf4, 0x01, 0]).unwrap();
    assert_eq!(e, Event::LeConnectionComplete { status: 0, handle: 0x40, role: 0, peer_type: 1, peer: Addr([1, 2, 3, 4, 5, 6]) });
    // LE LTK Request.
    let e = hci::parse_event(&[0x04, 0x3e, 0x0d, 0x05, 0x40, 0x00, 1, 2, 3, 4, 5, 6, 7, 8, 0x34, 0x12]).unwrap();
    assert_eq!(e, Event::LeLtkRequest { handle: 0x40, rand: [1, 2, 3, 4, 5, 6, 7, 8], ediv: 0x1234 });
    // Intel vendor event: boot up.
    assert_eq!(hci::parse_event(&[0x04, 0xff, 0x03, 0x02, 0x00, 0x01]), Some(Event::Vendor { params: &[0x02, 0x00, 0x01] }));
}

#[test]
fn acl_and_l2cap_goldens() {
    let f = l2cap::frame(l2cap::CID_SMP, &[0x0b, 0x0d]);
    assert_eq!(f, [0x02, 0x00, 0x06, 0x00, 0x0b, 0x0d]);
    assert_eq!(hci::acl(0x40, hci::PB_FIRST_NON_FLUSHABLE, &f), [0x02, 0x40, 0x00, 0x06, 0x00, 0x02, 0x00, 0x06, 0x00, 0x0b, 0x0d]);
    // A 70-byte frame in 27-byte ACL packets, reassembled.
    let big = l2cap::frame(l2cap::CID_ATT, &[7u8; 66]);
    let parts = l2cap::fragment(0x41, &big, 27);
    assert_eq!(parts.len(), 3);
    assert_eq!(&parts[1][..5], &[0x02, 0x41, 0x10, 27, 0]);
    let mut r = l2cap::Reassembly::default();
    let mut whole = None;
    for p in &parts {
        let (h, pb, d) = hci::parse_acl(p).unwrap();
        assert_eq!(h, 0x41);
        whole = r.push(pb, d);
    }
    assert_eq!(whole.unwrap(), big);
    // LE connection parameter update: accepted.
    let req = [0x12, 0x07, 0x08, 0x00, 0x18, 0x00, 0x28, 0x00, 0x00, 0x00, 0xf4, 0x01];
    match l2cap::signal(l2cap::CID_LE_SIGNALING, &req) {
        l2cap::Signal::Accepted { reply, min: 0x18, max: 0x28, .. } => assert_eq!(reply, [0x13, 0x07, 0x02, 0x00, 0x00, 0x00]),
        s => panic!("{s:?}"),
    }
    // Classic connection request for HID control (PSM 0x11): refused, PSM not supported.
    assert_eq!(l2cap::signal(l2cap::CID_SIGNALING, &[0x02, 0x03, 0x04, 0x00, 0x11, 0x00, 0x40, 0x00]),
        l2cap::Signal::Reply(vec![0x03, 0x03, 0x08, 0x00, 0x00, 0x00, 0x40, 0x00, 0x02, 0x00, 0x00, 0x00]));
}

#[test]
fn report_goldens() {
    // Extended Inquiry Result: one device, class 0x002540 (keyboard), RSSI -60, name in EIR.
    let mut p = vec![0x01, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x00, 0x40, 0x25, 0x00, 0x00, 0x00, 0xc4];
    p.extend_from_slice(&report::encode(&[(report::ad::COMPLETE_NAME, b"Silk Keyboard")]));
    p.resize(15 + 240, 0);
    let ev = hci::event(hci::ev::EXTENDED_INQUIRY_RESULT, &p);
    let Some(Event::Report { code, sub, params }) = hci::parse_event(&ev) else { panic!() };
    let f = report::reports(code, sub, params);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].addr, Addr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]));
    assert_eq!((f[0].class, f[0].rssi, f[0].name.as_deref()), (Some(0x002540), Some(-60), Some("Silk Keyboard")));
    // Inquiry Result with RSSI, two responses in the spec's per-field array layout.
    let p = [2, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 1, 1, 0, 0, 0x0c, 0x01, 0x1f, 0x04, 0x05, 0x00, 0, 0, 0, 0, 0xb0, 0xa6];
    let f = report::reports(hci::ev::INQUIRY_RESULT_WITH_RSSI, 0, &p);
    assert_eq!(f.len(), 2);
    assert_eq!((f[0].addr.0[0], f[0].class, f[0].rssi), (1, Some(0x1f010c), Some(-80)));
    assert_eq!((f[1].addr.0[0], f[1].class, f[1].rssi), (2, Some(0x000504), Some(-90)));
    // LE Advertising Report: ADV_IND, random address, flags + shortened then complete name.
    let data = report::encode(&[(report::ad::FLAGS, &[0x06]), (report::ad::SHORT_NAME, b"Cuff"), (report::ad::COMPLETE_NAME, b"Cuff Mouse"), (report::ad::APPEARANCE, &[0xc2, 0x03])]);
    let mut p = vec![0x02, 0x01, 0x00, 0x01, 1, 2, 3, 4, 5, 0xc6, data.len() as u8];
    p.extend_from_slice(&data);
    p.push(0xd0);
    let f = report::reports(hci::ev::LE_META, hci::ev::LE_ADVERTISING_REPORT, &p[1..]);
    assert_eq!(f.len(), 1);
    let d = &f[0];
    assert_eq!((d.transport, d.addr_type, d.connectable, d.rssi), (Transport::Le, 1, true, Some(-48)));
    assert_eq!((d.name.as_deref(), d.appearance, d.flags), (Some("Cuff Mouse"), Some(0x03c2), Some(0x06)));
    // Truncated: nothing rather than garbage.
    assert!(report::reports(hci::ev::LE_META, hci::ev::LE_ADVERTISING_REPORT, &p[1..p.len() - 2]).is_empty());
}
