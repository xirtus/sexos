// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bluetooth HCI between SexUSB (the USB transport) and Cufflinks (the host stack):
//! HCI packets through one shared region, each with its one-byte packet type in front.
//!
//! SexUSB drives the controller's USB interface (class 0xE0/0x01/0x01) and writes its
//! ids into the header. Packets from the controller go into the receive ring: events
//! from the interrupt endpoint as [`EVENT`], data from bulk IN as [`ACL`]. Cufflinks
//! queues packets in the send ring: [`COMMAND`] goes out on the control endpoint
//! (bmRequestType 0x20, bRequest 0), [`ACL`] on bulk OUT, and [`COMMAND_BULK`] (an HCI
//! command sent on bulk OUT, which Intel bootloaders want for Secure Send) on bulk OUT.
//! Each ring has one producer and one consumer and both sides poll it, as in `usb_net`.
//!
//! BOOT starts Cufflinks as domain [`CUFFLINKS_PD`] and pairs this region between it and
//! SexUSB (kernel `boot_memory::init_bluetooth`). SexUSB also holds a route to Cufflinks
//! ([`NOTIFY_SLOT`]) and sends [`OP_LINK`] when a controller comes or goes, so an idle
//! Cufflinks (no controller) sleeps until then instead of polling.
//!
//! Input from Bluetooth keyboards and mice comes back the other way: Cufflinks decodes
//! the HID reports and queues them in a small mailbox in the header ([`HidInput`]);
//! SexUSB merges them with its own USB keyboards and mice and sends them to Input as it
//! does for USB, so Input keeps one producer.

use core::sync::atomic::{AtomicU32, Ordering};

/// The shared region, in SexUSB and in Cufflinks.
pub const SLOT: u64 = 1003;
pub const REGION_ID: u32 = 0x4402;
pub const BYTES: u64 = 128 * 1024;
/// Cufflinks's read-only firmware pack (`sex_bt::pack` layout): BOOT grants the
/// `firmware/wifi.pack` boot module here, which also carries the `ibt-*` files
/// (`scripts/make_wifi_firmware_pack.py`). Without it Cufflinks logs the firmware missing.
pub const FIRMWARE_SLOT: u64 = 1004;
pub const FIRMWARE_REGION_ID: u32 = 0x4403;
/// Cufflinks's BOOT identity (domain and layout), outside the catalog's rows.
pub const CUFFLINKS_PD: u32 = 52;
/// The Senses organ, with SexUSB and Input.
pub const CUFFLINKS_KEY: u8 = 4;
/// SexUSB's route to Cufflinks.
pub const NOTIFY_SLOT: u64 = 1005;
/// SexUSB -> Cufflinks: the link changed (arg0 = 1 up, 0 down). Cufflinks replies 0.
pub const OP_LINK: u64 = 0x2C8;

/// Packets each ring holds.
pub const SLOTS: u32 = 30;
/// Bytes a ring slot takes: a 4-byte length, then the packet.
pub const SLOT_BYTES: usize = 2048;
/// The largest packet with its type byte (ACL header and 1021-byte payload fit).
pub const MAX_PACKET: usize = SLOT_BYTES - 4;

/// Packet types (the first byte of every packet).
pub const COMMAND: u8 = 0x01;
pub const ACL: u8 = 0x02;
pub const EVENT: u8 = 0x04;
/// An HCI command (opcode, length, parameters follow) to send on bulk OUT.
pub const COMMAND_BULK: u8 = 0x81;

const MAGIC: u32 = 0x4243_4948; // "HICB"
const HEADER: usize = 4096;
const RX: usize = HEADER;
const TX: usize = HEADER + SLOTS as usize * SLOT_BYTES;

