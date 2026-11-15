// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! AVRCP over AVCTP (AVCTP 1.4, AVRCP 1.6.2) on the control channel, L2CAP PSM 0x0017:
//! a headset's buttons and its volume.
//!
//! Roles, as a phone has them with headphones:
//! - We are Target (category 1, player) for the headset's PASS THROUGH buttons: play,
//!   pause, stop, forward, backward, and volume up/down and mute for a headset without
//!   absolute volume. Each press and release comes out as [`Out::Key`].
//! - We are Controller (category 2, amplifier) for absolute volume (AVRCP 1.4): we
//!   register for EVENT_VOLUME_CHANGED at the headset and send SetAbsoluteVolume. A
//!   headset that rejects the registration (or answers nothing) does no absolute volume.
//!
//! Everything the peer sends is bounded: only single AVCTP packets are taken (start,
//! continue and end packets are dropped: nothing we accept needs more than the 48-byte
//! minimum MTU, so there is no reassembly to grow), AVRCP's own fragmentation is
//! refused, every length is checked against the frame, and each packet in gives at most
//! one packet out, never more than [`MAX_REPLY`] bytes.

use alloc::vec::Vec;

pub const PSM: u16 = 0x0017;
/// The A/V remote control profile id in every AVCTP header (and its SDP class).
pub const PID: u16 = 0x110E;
/// The longest AV/C frame we answer (an echo of the peer's own); longer ones are dropped.
pub const MAX_REPLY: usize = 512;
/// Absolute volume runs 0..=0x7F.
pub const MAX_VOLUME: u8 = 0x7F;

/// AV/C command types and response codes.
pub mod ctype {
    pub const CONTROL: u8 = 0x0;
    pub const STATUS: u8 = 0x1;
    pub const NOTIFY: u8 = 0x3;
    pub const NOT_IMPLEMENTED: u8 = 0x8;
    pub const ACCEPTED: u8 = 0x9;
    pub const REJECTED: u8 = 0xA;
    pub const STABLE: u8 = 0xC;
    pub const CHANGED: u8 = 0xD;
    pub const INTERIM: u8 = 0xF;
}

/// AV/C opcodes, and the panel subunit (type 9, id 0) and the unit itself.
pub mod opcode {
    pub const VENDOR_DEPENDENT: u8 = 0x00;
    pub const UNIT_INFO: u8 = 0x30;
    pub const SUBUNIT_INFO: u8 = 0x31;
    pub const PASS_THROUGH: u8 = 0x7C;
    pub const PANEL: u8 = 0x48;
    pub const UNIT: u8 = 0xFF;
}

/// PASS THROUGH operation ids.
pub mod op {
    pub const VOLUME_UP: u8 = 0x41;
    pub const VOLUME_DOWN: u8 = 0x42;
    pub const MUTE: u8 = 0x43;
    pub const PLAY: u8 = 0x44;
    pub const STOP: u8 = 0x45;
    pub const PAUSE: u8 = 0x46;
    pub const FORWARD: u8 = 0x4B;
    pub const BACKWARD: u8 = 0x4C;
}

/// AVRCP PDU ids (vendor dependent, Bluetooth SIG company id), events and errors.
pub mod pdu {
    pub const GET_CAPABILITIES: u8 = 0x10;
    pub const REGISTER_NOTIFICATION: u8 = 0x31;
    pub const SET_ABSOLUTE_VOLUME: u8 = 0x50;
    pub const EVENT_VOLUME_CHANGED: u8 = 0x0D;
    pub const CAP_COMPANY_ID: u8 = 0x02;
    pub const CAP_EVENTS: u8 = 0x03;
    pub const ERR_INVALID_COMMAND: u8 = 0x00;
    pub const ERR_INVALID_PARAMETER: u8 = 0x01;
}
pub const BT_SIG: [u8; 3] = [0x00, 0x19, 0x58];

/// A headset button. Play and Pause are one key: the desktop's play/pause toggles, and
/// a headset alternates the two by its own guess of the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key { PlayPause, Stop, Next, Previous, VolumeUp, VolumeDown, Mute }

pub fn key(op_id: u8) -> Option<Key> {
    Some(match op_id {
        op::PLAY | op::PAUSE => Key::PlayPause,
        op::STOP => Key::Stop,
        op::FORWARD => Key::Next,
        op::BACKWARD => Key::Previous,
        op::VOLUME_UP => Key::VolumeUp,
        op::VOLUME_DOWN => Key::VolumeDown,
        op::MUTE => Key::Mute,
        _ => return None,
    })
}

/// One single-packet AVCTP message.
#[derive(Debug, PartialEq, Eq)]
pub struct Avctp<'a> { pub label: u8, pub response: bool, pub ipid: bool, pub pid: u16, pub msg: &'a [u8] }

