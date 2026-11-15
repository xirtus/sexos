// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! sex-bt: the core of Cufflinks, SexOS's own Bluetooth host stack (no BlueZ, no Linux
//! bridge). Everything here is transport-free state machines and codecs over HCI
//! packets with their one-byte type, so the same code runs in `servers/cufflinks` on
//! the USB ring from SexUSB and in host tests against scripted controllers.
//!
//! - `hci`: command/event/ACL encoding and decoding.
//! - `controller`: bring-up (reset, version, features, address, event masks, scan
//!   enable, LE setup) and the scan commands.
//! - `report`: inquiry results and LE advertising reports, AD/EIR structures.
//! - `intel`: Intel USB parts and their firmware load (Secure Send, reset, DDC).
//! - `l2cap`: basic frames, fragmentation and reassembly, signaling.
//! - `smp` + `crypto`: LE Secure Connections pairing (P-256, AES-CMAC f4/f5/f6/g2).
//! - `ssp`: classic Secure Simple Pairing event flow.
//! - `pack`: firmware pack lookup.
//! - `att` + `gatt`: ATT client codec and HID over GATT (HOGP) discovery and reports.
//! - `hid`: boot reports and report-map parsing into keys and pointer motion.
//! - `hidp`: classic HID (L2CAP PSM 0x11/0x13, boot protocol).
//! - `bonds`: LE (LTK, IRK, identity) and classic (link key) bonds and their stored form.
//! - `sdp`: service discovery (client for the A2DP Sink record, server for our Source).
//! - `avdtp`: A2DP stream setup as source (initiator and acceptor).
//! - `sbc`: the SBC encoder (A2DP's mandatory codec), frame header and CRC.
//! - `a2dp`: SBC frames in RTP media packets.
//! - `host`: all of the above composed into one packet-in/packet-out host stack.
#![no_std]
extern crate alloc;

pub mod a2dp;
pub mod att;
pub mod avdtp;
pub mod bonds;
pub mod controller;
pub mod crypto;
pub mod hci;
pub mod gatt;
pub mod hid;
pub mod hidp;
pub mod host;
pub mod intel;
pub mod l2cap;
pub mod pack;
pub mod report;
pub mod sbc;
pub mod sdp;
pub mod smp;
pub mod ssp;
