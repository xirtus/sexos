// SPDX-License-Identifier: GPL-3.0-only
//! The built-in microphones of Tiger Lake laptops (the Alienware x17 R1 and most
//! 11th-gen Intel laptops): digital microphones wired to the audio DSP, not to the
//! HD Audio codec `hda.rs` drives. The DSP lives in the same PCI function as the
//! controller; BOOT gives Bell its registers (BAR4), a DMA pool and the signed Sound
//! Open Firmware (`sof.pack`) only on those controllers (`sex_pdx::audio_dsp`).
//!
//! `open` loads the firmware (`sex_sof::boot`) once, beside the codec, which keeps
//! its own coupled streams exactly as before. While someone listens, `start` builds
//! the fixed DMIC capture graph (`sex_sof::topology`) and runs one *decoupled* input
//! stream: the DSP's host component fills a ring of 32-bit stereo periods there, and
//! `take` hands each finished period to Bell as 16-bit stereo, like the codec's mic.
//! `stop` stops and frees the graph; the firmware stays loaded.
//!
//! Every failure is logged with the step it stopped at (`[bell.dsp] ...`), so one
//! boot log from the machine says how far it got.
use sex_pdx::audio_dsp as a;
use sex_pdx::memory_region as m;
use sex_sof::boot::{self, Transport};
use sex_sof::hw::Hw;
use sex_sof::regs::{dsp as dregs, hda as hregs, Region};
use sex_sof::{ipc3, topology};
use sha2::{Digest, Sha256};

/// Bell's period of 16-bit stereo frames (`hda::PERIOD_BYTES`).
const OUT_PERIOD: usize = crate::hda::PERIOD_BYTES;
/// The same period as the DSP writes it: 32-bit stereo.
const PERIOD: usize = OUT_PERIOD * 2;
const PERIODS: usize = 8;
const RING_BYTES: usize = PERIOD * PERIODS;
/// The capture ring's place in the DSP pool, after the boot scratch (firmware copy).
const BDL_AT: usize = 0x80000;
const TABLE_AT: usize = 0x81000;
const RING_AT: usize = 0x82000;
/// Input stream lent to the DSP for capture (0-based among inputs). Bell's codec mic
/// uses input 0; the firmware load borrows the last input and output.
const CAPTURE_INPUT: u32 = 1;
/// 48 kHz, 32 bits, 2 channels (HD Audio stream format).
const FORMAT_48K_S32_STEREO: u16 = 0x0041;
/// DMA position in buffer of stream `index` (Linux `SOF_HDA_ADSP_DPIB_BASE`): the host
/// DMA's own position, which a decoupled stream reports instead of LPIB.
const fn dpib(index: u32) -> u32 { 0x1084 + 0x20 * index }
/// Room for FW_READY and its extended data, and for one IPC message or reply.
const MSG: usize = 512;

/// Bell's view of the controller and the DSP, for `sex_sof`.
struct Regs { bar0: usize, bar0_len: usize, bar4: usize, bar4_len: usize, dma_va: usize, dma_pa: u64, dma_len: usize }

impl Regs {
    fn at<T>(&self, base: usize, len: usize, o: u32) -> *mut T {
        assert!(o as usize + core::mem::size_of::<T>() <= len, "BELL_DSP_MMIO_OUTSIDE_VIEW");
        (base + o as usize) as *mut T
    }
    fn buf<T>(&self, at: usize) -> *mut T { (self.dma_va + at) as *mut T }
}

