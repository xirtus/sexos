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
//!
//! The microphone is the controller's first input stream: while someone may listen,
//! the chosen input pin (`crate::mic`) records through an input converter into a
//! second ring, and Bell copies out each period the controller has filled.
//!
//! Interrupts: when BOOT gave the lease an MSI or MSI-X vector, every period of both
//! rings ends with an interrupt-on-completion, so Bell sleeps until a period is
//! played or recorded instead of looking every tick. Bell arms the controller's
//! interrupt only while sound plays (`arm`) and acknowledges each wake (`ack`).
//! Without a vector it polls exactly as before.
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
/// Interrupt on completion: the stream raises its status bit at a flagged period end.
const SD_CTL_IOCE: u32 = 1 << 2;
/// Buffer completion, FIFO error and descriptor error; each is cleared by writing 1.
const SD_STS_BCIS: u8 = 1 << 2;
const SD_STS_ALL: u8 = 0x1c;
/// Global interrupt enable; bits 0..29 enable one stream descriptor each.
const INTCTL_GIE: u32 = 1 << 31;
/// The tag the output converter listens for; 0 means "no stream".
const STREAM_TAG: u32 = 1;
/// The tag the input converter sends with.
const MIC_TAG: u32 = 2;
/// 48 kHz, 16 bits, 2 channels.
const FORMAT_48K_S16_STEREO: u16 = 0x0011;

// ── Codec verbs and parameters (HDA 1.0a, section 7) ─────────────────
const GET_PARAMETER: u32 = 0xf00;
const GET_CONNECTION: u32 = 0xf02;
const GET_CONFIG_DEFAULT: u32 = 0xf1c;
const GET_PIN_SENSE: u32 = 0xf09;
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
const WIDGET_INPUT: u32 = 1;
const WIDGET_MIXER: u32 = 2;
const WIDGET_SELECTOR: u32 = 3;
const WIDGET_PIN: u32 = 4;
const AUDIO_FUNCTION: u32 = 1;

// ── Ring of periods ───────────────────────────────────────────────────
pub const PERIOD_BYTES: usize = 8192;
/// 16 periods of 8 KiB is 683 ms around the ring; Bell keeps at most 12 queued.
pub const PERIODS: usize = 16;
const RING_BYTES: usize = PERIOD_BYTES * PERIODS;
/// 8 periods of 8 KiB (340 ms) for the microphone.
pub const MIC_PERIODS: usize = 8;
const MIC_RING_BYTES: usize = PERIOD_BYTES * MIC_PERIODS;
// Layout inside the DMA pool: CORB 1 KiB, RIRB 2 KiB, BDL 256 B, the microphone's
// BDL 256 B, the ring, then the microphone's ring (200 KiB of the 256 KiB pool).
const CORB_AT: usize = 0;
const RIRB_AT: usize = 1024;
const BDL_AT: usize = 4096;
const MIC_BDL_AT: usize = BDL_AT + 256;
const RING_AT: usize = 8192;
const MIC_RING_AT: usize = RING_AT + RING_BYTES;
const POOL_NEEDS: usize = MIC_RING_AT + MIC_RING_BYTES;

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
    /// The outputs played by default, and whether Bell follows the headphone jack
    /// (until the person picks outputs themselves).
    pub home: u32,
    pub auto: bool,
    /// Every microphone or line in found, the input stream descriptor (0 when the
    /// controller has none), the input recording now, and the next period to copy.
    mics: [Option<Mic>; MAX_MICS],
    mic_count: usize,
    mic_sd: u32,
    mic_on: Option<usize>,
    mic_tail: usize,
    /// Widgets one path search may still visit (see `WALK_BUDGET`).
    walk: u32,
    /// Streams flag period ends for an interrupt (BOOT gave the lease a vector).
    irq: bool,
    /// The INTCTL value written last.
    armed: u32,
    /// Period ends seen by `ack`.
    pub completions: u64,
}

/// Widgets one pin's path search visits at most. Paths are at most 6 widgets with
/// up to 16 connections each, so a dense or looping mixer graph (or a hostile
/// codec) could otherwise cost 16^5 lookups; real codecs need a few dozen.
const WALK_BUDGET: u32 = 512;

