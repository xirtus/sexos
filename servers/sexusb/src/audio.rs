// SPDX-License-Identifier: GPL-3.0-only
//! USB sound cards (USB Audio Class 1 and 2): Bell's mix played through an
//! isochronous OUT endpoint.
//!
//! One card at a time. Bell writes 48 kHz stereo frames into the shared region of
//! `sex_pdx::usb_audio`; `pump`, called every loop turn, keeps up to `IN_FLIGHT`
//! packets queued, one service interval of frames each (48 a millisecond). When Bell
//! has nothing, a short tail of silence goes out and the stream rests until it has.
//!
//! An asynchronous card runs on its own clock and says how fast on a feedback
//! endpoint; its packets then carry that many frames (48 one packet, 49 the next),
//! so Bell's 48 kHz neither piles up nor runs dry on it.
//!
//! A microphone (a headset's, or a microphone on its own) is announced to Bell when
//! found but only runs while Bell says someone may listen: then its streaming setting
//! is chosen (a headset's "mic on" light comes on), packets come in on an isochronous
//! IN endpoint, and each becomes 48 kHz stereo 16-bit frames in the capture ring.

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
/// The feedback ring and its packet buffers share the data ring's page: data TRBs take
/// the first 1 KiB, feedback TRBs start at 2 KiB, their 8-byte buffers at 3 KiB.
const FB_TRBS: u32 = 16;
const FB_RING_AT: u64 = 2048;
const FB_BUF_AT: u64 = 3072;
const FB_SLOT: u64 = 8;
const FB_IN_FLIGHT: u32 = 4;
/// The microphone's ring sits between the data ring and the feedback ring (1 KiB at
/// 1 KiB); its packet buffers have their own pages, one per ring position.
const MIC_TRBS: u32 = 32;
const MIC_RING_AT: u64 = 1024;
const MIC_SLOT: u64 = 1024;
const MIC_IN_FLIGHT: u32 = 8;
/// Completion codes an isochronous IN endpoint reports without being broken, and the
/// ones a Stop Endpoint command leaves on the TRB it stopped at.
const CC_RING_OVERRUN: u32 = 21;
const CC_STOPPED: u32 = 26;
const CC_STOPPED_LENGTH: u32 = 27;

/// An asynchronous card's feedback endpoint, while it runs.
#[derive(Clone, Copy)]
struct Fb {
    dci: u32,
    ring: Ring,
    mps: u16,
    /// Bus units (1 ms frames or 125 µs microframes) per data packet.
    units: u32,
    nominal: u32,
    reports: u64,
    last: u32,
}

/// The microphone in use, and whether it runs (someone may listen).
#[derive(Clone, Copy)]
pub(super) struct Mic {
    pub(super) dev: usize,
    dci: u32,
    ring: Ring,
    c: uac::Capture,
    running: bool,
    heard: u64,
    packets: u64,
    errors: u32,
}

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
    high: bool,
    mps: u16,
    fb: Option<Fb>,
    pacer: uac::Pacer,
}

