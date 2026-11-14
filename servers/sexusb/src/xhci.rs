// SPDX-License-Identifier: GPL-3.0-only
//! The xHCI controller: taking it from the firmware, rings, commands, events and
//! control transfers. Device logic (enumeration, hubs, HID) lives in `usb.rs`.
//!
//! Everything here is polled from one task. A command or control transfer waits for
//! its own completion; events for other endpoints that arrive meanwhile are kept in
//! a small queue and handed to the main loop afterwards, so no report is lost.

use sex_pdx::{dma_pool::DmaRegionPool, serial_record as log, sys_yield};
use crate::{mmio_read32, mmio_write32, trb_read_dword, trb_write_volatile};

/// 64-bit registers written low half first, as Linux does: controllers (QEMU's among
/// them) act on CRCR and ERSTBA when the high half lands.
fn write64(base: u64, off: u64, v: u64) {
    mmio_write32(base, off, v as u32);
    mmio_write32(base, off + 4, (v >> 32) as u32);
}

pub const PAGE: u64 = 4096;

pub mod trb {
    pub const NORMAL: u32 = 1;
    pub const SETUP: u32 = 2;
    pub const DATA: u32 = 3;
    pub const STATUS: u32 = 4;
    pub const LINK: u32 = 6;
    pub const ENABLE_SLOT: u32 = 9;
    pub const DISABLE_SLOT: u32 = 10;
    pub const ADDRESS_DEVICE: u32 = 11;
    pub const CONFIGURE_ENDPOINT: u32 = 12;
    pub const EVALUATE_CONTEXT: u32 = 13;
    pub const RESET_ENDPOINT: u32 = 14;
    pub const STOP_ENDPOINT: u32 = 15;
    pub const SET_TR_DEQUEUE: u32 = 16;
    pub const TRANSFER_EVENT: u32 = 32;
    pub const COMMAND_COMPLETION: u32 = 33;
    pub const PORT_STATUS_CHANGE: u32 = 34;
    pub const IOC: u32 = 1 << 5;
    pub const ISP: u32 = 1 << 2;
    pub const IDT: u32 = 1 << 6;
}
pub const CC_SUCCESS: u32 = 1;
pub const CC_SHORT_PACKET: u32 = 13;

/// A transfer or command ring of one page, closed by a link TRB that toggles the cycle.
#[derive(Clone, Copy)]
pub struct Ring { pub phys: u64, pub va: u64, pub idx: u32, pub cycle: u32, pub trbs: u32 }

impl Ring {
    pub const fn empty() -> Self { Ring { phys: 0, va: 0, idx: 0, cycle: 1, trbs: 0 } }
    /// `trbs` includes the link TRB; at most 256 in one page.
    pub fn init(&mut self, phys: u64, va: u64, trbs: u32) {
        unsafe { core::ptr::write_bytes(va as *mut u8, 0, (trbs as usize) * 16); }
        *self = Ring { phys, va, idx: 0, cycle: 1, trbs };
    }
    /// Queue one TRB (its cycle bit is added here) and return its address.
    pub fn push(&mut self, d0: u32, d1: u32, d2: u32, d3: u32) -> u64 {
        let at = self.phys + self.idx as u64 * 16;
        trb_write_volatile(self.va, self.idx as u64, d0, d1, d2, (d3 & !1) | self.cycle);
        self.idx += 1;
        if self.idx == self.trbs - 1 {
            trb_write_volatile(self.va, self.idx as u64, self.phys as u32, (self.phys >> 32) as u32, 0,
                (trb::LINK << 10) | (1 << 1) | self.cycle);
            self.cycle ^= 1;
            self.idx = 0;
        }
        at
    }
    /// Index of the TRB at `phys` in this ring, if it is one of ours.
    pub fn index_of(&self, phys: u64) -> Option<u32> {
        if phys < self.phys || phys >= self.phys + self.trbs as u64 * 16 { return None; }
        Some(((phys - self.phys) / 16) as u32)
    }
    pub fn dequeue_pointer(&self) -> u64 { (self.phys + self.idx as u64 * 16) | self.cycle as u64 }
}

