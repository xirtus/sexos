// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The GSP-RM boot, one step at a time, so the service can log each step and
//! a boot photo of the screen is enough to see where real hardware stopped.

use crate::falcon::{GSP, SEC2};
use crate::fw::{self, FalconUcode};
use crate::layout::{self, FbLayout, SysmemRefs};
use crate::msgq::{self, Queues};
use crate::regs;
use crate::rpc::{self, SystemInfo};
use crate::vbios::{FwsecCommand, Vbios};
use crate::{chip::Chip, libos, radix3, sequencer, Clock, Dma, Error, Mmio};

/// Firmware files (linux-firmware `nvidia/ga102/gsp/*-535.113.01.bin`).
pub struct Firmware<'a> { pub booter_load: &'a [u8], pub bootloader: &'a [u8], pub gsp: &'a [u8] }

pub const FW_VERSION: &str = "535.113.01";

/// GPU-visible buffers the boot fills. Sizes come from [`Sizes::for_firmware`].
pub struct Memory<'a> {
    /// FWSEC, staged and patched (falcon DMA source).
    pub fwsec: Dma<'a>,
    /// Booter, staged and patched.
    pub booter: Dma<'a>,
    /// GSP-RM's `.fwimage`, copied page by page; Booter reads it through radix3.
    pub rm_image: Dma<'a>,
    pub radix3_l0: Dma<'a>,
    pub radix3_l1: Dma<'a>,
    pub radix3_l2: Dma<'a>,
    pub signature: Dma<'a>,
    pub bootloader: Dma<'a>,
    pub wpr_meta: Dma<'a>,
    pub libos: Dma<'a>,
    pub loginit: Dma<'a>,
    pub logintr: Dma<'a>,
    pub logrm: Dma<'a>,
    pub rmargs: Dma<'a>,
    pub shm: Dma<'a>,
    /// CPU-only scratch for assembling RPCs and receiving messages (>= 64 KiB).
    pub scratch: &'a mut [u8],
}

/// How big each buffer in [`Memory`] must be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sizes {
    pub booter: usize,
    pub rm_image: usize,
    pub radix3_l2: usize,
    pub signature: usize,
    pub bootloader: usize,
    pub shm: usize,
    /// FWSEC comes from the VBIOS; 256 KiB covers every Ampere/Ada FWSEC seen.
    pub fwsec_max: usize,
}

impl Sizes {
    pub fn for_firmware(chip: &Chip, f: &Firmware) -> Result<Sizes, Error> {
        let booter = fw::booter(f.booter_load)?;
        let (_, bl) = fw::bootloader(f.bootloader)?;
        let rm = fw::gsp_rm(f.gsp, chip.signature_section())?;
        Ok(Sizes {
            booter: page(booter.staged_len()),
            rm_image: page(rm.image.len()),
            radix3_l2: radix3::lvl2_len(rm.image.len()),
            signature: page(rm.signature.len()),
            bootloader: page(bl.len()),
            shm: msgq::shared_layout().total,
            fwsec_max: 256 * 1024,
        })
    }
    /// Total GPU-visible bytes, including the fixed small buffers.
    pub fn total(&self) -> usize {
        self.fwsec_max + self.booter + self.rm_image + 2 * 4096 + self.radix3_l2 + self.signature + self.bootloader
            + 4096 /* meta */ + 4096 /* libos */ + 3 * libos::LOG_BUFFER_SIZE + libos::RMARGS_SIZE + self.shm
    }
}

fn page(n: usize) -> usize { n.div_ceil(4096) * 4096 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Chip, Vbios, Layout, Prepare, Fwsec, Riscv, Booter, InitDone }

impl Step {
    pub fn name(self) -> &'static str {
        match self { Step::Chip => "chip", Step::Vbios => "vbios", Step::Layout => "layout", Step::Prepare => "prepare", Step::Fwsec => "fwsec_frts", Step::Riscv => "riscv", Step::Booter => "booter", Step::InitDone => "init_done" }
    }
}

