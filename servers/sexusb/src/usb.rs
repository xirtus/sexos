// SPDX-License-Identifier: GPL-3.0-only
//! USB devices on the xHCI controller: enumeration, hubs, hot-plug and HID.
//!
//! Any number of devices up to `MAX_DEVICES` work at once, on root ports or behind
//! hubs (USB 2 and USB 3, up to five deep). Keyboards use the boot protocol; mice and
//! tablets are read through their report descriptors (`hid.rs`). Every keyboard's keys
//! are merged into one snapshot so two keyboards don't fight; pointers send their own
//! reports. Unplugging releases whatever that device held.

use sex_pdx::serial_record as log;
use crate::xhci::{self, page, trb, Event, Failure, Ring, Xhci, CC_SHORT_PACKET, CC_SUCCESS};
use crate::hid::{PointerLayout, Pointer};
#[path = "ether.rs"]
mod ether;
#[path = "audio.rs"]
mod audio;

use crate::{send_report_to_sexinput, record_device_wheel, OP_USB_KEYBOARD_REPORT, OP_USB_MOUSE_REPORT};

pub const MAX_DEVICES: usize = 32;
const MAX_INTR: usize = 3;
const INTR_TRBS: u32 = 32; // 31 usable: one 64-byte report buffer per ring position
const IN_FLIGHT: u32 = 4;
const REPORT: u64 = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role { None, Keyboard, Pointer, Hub }

/// A USB mass storage device (bulk-only transport, SCSI commands): a stick, a card
/// reader, an external disk. Files sees it as disk `disk`.
#[derive(Clone, Copy)]
struct Storage {
    active: bool,
    iface: u8,
    in_ep: u8,
    out_ep: u8,
    in_dci: u32,
    out_dci: u32,
    in_ring: Ring,
    out_ring: Ring,
    io: (u64, u64),
    tag: u32,
    block: u32,
    blocks: u64,
    disk: u8,
    read_only: bool,
    name: [u8; 40],
    name_len: usize,
}
impl Storage {
    const fn empty() -> Self {
        Storage { active: false, iface: 0, in_ep: 0, out_ep: 0, in_dci: 0, out_dci: 0, in_ring: Ring::empty(), out_ring: Ring::empty(),
            io: (0, 0), tag: 1, block: 512, blocks: 0, disk: 0, read_only: false, name: [0; 40], name_len: 0 }
    }
}

#[derive(Clone, Copy)]
struct Interrupt {
    active: bool,
    dci: u32,
    ring: Ring,
    bufs: u64, // phys of the report page
    bufs_va: u64,
    len: u32,
    role: Role,
    layout: PointerLayout,
    errors: u32,
    keys: u64,
    modifiers: u8,
    buttons: u8,
}
impl Interrupt {
    const fn empty() -> Self {
        Interrupt { active: false, dci: 0, ring: Ring::empty(), bufs: 0, bufs_va: 0, len: 0, role: Role::None,
            layout: PointerLayout { report_id: 0, buttons: crate::hid::Field { bit: 0, size: 0, signed: false, min: 0, max: 0 },
                x: crate::hid::Field { bit: 0, size: 0, signed: false, min: 0, max: 0 },
                y: crate::hid::Field { bit: 0, size: 0, signed: false, min: 0, max: 0 },
                wheel: crate::hid::Field { bit: 0, size: 0, signed: false, min: 0, max: 0 }, absolute: false },
            errors: 0, keys: 0, modifiers: 0, buttons: 0 }
    }
}

/// DMA pages each device index owns for its whole life (reused after unplug).
#[derive(Clone, Copy)]
struct Memory { out_ctx: (u64, u64), in_ctx: (u64, u64), ep0: (u64, u64), buf: (u64, u64),
    rings: [(u64, u64); MAX_INTR], reports: [(u64, u64); MAX_INTR] }

#[derive(Clone, Copy)]
struct Device {
    used: bool,
    slot: u32,
    speed: u32,
    route: u32,
    depth: u8,
    root_port: u32,
    parent: u8, // index + 1, or 0 for a root port
    parent_port: u8,
    tt_slot: u32,
    tt_port: u32,
    mps0: u16,
    ep0: Ring,
    mem: Option<Memory>,
    intr: [Interrupt; MAX_INTR],
    hub_ports: u8,
    hub_ss: bool,
    hub_ttt: u8,
    hub_mtt: bool,
    hub_pending: u32,
    vendor: u16,
    product: u16,
    storage: Storage,
}
impl Device {
    const fn empty() -> Self {
        Device { used: false, slot: 0, speed: 0, route: 0, depth: 0, root_port: 0, parent: 0, parent_port: 0,
            tt_slot: 0, tt_port: 0, mps0: 8, ep0: Ring::empty(), mem: None, intr: [Interrupt::empty(); MAX_INTR],
            hub_ports: 0, hub_ss: false, hub_ttt: 0, hub_mtt: false, hub_pending: 0, vendor: 0, product: 0,
            storage: Storage::empty() }
    }
    fn is_hub(&self) -> bool { self.hub_ports > 0 }
}

pub struct Usb {
    pub x: Xhci,
    dev: [Device; MAX_DEVICES],
    slot_dev: [u8; 256],
    root_pending: u64,
    sent_keys: u64,
    sent_modifiers: u8,
    pub stats: Stats,
    /// The data buffer every SCSI command moves through (one command at a time).
    bounce: (u64, u64),
    /// Disk number -> device index + 1.
    disks: [u8; sex_pdx::usb_disk::MAX_DISKS as usize],
    pub generation: u64,
    pub changed: bool,
    /// Disk numbers freed since Files last asked for the disks (`OP_DISKS`).
    freed: u8,
    /// The USB network adapter in use, and the region its frames move through (0: none).
    ether: Option<ether::Ether>,
    pub net_region: u64,
    /// Receive and send buffers (2 KiB per ring position), made with the first adapter.
    ether_buffers: ((u64, u64), (u64, u64)),
    /// The USB sound card in use, and the region Bell's frames come through (0: none).
    audio: Option<audio::Audio>,
    pub audio_region: u64,
    audio_buffers: (u64, u64),
    audio_ring: (u64, u64),
}

const BOUNCE: u64 = sex_pdx::usb_disk::BUFFER_BYTES + 8192;

#[derive(Default, Clone, Copy)]
pub struct Stats { pub slots: u32, pub hid: u32, pub pointers: u32, pub keyboards: u32, pub hubs: u32, pub disks: u32, pub failed: u32 }

// Speeds as the slot context and PORTSC encode them.
const FULL: u32 = 1; const LOW: u32 = 2; const HIGH: u32 = 3; const SUPER: u32 = 4;

fn speed_name(s: u32) -> &'static str { match s { FULL => "full", LOW => "low", HIGH => "high", SUPER => "super", 5 => "super_plus", _ => "unknown" } }

