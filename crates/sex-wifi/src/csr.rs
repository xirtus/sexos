// SPDX-License-Identifier: GPL-3.0-only
//! Control/status registers and the power-up sequence up to "firmware started".
//! Same order and values as OpenBSD iwx(4) (`iwx_start_hw`, `iwx_apm_init`,
//! `iwx_start_fw`, `iwx_ctxt_info_*_init`), written as a state machine that does
//! one bounded step per call, like the other SexOS drivers: the service keeps
//! answering IPC between steps and never sleeps inside the driver.
use crate::devices::{Chip, Family, LTR_NONE};
use crate::context::Layout;

pub const HW_IF_CONFIG: u32 = 0x000;
pub const INT_COALESCING: u32 = 0x004;
pub const INT: u32 = 0x008;
pub const INT_MASK: u32 = 0x00c;
pub const FH_INT_STATUS: u32 = 0x010;
pub const RESET: u32 = 0x020;
pub const GP_CNTRL: u32 = 0x024;
pub const HW_REV: u32 = 0x028;
pub const GIO: u32 = 0x03c;
pub const CTXT_INFO_BA: u32 = 0x040;
pub const UCODE_DRV_GP1_CLR: u32 = 0x05c;
pub const MBOX_SET: u32 = 0x088;
pub const HW_RF_ID: u32 = 0x09c;
pub const MAC_SHADOW_REG_CTRL: u32 = 0x0a8;
pub const LTR_LONG_VAL_AD: u32 = 0x0d4;
pub const GIO_CHICKEN_BITS: u32 = 0x100;
pub const CTXT_INFO_BOOT_CTRL: u32 = 0x000;
pub const CTXT_INFO_ADDR: u32 = 0x118;
pub const IML_DATA_ADDR: u32 = 0x120;
pub const IML_SIZE_ADDR: u32 = 0x128;
pub const DBG_HPET_MEM: u32 = 0x240;
pub const DBG_LINK_PWR_MGMT: u32 = 0x250;
pub const PRPH_WADDR: u32 = 0x444;
pub const PRPH_RADDR: u32 = 0x448;
pub const PRPH_WDAT: u32 = 0x44c;
pub const PRPH_RDAT: u32 = 0x450;
pub const TARG_WRPTR: u32 = 0x460;

pub const IF_NIC_READY: u32 = 0x0040_0000;
pub const IF_PREPARE: u32 = 0x0800_0000;
pub const IF_HAP_WAKE_L1A: u32 = 0x0008_0000;
pub const MBOX_OS_ALIVE: u32 = 0x20;
pub const RESET_SW: u32 = 0x80;
pub const RESET_LINK_PWR_MGMT_DISABLED: u32 = 0x8000_0000;
pub const GP_MAC_CLOCK_READY: u32 = 0x1;
pub const GP_INIT_DONE: u32 = 0x4;
pub const GP_MAC_ACCESS_REQ: u32 = 0x8;
pub const GP_GOING_TO_SLEEP: u32 = 0x10;
pub const GP_RFKILL: u32 = 0x0800_0000;
pub const GIO_L0S_DISABLED: u32 = 0x2;
pub const CHICKEN_L1A_NO_L0S_RX: u32 = 0x0080_0000;
pub const HPET_VAL: u32 = 0xffff_0000;
pub const AUTO_FUNC_BOOT_ENA: u32 = 0x2;
pub const UCODE_SW_BIT_RFKILL: u32 = 0x2;
pub const UCODE_CMD_BLOCKED: u32 = 0x4;
pub const INT_ALIVE: u32 = 1 << 0;
pub const INT_SW_ERR: u32 = 1 << 25;
pub const INT_HW_ERR: u32 = 1 << 29;
pub const INT_FH_RX: u32 = 1 << 31;
pub const INT_RF_KILL: u32 = 1 << 7;
pub const HOST_INT_TIMEOUT_DEF: u32 = 0x40;

