// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The host stack in one place: bring-up, discovery, connections, L2CAP fixed channels,
//! LE Secure Connections and classic Secure Simple Pairing, driven by packets in and
//! packets out. `servers/cufflinks` feeds it the ring from SexUSB and prints its notes;
//! the host tests feed it a scripted controller.

use crate::controller::{self, Bringup, Info};
use crate::hci::{self, op, Addr, Event};
use crate::l2cap::{self, Reassembly, Signal};
use crate::report::{self, Found};
use crate::smp::{self, Action, Role, Smp};
use crate::ssp::{Ssp, Step};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// Something the server should report (boot log lines, desktop prompts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    Ready(Info),
    /// A device seen for the first time, or seen again with its name.
    Device(Found),
    Connected { handle: u16, addr: Addr, le: bool },
    Disconnected { handle: u16, reason: u8 },
    /// Ask the user whether this six-digit value matches the other device's screen.
    Compare { handle: u16, addr: Addr, value: u32 },
    /// LE: pairing finished with this long-term key; classic: a link key was stored.
    Paired { addr: Addr, le: bool },
    PairingFailed { addr: Addr, reason: u8 },
    Encrypted { handle: u16 },
    /// A command the controller refused.
    Refused { opcode: u16, status: u8 },
    Failed(controller::Error),
}

struct Conn {
    handle: u16,
    addr: Addr,
    addr_type: u8,
    le: bool,
    central: bool,
    rx: Reassembly,
    smp: Option<Smp>,
    ltk: Option<[u8; 16]>,
}

pub struct Host {
    pub bringup: Bringup,
    pub ssp: Ssp,
    /// LE IO capability and AuthReq for SMP.
    pub io: u8,
    pub auth: u8,
    commands: VecDeque<Vec<u8>>,
    acl: VecDeque<Vec<u8>>,
    credits: u8,
    notes: VecDeque<Note>,
    conns: Vec<Conn>,
    seen: Vec<Found>,
    rng: Box<dyn FnMut(&mut [u8])>,
    ready_noted: bool,
}

impl Host {
    /// `rng` fills buffers with random bytes (private keys, nonces).
    pub fn new(rng: Box<dyn FnMut(&mut [u8])>) -> Host {
        Host { bringup: Bringup::new(), ssp: Ssp::new(smp::io::DISPLAY_YES_NO, 0x03), io: smp::io::DISPLAY_YES_NO,
            auth: smp::AUTH_BONDING | smp::AUTH_MITM, commands: VecDeque::new(), acl: VecDeque::new(), credits: 1,
            notes: VecDeque::new(), conns: Vec::new(), seen: Vec::new(), rng, ready_noted: false }
    }

    pub fn ready(&self) -> bool { self.bringup.ready() }
    pub fn info(&self) -> &Info { &self.bringup.info }
    pub fn devices(&self) -> &[Found] { &self.seen }
    pub fn note(&mut self) -> Option<Note> { self.notes.pop_front() }

    /// The next packet (with its type byte) to send to the controller.
    pub fn poll(&mut self) -> Option<Vec<u8>> {
        if let Some(a) = self.acl.pop_front() { return Some(a); }
        if self.credits == 0 { return None; }
        let c = if !self.bringup.ready() { self.bringup.next() } else { self.commands.pop_front() };
        if c.is_some() { self.credits -= 1; }
        c
    }

    fn send_command(&mut self, c: Vec<u8>) { self.commands.push_back(c); }

    /// Start discovery: classic inquiry (`length` x 1.28 s) and an LE active scan.
    pub fn scan(&mut self, length: u8) {
        for c in controller::start_scan(&self.bringup.info, length) { self.send_command(c); }
    }
    pub fn stop_scan(&mut self) {
        for c in controller::stop_scan(&self.bringup.info) { self.send_command(c); }
    }

    /// Connect to an LE device (public or random address).
    pub fn connect_le(&mut self, addr: Addr, addr_type: u8) {
        let mut p = Vec::new();
        p.extend_from_slice(&0x0060u16.to_le_bytes()); // scan interval
        p.extend_from_slice(&0x0030u16.to_le_bytes()); // scan window
        p.push(0); // no filter accept list
        p.push(addr_type);
        p.extend_from_slice(&addr.0);
        p.push(0); // own address public
        p.extend_from_slice(&0x0018u16.to_le_bytes()); // interval min 30 ms
        p.extend_from_slice(&0x0028u16.to_le_bytes()); // interval max 50 ms
        p.extend_from_slice(&0u16.to_le_bytes()); // latency
        p.extend_from_slice(&0x01F4u16.to_le_bytes()); // supervision 5 s
        p.extend_from_slice(&[0, 0, 0, 0]); // CE lengths
        self.send_command(hci::command(op::LE_CREATE_CONNECTION, &p));
    }

    fn conn(&mut self, handle: u16) -> Option<&mut Conn> { self.conns.iter_mut().find(|c| c.handle == handle) }

