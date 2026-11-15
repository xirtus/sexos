//! A simulated headset's AVRCP through a scripted controller: a bonded headset pages us,
//! proves its stored link key, the link is encrypted and the headset opens the AVCTP
//! control channel (PSM 0x17).
//!
//! - its buttons (PASS THROUGH press and release) come out as `Note::RemoteKey`, each
//!   answered ACCEPTED; buttons we do not take are NOT IMPLEMENTED;
//! - absolute volume both ways: we register for EVENT_VOLUME_CHANGED, the headset's
//!   INTERIM and CHANGED come out as `Note::RemoteVolume` (and we register again after
//!   each CHANGED), `set_remote_volume` sends SetAbsoluteVolume, and setting the volume
//!   the headset already has sends nothing (no echo loop);
//! - a headset without absolute volume: `Note::RemoteNoVolume`, its volume buttons as keys;
//! - an unencrypted link: buttons ignored and nothing sent until encryption is on, and
//!   again when it goes off;
//! - hostile AVCTP/AVRCP (fragments, lying lengths, other profiles, random bytes):
//!   bounded answers, never a panic;
//! - our SDP server lists the AVRCP Target and Controller records.
use sex_bt::avrcp::{self, ctype, opcode, op, pdu, Key, Remote};
use sex_bt::bonds::Bonds;
use sex_bt::hci::{self, op as hop, Addr};
use sex_bt::host::{code, Host, Note};
use sex_bt::l2cap::{self, ChanEvent, Channels, Reassembly};
use sex_bt::sdp;
use sex_bt::ssp;
use std::collections::VecDeque;

const HOST_ADDR: [u8; 6] = [0x01, 0x00, 0x5e, 0x10, 0x87, 0x80];
const HEADSET: Addr = Addr([0x10, 0x20, 0x30, 0x40, 0x50, 0x61]);
const KEY: [u8; 16] = [0x5c; 16];
const HANDLE: u16 = 0x000b;

/// How the headset answers our volume registration.
#[derive(Clone, Copy, PartialEq)]
enum Volume { Has(u8), Refuses, NoProfile }

struct Controller {
    out: VecDeque<Vec<u8>>,
    rx: Reassembly,
    chans: Channels,
    /// The headset's AVCTP channel (its local cid).
    avctp: Option<u16>,
    /// Encryption is turned on when asked.
    encrypt: bool,
    /// The headset opens AVCTP as soon as the link is up (before encryption).
    early: bool,
    volume: Volume,
    /// What arrived on its AVCTP channel: our commands and our responses.
    commands: Vec<Vec<u8>>,
    responses: Vec<Vec<u8>>,
    /// The label of our last volume registration, and the volumes we set.
    register: Option<u8>,
    sets: Vec<u8>,
}

impl Controller {
    fn new() -> Controller {
        Controller { out: VecDeque::new(), rx: Reassembly::default(), chans: Channels::default(), avctp: None, encrypt: true, early: false,
            volume: Volume::Has(64), commands: vec![], responses: vec![], register: None, sets: vec![] }
    }
    fn cc(&mut self, opcode: u16, ret: &[u8]) { self.out.push_back(hci::command_complete(opcode, ret)); }
    fn ev(&mut self, code: u8, p: &[u8]) { self.out.push_back(hci::event(code, p)); }
    fn send(&mut self, cid: u16, pdu: &[u8]) { for p in l2cap::fragment(HANDLE, &l2cap::frame(cid, pdu), 1021) { self.out.push_back(p); } }
    fn encryption(&mut self, on: bool) { self.ev(hci::ev::ENCRYPTION_CHANGE, &[0, HANDLE as u8, 0, on as u8]); }

