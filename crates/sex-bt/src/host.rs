// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The host stack in one place: bring-up, discovery, connections, L2CAP fixed and
//! dynamic channels, LE Secure Connections and classic Secure Simple Pairing, bonds, and
//! HID input (classic HID on PSM 0x11/0x13, HID over GATT on LE), driven by packets in
//! and packets out. `servers/cufflinks` feeds it the ring from SexUSB and acts on its
//! notes; the host tests feed it a scripted controller.
//!
//! HID devices: [`Host::connect_hid`] connects a keyboard or mouse found by scanning
//! (pairing on the way); bonded devices are reconnected when they advertise (LE) or let
//! in when they page us (classic). Input reports come out as [`Note::Input`].

use crate::att;
use crate::bonds::{Bonds, LeBond};
use crate::controller::{self, Bringup, Info};
use crate::gatt::{self, Hogp};
use crate::hci::{self, op, Addr, Event};
use crate::hid::Input;
use crate::hidp;
use crate::l2cap::{self, ChanEvent, Channels, Reassembly, Signal};
use crate::report::{self, Found, Transport};
use crate::smp::{self, Action, Role, Smp};
use crate::ssp::{Ssp, Step};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// HCI codes this module adds to `hci`.
pub mod code {
    pub const ACCEPT_CONNECTION_REQUEST: u16 = 0x0409;
    pub const REJECT_CONNECTION_REQUEST: u16 = 0x040A;
    pub const SET_CONNECTION_ENCRYPTION: u16 = 0x0413;
    pub const DISCONNECT: u16 = 0x0406;
    pub const EV_CONNECTION_REQUEST: u8 = 0x04;
    pub const EV_AUTHENTICATION_COMPLETE: u8 = 0x06;
}

/// Something the server should report or act on (boot log lines, desktop prompts, input).
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
    /// The bonds changed (a new key, an identity, or one forgotten): save `Host::bonds`.
    BondsChanged,
    /// A keyboard or mouse is ready to send input.
    HidReady { handle: u16, addr: Addr, le: bool, keyboard: bool, pointer: bool, boot: bool },
    /// HID setup stopped (no HID service, refused attribute: the ATT error code).
    HidFailed { handle: u16, code: u8 },
    /// One input report.
    Input { handle: u16, input: Input },
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
    encrypted: bool,
    /// We want HID on this link (we connected it for HID, or it is a bonded HID device).
    hid: bool,
    chans: Channels,
    gatt: Option<Hogp>,
    hid_ready: bool,
}

impl Conn {
    fn new(handle: u16, addr: Addr, addr_type: u8, le: bool, central: bool, hid: bool) -> Conn {
        Conn { handle, addr, addr_type, le, central, rx: Reassembly::default(), smp: None, ltk: None, encrypted: false, hid,
            chans: Channels::default(), gatt: None, hid_ready: false }
    }
}

pub struct Host {
    pub bringup: Bringup,
    pub ssp: Ssp,
    /// LE IO capability and AuthReq for SMP.
    pub io: u8,
    pub auth: u8,
    /// Paired devices; load them at start (`set_bonds`), save them on `Note::BondsChanged`.
    pub bonds: Bonds,
    commands: VecDeque<Vec<u8>>,
    acl: VecDeque<Vec<u8>>,
    credits: u8,
    notes: VecDeque<Note>,
    conns: Vec<Conn>,
    seen: Vec<Found>,
    rng: Box<dyn FnMut(&mut [u8])>,
    ready_noted: bool,
    /// Devices we are connecting for HID (classic page or LE create connection sent).
    wanted: Vec<Addr>,
    /// The LE connection being created (one at a time).
    le_connecting: Option<Addr>,
}