    fn new_smp(&mut self, handle: u16, role: Role) -> Option<Smp> {
        let own = self.bringup.info.addr;
        let c = self.conns.iter().find(|c| c.handle == handle)?;
        let (mut local, mut remote) = ([0u8; 7], [0u8; 7]);
        local[..6].copy_from_slice(&own.0);
        remote[..6].copy_from_slice(&c.addr.0);
        remote[6] = c.addr_type;
        for _ in 0..8 {
            let mut secret = [0u8; 32];
            let mut nonce = [0u8; 16];
            (self.rng)(&mut secret);
            (self.rng)(&mut nonce);
            if let Some(s) = Smp::new(role, self.io, self.auth, local, remote, &secret, nonce) { return Some(s); }
        }
        None
    }

    /// Pair an LE connection (we start as initiator), or a classic one (authentication).
    pub fn pair(&mut self, handle: u16) {
        let Some(c) = self.conns.iter().find(|c| c.handle == handle) else { return };
        if !c.le { self.send_command(Ssp::authenticate(handle)); return; }
        let Some(mut s) = self.new_smp(handle, Role::Initiator) else { return };
        let actions = s.start();
        if let Some(c) = self.conn(handle) { c.smp = Some(s); }
        self.smp_actions(handle, actions);
    }

    /// The user's answer to `Note::Compare`.
    pub fn confirm(&mut self, handle: u16, matches: bool) {
        let Some(c) = self.conns.iter_mut().find(|c| c.handle == handle) else { return };
        if c.le {
            let actions = c.smp.as_mut().map(|s| s.confirm(matches)).unwrap_or_default();
            self.smp_actions(handle, actions);
        } else {
            let addr = c.addr;
            let cmd = self.ssp.confirm(addr, matches);
            self.send_command(cmd);
        }
    }

    fn send_l2cap(&mut self, handle: u16, cid: u16, payload: &[u8]) {
        let mtu = self.bringup.info.le_acl_mtu.max(23) as usize;
        for p in l2cap::fragment(handle, &l2cap::frame(cid, payload), mtu) { self.acl.push_back(p); }
    }

    fn smp_actions(&mut self, handle: u16, actions: Vec<Action>) {
        let Some(addr) = self.conns.iter().find(|c| c.handle == handle).map(|c| c.addr) else { return };
        for a in actions {
            match a {
                Action::Send(pdu) => self.send_l2cap(handle, l2cap::CID_SMP, &pdu),
                Action::Compare(value) => self.notes.push_back(Note::Compare { handle, addr, value }),
                Action::Encrypt(ltk) => {
                    if let Some(c) = self.conn(handle) { c.ltk = Some(ltk); }
                    let mut p = handle.to_le_bytes().to_vec();
                    p.extend_from_slice(&[0; 10]); // rand and ediv are zero for LE SC
                    p.extend_from_slice(&ltk);
                    self.send_command(hci::command(op::LE_START_ENCRYPTION, &p));
                    self.notes.push_back(Note::Paired { addr, le: true });
                }
                Action::Ltk(ltk) => {
                    if let Some(c) = self.conn(handle) { c.ltk = Some(ltk); }
                    self.notes.push_back(Note::Paired { addr, le: true });
                }
                Action::Failed(reason) => self.notes.push_back(Note::PairingFailed { addr, reason }),
            }
        }
    }

    fn found(&mut self, f: Found) {
        match self.seen.iter_mut().find(|s| s.addr == f.addr) {
            Some(s) => {
                if s.name.is_none() && f.name.is_some() {
                    s.name = f.name.clone();
                    s.rssi = f.rssi.or(s.rssi);
                    let s = s.clone();
                    self.notes.push_back(Note::Device(s));
                }
            }
            None => {
                self.seen.push(f.clone());
                self.notes.push_back(Note::Device(f));
            }
        }
    }

