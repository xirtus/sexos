// SPDX-License-Identifier: GPL-3.0-only
//! The firmware connection manager (ICM) on integrated Intel controllers: the
//! driver says it is ready, the firmware reports the devices plugged in (also
//! those already there at boot), and the driver approves the ones the user
//! trusts. The firmware builds the PCIe, DisplayPort and USB tunnels itself.
//!
//! Messages are 32-bit words; the first is the header: code (7:0), flags (15:8),
//! packet id (23:16), total packets (31:24). Layouts follow the Titan Ridge /
//! Ice Lake protocol that Tiger Lake speaks (Linux `tb_msgs.h`).

use crate::frame::{self, Words, PDF_ERROR, PDF_ICM_COMMAND, PDF_ICM_EVENT, PDF_ICM_RESPONSE, PDF_READ};
use crate::regs::{self, FRAME_SIZE};
use crate::ring::{Layout, Ring};
use crate::Nhi;

/// Commands.
pub const DRIVER_READY: u8 = 0x3;
pub const APPROVE_DEVICE: u8 = 0x4;
pub const CHALLENGE_DEVICE: u8 = 0x5;
pub const ADD_DEVICE_KEY: u8 = 0x6;
pub const PREBOOT_ACL: u8 = 0x18;
/// Events.
pub const EVENT_DEVICE_CONNECTED: u8 = 0x3;
pub const EVENT_DEVICE_DISCONNECTED: u8 = 0x4;
pub const EVENT_XDOMAIN_CONNECTED: u8 = 0x6;
pub const EVENT_XDOMAIN_DISCONNECTED: u8 = 0x7;
pub const EVENT_DP_CONFIG_CHANGED: u8 = 0x8;
pub const EVENT_RTD3_VETO: u8 = 0xa;

pub const FLAGS_ERROR: u8 = 1 << 0;
pub const FLAGS_NO_KEY: u8 = 1 << 1;
pub const FLAGS_DUAL_LANE: u8 = 1 << 5;
pub const FLAGS_SPEED_GEN3: u8 = 1 << 7;

pub const LINK_INFO_APPROVED: u16 = 1 << 8;
pub const LINK_INFO_REJECTED: u16 = 1 << 9;
pub const LINK_INFO_BOOT: u16 = 1 << 10;

/// Timeouts and retries (Linux `ICM_TIMEOUT`, `ICM_APPROVE_TIMEOUT`, `ICM_RETRIES`).
pub const TIMEOUT_US: u64 = 5_000_000;
pub const DRIVER_READY_TIMEOUT_US: u64 = 20_000_000;
pub const APPROVE_TIMEOUT_US: u64 = 10_000_000;
pub const RETRIES: u8 = 3;
/// How long the firmware may take to come up after BOOT's force power.
pub const FIRMWARE_WAIT_US: u64 = 3_000_000;
/// The root router's config space: tries every 50 ms, 50 times.
pub const ROOT_RETRY_US: u64 = 50_000;
pub const ROOT_TRIES: u8 = 50;
/// Devices tracked at once (a dock chain is at most 6 deep, 2 ports).
pub const MAX_DEVICES: usize = 16;

pub fn header(code: u8, flags: u8, packet: u8, total: u8) -> u32 {
    code as u32 | (flags as u32) << 8 | (packet as u32) << 16 | (total as u32) << 24
}
pub fn code(w: &Words) -> u8 { w.word(0) as u8 }
pub fn flags(w: &Words) -> u8 { (w.word(0) >> 8) as u8 }
pub fn packet(w: &Words) -> (u8, u8) { ((w.word(0) >> 16) as u8, (w.word(0) >> 24) as u8) }

fn uuid_words(uuid: &[u8; 16]) -> [u32; 4] {
    core::array::from_fn(|i| u32::from_le_bytes([uuid[i * 4], uuid[i * 4 + 1], uuid[i * 4 + 2], uuid[i * 4 + 3]]))
}
fn uuid_of(w: &Words, at: usize) -> [u8; 16] {
    let mut u = [0u8; 16];
    for i in 0..4 { u[i * 4..i * 4 + 4].copy_from_slice(&w.word(at + i).to_le_bytes()); }
    u
}

/// The security level the firmware enforces for a device (BIOS setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security { None, User, Secure, DpOnly, UsbOnly, NoPcie }
impl Security {
    pub fn of(level: u8) -> Self {
        match level { 0 => Self::None, 1 => Self::User, 2 => Self::Secure, 3 => Self::DpOnly, 4 => Self::UsbOnly, _ => Self::NoPcie }
    }
    /// Whether a device at this level waits for the OS to approve its PCIe tunnel.
    pub fn needs_approval(self) -> bool { matches!(self, Self::User | Self::Secure) }
}

