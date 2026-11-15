//! Simulated end to end: a scripted controller with a fake LE peer behind it. The host
//! stack brings the controller up, scans and finds a classic keyboard and an LE mouse by
//! name, connects to the mouse, and completes LE Secure Connections pairing against a
//! peer running the same crate's Security Manager as responder; then encryption starts
//! with the key both sides derived.
use sex_bt::hci::{self, op, Addr};
use sex_bt::host::{Host, Note};
use sex_bt::l2cap::{self, Reassembly};
use sex_bt::report::{self, ad};
use sex_bt::smp::{self, Action, Method, Role, Smp};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

const HOST_ADDR: [u8; 6] = [0x01, 0x00, 0x5e, 0x10, 0x87, 0x80];
const MOUSE: Addr = Addr([0x31, 0x32, 0x33, 0x34, 0x35, 0xc6]);
const KEYBOARD: Addr = Addr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
const HANDLE: u16 = 0x0040;

struct Peer { smp: Smp, rx: Reassembly, ltk: Option<[u8; 16]>, compare: Option<u32>, failed: Option<u8> }

struct Controller { out: VecDeque<Vec<u8>>, opcodes: Vec<u16>, peer: Peer, encrypted_with: Option<[u8; 16]> }

impl Controller {
    fn new(peer_io: u8, peer_auth: u8) -> Controller {
        let mut local = [0u8; 7];
        local[..6].copy_from_slice(&MOUSE.0);
        local[6] = 1;
        let mut remote = [0u8; 7];
        remote[..6].copy_from_slice(&HOST_ADDR);
        let smp = Smp::new(Role::Responder, peer_io, peer_auth, local, remote, &[0x5a; 32], [0x77; 16]).unwrap();
        Controller { out: VecDeque::new(), opcodes: vec![], peer: Peer { smp, rx: Reassembly::default(), ltk: None, compare: None, failed: None }, encrypted_with: None }
    }

    fn cc(&mut self, opcode: u16, ret: &[u8]) { self.out.push_back(hci::command_complete(opcode, ret)); }
    fn le_meta(&mut self, params: &[u8]) { self.out.push_back(hci::event(hci::ev::LE_META, params)); }

    fn peer_send(&mut self, pdu: &[u8]) {
        for p in l2cap::fragment(HANDLE, &l2cap::frame(l2cap::CID_SMP, pdu), 27) { self.out.push_back(p); }
    }

    fn peer_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Send(p) => self.peer_send(&p),
                Action::Compare(v) => {
                    self.peer.compare = Some(v);
                    // The peer's user says yes.
                    let more = self.peer.smp.confirm(true);
                    self.peer_actions(more);
                }
                Action::Ltk(k) => self.peer.ltk = Some(k),
                Action::Failed(r) => self.peer.failed = Some(r),
                Action::Encrypt(_) => unreachable!("peer is the responder"),
            }
        }
    }

    fn take(&mut self, p: &[u8]) {
        if let Some((h, pb, data)) = hci::parse_acl(p) {
            assert_eq!(h, HANDLE);
            assert!(data.len() <= 251, "fragment larger than the LE buffer");
            if let Some(f) = self.peer.rx.push(pb, data) {
                let (cid, payload) = l2cap::parse_frame(&f).unwrap();
                assert_eq!(cid, l2cap::CID_SMP);
                let a = self.peer.smp.on_pdu(payload);
                self.peer_actions(a);
            }
            return;
        }
        let (opcode, params) = hci::parse_command(p).expect("a command");
        self.opcodes.push(opcode);
        match opcode {
            op::READ_LOCAL_VERSION => self.cc(opcode, &[0, 0x0c, 0x00, 0x01, 0x0c, 0x02, 0x00, 0x00, 0x01]),
            op::READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0xfe, 0x8f, 0xfe, 0xd8, 0x3f, 0x5b, 0x87]),
            op::READ_BUFFER_SIZE => self.cc(opcode, &[0, 0xfd, 0x03, 0x40, 0x08, 0x00, 0x00, 0x00]),
            op::READ_BD_ADDR => { let mut r = vec![0]; r.extend_from_slice(&HOST_ADDR); self.cc(opcode, &r) }
            op::LE_READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0x59, 0, 0, 0, 0, 0, 0]),
            op::LE_READ_BUFFER_SIZE => self.cc(opcode, &[0, 0xfb, 0x00, 0x08]),
            op::INQUIRY => {
                self.out.push_back(hci::command_status(opcode, 0));
                let mut p = vec![0x01];
                p.extend_from_slice(&KEYBOARD.0);
                p.extend_from_slice(&[0x01, 0x00, 0x40, 0x25, 0x00, 0x00, 0x00, 0xc4]);
                p.extend_from_slice(&report::encode(&[(ad::COMPLETE_NAME, b"Silk Keyboard")]));
                p.resize(255, 0);
                self.out.push_back(hci::event(hci::ev::EXTENDED_INQUIRY_RESULT, &p));
                self.out.push_back(hci::event(hci::ev::INQUIRY_COMPLETE, &[0]));
            }
            op::LE_SET_EXTENDED_SCAN_ENABLE => {
                self.cc(opcode, &[0]);
                if params[0] == 1 {
                    let data = report::encode(&[(ad::FLAGS, &[0x06]), (ad::COMPLETE_NAME, b"Cuff Mouse"), (ad::APPEARANCE, &[0xc2, 0x03])]);
                    let mut r = vec![hci::ev::LE_EXTENDED_ADVERTISING_REPORT, 1, 0x13, 0x00, 0x01];
                    r.extend_from_slice(&MOUSE.0);
                    r.extend_from_slice(&[0x01, 0x00, 0xff, 0x7f, 0xd0, 0, 0, 0, 0, 0, 0, 0, 0, 0, data.len() as u8]);
                    r.extend_from_slice(&data);
                    self.le_meta(&r);
                }
            }
            op::LE_CREATE_CONNECTION => {
                assert_eq!(params[5], 1, "random address type");
                assert_eq!(&params[6..12], &MOUSE.0);
                self.out.push_back(hci::command_status(opcode, 0));
                let mut r = vec![hci::ev::LE_CONNECTION_COMPLETE, 0x00, 0x40, 0x00, 0x00, 0x01];
                r.extend_from_slice(&MOUSE.0);
                r.extend_from_slice(&[0x28, 0x00, 0x00, 0x00, 0xf4, 0x01, 0x00]);
                self.le_meta(&r);
                // The mouse asks for security.
                self.peer_send(&[smp::code::SECURITY_REQUEST, smp::AUTH_BONDING | smp::AUTH_MITM | smp::AUTH_SC]);
            }
            op::LE_START_ENCRYPTION => {
                self.out.push_back(hci::command_status(opcode, 0));
                let ltk: [u8; 16] = params[12..28].try_into().unwrap();
                self.encrypted_with = Some(ltk);
                let ok = Some(ltk) == self.peer.ltk;
                self.out.push_back(hci::event(hci::ev::ENCRYPTION_CHANGE, &[if ok { 0 } else { 0x06 }, 0x40, 0x00, ok as u8]));
            }
            _ => self.cc(opcode, &[0]),
        }
    }
}

