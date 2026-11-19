// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Falcon microcontrollers (GSP and SEC2 on GA10x): reset, DMA load, and the
//! heavy-secure boot through the boot ROM (BROM), which checks the patched
//! RSA signature before it lets the ucode run.

use crate::fw::FalconUcode;
use crate::regs::falcon::*;
use crate::{wait_for, Clock, Error, Mmio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Falcon { pub base: u32, pub name: &'static str }

pub const GSP: Falcon = Falcon { base: crate::regs::GSP_BASE, name: "gsp" };
pub const SEC2: Falcon = Falcon { base: crate::regs::SEC2_BASE, name: "sec2" };

/// Generous bounds: 2 s for a ucode run, 20 ms for scrubbing (nouveau's values).
const RUN_POLLS: u32 = 200_000;
const RUN_STEP_US: u32 = 10;

impl Falcon {
    pub fn rd(&self, m: &mut impl Mmio, r: u32) -> u32 { m.read(self.base + r) }
    pub fn wr(&self, m: &mut impl Mmio, r: u32, v: u32) { m.write(self.base + r, v) }
    pub fn mask(&self, m: &mut impl Mmio, r: u32, clear: u32, set: u32) { m.modify(self.base + r, clear, set); }
    pub fn rd2(&self, m: &mut impl Mmio, r: u32) -> u32 { m.read(self.base + ADDR2 + r) }
    pub fn wr2(&self, m: &mut impl Mmio, r: u32, v: u32) { m.write(self.base + ADDR2 + r, v) }

    /// Wait (best effort, up to 150 us) for the engine to say it may be reset.
    fn reset_prep(&self, m: &mut impl Mmio, c: &mut impl Clock) {
        self.rd(m, HWCFG2);
        let _ = wait_for(m, c, self.base + HWCFG2, HWCFG2_RESET_READY, HWCFG2_RESET_READY, 150, 1);
    }

    fn wait_scrubbing(&self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<(), Error> {
        wait_for(m, c, self.base + HWCFG2, HWCFG2_MEM_SCRUBBING, 0, 2_000, 10).map(|_| ())
    }

    /// Pulse the engine reset and wait for IMEM/DMEM scrubbing to end.
    pub fn reset_engine(&self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<(), Error> {
        self.reset_prep(m, c);
        self.mask(m, ENGINE_RESET, 0, 1);
        c.delay_us(10);
        self.mask(m, ENGINE_RESET, 1, 0);
        self.wait_scrubbing(m, c)
    }

    /// Make sure the Falcon core (not the RISC-V core) is selected.
    pub fn select_falcon(&self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<(), Error> {
        if self.rd2(m, RISCV_BCR_CTRL) & 0x10 != 0 {
            self.wr2(m, RISCV_BCR_CTRL, 0);
            wait_for(m, c, self.base + ADDR2 + RISCV_BCR_CTRL, 1, 1, 1_000, 10)?;
        }
        Ok(())
    }

    /// `nvkm_falcon_reset`: disable (interrupts off, engine reset) then enable.
    pub fn reset(&self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<(), Error> {
        self.select_falcon(m, c)?;
        self.mask(m, ITFEN, 3, 0);
        self.wr(m, IRQMCLR, 0xffff_ffff);
        self.reset_engine(m, c)?;
        self.reset_engine(m, c)?;
        self.select_falcon(m, c)?;
        self.wait_scrubbing(m, c)?;
        let boot0 = m.read(crate::regs::PMC_BOOT_0);
        self.wr(m, RM, boot0);
        Ok(())
    }

    /// Reset the GSP falcon into RISC-V mode (`ga102_gsp_reset`).
    pub fn reset_riscv(&self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<(), Error> {
        self.reset_engine(m, c)?;
        self.mask(m, ADDR2 + RISCV_BCR_CTRL, 0, 0x111);
        Ok(())
    }

    pub fn riscv_active(&self, m: &mut impl Mmio) -> bool { self.rd2(m, RISCV_CPUCTL) & RISCV_CPUCTL_ACTIVE != 0 }

    /// DMA a staged ucode image (at bus address `bus`, 256-byte aligned) into
    /// IMEM (secure) and DMEM.
    pub fn dma_load(&self, m: &mut impl Mmio, c: &mut impl Clock, bus: u64, u: &FalconUcode) -> Result<(), Error> {
        self.mask(m, FBIF_CTL, 0, 0x80);
        self.wr(m, DMACTL, 0);
        // Context 0: physical addressing, coherent system memory.
        self.mask(m, FBIF_TRANSCFG0, 0x0001_0007, (1 << 2) | 1);
        self.dma_wr(m, c, bus, u.imem_img_off, 0, u.imem_base, u.imem_size, true, true)?;
        self.dma_wr(m, c, bus, u.dmem_img_off, u.dmem_img_off, u.dmem_base, u.dmem_size, false, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn dma_wr(&self, m: &mut impl Mmio, c: &mut impl Clock, bus: u64, src: u32, dma_start: u32, mem_base: u32, len: u32, imem: bool, sec: bool) -> Result<(), Error> {
        const CHUNK: u32 = 256;
        if len == 0 || len % CHUNK != 0 { return Err(Error::BadFirmware("falcon: DMA length not a multiple of 256")); }
        let base = bus + dma_start as u64;
        if base & 0xff != 0 { return Err(Error::BadFirmware("falcon: DMA base not 256-byte aligned")); }
        let mut cmd = (CHUNK.trailing_zeros() - 2) << 8;
        if imem { cmd |= 0x10; }
        if sec { cmd |= 0x04; }
        self.wr(m, DMATRFBASE, (base >> 8) as u32);
        self.wr(m, DMATRFBASE1, (base >> 40) as u32);
        let mut off = 0;
        while off < len {
            self.wr(m, DMATRFMOFFS, mem_base + off);
            self.wr(m, DMATRFFBOFFS, src + off - dma_start);
            self.wr(m, DMATRFCMD, cmd);
            wait_for(m, c, self.base + DMATRFCMD, DMATRFCMD_IDLE, DMATRFCMD_IDLE, RUN_POLLS, RUN_STEP_US)?;
            off += CHUNK;
        }
        Ok(())
    }

    /// Tell the boot ROM where the signature is and which ucode this is, start
    /// the CPU, wait for it to halt; returns the two mailboxes.
    pub fn boot_brom(&self, m: &mut impl Mmio, c: &mut impl Clock, u: &FalconUcode, mbox0: u32, mbox1: Option<u32>) -> Result<(u32, u32), Error> {
        self.wr2(m, BROM_PARAADDR0, u.dmem_sign);
        self.wr2(m, BROM_ENGIDMASK, u.engine_id);
        self.wr2(m, BROM_CURR_UCODE_ID, u.ucode_id);
        self.wr2(m, BROM_MOD_SEL, 1);
        self.start(m, c, u.boot_addr, mbox0, mbox1)
    }

    pub fn start(&self, m: &mut impl Mmio, c: &mut impl Clock, boot_addr: u32, mbox0: u32, mbox1: Option<u32>) -> Result<(u32, u32), Error> {
        self.wr(m, MAILBOX0, mbox0);
        if let Some(v) = mbox1 { self.wr(m, MAILBOX1, v); }
        self.wr(m, BOOTVEC, boot_addr);
        self.wr(m, CPUCTL, CPUCTL_STARTCPU);
        let r = wait_for(m, c, self.base + CPUCTL, CPUCTL_HALTED, CPUCTL_HALTED, RUN_POLLS, RUN_STEP_US);
        let mb = (self.rd(m, MAILBOX0), self.rd(m, MAILBOX1));
        r.map(|_| mb)
    }

    /// The whole heavy-secure run: reset, load, boot; `Ok` only when mailbox 0
    /// comes back zero.
    pub fn run_hs(&self, m: &mut impl Mmio, c: &mut impl Clock, bus: u64, u: &FalconUcode, mbox0: u32, mbox1: Option<u32>) -> Result<(u32, u32), Error> {
        self.reset(m, c)?;
        self.dma_load(m, c, bus, u)?;
        let (a, b) = self.boot_brom(m, c, u, mbox0, mbox1)?;
        if a != 0 { return Err(Error::FalconFailed { falcon: self.name, mbox0: a, mbox1: b }); }
        Ok((a, b))
    }
}