#[derive(Clone, Copy, Default)]
pub struct Event { pub d0: u32, pub d1: u32, pub d2: u32, pub d3: u32 }
impl Event {
    pub fn kind(&self) -> u32 { (self.d3 >> 10) & 0x3f }
    pub fn code(&self) -> u32 { self.d2 >> 24 }
    pub fn slot(&self) -> u32 { self.d3 >> 24 }
    pub fn endpoint(&self) -> u32 { (self.d3 >> 16) & 0x1f }
    pub fn pointer(&self) -> u64 { self.d0 as u64 | ((self.d1 as u64) << 32) }
    pub fn residual(&self) -> u32 { self.d2 & 0x00ff_ffff }
}

const EVENTS: u32 = 256;
const KEPT: usize = 64;

#[allow(dead_code)]
pub struct Xhci {
    pub cap: u64,
    pub op: u64,
    pub rt: u64,
    pub db: u64,
    pub limit: u64,
    pub ports: u32,
    pub slots: u32,
    /// 32 or 64 bytes per context entry (HCCPARAMS1.CSZ).
    pub csz: u64,
    pub pool: DmaRegionPool,
    cmd: Ring,
    ev_phys: u64,
    ev_va: u64,
    ev_idx: u32,
    ev_cycle: u32,
    pub dcbaa: u64,
    kept: [Event; KEPT],
    kept_head: usize,
    kept_len: usize,
    /// USB major revision of each root port (2 or 3), from Supported Protocol capabilities.
    pub port_major: [u8; 256],
    pub dropped: u32,
}

#[derive(Debug)]
#[allow(dead_code)] // the code is printed with {:?}
pub enum Failure { Map, Dma, Reset, Timeout, Code(u32) }

pub fn page(pool: &mut DmaRegionPool, pages: u64) -> Result<(u64, u64), Failure> {
    let b = pool.reserve(pages * PAGE).map_err(|_| Failure::Dma)?;
    unsafe { core::ptr::write_bytes(b.virtual_address as *mut u8, 0, (pages * PAGE) as usize); }
    Ok((b.physical_address, b.virtual_address))
}

/// Sleeps `ticks` whole ticks. Once the controller's interrupt is unmasked, a pending
/// interrupt bit makes every self-wait return at once until it is taken, so take it
/// and keep waiting until the time is really up: polling loops that count ticks would
/// otherwise spin through their whole budget in microseconds and time out.
/// Set by `sleep_ticks`: an interrupt it took may belong to an event the main loop has
/// not drained yet, so the loop must look again before its long idle wait.
pub static SLEPT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
pub fn sleep_ticks(ticks: u64) {
    SLEPT.store(true, core::sync::atomic::Ordering::Relaxed);
    let end = sex_pdx::get_ticks().saturating_add(ticks);
    loop {
        let _ = sex_pdx::device::irq_take();
        let now = sex_pdx::get_ticks();
        if now >= end { return; }
        let _ = sex_pdx::sys_wait_self_ticks(end - now);
    }
}

fn wait(base: u64, off: u64, mask: u32, set: bool, polls: u32) -> bool {
    for i in 0..polls {
        if (mmio_read32(base, off) & mask != 0) == set { return true; }
        if i < 64 { sys_yield(); } else { sleep_ticks(1); }
    }
    false
}

