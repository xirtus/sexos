//! Byte-exact ATT requests and response parsing, report maps decoded into keys and
//! pointer motion, the random address hash against the spec's sample data, and the
//! stored bond format.
use sex_bt::att::{self, op, uuid, Pdu};
use sex_bt::bonds::{Bonds, LeBond};
use sex_bt::crypto;
use sex_bt::hci::Addr;
use sex_bt::hid::{self, Input, ReportMap};
use sex_bt::hidp;
use sex_bt::ssp;

#[test]
fn att_requests() {
    assert_eq!(att::exchange_mtu(185), [0x02, 0xb9, 0x00]);
    assert_eq!(att::read_by_group_type(0x0001, 0xffff, uuid::PRIMARY_SERVICE), [0x10, 0x01, 0x00, 0xff, 0xff, 0x00, 0x28]);
    assert_eq!(att::read_by_type(0x0010, 0x0030, uuid::CHARACTERISTIC), [0x08, 0x10, 0x00, 0x30, 0x00, 0x03, 0x28]);
    assert_eq!(att::find_information(0x0013, 0x0015), [0x04, 0x13, 0x00, 0x15, 0x00]);
    assert_eq!(att::read(0x0018), [0x0a, 0x18, 0x00]);
    assert_eq!(att::read_blob(0x0018, 22), [0x0c, 0x18, 0x00, 0x16, 0x00]);
    assert_eq!(att::write(0x0014, &[1, 0]), [0x12, 0x14, 0x00, 0x01, 0x00]);
    assert_eq!(att::write_command(0x0020, &[0]), [0x52, 0x20, 0x00, 0x00]);
}

#[test]
fn att_responses() {
    // Two services: Generic Access (16-bit) and HID.
    let groups = [0x11, 0x06, 0x01, 0x00, 0x07, 0x00, 0x00, 0x18, 0x10, 0x00, 0x30, 0x00, 0x12, 0x18];
    assert_eq!(att::parse(&groups), Some(Pdu::Groups(vec![(0x0001, 0x0007, &[0x00, 0x18][..]), (0x0010, 0x0030, &[0x12, 0x18][..])])));
    // A 128-bit vendor service is not HID; the base-UUID form of 0x1812 is.
    let base_hid = [0xfb, 0x34, 0x9b, 0x5f, 0x80, 0x00, 0x00, 0x80, 0x00, 0x10, 0x00, 0x00, 0x12, 0x18, 0x00, 0x00];
    assert_eq!(att::uuid16(&base_hid), Some(0x1812));
    assert_eq!(att::uuid16(&[0xaa; 16]), None);
    // Characteristic declarations: properties, value handle, UUID.
    let types = [0x09, 0x07, 0x11, 0x00, 0x1a, 0x12, 0x00, 0x4d, 0x2a, 0x14, 0x00, 0x02, 0x15, 0x00, 0x4b, 0x2a];
    assert_eq!(att::parse(&types), Some(Pdu::Types(vec![(0x0011, &[0x1a, 0x12, 0x00, 0x4d, 0x2a][..]), (0x0014, &[0x02, 0x15, 0x00, 0x4b, 0x2a][..])])));
    let info = [0x05, 0x01, 0x13, 0x00, 0x02, 0x29, 0x14, 0x00, 0x08, 0x29];
    assert_eq!(att::parse(&info), Some(Pdu::Information(vec![(0x0013, &[0x02, 0x29][..]), (0x0014, &[0x08, 0x29][..])])));
    assert_eq!(att::parse(&[0x01, 0x0a, 0x18, 0x00, 0x05]), Some(Pdu::Error { request: op::READ_REQ, handle: 0x0018, code: att::error::INSUFFICIENT_AUTHENTICATION }));
    assert_eq!(att::parse(&[0x1b, 0x12, 0x00, 0x00, 0x00, 0x04]), Some(Pdu::Notification { handle: 0x0012, value: &[0, 0, 4] }));
    assert_eq!(att::parse(&[0x03, 0x40, 0x00]), Some(Pdu::Mtu(64)));
    // Our server side: MTU answered, discovery finds nothing, other requests refused.
    assert_eq!(att::serve(&[0x02, 0x40, 0x00]), Some(vec![0x03, 0xb9, 0x00]));
    assert_eq!(att::serve(&att::read_by_group_type(1, 0xffff, 0x2800)), Some(vec![0x01, 0x10, 0x01, 0x00, 0x0a]));
    assert_eq!(att::serve(&[0x0a, 0x03, 0x00]), Some(vec![0x01, 0x0a, 0x03, 0x00, 0x01]));
    assert_eq!(att::serve(&[0x1b, 0x03, 0x00, 0x01]), None);
    assert_eq!(att::serve(&[0x52, 0x03, 0x00, 0x01]), None);
}

