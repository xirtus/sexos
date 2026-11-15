//! Simulated HID devices end to end, through a scripted controller:
//!
//! - an LE keyboard: the host connects it for HID, pairs (LE Secure Connections, Just
//!   Works), the keyboard hands over its identity (IRK), the host walks its GATT database
//!   (HID service, characteristics, descriptors, a report map longer than one read,
//!   report references), turns notifications on, and key reports come out as key events;
//!   the bond is stored; the keyboard comes back from a new private address, which the
//!   IRK resolves, and works again with the stored key and no pairing;
//! - a classic mouse: page, Secure Simple Pairing, encryption, HID control and interrupt
//!   channels, boot protocol, motion reports; it then reconnects to us by itself (it pages
//!   us and opens the channels) using the stored link key.
use sex_bt::att::{self, op, uuid};
use sex_bt::bonds::Bonds;
use sex_bt::crypto;
use sex_bt::hci::{self, op as hop, Addr};
use sex_bt::hid::Input;
use sex_bt::hidp;
use sex_bt::host::{code, Host, Note};
use sex_bt::l2cap::{self, ChanEvent, Channels, Reassembly};
use sex_bt::report::{self, ad, Found, Transport};
use sex_bt::smp::{self, Action, Role, Smp};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

const HOST_ADDR: [u8; 6] = [0x01, 0x00, 0x5e, 0x10, 0x87, 0x80];
const KEYBOARD_ID: Addr = Addr([0x11, 0x22, 0x33, 0x44, 0x55, 0xc6]); // static random identity
const MOUSE: Addr = Addr([0x66, 0x55, 0x44, 0x33, 0x22, 0x11]);
const HANDLE: u16 = 0x0040;
const IRK: [u8; 16] = [0x3c, 0x91, 0x0e, 0x55, 0x12, 0xa7, 0x40, 0x09, 0x6b, 0xde, 0x22, 0x81, 0x7f, 0x30, 0xc4, 0x5d];

/// A keyboard-only report map (report id 1) longer than one 22-byte read.
const MAP: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01,
    0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02,
    0x95, 0x01, 0x75, 0x08, 0x81, 0x01,
    0x95, 0x05, 0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x01,
    0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00,
    0xc0,
];

fn rpa(prand: u32) -> Addr {
    let prand = prand & 0x3f_ffff | 0x40_0000;
    let hash = crypto::ah(&IRK, prand);
    Addr([hash as u8, (hash >> 8) as u8, (hash >> 16) as u8, prand as u8, (prand >> 8) as u8, (prand >> 16) as u8])
}

/// One attribute of the keyboard's GATT database.
struct Attr { handle: u16, kind: u16, value: Vec<u8> }

fn database() -> Vec<Attr> {
    let decl = |props: u8, value: u16, uuid: u16| { let mut v = vec![props]; v.extend_from_slice(&value.to_le_bytes()); v.extend_from_slice(&uuid.to_le_bytes()); v };
    vec![
        Attr { handle: 0x0001, kind: uuid::PRIMARY_SERVICE, value: vec![0x00, 0x18] },
        Attr { handle: 0x0002, kind: uuid::CHARACTERISTIC, value: decl(0x02, 0x0003, 0x2a00) },
        Attr { handle: 0x0003, kind: 0x2a00, value: b"Silk BLE Keyboard".to_vec() },
        Attr { handle: 0x0010, kind: uuid::PRIMARY_SERVICE, value: vec![0x12, 0x18] },
        Attr { handle: 0x0011, kind: uuid::CHARACTERISTIC, value: decl(0x02, 0x0012, uuid::HID_INFORMATION) },
        Attr { handle: 0x0012, kind: uuid::HID_INFORMATION, value: vec![0x11, 0x01, 0x00, 0x02] },
        Attr { handle: 0x0013, kind: uuid::CHARACTERISTIC, value: decl(0x02, 0x0014, uuid::REPORT_MAP) },
        Attr { handle: 0x0014, kind: uuid::REPORT_MAP, value: MAP.to_vec() },
        Attr { handle: 0x0015, kind: uuid::CHARACTERISTIC, value: decl(0x12, 0x0016, uuid::REPORT) },
        Attr { handle: 0x0016, kind: uuid::REPORT, value: vec![0; 8] },
        Attr { handle: 0x0017, kind: uuid::CCCD, value: vec![0, 0] },
        Attr { handle: 0x0018, kind: uuid::REPORT_REFERENCE, value: vec![0x01, 0x01] },
        Attr { handle: 0x0019, kind: uuid::CHARACTERISTIC, value: decl(0x04, 0x001a, uuid::PROTOCOL_MODE) },
        Attr { handle: 0x001a, kind: uuid::PROTOCOL_MODE, value: vec![1] },
    ]
}