    fn open_avctp(&mut self) { let p = self.chans.connect(avrcp::PSM); self.send(l2cap::CID_SIGNALING, &p); }
    /// The headset sends one AVCTP packet on its control channel.
    fn avctp_send(&mut self, p: &[u8]) {
        let remote = self.chans.by_local(self.avctp.expect("control channel open")).unwrap().remote;
        self.send(remote, p);
    }
    fn press(&mut self, label: u8, id: u8, pressed: bool) {
        let m = [ctype::CONTROL, opcode::PANEL, opcode::PASS_THROUGH, id | (!pressed as u8) << 7, 0];
        self.avctp_send(&avrcp::packet(label, false, false, avrcp::PID, &m));
    }
    /// The headset's own volume change, on our registration.
    fn changed(&mut self, volume: u8) {
        let label = self.register.expect("registered");
        self.volume = Volume::Has(volume);
        self.avctp_send(&avrcp::packet(label, true, false, avrcp::PID, &avrcp::vendor(ctype::CHANGED, pdu::REGISTER_NOTIFICATION, &[pdu::EVENT_VOLUME_CHANGED, volume])));
    }

    fn on_avctp(&mut self, p: &[u8]) {
        let m = avrcp::parse(p).expect("single AVCTP packets from the host");
        if m.response { self.responses.push(p.to_vec()); return; }
        assert_eq!(m.pid, avrcp::PID);
        self.commands.push(p.to_vec());
        let reply = |c: &mut Controller, msg: &[u8]| { let r = avrcp::packet(m.label, true, false, avrcp::PID, msg); c.avctp_send(&r); };
        match avrcp::parse_vendor(m.msg) {
            Some((ctype::NOTIFY, pdu::REGISTER_NOTIFICATION, params)) => {
                assert_eq!(params, &[pdu::EVENT_VOLUME_CHANGED, 0, 0, 0, 0]);
                self.register = Some(m.label);
                match self.volume {
                    Volume::Has(v) => reply(self, &avrcp::vendor(ctype::INTERIM, pdu::REGISTER_NOTIFICATION, &[pdu::EVENT_VOLUME_CHANGED, v])),
                    Volume::Refuses => { let mut e = m.msg.to_vec(); e[0] = ctype::NOT_IMPLEMENTED; reply(self, &e); }
                    Volume::NoProfile => { let r = avrcp::packet(m.label, true, true, avrcp::PID, &[]); self.avctp_send(&r); }
                }
            }
            Some((ctype::CONTROL, pdu::SET_ABSOLUTE_VOLUME, &[v])) => {
                assert!(v <= avrcp::MAX_VOLUME);
                self.sets.push(v);
                self.volume = Volume::Has(v);
                reply(self, &avrcp::vendor(ctype::ACCEPTED, pdu::SET_ABSOLUTE_VOLUME, &[v]));
            }
            other => panic!("unexpected command {other:?}"),
        }
    }

    fn chan_events(&mut self, e: Vec<ChanEvent>) {
        for e in e {
            match e {
                ChanEvent::Send(p) => self.send(l2cap::CID_SIGNALING, &p),
                ChanEvent::Open { psm: avrcp::PSM, local } => self.avctp = Some(local),
                ChanEvent::Closed { local, .. } if self.avctp == Some(local) => self.avctp = None,
                _ => {}
            }
        }
    }

