// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! When to read a touchpad: when its interrupt line fires, or on a timer.
//!
//! An I2C HID device holds its interrupt asserted while it has a report and lets go
//! once the host reads it (HID over I2C 1.0 §7.4). With the line, Input reads only when
//! there is something to read: no reads at all while nobody touches the pad, and a new
//! contact is read at once instead of up to 8 ms later.
//!
//! Firmware can be wrong, so the line has to prove itself and polling stays the
//! fallback:
//! - a line that keeps firing with nothing to read (wrong polarity, a stuck wire) is
//!   given up after [`STUCK_AFTER`] empty reads in a row;
//! - a line that never fires is caught by a slow check read every [`CHECK_TICKS`]: a
//!   report there with no interrupt since the last check, [`SILENT_AFTER`] times in a
//!   row, gives the line up.
//!
//! Polling: about 125 reads a second while a finger was down in the last two seconds,
//! 20 when idle.

/// Ticks (ms) between polls while touched, and when idle.
pub const ACTIVE_TICKS: u64 = 8;
pub const IDLE_TICKS: u64 = 50;
/// Idle once nothing touched the pad for this long.
pub const IDLE_AFTER: u64 = 2000;
/// With a line: one check read this often.
pub const CHECK_TICKS: u64 = 1000;
/// Empty reads after interrupts, in a row, before the line is called stuck.
pub const STUCK_AFTER: u32 = 32;
/// Check reads that found a report no interrupt announced, in a row, before the line is
/// called silent.
pub const SILENT_AFTER: u32 = 3;
/// Reads one edge interrupt may take while draining.
pub const EDGE_DRAIN: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Poll,
    /// An I/O APIC line; `edge` lines are drained, level lines read once and unmasked.
    Line { edge: bool },
}

/// What one read of the input register found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// A zero-length answer: the pad had nothing.
    Empty,
    /// A report was taken; `touch`: a finger or the button is down.
    Report { touch: bool },
    /// The bus failed.
    Failed,
}

/// What the driver does after a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing more this turn.
    Done,
    /// Read again now (draining an edge).
    Again,
    /// Unmask the level line: the pad was quieted.
    Unmask,
    /// Mask the line for good and poll from now on.
    Fallback(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub mode: Mode,
    next: u64,
    last_touch: u64,
    empty_after_irq: u32,
    silent_checks: u32,
    fired_since_check: bool,
    /// Whether the read in progress was asked for by an interrupt.
    on_irq: bool,
    drained: u32,
    pub interrupts: u64,
}

impl Pace {
    pub fn polling() -> Self {
        Self { mode: Mode::Poll, next: 0, last_touch: 0, empty_after_irq: 0, silent_checks: 0, fired_since_check: false, on_irq: false, drained: 0, interrupts: 0 }
    }

    pub fn line(edge: bool, now: u64) -> Self {
        Self { mode: Mode::Line { edge }, next: now + CHECK_TICKS, ..Self::polling() }
    }

    /// Whether to read now. `fired`: the line's bit was taken since the last call.
    pub fn due(&mut self, now: u64, fired: bool) -> bool {
        match self.mode {
            Mode::Poll => now >= self.next,
            Mode::Line { .. } if fired => {
                self.interrupts += 1;
                self.fired_since_check = true;
                self.on_irq = true;
                self.drained = 0;
                true
            }
            Mode::Line { .. } => {
                self.on_irq = false;
                now >= self.next
            }
        }
    }

    /// After each read `due` asked for (and each `Again`).
    pub fn after(&mut self, now: u64, read: Read) -> Next {
        if let Read::Report { touch: true } = read { self.last_touch = now; }
        match self.mode {
            Mode::Poll => {
                self.next = now + if now.saturating_sub(self.last_touch) < IDLE_AFTER { ACTIVE_TICKS } else { IDLE_TICKS };
                Next::Done
            }
            Mode::Line { edge } if self.on_irq => {
                match read {
                    Read::Empty => {
                        self.empty_after_irq += 1;
                        // A drained edge ends on an empty read; that one is expected.
                        if edge && self.drained > 0 { self.empty_after_irq = 0; }
                        if self.empty_after_irq >= STUCK_AFTER { return self.fall_back(now, "line_stuck"); }
                    }
                    Read::Report { .. } => self.empty_after_irq = 0,
                    Read::Failed => {}
                }
                if !edge { return Next::Unmask; }
                self.drained += 1;
                if read != Read::Empty && read != Read::Failed && self.drained < EDGE_DRAIN { Next::Again } else { Next::Done }
            }
            Mode::Line { .. } => {
                // The slow check read.
                if let Read::Report { .. } = read {
                    if self.fired_since_check { self.silent_checks = 0; } else { self.silent_checks += 1; }
                } else if self.fired_since_check {
                    self.silent_checks = 0;
                }
                self.fired_since_check = false;
                self.next = now + CHECK_TICKS;
                if self.silent_checks >= SILENT_AFTER { return self.fall_back(now, "line_silent"); }
                Next::Done
            }
        }
    }

