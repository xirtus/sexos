// SPDX-License-Identifier: GPL-3.0-only
//! USB (servers/sexusb: xhci.rs, usb.rs, hid.rs) against an xHCI controller
//! model with hostile devices behind it. The controller model is honest enough to
//! start (capabilities, reset, rings, doorbells, ports, the event ring), and the
//! input decides everything a device or a broken controller controls: which ports
//! have devices and at what speed, command completion codes and slot ids, every
//! byte a device returns for a control or interrupt transfer (device, config,
//! interface, endpoint, HID and hub descriptors, HID reports, hub port status),
//! short packets, stalls, stray events, hot-unplug, and garbage register reads.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
#![allow(dead_code, static_mut_refs)]
#[path = "../../../servers/sexusb/src/xhci.rs"] mod xhci;
#[path = "../../../servers/sexusb/src/usb.rs"] mod usb;
#[path = "../../../servers/sexusb/src/hid.rs"] mod hid;
#[path = "../../../servers/sexusb/src/uac.rs"] mod uac;
use std::cell::RefCell;

const OP_USB_MOUSE_REPORT: u64 = 0x260;
const OP_USB_KEYBOARD_REPORT: u64 = 0x261;
fn send_report_to_sexinput(_op: u64, _a0: u64, _a1: u64, _a2: u64) -> Result<bool, u64> { Ok(true) }
fn record_device_wheel(_slot: u32, _bytes: u32, _absolute: bool, _wheel: i8, _packed: u64) {}

const CAP: u64 = 0x7000_0000;
const LIMIT: u64 = 0x10000;
const OP: u64 = CAP + 0x20;
const RT: u64 = CAP + 0x2000;
const DB: u64 = CAP + 0x3000;
const EXT: u64 = 0x100;

struct Port { connected: bool, enabled: bool, speed: u32, changes: u32, power: bool }

struct Model {
    data: Vec<u8>, at: usize,
    regs: std::collections::HashMap<u64, u32>,
    pool: (u64, u64),
    ports: Vec<Port>, slots: u32, hcc1: u32, hcs2: u32, ext: bool, major3_first: u32,
    usbcmd: u32,
    pending: Vec<(u64, u64, [u32; 4])>,
    ev: Vec<[u32; 4]>, ev_idx: usize, ev_cycle: u32,
    next_slot: u32,
}
impl Model {
    fn byte(&mut self) -> Option<u8> { let b = self.data.get(self.at).copied(); self.at += 1; b }
    fn u8(&mut self) -> u8 { self.byte().unwrap_or(0) }
    fn u32(&mut self) -> u32 { u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()]) }
    fn done(&self) -> bool { self.at >= self.data.len() }
    fn reg(&self, a: u64) -> u32 { self.regs.get(&a).copied().unwrap_or(0) }
    fn in_pool(&self, pa: u64, len: u64) -> bool { pa >= self.pool.0 && pa.checked_add(len).is_some_and(|e| e <= self.pool.0 + self.pool.1) }
    fn post(&mut self, d: [u32; 4]) {
        let i = self.ev_idx;
        self.ev[i] = [d[0], d[1], d[2], (d[3] & !1) | self.ev_cycle];
        self.ev_idx += 1;
        if self.ev_idx == self.ev.len() { self.ev_idx = 0; self.ev_cycle ^= 1; }
    }
    fn portsc(&self, p: usize) -> u32 {
        let Some(s) = self.ports.get(p) else { return 0 };
        s.connected as u32 | (s.enabled as u32) << 1 | (s.power as u32) << 9 | s.speed << 10 | s.changes
    }
    fn port_change(&mut self, p: usize) {
        self.post([((p as u32) + 1) << 24, 0, 1 << 24, 34 << 10]);
    }
    /// Commands: complete each one, mostly with success.
    fn commands(&mut self) {
        let crcr = (self.reg(OP + 0x18) as u64 | (self.reg(OP + 0x1c) as u64) << 32) & !0x3f;
        let (mine, rest): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|(base, _, _)| *base == crcr);
        self.pending = rest;
        for (base, idx, d) in mine {
            let kind = (d[3] >> 10) & 0x3f;
            if kind == 6 { continue; }
            let b = self.u8();
            if b == 0xff && !self.done() { continue; } // lost completion
            let code = if b < 0xe8 || self.done() { 1 } else { (b & 0x3f) as u32 };
            let slot = if kind == 9 { let s = if b & 1 == 1 && b >= 0xe8 { self.u8() as u32 } else { self.next_slot }; self.next_slot = self.next_slot % self.slots.max(1) + 1; s } else { d[3] >> 24 };
            let at = base + idx * 16;
            self.post([at as u32, (at >> 32) as u32, code << 24, (33 << 10) | slot << 24]);
        }
    }
    /// Transfers on a slot's endpoint `dci`: the device fills IN buffers.
    fn transfers(&mut self, slot: u32, dci: u32) {
        let crcr = (self.reg(OP + 0x18) as u64 | (self.reg(OP + 0x1c) as u64) << 32) & !0x3f;
        let (mine, rest): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|(base, _, _)| *base != crcr);
        self.pending = rest;
        for (base, idx, d) in mine {
            let kind = (d[3] >> 10) & 0x3f;
            if kind == 6 { continue; }
            let at = base + idx * 16;
            let len = (d[2] & 0x1ffff) as u64;
            let ptr = d[0] as u64 | (d[1] as u64) << 32;
            let input = (kind == 3 && d[3] & (1 << 16) != 0) || ((kind == 1 || kind == 5) && dci & 1 == 1);
            let mut residual = 0u32;
            if input && len > 0 && self.in_pool(ptr, len) && !self.done() {
                let want = self.u8() as u64;
                let n = if want >= 0xf0 { len } else { (want % (len + 1)).min(len) };
                for i in 0..n { let v = self.u8(); unsafe { *((ptr + i) as *mut u8) = v; } }
                residual = (len - n) as u32;
            }
            let b = self.u8();
            let short = residual > 0 && d[3] & (1 << 2) != 0;
            if d[3] & (1 << 5) == 0 && !short && b < 0xf0 { continue; }
            if b == 0xff && !self.done() { continue; } // the device never answers this one
            let code = if b >= 0xf0 && !self.done() { (self.u8() & 0x3f) as u32 } else if residual > 0 { 13 } else { 1 };
            self.post([at as u32, (at >> 32) as u32, code << 24 | residual, (32 << 10) | dci << 16 | slot << 24]);
        }
        // Now and then something unasked for: a stray event, a hot-plug, an unplug.
        if !self.done() {
            match self.u8() {
                0xf0..=0xf7 => { let d = [self.u32(), self.u32(), self.u32(), self.u32()]; self.post(d); }
                0xf8..=0xfb => { let p = self.u8() as usize % self.ports.len().max(1); if let Some(s) = self.ports.get_mut(p) { s.connected = !s.connected; s.enabled = false; s.changes |= 1 << 17; } self.port_change(p); }
                _ => {}
            }
        }
    }
}