const W_MAGIC: usize = 0;
const W_LINK: usize = 4;
const W_IDS: usize = 8;
const W_GENERATION: usize = 12;
const W_RX_PROD: usize = 16;
const W_RX_CONS: usize = 20;
const W_TX_PROD: usize = 24;
const W_TX_CONS: usize = 28;
const W_RX_DROPPED: usize = 32;
const W_HID_PROD: usize = 36;
const W_HID_CONS: usize = 40;
/// Bluetooth HID devices connected now (Cufflinks writes it; SexUSB polls faster while non-zero).
const W_HID_LINKS: usize = 44;
const HID_RING: usize = 512;
/// HID records the mailbox holds.
pub const HID_SLOTS: u32 = 32;
const HID_RECORD: usize = 16;

/// One input report from a Bluetooth keyboard or mouse, already decoded by Cufflinks.
/// `device` names the device (Cufflinks's own small number) so SexUSB keeps each
/// keyboard's held keys apart and drops them when it goes ([`HidInput::Gone`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HidInput {
    /// A boot-keyboard snapshot: modifier bits and up to six pressed usage ids (low byte first).
    Keys { device: u8, modifiers: u8, keys: u64 },
    /// Relative pointer motion: buttons (bit 0 left), dx, dy, wheel.
    Pointer { device: u8, buttons: u8, dx: i16, dy: i16, wheel: i8 },
    /// The device went away: release whatever it held.
    Gone { device: u8 },
}

impl HidInput {
    fn encode(&self) -> [u8; HID_RECORD] {
        let mut r = [0u8; HID_RECORD];
        match *self {
            HidInput::Keys { device, modifiers, keys } => { r[0] = 1; r[1] = device; r[2] = modifiers; r[8..16].copy_from_slice(&keys.to_le_bytes()); }
            HidInput::Pointer { device, buttons, dx, dy, wheel } => {
                r[0] = 2; r[1] = device; r[2] = buttons;
                r[8..10].copy_from_slice(&dx.to_le_bytes()); r[10..12].copy_from_slice(&dy.to_le_bytes()); r[12] = wheel as u8;
            }
            HidInput::Gone { device } => { r[0] = 3; r[1] = device; }
        }
        r
    }
    fn decode(r: &[u8; HID_RECORD]) -> Option<HidInput> {
        let le16 = |at: usize| i16::from_le_bytes([r[at], r[at + 1]]);
        match r[0] {
            1 => Some(HidInput::Keys { device: r[1], modifiers: r[2], keys: u64::from_le_bytes(r[8..16].try_into().ok()?) & 0xffff_ffff_ffff }),
            2 => Some(HidInput::Pointer { device: r[1], buttons: r[2], dx: le16(8), dy: le16(10), wheel: r[12] as i8 }),
            3 => Some(HidInput::Gone { device: r[1] }),
            _ => None,
        }
    }
}

/// The region, seen from either side. `base` must be [`BYTES`] long and 4-byte aligned.
#[derive(Clone, Copy)]
pub struct Shared { base: *mut u8 }

impl Shared {
    /// # Safety
    /// `base` points at a live mapping of at least [`BYTES`] bytes for as long as this is used.
    pub unsafe fn at(base: *mut u8) -> Shared { Shared { base } }
    fn word(&self, at: usize) -> &AtomicU32 { unsafe { &*(self.base.add(at) as *const AtomicU32) } }
    fn load(&self, at: usize) -> u32 { self.word(at).load(Ordering::Acquire) }
    fn store(&self, at: usize, v: u32) { self.word(at).store(v, Ordering::Release) }

