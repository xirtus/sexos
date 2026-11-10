// SPDX-License-Identifier: GPL-3.0-only
//! Intel High Definition Audio: the sound chip on nearly every PC since 2004.
//! BOOT leases Bell the controller's register BAR, its memory and bus-master
//! command bits, and a DMA pool; this file finds the analog outputs on every codec,
//! routes them to converters, and plays one cyclic 48 kHz stereo stream to the
//! outputs the person chose (`select`): by default the best output of the first
//! codec and every jack sharing its converter.
//!
//! The stream runs from start to finish over a ring of periods. Bell writes a
//! mixed period just ahead of the play position and zeroes each period once the
//! controller has read it, so the chip plays silence whenever nothing is queued.
use sex_pdx::memory_region as m;

// ── Controller registers (HDA 1.0a, section 3.3) ─────────────────────
const GCAP: u32 = 0x00;
const GCTL: u32 = 0x08;
const STATESTS: u32 = 0x0e;
const INTCTL: u32 = 0x20;
const CORBLBASE: u32 = 0x40;
const CORBUBASE: u32 = 0x44;
const CORBWP: u32 = 0x48;
const CORBRP: u32 = 0x4a;
const CORBCTL: u32 = 0x4c;
const CORBSIZE: u32 = 0x4e;
const RIRBLBASE: u32 = 0x50;
const RIRBUBASE: u32 = 0x54;
const RIRBWP: u32 = 0x58;
const RINTCNT: u32 = 0x5a;
const RIRBCTL: u32 = 0x5c;
const RIRBSTS: u32 = 0x5d;
const RIRBSIZE: u32 = 0x5e;
const SD_BASE: u32 = 0x80;
const SD_STRIDE: u32 = 0x20;
// Stream descriptor registers, relative to the descriptor.
const SD_CTL: u32 = 0x00;
const SD_STS: u32 = 0x03;
const SD_LPIB: u32 = 0x04;
const SD_CBL: u32 = 0x08;
const SD_LVI: u32 = 0x0c;
const SD_FMT: u32 = 0x12;
const SD_BDPL: u32 = 0x18;
const SD_BDPU: u32 = 0x1c;
const SD_CTL_RESET: u32 = 1;
const SD_CTL_RUN: u32 = 2;
/// The tag the output converter listens for; 0 means "no stream".
const STREAM_TAG: u32 = 1;
/// 48 kHz, 16 bits, 2 channels.
const FORMAT_48K_S16_STEREO: u16 = 0x0011;

// ── Codec verbs and parameters (HDA 1.0a, section 7) ─────────────────
const GET_PARAMETER: u32 = 0xf00;
const GET_CONNECTION: u32 = 0xf02;
const GET_CONFIG_DEFAULT: u32 = 0xf1c;
const SET_SELECT: u32 = 0x701;
const SET_POWER: u32 = 0x705;
const SET_STREAM: u32 = 0x706;
const SET_PIN_CONTROL: u32 = 0x707;
const SET_EAPD: u32 = 0x70c;
// Four-bit verbs take a 16-bit payload.
const SET_FORMAT: u32 = 0x2;
const SET_AMP: u32 = 0x3;
const P_VENDOR: u32 = 0x00;
const P_NODES: u32 = 0x04;
const P_FUNCTION: u32 = 0x05;
const P_WIDGET: u32 = 0x09;
const P_PIN: u32 = 0x0c;
const P_CONNECTIONS: u32 = 0x0e;
const P_IN_AMP: u32 = 0x0d;
const P_OUT_AMP: u32 = 0x12;
const WIDGET_OUTPUT: u32 = 0;
const WIDGET_MIXER: u32 = 2;
const WIDGET_SELECTOR: u32 = 3;
const WIDGET_PIN: u32 = 4;
const AUDIO_FUNCTION: u32 = 1;

// ── Ring of periods ───────────────────────────────────────────────────
pub const PERIOD_BYTES: usize = 8192;
/// 16 periods of 8 KiB is 683 ms around the ring; Bell keeps at most 12 queued.
pub const PERIODS: usize = 16;
const RING_BYTES: usize = PERIOD_BYTES * PERIODS;
// Layout inside the DMA pool: CORB 1 KiB, RIRB 2 KiB, BDL 256 B, then the ring.
const CORB_AT: usize = 0;
const RIRB_AT: usize = 1024;
const BDL_AT: usize = 4096;
const RING_AT: usize = 8192;
const POOL_NEEDS: usize = RING_AT + RING_BYTES;

