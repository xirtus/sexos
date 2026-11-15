// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Controller bring-up and scanning, as a state machine with no I/O: it hands out one
//! command at a time and moves on when that command's Command Complete (or Command
//! Status) arrives. The server owns the transport; the tests drive it with a scripted
//! controller and compare the exact command sequence.

use crate::hci::{self, op, Addr, Event};
use alloc::vec::Vec;

/// Classic event mask: everything the host stack handles, including Simple Pairing,
/// Extended Inquiry Results and LE meta events (the mask BlueZ also sets by default).
pub const EVENT_MASK: u64 = 0x3DBF_F807_FFFB_FFFF;
/// LE meta sub-events 0x01-0x05, 0x0A (enhanced connection) and 0x0D (extended reports).
pub const LE_EVENT_MASK: u64 = 0x0000_0000_0000_121F;
/// General Inquiry Access Code.
pub const GIAC: [u8; 3] = [0x33, 0x8B, 0x9E];

/// What the controller said about itself during bring-up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub hci_version: u8,
    pub hci_revision: u16,
    pub lmp_version: u8,
    pub manufacturer: u16,
    pub lmp_subversion: u16,
    pub features: [u8; 8],
    pub le_features: [u8; 8],
    pub addr: Addr,
    pub acl_mtu: u16,
    pub acl_packets: u16,
    pub le_acl_mtu: u16,
    pub le_acl_packets: u8,
}

