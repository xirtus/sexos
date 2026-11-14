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
#![no_std]
extern crate alloc;

pub mod controller;
pub mod crypto;
pub mod hci;
pub mod intel;
pub mod l2cap;
pub mod pack;
pub mod report;
pub mod smp;
pub mod ssp;
