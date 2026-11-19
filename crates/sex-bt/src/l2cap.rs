// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! L2CAP basic mode (Core 5.4, Vol 3 Part A): frames on fixed channels carried in ACL
//! packets, fragmentation and reassembly, and signaling: LE connection parameter
//! updates, classic information/echo requests, and classic connection-oriented channels
//! in basic mode (`Channels`: HID's PSM 0x11/0x13, opened by us or by a bonded device).
//! LE credit-based channels are refused (HID over GATT needs only ATT).

use crate::hci;
use alloc::vec::Vec;

/// Fixed channel ids.
pub const CID_SIGNALING: u16 = 0x0001;
pub const CID_ATT: u16 = 0x0004;
pub const CID_LE_SIGNALING: u16 = 0x0005;
pub const CID_SMP: u16 = 0x0006;
pub const CID_SMP_BREDR: u16 = 0x0007;

/// A basic L2CAP frame: length, channel, payload.
pub fn frame(cid: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(4 + payload.len());
    f.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    f.extend_from_slice(&cid.to_le_bytes());
    f.extend_from_slice(payload);
    f
}

/// (channel, payload) of a whole frame.
pub fn parse_frame(f: &[u8]) -> Option<(u16, &[u8])> {
    if f.len() < 4 { return None; }
    let len = u16::from_le_bytes([f[0], f[1]]) as usize;
    if f.len() != 4 + len { return None; }
    Some((u16::from_le_bytes([f[2], f[3]]), &f[4..]))
}

/// ACL packets (with type bytes) carrying one frame, each at most `mtu` data bytes.
pub fn fragment(handle: u16, frame: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let mtu = mtu.max(1);
    frame.chunks(mtu).enumerate()
        .map(|(i, c)| hci::acl(handle, if i == 0 { hci::PB_FIRST_NON_FLUSHABLE } else { hci::PB_CONTINUING }, c))
        .collect()
}

/// Reassembles frames on one connection from its ACL packets.
#[derive(Default)]
pub struct Reassembly { buf: Vec<u8>, want: usize }

impl Reassembly {
    /// Feed one ACL packet's data; a whole frame when one completes.
    pub fn push(&mut self, pb: u8, data: &[u8]) -> Option<Vec<u8>> {
        if pb == hci::PB_CONTINUING {
            if self.want == 0 { return None; } // a continuation without a start
            self.buf.extend_from_slice(data);
        } else {
            self.buf.clear();
            self.buf.extend_from_slice(data);
            if data.len() < 2 { self.want = 0; self.buf.clear(); return None; }
            self.want = 4 + u16::from_le_bytes([data[0], data[1]]) as usize;
        }
        if self.want != 0 && self.buf.len() >= self.want {
            let mut f = core::mem::take(&mut self.buf);
            f.truncate(self.want);
            self.want = 0;
            return Some(f);
        }
        None
    }
}

/// Signaling command codes.
pub mod sig {
    pub const COMMAND_REJECT: u8 = 0x01;
    pub const CONNECTION_REQUEST: u8 = 0x02;
    pub const CONNECTION_RESPONSE: u8 = 0x03;
    pub const ECHO_REQUEST: u8 = 0x08;
    pub const ECHO_RESPONSE: u8 = 0x09;
    pub const INFORMATION_REQUEST: u8 = 0x0A;
    pub const INFORMATION_RESPONSE: u8 = 0x0B;
    pub const CONNECTION_PARAMETER_UPDATE_REQUEST: u8 = 0x12;
    pub const CONNECTION_PARAMETER_UPDATE_RESPONSE: u8 = 0x13;
}

pub fn command(code: u8, id: u8, data: &[u8]) -> Vec<u8> {
    let mut c = alloc::vec![code, id];
    c.extend_from_slice(&(data.len() as u16).to_le_bytes());
    c.extend_from_slice(data);
    c
}

/// What a signaling frame asks of us.
#[derive(Debug, PartialEq, Eq)]
pub enum Signal {
    /// Send this payload back on the same signaling channel.
    Reply(Vec<u8>),
    /// The peer wants new LE connection parameters (interval min/max, latency, timeout),
    /// which we accepted; the server issues the HCI connection update.
    Accepted { min: u16, max: u16, latency: u16, timeout: u16, reply: Vec<u8> },
    Nothing,
}