const SERVICE_END: [(u16, u16); 2] = [(0x0001, 0x0003), (0x0010, 0x001a)];

struct Keyboard { smp: Smp, ltk: Option<[u8; 16]>, db: Vec<Attr>, encrypted: bool, notify: bool, mtu: usize, early_refusals: u32 }

impl Keyboard {
    fn new(air: Addr) -> Keyboard {
        let mut local = [0u8; 7];
        local[..6].copy_from_slice(&air.0);
        local[6] = 1;
        let mut remote = [0u8; 7];
        remote[..6].copy_from_slice(&HOST_ADDR);
        let mut smp = Smp::new(Role::Responder, smp::io::NO_INPUT_NO_OUTPUT, smp::AUTH_BONDING, local, remote, &[0x5a; 32], [0x77; 16]).unwrap();
        smp.set_identity(IRK, 1, KEYBOARD_ID.0);
        Keyboard { smp, ltk: None, db: database(), encrypted: false, notify: false, mtu: 23, early_refusals: 0 }
    }

    /// The keyboard's GATT server.
    fn att(&mut self, p: &[u8]) -> Option<Vec<u8>> {
        let le16 = |at: usize| u16::from_le_bytes([p[at], p[at + 1]]);
        let err = |req: u8, h: u16, code: u8| { let mut r = vec![op::ERROR, req]; r.extend_from_slice(&h.to_le_bytes()); r.push(code); r };
        let room = self.mtu - 2;
        match p[0] {
            op::EXCHANGE_MTU_REQ => { self.mtu = le16(1).min(att::OUR_MTU) as usize; self.mtu = self.mtu.min(23); Some(vec![op::EXCHANGE_MTU_RSP, 23, 0]) }
            op::READ_BY_GROUP_TYPE_REQ => {
                let (start, end) = (le16(1), le16(3));
                let mut r = vec![op::READ_BY_GROUP_TYPE_RSP, 6];
                for a in self.db.iter().filter(|a| a.kind == uuid::PRIMARY_SERVICE && a.handle >= start && a.handle <= end) {
                    if r.len() + 6 > room + 2 { break; }
                    let e = SERVICE_END.iter().find(|s| s.0 == a.handle).unwrap().1;
                    r.extend_from_slice(&a.handle.to_le_bytes()); r.extend_from_slice(&e.to_le_bytes()); r.extend_from_slice(&a.value);
                }
                Some(if r.len() == 2 { err(p[0], start, att::error::ATTRIBUTE_NOT_FOUND) } else { r })
            }
            op::READ_BY_TYPE_REQ => {
                let (start, end, kind) = (le16(1), le16(3), le16(5));
                let mut r = vec![op::READ_BY_TYPE_RSP, 7];
                for a in self.db.iter().filter(|a| a.kind == kind && a.handle >= start && a.handle <= end) {
                    if r.len() + 7 > room + 2 { break; }
                    r.extend_from_slice(&a.handle.to_le_bytes()); r.extend_from_slice(&a.value);
                }
                Some(if r.len() == 2 { err(p[0], start, att::error::ATTRIBUTE_NOT_FOUND) } else { r })
            }
            op::FIND_INFORMATION_REQ => {
                let (start, end) = (le16(1), le16(3));
                let mut r = vec![op::FIND_INFORMATION_RSP, 1];
                for a in self.db.iter().filter(|a| a.handle >= start && a.handle <= end) {
                    if r.len() + 4 > room + 2 { break; }
                    r.extend_from_slice(&a.handle.to_le_bytes()); r.extend_from_slice(&a.kind.to_le_bytes());
                }
                Some(if r.len() == 2 { err(p[0], start, att::error::ATTRIBUTE_NOT_FOUND) } else { r })
            }
            op::READ_REQ | op::READ_BLOB_REQ => {
                let h = le16(1);
                let offset = if p[0] == op::READ_BLOB_REQ { le16(3) as usize } else { 0 };
                let Some(a) = self.db.iter().find(|a| a.handle == h) else { return Some(err(p[0], h, att::error::INVALID_HANDLE)) };
                // HID attributes need an encrypted link.
                if !self.encrypted && h >= 0x0010 { self.early_refusals += 1; return Some(err(p[0], h, att::error::INSUFFICIENT_AUTHENTICATION)); }
                let v = &a.value[offset.min(a.value.len())..];
                let mut r = vec![if p[0] == op::READ_REQ { op::READ_RSP } else { op::READ_BLOB_RSP }];
                r.extend_from_slice(&v[..v.len().min(self.mtu - 1)]);
                Some(r)
            }
            op::WRITE_REQ => {
                let h = le16(1);
                if h == 0x0017 { self.notify = p[3] & 1 != 0; }
                Some(vec![op::WRITE_RSP])
            }
            op::WRITE_CMD => None,
            _ => None,
        }
    }
}

