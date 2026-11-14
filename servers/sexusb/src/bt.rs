// SPDX-License-Identifier: GPL-3.0-only
//! USB Bluetooth controllers (HCI over USB, interface class 0xE0/0x01/0x01): the
//! transport half of Cufflinks. Commands go out on the control endpoint (class request
//! to the interface: bmRequestType 0x20, bRequest 0), events come in on the interrupt
//! endpoint, ACL data on the bulk pair. Packets move to and from Cufflinks through the
//! shared region of `sex_pdx::bt_hci`; nothing here looks inside them beyond the event
//! length that splits the interrupt stream.
//!
//! Without the region (BOOT does not grant it yet) the controller is still set up and
//! asked for its version once, and the first events are logged, so a real machine shows
//! in its boot log that the transport works.

use sex_pdx::serial_record as log;
use sex_pdx::bt_hci::{self, Shared};
use super::{Usb, Failure, Ring, xhci, trb, page, CC_SHORT_PACKET, CC_SUCCESS};

/// 32 positions and a link TRB, so position % IN_FLIGHT is unique among TRBs in flight.
const TRBS: u32 = 33;
const IN_FLIGHT: u32 = 8;
const EVENT_BUFFER: u64 = 512;
const DATA_BUFFER: u64 = 2048;
const STREAM: usize = 1024;

/// The controller being driven.
pub(super) struct Bt {
    pub(super) dev: usize,
    intr_dci: u32,
    in_dci: u32,
    out_dci: u32,
    intr_ring: Ring,
    in_ring: Ring,
    out_ring: Ring,
    out_in_flight: u32,
    errors: u32,
    events: u32,
    /// Interrupt bytes not yet a whole event.
    stream: [u8; STREAM],
    stream_len: usize,
}

/// What the configuration descriptor says about a Bluetooth HCI interface.
pub(super) struct Found { iface: u8, intr: (u8, u16, u8), input: (u8, u16), output: (u8, u16) }

/// Find the HCI interface (alternate setting 0) in one configuration descriptor.
pub(super) fn parse(cfg: &[u8]) -> Option<Found> {
    let (mut hci, mut iface, mut alt) = (None, 0u8, 0u8);
    let (mut intr, mut input, mut output) = ((0u8, 0u16, 0u8), (0u8, 0u16), (0u8, 0u16));
    let mut at = 0;
    while at + 2 <= cfg.len() {
        let len = cfg[at] as usize;
        if len < 2 || at + len > cfg.len() { break; }
        match cfg[at + 1] {
            4 if len >= 9 => {
                (iface, alt) = (cfg[at + 2], cfg[at + 3]);
                if cfg[at + 5] == 0xE0 && cfg[at + 6] == 0x01 && cfg[at + 7] == 0x01 && alt == 0 && hci.is_none() { hci = Some(iface); }
            }
            5 if len >= 7 && alt == 0 && hci == Some(iface) => {
                let (addr, attr) = (cfg[at + 2], cfg[at + 3] & 3);
                let mps = u16::from_le_bytes([cfg[at + 4], cfg[at + 5]]);
                match (attr, addr & 0x80 != 0) {
                    (3, true) => intr = (addr, mps, cfg[at + 6]),
                    (2, true) => input = (addr, mps),
                    (2, false) => output = (addr, mps),
                    _ => {}
                }
            }
            _ => {}
        }
        at += len;
    }
    let iface = hci?;
    if intr.0 == 0 || input.0 == 0 || output.0 == 0 { return None; }
    Some(Found { iface, intr, input, output })
}

/// Whole events at the front of `stream` (code, length, parameters): their lengths.
fn next_event(stream: &[u8]) -> Option<usize> {
    if stream.len() < 2 { return None; }
    let n = 2 + stream[1] as usize;
    (stream.len() >= n).then_some(n)
}