/// A Thunderbolt device (router) the firmware reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    pub uuid: [u8; 16],
    pub route: u64,
    pub connection_id: u8,
    pub security: Security,
    /// The firmware already tunnels its PCIe (approved, boot ACL, or no security).
    pub authorized: bool,
    /// Approved by the BIOS's preboot list.
    pub boot: bool,
    pub link: u8,
    pub depth: u8,
    /// 20 Gb/s lanes (Gen 3) rather than 10.
    pub gen3: bool,
    pub dual_lane: bool,
}
impl Device {
    /// Linux `__icm_tr_device_connected`.
    pub fn parse(w: &Words) -> Option<Device> {
        if w.len < 8 { return None; }
        let f = flags(w);
        let link_info = (w.word(7) >> 16) as u16;
        Some(Device {
            uuid: uuid_of(w, 1),
            route: (w.word(5) as u64) << 32 | w.word(6) as u64,
            connection_id: w.word(7) as u8,
            security: Security::of((f >> 3) & 3),
            authorized: link_info & LINK_INFO_APPROVED != 0,
            boot: link_info & LINK_INFO_BOOT != 0,
            link: (link_info & 7) as u8,
            depth: ((link_info >> 4) & 0xf) as u8,
            gen3: f & FLAGS_SPEED_GEN3 != 0,
            dual_lane: f & FLAGS_DUAL_LANE != 0,
        })
    }
    /// The link speed in Gb/s per lane.
    pub fn gbps(&self) -> u8 { if self.gen3 { 20 } else { 10 } }
}

/// The routers below `parent` (each hop of a route is one byte, the port taken).
pub fn below(route: u64, parent: u64) -> bool {
    if route == parent { return false; }
    let depth = (0..8).take_while(|i| (parent >> (8 * i)) & 0xff != 0).count();
    if depth == 8 { return false; }
    route & ((1u64 << (8 * depth)) - 1) == parent
}

/// What the driver-ready answer says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ready { pub proto_version: u8, pub boot_acl: u8, pub nvm_version: u32, pub device_id: u16, pub security: u8 }
impl Ready {
    /// Linux `icm_tr_pkg_driver_ready_response`.
    pub fn parse(w: &Words) -> Ready {
        let info = (w.word(1) >> 16) as u16;
        Ready {
            security: (info & 7) as u8,
            proto_version: ((info >> 4) & 7) as u8,
            boot_acl: ((info >> 7) & 0x3f) as u8,
            nvm_version: w.word(2),
            device_id: w.word(3) as u16,
        }
    }
    /// NVM version as the vendor prints it (major.minor).
    pub fn nvm(&self) -> (u8, u8) { ((self.nvm_version >> 16) as u8, (self.nvm_version >> 8) as u8) }
}

/// APPROVE_DEVICE for a device the firmware reported.
pub fn approve(d: &Device) -> [u32; 8] {
    let u = uuid_words(&d.uuid);
    [header(APPROVE_DEVICE, 0, 0, 0), u[0], u[1], u[2], u[3], (d.route >> 32) as u32, d.route as u32, d.connection_id as u32]
}

/// Where the manager is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the firmware to finish its NVM check after force power.
    Firmware,
    /// DRIVER_READY sent.
    DriverReady,
    /// Reading the root router's config space until it answers.
    RootRouter,
    Ready,
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The controller reads all ones: off or gone.
    Gone,
    /// No connection-manager firmware running (an OS-managed USB4 host: stage T3).
    NoFirmware { status: u32 },
    /// The firmware never answered DRIVER_READY.
    NoAnswer,
    /// The firmware answered with its error flag.
    Refused,
    /// The root router's config space never answered.
    RootRouter,
}

/// Things the server acts on (log, ask the desktop, tell the hot-plug code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Ready(Ready),
    Connected(Device),
    /// The firmware refused a device (topology limit).
    Rejected { route: u64 },
    Disconnected(Device),
    /// APPROVE_DEVICE came back: the PCIe tunnel is up (ok) or failed.
    Approved { route: u64, ok: bool },
    /// The firmware asks the OS not to suspend the controller (non-zero) or allows it.
    Veto(u32),
    /// A host-to-host (XDomain) link came or went; not used yet.
    XDomain { connected: bool },
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request { code: u8, words: [u32; 8], len: usize, sent: u64, tries: u8, timeout: u64, route: u64 }