pub const UREG_CPU_INIT_RUN: u32 = 0xa0_5c44;
pub const HPM_DEBUG: u32 = 0xa0_3440;
pub const HPM_MAC_LTR_CSR: u32 = 0xa0_348c;
pub const HPM_MAC_LRT_ENABLE_ALL: u32 = 0xf;
pub const HPM_UMAC_LTR: u32 = 0xa0_3480;
pub const HPM_HIPM_GEN_CFG: u32 = 0xa0_3458;
pub const PREG_PRPH_WPROT_22000: u32 = 0xa0_4d00;
const PERSISTENCE_BIT: u32 = 1 << 12;
const PREG_WFPM_ACCESS: u32 = 1 << 12;
const HIPM_PG_EN: u32 = 1 << 0;
const HIPM_SLP_EN: u32 = 1 << 1;
const HIPM_FORCE_ACTIVE: u32 = 1 << 10;

/// 32-bit access to BAR0, granted by the kernel's device lease.
pub trait Registers {
    fn read(&mut self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32);
    fn set(&mut self, offset: u32, bits: u32) { let v = self.read(offset); self.write(offset, v | bits); }
    fn clear(&mut self, offset: u32, bits: u32) { let v = self.read(offset); self.write(offset, v & !bits); }
}

fn prph_mask(family: Family) -> u32 { if family == Family::Ax210 { 0x00ff_ffff } else { 0x000f_ffff } }
pub fn read_prph(r: &mut impl Registers, family: Family, addr: u32) -> u32 {
    r.write(PRPH_RADDR, (addr & prph_mask(family)) | (3 << 24));
    r.read(PRPH_RDAT)
}
pub fn write_prph(r: &mut impl Registers, family: Family, addr: u32, value: u32) {
    r.write(PRPH_WADDR, (addr & prph_mask(family)) | (3 << 24));
    r.write(PRPH_WDAT, value);
}

/// Address of a UMAC peripheral register for this family.
pub fn umac_prph(family: Family, addr: u32) -> u32 { if family == Family::Ax210 { addr + 0x30_0000 } else { addr } }