/// How long one controller or codec step may take before it counts as broken.
const SPIN: u32 = 200_000;

pub struct Hda {
    mmio: usize,
    length: usize,
    dma_va: usize,
    dma_pa: u64,
    corb_wp: u16,
    rirb_rp: u16,
    codec: u32,
    sd: u32,
    /// Period the controller was reading at the last look.
    tail: usize,
    /// Next period Bell writes.
    head: usize,
    queued: [bool; PERIODS],
    pending: usize,
    pub vendor: u32,
    pub dac: u32,
    pub pins: u32,
    /// Every analog output found, on any codec, and which of them play.
    routes: [Option<Route>; MAX_OUTPUTS],
    pub outputs: usize,
    pub on: u32,
}

pub const MAX_OUTPUTS: usize = 8;

/// `device` is the jack's default-configuration device: 0 line out, 1 speaker,
/// 2 headphones.
#[derive(Clone, Copy)]
struct Route { codec: u32, pin: u32, dac: u32, path: [u32; 6], len: usize, rank: u32, device: u32 }

impl Hda {
    fn reg<T>(&self, offset: u32) -> *mut T {
        assert!(offset as usize + size_of::<T>() <= self.length, "HDA_MMIO_OUTSIDE_VIEW");
        (self.mmio + offset as usize) as *mut T
    }
    fn r8(&self, o: u32) -> u8 { unsafe { self.reg::<u8>(o).read_volatile() } }
    fn r16(&self, o: u32) -> u16 { unsafe { self.reg::<u16>(o).read_volatile() } }
    fn r32(&self, o: u32) -> u32 { unsafe { self.reg::<u32>(o).read_volatile() } }
    fn w8(&mut self, o: u32, v: u8) { unsafe { self.reg::<u8>(o).write_volatile(v) } }
    fn w16(&mut self, o: u32, v: u16) { unsafe { self.reg::<u16>(o).write_volatile(v) } }
    fn w32(&mut self, o: u32, v: u32) { unsafe { self.reg::<u32>(o).write_volatile(v) } }
    fn dma<T>(&self, at: usize) -> *mut T {
        assert!(at + size_of::<T>() <= POOL_NEEDS, "HDA_DMA_OUTSIDE_POOL");
        (self.dma_va + at) as *mut T
    }

    fn wait(&self, mut done: impl FnMut(&Self) -> bool) -> bool {
        for i in 0..SPIN {
            if done(self) { return true; }
            if i % 64 == 63 { sex_pdx::sys_yield(); }
            core::hint::spin_loop();
        }
        false
    }