struct Mouse { chans: Channels, link_key: Option<[u8; 16]>, handshake: u32, boot_protocol: bool, initiator: bool,
    /// A peer using the mouse's address without its link key.
    spoof: bool }

struct Controller {
    out: VecDeque<Vec<u8>>,
    opcodes: Vec<u16>,
    kb: Keyboard,
    mouse: Mouse,
    rx: Reassembly,
    /// The address the keyboard uses now (a resolvable private address).
    kb_addr: Addr,
    le_handle: u16,
    classic_handle: u16,
}

impl Controller {
    fn new() -> Controller {
        Controller { out: VecDeque::new(), opcodes: vec![], kb: Keyboard::new(rpa(0x123456)), rx: Reassembly::default(), kb_addr: rpa(0x123456),
            mouse: Mouse { chans: Channels::default(), link_key: None, handshake: 0, boot_protocol: false, initiator: false, spoof: false }, le_handle: HANDLE, classic_handle: 0x000b }
    }
    fn cc(&mut self, opcode: u16, ret: &[u8]) { self.out.push_back(hci::command_complete(opcode, ret)); }
    fn status(&mut self, opcode: u16) { self.out.push_back(hci::command_status(opcode, 0)); }
    fn ev(&mut self, code: u8, p: &[u8]) { self.out.push_back(hci::event(code, p)); }
    fn le_send(&mut self, cid: u16, pdu: &[u8]) { for p in l2cap::fragment(self.le_handle, &l2cap::frame(cid, pdu), 27) { self.out.push_back(p); } }
    fn classic_send(&mut self, cid: u16, pdu: &[u8]) { for p in l2cap::fragment(self.classic_handle, &l2cap::frame(cid, pdu), 1021) { self.out.push_back(p); } }