impl Hw for Regs {
    fn hda_read8(&mut self, o: u32) -> u8 { unsafe { self.at::<u8>(self.bar0, self.bar0_len, o).read_volatile() } }
    fn hda_read16(&mut self, o: u32) -> u16 { unsafe { self.at::<u16>(self.bar0, self.bar0_len, o).read_volatile() } }
    fn hda_read32(&mut self, o: u32) -> u32 { unsafe { self.at::<u32>(self.bar0, self.bar0_len, o).read_volatile() } }
    fn hda_write8(&mut self, o: u32, v: u8) { unsafe { self.at::<u8>(self.bar0, self.bar0_len, o).write_volatile(v) } }
    fn hda_write16(&mut self, o: u32, v: u16) { unsafe { self.at::<u16>(self.bar0, self.bar0_len, o).write_volatile(v) } }
    fn hda_write32(&mut self, o: u32, v: u32) { unsafe { self.at::<u32>(self.bar0, self.bar0_len, o).write_volatile(v) } }
    fn dsp_read32(&mut self, o: u32) -> u32 { unsafe { self.at::<u32>(self.bar4, self.bar4_len, o).read_volatile() } }
    fn dsp_write32(&mut self, o: u32, v: u32) { unsafe { self.at::<u32>(self.bar4, self.bar4_len, o).write_volatile(v) } }
    /// Long waits sleep on the 1 ms tick; short ones read a register (each read is a
    /// round trip to the chipset of at least about half a microsecond).
    fn delay_us(&mut self, us: u32) {
        if us >= 2000 {
            let until = sex_pdx::get_ticks() + (us as u64).div_ceil(1000);
            while sex_pdx::get_ticks() < until { sex_pdx::sys_yield(); }
        } else {
            for _ in 0..us.max(1) * 2 { let _ = self.hda_read32(hregs::GCTL); }
        }
    }
    fn dma(&mut self) -> (&mut [u8], u64) {
        (unsafe { core::slice::from_raw_parts_mut(self.dma_va as *mut u8, BDL_AT) }, self.dma_pa)
    }
    fn dma_sync(&mut self) { core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst); }
}

pub struct Dsp {
    regs: Regs,
    transport: Transport,
    graph: topology::DmicCapture,
    stream: u32,
    running: bool,
    tail: usize,
    periods: u64,
}

fn log_absent(reason: &str) { sex_pdx::serial_record!("[bell.dsp.absent] reason={}", reason); }

/// The firmware file `name` in the pack (stored, not compressed), checked by SHA-256.
fn pack_file<'a>(pack: &'a [u8], name: &str) -> Result<&'a [u8], &'static str> {
    if pack.len() < 16 || &pack[..8] != b"SEXWIFW2" { return Err("pack_magic"); }
    let count = u32::from_le_bytes(pack[8..12].try_into().unwrap()) as usize;
    for i in 0..count.min(64) {
        let Some(e) = pack.get(16 + i * 88..16 + (i + 1) * 88) else { return Err("pack_truncated") };
        let len = e[..44].iter().position(|b| *b == 0).unwrap_or(44);
        if &e[..len] != name.as_bytes() { continue; }
        if u32::from_le_bytes(e[44..48].try_into().unwrap()) != 0 { return Err("pack_compressed"); }
        let at = u32::from_le_bytes(e[48..52].try_into().unwrap()) as usize;
        let n = u32::from_le_bytes(e[52..56].try_into().unwrap()) as usize;
        let data = pack.get(at..at.checked_add(n).ok_or("pack_truncated")?).ok_or("pack_truncated")?;
        if Sha256::digest(data).as_slice() != &e[56..88] { return Err("pack_damaged"); }
        return Ok(data);
    }
    Err("pack_missing")
}

fn view(slot: u64, region: u32, min: u64, rights: u8) -> Result<m::RegionView, &'static str> {
    let info = sex_pdx::memory_region_info(slot).map_err(|_| "no_region")?;
    if info.region_id != region || info.length < min { return Err("region_shape"); }
    sex_pdx::memory_region_full_view(slot, rights & info.rights).map_err(|_| "no_view")
}

impl Dsp {
    /// Boots the DSP on a cAVS controller whose BAR0 Bell maps at `bar0`. `None` (with
    /// one log line) on every other controller, in QEMU, or when anything is missing.
    pub fn open(bar0: usize, bar0_len: usize, vendor: u16, device: u16) -> Option<Dsp> {
        let Some(name) = a::firmware(vendor, device) else { return None };
        match Self::try_open(bar0, bar0_len, name) {
            Ok(d) => Some(d),
            Err(e) => { log_absent(e); None }
        }
    }

