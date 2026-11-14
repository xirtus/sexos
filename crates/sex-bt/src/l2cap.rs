// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! L2CAP basic mode (Core 5.4, Vol 3 Part A): frames on fixed channels carried in ACL
//! packets, fragmentation and reassembly, and the signaling the W1 host needs (LE
//! connection parameter updates, classic information/echo requests, and refusing
//! connection-oriented channels until W2 brings HID and GATT services).

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

fn command(code: u8, id: u8, data: &[u8]) -> Vec<u8> {
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