thread_local! { static M: RefCell<Option<Model>> = const { RefCell::new(None) }; }
fn with<R>(f: impl FnOnce(&mut Model) -> R) -> R { M.with(|m| f(m.borrow_mut().as_mut().expect("model"))) }

fn mmio_read32(base: u64, offset: u64) -> u32 {
    sexos_driver_fuzz::tick();
    let a = base + offset;
    with(|m| {
        let honest = match a {
            a if a == CAP => 0x0110_0020,
            a if a == CAP + 4 => (m.ports.len() as u32) << 24 | m.slots,
            a if a == CAP + 8 => m.hcs2,
            a if a == CAP + 0x10 => m.hcc1 | if m.ext { ((EXT >> 2) as u32) << 16 } else { 0 },
            a if a == CAP + 0x14 => 0x3000,
            a if a == CAP + 0x18 => 0x2000,
            a if a == CAP + EXT => m.reg(a) & !(1 << 16) | 1 | 4 << 8,
            a if a == CAP + EXT + 0x10 => 2 | 3 << 24,
            a if a == CAP + EXT + 0x18 => m.major3_first | (m.ports.len() as u32) << 8,
            a if a == OP => m.usbcmd,
            a if a == OP + 4 => (m.usbcmd & 1 == 0) as u32,
            a if a >= OP + 0x400 && a < OP + 0x400 + 0x10 * 256 && (a - OP - 0x400) % 0x10 == 0 => m.portsc(((a - OP - 0x400) / 0x10) as usize),
            a => m.reg(a),
        };
        match m.byte() { Some(b) if b >= 0xfa => m.u32(), _ => honest }
    })
}

fn mmio_write32(base: u64, offset: u64, v: u32) {
    let a = base + offset;
    with(|m| {
        if a == OP { m.usbcmd = v & !2; }
        if a >= OP + 0x400 && a < OP + 0x400 + 0x10 * 256 && (a - OP - 0x400) % 0x10 == 0 {
            let p = ((a - OP - 0x400) / 0x10) as usize;
            if let Some(s) = m.ports.get_mut(p) {
                s.changes &= !(v & 0x00fe_0000);
                if v & (1 << 9) != 0 { s.power = true; }
                if v & (1 << 4) != 0 && s.connected { s.enabled = true; s.changes |= 1 << 21; }
            }
        }
        if a >= DB && a < DB + 256 * 4 {
            let target = ((a - DB) / 4) as u32;
            if target == 0 { m.commands() } else { m.transfers(target, v & 0x1f) }
        }
        m.regs.insert(a, v);
    })
}