    /// SexUSB: a controller is ready (`Some((vendor, product))`) or gone (`None`). Rings start empty.
    pub fn set_link(&self, ids: Option<(u16, u16)>) {
        self.store(W_LINK, 0);
        for w in [W_RX_PROD, W_RX_CONS, W_TX_PROD, W_TX_CONS] { self.store(w, 0); }
        if let Some((vendor, product)) = ids {
            self.store(W_IDS, vendor as u32 | (product as u32) << 16);
            self.store(W_GENERATION, self.load(W_GENERATION).wrapping_add(1));
            self.store(W_MAGIC, MAGIC);
            self.store(W_LINK, 1);
        }
    }
    /// ((vendor, product), generation) while a controller is ready. The generation
    /// changes when one is plugged in again, and the rings are then new.
    pub fn link(&self) -> Option<((u16, u16), u32)> {
        if self.load(W_MAGIC) != MAGIC || self.load(W_LINK) != 1 { return None; }
        let ids = self.load(W_IDS);
        Some(((ids as u16, (ids >> 16) as u16), self.load(W_GENERATION)))
    }
    pub fn rx_dropped(&self) -> u32 { self.load(W_RX_DROPPED) }

    fn slot(&self, ring: usize, index: u32) -> *mut u8 { unsafe { self.base.add(ring + (index % SLOTS) as usize * SLOT_BYTES) } }
    fn push(&self, ring: usize, prod: usize, cons: usize, packet: &[u8]) -> bool {
        if packet.len() < 2 || packet.len() > MAX_PACKET { return false; }
        let (p, c) = (self.load(prod), self.load(cons));
        if p.wrapping_sub(c) >= SLOTS { return false; }
        let s = self.slot(ring, p);
        unsafe {
            core::ptr::copy_nonoverlapping(packet.as_ptr(), s.add(4), packet.len());
            (s as *mut u32).write_volatile(packet.len() as u32);
        }
        self.store(prod, p.wrapping_add(1));
        true
    }
    fn pop(&self, ring: usize, prod: usize, cons: usize, out: &mut [u8]) -> Option<usize> {
        let (p, c) = (self.load(prod), self.load(cons));
        if p == c { return None; }
        let s = self.slot(ring, c);
        let len = unsafe { (s as *const u32).read_volatile() } as usize;
        let n = len.min(MAX_PACKET).min(out.len());
        unsafe { core::ptr::copy_nonoverlapping(s.add(4), out.as_mut_ptr(), n); }
        self.store(cons, c.wrapping_add(1));
        Some(n)
    }

    /// SexUSB: a packet from the controller. False (and counted) when Cufflinks has not kept up.
    pub fn rx_push(&self, packet: &[u8]) -> bool {
        let ok = self.push(RX, W_RX_PROD, W_RX_CONS, packet);
        if !ok { self.store(W_RX_DROPPED, self.load(W_RX_DROPPED).wrapping_add(1)); }
        ok
    }
    /// SexUSB: an event or bulk data with its type byte put in front.
    pub fn rx_push_typed(&self, kind: u8, body: &[u8]) -> bool {
        let mut b = [0u8; MAX_PACKET];
        if body.len() + 1 > MAX_PACKET { return self.rx_push(&[]); }
        b[0] = kind;
        b[1..1 + body.len()].copy_from_slice(body);
        self.rx_push(&b[..1 + body.len()])
    }
    /// Cufflinks: queue one decoded input report for SexUSB. False when the mailbox is full.
    pub fn hid_push(&self, input: HidInput) -> bool {
        let (p, c) = (self.load(W_HID_PROD), self.load(W_HID_CONS));
        if p.wrapping_sub(c) >= HID_SLOTS { return false; }
        let at = HID_RING + (p % HID_SLOTS) as usize * HID_RECORD;
        let r = input.encode();
        unsafe { core::ptr::copy_nonoverlapping(r.as_ptr(), self.base.add(at), HID_RECORD); }
        self.store(W_HID_PROD, p.wrapping_add(1));
        true
    }
    /// SexUSB: the next decoded input report.
    pub fn hid_pop(&self) -> Option<HidInput> {
        loop {
            let (p, c) = (self.load(W_HID_PROD), self.load(W_HID_CONS));
            if p == c { return None; }
            let at = HID_RING + (c % HID_SLOTS) as usize * HID_RECORD;
            let mut r = [0u8; HID_RECORD];
            unsafe { core::ptr::copy_nonoverlapping(self.base.add(at), r.as_mut_ptr(), HID_RECORD); }
            self.store(W_HID_CONS, c.wrapping_add(1));
            if let Some(i) = HidInput::decode(&r) { return Some(i); }
        }
    }
    /// Cufflinks: how many HID devices are connected.
    pub fn set_hid_links(&self, n: u32) { self.store(W_HID_LINKS, n); }
    /// SexUSB: whether Bluetooth input may arrive (poll the mailbox often).
    pub fn hid_active(&self) -> bool { self.load(W_MAGIC) == MAGIC && self.load(W_HID_LINKS) != 0 }

