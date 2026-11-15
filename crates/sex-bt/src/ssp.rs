// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Classic Secure Simple Pairing (Core 5.4, Vol 2 Part F 4.2, Vol 4 Part E 7.7): the
//! controller does the cryptography; the host answers its events. Link Key Request ->
//! a stored key or a negative reply; IO Capability Request -> our capabilities; User
//! Confirmation Request -> the user's yes/no on the six-digit value (numeric
//! comparison) or an automatic yes for Just Works; Link Key Notification -> a bond to keep.

use crate::hci::{self, op, Addr, Event};
use alloc::vec::Vec;

/// A link key kept for a paired device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bond { pub addr: Addr, pub key: [u8; 16], pub key_type: u8 }

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Send this HCI command packet.
    Command(Vec<u8>),
    /// Ask the user whether this value matches the other device; answer with `confirm`.
    Compare { addr: Addr, value: u32 },
    /// A new bond: store it (the server writes it to its settings file in W2).
    Bonded(Bond),
    /// Pairing finished with this HCI status (0 = success).
    Complete { addr: Addr, status: u8 },
}

pub struct Ssp {
    pub io: u8,
    /// Authentication requirements: 0x03 = MITM not required, dedicated bonding; 0x05
    /// = MITM required, general bonding.
    pub auth: u8,
    pub bonds: Vec<Bond>,
    peer_io: Option<(Addr, u8, u8)>,
}

impl Ssp {
    pub fn new(io: u8, auth: u8) -> Ssp { Ssp { io, auth, bonds: Vec::new(), peer_io: None } }

    /// Answer one event; empty when the event is not about pairing.
    pub fn on_event(&mut self, e: &Event) -> Vec<Step> {
        let mut out = Vec::new();
        match *e {
            Event::LinkKeyRequest { addr } => match self.bonds.iter().find(|b| b.addr == addr) {
                Some(b) => {
                    let mut p = addr.0.to_vec();
                    p.extend_from_slice(&b.key);
                    out.push(Step::Command(hci::command(op::LINK_KEY_REQUEST_REPLY, &p)));
                }
                None => out.push(Step::Command(hci::command(op::LINK_KEY_REQUEST_NEGATIVE_REPLY, &addr.0))),
            },
            Event::IoCapabilityRequest { addr } => {
                let mut p = addr.0.to_vec();
                p.extend_from_slice(&[self.io, 0, self.auth]);
                out.push(Step::Command(hci::command(op::IO_CAPABILITY_REQUEST_REPLY, &p)));
            }
            Event::IoCapabilityResponse { addr, io, auth, .. } => self.peer_io = Some((addr, io, auth)),
            Event::UserConfirmationRequest { addr, value } => {
                // Numeric comparison needs both sides able to show and answer; otherwise
                // it is Just Works and the controller only wants a yes.
                let peer = self.peer_io.filter(|p| p.0 == addr).map(|p| p.1).unwrap_or(3);
                let both = |c: u8| c == 1 || c == 4;
                if both(self.io) && both(peer) { out.push(Step::Compare { addr, value }); }
                else { out.push(Step::Command(hci::command(op::USER_CONFIRMATION_REQUEST_REPLY, &addr.0))); }
            }
            Event::LinkKeyNotification { addr, key, key_type } => {
                let b = Bond { addr, key, key_type };
                self.bonds.retain(|x| x.addr != addr);
                self.bonds.push(b);
                out.push(Step::Bonded(b));
            }
            Event::SimplePairingComplete { status, addr } => out.push(Step::Complete { addr, status }),
            _ => {}
        }
        out
    }

    /// The user's answer to `Step::Compare`.
    pub fn confirm(&self, addr: Addr, matches: bool) -> Vec<u8> {
        hci::command(if matches { op::USER_CONFIRMATION_REQUEST_REPLY } else { op::USER_CONFIRMATION_REQUEST_NEGATIVE_REPLY }, &addr.0)
    }

    /// Start pairing (authentication) on a classic connection.
    pub fn authenticate(handle: u16) -> Vec<u8> { hci::command(op::AUTHENTICATION_REQUESTED, &handle.to_le_bytes()) }
}