    /// Takes the controller out of reset and reports which codecs answered.
    fn reset(&mut self) -> Result<u16, &'static str> {
        self.w8(CORBCTL, 0);
        self.w8(RIRBCTL, 0);
        let g = self.r32(GCTL);
        self.w32(GCTL, g & !1);
        if !self.wait(|h| h.r32(GCTL) & 1 == 0) { return Err("reset_enter"); }
        self.w32(GCTL, self.r32(GCTL) | 1);
        if !self.wait(|h| h.r32(GCTL) & 1 == 1) { return Err("reset_leave"); }
        // Codecs have 521 µs after reset to ask for an address.
        let start = sex_pdx::get_ticks();
        let _ = self.wait(|h| h.r16(STATESTS) != 0 && sex_pdx::get_ticks() > start + 2);
        let codecs = self.r16(STATESTS) & 0x7fff;
        self.w16(STATESTS, codecs);
        if codecs == 0 { return Err("no_codec"); }
        Ok(codecs)
    }

    fn rings(&mut self) -> Result<(), &'static str> {
        unsafe { core::ptr::write_bytes(self.dma::<u8>(0), 0, RING_AT) };
        let corb = self.dma_pa + CORB_AT as u64;
        let rirb = self.dma_pa + RIRB_AT as u64;
        self.w32(CORBLBASE, corb as u32);
        self.w32(CORBUBASE, (corb >> 32) as u32);
        // 256 entries when the controller offers it (bit 6 of the size capability).
        if self.r8(CORBSIZE) & 0x40 != 0 { self.w8(CORBSIZE, 2); }
        if self.r8(RIRBSIZE) & 0x40 != 0 { self.w8(RIRBSIZE, 2); }
        if self.r8(CORBSIZE) & 3 != 2 || self.r8(RIRBSIZE) & 3 != 2 { return Err("ring_size"); }
        self.w16(CORBWP, 0);
        self.w16(CORBRP, 0x8000);
        // Some controllers never show the reset bit; wait briefly, then clear it.
        let _ = self.wait(|h| h.r16(CORBRP) & 0x8000 != 0);
        self.w16(CORBRP, 0);
        if !self.wait(|h| h.r16(CORBRP) & 0x8000 == 0) { return Err("corb_reset"); }
        self.w32(RIRBLBASE, rirb as u32);
        self.w32(RIRBUBASE, (rirb >> 32) as u32);
        self.w16(RIRBWP, 0x8000);
        self.w16(RINTCNT, 1);
        self.w8(RIRBSTS, 0x05);
        self.w8(CORBCTL, 0x02);
        // DMA on, plus the response-count flag: some controllers (QEMU's among
        // them) hold further commands until software clears that flag.
        self.w8(RIRBCTL, 0x03);
        self.corb_wp = 0;
        self.rirb_rp = 0;
        Ok(())
    }

    /// One codec command and its answer, through the command and response rings.
    fn verb(&mut self, nid: u32, verb: u32, payload: u32) -> Option<u32> {
        let word = if verb <= 0xf {
            self.codec << 28 | nid << 20 | verb << 16 | (payload & 0xffff)
        } else {
            self.codec << 28 | nid << 20 | verb << 8 | (payload & 0xff)
        };
        let wp = (self.corb_wp + 1) & 0xff;
        unsafe { self.dma::<u32>(CORB_AT + wp as usize * 4).write_volatile(word) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.corb_wp = wp;
        self.w16(CORBWP, wp);
        loop {
            if !self.wait(|h| h.r16(RIRBWP) & 0xff != h.rirb_rp) { return None; }
            self.rirb_rp = (self.rirb_rp + 1) & 0xff;
            let at = RIRB_AT + self.rirb_rp as usize * 8;
            let (answer, extra) = unsafe { (self.dma::<u32>(at).read_volatile(), self.dma::<u32>(at + 4).read_volatile()) };
            // Clear the response count so the controller keeps sending commands.
            self.w8(RIRBSTS, 0x05);
            if extra & 0x10 == 0 { return Some(answer); } // skip unsolicited responses
        }
    }
    fn param(&mut self, nid: u32, p: u32) -> u32 { self.verb(nid, GET_PARAMETER, p).unwrap_or(0) }

    fn connections(&mut self, nid: u32, out: &mut [u32; 16]) -> usize {
        let info = self.param(nid, P_CONNECTIONS);
        let long = info & 0x80 != 0;
        let count = ((info & 0x7f) as usize).min(16);
        let per = if long { 2 } else { 4 };
        let mut n = 0;
        let mut i = 0;
        while i < count {
            let word = self.verb(nid, GET_CONNECTION, i as u32).unwrap_or(0);
            for k in 0..per {
                if i + k >= count { break; }
                let entry = if long { word >> (16 * k) & 0xffff } else { word >> (8 * k) & 0xff };
                let id = entry & if long { 0x7fff } else { 0x7f };
                // A range bit means "every node from the previous entry to this one".
                let range = entry & if long { 0x8000 } else { 0x80 } != 0;
                if range && n > 0 {
                    let mut from = out[n - 1] + 1;
                    while from <= id && n < 16 { out[n] = from; n += 1; from += 1; }
                } else if n < 16 { out[n] = id; n += 1; }
            }
            i += per;
        }
        n
    }

    /// A path from `nid` back to an output converter through mixers and selectors.
    fn find_dac(&mut self, nid: u32, depth: usize, route: &mut Route) -> bool {
        let caps = self.param(nid, P_WIDGET);
        let kind = caps >> 20 & 0xf;
        route.path[depth] = nid;
        route.len = depth + 1;
        if kind == WIDGET_OUTPUT && depth > 0 { route.dac = nid; return true; }
        if depth + 1 >= route.path.len() || !(depth == 0 || kind == WIDGET_MIXER || kind == WIDGET_SELECTOR) { return false; }
        let mut list = [0u32; 16];
        let n = self.connections(nid, &mut list);
        for &next in &list[..n] {
            if self.find_dac(next, depth + 1, route) {
                // Pins and selectors take one input; pick the one on the path.
                if kind != WIDGET_MIXER && n > 1 {
                    let index = list[..n].iter().position(|&x| x == next).unwrap_or(0);
                    let _ = self.verb(nid, SET_SELECT, index as u32);
                }
                return true;
            }
        }
        false
    }

    /// Analog output pins, best first: speaker, headphones, line out.
    fn routes(&mut self, afg: u32, out: &mut [Option<Route>; 8]) -> usize {
        let nodes = self.param(afg, P_NODES);
        let (first, count) = (nodes >> 16 & 0xff, nodes & 0xff);
        let mut n = 0;
        for nid in first..first + count {
            let caps = self.param(nid, P_WIDGET);
            if caps >> 20 & 0xf != WIDGET_PIN { continue; }
            let pin = self.param(nid, P_PIN);
            // Output capable; not HDMI or DisplayPort (those go to the monitor).
            if pin & 0x10 == 0 || pin & (1 << 7 | 1 << 24) != 0 { continue; }
            let config = self.verb(nid, GET_CONFIG_DEFAULT, 0).unwrap_or(0);
            if config >> 30 == 1 { continue; } // nothing is wired to this jack
            let device = config >> 20 & 0xf;
            let rank = match device { 1 => 0, 2 => 1, 0 => 2, 0xf | 5 => continue, _ => 3 };
            let mut route = Route { codec: self.codec, pin: nid, dac: 0, path: [0; 6], len: 0, rank, device };
            if self.find_dac(nid, 0, &mut route) && n < out.len() { out[n] = Some(route); n += 1; }
        }
        n
    }

    /// Powers, unmutes and enables every node on the route.
    fn open_route(&mut self, route: &Route) {
        for &nid in &route.path[..route.len] {
            let _ = self.verb(nid, SET_POWER, 0);
            let amp = self.param(nid, P_OUT_AMP);
            // Output amp, both channels, unmuted, at the 0 dB step.
            let _ = self.verb(nid, SET_AMP, 0xb000 | (amp & 0x7f));
            // Every input of a mixer on the path, unmuted at 0 dB.
            if self.param(nid, P_WIDGET) >> 20 & 0xf == WIDGET_MIXER {
                let input = self.param(nid, P_IN_AMP);
                for index in 0..4u32 { let _ = self.verb(nid, SET_AMP, 0x7000 | index << 8 | (input & 0x7f)); }
            }
        }
        let pin = route.pin;
        let caps = self.param(pin, P_PIN);
        let control = 0x40 | if self.verb(pin, GET_CONFIG_DEFAULT, 0).unwrap_or(0) >> 20 & 0xf == 2 { 0x80 } else { 0 };
        let _ = self.verb(pin, SET_PIN_CONTROL, control);
        if caps & 1 << 16 != 0 { let _ = self.verb(pin, SET_EAPD, 0x02); }
    }

    fn stream(&mut self) -> Result<(), &'static str> {
        let gcap = self.r16(GCAP) as u32;
        let (inputs, outputs) = (gcap >> 8 & 0xf, gcap >> 12 & 0xf);
        if outputs == 0 { return Err("no_output_stream"); }
        self.sd = SD_BASE + SD_STRIDE * inputs;
        if self.sd + SD_STRIDE > self.length as u32 { return Err("stream_outside_view"); }
        let sd = self.sd;
        self.w32(sd + SD_CTL, self.r32(sd + SD_CTL) & 0x00ff_fffc);
        self.w32(sd + SD_CTL, SD_CTL_RESET);
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RESET != 0) { return Err("stream_reset_enter"); }
        self.w32(sd + SD_CTL, 0);
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RESET == 0) { return Err("stream_reset_leave"); }
        unsafe { core::ptr::write_bytes(self.dma::<u8>(RING_AT), 0, RING_BYTES) };
        for i in 0..PERIODS {
            let at = BDL_AT + i * 16;
            unsafe {
                self.dma::<u64>(at).write_volatile(self.dma_pa + (RING_AT + i * PERIOD_BYTES) as u64);
                self.dma::<u32>(at + 8).write_volatile(PERIOD_BYTES as u32);
                self.dma::<u32>(at + 12).write_volatile(0);
            }
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let bdl = self.dma_pa + BDL_AT as u64;
        self.w32(sd + SD_BDPL, bdl as u32);
        self.w32(sd + SD_BDPU, (bdl >> 32) as u32);
        self.w32(sd + SD_CBL, RING_BYTES as u32);
        self.w16(sd + SD_LVI, (PERIODS - 1) as u16);
        self.w16(sd + SD_FMT, FORMAT_48K_S16_STEREO);
        self.w32(sd + SD_CTL, STREAM_TAG << 20);
        self.w8(sd + SD_STS, 0x1c);
        Ok(())
    }

    fn start(&mut self) -> Result<(), &'static str> {
        let sd = self.sd;
        self.w32(sd + SD_CTL, STREAM_TAG << 20 | SD_CTL_RUN);
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RUN != 0) { return Err("stream_run"); }
        Ok(())
    }

    /// Bring up the controller, a codec, an output route and the running stream.
    pub fn open(view: m::RegionView, dma: m::RegionView) -> Result<Hda, &'static str> {
        let dma_pa = dma.dma_address.ok_or("no_dma_address")?;
        if (dma.length as usize) < POOL_NEEDS { return Err("dma_small"); }
        let mut h = Hda { mmio: view.virtual_address as usize, length: view.length as usize,
            dma_va: dma.virtual_address as usize, dma_pa, corb_wp: 0, rirb_rp: 0, codec: 0, sd: 0,
            tail: 0, head: 1, queued: [false; PERIODS], pending: 0, vendor: 0, dac: 0, pins: 0,
            routes: [None; MAX_OUTPUTS], outputs: 0, on: 0 };
        if h.length < 0x180 { return Err("short_view"); }
        h.w32(INTCTL, 0);
        let codecs = h.reset()?;
        h.rings()?;
        // Every codec's analog outputs; HDMI-only codecs have none. The first codec
        // with one is where sound goes until the person picks another.
        let mut first_vendor = None;
        for cad in 0..15u32 {
            if codecs & 1 << cad == 0 { continue; }
            h.codec = cad;
            let vendor = h.param(0, P_VENDOR);
            let root = h.param(0, P_NODES);
            sex_pdx::serial_record!("[bell.hda.probe] address={} vendor={:#010x} nodes={:#x} corbrp={} rirbwp={} rirbsts={:#x}",
                cad, vendor, root, h.r16(CORBRP), h.r16(RIRBWP), h.r8(RIRBSTS));
            for afg in (root >> 16 & 0xff)..(root >> 16 & 0xff) + (root & 0xff) {
                if h.param(afg, P_FUNCTION) & 0xff != AUDIO_FUNCTION { continue; }
                let _ = h.verb(afg, SET_POWER, 0);
                let mut routes = [None; 8];
                let n = h.routes(afg, &mut routes);
                sex_pdx::serial_record!("[bell.hda.codec] address={} vendor={:#010x} function={} outputs={}", cad, vendor, afg, n);
                for r in routes[..n].iter().flatten() {
                    if h.outputs < MAX_OUTPUTS { h.routes[h.outputs] = Some(*r); h.outputs += 1; }
                }
                if n > 0 && first_vendor.is_none() { first_vendor = Some((vendor, cad)); }
            }
        }
        let (vendor, first) = first_vendor.ok_or("no_analog_output")?;
        h.vendor = vendor;
        let best = h.routes[..h.outputs].iter().flatten().filter(|r| r.codec == first)
            .min_by_key(|r| r.rank).copied().ok_or("no_route")?;
        // Every analog jack that reaches the same converter plays too, so sound
        // comes out of the speaker and whichever jack is plugged in.
        let mut on = 0;
        for (k, r) in h.routes[..h.outputs].iter().enumerate() {
            if r.is_some_and(|r| r.codec == best.codec && r.dac == best.dac) { on |= 1 << k; }
        }
        h.select(on);
        h.stream()?;
        h.start()?;
        h.tail = h.position();
        h.head = (h.tail + 1) % PERIODS;
        Ok(h)
    }

    /// Plays to the outputs in `mask` (bit k is output k) and silences the rest.
    /// False, and nothing changed, when the mask names no output.
    pub fn select(&mut self, mask: u32) -> bool {
        let mask = mask & ((1u32 << self.outputs) - 1);
        if mask == 0 { return false; }
        let routes = self.routes;
        // Off first, so a converter shared with a chosen output is set up again below.
        for (k, r) in routes[..self.outputs].iter().enumerate() {
            let Some(r) = r else { continue };
            if mask & 1 << k != 0 { continue; }
            self.codec = r.codec;
            let _ = self.verb(r.pin, SET_PIN_CONTROL, 0);
            let shared = routes[..self.outputs].iter().enumerate()
                .any(|(j, o)| mask & 1 << j != 0 && o.is_some_and(|o| o.codec == r.codec && o.dac == r.dac));
            if !shared { let _ = self.verb(r.dac, SET_STREAM, 0); }
        }
        self.pins = 0;
        for (k, r) in routes[..self.outputs].iter().enumerate() {
            let Some(r) = r else { continue };
            if mask & 1 << k == 0 { continue; }
            self.codec = r.codec;
            self.open_route(r);
            let _ = self.verb(r.dac, SET_FORMAT, FORMAT_48K_S16_STEREO as u32);
            let _ = self.verb(r.dac, SET_STREAM, STREAM_TAG << 4);
            self.dac = r.dac;
            self.pins += 1;
            sex_pdx::serial_record!("[bell.hda.route] output={} codec={} pin={} dac={} hops={} rank={}", k, r.codec, r.pin, r.dac, r.len, r.rank);
        }
        self.on = mask;
        true
    }

    /// Output `k`: its jack kind (0 line out, 1 speaker, 2 headphones, else other),
    /// codec address and pin.
    pub fn output(&self, k: usize) -> Option<(u32, u32, u32)> {
        self.routes.get(k).copied().flatten().map(|r| (r.device, r.codec, r.pin))
    }

    /// The period the controller is reading now.
    fn position(&self) -> usize { (self.r32(self.sd + SD_LPIB) as usize % RING_BYTES) / PERIOD_BYTES }

    /// Marks every period the controller has finished as played and zeroes it,
    /// so a ring that runs dry plays silence instead of old sound.
    fn advance(&mut self) {
        let now = self.position();
        while self.tail != now {
            let t = self.tail;
            unsafe { core::ptr::write_bytes(self.dma::<u8>(RING_AT + t * PERIOD_BYTES), 0, PERIOD_BYTES) };
            if self.queued[t] { self.queued[t] = false; self.pending -= 1; }
            self.tail = (t + 1) % PERIODS;
        }
        // Nothing queued: start the next write just ahead of the play position.
        if self.pending == 0 { self.head = (now + 1) % PERIODS; }
    }

    pub fn room(&mut self) -> bool { self.advance(); self.pending + 2 < PERIODS && self.head != self.tail }

    /// Queue one period; the token names it for `done`. None when the ring is full.
    pub fn submit(&mut self, period: &[u8; PERIOD_BYTES]) -> Option<u16> {
        self.advance();
        // Keep the period being played, and the one before it, out of reach.
        if self.pending + 2 >= PERIODS || self.head == self.tail { return None; }
        let slot = self.head;
        unsafe { core::ptr::copy_nonoverlapping(period.as_ptr(), self.dma::<u8>(RING_AT + slot * PERIOD_BYTES), PERIOD_BYTES) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.queued[slot] = true;
        self.pending += 1;
        self.head = (slot + 1) % PERIODS;
        Some(slot as u16)
    }

    /// True once the controller has played the period `token` names.
    pub fn done(&mut self, token: u16) -> bool {
        self.advance();
        !self.queued[token as usize % PERIODS]
    }
}