/// What the boot learned, for the log and for later stages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub boot0: u32,
    pub chipset: u16,
    pub fb_size: u64,
    pub vbios_version: [u8; 5],
    pub vbios_len: usize,
    pub layout: FbLayout,
    pub fwsec_sig: u32,
    pub booter_sig: u32,
    pub wpr2_lo: u32,
    pub wpr2_hi: u32,
    pub app_version: u32,
    pub events: u32,
    pub sequencer_runs: u32,
    pub init_done: bool,
}

/// A step failed: which, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed { pub step: Step, pub error: Error }

/// Boot settings that are not in the firmware.
pub struct Config {
    pub system: SystemInfo,
    /// Polls of the message queue while waiting for `GSP_INIT_DONE` (10 us each).
    pub init_polls: u32,
}

/// One line per finished step, for the service's log.
pub trait Log { fn step(&mut self, step: Step, r: &Report); }
impl<F: FnMut(Step, &Report)> Log for F { fn step(&mut self, step: Step, r: &Report) { self(step, r) } }

/// Run the whole boot. `vbios` receives the ROM (1 MiB is always enough).
pub fn boot(m: &mut impl Mmio, c: &mut impl Clock, cfg: &Config, f: &Firmware, vbios: &mut [u8], mem: &mut Memory, log: &mut impl Log) -> Result<Report, Failed> {
    let mut r = Report::default();
    let fail = |step| move |error| Failed { step, error };

    // 1. Chip and memory.
    let boot0 = m.read(regs::PMC_BOOT_0);
    let chip = Chip::decode(boot0).ok_or(Failed { step: Step::Chip, error: Error::UnsupportedChip(boot0) })?;
    if !chip.gsp_boot_supported() { return Err(Failed { step: Step::Chip, error: Error::UnsupportedChip(boot0) }); }
    r.boot0 = boot0;
    r.chipset = chip.chipset;
    r.fb_size = (m.read(regs::PFB_VIDMEM_SIZE_MB) as u64) << 20;
    let wpr2_hi = m.read(regs::PFB_WPR2_ADDR_HI);
    if wpr2_hi != 0 { return Err(Failed { step: Step::Chip, error: Error::Wpr2Busy(wpr2_hi) }); }
    log.step(Step::Chip, &r);

    // 2. VBIOS and FWSEC.
    let n = crate::vbios::read_prom(m, vbios).map_err(fail(Step::Vbios))?;
    r.vbios_len = n;
    let vb = Vbios::parse(&vbios[..n]).map_err(fail(Step::Vbios))?;
    r.vbios_version = vb.version().unwrap_or_default();
    let fwsec = vb.fwsec().map_err(fail(Step::Vbios))?;
    log.step(Step::Vbios, &r);

    // 3. Firmware files and the VRAM layout.
    let booter = fw::booter(f.booter_load).map_err(fail(Step::Layout))?;
    let (bl_desc, bl) = fw::bootloader(f.bootloader).map_err(fail(Step::Layout))?;
    let rm = fw::gsp_rm(f.gsp, chip.signature_section()).map_err(fail(Step::Layout))?;
    r.app_version = bl_desc.app_version;
    let vga = Some(m.read(regs::PDISP_VGA_WORKSPACE_BASE));
    let l = FbLayout::compute(r.fb_size, vga, bl.len() as u64, rm.image.len() as u64).map_err(fail(Step::Layout))?;
    r.layout = l;
    log.step(Step::Layout, &r);

    // 4. System memory: GSP-RM image + radix3, signature, bootloader, meta,
    //    LibOS arguments and logs, RM arguments, queues and first RPCs.
    prepare(m, &l, &booter, &bl_desc, bl, &rm, cfg, mem, &mut r).map_err(fail(Step::Prepare))?;
    log.step(Step::Prepare, &r);

    // 5. FWSEC-FRTS on the GSP falcon carves WPR2.
    let fuse = fwsec.ucode.fuse_register().ok_or(Failed { step: Step::Fwsec, error: Error::NoSignature("fwsec: unknown engine") })?;
    let idx = fwsec.ucode.signature_index(m.read(fuse)).map_err(fail(Step::Fwsec))?;
    r.fwsec_sig = idx;
    let need = fwsec.ucode.staged_len();
    if mem.fwsec.len() < need { return Err(Failed { step: Step::Fwsec, error: Error::BufferTooSmall { need, have: mem.fwsec.len() } }); }
    fwsec.ucode.stage(mem.fwsec.cpu, idx).map_err(fail(Step::Fwsec))?;
    fwsec.patch_command(mem.fwsec.cpu, FwsecCommand::Frts { addr: l.frts.addr, size: l.frts.size }).map_err(fail(Step::Fwsec))?;
    let (a, b) = GSP.run_hs(m, c, mem.fwsec.bus, &fwsec.ucode, 0, None).map_err(fail(Step::Fwsec))?;
    let err = m.read(regs::FRTS_ERR_SCRATCH) >> 16;
    if err != 0 { return Err(Failed { step: Step::Fwsec, error: Error::FalconFailed { falcon: "fwsec-frts", mbox0: err, mbox1: b.max(a) } }); }
    r.wpr2_lo = m.read(regs::PFB_WPR2_ADDR_LO);
    r.wpr2_hi = m.read(regs::PFB_WPR2_ADDR_HI);
    if r.wpr2_hi == 0 { return Err(Failed { step: Step::Fwsec, error: Error::Gsp("fwsec-frts ran but WPR2 is not set") }); }
    log.step(Step::Fwsec, &r);

    // 6. GSP into RISC-V mode, pointed at the LibOS arguments.
    GSP.reset_riscv(m, c).map_err(fail(Step::Riscv))?;
    GSP.wr(m, regs::falcon::MAILBOX0, mem.libos.bus as u32);
    GSP.wr(m, regs::falcon::MAILBOX1, (mem.libos.bus >> 32) as u32);
    log.step(Step::Riscv, &r);

    // 7. Booter on SEC2 loads GSP-RM into WPR2 and starts it.
    let fuse = booter.fuse_register().ok_or(Failed { step: Step::Booter, error: Error::NoSignature("booter: unknown engine") })?;
    let idx = booter.signature_index(m.read(fuse)).map_err(fail(Step::Booter))?;
    r.booter_sig = idx;
    booter.stage(mem.booter.cpu, idx).map_err(fail(Step::Booter))?;
    let meta = mem.wpr_meta.bus;
    SEC2.run_hs(m, c, mem.booter.bus, &booter, meta as u32, Some((meta >> 32) as u32)).map_err(fail(Step::Booter))?;
    GSP.wr(m, regs::falcon::OS, bl_desc.app_version);
    if !GSP.riscv_active(m) { return Err(Failed { step: Step::Booter, error: Error::Gsp("GSP RISC-V core not running after Booter") }); }
    log.step(Step::Booter, &r);

    // 8. Wait for GSP_INIT_DONE, serving the CPU sequencer meanwhile.
    wait_init_done(m, c, cfg, mem, &mut r, bl_desc.app_version).map_err(fail(Step::InitDone))?;
    log.step(Step::InitDone, &r);
    Ok(r)
}