impl Host {
    /// `rng` fills buffers with random bytes (private keys, nonces).
    pub fn new(rng: Box<dyn FnMut(&mut [u8])>) -> Host {
        Host { bringup: Bringup::new(), ssp: Ssp::new(smp::io::DISPLAY_YES_NO, 0x03), io: smp::io::DISPLAY_YES_NO,
            auth: smp::AUTH_BONDING | smp::AUTH_MITM, bonds: Bonds::default(), commands: VecDeque::new(), acl: VecDeque::new(), credits: 1,
            notes: VecDeque::new(), conns: Vec::new(), seen: Vec::new(), rng, ready_noted: false, wanted: Vec::new(), le_connecting: None }
    }

    pub fn ready(&self) -> bool { self.bringup.ready() }
    pub fn info(&self) -> &Info { &self.bringup.info }
    pub fn devices(&self) -> &[Found] { &self.seen }
    pub fn note(&mut self) -> Option<Note> { self.notes.pop_front() }

    /// Bonds read back from storage.
    pub fn set_bonds(&mut self, bonds: Bonds) {
        self.ssp.bonds = bonds.classic.clone();
        self.bonds = bonds;
    }

    /// Forget a device's keys (and drop its link).
    pub fn forget(&mut self, addr: Addr) {
        self.ssp.bonds.retain(|b| b.addr != addr);
        if self.bonds.forget(&addr) { self.notes.push_back(Note::BondsChanged); }
        let handles: Vec<u16> = self.conns.iter().filter(|c| c.addr == addr).map(|c| c.handle).collect();
        for h in handles { self.disconnect(h); }
    }

    pub fn disconnect(&mut self, handle: u16) {
        let mut p = handle.to_le_bytes().to_vec();
        p.push(0x13); // remote user terminated
        self.send_command(hci::command(code::DISCONNECT, &p));
    }

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
    /// Restart the LE scan only (the controller's duplicate filter forgets what it saw),
    /// so a bonded device that went away is seen, and reconnected, when it comes back.
    pub fn restart_le_scan(&mut self) {
        let classic = |c: &Vec<u8>| hci::parse_command(c).is_some_and(|(o, _)| o == op::INQUIRY || o == op::INQUIRY_CANCEL);
        let stop = controller::stop_scan(&self.bringup.info);
        let start = controller::start_scan(&self.bringup.info, 1);
        for c in stop.into_iter().chain(start).filter(|c| !classic(c)) { self.send_command(c); }
    }