impl Xhci {
    /// Take the controller from the firmware, reset it and start it with empty rings.
    pub fn start(cap: u64, limit: u64, mut pool: DmaRegionPool) -> Result<Xhci, Failure> {
        let caplen = mmio_read32(cap, 0) & 0xff;
        let hcs1 = mmio_read32(cap, 0x04);
        let hcs2 = mmio_read32(cap, 0x08);
        let hcc1 = mmio_read32(cap, 0x10);
        let op = cap + caplen as u64;
        let db = cap + (mmio_read32(cap, 0x14) & !3) as u64;
        let rt = cap + (mmio_read32(cap, 0x18) & !0x1f) as u64;
        if db >= cap + limit || rt + 0x40 > cap + limit { return Err(Failure::Map); }
        let ports = (hcs1 >> 24) & 0xff;
        let slots = (hcs1 & 0xff).min(255);
        let csz = if hcc1 & (1 << 2) != 0 { 64 } else { 32 };
        log!("[sexusb.xhci.caps] version={:#x} ports={} slots={} context_bytes={} scratchpads={} ext={:#x}",
            mmio_read32(cap, 0) >> 16, ports, slots, csz, scratchpads(hcs2), (hcc1 >> 16) << 2);

        let mut port_major = [0u8; 256];
        // Extended capabilities: firmware handoff and which ports speak USB 2 or 3.
        let mut ext = ((hcc1 >> 16) as u64) << 2;
        let mut guard = 0;
        while ext != 0 && ext + 8 <= limit && guard < 64 {
            guard += 1;
            let head = mmio_read32(cap, ext);
            match head & 0xff {
                1 => handoff(cap, ext)?,
                2 => {
                    let major = (head >> 24) as u8;
                    let span = mmio_read32(cap, ext + 8);
                    let (first, count) = (span & 0xff, (span >> 8) & 0xff);
                    for p in first..first.saturating_add(count) {
                        if (p as usize) < port_major.len() { port_major[p as usize] = major; }
                    }
                }
                _ => {}
            }
            let next = ((head >> 8) & 0xff) as u64;
            if next == 0 { break; }
            ext += next << 2;
        }

        // Stop, then reset.
        mmio_write32(op, 0x00, mmio_read32(op, 0x00) & !1);
        if !wait(op, 0x04, 1, true, 2000) { return Err(Failure::Reset); }
        mmio_write32(op, 0x00, 1 << 1);
        if !wait(op, 0x00, 1 << 1, false, 4000) || !wait(op, 0x04, 1 << 11, false, 4000) {
            return Err(Failure::Reset);
        }

        let (dcbaa_phys, dcbaa) = page(&mut pool, 1)?;
        // Scratchpad: pages the controller keeps for itself. QEMU asks for none; real
        // Intel and AMD controllers ask for a few to a few dozen and fail without them.
        let pads = scratchpads(hcs2);
        if pads > 0 {
            let (array_phys, array) = page(&mut pool, 1)?;
            for n in 0..pads.min(512) as u64 {
                let (p, _) = page(&mut pool, 1)?;
                unsafe { core::ptr::write_volatile((array + n * 8) as *mut u64, p); }
            }
            unsafe { core::ptr::write_volatile(dcbaa as *mut u64, array_phys); }
        }
        let (cmd_phys, cmd_va) = page(&mut pool, 1)?;
        let mut cmd = Ring::empty();
        cmd.init(cmd_phys, cmd_va, 256);
        let (ev_phys, ev_va) = page(&mut pool, 1)?;
        let (erst_phys, erst) = page(&mut pool, 1)?;
        unsafe {
            core::ptr::write_volatile(erst as *mut u64, ev_phys);
            core::ptr::write_volatile((erst + 8) as *mut u32, EVENTS);
        }

        mmio_write32(op, 0x38, slots);
        write64(op, 0x30, dcbaa_phys);
        write64(op, 0x18, cmd_phys | 1);
        let ir = rt + 0x20;
        mmio_write32(ir, 0x08, 1);
        write64(ir, 0x18, ev_phys);
        write64(ir, 0x10, erst_phys);
        // Polled: interrupter enabled so the event ring advances, no MSI wired yet.
        mmio_write32(ir, 0x00, mmio_read32(ir, 0x00) | 2);
        mmio_write32(op, 0x00, 1 | (1 << 2));
        if !wait(op, 0x04, 1, false, 2000) { return Err(Failure::Reset); }

        let x = Xhci { cap, op, rt, db, limit, ports, slots, csz, pool, cmd, ev_phys, ev_va, ev_idx: 0,
            ev_cycle: 1, dcbaa, kept: [Event::default(); KEPT], kept_head: 0, kept_len: 0, port_major, dropped: 0 };
        // Power every port the controller lets us switch (HCCPARAMS1.PPC).
        if hcc1 & (1 << 3) != 0 {
            for p in 1..=x.ports { let s = x.portsc(p); if s & (1 << 9) == 0 { x.set_portsc(p, (s & PORT_KEEP) | (1 << 9)); } }
        }
        log!("[sexusb.xhci.running] ports={} slots={} scratchpads={} ok=1", ports, slots, pads);
        Ok(x)
    }