/// The connection manager's state, driven by [`Manager::poll`].
pub struct Manager {
    pub phase: Phase,
    pub layout: Layout,
    pub tx: Ring,
    pub rx: Ring,
    pub hops: u16,
    pub ready: Option<Ready>,
    pub devices: [Option<Device>; MAX_DEVICES],
    started: u64,
    request: Option<Request>,
    approvals: [Option<u64>; 4],
    root_tries: u8,
    root_sent: u64,
    seq: u8,
    pub frames_in: u64,
    pub frames_out: u64,
    pub bad_frames: u64,
}

impl Manager {
    pub fn new(layout: Layout, now: u64) -> Self {
        Manager { phase: Phase::Firmware, layout, tx: Ring::tx(&layout, 0), rx: Ring::rx(&layout, 0), hops: 0, ready: None,
            devices: [None; MAX_DEVICES], started: now, request: None, approvals: [None; 4], root_tries: 0, root_sent: 0, seq: 0,
            frames_in: 0, frames_out: 0, bad_frames: 0 }
    }

    pub fn device(&self, route: u64) -> Option<&Device> { self.devices.iter().flatten().find(|d| d.route == route) }
    pub fn count(&self) -> usize { self.devices.iter().flatten().count() }

    /// Asks the firmware to tunnel a device's PCIe (the user approved it, or it
    /// is remembered). Queued until the request in flight finishes. False if the
    /// device is unknown or the queue is full.
    pub fn approve(&mut self, route: u64) -> bool {
        if self.device(route).is_none() { return false; }
        if self.approvals.contains(&Some(route)) || self.request.is_some_and(|r| r.code == APPROVE_DEVICE && r.route == route) { return true; }
        match self.approvals.iter_mut().find(|a| a.is_none()) { Some(a) => { *a = Some(route); true } None => false }
    }

    fn fail(&mut self, f: Failure, events: &mut impl FnMut(Event)) {
        self.phase = Phase::Failed(f);
        events(Event::Failed(f));
    }

    fn send(&mut self, n: &mut impl Nhi, words: &[u32], pdf: u8) -> bool {
        let mut buf = [0u8; FRAME_SIZE];
        let Some(len) = frame::encode(words, &mut buf) else { return false };
        let ok = self.tx.send(n, &buf[..len], pdf).is_ok();
        if ok { self.frames_out += 1; }
        ok
    }

    fn command(&mut self, n: &mut impl Nhi, words: &[u32], timeout: u64, route: u64, now: u64) {
        let mut w = [0u32; 8];
        w[..words.len()].copy_from_slice(words);
        let r = Request { code: words[0] as u8, words: w, len: words.len(), sent: now, tries: 0, timeout, route };
        self.send(n, words, PDF_ICM_COMMAND);
        self.request = Some(r);
    }

    fn read_root(&mut self, n: &mut impl Nhi, now: u64) {
        self.seq = (self.seq + 1) & 3;
        let w = frame::config_read(0, frame::SPACE_SWITCH, 0, 0, 1, self.seq);
        self.send(n, &w, PDF_READ);
        self.root_sent = now;
        self.root_tries += 1;
    }

    /// Runs the manager: firmware wait, rings, driver ready, root router, then
    /// events and approvals. `now` in microseconds.
    pub fn poll(&mut self, n: &mut impl Nhi, now: u64, mut events: impl FnMut(Event)) {
        match self.phase {
            Phase::Failed(_) => return,
            Phase::Firmware => {
                let status = n.read(regs::FW_STS);
                if status == u32::MAX { return self.fail(Failure::Gone, &mut events); }
                let want = regs::FW_STS_ICM_EN | regs::FW_STS_NVM_AUTH_DONE;
                if status & want != want {
                    if now.saturating_sub(self.started) >= FIRMWARE_WAIT_US { self.fail(Failure::NoFirmware { status }, &mut events); }
                    return;
                }
                self.hops = (n.read(regs::CAPS) & regs::CAPS_HOPS_MASK) as u16;
                self.tx.start(n);
                self.rx.start(n);
                self.command(n, &[header(DRIVER_READY, 0, 0, 0)], DRIVER_READY_TIMEOUT_US, 0, now);
                self.phase = Phase::DriverReady;
            }
            _ => {}
        }
        self.tx.reap(n);
        let mut buf = [0u8; FRAME_SIZE];
        // RX flag 0x4 (buffer overrun) is the same bit as POSTED, which the
        // controller may leave set; like Linux, trust the size and CRC instead.
        while let Some((pdf, len, _flags)) = self.rx.receive(n, &mut buf) {
            match frame::decode(pdf, &buf[..len]) {
                Ok(w) => { self.frames_in += 1; self.frame(n, &w, now, &mut events); }
                Err(_) => self.bad_frames += 1,
            }
            if matches!(self.phase, Phase::Failed(_)) { return; }
        }
        // A request that went unanswered: send again, then give up.
        if let Some(mut r) = self.request {
            if now.saturating_sub(r.sent) >= r.timeout {
                if r.tries < RETRIES {
                    r.tries += 1;
                    r.sent = now;
                    let words = r.words;
                    self.send(n, &words[..r.len], PDF_ICM_COMMAND);
                    self.request = Some(r);
                } else {
                    self.request = None;
                    if r.code == DRIVER_READY { return self.fail(Failure::NoAnswer, &mut events); }
                    if r.code == APPROVE_DEVICE { events(Event::Approved { route: r.route, ok: false }); }
                }
            }
        }
        if self.phase == Phase::RootRouter && now.saturating_sub(self.root_sent) >= ROOT_RETRY_US {
            if self.root_tries >= ROOT_TRIES { return self.fail(Failure::RootRouter, &mut events); }
            self.read_root(n, now);
        }
        if self.phase == Phase::Ready && self.request.is_none() {
            if let Some(route) = self.approvals.iter_mut().find_map(|a| a.take()) {
                match self.device(route).copied() {
                    Some(d) => self.command(n, &approve(&d), APPROVE_TIMEOUT_US, route, now),
                    None => events(Event::Approved { route, ok: false }),
                }
            }
        }
    }