/// A combined keyboard (report id 1) and mouse (report id 2) map, as many BLE keyboards with
/// touchpads send: keyboard with modifiers, reserved byte and 6-key array; mouse with five
/// buttons, 12-bit X/Y and a wheel.
pub const COMBO_MAP: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01,
    0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02,
    0x95, 0x01, 0x75, 0x08, 0x81, 0x01,
    0x95, 0x05, 0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x01,
    0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00,
    0xc0,
    0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x02, 0x09, 0x01, 0xa1, 0x00,
    0x05, 0x09, 0x19, 0x01, 0x29, 0x05, 0x15, 0x00, 0x25, 0x01, 0x95, 0x05, 0x75, 0x01, 0x81, 0x02,
    0x95, 0x01, 0x75, 0x03, 0x81, 0x01,
    0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x16, 0x01, 0xf8, 0x26, 0xff, 0x07, 0x75, 0x0c, 0x95, 0x02, 0x81, 0x06,
    0x09, 0x38, 0x15, 0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06,
    0xc0, 0xc0,
];

#[test]
fn report_maps() {
    let m = ReportMap::parse(COMBO_MAP);
    assert_eq!(m.layouts.len(), 2, "{m:?}");
    let k = &m.layouts[0];
    assert_eq!((k.id, k.modifiers.bit, k.keys.bit, k.key_count), (1, 0, 16, 6));
    assert!(k.keyboard() && !k.pointer());
    // Left shift + 'a' + 'b'.
    assert_eq!(m.decode(1, &[0x02, 0, 0x04, 0x05, 0, 0, 0, 0]), Some(Input::Keys { modifiers: 2, keys: [4, 5, 0, 0, 0, 0] }));
    // Rollover (every key 0x01) keeps the last state.
    assert_eq!(m.decode(1, &[0, 0, 1, 1, 1, 1, 1, 1]), None);
    let p = &m.layouts[1];
    assert_eq!((p.id, p.buttons.size, p.x.bit, p.x.size, p.y.bit, p.wheel.bit), (2, 5, 8, 12, 20, 32));
    // Left button, dx = -3 (0xffd), dy = +300 (0x12c), wheel -1.
    let (dx, dy) = (0xffdu32, 0x12cu32);
    let packed = dx | dy << 12;
    let r = [0x01, packed as u8, (packed >> 8) as u8, (packed >> 16) as u8, 0xff];
    assert_eq!(m.decode(2, &r), Some(Input::Pointer { buttons: 1, dx: -3, dy: 300, wheel: -1 }));
    assert_eq!(m.decode(3, &r), None);
    // Boot reports.
    assert_eq!(hid::boot_keyboard(&[0x01, 0, 0x29, 0, 0, 0, 0, 0]), Some(Input::Keys { modifiers: 1, keys: [0x29, 0, 0, 0, 0, 0] }));
    assert_eq!(hid::boot_mouse(&[0x0a, 0xfe, 0x03]), Some(Input::Pointer { buttons: 2, dx: -2, dy: 3, wheel: 0 }));
    assert_eq!(Input::packed_keys(&[4, 5, 0, 0, 0, 0]), 0x0504);
    // Classic HID interrupt messages.
    assert_eq!(hidp::input(&[0xa1, 0x02, 0x01, 0x05, 0xfb, 0x01]), Some(Input::Pointer { buttons: 1, dx: 5, dy: -5, wheel: 1 }));
    assert_eq!(hidp::input(&[0xa1, 0x01, 0x00, 0x00, 0x28, 0, 0, 0, 0, 0]), Some(Input::Keys { modifiers: 0, keys: [0x28, 0, 0, 0, 0, 0] }));
    assert_eq!(hidp::input(&[0xa2, 0x01, 0x00]), None);
    assert_eq!(hidp::control(&[0x15]), hidp::Control::Unplug);
    assert_eq!(hidp::control(&[0x00]), hidp::Control::Handshake(0));
}

