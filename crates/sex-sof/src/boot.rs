// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Booting the cAVS 2.5 DSP (Tiger Lake) with SOF firmware, and the IPC transport.
//!
//! The sequence is the one Linux's SOF driver runs on TGL (`cl_dsp_init`,
//! `hda_dsp_cl_boot_firmware_iccmax`, `hda_cl_copy_fw` in
//! `sound/soc/sof/intel/hda-loader.c`), written as explicit steps:
//!
//! 1. The HD Audio controller is out of reset (Bell brought it up; we only look).
//! 2. Find the processing pipe (PP) and SPIB capabilities; enable PP (GPROCEN, PIE).
//! 3. Run the ICCMAX recipe: a small decoupled capture stream runs while the
//!    firmware loads, with the LTRP guardband at 95 us (Intel's recommendation).
//! 4. Prepare the code-loader stream: a decoupled host-DMA *output* stream whose
//!    buffer (in the DMA scratch) holds the firmware payload.
//! 5. Power core 0 (SPA -> CPA), send the ROM "purge" IPC naming the loader stream,
//!    take core 0 out of reset and unstall it, wait for the ROM's IPC DONE, enable
//!    IPC interrupts and wait for ROM INIT_DONE. Linux retries this three times.
//! 6. Start the loader DMA, wait for FW_ENTERED, stop the DMA, release the streams.
//! 7. Wait for the firmware's FW_READY message and hand its raw bytes to the caller
//!    (the `ipc3` module parses them).
//!
//! Any failure after the DSP was touched powers core 0 down again, re-couples the
//! borrowed streams and restores the vendor registers, so Bell's codec keeps
//! working whatever happens here. Errors name the step that failed.

use crate::fw;
use crate::hw::Hw;
use crate::regs::{dsp, hda, timeout, Region};

/// IPC3 global message type field (`SOF_GLB_TYPE_MASK`).
const IPC3_GLB_TYPE_MASK: u32 = 0xf000_0000;
/// IPC3 FW_READY global type (`SOF_IPC_FW_READY`).
const IPC3_FW_READY: u32 = 0x7000_0000;

/// Scratch layout: buffer descriptor list of the code-loader stream.
pub const LOADER_BDL_AT: usize = 0x0000;
/// Scratch layout: buffer descriptor list of the ICCMAX stream.
pub const ICCMAX_BDL_AT: usize = 0x0100;
/// Scratch layout: the ICCMAX capture buffer (its contents are never used).
pub const ICCMAX_BUF_AT: usize = 0x1000;
/// Length of the ICCMAX capture buffer (one page, as Linux uses).
pub const ICCMAX_BUF_LEN: usize = 0x1000;
/// Scratch layout: the firmware payload copy.
pub const FW_AT: usize = 0x2000;

/// Bytes of DMA scratch the boot needs for a payload of `payload_len` bytes.
pub const fn scratch_needed(payload_len: usize) -> usize { FW_AT + payload_len }

/// Boot choices. The defaults are what Bell wants on the x17.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Output stream (0-based among output streams) lent to the code loader. `None`
    /// takes the last one; Bell's codec playback uses the first.
    pub loader_stream: Option<u8>,
    /// Run the ICCMAX recipe during the firmware load (Linux does on TGL).
    pub iccmax: bool,
    /// Input stream (0-based among input streams) lent to ICCMAX. `None` takes the
    /// last one; Bell's jack capture uses the first.
    pub iccmax_stream: Option<u8>,
    /// ROM handshake attempts (Linux: 3).
    pub attempts: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { loader_stream: None, iccmax: true, iccmax_stream: None, attempts: timeout::BOOT_ATTEMPTS }
    }
}

/// A boot step, named in errors and log lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// GCTL.CRST reads 1.
    ControllerReady,
    /// The extended capability walk (PP is required) and stream choice.
    Capabilities,
    /// The firmware image check.
    Image,
    /// The DMA scratch is big enough and aligned.
    Scratch,
    /// The ICCMAX capture stream.
    IccmaxStream,
    /// The code-loader stream (stop, reset, program).
    LoaderStream,
    /// Core 0 power up (CPA follows SPA).
    CorePowerUp,
    /// Core 0 leaves reset.
    CoreReset,
    /// Core 0 runs (powered, out of reset, not stalled).
    CoreRun,
    /// The ROM acknowledges the purge IPC (HIPCIDA.DONE).
    RomIpc,
    /// The ROM reports INIT_DONE.
    RomInit,
    /// The loader DMA starts.
    DmaStart,
    /// The ROM reports FW_ENTERED.
    FwEntered,
    /// The loader DMA stops.
    DmaStop,
    /// The firmware posts FW_READY.
    FwReady,
    /// Core 0 stall and reset on the way down.
    CoreStall,
    /// Core 0 power down (CPA clears).
    CorePowerDown,
}