    fn frame(&mut self, n: &mut impl Nhi, w: &Words, now: u64, events: &mut impl FnMut(Event)) {
        match w.pdf {
            PDF_ICM_RESPONSE => {
                let Some(r) = self.request else { return };
                if code(w) != r.code { return; }
                let (id, total) = packet(w);
                if total != 0 && id + 1 < total { return; }
                self.request = None;
                let error = flags(w) & FLAGS_ERROR != 0;
                match r.code {
                    DRIVER_READY if error => self.fail(Failure::Refused, events),
                    DRIVER_READY => {
                        let ready = Ready::parse(w);
                        self.ready = Some(ready);
                        events(Event::Ready(ready));
                        self.phase = Phase::RootRouter;
                        self.root_tries = 0;
                        self.read_root(n, now);
                    }
                    APPROVE_DEVICE => {
                        if !error { if let Some(d) = self.devices.iter_mut().flatten().find(|d| d.route == r.route) { d.authorized = true; } }
                        events(Event::Approved { route: r.route, ok: !error });
                    }
                    _ => {}
                }
            }
            PDF_READ if self.phase == Phase::RootRouter && frame::route_of(w) == 0 => self.phase = Phase::Ready,
            PDF_ERROR => {}
            PDF_ICM_EVENT => self.event(w, events),
            _ => {}
        }
    }

    fn event(&mut self, w: &Words, events: &mut impl FnMut(Event)) {
        match code(w) {
            EVENT_DEVICE_CONNECTED => {
                // A second packet carries QoS data nobody uses.
                if packet(w).0 != 0 { return; }
                let Some(d) = Device::parse(w) else { return };
                if (w.word(7) >> 16) as u16 & LINK_INFO_REJECTED != 0 { return events(Event::Rejected { route: d.route }); }
                // The same device again (after DRIVER_READY, or re-sent): update it.
                if let Some(old) = self.devices.iter_mut().flatten().find(|o| o.uuid == d.uuid || o.route == d.route) {
                    *old = d;
                } else if let Some(slot) = self.devices.iter_mut().find(|s| s.is_none()) {
                    *slot = Some(d);
                } else { return; }
                events(Event::Connected(d));
            }
            EVENT_DEVICE_DISCONNECTED => {
                let route = (w.word(1) as u64) << 32 | w.word(2) as u64;
                // Deepest first, so the hot-plug code drops a dock's children before the dock.
                for depth in (0..=8).rev() {
                    for slot in self.devices.iter_mut() {
                        let Some(d) = *slot else { continue };
                        let level = (0..8).take_while(|i| (d.route >> (8 * i)) & 0xff != 0).count();
                        if level == depth && (d.route == route || below(d.route, route)) { *slot = None; events(Event::Disconnected(d)); }
                    }
                }
                self.approvals.iter_mut().for_each(|a| if a.is_some_and(|r| r == route || below(r, route)) { *a = None });
            }
            EVENT_RTD3_VETO => events(Event::Veto(w.word(1))),
            EVENT_XDOMAIN_CONNECTED => events(Event::XDomain { connected: true }),
            EVENT_XDOMAIN_DISCONNECTED => events(Event::XDomain { connected: false }),
            _ => {}
        }
    }
}