    pub fn portsc(&self, port: u32) -> u32 { mmio_read32(self.op, 0x400 + (port as u64 - 1) * 0x10) }
    pub fn set_portsc(&self, port: u32, v: u32) { mmio_write32(self.op, 0x400 + (port as u64 - 1) * 0x10, v) }
    /// Acknowledge a port's change bits.
    pub fn clear_port_changes(&self, port: u32) {
        let s = self.portsc(port);
        self.set_portsc(port, (s & PORT_KEEP) | (s & PORT_CHANGES));
    }

    /// Reset a USB 2 root port so its device becomes enabled. USB 3 ports enable
    /// themselves when the link trains.
    pub fn reset_port(&self, port: u32) -> bool {
        let s = self.portsc(port);
        if s & 1 == 0 { return false; }
        if self.port_major[port as usize] == 3 || (self.port_major[port as usize] == 0 && s & 2 != 0) {
            for _ in 0..200 {
                if self.portsc(port) & 2 != 0 { return true; }
                sleep_ticks(1);
            }
            return self.portsc(port) & 2 != 0;
        }
        self.set_portsc(port, (s & PORT_KEEP) | (1 << 4));
        for _ in 0..400 {
            let s = self.portsc(port);
            if s & (1 << 21) != 0 && s & (1 << 4) == 0 {
                self.set_portsc(port, (s & PORT_KEEP) | (1 << 21));
                return s & 2 != 0;
            }
            sleep_ticks(1);
        }
        false
    }

    pub fn doorbell(&self, target: u32, value: u32) {
        unsafe { core::arch::asm!("sfence"); }
        mmio_write32(self.db, target as u64 * 4, value);
    }

    fn next_hw_event(&mut self) -> Option<Event> {
        let d3 = trb_read_dword(self.ev_va, self.ev_idx as u64, 3);
        if d3 & 1 != self.ev_cycle { return None; }
        unsafe { core::arch::asm!("lfence"); }
        let e = Event { d0: trb_read_dword(self.ev_va, self.ev_idx as u64, 0),
            d1: trb_read_dword(self.ev_va, self.ev_idx as u64, 1),
            d2: trb_read_dword(self.ev_va, self.ev_idx as u64, 2), d3 };
        self.ev_idx += 1;
        if self.ev_idx == EVENTS { self.ev_idx = 0; self.ev_cycle ^= 1; }
        write64(self.rt + 0x20, 0x18, (self.ev_phys + self.ev_idx as u64 * 16) | (1 << 3));
        Some(e)
    }

    fn keep(&mut self, e: Event) {
        if self.kept_len == KEPT { self.dropped += 1; return; }
        self.kept[(self.kept_head + self.kept_len) % KEPT] = e;
        self.kept_len += 1;
    }

    /// The next event for the main loop: ones kept while waiting come first.
    pub fn event(&mut self) -> Option<Event> {
        if self.kept_len > 0 {
            let e = self.kept[self.kept_head];
            self.kept_head = (self.kept_head + 1) % KEPT;
            self.kept_len -= 1;
            return Some(e);
        }
        self.next_hw_event()
    }