    /// Connect to an LE device (public or random address).
    pub fn connect_le(&mut self, addr: Addr, addr_type: u8) {
        self.le_connecting = Some(addr);
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

    /// Page a classic device.
    pub fn connect_classic(&mut self, addr: Addr) {
        let mut p = addr.0.to_vec();
        p.extend_from_slice(&0xCC18u16.to_le_bytes()); // DM1..DH5
        p.extend_from_slice(&[0x01, 0x00]); // page scan repetition R1, reserved
        p.extend_from_slice(&0u16.to_le_bytes()); // clock offset unknown
        p.push(0x01); // allow role switch
        self.send_command(hci::command(op::CREATE_CONNECTION, &p));
    }

    /// Connect a keyboard or mouse found by scanning, pair if needed and start its input.
    pub fn connect_hid(&mut self, f: &Found) {
        if self.conns.iter().any(|c| c.addr == f.addr) || self.wanted.contains(&f.addr) { return; }
        self.wanted.push(f.addr);
        if !self.seen.iter().any(|s| s.addr == f.addr) { self.seen.push(f.clone()); }
        match f.transport {
            Transport::Le => { if self.le_connecting.is_none() { self.connect_le(f.addr, f.addr_type); } }
            Transport::Classic => self.connect_classic(f.addr),
        }
    }

    /// Whether the device looks like a keyboard or a mouse (class of device or appearance).
    pub fn is_hid(f: &Found) -> bool {
        let class = f.class.map(|c| (c >> 8) & 0x1F == 0x05).unwrap_or(false);
        let appearance = f.appearance.map(|a| (0x03C0..=0x03C2).contains(&a)).unwrap_or(false);
        class || appearance
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
        let le = self.conns.iter().find(|c| c.handle == handle).map(|c| c.le).unwrap_or(true);
        let mtu = if le { self.bringup.info.le_acl_mtu.max(23) } else { self.bringup.info.acl_mtu.max(23) } as usize;
        for p in l2cap::fragment(handle, &l2cap::frame(cid, payload), mtu) { self.acl.push_back(p); }
    }

    fn start_encryption(&mut self, handle: u16, ltk: [u8; 16]) {
        if let Some(c) = self.conn(handle) { c.ltk = Some(ltk); }
        let mut p = handle.to_le_bytes().to_vec();
        p.extend_from_slice(&[0; 10]); // rand and ediv are zero for LE SC
        p.extend_from_slice(&ltk);
        self.send_command(hci::command(op::LE_START_ENCRYPTION, &p));
    }

    fn le_bonded(&mut self, handle: u16, ltk: [u8; 16]) {
        let Some(c) = self.conns.iter().find(|c| c.handle == handle) else { return };
        let b = LeBond { addr: c.addr, addr_type: c.addr_type, ltk, irk: None, identity: None };
        if self.bonds.put_le(b) { self.notes.push_back(Note::BondsChanged); }
    }

    fn smp_actions(&mut self, handle: u16, actions: Vec<Action>) {
        let Some(addr) = self.conns.iter().find(|c| c.handle == handle).map(|c| c.addr) else { return };
        for a in actions {
            match a {
                Action::Send(pdu) => self.send_l2cap(handle, l2cap::CID_SMP, &pdu),
                Action::Compare(value) => self.notes.push_back(Note::Compare { handle, addr, value }),
                Action::Encrypt(ltk) => {
                    self.start_encryption(handle, ltk);
                    self.notes.push_back(Note::Paired { addr, le: true });
                    self.le_bonded(handle, ltk);
                }
                Action::Ltk(ltk) => {
                    if let Some(c) = self.conn(handle) { c.ltk = Some(ltk); }
                    self.notes.push_back(Note::Paired { addr, le: true });
                    self.le_bonded(handle, ltk);
                }
                Action::Failed(reason) => self.notes.push_back(Note::PairingFailed { addr, reason }),
                Action::Identity { irk, addr_type, addr: id } => {
                    if let Some(b) = self.bonds.le.iter_mut().find(|b| b.addr == addr) {
                        b.irk = Some(irk);
                        b.identity = Some((addr_type, Addr(id)));
                        self.notes.push_back(Note::BondsChanged);
                    }
                }
            }
        }
    }

    fn found(&mut self, f: Found) {
        // A bonded LE device advertising: connect it back.
        if f.transport == Transport::Le && f.connectable && self.le_connecting.is_none()
            && !self.conns.iter().any(|c| c.addr == f.addr) && self.bonds.le_for(&f.addr).is_some() {
            self.wanted.retain(|a| *a != f.addr);
            self.wanted.push(f.addr);
            self.connect_le(f.addr, f.addr_type);
        }
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

    /// Start the next LE device that waited while another connection was pending.
    fn next_le(&mut self) {
        if self.le_connecting.is_some() { return; }
        let next = self.wanted.iter()
            .filter(|a| !self.conns.iter().any(|c| c.addr == **a))
            .find_map(|a| self.seen.iter().find(|s| s.addr == *a && s.transport == Transport::Le).map(|s| (s.addr, s.addr_type)));
        if let Some((addr, addr_type)) = next { self.connect_le(addr, addr_type); }
    }

    fn hid_wanted(&self, addr: &Addr) -> bool {
        self.wanted.contains(addr) || self.bonds.le_for(addr).is_some() || self.bonds.classic_for(addr).is_some()
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
                Step::Bonded(b) => {
                    self.notes.push_back(Note::Paired { addr: b.addr, le: false });
                    if self.bonds.put_classic(b) { self.notes.push_back(Note::BondsChanged); }
                }
                Step::Complete { addr, status } if status != 0 => self.notes.push_back(Note::PairingFailed { addr, reason: status }),
                Step::Complete { .. } => {}
            }
        }
        match e {
            Event::CommandStatus { status, opcode, .. } if status != 0 => {
                if opcode == op::LE_CREATE_CONNECTION { self.le_connecting = None; }
                if opcode == op::CREATE_CONNECTION || opcode == op::LE_CREATE_CONNECTION { self.wanted.clear(); }
                self.notes.push_back(Note::Refused { opcode, status })
            }
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
            Event::ConnectionComplete { status, handle, addr, .. } => {
                if status != 0 { self.wanted.retain(|a| *a != addr); return; }
                let ours = self.wanted.contains(&addr);
                let hid = self.hid_wanted(&addr);
                self.conns.push(Conn::new(handle, addr, 0, false, ours, hid));
                self.notes.push_back(Note::Connected { handle, addr, le: false });
                // We paged it for HID, or a bonded device paged us: authenticate (proving it
                // holds the link key), then encrypt, then open or accept the channels.
                if ours || self.bonds.classic_for(&addr).is_some() { self.send_command(Ssp::authenticate(handle)); }
            }
            Event::LeConnectionComplete { status, handle, role, peer_type, peer } => {
                self.le_connecting = None;
                if status != 0 { self.wanted.retain(|a| *a != peer); self.next_le(); return; }
                let hid = self.hid_wanted(&peer);
                self.conns.push(Conn::new(handle, peer, peer_type & 1, true, role == 0, hid));
                self.notes.push_back(Note::Connected { handle, addr: peer, le: true });
                if role == 0 && hid {
                    // Bonded: encrypt with the stored key. New: pair (the peer may also ask).
                    match self.bonds.le_for(&peer).map(|b| b.ltk) {
                        Some(ltk) => self.start_encryption(handle, ltk),
                        None => self.pair(handle),
                    }
                }
                self.next_le();
            }
            Event::DisconnectionComplete { status: 0, handle, reason } => {
                if let Some(c) = self.conns.iter().find(|c| c.handle == handle) { let a = c.addr; self.wanted.retain(|w| *w != a); }
                self.conns.retain(|c| c.handle != handle);
                self.notes.push_back(Note::Disconnected { handle, reason });
            }
            Event::EncryptionChange { status, handle, enabled } => {
                if status == 0 && enabled != 0 {
                    self.notes.push_back(Note::Encrypted { handle });
                    self.encrypted(handle);
                } else if status != 0 {
                    // The stored key was refused (the device forgot us): pair again.
                    let le_bonded = self.conns.iter().find(|c| c.handle == handle).filter(|c| c.le && c.central && c.smp.is_none()).map(|c| c.addr);
                    if let Some(addr) = le_bonded { self.bonds.forget(&addr); self.notes.push_back(Note::BondsChanged); self.pair(handle); }
                }
            }
            Event::LeLtkRequest { handle, .. } => {
                let ltk = self.conns.iter().find(|c| c.handle == handle).and_then(|c| c.ltk.or_else(|| self.bonds.le_for(&c.addr).map(|b| b.ltk)));
                let mut p = handle.to_le_bytes().to_vec();
                match ltk {
                    Some(k) => { p.extend_from_slice(&k); self.send_command(hci::command(op::LE_LTK_REQUEST_REPLY, &p)); }
                    None => self.send_command(hci::command(op::LE_LTK_REQUEST_NEGATIVE_REPLY, &p)),
                }
            }
            Event::Other { code: code::EV_CONNECTION_REQUEST, params } if params.len() >= 10 => {
                let addr = Addr(params[..6].try_into().unwrap());
                // Classic devices may page us only if bonded (a keyboard reconnecting).
                if params[9] == 1 && self.bonds.classic_for(&addr).is_some() {
                    let mut p = addr.0.to_vec();
                    p.push(0x01); // stay peripheral
                    self.send_command(hci::command(code::ACCEPT_CONNECTION_REQUEST, &p));
                } else {
                    let mut p = addr.0.to_vec();
                    p.push(0x0F); // unacceptable address
                    self.send_command(hci::command(code::REJECT_CONNECTION_REQUEST, &p));
                }
            }
            Event::Other { code: code::EV_AUTHENTICATION_COMPLETE, params } if params.len() >= 3 => {
                let handle = u16::from_le_bytes([params[1], params[2]]) & 0xFFF;
                if params[0] == 0 {
                    let mut p = handle.to_le_bytes().to_vec();
                    p.push(1);
                    self.send_command(hci::command(code::SET_CONNECTION_ENCRYPTION, &p));
                } else if let Some((addr, central)) = self.conns.iter().find(|c| c.handle == handle).map(|c| (c.addr, c.central)) {
                    self.notes.push_back(Note::PairingFailed { addr, reason: params[0] });
                    // A device that paged us claiming a bond but cannot prove the key: drop it.
                    if !central { self.disconnect(handle); }
                }
            }
            _ => {}
        }
    }