impl Usb {
    /// Drive device `i` as a Bluetooth controller if its configuration has an HCI interface.
    pub(super) fn try_bt(&mut self, i: usize, cfg: &[u8], config: u8, speed: u32) -> Result<bool, Failure> {
        if self.bt.is_some() { return Ok(false); }
        let Some(f) = parse(cfg) else { return Ok(false) };
        let slot = self.dev[i].slot;
        let (vendor, product) = (self.dev[i].vendor, self.dev[i].product);
        if self.bt_buffers.0 .0 == 0 {
            let p = &mut self.x.pool;
            self.bt_buffers = (page(p, 1)?, page(p, 4)?, page(p, 4)?);
        }
        let mem = self.dev[i].mem.unwrap();
        let intr_dci = (f.intr.0 & 0xf) as u32 * 2 + 1;
        let in_dci = (f.input.0 & 0xf) as u32 * 2 + 1;
        let out_dci = (f.output.0 & 0xf) as u32 * 2;
        let mut rings = [Ring::empty(); 3];
        for (k, r) in rings.iter_mut().enumerate() { r.init(mem.rings[k].0, mem.rings[k].1, TRBS); }
        self.input_context(i, 0, 0);
        let interval = super::interval_exponent(speed, f.intr.2);
        let imps = f.intr.1 & 0x7ff;
        self.endpoint_context(i, intr_dci, 7, imps, interval, rings[0].dequeue_pointer(), imps as u32, imps as u32);
        self.endpoint_context_burst(i, in_dci, 6, f.input.1 & 0x7ff, 0, rings[1].dequeue_pointer());
        self.endpoint_context_burst(i, out_dci, 2, f.output.1 & 0x7ff, 0, rings[2].dequeue_pointer());
        self.input_flags(i, 1 | (1 << intr_dci) | (1 << in_dci) | (1 << out_dci));
        self.slot_context(i, intr_dci.max(in_dci).max(out_dci));
        self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (slot << 24))?;
        self.set(i, 0x00, 9, config as u16, 0)?;
        self.bt = Some(Bt { dev: i, intr_dci, in_dci, out_dci, intr_ring: rings[0], in_ring: rings[1], out_ring: rings[2],
            out_in_flight: 0, errors: 0, events: 0, stream: [0; STREAM], stream_len: 0 });
        for _ in 0..IN_FLIGHT { self.bt_arm(true); self.bt_arm(false); }
        self.x.doorbell(slot, intr_dci);
        self.x.doorbell(slot, in_dci);
        let route = self.bt_region != 0;
        log!("[sexusb.bt] vendor={:#06x} product={:#06x} slot={} iface={} intr_ep={:#04x} bulk_in={:#04x} bulk_out={:#04x} route={}",
            vendor, product, slot, f.iface, f.intr.0, f.input.0, f.output.0, route as u8);
        if route {
            unsafe { Shared::at(self.bt_region as *mut u8) }.set_link(Some((vendor, product)));
        } else {
            // No host stack to hand packets to: ask for the version once, to show the link works.
            let probe: &[u8] = if vendor == 0x8087 { &[0x05, 0xfc, 0x01, 0xff] } else { &[0x01, 0x10, 0x00] };
            let ok = self.bt_command(probe).is_ok();
            log!("[sexusb.bt.probe] opcode={:#06x} sent={}", u16::from_le_bytes([probe[0], probe[1]]), ok as u8);
        }
        Ok(true)
    }

    /// Send one HCI command (opcode, length, parameters) on the control endpoint.
    fn bt_command(&mut self, cmd: &[u8]) -> Result<u32, Failure> {
        let Some(i) = self.bt.as_ref().map(|b| b.dev) else { return Err(Failure::Code(0)) };
        let buf = self.dev[i].mem.unwrap().buf;
        let n = cmd.len().min(xhci::PAGE as usize);
        unsafe { core::ptr::copy_nonoverlapping(cmd.as_ptr(), buf.1 as *mut u8, n); }
        self.get(i, 0x20, 0, 0, 0, buf.0, n as u16)
    }

    /// Queue one receive TRB on the interrupt (`events`) or bulk IN endpoint.
    fn bt_arm(&mut self, events: bool) {
        let (intr, data, _) = self.bt_buffers;
        let Some(b) = self.bt.as_mut() else { return };
        let (ring, base, size) = if events { (&mut b.intr_ring, intr.0, EVENT_BUFFER) } else { (&mut b.in_ring, data.0, DATA_BUFFER) };
        let at = base + (ring.idx % IN_FLIGHT) as u64 * size;
        ring.push(at as u32, (at >> 32) as u32, size as u32, (trb::NORMAL << 10) | trb::IOC | trb::ISP);
    }

    fn bt_deliver(&mut self, kind: u8, packet_at: u64, len: usize) {
        let body = unsafe { core::slice::from_raw_parts(packet_at as *const u8, len) };
        if self.bt_region != 0 {
            unsafe { Shared::at(self.bt_region as *mut u8) }.rx_push_typed(kind, body);
        }
    }

    /// A transfer event on the controller's endpoints. False: not the controller's.
    pub(super) fn bt_event(&mut self, i: usize, e: xhci::Event) -> bool {
        let Some(b) = self.bt.as_ref().filter(|b| b.dev == i) else { return false };
        let (dci, code) = (e.endpoint(), e.code());
        let ok = code == CC_SUCCESS || code == CC_SHORT_PACKET;
        let slot = self.dev[i].slot;
        let (intr_dci, in_dci, out_dci, intr_ring, in_ring) = (b.intr_dci, b.in_dci, b.out_dci, b.intr_ring, b.in_ring);
        let (intr, data, _) = self.bt_buffers;
        if dci == out_dci {
            let b = self.bt.as_mut().unwrap();
            b.out_in_flight = b.out_in_flight.saturating_sub(1);
            if !ok {
                b.errors += 1;
                log!("[sexusb.bt.error] slot={} dci={} code={}", slot, dci, code);
                let mut r = b.out_ring;
                self.x.recover(slot, dci, &mut r);
                let b = self.bt.as_mut().unwrap();
                b.out_ring = r;
                b.out_in_flight = 0;
            }
            return true;
        }
        if dci != intr_dci && dci != in_dci { return true; }
        let events = dci == intr_dci;
        let ring = if events { intr_ring } else { in_ring };
        let Some(pos) = ring.index_of(e.pointer()) else { return true };
        if !ok {
            let b = self.bt.as_mut().unwrap();
            b.errors += 1;
            let errors = b.errors;
            log!("[sexusb.bt.error] slot={} dci={} code={} errors={}", slot, dci, code, errors);
            let mut r = ring;
            self.x.recover(slot, dci, &mut r);
            let b = self.bt.as_mut().unwrap();
            if events { b.intr_ring = r; b.stream_len = 0; } else { b.in_ring = r; }
            if errors > 64 { return true; }
            for _ in 0..IN_FLIGHT { self.bt_arm(events); }
            self.x.doorbell(slot, dci);
            return true;
        }
        let size = if events { EVENT_BUFFER } else { DATA_BUFFER };
        let len = (size as u32).saturating_sub(e.residual()) as usize;
        let at = (if events { intr.1 } else { data.1 }) + (pos % IN_FLIGHT) as u64 * size;
        if events {
            // Events may span interrupt transfers, or share one: split by their lengths.
            let mut out = [0u8; STREAM];
            let mut whole: [(usize, usize); 8] = [(0, 0); 8];
            let (mut n, mut first_logs) = (0usize, [false; 8]);
            {
                let b = self.bt.as_mut().unwrap();
                let room = STREAM - b.stream_len;
                let take = len.min(room);
                unsafe { core::ptr::copy_nonoverlapping(at as *const u8, b.stream.as_mut_ptr().add(b.stream_len), take); }
                b.stream_len += take;
                let mut start = 0;
                while n < whole.len() {
                    let Some(l) = next_event(&b.stream[start..b.stream_len]) else { break };
                    out[start..start + l].copy_from_slice(&b.stream[start..start + l]);
                    whole[n] = (start, l);
                    b.events += 1;
                    first_logs[n] = b.events <= 4;
                    n += 1;
                    start += l;
                }
                b.stream.copy_within(start..b.stream_len, 0);
                b.stream_len -= start;
                if b.stream_len == STREAM { b.stream_len = 0; } // garbage: start over
            }
            for (k, &(s, l)) in whole[..n].iter().enumerate() {
                let ev = &out[s..s + l];
                if first_logs[k] {
                    let h = |j: usize| ev.get(j).copied().unwrap_or(0);
                    log!("[sexusb.bt.event] code={:#04x} length={} head={:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                        ev[0], l, h(2), h(3), h(4), h(5), h(6), h(7), h(8), h(9));
                }
                self.bt_deliver(bt_hci::EVENT, out.as_ptr() as u64 + s as u64, l);
            }
        } else if len > 0 {
            self.bt_deliver(bt_hci::ACL, at, len);
        }
        self.bt_arm(events);
        self.x.doorbell(slot, dci);
        true
    }

    /// Send what Cufflinks queued. True if anything moved.
    pub fn bt_pump(&mut self) -> bool {
        if self.bt_region == 0 { return false; }
        let Some(b) = self.bt.as_ref() else { return false };
        let (dev, out_dci) = (b.dev, b.out_dci);
        let shared = unsafe { Shared::at(self.bt_region as *mut u8) };
        let (_, _, out) = self.bt_buffers;
        let mut moved = false;
        let mut packet = [0u8; bt_hci::MAX_PACKET];
        loop {
            if self.bt.as_ref().is_some_and(|b| b.out_in_flight >= IN_FLIGHT) { break; }
            let Some(n) = shared.tx_pop(&mut packet) else { break };
            moved = true;
            if n < 2 { continue; }
            match packet[0] {
                bt_hci::COMMAND => {
                    if let Err(e) = self.bt_command(&packet[1..n]) {
                        log!("[sexusb.bt.command.fail] opcode={:#06x} reason={:?}", u16::from_le_bytes([packet[1], *packet.get(2).unwrap_or(&0)]), e);
                    }
                }
                bt_hci::ACL | bt_hci::COMMAND_BULK => {
                    let b = self.bt.as_mut().unwrap();
                    let pos = b.out_ring.idx % IN_FLIGHT;
                    let at = (out.0 + pos as u64 * DATA_BUFFER, out.1 + pos as u64 * DATA_BUFFER);
                    let len = (n - 1).min(DATA_BUFFER as usize);
                    unsafe { core::ptr::copy_nonoverlapping(packet.as_ptr().add(1), at.1 as *mut u8, len); }
                    b.out_ring.push(at.0 as u32, (at.0 >> 32) as u32, len as u32, (trb::NORMAL << 10) | trb::IOC);
                    b.out_in_flight += 1;
                    let slot = self.dev[dev].slot;
                    self.x.doorbell(slot, out_dci);
                }
                _ => {}
            }
        }
        moved
    }

    /// The controller's device left.
    pub(super) fn bt_gone(&mut self, i: usize) {
        if self.bt.as_ref().is_some_and(|b| b.dev == i) {
            self.bt = None;
            if self.bt_region != 0 { unsafe { Shared::at(self.bt_region as *mut u8) }.set_link(None); }
            log!("[sexusb.bt.gone] slot={}", self.dev[i].slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_the_hci_interface() {
        // Config, interface 0 (E0/01/01) with interrupt 0x81, bulk 0x82 and 0x02, interface 1 (SCO, isochronous).
        let cfg = [9, 2, 0, 0, 2, 1, 0, 0xe0, 50,
            9, 4, 0, 0, 3, 0xe0, 1, 1, 0,
            7, 5, 0x81, 3, 64, 0, 1,
            7, 5, 0x82, 2, 64, 0, 1,
            7, 5, 0x02, 2, 64, 0, 1,
            9, 4, 1, 0, 2, 0xe0, 1, 1, 0,
            7, 5, 0x83, 1, 0, 0, 1];
        let f = parse(&cfg).unwrap();
        assert_eq!((f.iface, f.intr.0, f.input.0, f.output.0), (0, 0x81, 0x82, 0x02));
        assert!(parse(&cfg[..25]).is_none());
        assert_eq!(next_event(&[0x0e, 4, 1, 3, 0x0c, 0, 0xff]), Some(6));
        assert_eq!(next_event(&[0x0e, 4, 1]), None);
    }
}