/// The message in an AVCTP packet; None for fragments (packet type other than single)
/// and packets too short for the header.
pub fn parse(p: &[u8]) -> Option<Avctp<'_>> {
    if p.len() < 3 || p[0] >> 2 & 3 != 0 { return None; }
    Some(Avctp { label: p[0] >> 4, response: p[0] & 2 != 0, ipid: p[0] & 1 != 0, pid: u16::from_be_bytes([p[1], p[2]]), msg: &p[3..] })
}

/// A single AVCTP packet.
pub fn packet(label: u8, response: bool, ipid: bool, pid: u16, msg: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(3 + msg.len());
    p.push((label & 0x0F) << 4 | (response as u8) << 1 | ipid as u8);
    p.extend_from_slice(&pid.to_be_bytes());
    p.extend_from_slice(msg);
    p
}

/// An AVRCP vendor-dependent frame: ctype, panel, company, PDU, single packet, params.
pub fn vendor(ctype: u8, pdu: u8, params: &[u8]) -> Vec<u8> {
    let mut m = alloc::vec![ctype, opcode::PANEL, opcode::VENDOR_DEPENDENT, BT_SIG[0], BT_SIG[1], BT_SIG[2], pdu, 0];
    m.extend_from_slice(&(params.len() as u16).to_be_bytes());
    m.extend_from_slice(params);
    m
}

/// (ctype, PDU id, parameters) of a vendor-dependent frame with the SIG company id, a
/// single AVRCP packet and a parameter length inside the frame.
pub fn parse_vendor(m: &[u8]) -> Option<(u8, u8, &[u8])> {
    if m.len() < 10 || m[2] != opcode::VENDOR_DEPENDENT || m[3..6] != BT_SIG || m[7] & 3 != 0 { return None; }
    let len = u16::from_be_bytes([m[8], m[9]]) as usize;
    Some((m[0] & 0x0F, m[6], m.get(10..10 + len)?))
}

/// What the link should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Send this AVCTP packet on the control channel.
    Send(Vec<u8>),
    /// A headset button went down (true) or up.
    Key { key: Key, pressed: bool },
    /// The headset's absolute volume (0..=0x7F): `changed` when the headset moved it
    /// itself (its buttons), otherwise its answer to a registration or to our setting.
    Volume { volume: u8, changed: bool },
    /// The headset does no absolute volume.
    NoVolume,
}

/// Where absolute volume stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Absolute { Unknown, Asked, Yes, No }

/// One link's AVRCP: our commands' labels, the volume registration and what the
/// headset last said its volume was.
pub struct Remote {
    label: u8,
    /// The label of our outstanding volume registration.
    notify: Option<u8>,
    pub absolute: Absolute,
    pub volume: Option<u8>,
    pub started: bool,
}

impl Default for Remote { fn default() -> Self { Remote::new() } }

impl Remote {
    pub fn new() -> Remote { Remote { label: 0, notify: None, absolute: Absolute::Unknown, volume: None, started: false } }

    fn command(&mut self, ctype: u8, pdu: u8, params: &[u8]) -> Out {
        self.label = (self.label + 1) & 0x0F;
        Out::Send(packet(self.label, false, false, PID, &vendor(ctype, pdu, params)))
    }
    fn register(&mut self) -> Out {
        let o = self.command(ctype::NOTIFY, pdu::REGISTER_NOTIFICATION, &[pdu::EVENT_VOLUME_CHANGED, 0, 0, 0, 0]);
        self.notify = Some(self.label);
        o
    }

    /// The channel is open on a secure link: ask for the headset's volume changes.
    pub fn start(&mut self) -> Vec<Out> {
        if self.started { return Vec::new(); }
        self.started = true;
        self.absolute = Absolute::Asked;
        alloc::vec![self.register()]
    }

    /// The absolute volume we know, when the headset does absolute volume.
    pub fn absolute_volume(&self) -> Option<u8> { (self.absolute == Absolute::Yes).then_some(self.volume).flatten() }

    /// Set the headset's volume (only once it does absolute volume; nothing when it is
    /// there already, so an echo of the headset's own change goes nowhere).
    pub fn set_volume(&mut self, volume: u8) -> Vec<Out> {
        let v = volume.min(MAX_VOLUME);
        if self.absolute != Absolute::Yes || self.volume == Some(v) { return Vec::new(); }
        self.volume = Some(v);
        alloc::vec![self.command(ctype::CONTROL, pdu::SET_ABSOLUTE_VOLUME, &[v])]
    }

    /// One AVCTP packet from the headset.
    pub fn on_packet(&mut self, p: &[u8]) -> Vec<Out> {
        let mut out = Vec::new();
        let Some(m) = parse(p) else { return out };
        if m.msg.len() > MAX_REPLY { return out; }
        if m.response { self.on_response(&m, &mut out); } else { self.on_command(&m, &mut out); }
        out
    }