impl Usb {
    /// Look through every configuration of device `i` for a USB Audio playback setting
    /// and a microphone, and drive what is free: one card plays and one microphone
    /// listens at a time, and Bell's region must be there.
    pub(super) fn try_audio(&mut self, i: usize, configs: u8) -> Result<bool, Failure> {
        let (want_play, want_mic) = (self.audio.is_none(), self.mic.is_none());
        if !(want_play || want_mic) || self.audio_region == 0 { return Ok(false); }
        let buf = self.dev[i].mem.unwrap().buf;
        let mut found = None;
        for index in 0..configs.min(4) {
            if self.get(i, 0x80, 6, 0x0200 | index as u16, 0, buf.0, 9).is_err() { continue; }
            let total = unsafe { core::ptr::read_unaligned((buf.1 + 2) as *const u16) }.min(2048);
            let Ok(got) = self.get(i, 0x80, 6, 0x0200 | index as u16, 0, buf.0, total) else { continue };
            let cfg = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, (got as usize).min(2048)) };
            let (p, m) = (uac::playback(cfg).filter(|_| want_play), uac::capture(cfg).filter(|_| want_mic));
            // A configuration that plays wins; else the first with a microphone.
            if p.is_some() { found = Some((p, m)); break; }
            if m.is_some() && found.is_none() { found = Some((p, m)); }
        }
        let Some((p, m)) = found else { return Ok(false) };
        let slot = self.dev[i].slot;
        let speed = self.dev[i].speed;
        if speed == LOW { return Ok(false); }
        let high = speed != FULL;
        let us = p.map_or(1000, |p| uac::service_us(high, p.interval));
        let frames = (sex_pdx::usb_audio::RATE as u64 * us as u64 / 1_000_000) as u32;
        let p = p.filter(|p| {
            let ok = frames != 0 && frames as u64 * 4 <= (p.mps as u64).min(SLOT_BYTES);
            if !ok { log!("[sexusb.audio.fail] slot={} reason=packet mps={} frames={}", slot, p.mps, frames); }
            ok
        });
        let m = m.filter(|m| {
            let ok = (m.channels as u16 * m.bytes as u16) <= m.mps && m.mps as u64 <= MIC_SLOT;
            if !ok { log!("[sexusb.mic.fail] slot={} reason=packet mps={}", slot, m.mps); }
            ok
        });
        if p.is_none() && m.is_none() { return Ok(false); }
        if self.audio_buffers.0 == 0 {
            let pool = &mut self.x.pool;
            self.audio_buffers = page(pool, (TRBS as u64 * SLOT_BYTES).div_ceil(4096))?;
            self.audio_ring = page(pool, 1)?;
            self.mic_buffers = page(pool, (MIC_TRBS as u64 * MIC_SLOT).div_ceil(4096))?;
        }
        self.input_context(i, 0, 0);
        let (mut add, mut last) = (1u32, 1u32);
        let mut play = None;
        if let Some(p) = p {
            let dci = (p.ep & 0xf) as u32 * 2;
            let mut ring = Ring::empty();
            ring.init(self.audio_ring.0, self.audio_ring.1, TRBS);
            // Isochronous OUT (type 1): Interval from bInterval, average and max ESIT payload.
            self.endpoint_context(i, dci, 1, p.mps, uac::xhci_interval(high, p.interval), ring.dequeue_pointer(), frames * 4, p.mps as u32);
            (add, last) = (add | 1 << dci, last.max(dci));
            // An asynchronous card's feedback endpoint: isochronous IN (type 5).
            let fb = p.feedback.filter(|f| (3..=FB_SLOT as u16).contains(&f.mps)).map(|f| {
                let mut r = Ring::empty();
                r.init(self.audio_ring.0 + FB_RING_AT, self.audio_ring.1 + FB_RING_AT, FB_TRBS);
                let units = us / if high { 125 } else { 1000 };
                let fb = Fb { dci: (f.ep & 0xf) as u32 * 2 + 1, ring: r, mps: f.mps, units: units.max(1), nominal: uac::nominal_q16(high), reports: 0, last: 0 };
                self.endpoint_context(i, fb.dci, 5, fb.mps, uac::xhci_interval(high, f.interval), r.dequeue_pointer(), f.mps as u32, f.mps as u32);
                fb
            });
            if let Some(f) = fb { (add, last) = (add | 1 << f.dci, last.max(f.dci)); }
            play = Some((p, dci, ring, fb));
        }
        let mic = m.map(|m| {
            let dci = (m.ep & 0xf) as u32 * 2 + 1;
            let mut ring = Ring::empty();
            ring.init(self.audio_ring.0 + MIC_RING_AT, self.audio_ring.1 + MIC_RING_AT, MIC_TRBS);
            self.endpoint_context(i, dci, 5, m.mps, uac::xhci_interval(high, m.interval), ring.dequeue_pointer(), m.mps as u32, m.mps as u32);
            Mic { dev: i, dci, ring, c: m, running: false, heard: 0, packets: 0, errors: 0 }
        });
        if let Some(m) = mic { (add, last) = (add | 1 << m.dci, last.max(m.dci)); }
        // Isochronous endpoints take no error count (CErr 0).
        for d in 2..=last {
            if add & 1 << d == 0 { continue; }
            let e = self.ctx(self.dev[i].mem.unwrap().in_ctx.1, 1 + d as u64);
            unsafe { core::ptr::write_volatile(e.add(1), core::ptr::read_volatile(e.add(1)) & !(3 << 1)); }
        }
        self.input_flags(i, add);
        self.slot_context(i, last);
        let mem = self.dev[i].mem.unwrap();
        self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (slot << 24))?;
        let config = p.map_or_else(|| m.map_or(1, |m| m.config), |p| p.config);
        self.set(i, 0x00, 9, config as u16, 0)?;
        let d = self.dev[i];
        let shared = unsafe { Shared::at(self.audio_region as *mut u8) };
        if let Some(m) = mic {
            // The microphone's setting is chosen only while someone may listen.
            self.mic = Some(m);
            shared.set_mic(Some((d.vendor, d.product)));
            log!("[sexusb.mic] slot={} uac={} speed={} iface={} alt={} ep={:#04x} mps={} channels={} bytes={} ok=1",
                slot, if m.c.uac2 { 2 } else { 1 }, if high { "high" } else { "full" }, m.c.iface, m.c.alt, m.c.ep, m.c.mps, m.c.channels, m.c.bytes);
        }
        let Some((p, dci, ring, fb)) = play else { return Ok(true) };
        // The streaming interface's setting with the endpoint starts the card listening.
        self.set(i, 0x01, 0x0B, p.alt as u16, p.iface as u16)?;
        // 48 kHz: a card with one rate may refuse the request, which is fine.
        let (rt, value, index, data, len) = uac::set_rate(&p);
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf.1 as *mut u8, 4); }
        let rate_ok = self.get(i, rt, 0x01, value, index, buf.0, len).is_ok();
        let pacer = uac::Pacer::new(frames << 16);
        self.audio = Some(Audio { dev: i, dci, ring, frames, in_flight: 0, silent: TAIL, packets: 0, sound: 0, errors: 0, high, mps: p.mps, fb, pacer });
        if let Some(mut f) = fb {
            for _ in 0..FB_IN_FLIGHT { self.fb_queue(&mut f); }
            self.x.doorbell(slot, f.dci);
            if let Some(a) = self.audio.as_mut() { a.fb = Some(f); }
        }
        shared.set_card(Some((d.vendor, d.product, if p.uac2 { 2 } else { 1 })));
        log!("[sexusb.audio] slot={} uac={} speed={} config={} iface={} alt={} ep={:#04x} mps={} interval_us={} frames_per_packet={} sync={} rate_set={} ok=1",
            slot, if p.uac2 { 2 } else { 1 }, if high { "high" } else { "full" }, p.config, p.iface, p.alt, p.ep, p.mps, us, frames, p.sync, rate_ok as u8);
        if let Some(f) = fb { log!("[sexusb.audio.feedback] slot={} dci={} mps={} units_per_packet={}", slot, f.dci, f.mps, f.units); }
        let _ = HIGH;
        Ok(true)
    }

    /// A transfer event on the card's endpoint. False: not the card's.
    pub(super) fn audio_event(&mut self, i: usize, e: crate::xhci::Event) -> bool {
        if self.mic.is_some_and(|m| m.dev == i && m.dci == e.endpoint()) { self.mic_event(i, e); return true; }
        if let Some(mut a) = self.audio.filter(|a| a.dev == i && a.fb.is_some_and(|f| f.dci == e.endpoint())) {
            let mut f = a.fb.unwrap();
            match e.code() {
                super::CC_SUCCESS | super::CC_SHORT_PACKET => {
                    // Which buffer: the TRB's place in the feedback ring.
                    let got = (f.mps as u32).saturating_sub(e.residual()) as usize;
                    if let Some(pos) = f.ring.index_of(e.pointer()).map(|p| p as u64).filter(|_| got > 0) {
                        let raw = unsafe { core::slice::from_raw_parts((self.audio_ring.1 + FB_BUF_AT + pos * FB_SLOT) as *const u8, got.min(FB_SLOT as usize)) };
                        if let Some(v) = uac::feedback_q16(a.high, raw, f.nominal) {
                            let per_packet = v.saturating_mul(f.units);
                            a.pacer.set(per_packet);
                            f.reports += 1;
                            if f.reports.is_power_of_two() || per_packet.abs_diff(f.last) > 0x4000 {
                                log!("[sexusb.audio.rate] reports={} frames_per_packet_q16={:#x}", f.reports, per_packet);
                                f.last = per_packet;
                            }
                        }
                    }
                }
                CC_MISSED_SERVICE | CC_RING_UNDERRUN => {}
                code => {
                    a.errors += 1;
                    if a.errors <= 8 || a.errors.is_power_of_two() { log!("[sexusb.audio.error] dci={} code={} errors={}", f.dci, code, a.errors); }
                    let slot = self.dev[i].slot;
                    self.x.recover(slot, f.dci, &mut f.ring);
                    for _ in 0..FB_IN_FLIGHT - 1 { self.fb_queue(&mut f); }
                }
            }
            // One report taken, one more asked for.
            self.fb_queue(&mut f);
            self.x.doorbell(self.dev[i].slot, f.dci);
            a.fb = Some(f);
            self.audio = Some(a);
            return true;
        }
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
        let heard = self.mic_pump();
        let Some(mut a) = self.audio else { return heard };
        let shared = unsafe { Shared::at(self.audio_region as *mut u8) };
        let mut moved = false;
        let mut frames = [[0i16; 2]; (SLOT_BYTES / 4) as usize];
        while a.in_flight < IN_FLIGHT {
            let have = shared.written().wrapping_sub(shared.taken());
            if have == 0 && a.silent >= TAIL { break; }
            // At the card's own rate when it says what that is, within a frame of 48.
            let count = if a.fb.is_some() {
                a.pacer.next().clamp(a.frames.saturating_sub(1), a.frames + 1).min(a.mps as u32 / 4).min(SLOT_BYTES as u32 / 4)
            } else { a.frames };
            let out = &mut frames[..count as usize];
            let n = shared.take(out);
            a.silent = if n == 0 { a.silent + 1 } else { 0 };
            let pos = a.ring.idx as u64;
            let (phys, va) = (self.audio_buffers.0 + pos * SLOT_BYTES, self.audio_buffers.1 + pos * SLOT_BYTES);
            unsafe { core::ptr::copy_nonoverlapping(out.as_ptr() as *const u8, va as *mut u8, out.len() * 4); }
            a.ring.push(phys as u32, (phys >> 32) as u32, count * 4, (ISOCH << 10) | trb::IOC | SIA);
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

    /// Ask the feedback endpoint for one more report.
    fn fb_queue(&mut self, f: &mut Fb) {
        let pos = f.ring.idx as u64;
        let phys = self.audio_ring.0 + FB_BUF_AT + pos * FB_SLOT;
        f.ring.push(phys as u32, (phys >> 32) as u32, f.mps as u32, (ISOCH << 10) | trb::IOC | trb::ISP | SIA);
    }

    /// A microphone packet came in (or a stop finished).
    fn mic_event(&mut self, i: usize, e: crate::xhci::Event) {
        let Some(mut m) = self.mic else { return };
        let slot = self.dev[i].slot;
        match e.code() {
            CC_STOPPED | CC_STOPPED_LENGTH => { self.mic = Some(m); return; }
            super::CC_SUCCESS | super::CC_SHORT_PACKET if m.running => {
                let got = (m.c.mps as u32).saturating_sub(e.residual()) as usize;
                if let Some(pos) = m.ring.index_of(e.pointer()) {
                    let raw = unsafe { core::slice::from_raw_parts((self.mic_buffers.1 + pos as u64 * MIC_SLOT) as *const u8, got.min(MIC_SLOT as usize)) };
                    let mut frames = [[0i16; 2]; (MIC_SLOT / 2) as usize];
                    let n = uac::to_stereo(raw, m.c.channels, m.c.bytes, &mut frames);
                    unsafe { Shared::at(self.audio_region as *mut u8) }.hear(&frames[..n]);
                    if m.heard == 0 && n > 0 { log!("[sexusb.mic.first] slot={} frames={}", slot, n); }
                    m.heard += n as u64;
                    m.packets += 1;
                }
            }
            CC_MISSED_SERVICE | CC_RING_UNDERRUN | CC_RING_OVERRUN => {}
            _ if !m.running => { self.mic = Some(m); return; }
            code => {
                m.errors += 1;
                if m.errors <= 8 || m.errors.is_power_of_two() { log!("[sexusb.mic.error] dci={} code={} errors={}", m.dci, code, m.errors); }
                self.x.recover(slot, m.dci, &mut m.ring);
                for _ in 0..MIC_IN_FLIGHT - 1 { self.mic_queue(&mut m); }
            }
        }
        if m.running { self.mic_queue(&mut m); self.x.doorbell(slot, m.dci); }
        self.mic = Some(m);
    }

    /// Runs the microphone while Bell says someone may listen, and stops it after.
    /// True when it started or stopped.
    fn mic_pump(&mut self) -> bool {
        let Some(mut m) = self.mic else { return false };
        let listen = unsafe { Shared::at(self.audio_region as *mut u8) }.listen();
        if listen == m.running { return false; }
        let (i, slot) = (m.dev, self.dev[m.dev].slot);
        if listen {
            // Its streaming setting starts the microphone; then 48 kHz, which a
            // microphone with one rate may refuse.
            let ok = self.set(i, 0x01, 0x0B, m.c.alt as u16, m.c.iface as u16).is_ok();
            let buf = self.dev[i].mem.unwrap().buf;
            let (rt, value, index, data, len) = uac::capture_rate(&m.c);
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf.1 as *mut u8, 4); }
            let rate_ok = ok && self.get(i, rt, 0x01, value, index, buf.0, len).is_ok();
            // Start from wherever the ring stands now.
            self.x.recover(slot, m.dci, &mut m.ring);
            m.running = ok;
            if ok {
                for _ in 0..MIC_IN_FLIGHT { self.mic_queue(&mut m); }
                self.x.doorbell(slot, m.dci);
            }
            log!("[sexusb.mic.on] slot={} ok={} rate_set={} heard={}", slot, ok as u8, rate_ok as u8, m.heard);
        } else {
            m.running = false;
            let ok = self.set(i, 0x01, 0x0B, 0, m.c.iface as u16).is_ok();
            self.x.recover(slot, m.dci, &mut m.ring);
            log!("[sexusb.mic.off] slot={} ok={} packets={} heard={}", slot, ok as u8, m.packets, m.heard);
        }
        self.mic = Some(m);
        true
    }

    /// Ask the microphone for one more packet.
    fn mic_queue(&mut self, m: &mut Mic) {
        let pos = m.ring.idx as u64;
        let phys = self.mic_buffers.0 + pos * MIC_SLOT;
        m.ring.push(phys as u32, (phys >> 32) as u32, m.c.mps as u32, (ISOCH << 10) | trb::IOC | trb::ISP | SIA);
    }

    /// The card's device left.
    pub(super) fn audio_gone(&mut self, i: usize) {
        if let Some(m) = self.mic.filter(|m| m.dev == i) {
            self.mic = None;
            unsafe { Shared::at(self.audio_region as *mut u8) }.set_mic(None);
            log!("[sexusb.mic.gone] slot={} packets={} heard={}", self.dev[i].slot, m.packets, m.heard);
        }
        if let Some(a) = self.audio.filter(|a| a.dev == i) {
            self.audio = None;
            unsafe { Shared::at(self.audio_region as *mut u8) }.set_card(None);
            log!("[sexusb.audio.gone] slot={} packets={} frames={}", self.dev[i].slot, a.packets, a.sound);
        }
    }
}
