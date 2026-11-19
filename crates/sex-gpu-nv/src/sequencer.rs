// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! GSP-RM's CPU sequencer: small register scripts it asks the driver to run
//! (`GSP_RUN_CPU_SEQUENCER`), e.g. to reset or resume its own core.

use crate::falcon::{GSP, SEC2};
use crate::regs::falcon::*;
use crate::{rd32, wait_for, Clock, Error, Mmio};

pub const OP_REG_WRITE: u32 = 0;
pub const OP_REG_MODIFY: u32 = 1;
pub const OP_REG_POLL: u32 = 2;
pub const OP_DELAY_US: u32 = 3;
pub const OP_REG_STORE: u32 = 4;
pub const OP_CORE_RESET: u32 = 5;
pub const OP_CORE_START: u32 = 6;
pub const OP_CORE_WAIT_FOR_HALT: u32 = 7;
pub const OP_CORE_RESUME: u32 = 8;

fn payload_words(op: u32) -> Option<usize> {
    Some(match op { OP_REG_WRITE => 2, OP_REG_MODIFY => 3, OP_REG_POLL => 5, OP_DELAY_US => 1, OP_REG_STORE => 2, 5..=8 => 0, _ => return None })
}

/// What the driver must supply for CORE_RESUME.
pub struct Resume { pub libos_bus: u64, pub app_version: u32 }

/// Run the script in a `rpc_run_cpu_sequencer_v17_00` payload. Returns the
/// number of commands run. `saved` receives REG_STORE values.
pub fn run(m: &mut impl Mmio, c: &mut impl Clock, payload: &[u8], resume: &Resume, saved: &mut [u32; 8]) -> Result<u32, Error> {
    let bad = Error::Gsp;
    let words = rd32(payload, 4).ok_or(bad("seq: short header"))? as usize; // cmdIndex
    let buf = 40; // after bufferSizeDWord, cmdIndex, regSaveArea[8]
    let w = |i: usize| rd32(payload, buf + i * 4).ok_or(bad("seq: command outside payload"));
    let mut p = 0usize;
    let mut n = 0;
    while p < words {
        let op = w(p)?;
        let len = payload_words(op).ok_or(bad("seq: unknown opcode"))?;
        let a = |k: usize| w(p + 1 + k);
        match op {
            OP_REG_WRITE => m.write(a(0)?, a(1)?),
            OP_REG_MODIFY => { let (r, mask, v) = (a(0)?, a(1)?, a(2)?); m.modify(r, mask, v); }
            OP_REG_POLL => {
                let (r, mask, v, t) = (a(0)?, a(1)?, a(2)?, a(3)?);
                let us = if t == 0 { 4_000_000 } else { t };
                m.read(r);
                let _ = wait_for(m, c, r, mask, v, us.div_ceil(10).max(1), 10);
            }
            OP_DELAY_US => c.delay_us(a(0)?),
            OP_REG_STORE => { let (r, slot) = (a(0)?, a(1)? as usize); if let Some(s) = saved.get_mut(slot) { *s = m.read(r); } }
            OP_CORE_RESET => {
                GSP.reset(m, c)?;
                GSP.mask(m, FBIF_CTL, 0, 0x80);
                GSP.wr(m, DMACTL, 0);
            }
            OP_CORE_START => {
                if GSP.rd(m, CPUCTL) & CPUCTL_ALIAS_EN != 0 { GSP.wr(m, CPUCTL_ALIAS, CPUCTL_STARTCPU); } else { GSP.wr(m, CPUCTL, CPUCTL_STARTCPU); }
            }
            OP_CORE_WAIT_FOR_HALT => { let _ = wait_for(m, c, GSP.base + CPUCTL, CPUCTL_HALTED, CPUCTL_HALTED, 200_000, 10); }
            OP_CORE_RESUME => {
                GSP.reset_riscv(m, c)?;
                GSP.wr(m, MAILBOX0, resume.libos_bus as u32);
                GSP.wr(m, MAILBOX1, (resume.libos_bus >> 32) as u32);
                // Start SEC2 (still holding Booter), wait for its resume handshake.
                if SEC2.rd(m, CPUCTL) & CPUCTL_ALIAS_EN != 0 { SEC2.wr(m, CPUCTL_ALIAS, CPUCTL_STARTCPU); } else { SEC2.wr(m, CPUCTL, CPUCTL_STARTCPU); }
                wait_for(m, c, crate::regs::SEC2_RESUME_STATUS, 0x0400_0000, 0x0400_0000, 200_000, 10)?;
                let mb = SEC2.rd(m, MAILBOX0);
                if mb != 0 { return Err(Error::FalconFailed { falcon: "sec2-resume", mbox0: mb, mbox1: 0 }); }
                GSP.wr(m, OS, resume.app_version);
                if !GSP.riscv_active(m) { return Err(bad("seq: GSP RISC-V not active after resume")); }
            }
            _ => unreachable!(),
        }
        p += 1 + len;
        n += 1;
    }
    Ok(n)
}