    fn take(&mut self, p: &[u8]) {
        if let Some((h, pb, data)) = hci::parse_acl(p) {
            assert_eq!(h, HANDLE);
            let h = HANDLE;
            self.ev(hci::ev::NUMBER_OF_COMPLETED_PACKETS, &[1, h as u8, (h >> 8) as u8, 1, 0]);
            let Some(f) = self.rx.push(pb, data) else { return };
            let (cid, payload) = l2cap::parse_frame(&f).unwrap();
            if cid == l2cap::CID_SIGNALING {
                for (c, id, d) in l2cap::commands(payload) {
                    let allow = |psm: u16| if psm == avrcp::PSM { Ok(()) } else { Err(l2cap::PSM_NOT_SUPPORTED) };
                    let e = self.chans.on_command(c, id, d, &allow).unwrap_or_default();
                    self.chan_events(e);
                }
                return;
            }
            assert_eq!(Some(cid), self.avctp, "only AVCTP on this link");
            self.on_avctp(payload);
            return;
        }
        let (opcode, params) = hci::parse_command(p).expect("a command");
        let params = params.to_vec();
        match opcode {
            hop::READ_LOCAL_VERSION => self.cc(opcode, &[0, 0x0c, 0x00, 0x01, 0x0c, 0x02, 0x00, 0x00, 0x01]),
            hop::READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0xfe, 0x8f, 0xfe, 0xd8, 0x3f, 0x5b, 0x87]),
            hop::READ_BUFFER_SIZE => self.cc(opcode, &[0, 0xfd, 0x03, 0x40, 8, 0x00, 0x00, 0x00]),
            hop::READ_BD_ADDR => { let mut r = vec![0]; r.extend_from_slice(&HOST_ADDR); self.cc(opcode, &r) }
            hop::LE_READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0x59, 0, 0, 0, 0, 0, 0]),
            hop::LE_READ_BUFFER_SIZE => self.cc(opcode, &[0, 0x00, 0x00, 0x00]),
            code::ACCEPT_CONNECTION_REQUEST => {
                assert_eq!(&params[..6], &HEADSET.0);
                self.out.push_back(hci::command_status(opcode, 0));
                let mut r = vec![0, HANDLE as u8, (HANDLE >> 8) as u8];
                r.extend_from_slice(&HEADSET.0);
                r.extend_from_slice(&[0x01, 0x00]);
                self.ev(hci::ev::CONNECTION_COMPLETE, &r);
                if self.early { self.open_avctp(); }
            }
            hop::AUTHENTICATION_REQUESTED => { self.out.push_back(hci::command_status(opcode, 0)); self.ev(hci::ev::LINK_KEY_REQUEST, &HEADSET.0); }
            hop::LINK_KEY_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                assert_eq!(params[6..22], KEY);
                self.ev(code::EV_AUTHENTICATION_COMPLETE, &[0, HANDLE as u8, (HANDLE >> 8) as u8]);
            }
            code::SET_CONNECTION_ENCRYPTION => {
                self.out.push_back(hci::command_status(opcode, 0));
                if self.encrypt {
                    self.encryption(true);
                    if !self.early { self.open_avctp(); }
                }
            }
            _ => self.cc(opcode, &[0]),
        }
    }
}

fn pump(h: &mut Host, c: &mut Controller) -> Vec<Note> {
    let mut notes = vec![];
    for _ in 0..100_000 {
        let mut moved = false;
        while let Some(p) = h.poll() { c.take(&p); moved = true; }
        while let Some(p) = c.out.pop_front() { h.on_packet(&p); moved = true; }
        while let Some(n) = h.note() { notes.push(n); moved = true; }
        if !moved { return notes; }
    }
    panic!("did not settle");
}

/// A host that knows the headset (bonded) and a headset that pages it.
fn paged(c: &mut Controller) -> (Host, Vec<Note>) {
    let mut h = Host::new(Box::new(|b: &mut [u8]| b.fill(7)));
    let n = pump(&mut h, c);
    assert!(matches!(n.first(), Some(Note::Ready(_))), "{n:?}");
    h.set_bonds(Bonds { le: vec![], classic: vec![ssp::Bond { addr: HEADSET, key: KEY, key_type: 0x04 }] });
    let mut req = HEADSET.0.to_vec();
    req.extend_from_slice(&[0x04, 0x04, 0x24, 0x01]);
    c.ev(code::EV_CONNECTION_REQUEST, &req);
    let notes = pump(&mut h, c);
    (h, notes)
}

fn keys(notes: &[Note]) -> Vec<(Key, bool)> {
    notes.iter().filter_map(|n| match n { Note::RemoteKey { handle: HANDLE, key, pressed } => Some((*key, *pressed)), _ => None }).collect()
}

/// (label, ctype) of the responses the host sent.
fn answers(c: &Controller) -> Vec<(u8, u8)> {
    c.responses.iter().map(|p| { let m = avrcp::parse(p).unwrap(); (m.label, m.msg[0]) }).collect()
}