    fn fall_back(&mut self, now: u64, why: &'static str) -> Next {
        self.mode = Mode::Poll;
        self.next = now;
        Next::Fallback(why)
    }

    /// The tick of the next timed read (a poll, or the line's check).
    pub fn next_read(&self) -> u64 { self.next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_speeds_up_while_touched() {
        let mut p = Pace::polling();
        assert!(p.due(0, false));
        assert_eq!(p.after(0, Read::Empty), Next::Done);
        // The first two seconds count as touched, so a pad found at boot answers at once.
        assert_eq!(p.next_read(), ACTIVE_TICKS);
        let mut p = Pace::polling();
        p.after(5000, Read::Empty);
        assert_eq!(p.next_read(), 5000 + IDLE_TICKS);
        assert!(!p.due(5010, false));
        assert!(p.due(5050, false));
        p.after(5050, Read::Report { touch: true });
        assert_eq!(p.next_read(), 5050 + ACTIVE_TICKS);
        // A fired bit means nothing to a polled pad.
        assert!(!p.due(5051, true));
    }

    #[test]
    fn level_line_reads_on_interrupt_and_unmasks() {
        let mut p = Pace::line(false, 0);
        assert!(!p.due(10, false));
        assert!(p.due(10, true));
        assert_eq!(p.after(10, Read::Report { touch: true }), Next::Unmask);
        // A failed read still unmasks: a level line still asserted fires again.
        assert!(p.due(12, true));
        assert_eq!(p.after(12, Read::Failed), Next::Unmask);
        assert_eq!(p.interrupts, 2);
        // No reads between interrupts until the check.
        assert!(!p.due(999, false));
        assert!(p.due(1000, false));
        assert_eq!(p.after(1000, Read::Empty), Next::Done);
        assert_eq!(p.next_read(), 2000);
    }

    #[test]
    fn stuck_line_falls_back_to_polling() {
        let mut p = Pace::line(false, 0);
        for i in 0..STUCK_AFTER - 1 {
            assert!(p.due(i as u64, true));
            assert_eq!(p.after(i as u64, Read::Empty), Next::Unmask);
        }
        // A real report resets the count.
        p.due(40, true);
        assert_eq!(p.after(40, Read::Report { touch: false }), Next::Unmask);
        for i in 0..STUCK_AFTER - 1 { p.due(50 + i as u64, true); p.after(50 + i as u64, Read::Empty); }
        p.due(100, true);
        assert_eq!(p.after(100, Read::Empty), Next::Fallback("line_stuck"));
        assert_eq!(p.mode, Mode::Poll);
        assert!(p.due(100, false));
    }

    #[test]
    fn silent_line_falls_back_to_polling() {
        let mut p = Pace::line(false, 0);
        // Reports at the check with interrupts in between: fine.
        p.due(500, true); p.after(500, Read::Report { touch: true });
        assert!(p.due(1000, false));
        assert_eq!(p.after(1000, Read::Report { touch: true }), Next::Done);
        // Reports at checks but no interrupts.
        assert!(p.due(2000, false)); assert_eq!(p.after(2000, Read::Report { touch: true }), Next::Done);
        assert!(p.due(3000, false)); assert_eq!(p.after(3000, Read::Report { touch: true }), Next::Done);
        // An empty check in between does not clear the count: an untouched pad proves nothing.
        assert!(p.due(4000, false)); assert_eq!(p.after(4000, Read::Empty), Next::Done);
        assert!(p.due(5000, false)); assert_eq!(p.after(5000, Read::Report { touch: false }), Next::Fallback("line_silent"));
        assert_eq!(p.mode, Mode::Poll);
    }

    #[test]
    fn edge_line_drains() {
        let mut p = Pace::line(true, 0);
        assert!(p.due(10, true));
        assert_eq!(p.after(10, Read::Report { touch: true }), Next::Again);
        assert_eq!(p.after(10, Read::Report { touch: true }), Next::Again);
        assert_eq!(p.after(10, Read::Empty), Next::Done);
        // Bounded.
        assert!(p.due(20, true));
        let mut n = 1;
        while p.after(20, Read::Report { touch: true }) == Next::Again { n += 1; }
        assert_eq!(n, EDGE_DRAIN);
        // Empty draining ends never count as stuck.
        for i in 0..2 * STUCK_AFTER as u64 {
            p.due(100 + i, true);
            assert_eq!(p.after(100 + i, Read::Report { touch: false }), Next::Again);
            assert_eq!(p.after(100 + i, Read::Empty), Next::Done);
        }
        assert_eq!(p.mode, Mode::Line { edge: true });
    }
}