    fn on_response(&mut self, m: &Avctp, out: &mut Vec<Out>) {
        // The headset has no remote control profile: no absolute volume.
        if m.ipid || m.pid != PID {
            if self.notify == Some(m.label) { self.no_volume(out); }
            return;
        }
        let Some(&c) = m.msg.first() else { return };
        let c = c & 0x0F;
        let Some((_, pdu_id, params)) = parse_vendor(m.msg) else {
            // An AV/C refusal of our vendor-dependent command (echoed or cut short).
            if self.notify == Some(m.label) && matches!(c, ctype::NOT_IMPLEMENTED | ctype::REJECTED) { self.no_volume(out); }
            return;
        };
        match pdu_id {
            pdu::REGISTER_NOTIFICATION if self.notify == Some(m.label) => match c {
                ctype::INTERIM | ctype::CHANGED if params.first() == Some(&pdu::EVENT_VOLUME_CHANGED) && params.len() >= 2 => {
                    let volume = params[1] & MAX_VOLUME;
                    self.absolute = Absolute::Yes;
                    self.volume = Some(volume);
                    out.push(Out::Volume { volume, changed: c == ctype::CHANGED });
                    // A notification is good for one change: register again for the next.
                    if c == ctype::CHANGED { let r = self.register(); out.push(r); }
                }
                ctype::INTERIM | ctype::CHANGED => {}
                _ => self.no_volume(out),
            },
            pdu::SET_ABSOLUTE_VOLUME if c == ctype::ACCEPTED && self.absolute == Absolute::Yes => {
                if let Some(&v) = params.first() {
                    self.volume = Some(v & MAX_VOLUME);
                    out.push(Out::Volume { volume: v & MAX_VOLUME, changed: false });
                }
            }
            _ => {}
        }
    }

    fn no_volume(&mut self, out: &mut Vec<Out>) {
        self.notify = None;
        if self.absolute != Absolute::No { self.absolute = Absolute::No; out.push(Out::NoVolume); }
    }

    fn on_command(&mut self, m: &Avctp, out: &mut Vec<Out>) {
        let reply = |msg: &[u8]| Out::Send(packet(m.label, true, false, PID, msg));
        if m.pid != PID {
            // Not a profile we have: the header back with "invalid profile id".
            out.push(Out::Send(packet(m.label, true, true, m.pid, &[])));
            return;
        }
        let msg = m.msg;
        if msg.len() < 3 { return; }
        let (c, unit, code) = (msg[0] & 0x0F, msg[1], msg[2]);
        let echo = |code: u8| { let mut r = msg.to_vec(); r[0] = code; reply(&r) };
        match code {
            opcode::PASS_THROUGH if msg.len() >= 5 && 5 + msg[4] as usize <= msg.len() => {
                let id = msg[3] & 0x7F;
                match key(id).filter(|_| c == ctype::CONTROL && unit == opcode::PANEL) {
                    Some(k) => { out.push(echo(ctype::ACCEPTED)); out.push(Out::Key { key: k, pressed: msg[3] & 0x80 == 0 }); }
                    None => out.push(echo(ctype::NOT_IMPLEMENTED)),
                }
            }
            // We are one panel subunit; company id unknown (all ones).
            opcode::UNIT_INFO if c == ctype::STATUS => out.push(reply(&[ctype::STABLE, opcode::UNIT, code, 0x07, opcode::PANEL, 0xFF, 0xFF, 0xFF])),
            opcode::SUBUNIT_INFO if c == ctype::STATUS => out.push(reply(&[ctype::STABLE, opcode::UNIT, code, 0x07, opcode::PANEL, 0xFF, 0xFF, 0xFF])),
            opcode::VENDOR_DEPENDENT => match parse_vendor(msg) {
                Some((_, pdu::GET_CAPABILITIES, p)) if c == ctype::STATUS => {
                    let r = match p.first() {
                        Some(&pdu::CAP_COMPANY_ID) => vendor(ctype::STABLE, pdu::GET_CAPABILITIES, &[pdu::CAP_COMPANY_ID, 1, BT_SIG[0], BT_SIG[1], BT_SIG[2]]),
                        // As target we only take buttons: no events to register for.
                        Some(&pdu::CAP_EVENTS) => vendor(ctype::STABLE, pdu::GET_CAPABILITIES, &[pdu::CAP_EVENTS, 0]),
                        _ => vendor(ctype::REJECTED, pdu::GET_CAPABILITIES, &[pdu::ERR_INVALID_PARAMETER]),
                    };
                    out.push(reply(&r));
                }
                Some((_, pdu::REGISTER_NOTIFICATION, _)) => out.push(reply(&vendor(ctype::REJECTED, pdu::REGISTER_NOTIFICATION, &[pdu::ERR_INVALID_PARAMETER]))),
                Some((_, id, _)) => out.push(reply(&vendor(ctype::REJECTED, id, &[pdu::ERR_INVALID_COMMAND]))),
                // Another company's, fragmented or cut short: not ours to answer.
                None => out.push(echo(ctype::NOT_IMPLEMENTED)),
            },
            _ => out.push(echo(ctype::NOT_IMPLEMENTED)),
        }
    }
}