impl Info {
    pub fn le(&self) -> bool { self.features[4] & 0x40 != 0 }
    pub fn ssp(&self) -> bool { self.features[6] & 0x08 != 0 }
    pub fn rssi_inquiry(&self) -> bool { self.features[3] & 0x40 != 0 }
    pub fn eir(&self) -> bool { self.features[6] & 0x01 != 0 }
    pub fn le_extended_advertising(&self) -> bool { self.le_features[1] & 0x10 != 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A command came back with this status.
    Status { opcode: u16, status: u8 },
    /// A Command Complete whose return parameters are too short.
    Short { opcode: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Reset, ReadVersion, ReadFeatures, ReadBufferSize, ReadAddr, SetEventMask, SimplePairing,
    InquiryMode, ScanEnable, LeHost, LeEventMask, LeFeatures, LeBufferSize, Ready,
}

/// Bring-up: Reset, Read Local Version, Read Local Features, Read Buffer Size, Read
/// BD_ADDR, Set Event Mask, then (when supported) Write Simple Pairing Mode, Write
/// Inquiry Mode, Write Scan Enable (page scan), Write LE Host Supported, LE Set Event
/// Mask, LE Read Local Features and LE Read Buffer Size.
pub struct Bringup {
    step: Step,
    waiting: Option<u16>,
    pub info: Info,
}

impl Default for Bringup { fn default() -> Self { Self::new() } }

impl Bringup {
    pub fn new() -> Bringup { Bringup { step: Step::Reset, waiting: None, info: Info::default() } }
    pub fn ready(&self) -> bool { self.step == Step::Ready }

    fn skip(&self, s: Step) -> bool {
        match s {
            Step::SimplePairing => !self.info.ssp(),
            Step::InquiryMode => !self.info.rssi_inquiry() && !self.info.eir(),
            Step::LeHost | Step::LeEventMask | Step::LeFeatures | Step::LeBufferSize => !self.info.le(),
            _ => false,
        }
    }

    fn command_for(&self, s: Step) -> Vec<u8> {
        match s {
            Step::Reset => hci::command(op::RESET, &[]),
            Step::ReadVersion => hci::command(op::READ_LOCAL_VERSION, &[]),
            Step::ReadFeatures => hci::command(op::READ_LOCAL_FEATURES, &[]),
            Step::ReadBufferSize => hci::command(op::READ_BUFFER_SIZE, &[]),
            Step::ReadAddr => hci::command(op::READ_BD_ADDR, &[]),
            Step::SetEventMask => hci::command(op::SET_EVENT_MASK, &EVENT_MASK.to_le_bytes()),
            Step::SimplePairing => hci::command(op::WRITE_SIMPLE_PAIRING_MODE, &[1]),
            Step::InquiryMode => hci::command(op::WRITE_INQUIRY_MODE, &[if self.info.eir() { 2 } else { 1 }]),
            // Page scan only: connectable by devices we paired, not discoverable.
            Step::ScanEnable => hci::command(op::WRITE_SCAN_ENABLE, &[0x02]),
            Step::LeHost => hci::command(op::WRITE_LE_HOST_SUPPORTED, &[1, 0]),
            Step::LeEventMask => hci::command(op::LE_SET_EVENT_MASK, &LE_EVENT_MASK.to_le_bytes()),
            Step::LeFeatures => hci::command(op::LE_READ_LOCAL_FEATURES, &[]),
            Step::LeBufferSize => hci::command(op::LE_READ_BUFFER_SIZE, &[]),
            Step::Ready => Vec::new(),
        }
    }

    fn after(s: Step) -> Step {
        use Step::*;
        match s {
            Reset => ReadVersion, ReadVersion => ReadFeatures, ReadFeatures => ReadBufferSize,
            ReadBufferSize => ReadAddr, ReadAddr => SetEventMask, SetEventMask => SimplePairing,
            SimplePairing => InquiryMode, InquiryMode => ScanEnable, ScanEnable => LeHost,
            LeHost => LeEventMask, LeEventMask => LeFeatures, LeFeatures => LeBufferSize,
            LeBufferSize => Ready, Ready => Ready,
        }
    }

    /// The next command to send, if one is due (none while one is outstanding).
    pub fn next(&mut self) -> Option<Vec<u8>> {
        if self.waiting.is_some() || self.step == Step::Ready { return None; }
        while self.skip(self.step) { self.step = Self::after(self.step); }
        if self.step == Step::Ready { return None; }
        let c = self.command_for(self.step);
        self.waiting = hci::parse_command(&c).map(|(o, _)| o);
        Some(c)
    }

    /// Feed an event. Ok(true): bring-up is complete.
    pub fn on_event(&mut self, e: &Event) -> Result<bool, Error> {
        let Some(want) = self.waiting else { return Ok(self.ready()) };
        let Event::CommandComplete { opcode, ret, .. } = e else { return Ok(false) };
        if *opcode != want { return Ok(false); }
        let status = ret.first().copied().ok_or(Error::Short { opcode: want })?;
        if status != 0 { return Err(Error::Status { opcode: want, status }); }
        let need = |n: usize| if ret.len() >= n { Ok(()) } else { Err(Error::Short { opcode: want }) };
        let i = &mut self.info;
        match self.step {
            Step::ReadVersion => {
                need(9)?;
                i.hci_version = ret[1];
                i.hci_revision = u16::from_le_bytes([ret[2], ret[3]]);
                i.lmp_version = ret[4];
                i.manufacturer = u16::from_le_bytes([ret[5], ret[6]]);
                i.lmp_subversion = u16::from_le_bytes([ret[7], ret[8]]);
            }
            Step::ReadFeatures => { need(9)?; i.features.copy_from_slice(&ret[1..9]); }
            Step::ReadBufferSize => { need(8)?; i.acl_mtu = u16::from_le_bytes([ret[1], ret[2]]); i.acl_packets = u16::from_le_bytes([ret[4], ret[5]]); }
            Step::ReadAddr => { need(7)?; i.addr.0.copy_from_slice(&ret[1..7]); }
            Step::LeFeatures => { need(9)?; i.le_features.copy_from_slice(&ret[1..9]); }
            Step::LeBufferSize => { need(4)?; i.le_acl_mtu = u16::from_le_bytes([ret[1], ret[2]]); i.le_acl_packets = ret[3]; }
            _ => {}
        }
        // A controller sharing one buffer pool reports LE buffers as zero.
        if self.step == Step::LeBufferSize && self.info.le_acl_mtu == 0 {
            self.info.le_acl_mtu = self.info.acl_mtu;
            self.info.le_acl_packets = self.info.acl_packets.min(255) as u8;
        }
        self.waiting = None;
        self.step = Self::after(self.step);
        while self.step != Step::Ready && self.skip(self.step) { self.step = Self::after(self.step); }
        Ok(self.ready())
    }
}

/// The commands that start discovery on a ready controller: a classic inquiry (`length`
/// in units of 1.28 s) and an LE active scan, extended when the controller has it.
pub fn start_scan(info: &Info, length: u8) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    let mut inquiry = GIAC.to_vec();
    inquiry.extend_from_slice(&[length, 0]);
    v.push(hci::command(op::INQUIRY, &inquiry));
    if info.le() {
        // Active scan, 60 ms interval, 30 ms window (in 0.625 ms units), public own address, accept all.
        let (interval, window) = (0x0060u16.to_le_bytes(), 0x0030u16.to_le_bytes());
        if info.le_extended_advertising() {
            v.push(hci::command(op::LE_SET_EXTENDED_SCAN_PARAMETERS,
                &[0, 0, 0x01, 1, interval[0], interval[1], window[0], window[1]]));
            // Enable, filter duplicates, duration 0 (until disabled), period 0.
            v.push(hci::command(op::LE_SET_EXTENDED_SCAN_ENABLE, &[1, 1, 0, 0, 0, 0]));
        } else {
            v.push(hci::command(op::LE_SET_SCAN_PARAMETERS, &[1, interval[0], interval[1], window[0], window[1], 0, 0]));
            v.push(hci::command(op::LE_SET_SCAN_ENABLE, &[1, 1]));
        }
    }
    v
}

/// The commands that stop discovery.
pub fn stop_scan(info: &Info) -> Vec<Vec<u8>> {
    let mut v = alloc::vec![hci::command(op::INQUIRY_CANCEL, &[])];
    if info.le() {
        if info.le_extended_advertising() { v.push(hci::command(op::LE_SET_EXTENDED_SCAN_ENABLE, &[0, 0, 0, 0, 0, 0])); }
        else { v.push(hci::command(op::LE_SET_SCAN_ENABLE, &[0, 0])); }
    }
    v
}