impl Usb {
    pub fn new(x: Xhci) -> Usb {
        Usb { x, dev: [Device::empty(); MAX_DEVICES], slot_dev: [0; 256], root_pending: 0, sent_keys: 0,
            sent_modifiers: 0, stats: Stats::default(), bounce: (0, 0), disks: [0; sex_pdx::usb_disk::MAX_DISKS as usize],
            generation: 0, changed: false, freed: 0, ether: None, net_region: 0, ether_buffers: ((0, 0), (0, 0)),
            audio: None, audio_region: 0, audio_buffers: (0, 0), audio_ring: (0, 0) }
    }

    fn ctx(&self, base: u64, index: u64) -> *mut u32 { (base + index * self.x.csz) as *mut u32 }

    fn memory(&mut self, i: usize) -> Result<Memory, Failure> {
        if let Some(m) = self.dev[i].mem { return Ok(m); }
        let p = &mut self.x.pool;
        let m = Memory { out_ctx: page(p, 1)?, in_ctx: page(p, 1)?, ep0: page(p, 1)?, buf: page(p, 1)?,
            rings: [page(p, 1)?, page(p, 1)?, page(p, 1)?], reports: [page(p, 1)?, page(p, 1)?, page(p, 1)?] };
        self.dev[i].mem = Some(m);
        Ok(m)
    }

    /// Scan the root ports once at start, and enumerate whatever is plugged in.
    pub fn scan(&mut self) -> (u32, u32) {
        let mut connected = 0;
        let mut first = 0;
        for p in 1..=self.x.ports {
            let s = self.x.portsc(p);
            log!("[sexusb.xhci.port] port={} connected={} enabled={} speed={} usb={}", p, s & 1, (s >> 1) & 1,
                (s >> 10) & 0xf, self.x.port_major[p as usize]);
            if s & 1 != 0 { connected += 1; if first == 0 { first = p; } }
        }
        for p in 1..=self.x.ports {
            self.x.clear_port_changes(p);
            if self.x.portsc(p) & 1 != 0 { self.attach_root(p); }
        }
        (connected, first)
    }

    fn attach_root(&mut self, port: u32) {
        if !self.x.reset_port(port) {
            log!("[sexusb.port.reset.fail] port={} portsc={:#x}", port, self.x.portsc(port));
            self.stats.failed += 1;
            return;
        }
        let speed = (self.x.portsc(port) >> 10) & 0xf;
        let _ = self.attach(port, 0, 0, speed);
    }

    fn free_index(&self) -> Option<usize> { self.dev.iter().position(|d| !d.used) }

    /// Address and configure one new device and start its drivers.
    fn attach(&mut self, root_port: u32, parent: u8, parent_port: u8, speed: u32) -> Result<usize, Failure> {
        let Some(i) = self.free_index() else { log!("[sexusb.device.refused] reason=table_full max={}", MAX_DEVICES); return Err(Failure::Dma); };
        let r = self.attach_at(i, root_port, parent, parent_port, speed);
        if let Err(ref e) = r {
            self.stats.failed += 1;
            log!("[sexusb.device.fail] port={} parent={} hub_port={} speed={} reason={:?}", root_port, parent, parent_port, speed_name(speed), e);
            if self.dev[i].used { self.detach(i); }
        }
        r
    }