impl Step {
    /// Short lowercase name for `[bell.dsp]` log lines.
    pub const fn name(self) -> &'static str {
        match self {
            Step::ControllerReady => "controller_ready",
            Step::Capabilities => "capabilities",
            Step::Image => "image",
            Step::Scratch => "scratch",
            Step::IccmaxStream => "iccmax_stream",
            Step::LoaderStream => "loader_stream",
            Step::CorePowerUp => "core_power_up",
            Step::CoreReset => "core_reset",
            Step::CoreRun => "core_run",
            Step::RomIpc => "rom_ipc",
            Step::RomInit => "rom_init",
            Step::DmaStart => "dma_start",
            Step::FwEntered => "fw_entered",
            Step::DmaStop => "dma_stop",
            Step::FwReady => "fw_ready",
            Step::CoreStall => "core_stall",
            Step::CorePowerDown => "core_power_down",
        }
    }
}

/// What went wrong in a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The hardware did not reach the expected state in time.
    Timeout,
    /// Something required is absent (PP capability, a free stream).
    Missing,
    /// The firmware image is malformed.
    Image(fw::Error),
    /// The DMA scratch is too small or not 128-byte aligned on the bus.
    Scratch,
    /// The firmware panicked; the value is HIPCTDR.
    Panic(u32),
}

/// A failed boot: the step, the cause, and the ROM status/error words at the time
/// (window 0 words 0 and 1), which say where the ROM itself got stuck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootError {
    /// The step that failed.
    pub step: Step,
    /// Why.
    pub cause: Cause,
    /// FSR (ROM / firmware status register) when the step failed.
    pub rom_status: u32,
    /// ROM error code when the step failed.
    pub rom_error: u32,
}

/// The controller's extended capabilities and stream counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// BAR0 offset of the processing pipe capability.
    pub pp: u32,
    /// BAR0 offset of the SPIB capability, if present.
    pub spib: Option<u32>,
    /// BAR0 offset of the multi-link capability, if present.
    pub ml: Option<u32>,
    /// Number of input streams (GCAP.ISS).
    pub inputs: u32,
    /// Number of output streams (GCAP.OSS).
    pub outputs: u32,
}

/// A stream descriptor lent to the DSP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream {
    /// Index over all streams (inputs first): selects SDx, PROCEN and SPIB bits.
    pub index: u32,
    /// Stream tag, 1-based within its direction; the ROM wants `tag - 1`.
    pub tag: u32,
}

impl Stream {
    /// BAR0 offset of this stream's descriptor.
    pub const fn sd(self) -> u32 { hda::sd(self.index) }
}

/// The result of a good boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Booted {
    /// IPC transport, with the mailboxes the image declared (or TGL defaults). The
    /// `ipc3` module may replace them with the FW_READY windows.
    pub transport: Transport,
    /// What was loaded.
    pub image: fw::Info,
    /// The capabilities found.
    pub caps: Caps,
    /// Bytes of FW_READY copied into the caller's buffer (the `sof_ipc_fw_ready`
    /// followed by its extended data: windows, compiler, debug ABI, ...).
    pub fw_ready_len: usize,
}

struct Ctx<'a, H: Hw> {
    hw: &'a mut H,
    step: Step,
}

impl<H: Hw> Ctx<'_, H> {
    fn fail(&mut self, cause: Cause) -> BootError {
        let rom_status = self.hw.dsp_read32(dsp::SRAM_ROM_STATUS);
        let rom_error = self.hw.dsp_read32(dsp::SRAM_ROM_ERROR);
        BootError { step: self.step, cause, rom_status, rom_error }
    }
}

/// Polls `done` every `interval` microseconds for up to `limit` microseconds.
fn poll<H: Hw>(hw: &mut H, interval: u32, limit: u32, mut done: impl FnMut(&mut H) -> bool) -> bool {
    let mut waited = 0u32;
    loop {
        if done(hw) {
            return true;
        }
        if waited >= limit {
            return false;
        }
        hw.delay_us(interval);
        waited = waited.saturating_add(interval);
    }
}

fn hda_update32<H: Hw>(hw: &mut H, off: u32, mask: u32, val: u32) {
    let v = hw.hda_read32(off);
    hw.hda_write32(off, (v & !mask) | (val & mask));
}

fn dsp_update32<H: Hw>(hw: &mut H, off: u32, mask: u32, val: u32) {
    let v = hw.dsp_read32(off);
    hw.dsp_write32(off, (v & !mask) | (val & mask));
}

/// SDxCTL is 24 bits wide with SDxSTS in the byte above; this keeps the status byte
/// zero on write so read-modify-write never clears status by accident.
fn sd_ctl_update<H: Hw>(hw: &mut H, sd: u32, mask: u32, val: u32) {
    let v = hw.hda_read32(sd + hda::SD_CTL) & 0x00ff_ffff;
    hw.hda_write32(sd + hda::SD_CTL, (v & !mask) | (val & mask));
}