    /// A link is encrypted: start HID on it if we want it.
    fn encrypted(&mut self, handle: u16) {
        let Some(c) = self.conn(handle) else { return };
        c.encrypted = true;
        let (le, central, hid) = (c.le, c.central, c.hid);
        if le {
            if !central {
                // Responder: hand over the keys the initiator asked for.
                let a = self.conn(handle).and_then(|c| c.smp.as_mut()).map(|s| s.distribute()).unwrap_or_default();
                self.smp_actions(handle, a);
            }
            if central && hid && self.conn(handle).is_some_and(|c| c.gatt.is_none()) {
                let mut g = Hogp::new();
                let out = g.start();
                if let Some(c) = self.conn(handle) { c.gatt = Some(g); }
                self.gatt_out(handle, out);
            }
        } else if central && hid && self.conn(handle).is_some_and(|c| c.chans.by_psm(hidp::PSM_CONTROL).is_none()) {
            let p = self.conn(handle).map(|c| c.chans.connect(hidp::PSM_CONTROL)).unwrap();
            self.send_l2cap(handle, l2cap::CID_SIGNALING, &p);
        }
    }

    fn gatt_out(&mut self, handle: u16, out: Vec<gatt::Out>) {
        let Some(addr) = self.conns.iter().find(|c| c.handle == handle).map(|c| c.addr) else { return };
        for o in out {
            match o {
                gatt::Out::Send(p) => self.send_l2cap(handle, l2cap::CID_ATT, &p),
                gatt::Out::Ready { keyboard, pointer, boot, .. } => {
                    if let Some(c) = self.conn(handle) { c.hid_ready = true; }
                    self.notes.push_back(Note::HidReady { handle, addr, le: true, keyboard, pointer, boot });
                }
                gatt::Out::Input(input) => self.notes.push_back(Note::Input { handle, input }),
                gatt::Out::Failed(code) => self.notes.push_back(Note::HidFailed { handle, code }),
            }
        }
    }

