// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! HID over GATT (HOGP 1.0), host role: a GATT client that finds the HID service
//! (0x1812) on an LE keyboard or mouse, reads its report map and report references,
//! turns on notifications for its input reports and decodes them.
//!
//! Order: Exchange MTU, primary services (Read By Group Type), the HID service's
//! characteristics (Read By Type 0x2803), the descriptors of its reports (Find
//! Information: CCCD 0x2902, Report Reference 0x2908), Report Map (Read, then Read Blob
//! while it fills the MTU), each input report's reference, then a CCCD write per input
//! report. A device whose map has no keyboard or mouse report but that offers the boot
//! reports (0x2A22, 0x2A33) is put in boot protocol (Protocol Mode 0x2A4E = 0) instead.
//! Run it on an encrypted link: HID devices refuse their attributes otherwise.

use crate::att::{self, uuid, Pdu};
use crate::hid::{self, Input, ReportMap};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Char { decl: u16, value: u16, props: u8, uuid: u16, end: u16, cccd: u16, reference: u16 }

/// An input report we listen to: its value handle, report id and CCCD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report { pub handle: u16, pub id: u8, pub cccd: u16, pub boot: Boot }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boot { No, Keyboard, Mouse }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State { Idle, Mtu, Services, Chars, Descriptors, Map, References, Enable, Ready, Failed }

/// What the client wants done.
#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    /// Send this ATT PDU on channel 4.
    Send(Vec<u8>),
    /// Discovery finished: input reports are on.
    Ready { keyboard: bool, pointer: bool, boot: bool, reports: usize },
    /// A decoded input report.
    Input(Input),
    /// Discovery stopped: no HID service, or the server refused (ATT error code).
    Failed(u8),
}

/// No HID service on the device.
pub const NO_HID: u8 = 0xF0;

pub struct Hogp {
    state: State,
    pub mtu: u16,
    services: Vec<(u16, u16)>,
    service: usize,
    chars: Vec<Char>,
    at: usize,
    next: u16,
    map_bytes: Vec<u8>,
    pub map: ReportMap,
    pub reports: Vec<Report>,
    boot: bool,
}

impl Default for Hogp { fn default() -> Self { Self::new() } }

impl Hogp {
    pub fn new() -> Hogp {
        Hogp { state: State::Idle, mtu: att::DEFAULT_MTU, services: Vec::new(), service: 0, chars: Vec::new(), at: 0, next: 1,
            map_bytes: Vec::new(), map: ReportMap::default(), reports: Vec::new(), boot: false }
    }
    pub fn ready(&self) -> bool { self.state == State::Ready }

    pub fn start(&mut self) -> Vec<Out> {
        self.state = State::Mtu;
        alloc::vec![Out::Send(att::exchange_mtu(att::OUR_MTU))]
    }

    fn fail(&mut self, code: u8) -> Vec<Out> { self.state = State::Failed; alloc::vec![Out::Failed(code)] }

    fn services(&mut self) -> Vec<Out> {
        self.state = State::Services;
        alloc::vec![Out::Send(att::read_by_group_type(self.next, 0xFFFF, uuid::PRIMARY_SERVICE))]
    }

    /// Characteristics of the current HID service, from `self.next`.
    fn chars(&mut self) -> Vec<Out> {
        while let Some(&(_, end)) = self.services.get(self.service) {
            if self.next <= end {
                self.state = State::Chars;
                return alloc::vec![Out::Send(att::read_by_type(self.next, end, uuid::CHARACTERISTIC))];
            }
            self.service += 1;
            if let Some(&(start, _)) = self.services.get(self.service) { self.next = start; }
        }
        // Each characteristic ends where the next declaration starts (or its service ends).
        for i in 0..self.chars.len() {
            let service_end = self.services.iter().find(|s| s.0 <= self.chars[i].decl && self.chars[i].decl <= s.1).map(|s| s.1).unwrap_or(0xFFFF);
            let next = self.chars.get(i + 1).map(|c| c.decl.saturating_sub(1)).unwrap_or(service_end);
            self.chars[i].end = next.min(service_end);
        }
        self.at = 0;
        self.next = 0;
        self.descriptors()
    }