fn rom_state<H: Hw>(hw: &mut H) -> u32 { hw.dsp_read32(dsp::SRAM_ROM_STATUS) & dsp::FSR_STATE_MASK }

/// Walks the extended capability list and reads the stream counts.
pub fn capabilities<H: Hw>(hw: &mut H) -> Option<Caps> {
    let gcap = hw.hda_read16(hda::GCAP) as u32;
    let mut caps = Caps { pp: 0, spib: None, ml: None, inputs: gcap >> 8 & 0xf, outputs: gcap >> 12 & 0xf };
    let mut pp = None;
    let mut at = hw.hda_read32(hda::LLCH) & hda::CAP_NEXT_MASK;
    let mut n = 0;
    while at != 0 && at < 0x4000 && n < hda::MAX_CAPS {
        let header = hw.hda_read32(at);
        if header == u32::MAX {
            break;
        }
        match header >> hda::CAP_ID_SHIFT & hda::CAP_ID_MASK {
            hda::CAP_ID_PP => pp = Some(at),
            hda::CAP_ID_SPIB => caps.spib = Some(at),
            hda::CAP_ID_ML => caps.ml = Some(at),
            _ => {}
        }
        at = header & hda::CAP_NEXT_MASK;
        n += 1;
    }
    caps.pp = pp?;
    Some(caps)
}

/// The loader (output) and ICCMAX (input) streams for `cfg`.
fn pick_streams(caps: &Caps, cfg: &Config) -> Option<(Stream, Option<Stream>)> {
    if caps.outputs == 0 {
        return None;
    }
    let out = cfg.loader_stream.map_or(caps.outputs - 1, u32::from);
    if out >= caps.outputs || out >= 15 {
        return None;
    }
    let loader = Stream { index: caps.inputs + out, tag: out + 1 };
    let iccmax = if cfg.iccmax {
        if caps.inputs == 0 {
            return None;
        }
        let inp = cfg.iccmax_stream.map_or(caps.inputs - 1, u32::from);
        if inp >= caps.inputs {
            return None;
        }
        Some(Stream { index: inp, tag: inp + 1 })
    } else {
        None
    };
    Some((loader, iccmax))
}

/// Writes a two-entry buffer descriptor list at `bdl_at` describing `len` bytes at
/// scratch offset `buf_at`. HDA needs LVI >= 1, so even one contiguous buffer is
/// split in two, at a 128-byte boundary.
fn write_bdl(scratch: &mut [u8], bus: u64, bdl_at: usize, buf_at: usize, len: usize) {
    let first = (len / 2) & !(hda::DMA_ALIGN as usize - 1);
    let parts = [(buf_at, first), (buf_at + first, len - first)];
    for (i, (at, size)) in parts.iter().enumerate() {
        let e = bdl_at + i * hda::BDLE_SIZE;
        scratch[e..e + 8].copy_from_slice(&(bus + *at as u64).to_le_bytes());
        scratch[e + 8..e + 12].copy_from_slice(&(*size as u32).to_le_bytes());
        scratch[e + 12..e + 16].copy_from_slice(&0u32.to_le_bytes());
    }
}

fn program_bdl<H: Hw>(hw: &mut H, sd: u32, bdl_bus: u64, len: u32) {
    hw.hda_write32(sd + hda::SD_BDPL, bdl_bus as u32);
    hw.hda_write32(sd + hda::SD_BDPU, (bdl_bus >> 32) as u32);
    hw.hda_write32(sd + hda::SD_CBL, len);
    hw.hda_write16(sd + hda::SD_LVI, 1);
}

/// Stops a stream and waits until RUN reads 0, then clears its status.
fn stream_stop<H: Hw>(hw: &mut H, sd: u32) -> bool {
    sd_ctl_update(hw, sd, hda::SD_CTL_RUN | hda::SD_CTL_INT_MASK, 0);
    let ok = poll(hw, timeout::STREAM_POLL_US, timeout::STREAM_US, |h| {
        h.hda_read32(sd + hda::SD_CTL) & hda::SD_CTL_RUN == 0
    });
    hw.hda_write8(sd + hda::SD_STS, hda::SD_STS_INT_MASK);
    ok
}

/// Puts a stream through reset (SRST 1, then 0).
fn stream_reset<H: Hw>(hw: &mut H, sd: u32) -> bool {
    sd_ctl_update(hw, sd, hda::SD_CTL_SRST, hda::SD_CTL_SRST);
    if !poll(hw, timeout::STREAM_POLL_US, timeout::STREAM_US, |h| {
        h.hda_read32(sd + hda::SD_CTL) & hda::SD_CTL_SRST != 0
    }) {
        return false;
    }
    sd_ctl_update(hw, sd, hda::SD_CTL_SRST, 0);
    hw.delay_us(3);
    poll(hw, timeout::STREAM_POLL_US, timeout::STREAM_US, |h| {
        h.hda_read32(sd + hda::SD_CTL) & hda::SD_CTL_SRST == 0
    })
}