fn trb_write_volatile(base_va: u64, index: u64, d0: u32, d1: u32, d2: u32, d3: u32) {
    let p = (base_va + index * 16) as *mut u32;
    unsafe { p.write(d0); p.add(1).write(d1); p.add(2).write(d2); p.add(3).write(d3); }
    with(|m| m.pending.push((base_va, index, [d0, d1, d2, d3])));
}

fn trb_read_dword(_base_va: u64, index: u64, word: usize) -> u32 {
    sexos_driver_fuzz::tick();
    with(|m| m.ev.get(index as usize).map_or(0, |e| e[word & 3]))
}

const POOL: usize = 2 << 20;

fn run(d: &[u8]) {
    let mut s = sexos_driver_fuzz::Src::new(d);
    let nports = 1 + s.below(6);
    let ports = (0..nports).map(|_| { let b = s.u8(); Port { connected: b & 1 != 0, enabled: false, speed: 1 + (b as u32 >> 1) % 4, changes: if b & 1 != 0 { 1 << 17 } else { 0 }, power: true } }).collect();
    let flags = s.u8();
    let slots = 1 + s.below(32) as u32;
    let hcs2 = if flags & 0x80 != 0 { s.u32() } else { 0 };
    let hcc1 = (flags & 0x0c) as u32;
    let major3_first = s.u8() as u32;
    let layout = std::alloc::Layout::from_size_align(POOL, 4096).unwrap();
    let mem = unsafe { std::alloc::alloc_zeroed(layout) };
    let base = mem as u64;
    M.with(|m| *m.borrow_mut() = Some(Model {
        data: s.rest().to_vec(), at: 0, regs: Default::default(), pool: (base, POOL as u64),
        ports, slots, hcc1, hcs2, ext: flags & 0x10 != 0, major3_first, usbcmd: 0, pending: Vec::new(),
        ev: vec![[0; 4]; 256], ev_idx: 0, ev_cycle: 1, next_slot: 1,
    }));
    let view = sex_pdx::memory_region::RegionView { region_id: 3, virtual_address: base, length: POOL as u64, dma_address: Some(base) };
    if let Ok(pool) = sex_pdx::dma_pool::DmaRegionPool::new(view) {
        if let Ok(x) = xhci::Xhci::start(CAP, LIMIT, pool) {
            let mut u = usb::Usb::new(x);
            // Bell's sound region, so sound cards and microphones are driven too; Bell
            // listens on some inputs, which starts a microphone.
            let mut region = vec![0u64; (sex_pdx::usb_audio::BYTES / 8) as usize];
            u.audio_region = region.as_mut_ptr() as u64;
            let shared = unsafe { sex_pdx::usb_audio::Shared::at(region.as_mut_ptr() as *mut u8) };
            shared.set_listen(flags & 0x20 != 0);
            let _ = u.scan();
            for _ in 0..48 {
                for _ in 0..64 { let Some(e) = u.x.event() else { break }; u.event(e); }
                u.service();
                let _ = u.audio_pump();
                if with(|m| m.done()) { break; }
            }
            let _ = u.summary();
            let _ = u.ether_active();
        }
    }
    M.with(|m| *m.borrow_mut() = None);
    unsafe { std::alloc::dealloc(mem, layout) };
}

/// A keyboard on port 1 (high speed): device descriptor, configuration with one
/// boot-keyboard interface and an interrupt IN endpoint, then key reports.
fn seeds() -> Vec<Vec<u8>> {
    let mut v = vec![0u8, 0, 0, 0, 7, 0x10, 8, 0, 0, 0, 0];
    let dev = [18u8, 1, 0, 2, 0, 0, 0, 64, 0x6d, 0x04, 0x1c, 0xc3, 0, 1, 1, 2, 0, 1];
    let cfg = [9u8, 2, 34, 0, 1, 1, 0, 0xa0, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 9, 0x21, 0x11, 1, 0, 1, 0x22, 63, 0, 7, 5, 0x81, 3, 8, 0, 10];
    for _ in 0..40 { v.push(0); }
    for blob in [&dev[..8], &dev[..], &cfg[..9], &cfg[..], &[0u8; 0][..]] {
        for _ in 0..3 { v.push(0); }
        v.push(blob.len() as u8);
        v.extend_from_slice(blob);
        v.extend_from_slice(&[0, 0, 0]);
    }
    for k in 0..16u8 { v.extend_from_slice(&[0, 8, 0, 0, 4 + k, 0, 0, 0, 0, 0, 0, 0]); }
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
