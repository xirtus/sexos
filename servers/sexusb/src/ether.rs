// SPDX-License-Identifier: GPL-3.0-only
//! USB network adapters speaking CDC Ethernet (ECM): most USB Ethernet dongles, phone
//! tethering, and Realtek RTL8152/8153 adapters through their ECM configuration.
//!
//! One adapter at a time. Frames go to and come from SexNet through the shared region
//! of `sex_pdx::usb_net`: received frames land in its receive ring as the controller
//! finishes them; frames SexNet queues are sent from `pump`, called every loop turn.

use sex_pdx::serial_record as log;
use sex_pdx::usb_net::{Shared, MAX_FRAME};
use super::{Usb, Failure, Ring, xhci, trb, page, CC_SHORT_PACKET, CC_SUCCESS};

const TRBS: u32 = 32; // 31 positions, one 2 KiB buffer each
const POSITIONS: u64 = TRBS as u64 - 1;
const BUFFER: u64 = 2048;
const RX_IN_FLIGHT: u32 = 24;
/// Frames on the wire at once; a frame may take two positions (data and a zero-length packet).
const TX_IN_FLIGHT: u32 = 12;

/// The adapter being driven.
#[derive(Clone, Copy)]
pub(super) struct Ether {
    pub(super) dev: usize,
    in_dci: u32,
    out_dci: u32,
    in_ring: Ring,
    out_ring: Ring,
    out_mps: u16,
    tx_in_flight: u32,
    errors: u32,
    pub(super) mac: [u8; 6],
    pub(super) received: u64,
    pub(super) sent: u64,
}

/// What the configuration descriptors say about an ECM function.
pub(super) struct Found {
    config: u8,
    comm: u8,
    data: u8,
    alt: u8,
    input: (u8, u16, u8),
    output: (u8, u16, u8),
    mac_string: u8,
}

/// Find an ECM function in one configuration descriptor.
pub(super) fn parse(cfg: &[u8]) -> Option<Found> {
    if cfg.len() < 9 { return None; }
    let config = cfg[5];
    let (mut comm, mut mac_string) = (None, 0u8);
    let (mut iface, mut alt, mut class) = (0u8, 0u8, 0u8);
    let (mut input, mut output) = ((0u8, 0u16, 0u8), (0u8, 0u16, 0u8));
    let mut data: Option<(u8, u8)> = None;
    let mut last_in: Option<bool> = None;
    let mut at = 0;
    while at + 2 <= cfg.len() {
        let len = cfg[at] as usize;
        if len < 2 || at + len > cfg.len() { break; }
        match cfg[at + 1] {
            4 if len >= 9 => {
                (iface, alt, class) = (cfg[at + 2], cfg[at + 3], cfg[at + 5]);
                // Communications class, Ethernet Networking Control Model.
                if class == 2 && cfg[at + 6] == 6 && comm.is_none() { comm = Some(iface); }
                last_in = None;
            }
            // CS_INTERFACE, Ethernet Networking functional descriptor: iMACAddress.
            0x24 if len >= 13 && cfg[at + 2] == 0x0F && class == 2 => mac_string = cfg[at + 3],
            5 if len >= 7 && class == 0x0A && cfg[at + 3] & 3 == 2 && comm.is_some() => {
                // The data interface's setting with two bulk endpoints (alt 1 in ECM).
                if data.is_some_and(|d| d != (iface, alt)) { at += len; continue; }
                data = Some((iface, alt));
                let (addr, mps) = (cfg[at + 2], u16::from_le_bytes([cfg[at + 4], cfg[at + 5]]));
                if addr & 0x80 != 0 { input = (addr, mps, 0); last_in = Some(true); } else { output = (addr, mps, 0); last_in = Some(false); }
            }
            0x30 if len >= 3 => match last_in {
                Some(true) => input.2 = cfg[at + 2],
                Some(false) => output.2 = cfg[at + 2],
                None => {}
            },
            _ => {}
        }
        at += len;
    }
    let (comm, (data, alt)) = (comm?, data?);
    if input.0 == 0 || output.0 == 0 || mac_string == 0 { return None; }
    Some(Found { config, comm, data, alt, input, output, mac_string })
}