#[test]
fn headset_buttons_become_keys() {
    let mut c = Controller::new();
    let (mut h, notes) = paged(&mut c);
    assert!(notes.contains(&Note::RemoteReady { handle: HANDLE, addr: HEADSET }), "{notes:?}");
    assert!(notes.contains(&Note::RemoteVolume { handle: HANDLE, volume: 64, changed: false }), "{notes:?}");
    for (label, id) in [(1, op::PLAY), (2, op::PAUSE), (3, op::FORWARD), (4, op::BACKWARD), (5, op::STOP)] {
        c.press(label, id, true);
        c.press(label, id, false);
    }
    // Rewind is not a key we take.
    c.press(6, 0x48, true);
    let notes = pump(&mut h, &mut c);
    assert_eq!(keys(&notes), [(Key::PlayPause, true), (Key::PlayPause, false), (Key::PlayPause, true), (Key::PlayPause, false),
        (Key::Next, true), (Key::Next, false), (Key::Previous, true), (Key::Previous, false), (Key::Stop, true), (Key::Stop, false)]);
    let a = answers(&c);
    assert_eq!(a.len(), 11);
    assert!(a[..10].iter().all(|r| r.1 == ctype::ACCEPTED), "{a:?}");
    assert_eq!(a[10], (6, ctype::NOT_IMPLEMENTED));
    // The answer echoes the operation (and its released bit).
    let last_accept = avrcp::parse(&c.responses[9]).unwrap();
    assert_eq!(last_accept.msg, &[ctype::ACCEPTED, opcode::PANEL, opcode::PASS_THROUGH, op::STOP | 0x80, 0]);
}

#[test]
fn absolute_volume_both_ways() {
    let mut c = Controller::new();
    let (mut h, _) = paged(&mut c);
    assert_eq!(h.remote_volume(HANDLE), Some(64));
    let first = c.register.unwrap();
    // The headset's buttons: CHANGED, and we register again (a new label).
    c.changed(80);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::RemoteVolume { handle: HANDLE, volume: 80, changed: true }), "{notes:?}");
    assert_ne!(c.register, Some(first));
    assert_eq!(c.commands.len(), 2, "two registrations");
    // A stale CHANGED (the old label) is ignored.
    let stale = avrcp::packet(first, true, false, avrcp::PID, &avrcp::vendor(ctype::CHANGED, pdu::REGISTER_NOTIFICATION, &[pdu::EVENT_VOLUME_CHANGED, 3]));
    c.avctp_send(&stale);
    assert!(pump(&mut h, &mut c).is_empty());
    assert_eq!(h.remote_volume(HANDLE), Some(80));
    // The system volume goes to the headset; the same value again sends nothing.
    h.set_remote_volume(HANDLE, 100);
    let notes = pump(&mut h, &mut c);
    assert_eq!(c.sets, [100]);
    assert!(notes.contains(&Note::RemoteVolume { handle: HANDLE, volume: 100, changed: false }), "{notes:?}");
    h.set_remote_volume(HANDLE, 100);
    h.set_remote_volume(HANDLE, 80 + 20);
    pump(&mut h, &mut c);
    assert_eq!(c.sets, [100], "no echo");
    h.set_remote_volume(HANDLE, 250);
    pump(&mut h, &mut c);
    assert_eq!(c.sets, [100, 127], "clamped to the AVRCP range");
    // Encryption off: nothing is sent, nothing taken.
    c.encryption(false);
    pump(&mut h, &mut c);
    h.set_remote_volume(HANDLE, 10);
    c.changed(20);
    let notes = pump(&mut h, &mut c);
    assert_eq!(c.sets, [100, 127]);
    assert!(!notes.iter().any(|n| matches!(n, Note::RemoteVolume { .. })), "{notes:?}");
}

#[test]
fn headset_without_absolute_volume_sends_volume_keys() {
    for refusal in [Volume::Refuses, Volume::NoProfile] {
        let mut c = Controller::new();
        c.volume = refusal;
        let (mut h, notes) = paged(&mut c);
        assert!(notes.contains(&Note::RemoteNoVolume { handle: HANDLE }), "{notes:?}");
        assert_eq!(h.remote_volume(HANDLE), None);
        h.set_remote_volume(HANDLE, 50);
        c.press(1, op::VOLUME_UP, true);
        c.press(1, op::VOLUME_UP, false);
        c.press(2, op::VOLUME_DOWN, true);
        c.press(3, op::MUTE, true);
        let notes = pump(&mut h, &mut c);
        assert!(c.sets.is_empty(), "no SetAbsoluteVolume");
        assert_eq!(keys(&notes), [(Key::VolumeUp, true), (Key::VolumeUp, false), (Key::VolumeDown, true), (Key::Mute, true)]);
    }
}