    fn kb_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Send(p) => self.le_send(l2cap::CID_SMP, &p),
                Action::Ltk(k) => self.kb.ltk = Some(k),
                Action::Compare(_) => { let more = self.kb.smp.confirm(true); self.kb_actions(more); }
                other => panic!("keyboard: {other:?}"),
            }
        }
    }

    fn mouse_events(&mut self, events: Vec<ChanEvent>) {
        for e in events {
            match e {
                ChanEvent::Send(p) => self.classic_send(l2cap::CID_SIGNALING, &p),
                ChanEvent::Open { psm: hidp::PSM_CONTROL, .. } if self.mouse.chans.by_psm(hidp::PSM_INTERRUPT).is_none() && self.mouse_initiates() => {
                    let p = self.mouse.chans.connect(hidp::PSM_INTERRUPT);
                    self.classic_send(l2cap::CID_SIGNALING, &p);
                }
                _ => {}
            }
        }
    }
    fn mouse_initiates(&self) -> bool { self.mouse.initiator }

    /// The mouse sends one boot report on its interrupt channel.
    fn mouse_moves(&mut self, report: &[u8]) {
        let remote = self.mouse.chans.by_psm(hidp::PSM_INTERRUPT).filter(|c| c.open).expect("interrupt channel").remote;
        let mut m = vec![hidp::DATA_INPUT, hidp::BOOT_MOUSE];
        m.extend_from_slice(report);
        self.classic_send(remote, &m);
    }

    /// The keyboard sends one report if notifications are on.
    fn key(&mut self, report: &[u8; 8]) {
        assert!(self.kb.notify, "notifications not enabled");
        let mut n = vec![op::NOTIFICATION, 0x16, 0x00];
        n.extend_from_slice(report);
        self.le_send(l2cap::CID_ATT, &n);
    }

    fn take(&mut self, p: &[u8]) {
        if let Some((h, pb, data)) = hci::parse_acl(p) {
            let Some(f) = self.rx.push(pb, data) else { return };
            let (cid, payload) = l2cap::parse_frame(&f).unwrap();
            if h == self.le_handle {
                match cid {
                    l2cap::CID_SMP => { let a = self.kb.smp.on_pdu(payload); self.kb_actions(a); }
                    l2cap::CID_ATT => { if let Some(r) = self.kb.att(payload) { self.le_send(l2cap::CID_ATT, &r); } }
                    _ => panic!("LE cid {cid:#x}"),
                }
            } else {
                assert_eq!(h, self.classic_handle);
                if cid == l2cap::CID_SIGNALING {
                    for (c, id, d) in l2cap::commands(payload) {
                        let e = self.mouse.chans.on_command(c, id, d, &|_| Ok(())).unwrap_or_default();
                        self.mouse_events(e);
                    }
                } else {
                    let ch = *self.mouse.chans.by_local(cid).expect("a mouse channel");
                    assert_eq!(ch.psm, hidp::PSM_CONTROL, "host writes only on control");
                    if payload == [hidp::SET_PROTOCOL_BOOT] {
                        self.mouse.boot_protocol = true;
                        self.mouse.handshake += 1;
                        self.classic_send(ch.remote, &[hidp::HANDSHAKE]);
                    }
                }
            }
            return;
        }
        let (opcode, params) = hci::parse_command(p).expect("a command");
        self.opcodes.push(opcode);
        let params = params.to_vec();
        match opcode {
            hop::READ_LOCAL_VERSION => self.cc(opcode, &[0, 0x0c, 0x00, 0x01, 0x0c, 0x02, 0x00, 0x00, 0x01]),
            hop::READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0xfe, 0x8f, 0xfe, 0xd8, 0x3f, 0x5b, 0x87]),
            hop::READ_BUFFER_SIZE => self.cc(opcode, &[0, 0xfd, 0x03, 0x40, 0x08, 0x00, 0x00, 0x00]),
            hop::READ_BD_ADDR => { let mut r = vec![0]; r.extend_from_slice(&HOST_ADDR); self.cc(opcode, &r) }
            hop::LE_READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0x59, 0, 0, 0, 0, 0, 0]),
            hop::LE_READ_BUFFER_SIZE => self.cc(opcode, &[0, 0xfb, 0x00, 0x08]),
            hop::LE_CREATE_CONNECTION => {
                assert_eq!((params[5], &params[6..12]), (1, &self.kb_addr.0[..]), "connects the keyboard's current address");
                self.status(opcode);
                let mut r = vec![hci::ev::LE_CONNECTION_COMPLETE, 0x00, self.le_handle as u8, 0x00, 0x00, 0x01];
                r.extend_from_slice(&self.kb_addr.0);
                r.extend_from_slice(&[0x28, 0x00, 0x00, 0x00, 0xf4, 0x01, 0x00]);
                self.ev(hci::ev::LE_META, &r);
                self.kb.encrypted = false;
                self.kb.notify = false;
                self.kb.mtu = 23;
                // A HID keyboard asks for security as soon as it is connected.
                self.le_send(l2cap::CID_SMP, &[smp::code::SECURITY_REQUEST, smp::AUTH_BONDING]);
            }
            hop::LE_START_ENCRYPTION => {
                self.status(opcode);
                let ltk: [u8; 16] = params[12..28].try_into().unwrap();
                let ok = Some(ltk) == self.kb.ltk;
                self.ev(hci::ev::ENCRYPTION_CHANGE, &[if ok { 0 } else { 0x06 }, self.le_handle as u8, 0x00, ok as u8]);
                if ok {
                    self.kb.encrypted = true;
                    // Key distribution after encryption (first pairing only).
                    let d = self.kb.smp.distribute();
                    self.kb_actions(d);
                }
            }
            hop::CREATE_CONNECTION => {
                assert_eq!(&params[..6], &MOUSE.0);
                self.status(opcode);
                let mut r = vec![0, self.classic_handle as u8, 0x00];
                r.extend_from_slice(&MOUSE.0);
                r.extend_from_slice(&[0x01, 0x00]);
                self.ev(hci::ev::CONNECTION_COMPLETE, &r);
            }
            code::ACCEPT_CONNECTION_REQUEST => {
                assert_eq!(&params[..6], &MOUSE.0);
                self.status(opcode);
                let mut r = vec![0, self.classic_handle as u8, 0x00];
                r.extend_from_slice(&MOUSE.0);
                r.extend_from_slice(&[0x01, 0x00]);
                self.ev(hci::ev::CONNECTION_COMPLETE, &r);
                if self.mouse.spoof {
                    // No key: open the HID channels straight away and hope.
                    self.mouse.chans = Channels::default();
                    self.mouse.initiator = true;
                    let p = self.mouse.chans.connect(hidp::PSM_CONTROL);
                    self.classic_send(l2cap::CID_SIGNALING, &p);
                }
                // The host authenticates the link with the stored key.
            }
            hop::AUTHENTICATION_REQUESTED if self.mouse.spoof => {
                // The challenge fails: the spoofer does not hold the link key.
                self.status(opcode);
                let h = self.classic_handle;
                self.ev(code::EV_AUTHENTICATION_COMPLETE, &[0x05, h as u8, (h >> 8) as u8]);
            }
            hop::AUTHENTICATION_REQUESTED => { self.status(opcode); self.ev(hci::ev::LINK_KEY_REQUEST, &MOUSE.0); }
            hop::LINK_KEY_REQUEST_NEGATIVE_REPLY => {
                self.cc(opcode, &[0]);
                self.ev(hci::ev::IO_CAPABILITY_REQUEST, &MOUSE.0);
            }
            hop::LINK_KEY_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                assert_eq!(Some(params[6..22].try_into().unwrap()), self.mouse.link_key, "the stored link key");
                let h = self.classic_handle;
                self.ev(code::EV_AUTHENTICATION_COMPLETE, &[0, h as u8, (h >> 8) as u8]);
                // The mouse turns on encryption and opens its HID channels.
                self.ev(hci::ev::ENCRYPTION_CHANGE, &[0, h as u8, (h >> 8) as u8, 1]);
                self.mouse.chans = Channels::default();
                self.mouse.initiator = true;
                let p = self.mouse.chans.connect(hidp::PSM_CONTROL);
                self.classic_send(l2cap::CID_SIGNALING, &p);
            }
            hop::IO_CAPABILITY_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                let mut r = MOUSE.0.to_vec();
                r.extend_from_slice(&[0x03, 0x00, 0x04]); // NoInputNoOutput: Just Works
                self.ev(hci::ev::IO_CAPABILITY_RESPONSE, &r);
                let mut u = MOUSE.0.to_vec();
                u.extend_from_slice(&123456u32.to_le_bytes());
                self.ev(hci::ev::USER_CONFIRMATION_REQUEST, &u);
            }
            hop::USER_CONFIRMATION_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                let mut s = vec![0];
                s.extend_from_slice(&MOUSE.0);
                self.ev(hci::ev::SIMPLE_PAIRING_COMPLETE, &s);
                let key = [0x5c; 16];
                self.mouse.link_key = Some(key);
                let mut n = MOUSE.0.to_vec();
                n.extend_from_slice(&key);
                n.push(0x04);
                self.ev(hci::ev::LINK_KEY_NOTIFICATION, &n);
                let h = self.classic_handle;
                self.ev(code::EV_AUTHENTICATION_COMPLETE, &[0, h as u8, (h >> 8) as u8]);
            }
            code::SET_CONNECTION_ENCRYPTION => {
                self.status(opcode);
                let h = self.classic_handle;
                self.ev(hci::ev::ENCRYPTION_CHANGE, &[0, h as u8, (h >> 8) as u8, 1]);
            }
            _ => self.cc(opcode, &[0]),
        }
    }

    fn le_disconnect(&mut self) {
        let h = self.le_handle;
        self.ev(hci::ev::DISCONNECTION_COMPLETE, &[0, h as u8, (h >> 8) as u8, 0x13]);
    }

    /// The keyboard advertises from address `addr`.
    fn advertise(&mut self, addr: Addr) {
        let data = report::encode(&[(ad::FLAGS, &[0x05]), (ad::APPEARANCE, &[0xc1, 0x03])]);
        let mut r = vec![hci::ev::LE_EXTENDED_ADVERTISING_REPORT, 1, 0x13, 0x00, 0x01];
        r.extend_from_slice(&addr.0);
        r.extend_from_slice(&[0x01, 0x00, 0xff, 0x7f, 0xc4, 0, 0, 0, 0, 0, 0, 0, 0, 0, data.len() as u8]);
        r.extend_from_slice(&data);
        self.ev(hci::ev::LE_META, &r);
    }
}

