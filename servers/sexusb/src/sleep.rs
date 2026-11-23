// SPDX-License-Identifier: GPL-3.0-only
//! Selective suspend while the machine sleeps (xHCI 4.15, USB 2.0 7.1.7.6, USB 3 U3).
//!
//! When the kernel says the machine is going to sleep, root ports whose device only
//! moves data on request (storage, idle devices) go to U3: their caches are flushed,
//! their endpoints stopped, and the link suspended, so the device drops to its
//! suspend current. Keyboards, pointers, hubs (which may carry them), the network
//! adapter and Bluetooth stay up, because they are what wakes the machine or what
//! must keep its connection. Coming back, the ports resume (USB 2: 20 ms of resume
//! signalling first) and stopped endpoints start again at their next doorbell. A disk
//! request that arrives while a port is suspended resumes it first.

use sex_pdx::serial_record as log;
use super::{Usb, xhci, trb, MAX_DEVICES};

/// PORTSC: link state (PLS), link write strobe (LWS), link state change (PLC).
const PLS_SHIFT: u32 = 5;
const PLS_MASK: u32 = 0xf << PLS_SHIFT;
const LWS: u32 = 1 << 16;
const PLC: u32 = 1 << 22;
const U0: u32 = 0;
const U3: u32 = 3;
const RESUME: u32 = 15;
/// Stop Endpoint's Suspend flag: the endpoint is stopped for a suspend.
const SUSPEND_FLAG: u32 = 1 << 23;

impl Usb {
    /// May this device's root port be suspended while the machine sleeps?
    fn may_suspend(&self, i: usize) -> bool {
        let d = &self.dev[i];
        d.used && d.parent == 0 && !d.is_hub() && !d.intr.iter().any(|x| x.active)
            && !self.ether.as_ref().is_some_and(|e| e.dev == i) && !self.bt.as_ref().is_some_and(|b| b.dev == i)
    }

    /// The machine is going to sleep (true) or has woken. Returns the ports changed.
    pub fn device_sleep(&mut self, asleep: bool) -> u32 {
        if !asleep { return self.resume_all(); }
        let mut n = 0;
        for i in 0..MAX_DEVICES {
            if !self.may_suspend(i) { continue; }
            self.select(self.dev[i].ctl);
            let c = self.ctl as usize;
            let port = self.dev[i].root_port;
            if port == 0 || port > 63 || self.suspended[c] & (1 << port) != 0 { continue; }
            if self.x.portsc(port) & 2 == 0 { continue; }
            let st = self.dev[i].storage;
            let flushed = !st.active || self.scsi(i, &[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], false, 0).is_ok();
            let slot = self.dev[i].slot;
            // Endpoints first (xHCI 4.15.1): bulk ones of storage; EP0 is idle between requests.
            if st.active {
                for dci in [st.in_dci, st.out_dci] {
                    let _ = self.x.command(0, 0, 0, (trb::STOP_ENDPOINT << 10) | (dci << 16) | (slot << 24) | SUSPEND_FLAG);
                }
            }
            self.select(c as u8);
            let ok = self.x.set_link(port, U3);
            if ok { self.suspended[c] |= 1 << port; n += 1; }
            log!("[sexusb.suspend] port={} slot={} id={:04x}:{:04x} storage={} flushed={} ok={}",
                port, slot, self.dev[i].vendor, self.dev[i].product, st.active as u8, flushed as u8, ok as u8);
        }
        n
    }

    /// Resume every port suspended for sleep.
    pub fn resume_all(&mut self) -> u32 {
        let mut n = 0;
        for c in 0..self.controllers {
        self.select(c);
        while self.suspended[c as usize] != 0 {
            let port = self.suspended[c as usize].trailing_zeros();
            self.suspended[c as usize] &= !(1 << port);
            let s = self.x.portsc(port);
            // Unplugged while asleep: the change event detaches it as usual.
            if s & 1 == 0 { continue; }
            if self.x.port_major[port as usize] != 3 {
                self.x.set_portsc(port, (s & xhci::PORT_KEEP & !PLS_MASK) | LWS | (RESUME << PLS_SHIFT));
                xhci::sleep_ticks(20); // ticks are 1 ms
            }
            let ok = self.x.set_link(port, U0);
            if ok { n += 1; }
            log!("[sexusb.resume] port={} ok={}", port, ok as u8);
        }
        }
        n
    }
}

impl xhci::Xhci {
    /// Move a root port's link to `state` and wait (up to ~100 ms) until it is there.
    pub fn set_link(&self, port: u32, state: u32) -> bool {
        let s = self.portsc(port);
        self.set_portsc(port, (s & xhci::PORT_KEEP & !PLS_MASK) | LWS | (state << PLS_SHIFT));
        for _ in 0..100 {
            let s = self.portsc(port);
            if (s & PLS_MASK) >> PLS_SHIFT == state {
                if s & PLC != 0 { self.set_portsc(port, (s & xhci::PORT_KEEP) | PLC); }
                return true;
            }
            xhci::sleep_ticks(1);
        }
        false
    }
}