/// Releases a borrowed stream: stopped, no BDL, control cleared, coupled again.
fn stream_release<H: Hw>(hw: &mut H, caps: &Caps, s: Stream) {
    let sd = s.sd();
    sd_ctl_update(hw, sd, hda::SD_CTL_RUN, 0);
    if let Some(spib) = caps.spib {
        hw.hda_write32(spib + hda::spib(s.index), 0);
        hda_update32(hw, spib + hda::SPIB_SPBFCCTL, 1 << s.index, 0);
    }
    hw.hda_write32(sd + hda::SD_BDPL, 0);
    hw.hda_write32(sd + hda::SD_BDPU, 0);
    hw.hda_write32(sd + hda::SD_CTL, 0);
    hda_update32(hw, caps.pp + hda::PP_PPCTL, hda::ppctl_procen(s.index), 0);
}

/// Saved vendor registers, put back when the boot ends either way.
struct Saved {
    em2: u32,
    ltrp_gb: u8,
}

fn restore<H: Hw>(hw: &mut H, saved: &Saved) {
    let l = hw.hda_read8(hda::VS_LTRP);
    hw.hda_write8(hda::VS_LTRP, (l & !hda::VS_LTRP_GB_MASK) | saved.ltrp_gb);
    hda_update32(hw, hda::VS_EM2, hda::VS_EM2_L1SEN, saved.em2);
}

/// ICCMAX recipe (Linux `hda_dsp_iccmax_stream_hw_params`): a decoupled capture
/// stream with a one-page buffer runs during the firmware load, guardband at 95 us.
fn iccmax_start<H: Hw>(hw: &mut H, caps: &Caps, s: Stream, bus: u64) {
    let sd = s.sd();
    hw.hda_write32(sd + hda::SD_BDPL, 0);
    hw.hda_write32(sd + hda::SD_BDPU, 0);
    program_bdl(hw, sd, bus + ICCMAX_BDL_AT as u64, ICCMAX_BUF_LEN as u32);
    let pp = caps.pp + hda::PP_PPCTL;
    hda_update32(hw, pp, hda::ppctl_procen(s.index), hda::ppctl_procen(s.index));
    let l = hw.hda_read8(hda::VS_LTRP);
    hw.hda_write8(hda::VS_LTRP, (l & !hda::VS_LTRP_GB_MASK) | hda::LTRP_GB_BOOT_US);
    sd_ctl_update(hw, sd, hda::SD_CTL_RUN, hda::SD_CTL_RUN);
}

/// Code-loader stream setup (Linux `hda_dsp_stream_hw_params` + SPIB).
fn loader_prepare<H: Hw>(hw: &mut H, caps: &Caps, s: Stream, bus: u64, len: u32) -> bool {
    let sd = s.sd();
    let procen = hda::ppctl_procen(s.index);
    hda_update32(hw, caps.pp + hda::PP_PPCTL, procen, procen);
    if !stream_stop(hw, sd) || !stream_reset(hw, sd) {
        return false;
    }
    hw.hda_write32(sd + hda::SD_BDPL, 0);
    hw.hda_write32(sd + hda::SD_BDPU, 0);
    sd_ctl_update(hw, sd, hda::SD_CTL_TAG_MASK, s.tag << hda::SD_CTL_TAG_SHIFT);
    hw.hda_write32(sd + hda::SD_CBL, len);
    hw.hda_write16(sd + hda::SD_FMT, hda::CL_STREAM_FORMAT);
    program_bdl(hw, sd, bus + LOADER_BDL_AT as u64, len);
    sd_ctl_update(hw, sd, hda::SD_CTL_INT_MASK, hda::SD_CTL_INT_MASK);
    if let Some(spib) = caps.spib {
        hda_update32(hw, spib + hda::SPIB_SPBFCCTL, 1 << s.index, 1 << s.index);
        hw.hda_write32(spib + hda::spib(s.index), len);
    }
    true
}

fn core_running<H: Hw>(hw: &mut H, m: u32) -> bool {
    let cs = hw.dsp_read32(dsp::ADSPCS);
    cs & dsp::adspcs_cpa(m) == dsp::adspcs_cpa(m)
        && cs & dsp::adspcs_spa(m) == dsp::adspcs_spa(m)
        && cs & (dsp::adspcs_crst(m) | dsp::adspcs_cstall(m)) == 0
}