pub const MAX_OUTPUTS: usize = 8;
const MAX_MICS: usize = 4;

/// An input pin and its path to an input converter: `path[0]` is the converter,
/// `path[len - 1]` the pin, and `pick[i]` which connection of `path[i]` leads on.
#[derive(Clone, Copy)]
struct Mic { codec: u32, afg: u32, pin: u32, path: [u32; 6], pick: [u32; 6], len: usize, input: crate::mic::Input }

/// `device` is the jack's default-configuration device: 0 line out, 1 speaker,
/// 2 headphones.
#[derive(Clone, Copy)]
struct Route { codec: u32, afg: u32, pin: u32, dac: u32, path: [u32; 6], len: usize, rank: u32, device: u32, sense: bool,
    /// A monitor over HDMI or DisplayPort rather than an analog jack.
    display: Option<crate::hdmi::Display> }

impl Hda {
    fn reg<T>(&self, offset: u32) -> *mut T {
        assert!(offset as usize + size_of::<T>() <= self.length, "HDA_MMIO_OUTSIDE_VIEW");
        (self.mmio + offset as usize) as *mut T
    }
    #[cfg(not(device_model))]
    fn r8(&self, o: u32) -> u8 { unsafe { self.reg::<u8>(o).read_volatile() } }
    #[cfg(not(device_model))]
    fn r16(&self, o: u32) -> u16 { unsafe { self.reg::<u16>(o).read_volatile() } }
    #[cfg(not(device_model))]
    fn r32(&self, o: u32) -> u32 { unsafe { self.reg::<u32>(o).read_volatile() } }
    #[cfg(not(device_model))]
    fn w8(&mut self, o: u32, v: u8) { unsafe { self.reg::<u8>(o).write_volatile(v) } }
    #[cfg(not(device_model))]
    fn w16(&mut self, o: u32, v: u16) { unsafe { self.reg::<u16>(o).write_volatile(v) } }
    #[cfg(not(device_model))]
    fn w32(&mut self, o: u32, v: u32) { unsafe { self.reg::<u32>(o).write_volatile(v) } }
    // Host fuzzing (fuzz/drivers, drv_hda): the registers are a device model.
    #[cfg(device_model)]
    fn r8(&self, o: u32) -> u8 { let _ = self.reg::<u8>(o); crate::device_model::read(o, 1) as u8 }
    #[cfg(device_model)]
    fn r16(&self, o: u32) -> u16 { let _ = self.reg::<u16>(o); crate::device_model::read(o, 2) as u16 }
    #[cfg(device_model)]
    fn r32(&self, o: u32) -> u32 { let _ = self.reg::<u32>(o); crate::device_model::read(o, 4) }
    #[cfg(device_model)]
    fn w8(&mut self, o: u32, v: u8) { let _ = self.reg::<u8>(o); crate::device_model::write(o, 1, v as u32) }
    #[cfg(device_model)]
    fn w16(&mut self, o: u32, v: u16) { let _ = self.reg::<u16>(o); crate::device_model::write(o, 2, v as u32) }
    #[cfg(device_model)]
    fn w32(&mut self, o: u32, v: u32) { let _ = self.reg::<u32>(o); crate::device_model::write(o, 4, v) }
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
        // A codec may queue unsolicited responses (jack events) ahead of the
        // answer, but not forever: past a full ring of them the answer is lost.
        for _ in 0..256 {
            if !self.wait(|h| h.r16(RIRBWP) & 0xff != h.rirb_rp) { return None; }
            self.rirb_rp = (self.rirb_rp + 1) & 0xff;
            let at = RIRB_AT + self.rirb_rp as usize * 8;
            let (answer, extra) = unsafe { (self.dma::<u32>(at).read_volatile(), self.dma::<u32>(at + 4).read_volatile()) };
            // Clear the response count so the controller keeps sending commands.
            self.w8(RIRBSTS, 0x05);
            if extra & 0x10 == 0 { return Some(answer); } // skip unsolicited responses
        }
        None
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
        if self.walk == 0 { return false; }
        self.walk -= 1;
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
            if pin & 0x10 == 0 { continue; }
            let config = self.verb(nid, GET_CONFIG_DEFAULT, 0).unwrap_or(0);
            if config >> 30 == 1 { continue; } // nothing is wired to this jack
            let device = config >> 20 & 0xf;
            // HDMI and DisplayPort pins send sound to a monitor; they come last.
            let display = crate::hdmi::display_pin(pin, config);
            let rank = if display.is_some() { 4 } else { match device { 1 => 0, 2 => 1, 0 => 2, 0xf | 5 => continue, _ => 3 } };
            let mut route = Route { codec: self.codec, afg, pin: nid, dac: 0, path: [0; 6], len: 0, rank, device, sense: pin & 1 << 2 != 0, display };
            self.walk = WALK_BUDGET;
            if self.find_dac(nid, 0, &mut route) && n < out.len() { out[n] = Some(route); n += 1; }
        }
        n
    }

    /// A path from the input converter or mixer `nid` up to the pin `pin`.
    fn find_pin(&mut self, nid: u32, pin: u32, depth: usize, mic: &mut Mic) -> bool {
        if self.walk == 0 { return false; }
        self.walk -= 1;
        mic.path[depth] = nid;
        mic.len = depth + 1;
        if nid == pin && depth > 0 { return true; }
        let kind = self.param(nid, P_WIDGET) >> 20 & 0xf;
        if depth + 1 >= mic.path.len() || !(depth == 0 || kind == WIDGET_MIXER || kind == WIDGET_SELECTOR) { return false; }
        let mut list = [0u32; 16];
        let n = self.connections(nid, &mut list);
        for (index, &next) in list[..n].iter().enumerate() {
            if self.find_pin(next, pin, depth + 1, mic) { mic.pick[depth] = index as u32; return true; }
        }
        false
    }

    /// Input pins that can record, each with a path to an input converter.
    fn mic_routes(&mut self, afg: u32) {
        let nodes = self.param(afg, P_NODES);
        let (first, count) = (nodes >> 16 & 0xff, nodes & 0xff);
        let mut adcs = [0u32; 8];
        let mut n_adc = 0;
        for nid in first..first + count {
            let caps = self.param(nid, P_WIDGET);
            // An input converter that takes digital audio is S/PDIF in, not a microphone.
            if caps >> 20 & 0xf == WIDGET_INPUT && caps & 1 << 9 == 0 && n_adc < adcs.len() { adcs[n_adc] = nid; n_adc += 1; }
        }
        for pin in first..first + count {
            if self.mic_count >= MAX_MICS { return; }
            if self.param(pin, P_WIDGET) >> 20 & 0xf != WIDGET_PIN { continue; }
            let caps = self.param(pin, P_PIN);
            let config = self.verb(pin, GET_CONFIG_DEFAULT, 0).unwrap_or(0);
            let Some(input) = crate::mic::input(config, caps) else { continue };
            for &adc in &adcs[..n_adc] {
                let mut mic = Mic { codec: self.codec, afg, pin, path: [0; 6], pick: [0; 6], len: 0, input };
                self.walk = WALK_BUDGET;
                if self.find_pin(adc, pin, 0, &mut mic) {
                    sex_pdx::serial_record!("[bell.hda.input] codec={} pin={} adc={} device={:#x} rank={} sense={} hops={}",
                        self.codec, pin, adc, input.device, input.rank, input.sense as u8, mic.len);
                    self.mics[self.mic_count] = Some(mic);
                    self.mic_count += 1;
                    break;
                }
            }
        }
    }

    /// A widget's amplifier capabilities (`P_IN_AMP` or `P_OUT_AMP`): its own when it
    /// sets Amp Param Override, otherwise the audio function group's defaults.
    fn amp_caps(&mut self, afg: u32, nid: u32, which: u32) -> u32 {
        if self.param(nid, P_WIDGET) & 1 << 3 != 0 { self.param(nid, which) } else { self.param(afg, which) }
    }

    /// Powers, unmutes and enables every node on the route.
    fn open_route(&mut self, route: &Route) {
        for &nid in &route.path[..route.len] {
            let _ = self.verb(nid, SET_POWER, 0);
            let amp = self.amp_caps(route.afg, nid, P_OUT_AMP);
            // Output amp, both channels, unmuted, at the 0 dB step.
            let _ = self.verb(nid, SET_AMP, 0xb000 | (amp & 0x7f));
            // Every input of a mixer on the path, unmuted at 0 dB.
            if self.param(nid, P_WIDGET) >> 20 & 0xf == WIDGET_MIXER {
                let input = self.amp_caps(route.afg, nid, P_IN_AMP);
                for index in 0..4u32 { let _ = self.verb(nid, SET_AMP, 0x7000 | index << 8 | (input & 0x7f)); }
            }
        }
        let pin = route.pin;
        let caps = self.param(pin, P_PIN);
        let control = 0x40 | if self.verb(pin, GET_CONFIG_DEFAULT, 0).unwrap_or(0) >> 20 & 0xf == 2 { 0x80 } else { 0 };
        let _ = self.verb(pin, SET_PIN_CONTROL, control);
        if caps & 1 << 16 != 0 { let _ = self.verb(pin, SET_EAPD, 0x02); }
        if route.display.is_some() { self.open_display(route); }
    }

    /// The digital half of a monitor route: two channels on the converter, digital
    /// output on, the monitor's ELD read (for its name and formats) and the audio
    /// InfoFrame loaded and sent.
    fn open_display(&mut self, route: &Route) {
        use crate::hdmi as d;
        let (pin, cvt) = (route.pin, route.dac);
        let _ = self.verb(cvt, d::SET_CVT_CHAN_COUNT, 1);
        let _ = self.verb(cvt, d::SET_DIGI_CONVERT_1, d::DIGITAL_ENABLE);
        let sense = self.verb(pin, GET_PIN_SENSE, 0).unwrap_or(0);
        let mut channels = 2u8;
        if sense & d::SENSE_PRESENT != 0 && sense & d::SENSE_ELD_VALID != 0 {
            let size = (self.verb(pin, d::GET_HDMI_DIP_SIZE, 0x08).unwrap_or(0) & 0xff) as usize;
            let mut eld = [0u8; 256];
            let size = size.min(eld.len());
            for (i, b) in eld[..size].iter_mut().enumerate() {
                let v = self.verb(pin, d::GET_HDMI_ELDD, i as u32).unwrap_or(0);
                if v & 1 << 31 == 0 { break; }
                *b = v as u8;
            }
            match d::parse_eld(&eld[..size]) {
                Some(e) => {
                    channels = 2.min(e.pcm_channels);
                    sex_pdx::serial_record!("[bell.hdmi.monitor] codec={} pin={} name={} channels={} rates={:#x} dp={} plays_48k={}",
                        route.codec, pin, core::str::from_utf8(e.name()).unwrap_or("?"), e.pcm_channels, e.pcm_rates, e.displayport as u8, e.plays_48k() as u8);
                }
                None => sex_pdx::serial_record!("[bell.hdmi.monitor] codec={} pin={} eld=invalid size={}", route.codec, pin, size),
            }
        } else {
            sex_pdx::serial_record!("[bell.hdmi.monitor] codec={} pin={} present={} eld_valid=0", route.codec, pin, (sense & d::SENSE_PRESENT != 0) as u8);
        }
        let frame = d::audio_infoframe(channels);
        let _ = self.verb(pin, d::SET_HDMI_DIP_INDEX, 0);
        for b in frame { let _ = self.verb(pin, d::SET_HDMI_DIP_DATA, b as u32); }
        // Send it every frame ("best effort").
        let _ = self.verb(pin, d::SET_HDMI_DIP_XMIT, 0xc0);
    }

    fn stream(&mut self) -> Result<(), &'static str> {
        let gcap = self.r16(GCAP) as u32;
        let (inputs, outputs) = (gcap >> 8 & 0xf, gcap >> 12 & 0xf);
        if outputs == 0 { return Err("no_output_stream"); }
        self.sd = SD_BASE + SD_STRIDE * inputs;
        if self.sd + SD_STRIDE > self.length as u32 { return Err("stream_outside_view"); }
        let sd = self.sd;
        self.setup_sd(sd, BDL_AT, RING_AT, PERIODS, STREAM_TAG)
    }

    fn start(&mut self) -> Result<(), &'static str> {
        let sd = self.sd;
        self.w32(sd + SD_CTL, STREAM_TAG << 20 | SD_CTL_RUN | self.ioce());
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RUN != 0) { return Err("stream_run"); }
        Ok(())
    }

    /// Bring up the controller, a codec, an output route and the running stream.
    /// `irq`: the lease has a vector, so periods end with an interrupt.
    pub fn open(view: m::RegionView, dma: m::RegionView, irq: bool) -> Result<Hda, &'static str> {
        let dma_pa = dma.dma_address.ok_or("no_dma_address")?;
        if (dma.length as usize) < POOL_NEEDS { return Err("dma_small"); }
        let mut h = Hda { mmio: view.virtual_address as usize, length: view.length as usize,
            dma_va: dma.virtual_address as usize, dma_pa, corb_wp: 0, rirb_rp: 0, codec: 0, sd: 0,
            tail: 0, head: 1, queued: [false; PERIODS], pending: 0, vendor: 0, dac: 0, pins: 0,
            routes: [None; MAX_OUTPUTS], outputs: 0, on: 0, home: 0, auto: true,
            mics: [None; MAX_MICS], mic_count: 0, mic_sd: 0, mic_on: None, mic_tail: 0, walk: 0,
            irq, armed: 0, completions: 0 };
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
            // Intel display codecs show their monitor pins only once asked to.
            if crate::hdmi::is_intel_display_codec(vendor) {
                let v = h.verb(crate::hdmi::INTEL_VENDOR_NODE, crate::hdmi::INTEL_GET_VENDOR, 0).unwrap_or(0);
                if v & crate::hdmi::INTEL_ALL_PINS == 0 {
                    let _ = h.verb(crate::hdmi::INTEL_VENDOR_NODE, crate::hdmi::INTEL_SET_VENDOR, v | crate::hdmi::INTEL_ALL_PINS);
                }
            }
            let root = h.param(0, P_NODES);
            sex_pdx::serial_record!("[bell.hda.probe] address={} vendor={:#010x} nodes={:#x} corbrp={} rirbwp={} rirbsts={:#x}",
                cad, vendor, root, h.r16(CORBRP), h.r16(RIRBWP), h.r8(RIRBSTS));
            for afg in (root >> 16 & 0xff)..(root >> 16 & 0xff) + (root & 0xff) {
                if h.param(afg, P_FUNCTION) & 0xff != AUDIO_FUNCTION { continue; }
                let _ = h.verb(afg, SET_POWER, 0);
                let mut routes = [None; 8];
                let n = h.routes(afg, &mut routes);
                h.mic_routes(afg);
                sex_pdx::serial_record!("[bell.hda.codec] address={} vendor={:#010x} function={} outputs={}", cad, vendor, afg, n);
                for r in routes[..n].iter().flatten() {
                    if h.outputs < MAX_OUTPUTS { h.routes[h.outputs] = Some(*r); h.outputs += 1; }
                    if let Some(d) = r.display {
                        sex_pdx::serial_record!("[bell.hda.display] codec={} pin={} converter={} kind={:?}", cad, r.pin, r.dac, d);
                    }
                }
                // Sound goes to the first codec with an analog output; a monitor plays
                // by default only when there is no analog output at all.
                if routes[..n].iter().flatten().any(|r| r.display.is_none()) && first_vendor.is_none() { first_vendor = Some((vendor, cad)); }
            }
        }
        let (vendor, first) = match first_vendor {
            Some(f) => f,
            None => h.routes[..h.outputs].iter().flatten().next().map(|r| (0, r.codec)).ok_or("no_output")?,
        };
        h.vendor = vendor;
        let best = h.routes[..h.outputs].iter().flatten().filter(|r| r.codec == first)
            .min_by_key(|r| r.rank).copied().ok_or("no_route")?;
        // Every analog jack that reaches the same converter plays too, so sound
        // comes out of the speaker and whichever jack is plugged in.
        let mut on = 0;
        for (k, r) in h.routes[..h.outputs].iter().enumerate() {
            if r.is_some_and(|r| r.codec == best.codec && r.dac == best.dac) { on |= 1 << k; }
        }
        h.home = on;
        h.select(on);
        h.follow_jack();
        h.stream()?;
        h.start()?;
        // The microphone needs an input stream descriptor; the first one is it.
        if h.r16(GCAP) >> 8 & 0xf != 0 && h.mic_count > 0 { h.mic_sd = SD_BASE; }
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

    /// While Bell follows the jack: speakers in the default set go quiet while
    /// headphones in it are plugged in, and play again when they come out.
    pub fn follow_jack(&mut self) {
        if !self.auto { return; }
        let routes = self.routes;
        let mut plugged = false;
        let mut kinds = [0u32; MAX_OUTPUTS];
        for (k, r) in routes[..self.outputs].iter().enumerate() {
            let Some(r) = r else { continue };
            kinds[k] = r.device;
            if r.device == crate::jack::HEADPHONES && r.sense && self.home & 1 << k != 0 {
                self.codec = r.codec;
                plugged |= self.verb(r.pin, GET_PIN_SENSE, 0).is_some_and(|v| v & 1 << 31 != 0);
            }
        }
        let want = crate::jack::wanted(self.home, &kinds[..self.outputs], plugged);
        if want != self.on {
            sex_pdx::serial_record!("[bell.hda.jack] headphones={} on={:#x}", plugged as u8, want);
            self.select(want);
        }
    }

    /// Output `k`: its jack kind (0 line out, 1 speaker, 2 headphones, else other),
    /// codec address and pin.
    pub fn output(&self, k: usize) -> Option<(u32, u32, u32)> {
        self.routes.get(k).copied().flatten().map(|r| (r.device, r.codec, r.pin))
    }
    /// Whether output `k` is a monitor, and over which.
    pub fn display(&self, k: usize) -> Option<crate::hdmi::Display> {
        self.routes.get(k).copied().flatten().and_then(|r| r.display)
    }

    fn ioce(&self) -> u32 { if self.irq { SD_CTL_IOCE } else { 0 } }

    /// True while periods end with an interrupt.
    pub fn interrupts(&self) -> bool { self.irq }

    /// The vector could not be unmasked after all: stay quiet and poll.
    pub fn poll_instead(&mut self) {
        self.arm(false);
        self.irq = false;
    }

    /// Lets the playback stream (and the microphone's, when there is one) interrupt,
    /// or stops them: Bell arms only while sound plays, so an idle Bell is never woken.
    pub fn arm(&mut self, on: bool) {
        if !self.irq { return; }
        let mut want = 0;
        if on {
            want = INTCTL_GIE | 1 << ((self.sd - SD_BASE) / SD_STRIDE);
            if self.mic_sd != 0 { want |= 1 << ((self.mic_sd - SD_BASE) / SD_STRIDE); }
        }
        if want != self.armed { self.w32(INTCTL, want); self.armed = want; }
    }

    /// Clears the streams' completion bits, so the next period end interrupts again
    /// (a message-signalled interrupt fires on the edge). Returns how many were set.
    pub fn ack(&mut self) -> u32 {
        if !self.irq { return 0; }
        let mut n = 0;
        for sd in [self.sd, self.mic_sd] {
            if sd == 0 { continue; }
            let sts = self.r8(sd + SD_STS);
            if sts & SD_STS_ALL != 0 { self.w8(sd + SD_STS, sts & SD_STS_ALL); }
            n += (sts & SD_STS_BCIS != 0) as u32;
        }
        self.completions += n as u64;
        n
    }

    /// True when there is an input to record from and a stream to record with.
    pub fn has_mic(&self) -> bool { self.mic_sd != 0 }

    /// How many inputs can record (-1 without an input stream), for the boot log.
    pub fn mic_inputs(&self) -> i64 { if self.mic_sd == 0 { -1 } else { self.mic_count as i64 } }

    /// Resets a stream descriptor and points it at a ring of `periods` periods.
    fn setup_sd(&mut self, sd: u32, bdl_at: usize, ring_at: usize, periods: usize, tag: u32) -> Result<(), &'static str> {
        self.w32(sd + SD_CTL, self.r32(sd + SD_CTL) & 0x00ff_fffc);
        self.w32(sd + SD_CTL, SD_CTL_RESET);
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RESET != 0) { return Err("stream_reset_enter"); }
        self.w32(sd + SD_CTL, 0);
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RESET == 0) { return Err("stream_reset_leave"); }
        unsafe { core::ptr::write_bytes(self.dma::<u8>(ring_at), 0, PERIOD_BYTES * periods) };
        for i in 0..periods {
            let at = bdl_at + i * 16;
            unsafe {
                self.dma::<u64>(at).write_volatile(self.dma_pa + (ring_at + i * PERIOD_BYTES) as u64);
                self.dma::<u32>(at + 8).write_volatile(PERIOD_BYTES as u32);
                // Bit 0: interrupt on completion of this period.
                self.dma::<u32>(at + 12).write_volatile(self.irq as u32);
            }
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let bdl = self.dma_pa + bdl_at as u64;
        self.w32(sd + SD_BDPL, bdl as u32);
        self.w32(sd + SD_BDPU, (bdl >> 32) as u32);
        self.w32(sd + SD_CBL, (PERIOD_BYTES * periods) as u32);
        self.w16(sd + SD_LVI, (periods - 1) as u16);
        self.w16(sd + SD_FMT, FORMAT_48K_S16_STEREO);
        self.w32(sd + SD_CTL, tag << 20 | self.ioce());
        self.w8(sd + SD_STS, SD_STS_ALL);
        Ok(())
    }

    /// Starts recording: a plugged-in jack microphone, else the best input, routed
    /// to its converter, unmuted at 0 dB, with microphone bias; then the stream runs.
    pub fn mic_start(&mut self) -> bool {
        if self.mic_sd == 0 { return false; }
        if self.mic_on.is_some() { return true; }
        let mut inputs = [crate::mic::Input { device: 0, internal: false, sense: false, rank: 0 }; MAX_MICS];
        let mut plugged = [false; MAX_MICS];
        let mics = self.mics;
        for (k, m) in mics[..self.mic_count].iter().enumerate() {
            let Some(m) = m else { continue };
            inputs[k] = m.input;
            if m.input.sense {
                self.codec = m.codec;
                plugged[k] = self.verb(m.pin, GET_PIN_SENSE, 0).is_some_and(|v| v & 1 << 31 != 0);
            }
        }
        let Some(k) = crate::mic::choose(&inputs[..self.mic_count], &plugged[..self.mic_count]) else { return false };
        let Some(m) = mics[k] else { return false };
        self.codec = m.codec;
        for (i, &nid) in m.path[..m.len].iter().enumerate() {
            let _ = self.verb(nid, SET_POWER, 0);
            let widget = self.param(nid, P_WIDGET);
            let kind = widget >> 20 & 0xf;
            let in_amp = self.amp_caps(m.afg, nid, P_IN_AMP);
            let out_amp = self.amp_caps(m.afg, nid, P_OUT_AMP);
            if nid == m.pin {
                // The pin's own input amplifier (a microphone boost on many codecs).
                let _ = self.verb(nid, SET_AMP, 0x7000 | (in_amp & 0x7f));
            } else if kind == WIDGET_MIXER {
                // Only the input on the path: other mixer inputs would bring playback in.
                let mut list = [0u32; 16];
                let n = self.connections(nid, &mut list) as u32;
                for index in 0..n.min(16) {
                    let mute = if index == m.pick[i] { in_amp & 0x7f } else { 0x80 };
                    let _ = self.verb(nid, SET_AMP, 0x7000 | index << 8 | mute);
                }
                let _ = self.verb(nid, SET_AMP, 0xb000 | (out_amp & 0x7f));
            } else {
                // Converters and selectors take one input: pick it and unmute it, and
                // a selector's own output amplifier too, when it has one.
                let _ = self.verb(nid, SET_SELECT, m.pick[i]);
                let _ = self.verb(nid, SET_AMP, 0x7000 | m.pick[i] << 8 | (in_amp & 0x7f));
                if kind == WIDGET_SELECTOR && widget & 1 << 2 != 0 { let _ = self.verb(nid, SET_AMP, 0xb000 | (out_amp & 0x7f)); }
            }
        }
        let caps = self.param(m.pin, P_PIN);
        let control = crate::mic::pin_control(m.input.device, caps);
        let _ = self.verb(m.pin, SET_PIN_CONTROL, control);
        let adc = m.path[0];
        let _ = self.verb(adc, SET_FORMAT, FORMAT_48K_S16_STEREO as u32);
        let _ = self.verb(adc, SET_STREAM, MIC_TAG << 4);
        let sd = self.mic_sd;
        if self.setup_sd(sd, MIC_BDL_AT, MIC_RING_AT, MIC_PERIODS, MIC_TAG).is_err() { return false; }
        self.w32(sd + SD_CTL, MIC_TAG << 20 | SD_CTL_RUN | self.ioce());
        if !self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RUN != 0) { return false; }
        self.mic_tail = self.mic_position();
        self.mic_on = Some(k);
        sex_pdx::serial_record!("[bell.hda.mic] codec={} pin={} adc={} device={:#x} internal={} plugged={} control={:#x}",
            m.codec, m.pin, adc, m.input.device, m.input.internal as u8, plugged[k] as u8, control);
        true
    }

    /// Stops recording: the stream halts, the converter drops its tag, the pin closes.
    pub fn mic_stop(&mut self) -> bool {
        let Some(k) = self.mic_on.take() else { return true };
        let sd = self.mic_sd;
        self.w32(sd + SD_CTL, self.r32(sd + SD_CTL) & !SD_CTL_RUN);
        let stopped = self.wait(|h| h.r32(sd + SD_CTL) & SD_CTL_RUN == 0);
        if let Some(m) = self.mics[k] {
            self.codec = m.codec;
            let _ = self.verb(m.path[0], SET_STREAM, 0);
            let _ = self.verb(m.pin, SET_PIN_CONTROL, 0);
        }
        stopped
    }

    fn mic_position(&self) -> usize { (self.r32(self.mic_sd + SD_LPIB) as usize % MIC_RING_BYTES) / PERIOD_BYTES }

    /// Copies the next period the microphone filled into `out`; false when the
    /// controller is still filling it.
    pub fn mic_take(&mut self, out: &mut [u8]) -> bool {
        if self.mic_on.is_none() || out.len() < PERIOD_BYTES { return false; }
        if self.mic_tail == self.mic_position() { return false; }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let at = MIC_RING_AT + self.mic_tail * PERIOD_BYTES;
        unsafe { core::ptr::copy_nonoverlapping(self.dma::<u8>(at), out.as_mut_ptr(), PERIOD_BYTES) };
        self.mic_tail = (self.mic_tail + 1) % MIC_PERIODS;
        true
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

    /// The machine is going to sleep (D3 for every audio function group with an
    /// output or microphone: amplifiers and converters power off, settings are kept)
    /// or woke (D0). HDA 7.3.3.10: the group's widgets follow it. Returns how many
    /// groups reached the state.
    pub fn power(&mut self, asleep: bool) -> u32 {
        let state = if asleep { 3 } else { 0 };
        let home = self.codec;
        let mut groups = [(u32::MAX, 0u32); MAX_OUTPUTS + MAX_MICS];
        let mut n = 0;
        let mics = self.mics;
        for (codec, afg) in self.routes.iter().flatten().map(|r| (r.codec, r.afg)).chain(mics.iter().flatten().map(|m| (m.codec, m.afg))) {
            if groups[..n].contains(&(codec, afg)) || n == groups.len() { continue; }
            groups[n] = (codec, afg);
            n += 1;
        }
        let mut reached = 0;
        for &(codec, afg) in &groups[..n] {
            self.codec = codec;
            let _ = self.verb(afg, SET_POWER, state);
            // GET_POWER: bits 7:4 are the state the group is actually in.
            let actual = self.verb(afg, 0xF05, 0).map_or(0xf, |v| v >> 4 & 0xf);
            if actual == state { reached += 1; }
        }
        self.codec = home;
        sex_pdx::serial_record!("[bell.hda.power] asleep={} groups={} reached={}", asleep as u8, n, reached);
        reached
    }
}