fn host() -> Host {
    let n = Rc::new(Cell::new(1u8));
    Host::new(Box::new(move |b: &mut [u8]| { for x in b.iter_mut() { *x = n.get(); n.set(n.get().wrapping_mul(31).wrapping_add(7)); } }))
}

/// Move packets both ways until nothing moves; the notes the host made, in order.
fn pump(h: &mut Host, c: &mut Controller, compare_answer: bool) -> Vec<Note> {
    let mut notes = vec![];
    for _ in 0..10_000 {
        let mut moved = false;
        while let Some(p) = h.poll() { c.take(&p); moved = true; }
        while let Some(p) = c.out.pop_front() { h.on_packet(&p); moved = true; }
        while let Some(n) = h.note() {
            if let Note::Compare { handle, .. } = n { h.confirm(handle, compare_answer); }
            notes.push(n);
            moved = true;
        }
        if !moved { return notes; }
    }
    panic!("did not settle");
}

fn bring_up_and_scan(peer_io: u8, peer_auth: u8) -> (Host, Controller, Vec<Note>) {
    let mut h = host();
    let mut c = Controller::new(peer_io, peer_auth);
    let mut notes = pump(&mut h, &mut c, true);
    assert_eq!(c.opcodes, [op::RESET, op::READ_LOCAL_VERSION, op::READ_LOCAL_FEATURES, op::READ_BUFFER_SIZE, op::READ_BD_ADDR,
        op::SET_EVENT_MASK, op::WRITE_SIMPLE_PAIRING_MODE, op::WRITE_INQUIRY_MODE, op::WRITE_SCAN_ENABLE, op::WRITE_LE_HOST_SUPPORTED,
        op::LE_SET_EVENT_MASK, op::LE_READ_LOCAL_FEATURES, op::LE_READ_BUFFER_SIZE]);
    let Some(Note::Ready(info)) = notes.first() else { panic!("{notes:?}") };
    assert_eq!(info.addr, Addr(HOST_ADDR));
    assert!(info.le() && info.ssp() && info.eir() && info.le_extended_advertising());
    assert_eq!((info.acl_mtu, info.le_acl_mtu), (1021, 251));
    h.scan(4);
    notes.extend(pump(&mut h, &mut c, true));
    (h, c, notes)
}