    fn wants_descriptors(c: &Char) -> bool {
        matches!(c.uuid, uuid::REPORT | uuid::BOOT_KEYBOARD_INPUT | uuid::BOOT_MOUSE_INPUT) && c.end > c.value
    }

    fn descriptors(&mut self) -> Vec<Out> {
        while let Some(c) = self.chars.get(self.at).copied() {
            if Self::wants_descriptors(&c) {
                let start = if self.next > c.value { self.next } else { c.value + 1 };
                if start <= c.end {
                    self.state = State::Descriptors;
                    return alloc::vec![Out::Send(att::find_information(start, c.end))];
                }
            }
            self.at += 1;
            self.next = 0;
        }
        self.read_map()
    }

    fn read_map(&mut self) -> Vec<Out> {
        match self.chars.iter().find(|c| c.uuid == uuid::REPORT_MAP) {
            Some(c) => {
                self.state = State::Map;
                let h = c.value;
                alloc::vec![Out::Send(if self.map_bytes.is_empty() { att::read(h) } else { att::read_blob(h, self.map_bytes.len() as u16) })]
            }
            None => { self.at = 0; self.references() }
        }
    }

    fn references(&mut self) -> Vec<Out> {
        if self.state != State::References {
            self.map = ReportMap::parse(&self.map_bytes);
            self.state = State::References;
            self.at = 0;
        }
        while let Some(c) = self.chars.get(self.at).copied() {
            self.at += 1;
            // Notifying reports with a reference: read it (id, type).
            if c.uuid == uuid::REPORT && c.reference != 0 && c.cccd != 0 && c.props & 0x10 != 0 {
                self.at -= 1;
                return alloc::vec![Out::Send(att::read(c.reference))];
            }
        }
        self.enable_plan()
    }

    /// Choose report or boot protocol, then turn notifications on.
    fn enable_plan(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        let usable = self.reports.iter().any(|r| self.map.layouts.iter().any(|l| l.id == r.id));
        if !usable {
            self.reports.clear();
            for c in self.chars.iter().filter(|c| c.cccd != 0) {
                let boot = match c.uuid { uuid::BOOT_KEYBOARD_INPUT => Boot::Keyboard, uuid::BOOT_MOUSE_INPUT => Boot::Mouse, _ => continue };
                self.reports.push(Report { handle: c.value, id: 0, cccd: c.cccd, boot });
            }
            if self.reports.is_empty() { return self.fail(NO_HID); }
            self.boot = true;
            if let Some(pm) = self.chars.iter().find(|c| c.uuid == uuid::PROTOCOL_MODE) {
                out.push(Out::Send(att::write_command(pm.value, &[0])));
            }
        }
        self.state = State::Enable;
        self.at = 0;
        out.extend(self.enable_next());
        out
    }

    fn enable_next(&mut self) -> Vec<Out> {
        if let Some(r) = self.reports.get(self.at) {
            return alloc::vec![Out::Send(att::write(r.cccd, &[1, 0]))];
        }
        self.state = State::Ready;
        let keyboard = if self.boot { self.reports.iter().any(|r| r.boot == Boot::Keyboard) } else { self.map.layouts.iter().any(|l| l.keyboard()) };
        let pointer = if self.boot { self.reports.iter().any(|r| r.boot == Boot::Mouse) } else { self.map.layouts.iter().any(|l| l.pointer()) };
        alloc::vec![Out::Ready { keyboard, pointer, boot: self.boot, reports: self.reports.len() }]
    }