#[test]
fn unencrypted_link_drives_nothing() {
    let mut c = Controller::new();
    c.early = true;
    c.encrypt = false;
    let (mut h, notes) = paged(&mut c);
    // The bonded headset may open the channel, but before encryption nothing counts.
    assert!(c.avctp.is_some());
    assert!(!notes.iter().any(|n| matches!(n, Note::RemoteReady { .. })), "{notes:?}");
    c.press(1, op::PLAY, true);
    c.changed_unregistered(70);
    let notes = pump(&mut h, &mut c);
    assert!(keys(&notes).is_empty() && c.responses.is_empty() && c.commands.is_empty(), "{notes:?}");
    // Encryption on: AVRCP starts, buttons count.
    c.encryption(true);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::RemoteReady { handle: HANDLE, addr: HEADSET }), "{notes:?}");
    assert_eq!(c.commands.len(), 1, "the volume registration");
    c.press(2, op::PLAY, true);
    assert_eq!(keys(&pump(&mut h, &mut c)), [(Key::PlayPause, true)]);
    // Off again: ignored again.
    c.encryption(false);
    c.press(3, op::PLAY, false);
    assert!(keys(&pump(&mut h, &mut c)).is_empty());
}

impl Controller {
    /// A CHANGED on a label the host never used.
    fn changed_unregistered(&mut self, volume: u8) {
        self.avctp_send(&avrcp::packet(9, true, false, avrcp::PID, &avrcp::vendor(ctype::CHANGED, pdu::REGISTER_NOTIFICATION, &[pdu::EVENT_VOLUME_CHANGED, volume])));
    }
}