    /// One packet (with its type byte) from the controller.
    pub fn on_packet(&mut self, p: &[u8]) {
        if let Some((handle, pb, data)) = hci::parse_acl(p) { self.on_acl(handle, pb, data); return; }
        let Some(e) = hci::parse_event(p) else { return };
        match e {
            // One command outstanding at a time. Some Intel parts report ncmd 0 on their
            // first completion, so any completion frees the slot.
            Event::CommandComplete { .. } | Event::CommandStatus { .. } => self.credits = 1,
            _ => {}
        }
        if !self.bringup.ready() {
            match self.bringup.on_event(&e) {
                Ok(true) if !self.ready_noted => { self.ready_noted = true; self.notes.push_back(Note::Ready(self.bringup.info)); }
                Err(err) => self.notes.push_back(Note::Failed(err)),
                _ => {}
            }
            return;
        }
        for step in self.ssp.on_event(&e) {
            match step {
                Step::Command(c) => self.send_command(c),
                Step::Compare { addr, value } => {
                    let handle = self.conns.iter().find(|c| c.addr == addr && !c.le).map(|c| c.handle).unwrap_or(0);
                    self.notes.push_back(Note::Compare { handle, addr, value });
                }
                Step::Bonded(b) => self.notes.push_back(Note::Paired { addr: b.addr, le: false }),
                Step::Complete { addr, status } if status != 0 => self.notes.push_back(Note::PairingFailed { addr, reason: status }),
                Step::Complete { .. } => {}
            }
        }
        match e {
            Event::CommandStatus { status, opcode, .. } if status != 0 => self.notes.push_back(Note::Refused { opcode, status }),
            Event::CommandComplete { opcode, ret, .. } if ret.first().is_some_and(|&s| s != 0) && opcode != op::INQUIRY_CANCEL => {
                self.notes.push_back(Note::Refused { opcode, status: ret[0] })
            }
            Event::Report { code, sub, params } => for f in report::reports(code, sub, params) { self.found(f); },
            Event::RemoteNameComplete { status: 0, addr, name } => {
                let mut f = Found { addr, transport: report::Transport::Classic, addr_type: 0, rssi: None, class: None,
                    name: Some(alloc::string::String::from_utf8_lossy(name).into_owned()), appearance: None, flags: None, connectable: true };
                if let Some(s) = self.seen.iter().find(|s| s.addr == addr) { f.rssi = s.rssi; f.class = s.class; }
                self.found(f);
            }
            Event::ConnectionComplete { status: 0, handle, addr, .. } => {
                self.conns.push(Conn { handle, addr, addr_type: 0, le: false, central: true, rx: Reassembly::default(), smp: None, ltk: None });
                self.notes.push_back(Note::Connected { handle, addr, le: false });
            }
            Event::LeConnectionComplete { status: 0, handle, role, peer_type, peer } => {
                self.conns.push(Conn { handle, addr: peer, addr_type: peer_type & 1, le: true, central: role == 0, rx: Reassembly::default(), smp: None, ltk: None });
                self.notes.push_back(Note::Connected { handle, addr: peer, le: true });
            }
            Event::DisconnectionComplete { status: 0, handle, reason } => {
                self.conns.retain(|c| c.handle != handle);
                self.notes.push_back(Note::Disconnected { handle, reason });
            }
            Event::EncryptionChange { status: 0, handle, enabled } if enabled != 0 => self.notes.push_back(Note::Encrypted { handle }),
            Event::LeLtkRequest { handle, .. } => {
                let ltk = self.conns.iter().find(|c| c.handle == handle).and_then(|c| c.ltk);
                let mut p = handle.to_le_bytes().to_vec();
                match ltk {
                    Some(k) => { p.extend_from_slice(&k); self.send_command(hci::command(op::LE_LTK_REQUEST_REPLY, &p)); }
                    None => self.send_command(hci::command(op::LE_LTK_REQUEST_NEGATIVE_REPLY, &p)),
                }
            }
            _ => {}
        }
    }

    fn on_acl(&mut self, handle: u16, pb: u8, data: &[u8]) {
        let Some(c) = self.conn(handle) else { return };
        let Some(frame) = c.rx.push(pb, data) else { return };
        let Some((cid, payload)) = l2cap::parse_frame(&frame) else { return };
        match cid {
            l2cap::CID_SMP => {
                let (central, has) = self.conns.iter().find(|c| c.handle == handle).map(|c| (c.central, c.smp.is_some())).unwrap_or((true, false));
                if !has {
                    match payload.first() {
                        // The peripheral asks us (central) to pair: start as initiator.
                        Some(&smp::code::SECURITY_REQUEST) if central => { self.pair(handle); return; }
                        Some(&smp::code::PAIRING_REQUEST) if !central => {
                            let s = self.new_smp(handle, Role::Responder);
                            if let Some(c) = self.conn(handle) { c.smp = s; }
                        }
                        _ => {
                            // Not pairing and not asked to: refuse.
                            self.send_l2cap(handle, l2cap::CID_SMP, &[smp::code::PAIRING_FAILED, smp::reason::PAIRING_NOT_SUPPORTED]);
                            return;
                        }
                    }
                }
                let actions = self.conn(handle).and_then(|c| c.smp.as_mut()).map(|s| s.on_pdu(payload)).unwrap_or_default();
                self.smp_actions(handle, actions);
            }
            l2cap::CID_SIGNALING | l2cap::CID_LE_SIGNALING => match l2cap::signal(cid, payload) {
                Signal::Reply(r) => self.send_l2cap(handle, cid, &r),
                Signal::Accepted { reply, .. } => self.send_l2cap(handle, cid, &reply),
                Signal::Nothing => {}
            },
            // ATT: no GATT server until W2; answer every request with "request not supported".
            l2cap::CID_ATT => {
                if let Some(&opcode) = payload.first() {
                    let is_request = opcode & 0x40 == 0 && opcode & 1 == 0 && opcode != 0x1E;
                    if opcode == 0x02 {
                        self.send_l2cap(handle, cid, &[0x03, 23, 0]); // Exchange MTU: stay at 23
                    } else if is_request {
                        let handle_in = if payload.len() >= 3 { [payload[1], payload[2]] } else { [0, 0] };
                        self.send_l2cap(handle, cid, &[0x01, opcode, handle_in[0], handle_in[1], 0x06]);
                    }
                }
            }
            _ => {}
        }
    }
}