    /// Cufflinks: the next packet from the controller; its length.
    pub fn rx_pop(&self, out: &mut [u8]) -> Option<usize> { self.pop(RX, W_RX_PROD, W_RX_CONS, out) }
    /// Cufflinks: queue a packet for the controller. False when the ring is full.
    pub fn tx_push(&self, packet: &[u8]) -> bool { self.push(TX, W_TX_PROD, W_TX_CONS, packet) }
    /// SexUSB: the next packet for the controller; its length.
    pub fn tx_pop(&self, out: &mut [u8]) -> Option<usize> { self.pop(TX, W_TX_PROD, W_TX_CONS, out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_and_rings() {
        let mut mem = vec![0u32; BYTES as usize / 4];
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert!(s.link().is_none());
        s.set_link(Some((0x8087, 0x0032)));
        assert_eq!(s.link(), Some(((0x8087, 0x0032), 1)));
        let mut out = [0u8; MAX_PACKET];
        for n in 0..SLOTS { assert!(s.rx_push_typed(EVENT, &[0x0e, 1, n as u8])); }
        assert!(!s.rx_push(&[EVENT, 0x0e, 0]));
        assert_eq!(s.rx_dropped(), 1);
        for n in 0..SLOTS { assert_eq!(s.rx_pop(&mut out), Some(4)); assert_eq!(out[..4], [EVENT, 0x0e, 1, n as u8]); }
        for n in 0..3 * SLOTS { assert!(s.tx_push(&[COMMAND, 0x03, 0x0c, n as u8])); assert_eq!(s.tx_pop(&mut out), Some(4)); assert_eq!(out[3], n as u8); }
        assert!(!s.tx_push(&[0; MAX_PACKET + 1]) && !s.tx_push(&[COMMAND]));
        assert!(s.tx_push(&[ACL; MAX_PACKET]));
        s.set_link(None);
        assert!(s.link().is_none());
        s.set_link(Some((0x8087, 0x0026)));
        assert_eq!(s.link().unwrap().1, 2);
    }
    #[test]
    fn hid_mailbox() {
        let mut mem = vec![0u32; BYTES as usize / 4];
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert!(s.hid_pop().is_none() && !s.hid_active());
        let k = HidInput::Keys { device: 2, modifiers: 0x02, keys: 0x0504 };
        let p = HidInput::Pointer { device: 3, buttons: 1, dx: -300, dy: 7, wheel: -1 };
        assert!(s.hid_push(k) && s.hid_push(p) && s.hid_push(HidInput::Gone { device: 2 }));
        assert_eq!((s.hid_pop(), s.hid_pop(), s.hid_pop(), s.hid_pop()), (Some(k), Some(p), Some(HidInput::Gone { device: 2 }), None));
        for n in 0..HID_SLOTS { assert!(s.hid_push(HidInput::Gone { device: n as u8 })); }
        assert!(!s.hid_push(k));
        for n in 0..HID_SLOTS { assert_eq!(s.hid_pop(), Some(HidInput::Gone { device: n as u8 })); }
        s.set_link(Some((1, 2)));
        s.set_hid_links(1);
        assert!(s.hid_active());
        // The rings live past the header's mailbox.
        assert!(HID_RING + HID_SLOTS as usize * HID_RECORD <= HEADER);
    }
}
