// SPDX-License-Identifier: GPL-3.0-only
//! USB sound cards (USB Audio Class 1 and 2): Bell's mix played through an
//! isochronous OUT endpoint.
//!
//! One card at a time. Bell writes 48 kHz stereo frames into the shared region of
//! `sex_pdx::usb_audio`; `pump`, called every loop turn, keeps up to `IN_FLIGHT`
//! packets queued, one service interval of frames each (48 a millisecond). When Bell
//! has nothing, a short tail of silence goes out and the stream rests until it has.

use sex_pdx::serial_record as log;
use sex_pdx::usb_audio::Shared;
use super::{Usb, Failure, Ring, trb, page, FULL, HIGH, LOW};
use crate::uac;

/// Ring positions (one link TRB closes the ring), and the packet buffer each one owns.
const TRBS: u32 = 64;
const SLOT_BYTES: u64 = 1024;
/// Packets queued with the controller at once.
const IN_FLIGHT: u32 = 24;
/// Silent packets sent after Bell stops before the stream rests.
const TAIL: u32 = 32;
/// Isoch TRB type and Start Isoch ASAP.
const ISOCH: u32 = 5;
const SIA: u32 = 1 << 31;
/// Completion codes an isochronous OUT endpoint reports without being broken.
const CC_RING_UNDERRUN: u32 = 14;
const CC_MISSED_SERVICE: u32 = 23;

#[derive(Clone, Copy)]
pub(super) struct Audio {
    pub(super) dev: usize,
    dci: u32,
    ring: Ring,
    /// Frames one packet carries.
    frames: u32,
    in_flight: u32,
    silent: u32,
    packets: u64,
    sound: u64,
    errors: u32,
}