#[allow(clippy::too_many_arguments)]
fn prepare(m: &mut impl Mmio, l: &FbLayout, _booter: &FalconUcode, bl_desc: &fw::RiscvDesc, bl: &[u8], rm: &fw::GspRm, cfg: &Config, mem: &mut Memory, _r: &mut Report) -> Result<(), Error> {
    let fit = |d: &Dma, n: usize| if d.len() < n { Err(Error::BufferTooSmall { need: n, have: d.len() }) } else { Ok(()) };
    fit(&mem.rm_image, rm.image.len())?;
    mem.rm_image.cpu[..rm.image.len()].copy_from_slice(rm.image);
    let base = mem.rm_image.bus;
    radix3::build(&mut mem.radix3_l0, &mut mem.radix3_l1, &mut mem.radix3_l2, rm.image.len(), |i| base + (i * radix3::PAGE) as u64)?;
    fit(&mem.signature, rm.signature.len())?;
    mem.signature.cpu.fill(0);
    mem.signature.cpu[..rm.signature.len()].copy_from_slice(rm.signature);
    fit(&mem.bootloader, bl.len())?;
    mem.bootloader.cpu[..bl.len()].copy_from_slice(bl);
    fit(&mem.wpr_meta, layout::WPR_META_SIZE)?;
    let refs = SysmemRefs {
        radix3_elf: mem.radix3_l0.bus,
        bootloader: mem.bootloader.bus,
        bootloader_size: bl.len() as u64,
        bootloader_code_offset: bl_desc.monitor_code_offset as u64,
        bootloader_data_offset: bl_desc.monitor_data_offset as u64,
        bootloader_manifest_offset: bl_desc.manifest_offset as u64,
        signature: mem.signature.bus,
        signature_size: rm.signature.len() as u64,
    };
    layout::write_wpr_meta(mem.wpr_meta.cpu, l, &refs);

    // LibOS: three log buffers and the RM arguments.
    for d in [&mut mem.loginit, &mut mem.logintr, &mut mem.logrm] { fit(d, libos::LOG_BUFFER_SIZE)?; libos::init_log_buffer(d); }
    fit(&mem.libos, 4 * libos::ARG_SIZE)?;
    mem.libos.cpu.fill(0);
    libos::write_region(mem.libos.cpu, 0, "LOGINIT", mem.loginit.bus, mem.loginit.len() as u64);
    libos::write_region(mem.libos.cpu, 1, "LOGINTR", mem.logintr.bus, mem.logintr.len() as u64);
    libos::write_region(mem.libos.cpu, 2, "LOGRM", mem.logrm.bus, mem.logrm.len() as u64);
    let sl = msgq::init_shared(&mut mem.shm)?;
    fit(&mem.rmargs, libos::RMARGS_SIZE)?;
    libos::write_rmargs(mem.rmargs.cpu, mem.shm.bus, sl.pte_count, sl.cmdq_off as u64, sl.msgq_off as u64);
    libos::write_region(mem.libos.cpu, 3, "RMARGS", mem.rmargs.bus, mem.rmargs.len() as u64);

    // First RPCs, read by GSP-RM as it starts.
    let mut q = Queues::new(sl);
    let (scratch, payload) = mem.scratch.split_at_mut(msgq::MAX_MSG.min(mem.scratch.len() / 2));
    let mut info = [0u8; rpc::SYSTEM_INFO_SIZE];
    rpc::system_info(&mut info, &cfg.system);
    q.push(mem.shm.cpu, scratch, rpc::FN_GSP_SET_SYSTEM_INFO, &info)?;
    let n = rpc::registry(payload, rpc::DEFAULT_REGISTRY)?;
    q.push(mem.shm.cpu, scratch, rpc::FN_SET_REGISTRY, &payload[..n])?;
    GSP.wr(m, regs::falcon::QUEUE_HEAD0, 0);
    Ok(())
}

fn wait_init_done(m: &mut impl Mmio, c: &mut impl Clock, cfg: &Config, mem: &mut Memory, r: &mut Report, app_version: u32) -> Result<(), Error> {
    let mut q = Queues::new(msgq::shared_layout());
    let resume = sequencer::Resume { libos_bus: mem.libos.bus, app_version };
    let mut saved = [0u32; 8];
    for _ in 0..cfg.init_polls.max(1) {
        match q.recv(mem.shm.cpu, mem.scratch)? {
            Some(msg) => {
                r.events += 1;
                match msg.function {
                    rpc::EV_GSP_INIT_DONE => { r.init_done = true; return Ok(()); }
                    rpc::EV_GSP_RUN_CPU_SEQUENCER => {
                        sequencer::run(m, c, &mem.scratch[..msg.len], &resume, &mut saved)?;
                        r.sequencer_runs += 1;
                    }
                    _ => {} // logs, user shared data, perf updates: not needed yet
                }
            }
            None => c.delay_us(10),
        }
    }
    Err(Error::Gsp("timed out waiting for GSP_INIT_DONE"))
}