/// Stalls core 0, puts it in reset and removes its power
/// (Linux `hda_dsp_core_reset_power_down`).
fn core_down<H: Hw>(ctx: &mut Ctx<'_, H>) -> Result<(), BootError> {
    let m = dsp::CORE0;
    ctx.step = Step::CoreStall;
    dsp_update32(ctx.hw, dsp::ADSPCS, dsp::adspcs_cstall(m), dsp::adspcs_cstall(m));
    dsp_update32(ctx.hw, dsp::ADSPCS, dsp::adspcs_crst(m), dsp::adspcs_crst(m));
    if !poll(ctx.hw, timeout::POLL_US, timeout::CORE_US, |h| {
        h.dsp_read32(dsp::ADSPCS) & dsp::adspcs_crst(m) != 0
    }) {
        return Err(ctx.fail(Cause::Timeout));
    }
    ctx.step = Step::CorePowerDown;
    dsp_update32(ctx.hw, dsp::ADSPCS, dsp::adspcs_spa(m), 0);
    if !poll(ctx.hw, timeout::POLL_US, timeout::CORE_POWER_DOWN_US, |h| {
        h.dsp_read32(dsp::ADSPCS) & dsp::adspcs_cpa(m) == 0
    }) {
        return Err(ctx.fail(Cause::Timeout));
    }
    Ok(())
}

fn ipc_int_enable<H: Hw>(hw: &mut H, on: bool) {
    let both = dsp::HIPCCTL_BUSY | dsp::HIPCCTL_DONE;
    if on {
        dsp_update32(hw, dsp::HIPCCTL, both, both);
        dsp_update32(hw, dsp::ADSPIC, dsp::ADSPIC_IPC, dsp::ADSPIC_IPC);
    } else {
        dsp_update32(hw, dsp::ADSPIC, dsp::ADSPIC_IPC, 0);
        dsp_update32(hw, dsp::HIPCCTL, both, 0);
    }
}

/// One ROM handshake attempt (Linux `cl_dsp_init`).
fn rom_handshake<H: Hw>(ctx: &mut Ctx<'_, H>, loader: Stream) -> Result<(), BootError> {
    let m = dsp::CORE0;
    ctx.step = Step::CorePowerUp;
    dsp_update32(ctx.hw, dsp::ADSPCS, dsp::adspcs_spa(m), dsp::adspcs_spa(m));
    if !poll(ctx.hw, timeout::POLL_US, timeout::CORE_US, |h| {
        h.dsp_read32(dsp::ADSPCS) & dsp::adspcs_cpa(m) != 0
    }) {
        return Err(ctx.fail(Cause::Timeout));
    }
    for i in 0..dsp::SSP_COUNT {
        let ssc1 = dsp::SSP_BASE + i * dsp::SSP_STRIDE + dsp::SSP_SSC1;
        dsp_update32(ctx.hw, ssc1, dsp::SSP_SSC1_CONSUMER, dsp::SSP_SSC1_CONSUMER);
    }
    // The purge request goes in before the core runs; the ROM reads it on start.
    let purge = dsp::HIPCIDR_BUSY
        | dsp::ROM_IPC_CONTROL
        | dsp::ROM_IPC_PURGE_FW
        | (loader.tag - 1) << dsp::ROM_IPC_DMA_SHIFT;
    ctx.hw.dsp_write32(dsp::HIPCIDR, purge);

    ctx.step = Step::CoreReset;
    dsp_update32(ctx.hw, dsp::ADSPCS, dsp::adspcs_crst(m), 0);
    if !poll(ctx.hw, timeout::POLL_US, timeout::CORE_US, |h| {
        h.dsp_read32(dsp::ADSPCS) & dsp::adspcs_crst(m) == 0
    }) {
        return Err(ctx.fail(Cause::Timeout));
    }
    ctx.step = Step::CoreRun;
    dsp_update32(ctx.hw, dsp::ADSPCS, dsp::adspcs_cstall(m), 0);
    if !core_running(ctx.hw, m) {
        return Err(ctx.fail(Cause::Timeout));
    }

    ctx.step = Step::RomIpc;
    if !poll(ctx.hw, timeout::POLL_US, timeout::ROM_IPC_US, |h| {
        h.dsp_read32(dsp::HIPCIDA) & dsp::HIPCIDA_DONE != 0
    }) {
        return Err(ctx.fail(Cause::Timeout));
    }
    dsp_update32(ctx.hw, dsp::HIPCIDA, dsp::HIPCIDA_DONE, dsp::HIPCIDA_DONE);
    ipc_int_enable(ctx.hw, true);

    ctx.step = Step::RomInit;
    if !poll(ctx.hw, timeout::POLL_US, timeout::ROM_INIT_US, |h| rom_state(h) == dsp::FSR_STATE_INIT_DONE) {
        return Err(ctx.fail(Cause::Timeout));
    }
    Ok(())
}

/// Loader DMA run (Linux `hda_cl_copy_fw`): start, wait FW_ENTERED, stop.
fn copy_fw<H: Hw>(ctx: &mut Ctx<'_, H>, loader: Stream) -> Result<(), BootError> {
    let sd = loader.sd();
    let bit = 1 << loader.index;
    ctx.step = Step::DmaStart;
    hda_update32(ctx.hw, hda::INTCTL, bit, bit);
    let run = hda::SD_CTL_RUN | hda::SD_CTL_INT_MASK;
    sd_ctl_update(ctx.hw, sd, run, run);

    ctx.step = Step::FwEntered;
    let entered = poll(ctx.hw, timeout::POLL_US, timeout::BASEFW_US, |h| {
        rom_state(h) == dsp::FSR_STATE_FW_ENTERED
    });
    let entered = if entered { Ok(()) } else { Err(ctx.fail(Cause::Timeout)) };

    ctx.step = Step::DmaStop;
    let stopped = stream_stop(ctx.hw, sd);
    hda_update32(ctx.hw, hda::INTCTL, bit, 0);
    entered?;
    if !stopped {
        return Err(ctx.fail(Cause::Timeout));
    }
    Ok(())
}

