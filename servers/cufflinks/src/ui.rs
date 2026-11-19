// SPDX-License-Identifier: GPL-3.0-only
//! The desktop's Bluetooth panel, Cufflinks's side (`sex_pdx::bt_ui`): what the Shell
//! shows (controller phase, devices, a comparison waiting for the user, the last
//! outcome) goes into the region shared with the Shell; the Shell's commands come
//! back through it. A wake (`OP_EVENT`) tells a sleeping Shell to look: always for a
//! comparison or an outcome, for list changes only while its panel is open.
//!
//! Boot log: `[cufflinks.ui.route] ok=1 slot=1007` (or `ok=0`), `[cufflinks.ui.wake]
//! event=… ok=0` when the Shell cannot be reached (pairing falls back to the
//! first-setup rule in `pairing.rs`).

use sex_pdx::bt_ui::{self as u, Command, Outcome, Shared, Snapshot};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::serial_record as log;

/// Republish list changes at most this often (ticks).
const PUBLISH_TICKS: u64 = 100;

pub struct Desk {
    region: Option<Shared>,
    pub snap: Snapshot,
    dirty: bool,
    published: u64,
    /// The last wake reached the Shell (assumed until one fails).
    reachable: bool,
    serial: u32,
    ticket: u32,
    /// Discovery runs until then (ticks).
    pub scan_until: u64,
}

impl Desk {
    pub fn probe() -> Desk {
        let view = sex_pdx::memory_region_full_view(u::SLOT, QUERY_VIEW | READ | WRITE).ok()
            .filter(|v| v.length >= u::BYTES && v.virtual_address != 0);
        let region = view.map(|v| unsafe { Shared::at(v.virtual_address as *mut u8) });
        let snap = Snapshot { phase: u::NO_CONTROLLER, ..Snapshot::EMPTY };
        if let Some(r) = region { r.cufflinks_start(&snap); }
        log!("[cufflinks.ui.route] ok={} slot={}", region.is_some() as u8, u::SLOT);
        Desk { region, snap, dirty: false, published: 0, reachable: true, serial: 0, ticket: 0, scan_until: 0 }
    }

    /// The desktop can ask the user: a Shell attached to the region and our last
    /// wake reached it.
    pub fn prompt(&self) -> bool { self.reachable && self.region.is_some_and(|r| r.shell_attached()) }

    fn wake(&mut self, event: u64) -> bool {
        let (status, _) = sex_pdx::pdx_call(u::NOTIFY_SLOT, u::OP_EVENT, event, 0, 0);
        let ok = status == 0;
        if !ok && self.reachable { log!("[cufflinks.ui.wake] event={} ok=0 status={:#x}", event, status); }
        self.reachable = ok;
        ok
    }

    fn flush(&mut self, now: u64) {
        if let Some(r) = self.region { r.publish(&self.snap); }
        self.dirty = false;
        self.published = now;
    }

    pub fn phase(&mut self, phase: u8) {
        if self.snap.phase == phase { return; }
        self.snap.phase = phase;
        if phase != u::READY { self.snap.count = 0; self.snap.compare = None; self.snap.scanning = false; self.scan_until = 0; }
        self.flush(sex_pdx::get_ticks());
        if self.region.is_some_and(|r| r.shell_open()) { self.wake(u::EVENT_CHANGED); }
    }

    pub fn changed(&mut self, yes: bool) { self.dirty |= yes; }
    pub fn put(&mut self, addr: [u8; 6], kind: u8, name: Option<&[u8]>, rssi: Option<i8>, le: bool) {
        let c = self.snap.put(addr, kind, name, rssi, le);
        self.dirty |= c;
    }
    pub fn set(&mut self, addr: &[u8; 6], flags: u8, on: bool) {
        let c = self.snap.set(addr, flags, on);
        self.dirty |= c;
    }

    /// Show a comparison; Some(ticket) when the Shell was woken to ask the user.
    pub fn ask(&mut self, addr: [u8; 6], value: u32) -> Option<u32> {
        if !self.region.is_some_and(|r| r.shell_attached()) { return None; }
        self.ticket = self.ticket.wrapping_add(1).max(1);
        self.snap.compare = Some(u::Compare { ticket: self.ticket, addr, value });
        self.flush(sex_pdx::get_ticks());
        if self.wake(u::EVENT_COMPARE) { return Some(self.ticket); }
        self.snap.compare = None;
        self.flush(sex_pdx::get_ticks());
        None
    }

    /// The comparison was answered (or given up): the dialog goes away.
    pub fn settle(&mut self) {
        if self.snap.compare.take().is_some() { self.dirty = true; }
    }

    /// How a pairing ended, for the panel's status line.
    pub fn outcome(&mut self, addr: [u8; 6], ok: bool, reason: u8) {
        self.serial = self.serial.wrapping_add(1);
        self.snap.outcome = Some(Outcome { serial: self.serial, addr, ok, reason });
        self.snap.set(&addr, u::PAIRING, false);
        self.flush(sex_pdx::get_ticks());
        if self.region.is_some_and(|r| r.shell_attached()) { self.wake(u::EVENT_OUTCOME); }
    }

    /// Each pass: scanning ends on time, list changes go out (and wake an open panel).
    pub fn pump(&mut self, now: u64) {
        let scanning = now < self.scan_until;
        if scanning != self.snap.scanning { self.snap.scanning = scanning; self.dirty = true; }
        if !self.dirty || now.saturating_sub(self.published) < PUBLISH_TICKS { return; }
        self.flush(now);
        if self.region.is_some_and(|r| r.shell_open()) { self.wake(u::EVENT_CHANGED); }
    }

    pub fn command(&self) -> Option<Command> { self.region.and_then(|r| r.take_command()) }
}