#[test]
fn hostile_avctp_gets_bounded_answers() {
    let mut c = Controller::new();
    let (mut h, _) = paged(&mut c);
    let pid = avrcp::PID.to_be_bytes();
    let hostile: Vec<Vec<u8>> = vec![
        vec![],
        vec![0x10],
        vec![0x10, pid[0], pid[1]],
        // AVCTP fragments: start (with a packet count), continue, end.
        vec![0x14, 9, pid[0], pid[1], 0, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 0],
        vec![0x18, 0, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 0],
        vec![0x1C, 0, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 0],
        // PASS THROUGH whose operand length runs past the end, or cut short.
        vec![0x20, pid[0], pid[1], 0, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 200],
        vec![0x30, pid[0], pid[1], 0, opcode::PANEL, opcode::PASS_THROUGH],
        // PASS THROUGH as STATUS, or to the unit: not a button.
        vec![0x40, pid[0], pid[1], ctype::STATUS, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 0],
        vec![0x50, pid[0], pid[1], 0, opcode::UNIT, opcode::PASS_THROUGH, op::PLAY, 0],
        // Vendor dependent: lying parameter length, another company, AVRCP fragments.
        vec![0x60, pid[0], pid[1], ctype::STATUS, opcode::PANEL, 0, 0x00, 0x19, 0x58, pdu::GET_CAPABILITIES, 0, 0xFF, 0xFF, 2],
        vec![0x70, pid[0], pid[1], ctype::STATUS, opcode::PANEL, 0, 0x00, 0x11, 0x22, pdu::GET_CAPABILITIES, 0, 0, 1, 2],
        vec![0x80, pid[0], pid[1], ctype::STATUS, opcode::PANEL, 0, 0x00, 0x19, 0x58, pdu::GET_CAPABILITIES, 1, 0, 1, 2],
        // GetCapabilities: company ids, events, nonsense.
        avrcp::packet(9, false, false, avrcp::PID, &avrcp::vendor(ctype::STATUS, pdu::GET_CAPABILITIES, &[pdu::CAP_COMPANY_ID])),
        avrcp::packet(10, false, false, avrcp::PID, &avrcp::vendor(ctype::STATUS, pdu::GET_CAPABILITIES, &[pdu::CAP_EVENTS])),
        avrcp::packet(11, false, false, avrcp::PID, &avrcp::vendor(ctype::STATUS, pdu::GET_CAPABILITIES, &[])),
        avrcp::packet(12, false, false, avrcp::PID, &avrcp::vendor(ctype::NOTIFY, pdu::REGISTER_NOTIFICATION, &[1, 0, 0, 0, 0])),
        avrcp::packet(13, false, false, avrcp::PID, &avrcp::vendor(ctype::CONTROL, 0x77, &[])),
        // Another profile id.
        avrcp::packet(14, false, false, 0x1234, &[0, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 0]),
        // Unit and subunit info.
        avrcp::packet(15, false, false, avrcp::PID, &[ctype::STATUS, opcode::UNIT, opcode::UNIT_INFO, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        // Too long to echo.
        avrcp::packet(1, false, false, avrcp::PID, &[0u8; avrcp::MAX_REPLY + 1]),
        // Responses to nothing.
        avrcp::packet(5, true, false, avrcp::PID, &avrcp::vendor(ctype::ACCEPTED, pdu::SET_ABSOLUTE_VOLUME, &[]),),
        avrcp::packet(5, true, true, avrcp::PID, &[]),
    ];
    for p in &hostile { c.avctp_send(p); }
    let notes = pump(&mut h, &mut c);
    assert!(keys(&notes).is_empty(), "{notes:?}");
    assert!(!notes.iter().any(|n| matches!(n, Note::RemoteVolume { .. } | Note::RemoteNoVolume { .. } | Note::RemoteClosed { .. })), "{notes:?}");
    let a: Vec<(u8, Vec<u8>)> = c.responses.iter().map(|p| (p[0], p[3..].to_vec())).collect();
    let vendor = |c: u8, id: u8, p: &[u8]| avrcp::vendor(c, id, p);
    assert_eq!(a, [
        // A malformed PASS THROUGH is no button: refused like any other frame.
        (0x20 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 200]),
        (0x30 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::PANEL, opcode::PASS_THROUGH]),
        (0x40 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::PANEL, opcode::PASS_THROUGH, op::PLAY, 0]),
        (0x50 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::UNIT, opcode::PASS_THROUGH, op::PLAY, 0]),
        (0x60 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::PANEL, 0, 0x00, 0x19, 0x58, pdu::GET_CAPABILITIES, 0, 0xFF, 0xFF, 2]),
        (0x70 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::PANEL, 0, 0x00, 0x11, 0x22, pdu::GET_CAPABILITIES, 0, 0, 1, 2]),
        (0x80 | 2, vec![ctype::NOT_IMPLEMENTED, opcode::PANEL, 0, 0x00, 0x19, 0x58, pdu::GET_CAPABILITIES, 1, 0, 1, 2]),
        (0x90 | 2, vendor(ctype::STABLE, pdu::GET_CAPABILITIES, &[pdu::CAP_COMPANY_ID, 1, 0x00, 0x19, 0x58])),
        (0xA0 | 2, vendor(ctype::STABLE, pdu::GET_CAPABILITIES, &[pdu::CAP_EVENTS, 0])),
        (0xB0 | 2, vendor(ctype::REJECTED, pdu::GET_CAPABILITIES, &[pdu::ERR_INVALID_PARAMETER])),
        (0xC0 | 2, vendor(ctype::REJECTED, pdu::REGISTER_NOTIFICATION, &[pdu::ERR_INVALID_PARAMETER])),
        (0xD0 | 2, vendor(ctype::REJECTED, 0x77, &[pdu::ERR_INVALID_COMMAND])),
        (0xE0 | 3, vec![]),
        (0xF0 | 2, vec![ctype::STABLE, opcode::UNIT, opcode::UNIT_INFO, 0x07, opcode::PANEL, 0xFF, 0xFF, 0xFF]),
    ]);
    // The profile id comes back as sent.
    assert_eq!(&c.responses[12][1..3], &0x1234u16.to_be_bytes());
    // Still working.
    assert_eq!(h.remote_volume(HANDLE), Some(64));
    c.press(1, op::STOP, true);
    assert_eq!(keys(&pump(&mut h, &mut c)), [(Key::Stop, true)]);
}