fn host() -> Host {
    let n = Rc::new(Cell::new(1u8));
    Host::new(Box::new(move |b: &mut [u8]| { for x in b.iter_mut() { *x = n.get(); n.set(n.get().wrapping_mul(31).wrapping_add(7)); } }))
}

fn pump(h: &mut Host, c: &mut Controller) -> Vec<Note> {
    let mut notes = vec![];
    for _ in 0..10_000 {
        let mut moved = false;
        while let Some(p) = h.poll() { c.take(&p); moved = true; }
        while let Some(p) = c.out.pop_front() { h.on_packet(&p); moved = true; }
        while let Some(n) = h.note() {
            if let Note::Compare { handle, .. } = n { h.confirm(handle, true); }
            notes.push(n);
            moved = true;
        }
        if !moved { return notes; }
    }
    panic!("did not settle");
}

fn ready() -> (Host, Controller) {
    let mut h = host();
    let mut c = Controller::new();
    let n = pump(&mut h, &mut c);
    assert!(matches!(n.first(), Some(Note::Ready(_))), "{n:?}");
    (h, c)
}

fn inputs(notes: &[Note]) -> Vec<Input> { notes.iter().filter_map(|n| match n { Note::Input { input, .. } => Some(*input), _ => None }).collect() }

#[test]
fn ble_keyboard_pairs_types_and_comes_back() {
    let (mut h, mut c) = ready();
    let first = c.kb_addr;
    let found = Found { addr: first, transport: Transport::Le, addr_type: 1, rssi: Some(-50), class: None,
        name: Some("Silk BLE Keyboard".into()), appearance: Some(0x03c1), flags: Some(0x05), connectable: true };
    assert!(Host::is_hid(&found));
    h.connect_hid(&found);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::Paired { addr: first, le: true }), "{notes:?}");
    assert!(notes.contains(&Note::Encrypted { handle: HANDLE }));
    assert!(notes.contains(&Note::HidReady { handle: HANDLE, addr: first, le: true, keyboard: true, pointer: false, boot: false }), "{notes:?}");
    assert_eq!(c.kb.early_refusals, 0, "GATT discovery waited for encryption");
    assert!(c.kb.notify, "input report notifications on");
    // The bond holds the LTK and the identity the keyboard handed over.
    let b = h.bonds.le_for(&first).copied().expect("bonded");
    assert_eq!((b.ltk, b.irk, b.identity), (c.kb.ltk.unwrap(), Some(IRK), Some((1, KEYBOARD_ID))));
    assert!(notes.iter().filter(|n| **n == Note::BondsChanged).count() >= 2);
    // Typing: shift + 'h', then 'i', then release.
    c.key(&[0x02, 0, 0x0b, 0, 0, 0, 0, 0]);
    c.key(&[0x00, 0, 0x0c, 0, 0, 0, 0, 0]);
    c.key(&[0; 8]);
    let typed = inputs(&pump(&mut h, &mut c));
    assert_eq!(typed, [Input::Keys { modifiers: 2, keys: [0x0b, 0, 0, 0, 0, 0] }, Input::Keys { modifiers: 0, keys: [0x0c, 0, 0, 0, 0, 0] },
        Input::Keys { modifiers: 0, keys: [0; 6] }]);

    // The bonds survive a restart through the stored form.
    let stored = h.bonds.encode();
    let mut h2 = host();
    let mut c2 = Controller { kb: c.kb, ..Controller::new() };
    pump(&mut h2, &mut c2);
    h2.set_bonds(Bonds::decode(&stored).unwrap());
    // The keyboard went away and comes back from a new private address.
    c2.le_disconnect();
    c2.kb_addr = rpa(0x2a0b1c);
    assert_ne!(c2.kb_addr, first);
    let back = c2.kb_addr;
    c2.advertise(back);
    let notes = pump(&mut h2, &mut c2);
    assert!(notes.contains(&Note::Encrypted { handle: HANDLE }), "{notes:?}");
    assert!(!notes.iter().any(|n| matches!(n, Note::Paired { .. } | Note::Compare { .. } | Note::PairingFailed { .. })), "no pairing again: {notes:?}");
    assert!(notes.iter().any(|n| matches!(n, Note::HidReady { addr, .. } if *addr == back)), "{notes:?}");
    c2.key(&[0x00, 0, 0x28, 0, 0, 0, 0, 0]);
    assert_eq!(inputs(&pump(&mut h2, &mut c2)), [Input::Keys { modifiers: 0, keys: [0x28, 0, 0, 0, 0, 0] }]);
    // Encryption used the stored key, not a new pairing.
    assert_eq!(c2.opcodes.iter().filter(|o| **o == hop::LE_START_ENCRYPTION).count(), 1);
}

