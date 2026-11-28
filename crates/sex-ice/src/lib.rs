// SPDX-License-Identifier: GPL-3.0-only
//! How a SexOS WebRTC peer finds a path to the other side: STUN (RFC 8489), ICE
//! (RFC 8445) with trickle and consent freshness (RFC 7675), and TURN (RFC 8656) over
//! UDP, TCP or TLS.
//!
//! No I/O, like the rest of the network protocol crates: the program (Kaleidoscope, a
//! run with `+net`) owns the UDP sockets (`sex_app::net::UdpSocket`) and any TCP or TLS
//! stream to a TURN server, feeds in what arrives with the time in milliseconds, and
//! sends what comes back. The network grant stays with the program.
//!
//! - `stun`: messages, MESSAGE-INTEGRITY (HMAC-SHA1), FINGERPRINT (CRC-32), the long-term
//!   key, ChannelData and stream framing.
//! - `turn`: one allocation on one server.
//! - `agent`: the ICE agent that gathers, checks, nominates and keeps consent.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod agent;
pub mod rand;
pub mod stun;
pub mod turn;

pub use agent::{Agent, Candidate, Config, Event, Kind, Received, Route, State, Transmit};