/// Answer one signaling payload (classic channel 1 or LE channel 5).
pub fn signal(cid: u16, payload: &[u8]) -> Signal {
    if payload.len() < 4 { return Signal::Nothing; }
    let (code, id) = (payload[0], payload[1]);
    let len = u16::from_le_bytes([payload[2], payload[3]]) as usize;
    let Some(data) = payload.get(4..4 + len) else { return Signal::Nothing };
    let le16 = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    match code {
        sig::CONNECTION_PARAMETER_UPDATE_REQUEST if cid == CID_LE_SIGNALING && data.len() >= 8 => {
            let (min, max, latency, timeout) = (le16(0), le16(2), le16(4), le16(6));
            let ok = min >= 6 && min <= max && max <= 3200 && latency <= 499 && (10..=3200).contains(&timeout);
            let reply = command(sig::CONNECTION_PARAMETER_UPDATE_RESPONSE, id, &(!ok as u16).to_le_bytes());
            if ok { Signal::Accepted { min, max, latency, timeout, reply } } else { Signal::Reply(reply) }
        }
        sig::ECHO_REQUEST if cid == CID_SIGNALING => Signal::Reply(command(sig::ECHO_RESPONSE, id, data)),
        sig::INFORMATION_REQUEST if cid == CID_SIGNALING && data.len() >= 2 => {
            let kind = le16(0);
            let mut r = kind.to_le_bytes().to_vec();
            match kind {
                // Extended features: fixed channels supported (bit 7).
                2 => { r.extend_from_slice(&[0, 0]); r.extend_from_slice(&0x0000_0080u32.to_le_bytes()); }
                // Fixed channels: signaling and BR/EDR security manager.
                3 => { r.extend_from_slice(&[0, 0]); r.extend_from_slice(&((1u64 << CID_SIGNALING) | (1u64 << CID_SMP_BREDR)).to_le_bytes()); }
                _ => r.extend_from_slice(&[1, 0]), // not supported
            }
            Signal::Reply(command(sig::INFORMATION_RESPONSE, id, &r))
        }
        // No dynamic channels in W1: refuse with "PSM not supported".
        sig::CONNECTION_REQUEST if cid == CID_SIGNALING && data.len() >= 4 => {
            let scid = le16(2);
            let mut r = Vec::new();
            r.extend_from_slice(&0u16.to_le_bytes());
            r.extend_from_slice(&scid.to_le_bytes());
            r.extend_from_slice(&2u16.to_le_bytes());
            r.extend_from_slice(&0u16.to_le_bytes());
            Signal::Reply(command(sig::CONNECTION_RESPONSE, id, &r))
        }
        // Responses and rejects need no answer.
        c if c & 1 == 1 => Signal::Nothing,
        // Anything else: Command Reject, "command not understood".
        _ => Signal::Reply(command(sig::COMMAND_REJECT, id, &[0, 0])),
    }
}

/// More signaling codes, for connection-oriented channels.
pub mod sig2 {
    pub const CONFIGURATION_REQUEST: u8 = 0x04;
    pub const CONFIGURATION_RESPONSE: u8 = 0x05;
    pub const DISCONNECTION_REQUEST: u8 = 0x06;
    pub const DISCONNECTION_RESPONSE: u8 = 0x07;
}

/// The signaling commands in one frame (classic frames may carry several): (code, id, data).
pub fn commands(payload: &[u8]) -> Vec<(u8, u8, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= payload.len() {
        let len = u16::from_le_bytes([payload[at + 2], payload[at + 3]]) as usize;
        let Some(data) = payload.get(at + 4..at + 4 + len) else { break };
        out.push((payload[at], payload[at + 1], data));
        at += 4 + len;
    }
    out
}

/// A connection-oriented channel in basic mode (classic): PSM, our and the peer's channel ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chan { pub psm: u16, pub local: u16, pub remote: u16, ours_done: bool, theirs_done: bool, pub open: bool,
    /// The largest payload the peer takes on this channel (its MTU option; 672 by default).
    pub mtu: u16 }