#[test]
fn unknown_le_device_is_not_reconnected() {
    let (mut h, mut c) = ready();
    c.advertise(rpa(0x111111));
    let notes = pump(&mut h, &mut c);
    assert!(notes.iter().any(|n| matches!(n, Note::Device(_))));
    assert!(!c.opcodes.contains(&hop::LE_CREATE_CONNECTION), "no bond, no connection");
}

#[test]
fn classic_mouse_pairs_moves_and_reconnects() {
    let (mut h, mut c) = ready();
    let found = Found { addr: MOUSE, transport: Transport::Classic, addr_type: 0, rssi: Some(-40), class: Some(0x002580),
        name: Some("Cuff Classic Mouse".into()), appearance: None, flags: None, connectable: true };
    assert!(Host::is_hid(&found));
    h.connect_hid(&found);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::Paired { addr: MOUSE, le: false }), "{notes:?}");
    assert!(notes.contains(&Note::Encrypted { handle: 0x000b }), "{notes:?}");
    // Minor class 0x80: a pointing device, not a keyboard.
    assert!(notes.contains(&Note::HidReady { handle: 0x000b, addr: MOUSE, le: false, keyboard: false, pointer: true, boot: true }), "{notes:?}");
    assert!(c.mouse.boot_protocol && c.mouse.handshake == 1);
    assert_eq!(h.bonds.classic_for(&MOUSE).map(|b| b.key), Some([0x5c; 16]));
    c.mouse_moves(&[0x01, 0x05, 0xfb, 0x00]);
    c.mouse_moves(&[0x00, 0x80, 0x7f, 0x01]);
    assert_eq!(inputs(&pump(&mut h, &mut c)), [Input::Pointer { buttons: 1, dx: 5, dy: -5, wheel: 0 }, Input::Pointer { buttons: 0, dx: -128, dy: 127, wheel: 1 }]);
    let order: Vec<u16> = c.opcodes.iter().copied().filter(|o| [hop::CREATE_CONNECTION, hop::AUTHENTICATION_REQUESTED, hop::LINK_KEY_REQUEST_NEGATIVE_REPLY,
        hop::IO_CAPABILITY_REQUEST_REPLY, hop::USER_CONFIRMATION_REQUEST_REPLY, code::SET_CONNECTION_ENCRYPTION].contains(o)).collect();
    assert_eq!(order, [hop::CREATE_CONNECTION, hop::AUTHENTICATION_REQUESTED, hop::LINK_KEY_REQUEST_NEGATIVE_REPLY,
        hop::IO_CAPABILITY_REQUEST_REPLY, hop::USER_CONFIRMATION_REQUEST_REPLY, code::SET_CONNECTION_ENCRYPTION]);

    // The link drops; later the mouse pages us and opens its channels itself.
    c.ev(hci::ev::DISCONNECTION_COMPLETE, &[0, 0x0b, 0x00, 0x08]);
    pump(&mut h, &mut c);
    c.classic_handle = 0x000c;
    let mut req = MOUSE.0.to_vec();
    req.extend_from_slice(&[0x80, 0x25, 0x00, 0x01]);
    c.ev(code::EV_CONNECTION_REQUEST, &req);
    let notes = pump(&mut h, &mut c);
    assert!(c.opcodes.contains(&code::ACCEPT_CONNECTION_REQUEST));
    assert!(notes.iter().any(|n| matches!(n, Note::HidReady { handle: 0x000c, addr, .. } if *addr == MOUSE)), "{notes:?}");
    assert!(!notes.iter().any(|n| matches!(n, Note::Paired { .. })), "stored key, no pairing: {notes:?}");
    assert_eq!(c.mouse.handshake, 2);
    c.mouse_moves(&[0x04, 0x01, 0x01]);
    assert_eq!(inputs(&pump(&mut h, &mut c)), [Input::Pointer { buttons: 4, dx: 1, dy: 1, wheel: 0 }]);
}