    fn attach_at(&mut self, i: usize, root_port: u32, parent: u8, parent_port: u8, speed: u32) -> Result<usize, Failure> {
        let mem = self.memory(i)?;
        let (route, depth, tt_slot, tt_port) = if parent == 0 { (0, 0, 0, 0) } else {
            let p = self.dev[parent as usize - 1];
            let route = p.route | ((parent_port.min(15) as u32) << (4 * p.depth as u32));
            // Low and full speed devices behind a high speed hub go through its translator.
            let (tts, ttp) = if p.speed == HIGH && (speed == LOW || speed == FULL) { (p.slot, parent_port as u32) }
                else { (p.tt_slot, p.tt_port) };
            (route, p.depth + 1, tts, ttp)
        };
        let slot = self.x.command_ok(0, 0, 0, trb::ENABLE_SLOT << 10)?;
        let mps0 = match speed { LOW | FULL => 8, HIGH => 64, _ => 512 };
        self.dev[i] = Device { used: true, slot, speed, route, depth, root_port, parent, parent_port, tt_slot, tt_port,
            mps0, mem: Some(mem), ..Device::empty() };
        self.slot_dev[slot as usize] = i as u8 + 1;
        self.stats.slots += 1;
        self.dev[i].ep0.init(mem.ep0.0, mem.ep0.1, 256);
        unsafe {
            core::ptr::write_bytes(mem.out_ctx.1 as *mut u8, 0, xhci::PAGE as usize);
            core::ptr::write_volatile((self.x.dcbaa + slot as u64 * 8) as *mut u64, mem.out_ctx.0);
        }
        // Address Device with slot and EP0 contexts.
        self.input_context(i, 0b11, 1);
        self.endpoint_context(i, 1, 4, mps0, 0, self.dev[i].ep0.dequeue_pointer(), 8, 0);
        self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::ADDRESS_DEVICE << 10) | (slot << 24))?;

        // Device descriptor: 8 bytes first for a full speed device's EP0 packet size.
        let buf = mem.buf;
        let got = self.get(i, 0x80, 6, 0x0100, 0, buf.0, 8)?;
        if got < 8 { return Err(Failure::Code(0)); }
        let b7 = unsafe { core::ptr::read_volatile((buf.1 + 7) as *const u8) };
        let real = if speed >= SUPER { 1u16 << b7.min(9) } else { b7 as u16 };
        if real != mps0 && real >= 8 {
            self.dev[i].mps0 = real;
            self.input_context(i, 0b10, 1);
            self.endpoint_context(i, 1, 4, real, 0, self.dev[i].ep0.dequeue_pointer(), 8, 0);
            self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::EVALUATE_CONTEXT << 10) | (slot << 24))?;
        }
        self.get(i, 0x80, 6, 0x0100, 0, buf.0, 18)?;
        let d = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, 18) };
        let (class, protocol, configs) = (d[4], d[6], d[17]);
        self.dev[i].vendor = u16::from_le_bytes([d[8], d[9]]);
        self.dev[i].product = u16::from_le_bytes([d[10], d[11]]);
        // Configuration descriptor, whole.
        self.get(i, 0x80, 6, 0x0200, 0, buf.0, 9)?;
        let total = unsafe { core::ptr::read_unaligned((buf.1 + 2) as *const u16) }.min(2048);
        let got = self.get(i, 0x80, 6, 0x0200, 0, buf.0, total)? as usize;
        let mut cfg = [0u8; 2048];
        unsafe { core::ptr::copy_nonoverlapping(buf.1 as *const u8, cfg.as_mut_ptr(), got.min(2048)); }
        let cfg = &cfg[..got.min(2048)];
        log!("[sexusb.device] slot={} port={} route={:#x} depth={} speed={} vendor={:#06x} product={:#06x} class={}",
            slot, root_port, route, depth, speed_name(speed), self.dev[i].vendor, self.dev[i].product, class);

        // Choose interfaces we drive: HID with an interrupt IN endpoint, or a hub.
        struct Pick { iface: u8, sub: u8, proto: u8, ep: u8, mps: u16, interval: u8, report_len: u16, hub: bool }
        let mut picks: [Option<Pick>; MAX_INTR] = [None, None, None];
        let mut n = 0usize;
        let (mut iface, mut iclass, mut isub, mut iproto, mut alt, mut report_len) = (0u8, 0u8, 0u8, 0u8, 0u8, 0u16);
        let mut at = 0usize;
        let config_value = if cfg.len() > 5 { cfg[5] } else { 1 };
        // Mass storage: (interface, (IN address, mps, burst), (OUT address, mps, burst)).
        let mut msc: Option<(u8, (u8, u16, u8), (u8, u16, u8))> = None;
        let mut last_bulk: Option<bool> = None;
        while at + 2 <= cfg.len() {
            let len = cfg[at] as usize;
            if len < 2 || at + len > cfg.len() { break; }
            match cfg[at + 1] {
                4 if len >= 9 => { iface = cfg[at + 2]; alt = cfg[at + 3]; iclass = cfg[at + 5]; isub = cfg[at + 6]; iproto = cfg[at + 7]; report_len = 0; }
                0x21 if len >= 9 => { report_len = u16::from_le_bytes([cfg[at + 7], cfg[at + 8]]); }
                // SuperSpeed companion: the burst of the endpoint just before it.
                0x30 if len >= 3 => {
                    if let (Some(input), Some(m)) = (last_bulk, msc.as_mut()) {
                        if input { m.1 .2 = cfg[at + 2]; } else { m.2 .2 = cfg[at + 2]; }
                    }
                }
                5 if len >= 7 && alt == 0 && iclass == 8 && isub == 6 && iproto == 0x50 && cfg[at + 3] & 3 == 2 => {
                    let (addr, mps) = (cfg[at + 2], u16::from_le_bytes([cfg[at + 4], cfg[at + 5]]));
                    let m = msc.get_or_insert((iface, (0, 0, 0), (0, 0, 0)));
                    if m.0 == iface {
                        if addr & 0x80 != 0 { m.1 = (addr, mps, 0); last_bulk = Some(true); } else { m.2 = (addr, mps, 0); last_bulk = Some(false); }
                    }
                }
                5 if len >= 7 && alt == 0 => {
                    last_bulk = None;
                    let (addr, attr) = (cfg[at + 2], cfg[at + 3]);
                    let mps = u16::from_le_bytes([cfg[at + 4], cfg[at + 5]]);
                    let interval = cfg[at + 6];
                    let hub = iclass == 9 || class == 9;
                    if addr & 0x80 != 0 && attr & 3 == 3 && (iclass == 3 || hub) && n < MAX_INTR {
                        picks[n] = Some(Pick { iface, sub: isub, proto: iproto, ep: addr & 0xf, mps, interval, report_len, hub });
                        n += 1;
                    }
                }
                _ => {}
            }
            at += len;
        }
        let _ = protocol;
        if picks.iter().flatten().any(|p| p.hub) { picks.iter_mut().for_each(|p| if p.as_ref().is_some_and(|p| !p.hub) { *p = None; }); }
        let msc = msc.filter(|m| m.1 .0 != 0 && m.2 .0 != 0);
        if msc.is_some() { picks = [None, None, None]; }
        // Nothing else to drive: perhaps a network adapter (its ECM function may be in
        // another configuration, as on Realtek RTL8152/8153).
        if msc.is_none() && picks.iter().all(|p| p.is_none()) && self.try_ether(i, configs)? { return Ok(i); }
        // A sound card (its buttons, if it has a HID interface, are not driven yet).
        if msc.is_none() && !picks.iter().flatten().any(|p| p.hub) && self.try_audio(i, configs)? { return Ok(i); }

        // Configure Endpoint for each picked interrupt endpoint.
        let mut add = 1u32;
        let mut last_dci = 1u32;
        self.input_context(i, 0, 0);
        for (k, p) in picks.iter().enumerate() {
            let Some(p) = p else { continue };
            let dci = p.ep as u32 * 2 + 1;
            let ring_mem = mem.rings[k];
            let mut ring = Ring::empty();
            ring.init(ring_mem.0, ring_mem.1, INTR_TRBS);
            let len = (p.mps & 0x7ff).min(REPORT as u16) as u32;
            self.dev[i].intr[k] = Interrupt { active: true, dci, ring, bufs: mem.reports[k].0, bufs_va: mem.reports[k].1,
                len, role: if p.hub { Role::Hub } else { Role::None }, ..Interrupt::empty() };
            let interval = interval_exponent(speed, p.interval);
            self.endpoint_context(i, dci, 7, p.mps & 0x7ff, interval, ring.dequeue_pointer(), len, len);
            add |= 1 << dci;
            last_dci = last_dci.max(dci);
        }
        if let Some((iface, inp, out)) = msc {
            let (in_dci, out_dci) = ((inp.0 & 0xf) as u32 * 2 + 1, (out.0 & 0xf) as u32 * 2);
            let mut in_ring = Ring::empty();
            in_ring.init(mem.rings[0].0, mem.rings[0].1, 256);
            let mut out_ring = Ring::empty();
            out_ring.init(mem.rings[1].0, mem.rings[1].1, 256);
            self.endpoint_context_burst(i, in_dci, 6, inp.1 & 0x7ff, inp.2, in_ring.dequeue_pointer());
            self.endpoint_context_burst(i, out_dci, 2, out.1 & 0x7ff, out.2, out_ring.dequeue_pointer());
            self.dev[i].storage = Storage { active: true, iface, in_ep: inp.0, out_ep: out.0, in_dci, out_dci, in_ring, out_ring,
                io: mem.reports[0], ..Storage::empty() };
            add |= (1 << in_dci) | (1 << out_dci);
            last_dci = last_dci.max(in_dci).max(out_dci);
        }
        if add != 1 {
            self.input_flags(i, add);
            self.slot_context(i, last_dci);
            self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (slot << 24))?;
        }
        self.set(i, 0x00, 9, config_value as u16, 0)?;
        if self.dev[i].storage.active {
            if let Err(e) = self.start_storage(i) {
                log!("[sexusb.storage.fail] slot={} reason={:?}", slot, e);
                self.dev[i].storage.active = false;
            }
        }

        for k in 0..MAX_INTR {
            let Some(p) = &picks[k] else { continue };
            if p.hub {
                self.start_hub(i, k)?;
            } else {
                self.start_hid(i, k, p.iface, p.sub, p.proto, p.report_len);
            }
        }
        Ok(i)
    }

    fn get(&mut self, i: usize, rt: u8, req: u8, value: u16, index: u16, phys: u64, len: u16) -> Result<u32, Failure> {
        let slot = self.dev[i].slot;
        let mut ring = self.dev[i].ep0;
        let r = self.x.control(slot, &mut ring, rt, req, value, index, phys, len);
        self.dev[i].ep0 = ring;
        r
    }
    fn set(&mut self, i: usize, rt: u8, req: u8, value: u16, index: u16) -> Result<u32, Failure> {
        self.get(i, rt, req, value, index, 0, 0)
    }

    /// Clear the input context and set its add flags; fill the slot context.
    fn input_context(&self, i: usize, add: u32, entries: u32) {
        let m = self.dev[i].mem.unwrap();
        unsafe { core::ptr::write_bytes(m.in_ctx.1 as *mut u8, 0, xhci::PAGE as usize); }
        self.input_flags(i, add);
        self.slot_context(i, entries.max(1));
    }
    fn input_flags(&self, i: usize, add: u32) {
        let m = self.dev[i].mem.unwrap();
        unsafe { core::ptr::write_volatile((m.in_ctx.1 + 4) as *mut u32, add); }
    }
    fn slot_context(&self, i: usize, entries: u32) {
        let d = &self.dev[i];
        let m = d.mem.unwrap();
        let s = self.ctx(m.in_ctx.1, 1);
        let hub = d.is_hub() as u32;
        unsafe {
            core::ptr::write_volatile(s, d.route | (d.speed << 20) | ((d.hub_mtt as u32) << 25) | (hub << 26) | (entries << 27));
            core::ptr::write_volatile(s.add(1), (d.root_port << 16) | ((d.hub_ports as u32) << 24));
            core::ptr::write_volatile(s.add(2), d.tt_slot | (d.tt_port << 8) | ((d.hub_ttt as u32) << 16));
        }
    }
    /// Endpoint context `dci` in the input context.
    fn endpoint_context(&self, i: usize, dci: u32, kind: u32, mps: u16, interval: u32, deq: u64, avg: u32, esit: u32) {
        let m = self.dev[i].mem.unwrap();
        let e = self.ctx(m.in_ctx.1, 1 + dci as u64);
        unsafe {
            core::ptr::write_volatile(e, interval << 16);
            core::ptr::write_volatile(e.add(1), (3 << 1) | (kind << 3) | ((mps as u32) << 16));
            core::ptr::write_volatile(e.add(2), deq as u32);
            core::ptr::write_volatile(e.add(3), (deq >> 32) as u32);
            core::ptr::write_volatile(e.add(4), avg | (esit << 16));
        }
    }

    fn endpoint_context_burst(&self, i: usize, dci: u32, kind: u32, mps: u16, burst: u8, deq: u64) {
        let m = self.dev[i].mem.unwrap();
        let e = self.ctx(m.in_ctx.1, 1 + dci as u64);
        unsafe {
            core::ptr::write_volatile(e, 0);
            core::ptr::write_volatile(e.add(1), (3 << 1) | (kind << 3) | ((burst as u32) << 8) | ((mps as u32) << 16));
            core::ptr::write_volatile(e.add(2), deq as u32);
            core::ptr::write_volatile(e.add(3), (deq >> 32) as u32);
            core::ptr::write_volatile(e.add(4), 3072);
        }
    }

    fn start_hid(&mut self, i: usize, k: usize, iface: u8, sub: u8, proto: u8, report_len: u16) {
        let _ = self.set(i, 0x21, 0x0A, 0, iface as u16); // SET_IDLE 0: report on change only
        let buf = self.dev[i].mem.unwrap().buf;
        let mut role = Role::None;
        if sub == 1 && proto == 1 {
            let _ = self.set(i, 0x21, 0x0B, 0, iface as u16); // boot protocol
            role = Role::Keyboard;
            self.stats.keyboards += 1;
        } else {
            let len = report_len.clamp(1, 4096);
            if let Ok(got) = self.get(i, 0x81, 6, 0x2200, iface as u16, buf.0, len) {
                let desc = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, got as usize) };
                if let Some(layout) = crate::hid::parse_pointer(desc) {
                    self.dev[i].intr[k].layout = layout;
                    role = Role::Pointer;
                }
            }
            if role == Role::None && sub == 1 && proto == 2 {
                // A boot mouse whose descriptor we couldn't read: the boot protocol has a known layout.
                let _ = self.set(i, 0x21, 0x0B, 0, iface as u16);
                self.dev[i].intr[k].layout = boot_mouse_layout();
                role = Role::Pointer;
            }
            if role == Role::Pointer { self.stats.pointers += 1; }
        }
        self.dev[i].intr[k].role = role;
        if role == Role::None { self.dev[i].intr[k].active = false; return; }
        self.stats.hid += 1;
        let l = self.dev[i].intr[k].layout;
        log!("[sexusb.hid.bind.summary] keyboard_ep={} pointer_role={} slot={} iface={} report_id={} wheel={} ep_dci={}",
            if role == Role::Keyboard { "set" } else { "none" },
            if role == Role::Keyboard { "none" } else if l.absolute { "tablet" } else { "mouse" },
            self.dev[i].slot, iface, l.report_id, l.wheel.present() as u8, self.dev[i].intr[k].dci);
        self.arm(i, k, IN_FLIGHT);
    }

    /// Queue `n` interrupt-IN TRBs, each with the report buffer of its ring position.
    fn arm(&mut self, i: usize, k: usize, n: u32) {
        let slot = self.dev[i].slot;
        let e = &mut self.dev[i].intr[k];
        for _ in 0..n {
            let at = e.bufs + e.ring.idx as u64 * REPORT;
            e.ring.push(at as u32, (at >> 32) as u32, e.len, (trb::NORMAL << 10) | trb::IOC | trb::ISP);
        }
        let dci = e.dci;
        self.x.doorbell(slot, dci);
    }

    fn start_hub(&mut self, i: usize, k: usize) -> Result<(), Failure> {
        let buf = self.dev[i].mem.unwrap().buf;
        let ss = self.dev[i].speed >= SUPER;
        if ss {
            // SET_HUB_DEPTH before anything else on a USB 3 hub.
            self.set(i, 0x20, 12, self.dev[i].depth as u16, 0)?;
        }
        let got = self.get(i, 0xA0, 6, if ss { 0x2A00 } else { 0x2900 }, 0, buf.0, 12)?;
        if got < 7 { return Err(Failure::Code(0)); }
        let d = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, 12) };
        let ports = d[2].min(15);
        let chars = u16::from_le_bytes([d[3], d[4]]);
        let power_ms = d[5] as u32 * 2;
        let dev = &mut self.dev[i];
        dev.hub_ports = ports;
        dev.hub_ss = ss;
        dev.hub_ttt = if dev.speed == HIGH { ((chars >> 5) & 3) as u8 } else { 0 };
        dev.hub_mtt = false;
        // Tell the controller this slot is a hub (Configure Endpoint, slot context only).
        let entries = dev.intr.iter().filter(|e| e.active).map(|e| e.dci).max().unwrap_or(1);
        self.input_context(i, 1, entries);
        let m = self.dev[i].mem.unwrap();
        self.x.command_ok(m.in_ctx.0 as u32, (m.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (self.dev[i].slot << 24))?;
        self.stats.hubs += 1;
        log!("[sexusb.hub] slot={} ports={} usb={} depth={} power_ms={}", self.dev[i].slot, ports, if ss { 3 } else { 2 },
            self.dev[i].depth, power_ms);
        for p in 1..=ports { let _ = self.set(i, 0x23, 3, 8, p as u16); } // PORT_POWER
        let ticks = (power_ms as u64 / 10 + 2).max(10);
        crate::xhci::sleep_ticks(ticks);
        self.dev[i].hub_pending = ((1u32 << (ports + 1)) - 1) & !1;
        self.dev[i].intr[k].role = Role::Hub;
        self.arm(i, k, 1);
        Ok(())
    }

    /// Check the ports a hub said changed (or all, after it starts).
    fn service_hub(&mut self, i: usize) {
        let pending = core::mem::take(&mut self.dev[i].hub_pending);
        let buf = self.dev[i].mem.unwrap().buf;
        for p in 1..=self.dev[i].hub_ports as u32 {
            if pending & (1 << p) == 0 || !self.dev[i].used { continue; }
            let Ok(4) = self.get(i, 0xA3, 0, 0, p as u16, buf.0, 4) else { continue };
            let (status, change) = unsafe { (core::ptr::read_volatile(buf.1 as *const u16), core::ptr::read_volatile((buf.1 + 2) as *const u16)) };
            // Acknowledge every change bit we saw.
            for (bit, feature) in [(0, 16u16), (1, 17), (2, 18), (3, 19), (4, 20), (5, 29), (6, 25), (7, 26)] {
                if change & (1 << bit) != 0 { let _ = self.set(i, 0x23, 1, feature, p as u16); }
            }
            let child = self.child(i, p as u8);
            let connected = status & 1 != 0;
            if !connected || change & 1 != 0 { if let Some(c) = child { self.detach(c); } }
            if connected && (child.is_none() || change & 1 != 0) {
                if let Some(speed) = self.reset_hub_port(i, p as u16) {
                    let root = self.dev[i].root_port;
                    let _ = self.attach(root, i as u8 + 1, p as u8, speed);
                }
            }
        }
    }

    fn reset_hub_port(&mut self, i: usize, p: u16) -> Option<u32> {
        let buf = self.dev[i].mem.unwrap().buf;
        crate::xhci::sleep_ticks(10); // debounce
        self.set(i, 0x23, 3, 4, p).ok()?; // PORT_RESET
        for _ in 0..100 {
            crate::xhci::sleep_ticks(2);
            if self.get(i, 0xA3, 0, 0, p, buf.0, 4).ok()? != 4 { continue; }
            let (status, change) = unsafe { (core::ptr::read_volatile(buf.1 as *const u16), core::ptr::read_volatile((buf.1 + 2) as *const u16)) };
            if change & (1 << 4) != 0 || (status & 2 != 0 && status & (1 << 4) == 0) {
                let _ = self.set(i, 0x23, 1, 20, p); // C_PORT_RESET
                if self.dev[i].hub_ss { let _ = self.set(i, 0x23, 1, 29, p); }
                if status & 2 == 0 { return None; }
                crate::xhci::sleep_ticks(2);
                return Some(if self.dev[i].hub_ss { SUPER } else if status & (1 << 9) != 0 { LOW }
                    else if status & (1 << 10) != 0 { HIGH } else { FULL });
            }
        }
        None
    }

    fn child(&self, hub: usize, port: u8) -> Option<usize> {
        self.dev.iter().position(|d| d.used && d.parent == hub as u8 + 1 && d.parent_port == port)
    }

    /// Forget a device and everything behind it; release its keys and buttons.
    fn detach(&mut self, i: usize) {
        for c in 0..MAX_DEVICES {
            if self.dev[c].used && self.dev[c].parent == i as u8 + 1 { self.detach(c); }
        }
        let slot = self.dev[i].slot;
        let mut keys_changed = false;
        for k in 0..MAX_INTR {
            let e = &mut self.dev[i].intr[k];
            if !e.active { continue; }
            if e.role == Role::Keyboard && (e.keys != 0 || e.modifiers != 0) { e.keys = 0; e.modifiers = 0; keys_changed = true; }
            if e.role == Role::Pointer && e.buttons != 0 {
                e.buttons = 0;
                let packed = if e.layout.absolute { 1u64 << 32 } else { 0 };
                if !e.layout.absolute { let _ = send_report_to_sexinput(OP_USB_MOUSE_REPORT, 0, 0, packed); }
            }
            e.active = false;
        }
        if keys_changed { self.send_keys(); }
        self.ether_gone(i);
        self.audio_gone(i);
        if self.dev[i].storage.active {
            let disk = self.dev[i].storage.disk as usize;
            if self.disks[disk] == i as u8 + 1 { self.disks[disk] = 0; self.freed |= 1 << disk; }
            self.dev[i].storage.active = false;
            self.generation += 1;
            self.changed = true;
            log!("[sexusb.storage.gone] disk={} generation={}", disk, self.generation);
        }
        if slot != 0 {
            let _ = self.x.command(0, 0, 0, (trb::DISABLE_SLOT << 10) | (slot << 24));
            unsafe { core::ptr::write_volatile((self.x.dcbaa + slot as u64 * 8) as *mut u64, 0); }
            self.slot_dev[slot as usize] = 0;
        }
        log!("[sexusb.device.gone] slot={} port={} route={:#x}", slot, self.dev[i].root_port, self.dev[i].route);
        let mem = self.dev[i].mem;
        self.dev[i] = Device { mem, ..Device::empty() };
    }

    /// Handle one event from the controller.
    pub fn event(&mut self, e: Event) {
        match e.kind() {
            trb::PORT_STATUS_CHANGE => {
                let port = e.d0 >> 24;
                if port >= 1 && port <= self.x.ports.min(63) { self.root_pending |= 1 << port; }
            }
            trb::TRANSFER_EVENT => self.transfer(e),
            _ => {}
        }
    }

    /// Work that may need commands or control transfers: root port and hub changes.
    pub fn service(&mut self) {
        while self.root_pending != 0 {
            let port = self.root_pending.trailing_zeros();
            self.root_pending &= !(1 << port);
            let s = self.x.portsc(port);
            self.x.clear_port_changes(port);
            let root_dev = self.dev.iter().position(|d| d.used && d.parent == 0 && d.root_port == port);
            log!("[sexusb.port.change] port={} connected={} enabled={} portsc={:#x}", port, s & 1, (s >> 1) & 1, s);
            if s & 1 == 0 || s & (1 << 17) != 0 {
                if let Some(d) = root_dev { self.detach(d); }
            }
            if s & 1 != 0 && self.dev.iter().all(|d| !(d.used && d.parent == 0 && d.root_port == port)) {
                self.attach_root(port);
            }
        }
        for i in 0..MAX_DEVICES {
            if self.dev[i].used && self.dev[i].hub_pending != 0 { self.service_hub(i); }
        }
    }

    fn transfer(&mut self, e: Event) {
        let Some(i) = (self.slot_dev[e.slot() as usize] as usize).checked_sub(1) else { return };
        let dci = e.endpoint();
        if self.ether_event(i, e) { return; }
        if self.audio_event(i, e) { return; }
        let Some(k) = self.dev[i].intr.iter().position(|x| x.active && x.dci == dci) else { return };
        let Some(pos) = self.dev[i].intr[k].ring.index_of(e.pointer()) else { return };
        let code = e.code();
        if code != CC_SUCCESS && code != CC_SHORT_PACKET {
            let slot = self.dev[i].slot;
            let x = &mut self.dev[i].intr[k];
            x.errors += 1;
            log!("[sexusb.intr.error] slot={} dci={} code={} errors={}", slot, dci, code, x.errors);
            if x.errors > 8 { x.active = false; return; }
            let mut ring = x.ring;
            self.x.recover(slot, dci, &mut ring);
            self.dev[i].intr[k].ring = ring;
            let n = if self.dev[i].intr[k].role == Role::Hub { 1 } else { IN_FLIGHT };
            self.arm(i, k, n);
            return;
        }
        let x = self.dev[i].intr[k];
        let len = x.len.saturating_sub(e.residual()) as usize;
        let mut report = [0u8; REPORT as usize];
        unsafe { core::ptr::copy_nonoverlapping((x.bufs_va + pos as u64 * REPORT) as *const u8, report.as_mut_ptr(), len.min(REPORT as usize)); }
        let report = &report[..len.min(REPORT as usize)];
        match x.role {
            Role::Keyboard => self.keyboard(i, k, report),
            Role::Pointer => self.pointer(i, k, report),
            Role::Hub => {
                let mut bits = 0u32;
                for (b, byte) in report.iter().enumerate().take(4) { bits |= (*byte as u32) << (8 * b); }
                self.dev[i].hub_pending |= bits & !1;
            }
            Role::None => {}
        }
        self.arm(i, k, 1);
    }

    fn keyboard(&mut self, i: usize, k: usize, r: &[u8]) {
        if r.len() < 3 { return; }
        // Phantom state (rollover overflow): every key reads 0x01. Keep the last state.
        if r[2..].iter().take(6).any(|b| *b == 1) { return; }
        let mut keys = 0u64;
        let mut n = 0;
        for b in r[2..].iter().take(6) {
            if *b > 3 && n < 6 { keys |= (*b as u64) << (8 * n); n += 1; }
        }
        let e = &mut self.dev[i].intr[k];
        if e.keys == keys && e.modifiers == r[0] { return; }
        e.keys = keys;
        e.modifiers = r[0];
        self.send_keys();
    }

    /// Every keyboard's keys and modifiers, merged into one snapshot for Input.
    fn send_keys(&mut self) {
        let mut keys = 0u64;
        let mut n = 0;
        let mut modifiers = 0u8;
        for d in self.dev.iter().filter(|d| d.used) {
            for e in d.intr.iter().filter(|e| e.active && e.role == Role::Keyboard) {
                modifiers |= e.modifiers;
                for b in 0..6 {
                    let key = (e.keys >> (8 * b)) as u8;
                    if key == 0 || n == 6 || (0..n).any(|j| (keys >> (8 * j)) as u8 == key) { continue; }
                    keys |= (key as u64) << (8 * n);
                    n += 1;
                }
            }
        }
        if keys == self.sent_keys && modifiers == self.sent_modifiers { return; }
        self.sent_keys = keys;
        self.sent_modifiers = modifiers;
        unsafe {
            static mut FORWARD_BUDGET: u32 = 128;
            if FORWARD_BUDGET > 0 && keys != 0 {
                FORWARD_BUDGET -= 1;
                log!("[sexusb.kbd.forward] key=0x{:x} keys={:#x} mod=0x{:x}", keys as u8, keys, modifiers);
            }
        }
        let _ = send_report_to_sexinput(OP_USB_KEYBOARD_REPORT, 0, modifiers as u64, keys);
    }

    fn pointer(&mut self, i: usize, k: usize, r: &[u8]) {
        let slot = self.dev[i].slot;
        let e = &mut self.dev[i].intr[k];
        let Some(Pointer { buttons, x, y, wheel, absolute }) = e.layout.decode(r) else { return };
        let buttons = buttons & 7;
        e.buttons = buttons;
        let wheel = wheel.clamp(-127, 127) as i8;
        if absolute {
            let ax = PointerLayout::scale(&e.layout.x, x) as u64;
            let ay = PointerLayout::scale(&e.layout.y, y) as u64;
            let packed = ax | (ay << 16) | (1 << 32) | ((wheel as u8 as u64) << 40);
            record_device_wheel(slot, r.len() as u32, true, wheel, packed);
            let _ = send_report_to_sexinput(OP_USB_MOUSE_REPORT, 0, buttons as u64, packed);
            return;
        }
        // Relative motion wider than 8 bits is split into several reports.
        let (mut dx, mut dy) = (x, y);
        let mut first = true;
        loop {
            let sx = dx.clamp(-127, 127);
            let sy = dy.clamp(-127, 127);
            dx -= sx; dy -= sy;
            let w = if first { wheel } else { 0 };
            let packed = (sx as i8 as u8 as u64) | ((sy as i8 as u8 as u64) << 8) | ((w as u8 as u64) << 16);
            if first { record_device_wheel(slot, r.len() as u32, false, w, packed); }
            let _ = send_report_to_sexinput(OP_USB_MOUSE_REPORT, 0, buttons as u64, packed);
            first = false;
            if dx == 0 && dy == 0 { break; }
        }
    }

    pub fn summary(&self) -> Stats { self.stats }
}