/// The LTR value written before boot (~250 us), from `iwx_set_ltr`.
pub fn boot_ltr() -> u32 {
    const NO_SNOOP_REQ: u32 = 0x8000_0000;
    const SNOOP_REQ: u32 = 0x0000_8000;
    const SCALE_USEC: u32 = 2;
    NO_SNOOP_REQ | ((SCALE_USEC << 24) & 0x1c00_0000) | ((250 << 16) & 0x03ff_0000)
        | SNOOP_REQ | ((SCALE_USEC << 8) & 0x0000_1c00) | (250 & 0x3ff)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Ready, Prepare, Persistence, Reset, IntegratedClock, IntegratedReset, Clock, Configure,
    NicLock, WaitAlive, Alive, Failed(Failure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { NotReady, Persistence, ClockTimeout, NicLock, AliveTimeout, RfKill, Gone }

/// Power-up and firmware start. Times are microseconds from a monotonic clock.
pub struct Boot {
    pub chip: Chip,
    pub step: Step,
    pub hw_rev: u32,
    since: u64,
    started: u64,
    prepare_rounds: u8,
    /// The chip raised the ALIVE interrupt: its firmware has configured the receive
    /// hardware (RFH), so the driver must publish the free ring again.
    pub alive_interrupts: u32,
}

const READY_US: u64 = 150_000;
const RESET_US: u64 = 5_000;
const CLOCK_US: u64 = 25_000;
const LOCK_US: u64 = 150_000;
const ALIVE_US: u64 = 1_000_000;

impl Boot {
    pub fn new(chip: Chip, now: u64) -> Self {
        Self { chip, step: Step::Ready, hw_rev: 0, since: now, started: now, prepare_rounds: 0, alive_interrupts: 0 }
    }
    pub fn done(&self) -> bool { matches!(self.step, Step::Alive | Step::Failed(_)) }
    fn fail(&mut self, f: Failure) { self.step = Step::Failed(f); }
    fn go(&mut self, step: Step, now: u64) { self.step = step; self.since = now; }

    /// One step. `layout`/`base`/`iml_len` describe the DMA memory already filled by
    /// `Layout::write`. Returns true while more steps are needed.
    pub fn advance(&mut self, r: &mut impl Registers, now: u64, layout: &Layout, base: u64, iml_len: u32) -> bool {
        if self.done() { return false; }
        let family = self.chip.family;
        let waited = now.saturating_sub(self.since);
        match self.step {
            Step::Ready => {
                if r.read(HW_IF_CONFIG) == u32::MAX { self.fail(Failure::Gone); return false; }
                r.set(HW_IF_CONFIG, IF_NIC_READY);
                if r.read(HW_IF_CONFIG) & IF_NIC_READY != 0 {
                    r.set(MBOX_SET, MBOX_OS_ALIVE);
                    let next = if family == Family::Gen22000 { Step::Persistence } else { Step::Reset };
                    if next == Step::Reset { r.set(RESET, RESET_SW); }
                    self.go(next, now);
                } else {
                    r.set(DBG_LINK_PWR_MGMT, RESET_LINK_PWR_MGMT_DISABLED);
                    self.go(Step::Prepare, now);
                }
            }
            Step::Prepare => {
                r.set(HW_IF_CONFIG, IF_PREPARE | IF_NIC_READY);
                if r.read(HW_IF_CONFIG) & IF_NIC_READY != 0 {
                    r.set(MBOX_SET, MBOX_OS_ALIVE);
                    let next = if family == Family::Gen22000 { Step::Persistence } else { Step::Reset };
                    if next == Step::Reset { r.set(RESET, RESET_SW); }
                    self.go(next, now);
                } else if waited >= READY_US {
                    self.prepare_rounds += 1;
                    if self.prepare_rounds >= 10 { self.fail(Failure::NotReady); } else { self.since = now; }
                }
            }
            Step::Persistence => {
                let hpm = read_prph(r, family, HPM_DEBUG);
                if hpm != 0xa5a5_a5a0 && hpm & PERSISTENCE_BIT != 0 {
                    if read_prph(r, family, PREG_PRPH_WPROT_22000) & PREG_WFPM_ACCESS != 0 { self.fail(Failure::Persistence); return false; }
                    write_prph(r, family, HPM_DEBUG, hpm & !PERSISTENCE_BIT);
                }
                r.set(RESET, RESET_SW);
                self.go(Step::Reset, now);
            }
            Step::Reset if waited >= RESET_US => {
                if family == Family::Gen22000 && self.chip.integrated && self.prepare_rounds < 100 {
                    r.set(GP_CNTRL, GP_INIT_DONE);
                    self.prepare_rounds = 100; // integrated parts: one extra power-gating reset
                    self.go(Step::IntegratedClock, now);
                } else {
                    self.apm_init(r);
                    self.go(Step::Clock, now);
                }
            }
            Step::IntegratedClock => {
                if r.read(GP_CNTRL) & GP_MAC_CLOCK_READY != 0 {
                    for (set, clear) in [(HIPM_FORCE_ACTIVE, 0), (HIPM_PG_EN | HIPM_SLP_EN, 0), (0, HIPM_FORCE_ACTIVE)] {
                        let v = read_prph(r, family, HPM_HIPM_GEN_CFG);
                        write_prph(r, family, HPM_HIPM_GEN_CFG, (v & !clear) | set);
                    }
                    r.set(RESET, RESET_SW);
                    self.go(Step::IntegratedReset, now);
                } else if waited >= CLOCK_US { self.fail(Failure::ClockTimeout); }
            }
            Step::IntegratedReset if waited >= RESET_US => { self.apm_init(r); self.go(Step::Clock, now); }
            Step::Clock => {
                if r.read(GP_CNTRL) & GP_MAC_CLOCK_READY != 0 { self.go(Step::Configure, now); }
                else if waited >= CLOCK_US { self.fail(Failure::ClockTimeout); }
            }
            Step::Configure => {
                self.hw_rev = r.read(HW_REV);
                // iwx_start_fw: quiet interrupts, clear the RF-kill handshake, start the NIC.
                r.write(INT, u32::MAX);
                r.write(INT_MASK, 0);
                r.write(FH_INT_STATUS, u32::MAX);
                r.write(UCODE_DRV_GP1_CLR, UCODE_SW_BIT_RFKILL);
                r.write(UCODE_DRV_GP1_CLR, UCODE_CMD_BLOCKED);
                r.write(INT, u32::MAX);
                r.write(INT_COALESCING, HOST_INT_TIMEOUT_DEF);
                r.set(MAC_SHADOW_REG_CTRL, 0x800f_ffff);
                if r.read(GP_CNTRL) & GP_RFKILL == 0 {
                    // The hardware radio switch is off (bit clear = killed).
                    self.fail(Failure::RfKill);
                    return false;
                }
                // Tell the chip where the context info is.
                let ctx = base + layout.context.offset;
                if family == Family::Ax210 {
                    r.write(CTXT_INFO_ADDR, ctx as u32);
                    r.write(CTXT_INFO_ADDR + 4, (ctx >> 32) as u32);
                    let iml = base + layout.iml.offset;
                    r.write(IML_DATA_ADDR, iml as u32);
                    r.write(IML_DATA_ADDR + 4, (iml >> 32) as u32);
                    r.write(IML_SIZE_ADDR, iml_len);
                    r.set(CTXT_INFO_BOOT_CTRL, AUTO_FUNC_BOOT_ENA);
                } else {
                    r.write(CTXT_INFO_BA, ctx as u32);
                    r.write(CTXT_INFO_BA + 4, (ctx >> 32) as u32);
                }
                r.set(GP_CNTRL, GP_MAC_ACCESS_REQ);
                self.go(Step::NicLock, now);
            }
            Step::NicLock => {
                let v = r.read(GP_CNTRL);
                if v & (GP_MAC_CLOCK_READY | GP_GOING_TO_SLEEP) == GP_MAC_CLOCK_READY {
                    self.set_ltr(r);
                    // UMAC registers sit 0x300000 higher on AX210-class chips (iwx_write_umac_prph).
                    write_prph(r, family, umac_prph(family, UREG_CPU_INIT_RUN), 1);
                    r.clear(GP_CNTRL, GP_MAC_ACCESS_REQ);
                    // Unmask ALIVE (and errors) so the firmware can report in.
                    r.write(INT_MASK, INT_ALIVE | INT_SW_ERR | INT_HW_ERR | INT_FH_RX | INT_RF_KILL);
                    self.go(Step::WaitAlive, now);
                } else if waited >= LOCK_US { self.fail(Failure::NicLock); }
            }
            Step::WaitAlive => {
                let cause = r.read(INT);
                if cause & (INT_SW_ERR | INT_HW_ERR) != 0 { self.fail(Failure::AliveTimeout); }
                else if cause & (INT_ALIVE | INT_FH_RX) != 0 {
                    // The ALIVE notification itself arrives on the RX ring; the queue
                    // layer confirms it with `alive_received`.
                    if cause & INT_ALIVE != 0 { self.alive_interrupts = self.alive_interrupts.saturating_add(1); }
                    r.write(INT, cause);
                } else if now.saturating_sub(self.since) >= ALIVE_US { self.fail(Failure::AliveTimeout); }
            }
            _ => {}
        }
        let _ = self.started;
        !self.done()
    }

    /// Called by the RX path when the firmware's ALIVE notification says OK.
    pub fn alive_received(&mut self, ok: bool) {
        if self.step == Step::WaitAlive { self.step = if ok { Step::Alive } else { Step::Failed(Failure::AliveTimeout) }; }
    }

    fn apm_init(&self, r: &mut impl Registers) {
        r.set(GIO_CHICKEN_BITS, CHICKEN_L1A_NO_L0S_RX);
        r.set(DBG_HPET_MEM, HPET_VAL);
        r.set(HW_IF_CONFIG, IF_HAP_WAKE_L1A);
        r.set(GIO, GIO_L0S_DISABLED);
        r.set(GP_CNTRL, GP_INIT_DONE);
    }

    fn set_ltr(&self, r: &mut impl Registers) {
        let ltr = boot_ltr();
        if !self.chip.integrated {
            r.write(LTR_LONG_VAL_AD, ltr);
        } else if self.chip.family == Family::Gen22000 {
            write_prph(r, self.chip.family, HPM_MAC_LTR_CSR, HPM_MAC_LRT_ENABLE_ALL);
            write_prph(r, self.chip.family, HPM_UMAC_LTR, ltr);
        }
        let _ = LTR_NONE;
    }
}
