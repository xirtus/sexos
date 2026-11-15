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
//!
//! Headphones (A2DP Source): [`Host::connect_audio`] pages a headset, pairs, encrypts,
//! finds its A2DP Sink with SDP, sets an SBC stream up with AVDTP and opens the
//! transport channel; a bonded headset may page us and set the stream up itself.
//! [`Host::play`] encodes PCM into RTP media packets, only on an encrypted link to a
//! bonded device and only as fast as the controller frees ACL buffers.
//!
//! ACL flow control: ACL packets go to the controller only while it has buffers free
//! (Read Buffer Size, then Number Of Completed Packets gives them back), per pool
//! (classic, LE, or one shared pool).

use crate::a2dp;
use crate::att;
use crate::avdtp::{self, Source};
use crate::sbc;
use crate::sdp;
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
    pub const LE_CONNECTION_UPDATE: u16 = 0x2013;
}

/// The largest ACL data length we send in one packet, whatever the controller reports:
/// the packet (type byte, 4-byte ACL header, data) must fit the SexUSB ring slot
/// (`sex_pdx::bt_hci::MAX_PACKET`, 2044 bytes).
pub const MAX_ACL_DATA: usize = 2044 - 5;

/// Something the server should report or act on (boot log lines, desktop prompts, input).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    Ready(Info),
    /// A device seen for the first time, or seen again with its name.
    Device(Found),
    Connected { handle: u16, addr: Addr, le: bool },
    /// A connection we asked for did not come up (HCI status, e.g. 0x04 page timeout).
    ConnectFailed { addr: Addr, status: u8 },
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
    /// The headset's A2DP Sink record (from SDP).
    AudioSink { handle: u16, sink: sdp::A2dpSink },
    /// A stream is configured and its transport channel open: SBC coding and how many
    /// frames go in one media packet.
    AudioReady { handle: u16, addr: Addr, config: sbc::Config, frames_per_packet: u8 },
    AudioStreaming { handle: u16 },
    AudioSuspended { handle: u16 },
    AudioClosed { handle: u16 },
    /// Audio setup stopped: an AVDTP or SDP error code, or `avdtp::error::NO_*`.
    AudioFailed { handle: u16, code: u16 },
}

/// Media packets a stream may have queued or in the controller at once (about 100 ms
/// at 48 kHz with 7 SBC frames a packet).
pub const MEDIA_WINDOW: usize = 6;

/// One link's audio: SDP, AVDTP signaling and transport channels, and the encoder.
struct Audio {
    /// Our SDP client channel and its search.
    sdp: Option<(u16, sdp::Client)>,
    /// The AVDTP PSM (from SDP; 0x0019 when the headset opened it).
    psm: u16,
    signal: Option<u16>,
    media: Option<u16>,
    source: Source,
    config: Option<sbc::Config>,
    packets: Option<a2dp::Packetizer>,
    /// The signaling channel's outgoing MTU.
    sig_mtu: usize,
}

impl Audio {
    fn source_mtu(&self) -> usize { self.sig_mtu }
    fn new() -> Audio { Audio { sdp: None, psm: avdtp::PSM, signal: None, media: None, source: Source::new(l2cap::DEFAULT_MTU as usize), config: None, packets: None, sig_mtu: l2cap::DEFAULT_MTU as usize } }
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
    /// We paged it for audio.
    audio_wanted: bool,
    audio: Option<Box<Audio>>,
    /// ACL packets sent and not yet completed by the controller.
    outstanding: u16,
}