/// The MTU a channel has when its configuration names none.
pub const DEFAULT_MTU: u16 = 672;
/// The smallest MTU a classic channel may have.
pub const MIN_MTU: u16 = 48;

/// What happened to the channels.
#[derive(Debug, PartialEq, Eq)]
pub enum ChanEvent {
    /// Send this signaling payload on channel 1.
    Send(Vec<u8>),
    /// Both sides configured: data may flow.
    Open { psm: u16, local: u16 },
    Closed { psm: u16, local: u16 },
}

/// L2CAP connection result codes.
pub const PSM_NOT_SUPPORTED: u16 = 0x0002;
pub const SECURITY_BLOCK: u16 = 0x0003;

/// The dynamic channels of one classic connection: we open channels (connection request,
/// then configuration both ways, no options: defaults) and accept the peer's to PSMs we
/// allow. Results other than success close the channel.
#[derive(Default)]
pub struct Channels { pub chans: Vec<Chan>, next_cid: u16, next_id: u8 }

impl Channels {
    fn cid(&mut self) -> u16 {
        if self.next_cid < 0x0040 { self.next_cid = 0x0040; }
        let c = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1).max(0x0040);
        c
    }
    fn id(&mut self) -> u8 { self.next_id = self.next_id.wrapping_add(1).max(1); self.next_id }
    pub fn by_local(&self, cid: u16) -> Option<&Chan> { self.chans.iter().find(|c| c.local == cid) }
    pub fn by_psm(&self, psm: u16) -> Option<&Chan> { self.chans.iter().find(|c| c.psm == psm) }

    fn config_request(&mut self, remote: u16) -> Vec<u8> {
        let id = self.id();
        let mut d = remote.to_le_bytes().to_vec();
        d.extend_from_slice(&[0, 0]);
        command(sig2::CONFIGURATION_REQUEST, id, &d)
    }

    /// Open a channel to `psm`; the signaling payload to send.
    pub fn connect(&mut self, psm: u16) -> Vec<u8> {
        let local = self.cid();
        self.chans.push(Chan { psm, local, remote: 0, ours_done: false, theirs_done: false, open: false, mtu: DEFAULT_MTU });
        let id = self.id();
        let mut d = psm.to_le_bytes().to_vec();
        d.extend_from_slice(&local.to_le_bytes());
        command(sig::CONNECTION_REQUEST, id, &d)
    }

    /// Close a channel: the signaling payload to send (the channel goes when the peer answers).
    pub fn disconnect(&mut self, local: u16) -> Option<Vec<u8>> {
        let c = *self.by_local(local)?;
        let id = self.id();
        let mut d = c.remote.to_le_bytes().to_vec();
        d.extend_from_slice(&c.local.to_le_bytes());
        Some(command(sig2::DISCONNECTION_REQUEST, id, &d))
    }

    fn opened(&mut self, at: usize, out: &mut Vec<ChanEvent>) {
        let c = &mut self.chans[at];
        if c.ours_done && c.theirs_done && !c.open { c.open = true; out.push(ChanEvent::Open { psm: c.psm, local: c.local }); }
    }

    /// One signaling command; None when it is not about dynamic channels (answer it with
    /// `signal`). `allow(psm)` decides the peer's connection requests: Ok to accept, or the
    /// refusal result.
    pub fn on_command(&mut self, code: u8, id: u8, data: &[u8], allow: &dyn Fn(u16) -> Result<(), u16>) -> Option<Vec<ChanEvent>> {
        let le16 = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let mut out = Vec::new();
        match code {
            sig::CONNECTION_REQUEST => {
                let (psm, scid) = (le16(0)?, le16(2)?);
                let mut r = Vec::new();
                match allow(psm) {
                    Ok(()) => {
                        let local = self.cid();
                        self.chans.retain(|c| c.psm != psm || c.open);
                        self.chans.push(Chan { psm, local, remote: scid, ours_done: false, theirs_done: false, open: false, mtu: DEFAULT_MTU });
                        r.extend_from_slice(&local.to_le_bytes());
                        r.extend_from_slice(&scid.to_le_bytes());
                        r.extend_from_slice(&[0, 0, 0, 0]);
                        out.push(ChanEvent::Send(command(sig::CONNECTION_RESPONSE, id, &r)));
                        let c = self.config_request(scid);
                        out.push(ChanEvent::Send(c));
                    }
                    Err(result) => {
                        r.extend_from_slice(&[0, 0]);
                        r.extend_from_slice(&scid.to_le_bytes());
                        r.extend_from_slice(&result.to_le_bytes());
                        r.extend_from_slice(&[0, 0]);
                        out.push(ChanEvent::Send(command(sig::CONNECTION_RESPONSE, id, &r)));
                    }
                }
            }
            sig::CONNECTION_RESPONSE => {
                let (dcid, scid, result) = (le16(0)?, le16(2)?, le16(4)?);
                let at = self.chans.iter().position(|c| c.local == scid)?;
                match result {
                    0 => {
                        self.chans[at].remote = dcid;
                        let c = self.config_request(dcid);
                        out.push(ChanEvent::Send(c));
                    }
                    1 => {} // pending: wait for the final response
                    _ => { let c = self.chans.remove(at); out.push(ChanEvent::Closed { psm: c.psm, local: c.local }); }
                }
            }
            sig2::CONFIGURATION_REQUEST => {
                let dcid = le16(0)?;
                let Some(at) = self.chans.iter().position(|c| c.local == dcid) else {
                    // Unknown channel: Command Reject, invalid CID.
                    let mut r = 0x0002u16.to_le_bytes().to_vec();
                    r.extend_from_slice(&dcid.to_le_bytes());
                    r.extend_from_slice(&[0, 0]);
                    out.push(ChanEvent::Send(command(sig::COMMAND_REJECT, id, &r)));
                    return Some(out);
                };
                // Accept whatever the peer proposes (MTU, flush timeout, QoS): success, no
                // options. Its MTU option bounds what we send (at least the minimum).
                let mut o = 4;
                while let (Some(&kind), Some(&len)) = (data.get(o), data.get(o + 1)) {
                    let Some(v) = data.get(o + 2..o + 2 + len as usize) else { break };
                    if kind & 0x7F == 0x01 && len == 2 { self.chans[at].mtu = u16::from_le_bytes([v[0], v[1]]).max(MIN_MTU); }
                    o += 2 + len as usize;
                }
                let mut r = self.chans[at].remote.to_le_bytes().to_vec();
                r.extend_from_slice(&[0, 0, 0, 0]);
                out.push(ChanEvent::Send(command(sig2::CONFIGURATION_RESPONSE, id, &r)));
                // A continuation flag means more options follow: done only on the last.
                if le16(2)? & 1 == 0 { self.chans[at].theirs_done = true; }
                self.opened(at, &mut out);
            }
            sig2::CONFIGURATION_RESPONSE => {
                let (scid, result) = (le16(0)?, le16(4)?);
                let at = self.chans.iter().position(|c| c.local == scid)?;
                if result == 0 { self.chans[at].ours_done = true; self.opened(at, &mut out); }
                else { let c = self.chans.remove(at); out.push(ChanEvent::Closed { psm: c.psm, local: c.local }); }
            }
            sig2::DISCONNECTION_REQUEST => {
                let (dcid, scid) = (le16(0)?, le16(2)?);
                let mut r = dcid.to_le_bytes().to_vec();
                r.extend_from_slice(&scid.to_le_bytes());
                out.push(ChanEvent::Send(command(sig2::DISCONNECTION_RESPONSE, id, &r)));
                if let Some(at) = self.chans.iter().position(|c| c.local == dcid) {
                    let c = self.chans.remove(at);
                    out.push(ChanEvent::Closed { psm: c.psm, local: c.local });
                }
            }
            sig2::DISCONNECTION_RESPONSE => {
                let scid = le16(2)?;
                if let Some(at) = self.chans.iter().position(|c| c.local == scid) {
                    let c = self.chans.remove(at);
                    out.push(ChanEvent::Closed { psm: c.psm, local: c.local });
                }
            }
            _ => return None,
        }
        Some(out)
    }
}