/// Endpoint context interval: 2^n x 125 us frames.
fn interval_exponent(speed: u32, b_interval: u8) -> u32 {
    match speed {
        HIGH | SUPER | 5 => (b_interval.clamp(1, 16) - 1) as u32,
        _ => {
            // bInterval in ms (1..255): largest 2^n x 125 us not above it.
            let frames = (b_interval.max(1) as u32) * 8;
            (31 - frames.leading_zeros()).clamp(3, 10)
        }
    }
}

fn boot_mouse_layout() -> PointerLayout {
    use crate::hid::Field;
    PointerLayout { report_id: 0, buttons: Field { bit: 0, size: 3, signed: false, min: 0, max: 1 },
        x: Field { bit: 8, size: 8, signed: true, min: -127, max: 127 },
        y: Field { bit: 16, size: 8, signed: true, min: -127, max: 127 },
        wheel: Field { bit: 0, size: 0, signed: true, min: 0, max: 0 }, absolute: false }
}

// ── Mass storage: bulk-only transport and the SCSI commands a disk needs ──────────
use sex_pdx::usb_disk as ud;

#[derive(Debug)]
enum Scsi { Usb(Failure), Check, Phase }
impl From<Failure> for Scsi { fn from(f: Failure) -> Self { Scsi::Usb(f) } }