/// Clears a DSP-to-host doorbell: BUSY written back, DONE raised
/// (Linux `cnl_ipc_host_done`).
fn host_done<H: Hw>(hw: &mut H) {
    dsp_update32(hw, dsp::HIPCTDR, dsp::HIPCTDR_BUSY, dsp::HIPCTDR_BUSY);
    dsp_update32(hw, dsp::HIPCTDA, dsp::HIPCTDA_DONE, dsp::HIPCTDA_DONE);
}

fn read_box<H: Hw>(hw: &mut H, offset: u32, buf: &mut [u8]) {
    for (i, chunk) in buf.chunks_mut(4).enumerate() {
        let w = hw.dsp_read32(offset + 4 * i as u32).to_le_bytes();
        chunk.copy_from_slice(&w[..chunk.len()]);
    }
}

fn write_box<H: Hw>(hw: &mut H, offset: u32, msg: &[u8]) {
    for (i, chunk) in msg.chunks(4).enumerate() {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        hw.dsp_write32(offset + 4 * i as u32, u32::from_le_bytes(w));
    }
}

/// Waits for FW_READY in the uplink mailbox and copies it out.
fn wait_fw_ready<H: Hw>(ctx: &mut Ctx<'_, H>, out: &mut [u8]) -> Result<usize, BootError> {
    ctx.step = Step::FwReady;
    let mut got = None;
    let mut panic = None;
    let ok = poll(ctx.hw, timeout::POLL_US, timeout::FW_READY_US, |h| {
        let tdr = h.dsp_read32(dsp::HIPCTDR);
        if tdr & dsp::HIPCTDR_BUSY == 0 {
            return false;
        }
        if tdr & dsp::IPC_PANIC_MAGIC_MASK == dsp::IPC_PANIC_MAGIC {
            panic = Some(tdr);
            host_done(h);
            return true;
        }
        let cmd = h.dsp_read32(dsp::MBOX_UPLINK + 4);
        if cmd & IPC3_GLB_TYPE_MASK == IPC3_FW_READY {
            let n = out.len().min(dsp::DEFAULT_DSP_BOX.size as usize);
            read_box(h, dsp::MBOX_UPLINK, &mut out[..n]);
            got = Some(n);
        }
        // Anything else before FW_READY is dropped, as Linux does.
        host_done(h);
        got.is_some()
    });
    if let Some(tdr) = panic {
        return Err(ctx.fail(Cause::Panic(tdr)));
    }
    match got {
        Some(n) if ok => Ok(n),
        _ => Err(ctx.fail(Cause::Timeout)),
    }
}

/// Boots the DSP with `image` (a whole `.ri` file) and waits for FW_READY, whose
/// raw bytes land in `fw_ready` (1 KiB is plenty). On error the DSP is left powered
/// down and the borrowed streams are coupled again.
pub fn boot<H: Hw>(hw: &mut H, image: &[u8], cfg: &Config, fw_ready: &mut [u8]) -> Result<Booted, BootError> {
    let mut ctx = Ctx { hw, step: Step::ControllerReady };
    if !poll(ctx.hw, timeout::POLL_US, timeout::CTRL_RESET_US, |h| {
        h.hda_read32(hda::GCTL) & hda::GCTL_CRST != 0
    }) {
        return Err(ctx.fail(Cause::Timeout));
    }

    ctx.step = Step::Capabilities;
    let caps = capabilities(ctx.hw).ok_or_else(|| ctx.fail(Cause::Missing))?;
    let (loader, iccmax) = pick_streams(&caps, cfg).ok_or_else(|| ctx.fail(Cause::Missing))?;

    ctx.step = Step::Image;
    let info = fw::check(image).map_err(|e| ctx.fail(Cause::Image(e)))?;
    let payload = info.payload(image);

    ctx.step = Step::Scratch;
    let bus = {
        let (scratch, bus) = ctx.hw.dma();
        if scratch.len() < scratch_needed(payload.len()) || bus % hda::DMA_ALIGN != 0 {
            None
        } else {
            scratch[FW_AT..FW_AT + payload.len()].copy_from_slice(payload);
            write_bdl(scratch, bus, LOADER_BDL_AT, FW_AT, payload.len());
            write_bdl(scratch, bus, ICCMAX_BDL_AT, ICCMAX_BUF_AT, ICCMAX_BUF_LEN);
            Some(bus)
        }
    };
    let bus = bus.ok_or_else(|| ctx.fail(Cause::Scratch))?;
    ctx.hw.dma_sync();

    // From here on the hardware is touched: undo on every error.
    let pp = caps.pp + hda::PP_PPCTL;
    hda_update32(ctx.hw, pp, hda::PPCTL_GPROCEN | hda::PPCTL_PIE, hda::PPCTL_GPROCEN | hda::PPCTL_PIE);
    let saved = Saved {
        em2: ctx.hw.hda_read32(hda::VS_EM2) & hda::VS_EM2_L1SEN,
        ltrp_gb: ctx.hw.hda_read8(hda::VS_LTRP) & hda::VS_LTRP_GB_MASK,
    };
    hda_update32(ctx.hw, hda::VS_EM2, hda::VS_EM2_L1SEN, 0);

    let result = load(&mut ctx, &caps, loader, iccmax, bus, payload.len() as u32, cfg.attempts);
    if let Some(s) = iccmax {
        stream_release(ctx.hw, &caps, s);
    }
    stream_release(ctx.hw, &caps, loader);
    restore(ctx.hw, &saved);

    let result = result.and_then(|()| wait_fw_ready(&mut ctx, fw_ready));
    match result {
        Ok(fw_ready_len) => {
            let transport = match info.mailboxes {
                Some(m) => Transport::new(m.host, m.dsp),
                None => Transport::new(dsp::DEFAULT_HOST_BOX, dsp::DEFAULT_DSP_BOX),
            };
            Ok(Booted { transport, image: info, caps, fw_ready_len })
        }
        Err(e) => {
            dsp_off(ctx.hw, caps.pp);
            Err(e)
        }
    }
}

