// SPDX-License-Identifier: GPL-3.0-only
//! Waiting for an NVMe controller to finish enabling or disabling (CSTS.RDY).
//!
//! Real SSDs take up to CAP.TO x 500 ms to change state; a Samsung or WD drive that
//! firmware left running can need seconds to stop. A fixed poll count is a few
//! milliseconds on bare metal, so the wait is measured in timer ticks (about 1 ms).
//! A controller that reports a fatal status (CSTS.CFS) while enabling is given up on at
//! once; while disabling, CFS is expected (a reset is how it is cleared) and is waited out.

pub const RDY: u32 = 1 << 0;
pub const CFS: u32 = 1 << 1;
/// CAP.TO is in 500 ms units; a controller that reports 0 still gets one unit.
pub const TICKS_PER_UNIT: u64 = 500;
/// Polls between checks of the clock and a yield of the core.
pub const POLL_QUANTUM: u32 = 64;
/// Upper bound on polls, in case the clock does not move.
pub const MAX_POLLS: u32 = 64 * 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome { Ready, Fatal, Deadline, PollLimit }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report { pub outcome: Outcome, pub polls: u32, pub elapsed: u64, pub csts: u32 }

pub fn deadline_ticks(cap_to: u32) -> u64 { (cap_to.max(1) as u64) * TICKS_PER_UNIT }

/// Polls CSTS until RDY equals `want`. `csts` reads the register, `ticks` the clock,
/// `pause` gives the core away between quanta.
pub fn wait(want: bool, cap_to: u32, mut csts: impl FnMut() -> u32, mut ticks: impl FnMut() -> u64, mut pause: impl FnMut()) -> Report {
    let start = ticks();
    let limit = deadline_ticks(cap_to);
    let mut report = Report { outcome: Outcome::PollLimit, polls: 0, elapsed: 0, csts: 0 };
    while report.polls < MAX_POLLS {
        for _ in 0..POLL_QUANTUM {
            let value = csts();
            report.polls += 1;
            report.csts = value;
            // All ones: the device fell off the bus (surprise removal, or a link down).
            if want && value != u32::MAX && value & CFS != 0 { report.outcome = Outcome::Fatal; return report; }
            if value != u32::MAX && (value & RDY != 0) == want { report.outcome = Outcome::Ready; return report; }
        }
        report.elapsed = ticks().saturating_sub(start);
        if report.elapsed >= limit { report.outcome = Outcome::Deadline; return report; }
        pause();
    }
    report
}