impl Conn {
    fn new(handle: u16, addr: Addr, addr_type: u8, le: bool, central: bool, hid: bool) -> Conn {
        Conn { handle, addr, addr_type, le, central, rx: Reassembly::default(), smp: None, ltk: None, encrypted: false, hid,
            chans: Channels::default(), gatt: None, hid_ready: false, audio_wanted: false, audio: None, outstanding: 0 }
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
    /// Free controller ACL buffers: classic (or shared) pool, LE pool.
    free: [u16; 2],
    /// Headsets we are connecting for audio.
    audio_wanted: Vec<Addr>,
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
            auth: smp::AUTH_BONDING | smp::AUTH_MITM, bonds: Bonds::default(), commands: VecDeque::new(), acl: VecDeque::new(), credits: 1, free: [0; 2], audio_wanted: Vec::new(),
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

    /// The buffer pool a link's ACL packets take.
    fn pool(&self, le: bool) -> usize { if le && !self.bringup.info.le_shared { 1 } else { 0 } }

    /// The next packet (with its type byte) to send to the controller.
    pub fn poll(&mut self) -> Option<Vec<u8>> {
        // ACL first, while the controller has a buffer for it; packets for a link that
        // went away are dropped.
        while let Some(front) = self.acl.front() {
            let at = hci::parse_acl(front).and_then(|(h, _, _)| self.conns.iter().position(|c| c.handle == h));
            let Some(at) = at else { self.acl.pop_front(); continue };
            let pool = self.pool(self.conns[at].le);
            if self.free[pool] == 0 { break; }
            self.free[pool] -= 1;
            self.conns[at].outstanding += 1;
            return self.acl.pop_front();
        }
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

    /// Connect headphones found by scanning (classic): page, pair, encrypt, then SDP and
    /// an AVDTP stream (notes `AudioSink`, `AudioReady`, `AudioStreaming`).
    pub fn connect_audio(&mut self, f: &Found) {
        if f.transport != Transport::Classic || self.conns.iter().any(|c| c.addr == f.addr) || self.wanted.contains(&f.addr) { return; }
        self.wanted.push(f.addr);
        self.audio_wanted.push(f.addr);
        if !self.seen.iter().any(|s| s.addr == f.addr) { self.seen.push(f.clone()); }
        self.connect_classic(f.addr);
    }

    /// Whether the device looks like headphones or a speaker: major class Audio/Video
    /// (0x04) with the Rendering or Audio service bit.
    pub fn is_audio(f: &Found) -> bool {
        f.class.is_some_and(|c| (c >> 8) & 0x1F == 0x04 && c & (1 << 18 | 1 << 21) != 0)
    }

    /// The links whose stream is streaming now.
    pub fn streaming(&self) -> impl Iterator<Item = u16> + '_ {
        self.conns.iter().filter(|c| c.audio.as_ref().is_some_and(|a| a.source.state == avdtp::State::Streaming)).map(|c| c.handle)
    }

    /// The AVDTP state of a link's stream.
    pub fn audio_state(&self, handle: u16) -> Option<avdtp::State> {
        self.conns.iter().find(|c| c.handle == handle)?.audio.as_ref().map(|a| a.source.state)
    }

    /// Encode and queue PCM (48 kHz stereo pairs) on a streaming link. Takes whole SBC
    /// frames, no more than the media window allows; returns how many pairs it took
    /// (0 when the link is not streaming, not encrypted, not bonded, or full). The rest
    /// waits in the caller's buffer.
    pub fn play(&mut self, handle: u16, pcm: &[[i16; 2]]) -> usize {
        let queued = self.acl.iter().filter(|p| hci::parse_acl(p).is_some_and(|(h, _, _)| h == handle)).count();
        let Some(c) = self.conns.iter_mut().find(|c| c.handle == handle) else { return 0 };
        let bonded = self.bonds.classic_for(&c.addr).is_some();
        if !c.encrypted || !bonded || c.le { return 0; }
        let mut in_flight = queued + c.outstanding as usize;
        let Some(a) = c.audio.as_mut() else { return 0 };
        if a.source.state != avdtp::State::Streaming { return 0; }
        let (Some(media), Some(packets)) = (a.media, a.packets.as_mut()) else { return 0 };
        let Some(remote) = c.chans.by_local(media).filter(|ch| ch.open).map(|ch| ch.remote) else { return 0 };
        let mut used = 0;
        let mut out = Vec::new();
        let acl_mtu = (self.bringup.info.acl_mtu.max(23) as usize).min(MAX_ACL_DATA);
        while in_flight < MEDIA_WINDOW {
            let Some((p, n)) = packets.packet(&pcm[used..]) else { break };
            used += n;
            // The ACL packets this media packet (with its L2CAP header) takes.
            in_flight += (p.len() + 4).div_ceil(acl_mtu);
            out.push(p);
        }
        for p in out { self.send_l2cap(handle, remote, &p); }
        used
    }

    /// Pause a streaming link's stream (AVDTP SUSPEND).
    pub fn suspend_audio(&mut self, handle: u16) { self.with_source(handle, |s| s.suspend()); }
    /// Start a configured, open stream again (AVDTP START).
    pub fn resume_audio(&mut self, handle: u16) { self.with_source(handle, |s| s.resume()); }
    /// Close a link's stream (AVDTP CLOSE).
    pub fn close_audio(&mut self, handle: u16) { self.with_source(handle, |s| s.close()); }
    /// Set a stream up on a link whose headset opened AVDTP signaling but has not
    /// configured a stream itself (we become initiator).
    pub fn start_audio(&mut self, handle: u16) { self.with_source(handle, |s| s.start()); }

    fn with_source(&mut self, handle: u16, f: impl FnOnce(&mut Source) -> Vec<avdtp::Out>) {
        let Some(a) = self.conn(handle).and_then(|c| c.audio.as_mut()) else { return };
        if a.signal.is_none() { return; }
        let out = f(&mut a.source);
        self.avdtp_out(handle, out);
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
        let reported = if le { self.bringup.info.le_acl_mtu } else { self.bringup.info.acl_mtu };
        let mtu = (reported.max(23) as usize).min(MAX_ACL_DATA);
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
                Ok(true) if !self.ready_noted => {
                    self.ready_noted = true;
                    let i = self.bringup.info;
                    // A controller that reports no buffers still takes one packet at a time.
                    self.free = [i.acl_packets.max(1), if i.le_shared { 0 } else { (i.le_acl_packets as u16).max(1) }];
                    self.notes.push_back(Note::Ready(i));
                }
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
            Event::CompletedPackets { handles } => {
                // Buffers come back per link, never more than the link has out.
                let n = (handles[0] as usize).min((handles.len() - 1) / 4);
                for k in 0..n {
                    let at = 1 + 4 * k;
                    let handle = u16::from_le_bytes([handles[at], handles[at + 1]]) & 0xFFF;
                    let count = u16::from_le_bytes([handles[at + 2], handles[at + 3]]);
                    let Some(i) = self.conns.iter().position(|c| c.handle == handle) else { continue };
                    let done = count.min(self.conns[i].outstanding);
                    self.conns[i].outstanding -= done;
                    let pool = self.pool(self.conns[i].le);
                    self.free[pool] = self.free[pool].saturating_add(done);
                }
            }
            Event::RemoteNameComplete { status: 0, addr, name } => {
                let mut f = Found { addr, transport: report::Transport::Classic, addr_type: 0, rssi: None, class: None,
                    name: Some(alloc::string::String::from_utf8_lossy(name).into_owned()), appearance: None, flags: None, connectable: true };
                if let Some(s) = self.seen.iter().find(|s| s.addr == addr) { f.rssi = s.rssi; f.class = s.class; }
                self.found(f);
            }
            Event::ConnectionComplete { status, handle, addr, .. } => {
                if status != 0 {
                    if self.wanted.contains(&addr) { self.notes.push_back(Note::ConnectFailed { addr, status }); }
                    self.wanted.retain(|a| *a != addr);
                    self.audio_wanted.retain(|a| *a != addr);
                    return;
                }
                let ours = self.wanted.contains(&addr);
                let audio = self.audio_wanted.contains(&addr);
                let hid = self.hid_wanted(&addr) && !audio;
                let mut c = Conn::new(handle, addr, 0, false, ours, hid);
                c.audio_wanted = audio;
                self.conns.push(c);
                self.notes.push_back(Note::Connected { handle, addr, le: false });
                // We paged it for HID, or a bonded device paged us: authenticate (proving it
                // holds the link key), then encrypt, then open or accept the channels.
                if ours || self.bonds.classic_for(&addr).is_some() { self.send_command(Ssp::authenticate(handle)); }
            }
            Event::LeConnectionComplete { status, handle, role, peer_type, peer } => {
                self.le_connecting = None;
                if status != 0 {
                    if self.wanted.contains(&peer) { self.notes.push_back(Note::ConnectFailed { addr: peer, status }); }
                    self.wanted.retain(|a| *a != peer);
                    self.next_le();
                    return;
                }
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
                if let Some(c) = self.conns.iter().find(|c| c.handle == handle) {
                    let (a, le, out, audio) = (c.addr, c.le, c.outstanding, c.audio.is_some());
                    self.wanted.retain(|w| *w != a);
                    self.audio_wanted.retain(|w| *w != a);
                    // The controller drops what the link had in its buffers.
                    let pool = self.pool(le);
                    self.free[pool] = self.free[pool].saturating_add(out);
                    if audio { self.notes.push_back(Note::AudioClosed { handle }); }
                }
                self.conns.retain(|c| c.handle != handle);
                self.notes.push_back(Note::Disconnected { handle, reason });
            }
            Event::EncryptionChange { status, handle, enabled } => {
                if status == 0 && enabled != 0 {
                    self.notes.push_back(Note::Encrypted { handle });
                    self.encrypted(handle);
                } else if status == 0 {
                    // Encryption turned off: nothing more is trusted on this link
                    // (HID input and AVDTP are ignored, audio stops) until it is back on.
                    if let Some(c) = self.conn(handle) { c.encrypted = false; }
                    // Nothing already queued for the link goes out in the clear either
                    // (media waiting for controller buffers): re-securing the link takes
                    // HCI commands, not ACL data.
                    self.acl.retain(|p| !hci::parse_acl(p).is_some_and(|(h, _, _)| h == handle));
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
        } else if central && self.conn(handle).is_some_and(|c| c.audio_wanted && c.audio.is_none()) {
            // Headphones: find the A2DP Sink first.
            let Some(c) = self.conn(handle) else { return };
            let p = c.chans.connect(sdp::PSM);
            let local = c.chans.chans.last().map(|ch| ch.local).unwrap_or(0);
            let mut a = Box::new(Audio::new());
            a.sdp = Some((local, sdp::Client::a2dp_sink()));
            c.audio = Some(a);
            self.send_l2cap(handle, l2cap::CID_SIGNALING, &p);
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
                ChanEvent::Open { psm, local } if psm == sdp::PSM || psm == avdtp::PSM => self.audio_chan_open(handle, psm, local),
                ChanEvent::Closed { psm, local } if psm == sdp::PSM || psm == avdtp::PSM => self.audio_chan_closed(handle, local),
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

    fn audio_chan_open(&mut self, handle: u16, psm: u16, local: u16) {
        let Some(c) = self.conn(handle) else { return };
        let Some(ch) = c.chans.by_local(local).copied() else { return };
        if psm == sdp::PSM {
            // Our search starts; a peer's SDP channel just waits for its requests.
            let req = c.audio.as_mut().and_then(|a| a.sdp.as_mut()).filter(|(cid, _)| *cid == local).map(|(_, client)| client.start());
            if let Some(r) = req { self.send_l2cap(handle, ch.remote, &r); }
            return;
        }
        let a = c.audio.get_or_insert_with(|| Box::new(Audio::new()));
        if a.signal.is_none() || a.signal == Some(local) {
            // The first AVDTP channel is signaling.
            a.signal = Some(local);
            a.sig_mtu = ch.mtu as usize;
            a.source = Source::new(a.sig_mtu);
            let ours = c.central;
            let out = if ours { a.source.start() } else { Vec::new() };
            self.avdtp_out(handle, out);
            return;
        }
        // The second is the stream's transport channel.
        a.media = Some(local);
        let Some(config) = a.config else { return };
        match a2dp::Packetizer::new(config, ch.mtu as usize) {
            Some(p) => {
                let (addr, frames) = (c.addr, p.frames_per_packet() as u8);
                a.packets = Some(p);
                let out = a.source.media_opened();
                self.notes.push_back(Note::AudioReady { handle, addr, config, frames_per_packet: frames });
                self.avdtp_out(handle, out);
            }
            None => {
                let out = a.source.close();
                self.notes.push_back(Note::AudioFailed { handle, code: avdtp::error::NO_CONFIGURATION as u16 });
                self.avdtp_out(handle, out);
            }
        }
    }

    fn audio_chan_closed(&mut self, handle: u16, local: u16) {
        let Some(a) = self.conn(handle).and_then(|c| c.audio.as_mut()) else { return };
        if a.media == Some(local) {
            a.media = None;
            a.packets = None;
            if matches!(a.source.state, avdtp::State::Streaming | avdtp::State::Open | avdtp::State::Starting | avdtp::State::Suspending) {
                // The transport went without CLOSE: the stream is gone; abort it.
                a.source = Source::new(a.source_mtu());
                self.notes.push_back(Note::AudioClosed { handle });
            }
        } else if a.signal == Some(local) {
            // Signaling gone: everything with it.
            let media = a.media.take();
            a.signal = None;
            a.packets = None;
            a.config = None;
            a.source = Source::new(l2cap::DEFAULT_MTU as usize);
            self.notes.push_back(Note::AudioClosed { handle });
            if let Some(m) = media { self.close_chan(handle, m); }
        } else if a.sdp.as_ref().is_some_and(|(cid, _)| *cid == local) && a.signal.is_none() {
            a.sdp = None;
            self.notes.push_back(Note::AudioFailed { handle, code: sdp::error::INVALID_SYNTAX });
        }
    }

    fn close_chan(&mut self, handle: u16, local: u16) {
        let p = self.conn(handle).and_then(|c| c.chans.disconnect(local));
        if let Some(p) = p { self.send_l2cap(handle, l2cap::CID_SIGNALING, &p); }
    }

    /// SDP on a channel: answers from our search, or a peer's requests to our server.
    fn on_sdp(&mut self, handle: u16, cid: u16, remote: u16, payload: &[u8]) {
        let Some(c) = self.conn(handle) else { return };
        let ours = c.audio.as_mut().and_then(|a| a.sdp.as_mut()).filter(|(l, _)| *l == cid);
        let Some((_, client)) = ours else {
            let r = sdp::serve(payload, &[sdp::a2dp_source_record()]);
            self.send_l2cap(handle, remote, &r);
            return;
        };
        match client.on_pdu(payload) {
            sdp::Progress::Send(r) => self.send_l2cap(handle, remote, &r),
            sdp::Progress::Done(records) => {
                let found = sdp::a2dp_sink(&records);
                let a = c.audio.as_mut().unwrap();
                a.sdp = None;
                self.close_chan(handle, cid);
                match found {
                    Some(sink) => {
                        self.notes.push_back(Note::AudioSink { handle, sink });
                        let Some(c) = self.conn(handle) else { return };
                        let p = c.chans.connect(sink.psm);
                        if let Some(a) = c.audio.as_mut() { a.psm = sink.psm; }
                        self.send_l2cap(handle, l2cap::CID_SIGNALING, &p);
                    }
                    None => self.notes.push_back(Note::AudioFailed { handle, code: avdtp::error::NO_SINK as u16 }),
                }
            }
            sdp::Progress::Failed(code) => {
                if let Some(a) = c.audio.as_mut() { a.sdp = None; }
                self.close_chan(handle, cid);
                self.notes.push_back(Note::AudioFailed { handle, code });
            }
        }
    }

    fn on_avdtp(&mut self, handle: u16, cid: u16, payload: &[u8]) {
        let Some(a) = self.conn(handle).and_then(|c| c.audio.as_mut()) else { return };
        // Media from the headset (it should send none) is ignored.
        if a.signal != Some(cid) { return; }
        let out = a.source.on_packet(payload);
        self.avdtp_out(handle, out);
    }

    fn avdtp_out(&mut self, handle: u16, out: Vec<avdtp::Out>) {
        for o in out {
            let Some(c) = self.conn(handle) else { return };
            let Some(a) = c.audio.as_mut() else { return };
            match o {
                avdtp::Out::Send(p) => {
                    let Some(remote) = a.signal.and_then(|s| c.chans.by_local(s)).map(|ch| ch.remote) else { continue };
                    self.send_l2cap(handle, remote, &p);
                }
                avdtp::Out::OpenMedia => {
                    let psm = a.psm;
                    let p = c.chans.connect(psm);
                    self.send_l2cap(handle, l2cap::CID_SIGNALING, &p);
                }
                avdtp::Out::Configured(config) => a.config = Some(config),
                avdtp::Out::Streaming => self.notes.push_back(Note::AudioStreaming { handle }),
                avdtp::Out::Suspended => self.notes.push_back(Note::AudioSuspended { handle }),
                avdtp::Out::Closed => {
                    a.config = None;
                    a.packets = None;
                    let media = a.media.take();
                    self.notes.push_back(Note::AudioClosed { handle });
                    if let Some(m) = media { self.close_chan(handle, m); }
                }
                avdtp::Out::Failed(code) => self.notes.push_back(Note::AudioFailed { handle, code: code as u16 }),
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
                // The same for AVDTP (a bonded headset reconnecting); SDP (what we offer) is
                // open to any link we let in.
                let allow = move |psm: u16| -> Result<(), u16> {
                    if psm == sdp::PSM { return Ok(()); }
                    if psm != hidp::PSM_CONTROL && psm != hidp::PSM_INTERRUPT && psm != avdtp::PSM { return Err(l2cap::PSM_NOT_SUPPORTED); }
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
            l2cap::CID_LE_SIGNALING => {
                // Only an LE link where we are central may be asked for new parameters
                // (the peripheral asks the central); anything else is refused.
                let le_central = self.conns.iter().find(|c| c.handle == handle).is_some_and(|c| c.le && c.central);
                match l2cap::signal(cid, payload) {
                    Signal::Reply(r) => self.send_l2cap(handle, cid, &r),
                    Signal::Accepted { min, max, latency, timeout, reply } if le_central => {
                        self.send_l2cap(handle, cid, &reply);
                        let mut p = handle.to_le_bytes().to_vec();
                        for v in [min, max, latency, timeout, 0, 0] { p.extend_from_slice(&v.to_le_bytes()); }
                        self.send_command(hci::command(code::LE_CONNECTION_UPDATE, &p));
                    }
                    Signal::Accepted { .. } => {
                        // Command not understood (0x0000): we are not the central here.
                        if payload.len() >= 2 {
                            self.send_l2cap(handle, cid, &l2cap::command(l2cap::sig::COMMAND_REJECT, payload[1], &[0, 0]));
                        }
                    }
                    Signal::Nothing => {}
                }
            }
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
                if ch.psm == sdp::PSM { return self.on_sdp(handle, cid, ch.remote, payload); }
                if !c.encrypted { return; }
                match ch.psm {
                    avdtp::PSM => self.on_avdtp(handle, cid, payload),
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