fn load<H: Hw>(
    ctx: &mut Ctx<'_, H>,
    caps: &Caps,
    loader: Stream,
    iccmax: Option<Stream>,
    bus: u64,
    len: u32,
    attempts: u32,
) -> Result<(), BootError> {
    if let Some(s) = iccmax {
        ctx.step = Step::IccmaxStream;
        iccmax_start(ctx.hw, caps, s, bus);
    }
    ctx.step = Step::LoaderStream;
    if !loader_prepare(ctx.hw, caps, loader, bus, len) {
        return Err(ctx.fail(Cause::Timeout));
    }
    // A core left running (an earlier boot, a crashed Bell) is taken down first.
    if ctx.hw.dsp_read32(dsp::ADSPCS) & dsp::adspcs_cpa(dsp::CORE0) != 0 {
        core_down(ctx)?;
    }
    let mut last = None;
    for _ in 0..attempts.max(1) {
        match rom_handshake(ctx, loader) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                ipc_int_enable(ctx.hw, false);
                let _ = core_down(ctx);
            }
        }
    }
    if let Some(e) = last {
        return Err(e);
    }
    copy_fw(ctx, loader)
}

/// Disables IPC interrupts, takes core 0 down and turns the processing pipe off.
/// Errors are ignored: this is the last resort path.
fn dsp_off<H: Hw>(hw: &mut H, pp: u32) {
    ipc_int_enable(hw, false);
    let mut ctx = Ctx { hw, step: Step::CoreStall };
    let _ = core_down(&mut ctx);
    hda_update32(ctx.hw, pp + hda::PP_PPCTL, hda::PPCTL_GPROCEN | hda::PPCTL_PIE, 0);
}

/// Powers the DSP down: IPC interrupts off, core 0 stalled, reset and unpowered,
/// processing pipe off (Linux `hda_dsp_core_reset_power_down` + `ppcap_enable(false)`).
/// Bell's coupled codec streams are not affected.
pub fn power_down<H: Hw>(hw: &mut H) -> Result<(), BootError> {
    ipc_int_enable(hw, false);
    let mut ctx = Ctx { hw, step: Step::CoreStall };
    core_down(&mut ctx)?;
    if let Some(caps) = capabilities(ctx.hw) {
        hda_update32(ctx.hw, caps.pp + hda::PP_PPCTL, hda::PPCTL_GPROCEN | hda::PPCTL_PIE, 0);
    }
    Ok(())
}

/// IPC failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcError {
    /// The previous host message is still pending (HIPCIDR.BUSY never cleared).
    Busy,
    /// The DSP did not acknowledge (HIPCIDA.DONE) in time.
    Timeout,
    /// The message is empty or larger than the host mailbox.
    TooLarge,
    /// The reply header says 0..8 bytes (no `sof_ipc_reply` header).
    EmptyReply,
    /// The reply is larger than the caller's buffer; the value is its size.
    ReplyTooLarge(u32),
    /// The firmware panicked: HIPCTDR and HIPCTDD (panic record location).
    Panic {
        /// HIPCTDR, the panic code.
        code: u32,
        /// HIPCTDD, offset of the panic record in window 0 (low 24 bits).
        extension: u32,
    },
}