    fn chan_events(&mut self, handle: u16, events: Vec<ChanEvent>) {
        for e in events {
            match e {
                ChanEvent::Send(p) => self.send_l2cap(handle, l2cap::CID_SIGNALING, &p),
                ChanEvent::Open { psm, .. } => {
                    let Some(c) = self.conn(handle) else { return };
                    // We opened control: interrupt next. Both open: boot protocol, then input.
                    if psm == hidp::PSM_CONTROL && c.central && c.chans.by_psm(hidp::PSM_INTERRUPT).is_none() {
                        let p = c.chans.connect(hidp::PSM_INTERRUPT);
                        self.send_l2cap(handle, l2cap::CID_SIGNALING, &p);
                        continue;
                    }
                    let open = |psm| c.chans.by_psm(psm).filter(|ch| ch.open).map(|ch| ch.remote);
                    if let (Some(control), Some(_)) = (open(hidp::PSM_CONTROL), open(hidp::PSM_INTERRUPT)) {
                        if c.hid_ready { continue; }
                        c.hid_ready = true;
                        let addr = c.addr;
                        self.send_l2cap(handle, control, &[hidp::SET_PROTOCOL_BOOT]);
                        let class = self.seen.iter().find(|s| s.addr == addr).and_then(|s| s.class);
                        // Minor class: bit 6 keyboard, bit 7 pointing device (both when unknown).
                        let minor = class.map(|c| (c >> 2) as u8 & 0x30).unwrap_or(0x30);
                        let (keyboard, pointer) = (minor & 0x10 != 0 || minor == 0, minor & 0x20 != 0 || minor == 0);
                        self.notes.push_back(Note::HidReady { handle, addr, le: false, keyboard, pointer, boot: true });
                    }
                }
                ChanEvent::Closed { psm, .. } => {
                    if psm == hidp::PSM_CONTROL || psm == hidp::PSM_INTERRUPT { if let Some(c) = self.conn(handle) { c.hid_ready = false; } }
                }
            }
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
                        Some(&smp::code::SECURITY_REQUEST) if central => {
                            // A bonded device asking again: our stored key is already on its way.
                            let addr = self.conns.iter().find(|c| c.handle == handle).map(|c| c.addr).unwrap();
                            if self.bonds.le_for(&addr).is_none() { self.pair(handle); }
                            return;
                        }
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
            l2cap::CID_SIGNALING => {
                let (addr, encrypted) = self.conns.iter().find(|c| c.handle == handle).map(|c| (c.addr, c.encrypted)).unwrap();
                let bonded = self.bonds.classic_for(&addr).is_some();
                // HID channels to us: only from a bonded device, on an encrypted link or one
                // that will be (the link key is known). Input on them is ignored until the
                // link is encrypted, so a peer that only spoofs the address injects nothing.
                let allow = move |psm: u16| -> Result<(), u16> {
                    if psm != hidp::PSM_CONTROL && psm != hidp::PSM_INTERRUPT { return Err(l2cap::PSM_NOT_SUPPORTED); }
                    if bonded || encrypted { Ok(()) } else { Err(l2cap::SECURITY_BLOCK) }
                };
                for (code, id, data) in l2cap::commands(payload) {
                    let events = self.conn(handle).and_then(|c| c.chans.on_command(code, id, data, &allow));
                    match events {
                        Some(e) => self.chan_events(handle, e),
                        None => match l2cap::signal(cid, &l2cap::command(code, id, data)) {
                            Signal::Reply(r) => self.send_l2cap(handle, cid, &r),
                            Signal::Accepted { reply, .. } => self.send_l2cap(handle, cid, &reply),
                            Signal::Nothing => {}
                        },
                    }
                }
            }
            l2cap::CID_LE_SIGNALING => match l2cap::signal(cid, payload) {
                Signal::Reply(r) => self.send_l2cap(handle, cid, &r),
                Signal::Accepted { reply, .. } => self.send_l2cap(handle, cid, &reply),
                Signal::Nothing => {}
            },
            l2cap::CID_ATT => {
                // Requests from the peer's client: we serve no attributes yet.
                if let Some(r) = att::serve(payload) { self.send_l2cap(handle, cid, &r); return; }
                if payload.first() == Some(&att::op::INDICATION) { self.send_l2cap(handle, cid, &[att::op::CONFIRMATION]); }
                let out = self.conn(handle).and_then(|c| c.gatt.as_mut()).map(|g| g.on_pdu(payload)).unwrap_or_default();
                self.gatt_out(handle, out);
            }
            cid => {
                // Classic HID channels.
                let Some(c) = self.conns.iter().find(|c| c.handle == handle) else { return };
                let Some(ch) = c.chans.by_local(cid).copied() else { return };
                if !c.encrypted { return; }
                match ch.psm {
                    hidp::PSM_INTERRUPT => {
                        if let Some(input) = hidp::input(payload) { self.notes.push_back(Note::Input { handle, input }); }
                    }
                    hidp::PSM_CONTROL => {
                        if hidp::control(payload) == hidp::Control::Unplug { let a = c.addr; self.forget(a); }
                    }
                    _ => {}
                }
            }
        }
    }
}