    /// Wait for an event `want` accepts; keep the others for later.
    fn wait_event(&mut self, polls: u32, mut want: impl FnMut(&Event) -> bool) -> Option<Event> {
        for i in 0..polls {
            while let Some(e) = self.next_hw_event() {
                if want(&e) { return Some(e); }
                self.keep(e);
            }
            if i < 256 { sys_yield(); } else { sleep_ticks(1); }
        }
        None
    }

    /// Run one command and return (completion code, slot id).
    pub fn command(&mut self, d0: u32, d1: u32, d2: u32, d3: u32) -> Result<(u32, u32), Failure> {
        let at = self.cmd.push(d0, d1, d2, d3);
        self.doorbell(0, 0);
        let Some(e) = self.wait_event(1500, |e| e.kind() == trb::COMMAND_COMPLETION && e.pointer() == at) else {
            log!("[sexusb.xhci.command.timeout] type={} at={:#x} usbsts={:#x} crcr={:#x}", (d3 >> 10) & 0x3f, at,
                mmio_read32(self.op, 0x04), mmio_read32(self.op, 0x18));
            return Err(Failure::Timeout);
        };
        Ok((e.code(), e.slot()))
    }

    pub fn command_ok(&mut self, d0: u32, d1: u32, d2: u32, d3: u32) -> Result<u32, Failure> {
        let (code, slot) = self.command(d0, d1, d2, d3)?;
        if code != CC_SUCCESS { return Err(Failure::Code(code)); }
        Ok(slot)
    }

    /// One control transfer on endpoint 0 of `slot`. `data` is a DMA buffer (phys) of
    /// `len` bytes; returns the bytes moved.
    pub fn control(&mut self, slot: u32, ep0: &mut Ring, request_type: u8, request: u8, value: u16,
        index: u16, data: u64, len: u16) -> Result<u32, Failure> {
        let input = request_type & 0x80 != 0;
        let trt = if len == 0 { 0 } else if input { 3 } else { 2 };
        let first = ep0.push(request_type as u32 | ((request as u32) << 8) | ((value as u32) << 16),
            index as u32 | ((len as u32) << 16), 8, (trb::SETUP << 10) | trb::IDT | (trt << 16));
        let mut data_at = 0;
        if len > 0 {
            data_at = ep0.push(data as u32, (data >> 32) as u32, len as u32,
                (trb::DATA << 10) | trb::ISP | ((input as u32) << 16));
        }
        let status_in = len == 0 || !input;
        let last = ep0.push(0, 0, 0, (trb::STATUS << 10) | trb::IOC | ((status_in as u32) << 16));
        self.doorbell(slot, 1);
        let mut moved = len as u32;
        let ring = *ep0;
        loop {
            let e = self.wait_event(600, |e| e.kind() == trb::TRANSFER_EVENT && e.slot() == slot
                && e.endpoint() == 1 && ring.index_of(e.pointer()).is_some());
            let Some(e) = e else {
                log!("[sexusb.xhci.control.timeout] slot={} request={:#x} value={:#x} len={} usbsts={:#x}", slot, request, value, len,
                    mmio_read32(self.op, 0x04));
                return Err(Failure::Timeout);
            };
            let at = e.pointer();
            let _ = first;
            if e.code() == CC_SHORT_PACKET && at == data_at {
                moved = (len as u32).saturating_sub(e.residual());
                continue;
            }
            if e.code() != CC_SUCCESS && e.code() != CC_SHORT_PACKET {
                self.recover(slot, 1, ep0);
                return Err(Failure::Code(e.code()));
            }
            if at == last { return Ok(moved); }
        }
    }