impl Usb {
    fn start_storage(&mut self, i: usize) -> Result<(), Scsi> {
        if self.bounce.0 == 0 { self.bounce = page(&mut self.x.pool, BOUNCE / xhci::PAGE)?; }
        let st = self.dev[i].storage;
        let slot = self.dev[i].slot;
        // INQUIRY: a direct-access device, and its names.
        let got = self.scsi(i, &[0x12, 0, 0, 0, 36, 0], true, 36)?;
        let b = self.bounce.1;
        let inq = unsafe { core::slice::from_raw_parts(b as *const u8, 36) };
        if got < 5 || inq[0] & 0x1f != 0 { log!("[sexusb.storage.skip] slot={} type={:#x}", slot, inq[0] & 0x1f); return Ok(()); }
        let mut name = [0u8; 40];
        let mut n = 0;
        for &c in inq[8..16].iter().chain(b" ".iter()).chain(inq[16..32].iter()) {
            if n < name.len() && (0x20..0x7f).contains(&c) && !(c == b' ' && n > 0 && name[n - 1] == b' ') { name[n] = c; n += 1; }
        }
        while n > 0 && name[n - 1] == b' ' { n -= 1; }
        // TEST UNIT READY until the medium is ready (sticks report a reset first).
        let mut ready = false;
        for _ in 0..30 {
            match self.scsi(i, &[0, 0, 0, 0, 0, 0], false, 0) {
                Ok(_) => { ready = true; break; }
                Err(Scsi::Check) => { let _ = self.scsi(i, &[0x03, 0, 0, 0, 18, 0], true, 18); crate::xhci::sleep_ticks(5); }
                Err(e) => return Err(e),
            }
        }
        if !ready { log!("[sexusb.storage.no_medium] slot={}", slot); return Ok(()); }
        self.scsi(i, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], true, 8)?;
        let cap = unsafe { core::slice::from_raw_parts(b as *const u8, 32) };
        let mut last = u32::from_be_bytes([cap[0], cap[1], cap[2], cap[3]]) as u64;
        let mut block = u32::from_be_bytes([cap[4], cap[5], cap[6], cap[7]]);
        if last == 0xffff_ffff {
            self.scsi(i, &[0x9E, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 32, 0, 0], true, 32)?;
            let cap = unsafe { core::slice::from_raw_parts(b as *const u8, 32) };
            last = u64::from_be_bytes(cap[0..8].try_into().unwrap());
            block = u32::from_be_bytes([cap[8], cap[9], cap[10], cap[11]]);
        }
        if !(512..=4096).contains(&block) || !block.is_power_of_two() {
            log!("[sexusb.storage.skip] slot={} block={} reason=block_size", slot, block);
            return Ok(());
        }
        // MODE SENSE(6): the write-protect switch.
        let read_only = match self.scsi(i, &[0x1A, 0, 0x3F, 0, 4, 0], true, 4) {
            Ok(n) if n >= 3 => (unsafe { core::ptr::read_volatile((b + 2) as *const u8) } & 0x80) != 0,
            _ => false,
        };
        // A number freed since Files last asked stays unused, so Files always sees a stick
        // leave before another can take its number.
        let freed = self.freed;
        let Some(disk) = self.disks.iter().enumerate().position(|(k, d)| *d == 0 && freed & (1 << k) == 0) else { log!("[sexusb.storage.skip] slot={} reason=disks_full", slot); return Ok(()); };
        self.disks[disk] = i as u8 + 1;
        let s = &mut self.dev[i].storage;
        s.block = block; s.blocks = last + 1; s.disk = disk as u8; s.read_only = read_only; s.name = name; s.name_len = n;
        self.generation += 1;
        self.changed = true;
        self.stats.disks += 1;
        log!("[sexusb.storage] slot={} disk={} name=\"{}\" block={} sectors={} mib={} read_only={} iface={}", slot, disk,
            core::str::from_utf8(&name[..n]).unwrap_or("?"), block, (last + 1) * block as u64 / 512,
            (last + 1) * block as u64 / (1024 * 1024), read_only as u8, st.iface);
        Ok(())
    }

    /// One SCSI command: CBW out, data through the bounce buffer, CSW in.
    fn scsi(&mut self, i: usize, cdb: &[u8], input: bool, len: u32) -> Result<u32, Scsi> {
        let slot = self.dev[i].slot;
        let st = &mut self.dev[i].storage;
        st.tag = st.tag.wrapping_add(1);
        let tag = st.tag;
        let (io_phys, io) = st.io;
        unsafe {
            core::ptr::write_bytes(io as *mut u8, 0, 64);
            core::ptr::write_unaligned(io as *mut u32, 0x4342_5355);
            core::ptr::write_unaligned((io + 4) as *mut u32, tag);
            core::ptr::write_unaligned((io + 8) as *mut u32, len);
            core::ptr::write_volatile((io + 12) as *mut u8, if input { 0x80 } else { 0 });
            core::ptr::write_volatile((io + 14) as *mut u8, cdb.len() as u8);
            core::ptr::copy_nonoverlapping(cdb.as_ptr(), (io + 15) as *mut u8, cdb.len().min(16));
        }
        let (in_dci, out_dci, in_ep, out_ep) = (st.in_dci, st.out_dci, st.in_ep, st.out_ep);
        let mut out = st.out_ring;
        let r = self.x.bulk(slot, out_dci, &mut out, io_phys, 31, 600);
        self.dev[i].storage.out_ring = out;
        if r.is_err() { self.reset_recovery(i); return Err(Scsi::Phase); }
        let mut moved = 0;
        if len > 0 {
            let (dci, ep) = if input { (in_dci, in_ep) } else { (out_dci, out_ep) };
            let mut ring = if input { self.dev[i].storage.in_ring } else { self.dev[i].storage.out_ring };
            let r = self.x.bulk(slot, dci, &mut ring, self.bounce.0, len, 3000);
            if input { self.dev[i].storage.in_ring = ring; } else { self.dev[i].storage.out_ring = ring; }
            match r {
                Ok(n) => moved = n,
                Err(Failure::Code(6)) => { let _ = self.set(i, 0x02, 1, 0, ep as u16); }
                Err(e) => { self.reset_recovery(i); return Err(Scsi::Usb(e)); }
            }
        }
        // CSW, once more after clearing a stall.
        let mut csw_ok = false;
        for _ in 0..2 {
            let mut ring = self.dev[i].storage.in_ring;
            let r = self.x.bulk(slot, in_dci, &mut ring, io_phys + 64, 13, 3000);
            self.dev[i].storage.in_ring = ring;
            match r {
                Ok(13) => { csw_ok = true; break; }
                Err(Failure::Code(6)) => { let _ = self.set(i, 0x02, 1, 0, in_ep as u16); }
                _ => break,
            }
        }
        let (sig, ctag, status) = unsafe {
            (core::ptr::read_unaligned((io + 64) as *const u32), core::ptr::read_unaligned((io + 68) as *const u32),
             core::ptr::read_volatile((io + 76) as *const u8))
        };
        if !csw_ok || sig != 0x5342_5355 || ctag != tag || status == 2 { self.reset_recovery(i); return Err(Scsi::Phase); }
        if status == 1 { return Err(Scsi::Check); }
        Ok(moved)
    }

    fn reset_recovery(&mut self, i: usize) {
        let st = self.dev[i].storage;
        let _ = self.set(i, 0x21, 0xFF, 0, st.iface as u16);
        let _ = self.set(i, 0x02, 1, 0, st.in_ep as u16);
        let _ = self.set(i, 0x02, 1, 0, st.out_ep as u16);
    }

    fn rw10(&mut self, i: usize, write: bool, lba: u64, count: u32) -> Result<u32, Scsi> {
        let bytes = count * self.dev[i].storage.block;
        let op = if write { 0x2A } else { 0x28 };
        if lba + count as u64 > 0xffff_ffff {
            let mut c = [0u8; 16];
            c[0] = if write { 0x8A } else { 0x88 };
            c[2..10].copy_from_slice(&lba.to_be_bytes());
            c[10..14].copy_from_slice(&count.to_be_bytes());
            return self.scsi(i, &c, !write, bytes);
        }
        let l = (lba as u32).to_be_bytes();
        let n = (count as u16).to_be_bytes();
        self.scsi(i, &[op, 0, l[0], l[1], l[2], l[3], 0, n[0], n[1], 0], !write, bytes)
    }

    /// Move `sectors` 512-byte sectors at `lba` between the disk and `region`.
    fn disk_io(&mut self, disk: u64, write: bool, lba: u64, sectors: u64, region: u64) -> u64 {
        let Some(i) = self.disks.get(disk as usize).and_then(|d| (*d as usize).checked_sub(1)) else { return ud::GONE };
        if sectors == 0 || sectors > ud::MAX_SECTORS { return ud::INVALID; }
        let st = self.dev[i].storage;
        if write && st.read_only { return ud::READ_ONLY; }
        let bs = st.block as u64;
        let (start, end) = (lba * 512, (lba + sectors) * 512);
        if end > st.blocks * bs { return ud::INVALID; }
        let first = start / bs;
        let count = (end + bs - 1) / bs - first;
        let skip = (start - first * bs) as usize;
        let len = (sectors * 512) as usize;
        let b = self.bounce.1;
        let aligned = skip == 0 && len as u64 == count * bs;
        let result = if !write {
            self.rw10(i, false, first, count as u32).map(|_| unsafe {
                core::ptr::copy_nonoverlapping((b as *const u8).add(skip), region as *mut u8, len);
            })
        } else {
            // Larger blocks than a sector: read the blocks around the sectors first.
            let pre = if aligned { Ok(0) } else { self.rw10(i, false, first, count as u32) };
            pre.and_then(|_| {
                unsafe { core::ptr::copy_nonoverlapping(region as *const u8, (b as *mut u8).add(skip), len); }
                self.rw10(i, true, first, count as u32).map(|_| ())
            })
        };
        match result {
            Ok(()) => ud::OK,
            Err(e) => { log!("[sexusb.storage.io.fail] disk={} write={} lba={} sectors={} reason={:?}", disk, write as u8, lba, sectors, e); ud::IO }
        }
    }

    /// One request from Files.
    pub fn disk_request(&mut self, op: u64, a0: u64, a1: u64, a2: u64, region: u64) -> u64 {
        match op {
            ud::OP_DISKS => {
                let mask = self.disks.iter().enumerate().fold(0u64, |m, (k, d)| if *d != 0 { m | 1 << k } else { m });
                // Files now knows which numbers are free; they may be given out again.
                self.freed = 0;
                (self.generation << 8) | mask
            }
            ud::OP_INFO => {
                let Some(i) = self.disks.get(a0 as usize).and_then(|d| (*d as usize).checked_sub(1)) else { return 0 };
                let st = self.dev[i].storage;
                unsafe {
                    core::ptr::copy_nonoverlapping(st.name.as_ptr(), region as *mut u8, st.name_len);
                    core::ptr::write_volatile((region + st.name_len as u64) as *mut u8, 0);
                }
                (st.blocks * st.block as u64 / 512) | if st.read_only { ud::READ_ONLY_FLAG } else { 0 }
            }
            ud::OP_READ => self.disk_io(a0, false, a1, a2, region),
            ud::OP_WRITE => self.disk_io(a0, true, a1, a2, region),
            ud::OP_SYNC => {
                let Some(i) = self.disks.get(a0 as usize).and_then(|d| (*d as usize).checked_sub(1)) else { return ud::GONE };
                // SYNCHRONIZE CACHE(10); devices without a cache refuse it, which is fine.
                match self.scsi(i, &[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], false, 0) {
                    Ok(_) => ud::OK,
                    // Only "no such command" is fine; any other refusal is a flush that failed.
                    Err(Scsi::Check) => match self.scsi(i, &[0x03, 0, 0, 0, 18, 0], true, 18) {
                        Ok(n) if n >= 13 => {
                            let sense = unsafe { core::slice::from_raw_parts(self.bounce.1 as *const u8, 13) };
                            let (key, asc) = (sense[2] & 0xf, sense[12]);
                            if key == 5 && (asc == 0x20 || asc == 0x24) { ud::OK } else {
                                log!("[sexusb.storage.sync.fail] disk={} key={:#x} asc={:#x}", a0, key, asc);
                                ud::IO
                            }
                        }
                        _ => ud::IO,
                    },
                    Err(_) => ud::IO,
                }
            }
            _ => ud::INVALID,
        }
    }
}