#[test]
fn random_avctp_never_panics_and_answers_at_most_once() {
    let mut seed = 0x2545_f491u32;
    let mut next = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed };
    let mut r = Remote::new();
    let _ = r.start();
    for i in 0..200_000 {
        let len = (next() % 40) as usize;
        let mut b: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        // Bias towards plausible frames: single packets, our profile, AVRCP's company.
        if len > 3 && next() % 2 == 0 { b[0] &= 0xF3; b[1] = 0x11; b[2] = 0x0E; }
        if len > 10 && next() % 2 == 0 { b[5] = 0; b[6] = 0x00; b[7] = 0x19; b[8] = 0x58; b[10] &= 0xFC; }
        if len > 12 && next() % 2 == 0 { b[9] = [pdu::REGISTER_NOTIFICATION, pdu::SET_ABSOLUTE_VOLUME, pdu::GET_CAPABILITIES][(next() % 3) as usize]; b[11] = 0; b[12] = (len - 13) as u8; }
        let out = r.on_packet(&b);
        let sends: Vec<&Vec<u8>> = out.iter().filter_map(|o| match o { avrcp::Out::Send(p) => Some(p), _ => None }).collect();
        assert!(sends.len() <= 1 && out.len() <= 3, "{b:?} -> {out:?}");
        for s in sends { assert!(s.len() <= 3 + avrcp::MAX_REPLY, "bounded answer"); }
        for o in &out { if let avrcp::Out::Volume { volume, .. } = o { assert!(*volume <= avrcp::MAX_VOLUME); } }
        let _ = avrcp::parse(&b);
        let _ = avrcp::parse_vendor(&b);
        if i % 1000 == 0 { let _ = r.set_volume(next() as u8); }
        if next() % 256 == 0 { r = Remote::new(); let _ = r.start(); }
    }
}

#[test]
fn random_avctp_through_the_host() {
    let mut c = Controller::new();
    let (mut h, _) = paged(&mut c);
    let mut seed = 0x1234_5678u32;
    let mut next = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed };
    for _ in 0..3_000 {
        let len = (next() % 48) as usize;
        let mut b: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if len > 3 { b[0] &= 0xF1; b[1] = 0x11; b[2] = 0x0E; } // commands only: the fake answers none
        c.avctp_send(&b);
        let _ = pump(&mut h, &mut c);
    }
    assert!(c.responses.len() <= 3_000);
    assert!(c.avctp.is_some(), "the channel survives");
}

#[test]
fn sdp_lists_avrcp_target_and_controller() {
    let records = [sdp::a2dp_source_record(), sdp::avrcp_target_record(), sdp::avrcp_controller_record()];
    // A search for the profile (0x110E) finds both; each role's own class finds one.
    for (class, count, features) in [(sdp::uuid::AV_REMOTE_CONTROL_TARGET, 1, 0x0001u64), (sdp::uuid::AV_REMOTE_CONTROL_CONTROLLER, 1, 0x0002),
        (sdp::uuid::AV_REMOTE_CONTROL, 2, 0x0001)] {
        let mut client = sdp::Client::new(class, &[0x0000, 0x0001, 0x0004, 0x0009, 0x0311], 64);
        let mut req = client.start();
        let found = loop {
            match client.on_pdu(&sdp::serve(&req, &records)) {
                sdp::Progress::Send(next) => req = next,
                sdp::Progress::Done(r) => break r,
                sdp::Progress::Failed(e) => panic!("failed {e:#x}"),
            }
        };
        assert_eq!(found.len(), count, "{class:#x}");
        assert_eq!(found[0].protocol(sdp::uuid::AVCTP), Some((avrcp::PSM, 0x0104)));
        assert_eq!(found[0].profile_version(sdp::uuid::AV_REMOTE_CONTROL), Some(0x0104));
        assert_eq!(found[0].get(sdp::attr::SUPPORTED_FEATURES).and_then(|e| e.as_uint()), Some(features));
    }
}