impl Usb {
    /// Look through every configuration of device `i` for a USB Audio playback setting
    /// and drive it, if no card is running yet and Bell's region is there.
    pub(super) fn try_audio(&mut self, i: usize, configs: u8) -> Result<bool, Failure> {
        if self.audio.is_some() || self.audio_region == 0 { return Ok(false); }
        let buf = self.dev[i].mem.unwrap().buf;
        let mut found = None;
        for index in 0..configs.min(4) {
            if self.get(i, 0x80, 6, 0x0200 | index as u16, 0, buf.0, 9).is_err() { continue; }
            let total = unsafe { core::ptr::read_unaligned((buf.1 + 2) as *const u16) }.min(2048);
            let Ok(got) = self.get(i, 0x80, 6, 0x0200 | index as u16, 0, buf.0, total) else { continue };
            let cfg = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, (got as usize).min(2048)) };
            if let Some(p) = uac::playback(cfg) { found = Some(p); break; }
        }
        let Some(p) = found else { return Ok(false) };
        let slot = self.dev[i].slot;
        let speed = self.dev[i].speed;
        if speed == LOW { return Ok(false); }
        let high = speed != FULL;
        let us = uac::service_us(high, p.interval);
        let frames = (sex_pdx::usb_audio::RATE as u64 * us as u64 / 1_000_000) as u32;
        if frames == 0 || frames as u64 * 4 > (p.mps as u64).min(SLOT_BYTES) {
            log!("[sexusb.audio.fail] slot={} reason=packet mps={} frames={}", slot, p.mps, frames);
            return Ok(false);
        }
        if self.audio_buffers.0 == 0 {
            let pool = &mut self.x.pool;
            self.audio_buffers = page(pool, (TRBS as u64 * SLOT_BYTES).div_ceil(4096))?;
            self.audio_ring = page(pool, 1)?;
        }
        let dci = (p.ep & 0xf) as u32 * 2;
        let mut ring = Ring::empty();
        ring.init(self.audio_ring.0, self.audio_ring.1, TRBS);
        // Isochronous OUT (type 1): Interval from bInterval, average and max ESIT payload.
        let bytes = frames * 4;
        self.input_context(i, 0, 0);
        self.endpoint_context(i, dci, 1, p.mps, uac::xhci_interval(high, p.interval), ring.dequeue_pointer(), bytes, p.mps as u32);
        // Isochronous endpoints take no error count (CErr 0).
        let e = self.ctx(self.dev[i].mem.unwrap().in_ctx.1, 1 + dci as u64);
        unsafe { core::ptr::write_volatile(e.add(1), core::ptr::read_volatile(e.add(1)) & !(3 << 1)); }
        self.input_flags(i, 1 | (1 << dci));
        self.slot_context(i, dci);
        let mem = self.dev[i].mem.unwrap();
        self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (slot << 24))?;
        self.set(i, 0x00, 9, p.config as u16, 0)?;
        // The streaming interface's setting with the endpoint starts the card listening.
        self.set(i, 0x01, 0x0B, p.alt as u16, p.iface as u16)?;
        // 48 kHz: a card with one rate may refuse the request, which is fine.
        let (rt, value, index, data, len) = uac::set_rate(&p);
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf.1 as *mut u8, 4); }
        let rate_ok = self.get(i, rt, 0x01, value, index, buf.0, len).is_ok();
        self.audio = Some(Audio { dev: i, dci, ring, frames, in_flight: 0, silent: TAIL, packets: 0, sound: 0, errors: 0 });
        let d = self.dev[i];
        unsafe { Shared::at(self.audio_region as *mut u8) }.set_card(Some((d.vendor, d.product, if p.uac2 { 2 } else { 1 })));
        log!("[sexusb.audio] slot={} uac={} speed={} config={} iface={} alt={} ep={:#04x} mps={} interval_us={} frames_per_packet={} sync={} rate_set={} ok=1",
            slot, if p.uac2 { 2 } else { 1 }, if high { "high" } else { "full" }, p.config, p.iface, p.alt, p.ep, p.mps, us, frames, p.sync, rate_ok as u8);
        let _ = HIGH;
        Ok(true)
    }

    /// A transfer event on the card's endpoint. False: not the card's.
    pub(super) fn audio_event(&mut self, i: usize, e: crate::xhci::Event) -> bool {
        let Some(mut a) = self.audio.filter(|a| a.dev == i && a.dci == e.endpoint()) else { return false };
        let code = e.code();
        match code {
            super::CC_SUCCESS | super::CC_SHORT_PACKET | CC_MISSED_SERVICE => a.in_flight = a.in_flight.saturating_sub(1),
            // The controller ran out of packets: everything queued has gone.
            CC_RING_UNDERRUN => a.in_flight = 0,
            _ => {
                a.errors += 1;
                if a.errors <= 8 || a.errors.is_power_of_two() { log!("[sexusb.audio.error] dci={} code={} errors={}", a.dci, code, a.errors); }
                let slot = self.dev[i].slot;
                self.x.recover(slot, a.dci, &mut a.ring);
                a.in_flight = 0;
            }
        }
        self.audio = Some(a);
        true
    }

    /// Queue Bell's frames as packets, as far as the ring allows. True if any went.
    pub fn audio_pump(&mut self) -> bool {
        let Some(mut a) = self.audio else { return false };
        let shared = unsafe { Shared::at(self.audio_region as *mut u8) };
        let mut moved = false;
        let mut frames = [[0i16; 2]; (SLOT_BYTES / 4) as usize];
        while a.in_flight < IN_FLIGHT {
            let have = shared.written().wrapping_sub(shared.taken());
            if have == 0 && a.silent >= TAIL { break; }
            let out = &mut frames[..a.frames as usize];
            let n = shared.take(out);
            a.silent = if n == 0 { a.silent + 1 } else { 0 };
            let pos = a.ring.idx as u64;
            let (phys, va) = (self.audio_buffers.0 + pos * SLOT_BYTES, self.audio_buffers.1 + pos * SLOT_BYTES);
            unsafe { core::ptr::copy_nonoverlapping(out.as_ptr() as *const u8, va as *mut u8, out.len() * 4); }
            a.ring.push(phys as u32, (phys >> 32) as u32, a.frames * 4, (ISOCH << 10) | trb::IOC | SIA);
            a.in_flight += 1;
            a.packets += 1;
            a.sound += n as u64;
            if a.packets.is_power_of_two() && a.packets >= 64 {
                log!("[sexusb.audio.sent] packets={} frames={} underruns={}", a.packets, a.sound, shared.underruns());
            }
            moved = true;
        }
        let slot = self.dev[a.dev].slot;
        let dci = a.dci;
        self.audio = Some(a);
        if moved { self.x.doorbell(slot, dci); }
        moved
    }

    /// The card's device left.
    pub(super) fn audio_gone(&mut self, i: usize) {
        if let Some(a) = self.audio.filter(|a| a.dev == i) {
            self.audio = None;
            unsafe { Shared::at(self.audio_region as *mut u8) }.set_card(None);
            log!("[sexusb.audio.gone] slot={} packets={} frames={}", self.dev[i].slot, a.packets, a.sound);
        }
    }
}