/// "525400123456" (UTF-16LE in a string descriptor) to its six bytes.
pub(super) fn mac_from_string(desc: &[u8]) -> Option<[u8; 6]> {
    if desc.len() < 2 + 24 { return None; }
    let mut mac = [0u8; 6];
    for (k, byte) in mac.iter_mut().enumerate() {
        let hex = |c: u8| (c as char).to_digit(16);
        let (hi, lo) = (hex(desc[2 + 4 * k])?, hex(desc[2 + 4 * k + 2])?);
        *byte = (hi * 16 + lo) as u8;
    }
    Some(mac)
}

impl Usb {
    /// Look through every configuration of device `i` for an ECM function; drive it if the
    /// device has nothing else we drive and no adapter is running yet.
    pub(super) fn try_ether(&mut self, i: usize, configs: u8) -> Result<bool, Failure> {
        if self.ether.is_some() || self.net_region == 0 { return Ok(false); }
        let buf = self.dev[i].mem.unwrap().buf;
        let mut found = None;
        for index in 0..configs.min(4) {
            if self.get(i, 0x80, 6, 0x0200 | index as u16, 0, buf.0, 9).is_err() { continue; }
            let total = unsafe { core::ptr::read_unaligned((buf.1 + 2) as *const u16) }.min(2048);
            let Ok(got) = self.get(i, 0x80, 6, 0x0200 | index as u16, 0, buf.0, total) else { continue };
            let cfg = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, (got as usize).min(2048)) };
            if let Some(f) = parse(cfg) { found = Some(f); break; }
        }
        let Some(f) = found else { return Ok(false) };
        let slot = self.dev[i].slot;
        // MAC address string, in US English.
        let got = self.get(i, 0x80, 6, 0x0300 | f.mac_string as u16, 0x0409, buf.0, 64)? as usize;
        let desc = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, got.min(64)) };
        let Some(mac) = mac_from_string(desc) else {
            log!("[sexusb.ether.fail] slot={} reason=mac_string", slot);
            return Ok(false);
        };
        if self.ether_buffers.0 .0 == 0 {
            let p = &mut self.x.pool;
            self.ether_buffers = (page(p, 16)?, page(p, 16)?);
        }
        let mem = self.dev[i].mem.unwrap();
        let (in_dci, out_dci) = ((f.input.0 & 0xf) as u32 * 2 + 1, (f.output.0 & 0xf) as u32 * 2);
        let mut in_ring = Ring::empty();
        in_ring.init(mem.rings[0].0, mem.rings[0].1, TRBS);
        let mut out_ring = Ring::empty();
        out_ring.init(mem.rings[1].0, mem.rings[1].1, TRBS);
        self.input_context(i, 0, 0);
        self.endpoint_context_burst(i, in_dci, 6, f.input.1 & 0x7ff, f.input.2, in_ring.dequeue_pointer());
        self.endpoint_context_burst(i, out_dci, 2, f.output.1 & 0x7ff, f.output.2, out_ring.dequeue_pointer());
        self.input_flags(i, 1 | (1 << in_dci) | (1 << out_dci));
        self.slot_context(i, in_dci.max(out_dci));
        self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (slot << 24))?;
        self.set(i, 0x00, 9, f.config as u16, 0)?;
        // The data interface's alternate setting with the endpoints turns the link on.
        self.set(i, 0x01, 0x0B, f.alt as u16, f.data as u16)?;
        // SET_ETHERNET_PACKET_FILTER: directed, broadcast, all multicast (IPv6 needs it).
        let _ = self.set(i, 0x21, 0x43, 0x0E, f.comm as u16);
        self.ether = Some(Ether { dev: i, in_dci, out_dci, in_ring, out_ring, out_mps: f.output.1 & 0x7ff,
            tx_in_flight: 0, errors: 0, mac, received: 0, sent: 0 });
        for _ in 0..RX_IN_FLIGHT { self.ether_arm(); }
        self.x.doorbell(slot, in_dci);
        unsafe { Shared::at(self.net_region as *mut u8) }.set_link(Some(mac));
        log!("[sexusb.ether] slot={} config={} iface={} alt={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} in_mps={} out_mps={} ok=1",
            slot, f.config, f.data, f.alt, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], f.input.1 & 0x7ff, f.output.1 & 0x7ff);
        Ok(true)
    }

    /// Queue one receive TRB with the buffer of its ring position (no doorbell).
    fn ether_arm(&mut self) {
        let rx = self.ether_buffers.0 .0;
        let Some(e) = self.ether.as_mut() else { return };
        let at = rx + e.in_ring.idx as u64 * BUFFER;
        e.in_ring.push(at as u32, (at >> 32) as u32, BUFFER as u32, (trb::NORMAL << 10) | trb::IOC | trb::ISP);
    }

    /// A transfer event on the adapter's endpoints. False: not the adapter's.
    pub(super) fn ether_event(&mut self, i: usize, e: xhci::Event) -> bool {
        let Some(mut a) = self.ether.filter(|a| a.dev == i) else { return false };
        let dci = e.endpoint();
        let code = e.code();
        let ok = code == CC_SUCCESS || code == CC_SHORT_PACKET;
        let slot = self.dev[i].slot;
        if dci == a.in_dci {
            let Some(pos) = a.in_ring.index_of(e.pointer()) else { return true };
            if ok {
                let len = (BUFFER as u32).saturating_sub(e.residual()) as usize;
                if (14..=MAX_FRAME).contains(&len) {
                    let frame = unsafe { core::slice::from_raw_parts((self.ether_buffers.0 .1 + pos as u64 * BUFFER) as *const u8, len) };
                    unsafe { Shared::at(self.net_region as *mut u8) }.rx_push(frame);
                    a.received += 1;
                    if a.received <= 4 || a.received.is_power_of_two() { log!("[sexusb.ether.rx] frames={} length={}", a.received, len); }
                }
                self.ether = Some(a);
                self.ether_arm();
            } else {
                a.errors += 1;
                log!("[sexusb.ether.error] slot={} dci={} code={} errors={}", slot, dci, code, a.errors);
                self.x.recover(slot, dci, &mut a.in_ring);
                self.ether = Some(a);
                if a.errors > 64 { return true; }
                for _ in 0..RX_IN_FLIGHT { self.ether_arm(); }
            }
            self.x.doorbell(slot, dci);
        } else if dci == a.out_dci {
            a.tx_in_flight = a.tx_in_flight.saturating_sub(1);
            if !ok {
                a.errors += 1;
                log!("[sexusb.ether.error] slot={} dci={} code={} errors={}", slot, dci, code, a.errors);
                self.x.recover(slot, dci, &mut a.out_ring);
                a.tx_in_flight = 0;
            }
            self.ether = Some(a);
        }
        true
    }

    /// Send what SexNet queued, as far as the ring allows. True if anything moved.
    pub fn ether_pump(&mut self) -> bool {
        let Some(mut a) = self.ether else { return false };
        let shared = unsafe { Shared::at(self.net_region as *mut u8) };
        let (tx, tx_va) = self.ether_buffers.1;
        let mut moved = false;
        while a.tx_in_flight < TX_IN_FLIGHT {
            let pos = a.out_ring.idx as u64;
            debug_assert!(pos < POSITIONS);
            let dst = unsafe { core::slice::from_raw_parts_mut((tx_va + pos * BUFFER) as *mut u8, MAX_FRAME) };
            let Some(n) = shared.tx_pop(dst) else { break };
            let at = tx + pos * BUFFER;
            // A frame filling whole packets is ended by a zero-length packet.
            let zlp = a.out_mps != 0 && n % a.out_mps as usize == 0;
            a.out_ring.push(at as u32, (at >> 32) as u32, n as u32, (trb::NORMAL << 10) | trb::IOC);
            if zlp { a.out_ring.push(0, 0, 0, trb::NORMAL << 10); }
            a.tx_in_flight += 1;
            a.sent += 1;
            if a.sent <= 4 || a.sent.is_power_of_two() { log!("[sexusb.ether.tx] frames={} length={}", a.sent, n); }
            moved = true;
        }
        let slot = self.dev[a.dev].slot;
        let dci = a.out_dci;
        self.ether = Some(a);
        if moved { self.x.doorbell(slot, dci); }
        moved
    }

    /// The adapter's device left.
    pub(super) fn ether_gone(&mut self, i: usize) {
        if self.ether.is_some_and(|a| a.dev == i) {
            self.ether = None;
            unsafe { Shared::at(self.net_region as *mut u8) }.set_link(None);
            log!("[sexusb.ether.gone] slot={}", self.dev[i].slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mac_string() {
        let s: Vec<u8> = [26u8, 3].into_iter().chain("525400AbCdEf".encode_utf16().flat_map(|c| c.to_le_bytes())).collect();
        assert_eq!(mac_from_string(&s), Some([0x52, 0x54, 0, 0xab, 0xcd, 0xef]));
        assert_eq!(mac_from_string(&s[..20]), None);
    }
}