    /// One bulk transfer of `len` bytes at `phys` (contiguous), split at page edges so
    /// no TRB crosses a 64 KiB line. Returns the bytes moved, or the completion code.
    pub fn bulk(&mut self, slot: u32, dci: u32, ring: &mut Ring, phys: u64, len: u32, polls: u32) -> Result<u32, Failure> {
        let mut at = [0u64; 24];
        let mut size = [0u32; 24];
        let mut n = 0usize;
        let mut done = 0u32;
        while done < len && n < at.len() {
            let addr = phys + done as u64;
            let room = (PAGE - (addr & (PAGE - 1))) as u32;
            let chunk = room.min(len - done);
            let last = done + chunk == len;
            let flags = (trb::NORMAL << 10) | trb::ISP | if last { trb::IOC } else { 1 << 4 };
            at[n] = ring.push(addr as u32, (addr >> 32) as u32, chunk, flags);
            size[n] = chunk;
            n += 1;
            done += chunk;
        }
        if done != len { return Err(Failure::Code(0)); }
        self.doorbell(slot, dci);
        let range = *ring;
        let Some(e) = self.wait_event(polls, |e| e.kind() == trb::TRANSFER_EVENT && e.slot() == slot
            && e.endpoint() == dci && range.index_of(e.pointer()).is_some()) else {
            log!("[sexusb.xhci.bulk.timeout] slot={} dci={} len={}", slot, dci, len);
            self.recover(slot, dci, ring);
            return Err(Failure::Timeout);
        };
        let code = e.code();
        if code != CC_SUCCESS && code != CC_SHORT_PACKET {
            self.recover(slot, dci, ring);
            return Err(Failure::Code(code));
        }
        let which = at[..n].iter().position(|a| *a == e.pointer()).unwrap_or(n - 1);
        let before: u32 = size[..which].iter().sum();
        Ok(before + size[which].saturating_sub(e.residual()))
    }

    /// After a stall or error the endpoint halts: reset it and point it past what failed.
    pub fn recover(&mut self, slot: u32, dci: u32, ring: &mut Ring) {
        // A running endpoint (a timeout) must stop first; a halted one refuses, harmlessly.
        let _ = self.command(0, 0, 0, (trb::STOP_ENDPOINT << 10) | (dci << 16) | (slot << 24));
        let _ = self.command(0, 0, 0, (trb::RESET_ENDPOINT << 10) | (dci << 16) | (slot << 24));
        let deq = ring.dequeue_pointer();
        let _ = self.command(deq as u32, (deq >> 32) as u32, 0,
            (trb::SET_TR_DEQUEUE << 10) | (dci << 16) | (slot << 24));
    }
}

/// PORTSC bits that are kept when writing (RO and RWS), without RW1C changes or PED.
pub const PORT_KEEP: u32 = 0x4E00_FFE9 & !(1 << 1) & !(0xf << 5);
pub const PORT_CHANGES: u32 = 0x00FE_0000;

fn scratchpads(hcs2: u32) -> u32 { ((hcs2 >> 27) & 0x1f) | (((hcs2 >> 21) & 0x1f) << 5) }

/// USB Legacy Support: ask the firmware (SMM) to let go of the controller.
fn handoff(cap: u64, ext: u64) -> Result<(), Failure> {
    let v = mmio_read32(cap, ext);
    if v & (1 << 16) != 0 {
        mmio_write32(cap, ext, v | (1 << 24));
        let mut released = false;
        for _ in 0..1000 {
            if mmio_read32(cap, ext) & (1 << 16) == 0 { released = true; break; }
            sleep_ticks(1);
        }
        log!("[sexusb.xhci.handoff] firmware_owned=1 released={}", released as u8);
        if !released {
            // As Linux does: take it anyway rather than leave the machine without input.
            mmio_write32(cap, ext, mmio_read32(cap, ext) & !(1 << 16));
        }
    } else {
        mmio_write32(cap, ext, v | (1 << 24));
    }
    // Turn off the firmware's SMIs and clear their status.
    let ctl = mmio_read32(cap, ext + 4);
    let keep = (0x7 << 1) | (0xff << 5) | (0x7 << 17);
    mmio_write32(cap, ext + 4, (ctl & keep) | (0x7 << 29));
    Ok(())
}