#[test]
fn random_address_hash() {
    // Core 5.4, Vol 3 Part H, D.7: IRK ec0234a3 57c8ad05 341010a6 0a397d9b, prand 708194 -> 0dfbaa.
    let mut irk = [0xec, 0x02, 0x34, 0xa3, 0x57, 0xc8, 0xad, 0x05, 0x34, 0x10, 0x10, 0xa6, 0x0a, 0x39, 0x7d, 0x9b];
    irk.reverse();
    assert_eq!(crypto::ah(&irk, 0x708194), 0x0dfbaa);
    // The resolvable private address 70:81:94:0d:fb:aa, in wire order.
    let rpa = [0xaa, 0xfb, 0x0d, 0x94, 0x81, 0x70];
    assert!(crypto::resolves(&irk, &rpa));
    let mut other = rpa;
    other[0] ^= 1;
    assert!(!crypto::resolves(&irk, &other));
    // Not a resolvable address (top bits 11: static random).
    assert!(!crypto::resolves(&irk, &[0xaa, 0xfb, 0x0d, 0x94, 0x81, 0xf0]));
}

#[test]
fn bond_store() {
    let mut irk = [0u8; 16];
    irk.iter_mut().enumerate().for_each(|(i, b)| *b = i as u8);
    let mut b = Bonds::default();
    assert!(b.put_le(LeBond { addr: Addr([1, 2, 3, 4, 5, 0xc6]), addr_type: 1, ltk: [0x42; 16], irk: Some(irk), identity: Some((0, Addr([9, 8, 7, 6, 5, 4]))) }));
    assert!(b.put_classic(ssp::Bond { addr: Addr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]), key: [0x17; 16], key_type: 7 }));
    assert!(!b.put_classic(ssp::Bond { addr: Addr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]), key: [0x17; 16], key_type: 7 }), "unchanged");
    let bytes = b.encode();
    assert_eq!(bytes.len(), 8 + 2 * 48 + 4);
    assert_eq!(&bytes[..6], b"CUFB\x01\x02");
    assert_eq!(Bonds::decode(&bytes), Some(b.clone()));
    // Any damaged byte: nothing, never wrong keys.
    for at in [0, 5, 20, 60, bytes.len() - 1] {
        let mut bad = bytes.clone();
        bad[at] ^= 0x40;
        assert_eq!(Bonds::decode(&bad), None, "byte {at}");
    }
    assert_eq!(Bonds::decode(&bytes[..bytes.len() - 1]), None);
    // The identity address and an address the IRK resolves both find the bond.
    assert!(b.le_for(&Addr([9, 8, 7, 6, 5, 4])).is_some());
    let prand = 0x400000 | 0x1234;
    let hash = crypto::ah(&irk, prand);
    let rpa = Addr([hash as u8, (hash >> 8) as u8, (hash >> 16) as u8, prand as u8, (prand >> 8) as u8, (prand >> 16) as u8]);
    assert!(b.le_for(&rpa).is_some());
    assert!(b.forget(&rpa));
    assert_eq!(b.len(), 1);
    // Bounded: the oldest bond goes.
    for i in 0..40u8 { b.put_classic(ssp::Bond { addr: Addr([i; 6]), key: [i; 16], key_type: 4 }); }
    assert_eq!(b.len(), sex_bt::bonds::MAX);
    assert!(b.classic_for(&Addr([39; 6])).is_some() && b.classic_for(&Addr([0; 6])).is_none());
    assert_eq!(Bonds::decode(&b.encode()).unwrap().len(), sex_bt::bonds::MAX);
}