/// A message the DSP posted on its own (a notification, or a position update).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notification {
    /// HIPCTDR message bits.
    pub primary: u32,
    /// HIPCTDD message bits.
    pub extension: u32,
    /// Bytes copied from the DSP mailbox into the caller's buffer.
    pub len: usize,
}

/// IPC over the CNL-style doorbells and the SRAM mailboxes (IPC3 framing: every
/// mailbox message starts with a `u32` byte size).
///
/// Host to DSP: the request goes into the host mailbox, HIPCIDR gets BUSY; the DSP
/// answers in the same mailbox and raises HIPCIDA.DONE, which the host clears.
/// DSP to host: the DSP fills its mailbox and raises HIPCTDR.BUSY; the host reads,
/// writes BUSY back and raises HIPCTDA.DONE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transport {
    /// Host-to-DSP mailbox (requests and replies), SOF `DOWNBOX`.
    pub host_box: Region,
    /// DSP-to-host mailbox (notifications), SOF `UPBOX`.
    pub dsp_box: Region,
}

impl Transport {
    /// A transport over the given mailboxes.
    pub const fn new(host_box: Region, dsp_box: Region) -> Self { Transport { host_box, dsp_box } }

    /// Switches to the mailboxes FW_READY announced.
    pub fn set_mailboxes(&mut self, host_box: Region, dsp_box: Region) {
        self.host_box = host_box;
        self.dsp_box = dsp_box;
    }

    /// Sends `msg` and copies the reply into `reply`. Returns the reply length.
    pub fn send<H: Hw>(&mut self, hw: &mut H, msg: &[u8], reply: &mut [u8]) -> Result<usize, IpcError> {
        if msg.is_empty() || msg.len() > self.host_box.size as usize {
            return Err(IpcError::TooLarge);
        }
        if !poll(hw, timeout::STREAM_POLL_US, timeout::IPC_US, |h| {
            h.dsp_read32(dsp::HIPCIDR) & dsp::HIPCIDR_BUSY == 0
        }) {
            return Err(IpcError::Busy);
        }
        write_box(hw, self.host_box.offset, msg);
        hw.dsp_write32(dsp::HIPCIDR, dsp::HIPCIDR_BUSY);
        if !poll(hw, timeout::STREAM_POLL_US, timeout::IPC_US, |h| {
            h.dsp_read32(dsp::HIPCIDA) & dsp::HIPCIDA_DONE != 0
        }) {
            return Err(IpcError::Timeout);
        }
        dsp_update32(hw, dsp::HIPCCTL, dsp::HIPCCTL_DONE, 0);
        let size = hw.dsp_read32(self.host_box.offset);
        let result = if size < 8 {
            Err(IpcError::EmptyReply)
        } else if size as usize > reply.len() || size > self.host_box.size {
            Err(IpcError::ReplyTooLarge(size))
        } else {
            read_box(hw, self.host_box.offset, &mut reply[..size as usize]);
            Ok(size as usize)
        };
        dsp_update32(hw, dsp::HIPCIDA, dsp::HIPCIDA_DONE, dsp::HIPCIDA_DONE);
        dsp_update32(hw, dsp::HIPCCTL, dsp::HIPCCTL_DONE, dsp::HIPCCTL_DONE);
        result
    }

    /// Takes a pending DSP message, if any, copying up to `buf.len()` bytes of it.
    pub fn poll<H: Hw>(&mut self, hw: &mut H, buf: &mut [u8]) -> Result<Option<Notification>, IpcError> {
        let tdr = hw.dsp_read32(dsp::HIPCTDR);
        if tdr & dsp::HIPCTDR_BUSY == 0 {
            return Ok(None);
        }
        let extension = hw.dsp_read32(dsp::HIPCTDD) & dsp::HIPCTDD_MSG_MASK;
        if tdr & dsp::IPC_PANIC_MAGIC_MASK == dsp::IPC_PANIC_MAGIC {
            host_done(hw);
            return Err(IpcError::Panic { code: tdr, extension });
        }
        let size = hw.dsp_read32(self.dsp_box.offset).min(self.dsp_box.size) as usize;
        let len = size.min(buf.len());
        read_box(hw, self.dsp_box.offset, &mut buf[..len]);
        host_done(hw);
        Ok(Some(Notification { primary: tdr & dsp::HIPCTDR_MSG_MASK, extension, len }))
    }

    /// Waits up to `timeout_us` for a DSP message.
    pub fn wait<H: Hw>(&mut self, hw: &mut H, buf: &mut [u8], timeout_us: u32) -> Result<Notification, IpcError> {
        let mut waited = 0u32;
        loop {
            if let Some(n) = self.poll(hw, buf)? {
                return Ok(n);
            }
            if waited >= timeout_us {
                return Err(IpcError::Timeout);
            }
            hw.delay_us(timeout::STREAM_POLL_US);
            waited = waited.saturating_add(timeout::STREAM_POLL_US);
        }
    }
}