#[test]
fn scan_then_le_secure_connections_numeric_comparison() {
    let (mut h, mut c, notes) = bring_up_and_scan(smp::io::DISPLAY_YES_NO, smp::AUTH_BONDING | smp::AUTH_MITM);
    let names: Vec<_> = notes.iter().filter_map(|n| match n { Note::Device(f) => Some((f.addr, f.name.clone(), f.rssi)), _ => None }).collect();
    assert_eq!(names, [(KEYBOARD, Some("Silk Keyboard".into()), Some(-60)), (MOUSE, Some("Cuff Mouse".into()), Some(-48))]);
    let mouse = h.devices().iter().find(|d| d.addr == MOUSE).unwrap().clone();
    assert_eq!((mouse.addr_type, mouse.appearance, mouse.connectable), (1, Some(0x03c2), true));

    h.connect_le(mouse.addr, mouse.addr_type);
    let notes = pump(&mut h, &mut c, true);
    assert!(notes.contains(&Note::Connected { handle: HANDLE, addr: MOUSE, le: true }), "{notes:?}");
    let value = notes.iter().find_map(|n| match n { Note::Compare { value, .. } => Some(*value), _ => None }).expect("numeric comparison");
    assert_eq!(Some(value), c.peer.compare, "both screens show the same number");
    assert!(value < 1_000_000);
    assert!(notes.contains(&Note::Paired { addr: MOUSE, le: true }));
    assert!(notes.contains(&Note::Encrypted { handle: HANDLE }), "{notes:?}");
    assert!(c.peer.ltk.is_some() && c.encrypted_with == c.peer.ltk);
    assert!(c.peer.smp.done() && c.peer.smp.method() == Method::NumericComparison);
}

#[test]
fn just_works_with_a_headless_peer() {
    let (mut h, mut c, _) = bring_up_and_scan(smp::io::NO_INPUT_NO_OUTPUT, smp::AUTH_BONDING);
    h.connect_le(MOUSE, 1);
    let notes = pump(&mut h, &mut c, true);
    assert!(!notes.iter().any(|n| matches!(n, Note::Compare { .. })));
    assert!(notes.contains(&Note::Encrypted { handle: HANDLE }), "{notes:?}");
    assert_eq!(c.peer.smp.method(), Method::JustWorks);
}

#[test]
fn user_says_no() {
    let (mut h, mut c, _) = bring_up_and_scan(smp::io::DISPLAY_YES_NO, smp::AUTH_BONDING | smp::AUTH_MITM);
    h.connect_le(MOUSE, 1);
    let notes = pump(&mut h, &mut c, false);
    assert!(notes.contains(&Note::PairingFailed { addr: MOUSE, reason: smp::reason::NUMERIC_COMPARISON_FAILED }), "{notes:?}");
    assert_eq!(c.peer.failed, Some(smp::reason::NUMERIC_COMPARISON_FAILED));
    assert!(c.encrypted_with.is_none() && c.peer.ltk.is_none());
}

#[test]
fn classic_ssp_event_flow() {
    let mut h = host();
    let mut c = Controller::new(smp::io::DISPLAY_YES_NO, 0);
    pump(&mut h, &mut c, true);
    let ev = |code: u8, p: &[u8]| hci::event(code, p);
    h.on_packet(&ev(hci::ev::CONNECTION_COMPLETE, &[0, 0x0b, 0x00, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x00]));
    h.pair(0x000b);
    h.on_packet(&ev(hci::ev::LINK_KEY_REQUEST, &KEYBOARD.0));
    h.on_packet(&ev(hci::ev::IO_CAPABILITY_REQUEST, &KEYBOARD.0));
    let mut resp = KEYBOARD.0.to_vec();
    resp.extend_from_slice(&[0x01, 0x00, 0x03]);
    h.on_packet(&ev(hci::ev::IO_CAPABILITY_RESPONSE, &resp));
    let mut ucr = KEYBOARD.0.to_vec();
    ucr.extend_from_slice(&654321u32.to_le_bytes());
    h.on_packet(&ev(hci::ev::USER_CONFIRMATION_REQUEST, &ucr));
    let mut n = vec![];
    while let Some(x) = h.note() { n.push(x); }
    assert!(n.contains(&Note::Compare { handle: 0x0b, addr: KEYBOARD, value: 654321 }), "{n:?}");
    h.confirm(0x0b, true);
    let mut lkn = KEYBOARD.0.to_vec();
    lkn.extend_from_slice(&[0x42; 16]);
    lkn.push(0x07);
    h.on_packet(&ev(hci::ev::LINK_KEY_NOTIFICATION, &lkn));
    let mut sent = vec![];
    while let Some(p) = h.poll() { sent.push(p.clone()); h.on_packet(&hci::command_status(0, 0)); }
    let ops: Vec<u16> = sent.iter().map(|p| hci::parse_command(p).unwrap().0).collect();
    assert_eq!(ops, [op::AUTHENTICATION_REQUESTED, op::LINK_KEY_REQUEST_NEGATIVE_REPLY, op::IO_CAPABILITY_REQUEST_REPLY, op::USER_CONFIRMATION_REQUEST_REPLY]);
    assert_eq!(hci::parse_command(&sent[2]).unwrap().1, [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x00, 0x03]);
    assert_eq!(h.ssp.bonds.len(), 1);
    // Next time the stored key answers the Link Key Request.
    h.on_packet(&ev(hci::ev::LINK_KEY_REQUEST, &KEYBOARD.0));
    let p = h.poll().unwrap();
    let (o, params) = hci::parse_command(&p).unwrap();
    assert_eq!((o, &params[6..]), (op::LINK_KEY_REQUEST_REPLY, &[0x42u8; 16][..]));
}