    /// One ATT PDU from the device.
    pub fn on_pdu(&mut self, p: &[u8]) -> Vec<Out> {
        let Some(pdu) = att::parse(p) else { return Vec::new() };
        if let Pdu::Notification { handle, value } = pdu {
            if self.state != State::Ready { return Vec::new(); }
            let Some(r) = self.reports.iter().find(|r| r.handle == handle) else { return Vec::new() };
            let input = match r.boot {
                Boot::Keyboard => hid::boot_keyboard(value),
                Boot::Mouse => hid::boot_mouse(value),
                Boot::No => {
                    // Report characteristics carry the report without its id byte.
                    let id = if self.map.layouts.iter().all(|l| l.id == 0) { 0 } else { r.id };
                    self.map.decode(id, value)
                }
            };
            return input.map(|i| alloc::vec![Out::Input(i)]).unwrap_or_default();
        }
        match (self.state, pdu) {
            (State::Mtu, Pdu::Mtu(m)) => { self.mtu = m.clamp(att::DEFAULT_MTU, att::OUR_MTU); self.next = 1; self.services() }
            (State::Mtu, Pdu::Error { .. }) => { self.next = 1; self.services() }
            (State::Services, Pdu::Groups(entries)) => {
                let mut last = 0xFFFF;
                for (start, end, value) in entries {
                    if att::uuid16(value) == Some(uuid::HID_SERVICE) { self.services.push((start, end)); }
                    last = end;
                }
                if last == 0xFFFF { return self.services_done(); }
                self.next = last + 1;
                self.services()
            }
            (State::Services, Pdu::Error { code: att::error::ATTRIBUTE_NOT_FOUND, .. }) => self.services_done(),
            (State::Chars, Pdu::Types(entries)) => {
                for (decl, v) in entries {
                    if v.len() < 5 { continue; }
                    let uuid = att::uuid16(&v[3..]).unwrap_or(0);
                    self.chars.push(Char { decl, value: u16::from_le_bytes([v[1], v[2]]), props: v[0], uuid, ..Char::default() });
                    self.next = decl.saturating_add(1);
                }
                self.chars()
            }
            (State::Chars, Pdu::Error { .. }) => { self.next = 0xFFFF; self.service_advance() }
            (State::Descriptors, Pdu::Information(entries)) => {
                let c = &mut self.chars[self.at];
                let mut last = c.end;
                for (h, u) in entries {
                    match att::uuid16(u) { Some(uuid::CCCD) => c.cccd = h, Some(uuid::REPORT_REFERENCE) => c.reference = h, _ => {} }
                    last = h;
                }
                self.next = last.saturating_add(1);
                if last >= c.end { self.at += 1; self.next = 0; }
                self.descriptors()
            }
            (State::Descriptors, Pdu::Error { .. }) => { self.at += 1; self.next = 0; self.descriptors() }
            (State::Map, Pdu::Read(bytes)) => {
                self.map_bytes.extend_from_slice(bytes);
                if bytes.len() + 1 >= self.mtu as usize && self.map_bytes.len() < 4096 { return self.read_map(); }
                self.at = 0;
                self.references()
            }
            (State::Map, Pdu::Error { code, .. }) => {
                // A map that fit in one read answers Read Blob with "not long".
                if code == att::error::ATTRIBUTE_NOT_LONG || !self.map_bytes.is_empty() { self.at = 0; return self.references(); }
                self.fail(code)
            }
            (State::References, Pdu::Read(bytes)) => {
                let c = self.chars[self.at];
                self.at += 1;
                // Type 1: input report.
                if bytes.len() >= 2 && bytes[1] == 1 { self.reports.push(Report { handle: c.value, id: bytes[0], cccd: c.cccd, boot: Boot::No }); }
                self.references()
            }
            (State::References, Pdu::Error { .. }) => { self.at += 1; self.references() }
            (State::Enable, Pdu::Written) | (State::Enable, Pdu::Error { .. }) => { self.at += 1; self.enable_next() }
            _ => Vec::new(),
        }
    }

    fn service_advance(&mut self) -> Vec<Out> {
        self.service += 1;
        if let Some(&(start, _)) = self.services.get(self.service) { self.next = start; }
        self.chars()
    }

    fn services_done(&mut self) -> Vec<Out> {
        if self.services.is_empty() { return self.fail(NO_HID); }
        self.service = 0;
        self.next = self.services[0].0;
        self.chars()
    }
}
