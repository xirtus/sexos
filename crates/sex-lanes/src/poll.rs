// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! When a lane's worker polls and when it sleeps on its queue's interrupt.
//!
//! Lightly loaded, every completion costs an interrupt and a wake, which is cheap
//! next to the work. Heavily loaded, those interrupts are most of the cost, so the
//! worker masks its queue's vector and polls the rings instead (Linux's NAPI and
//! io_poll do the same). The policy:
//!
//! - In [`Mode::Interrupt`], a round that finds at least `enter_batch` items (a
//!   burst) switches to polling and masks the vector.
//! - In [`Mode::Polling`], every round that finds nothing counts toward
//!   `idle_rounds`; one that finds work resets the count. After `idle_rounds` empty
//!   rounds the worker unmasks, looks once more and sleeps.
//! - A polling stretch is cut after `max_rounds` rounds even under load, so the
//!   worker yields its core now and then (`Action::Yield`) and other tasks run.
//!
//! The thresholds adapt: a burst that turns out to be a single round (polling found
//! nothing at all afterwards) makes the next entry harder, and a stretch that kept
//! finding work makes it easier, within `[min_batch, max_batch]`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Interrupt,
    Polling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Look at the rings again now.
    Poll,
    /// Mask the queue's interrupt and look again now.
    MaskAndPoll,
    /// Give the core away once, then poll again.
    Yield,
    /// Unmask, look once more, and sleep if still nothing.
    UnmaskAndSleep,
    /// Sleep on the interrupt (it is already unmasked).
    Sleep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub min_batch: u32,
    pub max_batch: u32,
    pub idle_rounds: u32,
    pub max_rounds: u32,
}

impl Config {
    pub const DEFAULT: Config = Config { min_batch: 2, max_batch: 32, idle_rounds: 64, max_rounds: 4096 };
}

#[derive(Clone, Copy, Debug)]
pub struct Adaptive {
    cfg: Config,
    mode: Mode,
    enter_batch: u32,
    empty: u32,
    rounds: u32,
    found_in_stretch: u64,
    /// Totals, for a driver's counters.
    pub stretches: u64,
    pub polled_items: u64,
    pub interrupt_items: u64,
}

impl Adaptive {
    pub const fn new(cfg: Config) -> Self {
        let start = if cfg.min_batch * 4 > cfg.max_batch { cfg.max_batch } else { cfg.min_batch * 4 };
        Self { cfg, mode: Mode::Interrupt, enter_batch: start, empty: 0, rounds: 0, found_in_stretch: 0,
               stretches: 0, polled_items: 0, interrupt_items: 0 }
    }

    pub fn mode(&self) -> Mode { self.mode }
    pub fn enter_batch(&self) -> u32 { self.enter_batch }

    /// One round of work found `found` items (completions plus submissions). Says
    /// what to do next.
    pub fn round(&mut self, found: u32) -> Action {
        match self.mode {
            Mode::Interrupt => {
                self.interrupt_items += found as u64;
                if found >= self.enter_batch {
                    self.mode = Mode::Polling;
                    self.empty = 0;
                    self.rounds = 0;
                    self.found_in_stretch = 0;
                    self.stretches += 1;
                    Action::MaskAndPoll
                } else if found != 0 {
                    Action::Poll
                } else {
                    Action::Sleep
                }
            }
            Mode::Polling => {
                self.polled_items += found as u64;
                self.found_in_stretch += found as u64;
                self.rounds += 1;
                if found != 0 { self.empty = 0; } else { self.empty += 1; }
                if self.empty >= self.cfg.idle_rounds {
                    self.leave();
                    return Action::UnmaskAndSleep;
                }
                if self.rounds >= self.cfg.max_rounds {
                    self.rounds = 0;
                    return Action::Yield;
                }
                Action::Poll
            }
        }
    }

    /// Back to interrupts, learning from the stretch that just ended.
    fn leave(&mut self) {
        self.mode = Mode::Interrupt;
        if self.found_in_stretch == 0 {
            // Polled for nothing: the burst was a one-off. Want a bigger one next time.
            self.enter_batch = (self.enter_batch * 2).min(self.cfg.max_batch);
        } else if self.found_in_stretch >= self.enter_batch as u64 * 4 {
            // Polling paid off several times over: enter sooner.
            self.enter_batch = (self.enter_batch / 2).max(self.cfg.min_batch);
        }
        self.empty = 0;
        self.rounds = 0;
    }
}

/// Hybrid waiting for one synchronous command: spin on the completion for about as
/// long as the device usually takes, and only then sleep on its interrupt.
///
/// An interrupt and the wake that follows cost far more than a few microseconds of
/// spinning when the device is that quick (a fast SSD answers a 4 KiB read in tens of
/// microseconds; the interrupt, the wake and the switch back cost about as much). A
/// slow command (a flush, a spun-down disk) spins only `floor` and then sleeps, so
/// spinning never costs more than a fraction of what it might save.
///
/// Times are in whatever unit the caller measures (TSC cycles in SexDrive).
#[derive(Clone, Copy, Debug)]
pub struct Hybrid {
    /// Moving average of recent completion times, in 1/8 units for precision.
    avg8: u64,
    samples: u64,
    /// Spin at least this long (covers the first few polls).
    pub floor: u64,
    /// Never spin longer than this: past it, sleeping is cheaper than burning the core.
    pub cap: u64,
    pub spun_hits: u64,
    pub slept: u64,
}

impl Hybrid {
    pub const fn new(floor: u64, cap: u64) -> Self {
        Self { avg8: 0, samples: 0, floor, cap, spun_hits: 0, slept: 0 }
    }

    /// How long to spin before sleeping.
    pub fn spin_budget(&self) -> u64 {
        if self.samples < 4 { return self.floor; }
        let avg = self.avg8 / 8;
        if avg > self.cap { return self.floor; }
        // A little past the average catches most completions without spinning long
        // on the ones that will be slow anyway.
        (avg + avg / 2).clamp(self.floor, self.cap)
    }

    /// A command finished after `took`; `spun` says whether spinning caught it.
    pub fn record(&mut self, took: u64, spun: bool) {
        let t8 = took.min(self.cap.saturating_mul(16)).saturating_mul(8);
        self.avg8 = if self.samples == 0 { t8 } else { self.avg8 - self.avg8 / 8 + t8 / 8 };
        self.samples += 1;
        if spun { self.spun_hits += 1; } else { self.slept += 1; }
    }

    pub fn average(&self) -> u64 { self.avg8 / 8 }
}