    fn try_open(bar0: usize, bar0_len: usize, name: &str) -> Result<Dsp, &'static str> {
        if (bar0_len as u64) < a::HDA_BYTES { return Err("bar0_short"); }
        let r = view(a::SLOT_REGISTERS, a::REGISTERS_REGION_ID, a::REGISTER_BYTES, m::QUERY_VIEW | m::READ | m::WRITE)?;
        let d = view(a::SLOT_DMA, a::DMA_REGION_ID, a::DMA_BYTES, m::QUERY_VIEW | m::READ | m::WRITE | m::DMA_ADDRESS)?;
        let f = view(a::SLOT_FIRMWARE, a::FIRMWARE_REGION_ID, 1, m::QUERY_VIEW | m::READ)?;
        let dma_pa = d.dma_address.ok_or("no_dma_address")?;
        let pack = unsafe { core::slice::from_raw_parts(f.virtual_address as *const u8, f.length as usize) };
        let image = pack_file(pack, name)?;
        let mut regs = Regs { bar0, bar0_len, bar4: r.virtual_address as usize, bar4_len: r.length as usize,
            dma_va: d.virtual_address as usize, dma_pa, dma_len: d.length as usize };
        if regs.dma_len < RING_AT + RING_BYTES { return Err("dma_small"); }
        let started = sex_pdx::get_ticks();
        let mut ready = [0u8; MSG];
        let booted = match boot::boot(&mut regs, image, &boot::Config::default(), &mut ready) {
            Ok(b) => b,
            Err(e) => {
                sex_pdx::serial_record!("[bell.dsp.boot] failed step={} cause={:?} rom_status={:#x} rom_error={:#x}",
                    e.step.name(), e.cause, e.rom_status, e.rom_error);
                return Err("boot");
            }
        };
        let ready = &ready[..booted.fw_ready_len.min(MSG)];
        let fw = ipc3::FwReady::parse(ready).map_err(|_| "fw_ready_parse")?;
        let v = fw.version;
        sex_pdx::serial_record!("[bell.dsp] fw_ready version={}.{}.{} build={} abi={}.{}.{} ms={} file={} bytes={}",
            v.major, v.minor, v.micro, v.build, v.abi_version >> 24, v.abi_version >> 12 & 0xfff, v.abi_version & 0xfff,
            sex_pdx::get_ticks() - started, name, image.len());
        if !ipc3::abi_compatible(v.abi_version) { boot_off(&mut regs); return Err("abi"); }
        let mut transport = booted.transport;
        if let Some(ext) = ready.get(ipc3::FW_READY_SIZE..) {
            if let Ok(w) = ipc3::Windows::from_ext_data(ext) {
                if let Ok(l) = w.layout(|id| (id < dregs::SRAM_WINDOWS).then(|| dregs::sram_window(id))) {
                    if l.host_box.1 != 0 && l.dsp_box.1 != 0 {
                        transport.set_mailboxes(Region { offset: l.host_box.0, size: l.host_box.1 }, Region { offset: l.dsp_box.0, size: l.dsp_box.1 });
                    }
                }
            }
        }
        // Input 0 is the codec mic's, the last one the firmware load's.
        if booted.caps.inputs < 3 { boot_off(&mut regs); return Err("few_inputs"); }
        let stream = CAPTURE_INPUT;
        let mut graph = topology::DmicCapture::TGL_DEFAULT.for_firmware_abi(v.abi_version);
        graph.stream_tag = (stream + 1) as u16;
        let mut dsp = Dsp { regs, transport, graph, stream, running: false, tail: 0, periods: 0 };
        dsp.graph.ring = dsp.ring()?;
        sex_pdx::serial_record!("[bell.dsp] ready inputs={} outputs={} capture_stream={} tag={} host_box={:#x}/{} dsp_box={:#x}/{}",
            booted.caps.inputs, booted.caps.outputs, stream, dsp.graph.stream_tag,
            dsp.transport.host_box.offset, dsp.transport.host_box.size, dsp.transport.dsp_box.offset, dsp.transport.dsp_box.size);
        Ok(dsp)
    }

    /// The capture ring: its BDL, the firmware's page table, and the host ring record.
    fn ring(&mut self) -> Result<topology::HostRing, &'static str> {
        let mut pages = [0u64; RING_BYTES / 4096];
        for (i, p) in pages.iter_mut().enumerate() { *p = self.regs.dma_pa + (RING_AT + i * 4096) as u64; }
        let table = unsafe { core::slice::from_raw_parts_mut(self.regs.buf::<u8>(TABLE_AT), 4096) };
        let n = topology::encode_page_table(&pages, table).map_err(|_| "page_table")?;
        Ok(topology::HostRing { page_table_addr: self.regs.dma_pa + TABLE_AT as u64, pages: n, bytes: RING_BYTES as u32, period_bytes: PERIOD as u32 })
    }

    fn sd(&self) -> u32 { hregs::sd(self.stream) }

    /// Programs the decoupled input stream: reset, BDL over the ring, 48 kHz S32 stereo.
    fn setup_stream(&mut self) -> Result<(), &'static str> {
        let sd = self.sd();
        let r = &mut self.regs;
        let ctl = r.hda_read32(sd + hregs::SD_CTL);
        r.hda_write32(sd + hregs::SD_CTL, ctl & !(hregs::SD_CTL_RUN | hregs::SD_CTL_INT_MASK));
        r.hda_write32(sd + hregs::SD_CTL, hregs::SD_CTL_SRST);
        if !wait(r, |r| r.hda_read32(sd + hregs::SD_CTL) & hregs::SD_CTL_SRST != 0) { return Err("stream_reset_enter"); }
        r.hda_write32(sd + hregs::SD_CTL, 0);
        if !wait(r, |r| r.hda_read32(sd + hregs::SD_CTL) & hregs::SD_CTL_SRST == 0) { return Err("stream_reset_leave"); }
        unsafe { core::ptr::write_bytes(r.buf::<u8>(RING_AT), 0, RING_BYTES) };
        for i in 0..PERIODS {
            let at = BDL_AT + i * 16;
            unsafe {
                r.buf::<u64>(at).write_volatile(r.dma_pa + (RING_AT + i * PERIOD) as u64);
                r.buf::<u32>(at + 8).write_volatile(PERIOD as u32);
                r.buf::<u32>(at + 12).write_volatile(0);
            }
        }
        r.dma_sync();
        let bdl = r.dma_pa + BDL_AT as u64;
        r.hda_write32(sd + hregs::SD_BDPL, bdl as u32);
        r.hda_write32(sd + hregs::SD_BDPU, (bdl >> 32) as u32);
        r.hda_write32(sd + hregs::SD_CBL, RING_BYTES as u32);
        r.hda_write16(sd + hregs::SD_LVI, (PERIODS - 1) as u16);
        r.hda_write16(sd + hregs::SD_FMT, FORMAT_48K_S32_STEREO);
        r.hda_write32(sd + hregs::SD_CTL, (self.graph.stream_tag as u32) << hregs::SD_CTL_TAG_SHIFT);
        r.hda_write8(sd + hregs::SD_STS, hregs::SD_STS_INT_MASK);
        self.decouple(true);
        Ok(())
    }

    fn decouple(&mut self, on: bool) {
        let Some(caps) = boot::capabilities(&mut self.regs) else { return };
        let at = caps.pp + hregs::PP_PPCTL;
        let bit = hregs::ppctl_procen(self.stream);
        let v = self.regs.hda_read32(at);
        self.regs.hda_write32(at, if on { v | bit } else { v & !bit });
    }

    fn run(&mut self, on: bool) -> bool {
        let sd = self.sd();
        let v = self.regs.hda_read32(sd + hregs::SD_CTL);
        self.regs.hda_write32(sd + hregs::SD_CTL, if on { v | hregs::SD_CTL_RUN } else { v & !hregs::SD_CTL_RUN });
        wait(&mut self.regs, |r| (r.hda_read32(sd + hregs::SD_CTL) & hregs::SD_CTL_RUN != 0) == on)
    }

    /// Sends one graph message and checks the firmware's answer.
    fn send(&mut self, step: topology::Step) -> Result<(), &'static str> {
        let mut msg = [0u8; MSG];
        let mut reply = [0u8; MSG];
        let n = self.graph.encode(step, &mut msg).map_err(|_| "encode")?;
        let got = self.transport.send(&mut self.regs, &msg[..n], &mut reply).map_err(|e| {
            sex_pdx::serial_record!("[bell.dsp.ipc] step={} error={:?}", step.name(), e);
            "ipc"
        })?;
        let r = ipc3::Reply::parse(&reply[..got]).map_err(|_| "reply_parse")?;
        if r.result().is_err() {
            sex_pdx::serial_record!("[bell.dsp.ipc] step={} firmware_error={}", step.name(), r.error);
            return Err("firmware_error");
        }
        Ok(())
    }

    /// Builds the DMIC graph and starts capture: host DMA first, then the DSP's
    /// trigger, as Linux starts a capture stream.
    pub fn start(&mut self) -> bool {
        if self.running { return true; }
        match self.try_start() {
            Ok(()) => {
                self.running = true;
                self.tail = self.position();
                sex_pdx::serial_record!("[bell.dsp] dmic on stream={} tag={} periods={} period_bytes={}", self.stream, self.graph.stream_tag, PERIODS, PERIOD);
                true
            }
            Err(e) => {
                sex_pdx::serial_record!("[bell.dsp] dmic start_failed reason={}", e);
                self.teardown();
                false
            }
        }
    }

    fn try_start(&mut self) -> Result<(), &'static str> {
        self.setup_stream()?;
        for &step in self.graph.start_steps() {
            if step == topology::Step::TrigStart && !self.run(true) { return Err("dma_run"); }
            self.send(step).map_err(|e| { sex_pdx::serial_record!("[bell.dsp] dmic step={} failed={}", step.name(), e); e })?;
        }
        Ok(())
    }

    /// Stops capture and frees the graph; the firmware stays loaded for next time.
    pub fn stop(&mut self) -> bool {
        if !self.running { return true; }
        self.running = false;
        sex_pdx::serial_record!("[bell.dsp] dmic off periods={}", self.periods);
        self.teardown()
    }

    fn teardown(&mut self) -> bool {
        let mut ok = true;
        for &step in self.graph.stop_steps() {
            // Stop the DSP first, then the host DMA (Linux's capture stop order).
            if self.send(step).is_err() { ok = false; }
            if step == topology::Step::TrigStop { ok &= self.run(false); }
        }
        self.decouple(false);
        ok
    }

    fn position(&mut self) -> usize { (self.regs.hda_read32(dpib(self.stream)) as usize % RING_BYTES) / PERIOD }

    /// The next period the microphones filled, as 16-bit stereo in `out`; false while
    /// the DSP is still filling it.
    pub fn take(&mut self, out: &mut [u8]) -> bool {
        if !self.running || out.len() < OUT_PERIOD { return false; }
        if self.tail == self.position() { return false; }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let src = self.regs.buf::<u32>(RING_AT + self.tail * PERIOD) as *const u32;
        for i in 0..PERIOD / 4 {
            // The top 16 bits of each 32-bit sample.
            let s = unsafe { src.add(i).read_volatile() };
            out[i * 2..i * 2 + 2].copy_from_slice(&((s >> 16) as u16).to_le_bytes());
        }
        self.tail = (self.tail + 1) % PERIODS;
        if self.periods == 0 { sex_pdx::serial_record!("[bell.dsp] dmic first_period"); }
        self.periods += 1;
        true
    }

}

fn boot_off(regs: &mut Regs) { let _ = boot::power_down(regs); }

fn wait(r: &mut Regs, mut done: impl FnMut(&mut Regs) -> bool) -> bool {
    for _ in 0..20_000 {
        if done(r) { return true; }
        r.delay_us(10);
    }
    false
}

static mut DSP: Option<Dsp> = None;
/// Keeps the booted DSP for the life of Bell.
pub fn set(d: Option<Dsp>) { unsafe { *core::ptr::addr_of_mut!(DSP) = d } }
/// The booted DSP, if this controller has one.
pub fn get() -> Option<&'static mut Dsp> { unsafe { (*core::ptr::addr_of_mut!(DSP)).as_mut() } }