#[test]
fn stranger_paging_us_is_refused() {
    let (mut h, mut c) = ready();
    let mut req = MOUSE.0.to_vec();
    req.extend_from_slice(&[0x80, 0x25, 0x00, 0x01]);
    c.ev(code::EV_CONNECTION_REQUEST, &req);
    pump(&mut h, &mut c);
    assert!(c.opcodes.contains(&code::REJECT_CONNECTION_REQUEST));
    assert!(!c.opcodes.contains(&code::ACCEPT_CONNECTION_REQUEST));
}

#[test]
fn spoofed_bonded_address_injects_nothing() {
    let (mut h, mut c) = ready();
    let found = Found { addr: MOUSE, transport: Transport::Classic, addr_type: 0, rssi: Some(-40), class: Some(0x002580),
        name: Some("Cuff Classic Mouse".into()), appearance: None, flags: None, connectable: true };
    h.connect_hid(&found);
    pump(&mut h, &mut c);
    assert!(h.bonds.classic_for(&MOUSE).is_some());
    c.ev(hci::ev::DISCONNECTION_COMPLETE, &[0, 0x0b, 0x00, 0x08]);
    pump(&mut h, &mut c);
    // Someone else pages us with the mouse's address, opens HID channels and sends input.
    c.classic_handle = 0x000d;
    c.mouse.spoof = true;
    let mut req = MOUSE.0.to_vec();
    req.extend_from_slice(&[0x80, 0x25, 0x00, 0x01]);
    c.ev(code::EV_CONNECTION_REQUEST, &req);
    let mut notes = pump(&mut h, &mut c);
    if c.mouse.chans.by_psm(hidp::PSM_INTERRUPT).is_some_and(|ch| ch.open) {
        c.mouse_moves(&[0x01, 0x10, 0x10]);
        notes.extend(pump(&mut h, &mut c));
    }
    assert!(inputs(&notes).is_empty(), "{notes:?}");
    assert!(!notes.iter().any(|n| matches!(n, Note::Encrypted { handle: 0x000d })), "{notes:?}");
    assert!(notes.iter().any(|n| matches!(n, Note::PairingFailed { addr, .. } if *addr == MOUSE)), "{notes:?}");
    assert!(c.opcodes.contains(&code::DISCONNECT), "the impostor is dropped");
}
