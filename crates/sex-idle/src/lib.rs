// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Which C-state an idle core enters (the kernel's mechanism is MONITOR/MWAIT).
//!
//! The ACPI service hands the kernel the processor's `_CST` states that MWAIT enters
//! ([`Table::add`]). Each time a core goes idle the kernel asks [`choose`] with the time
//! until its next timer event and the core's [`History`] of how long recent idle
//! periods really lasted. A deep state saves power only when the core stays in it long
//! enough to pay for getting in and out (its target residency, here twice the exit
//! latency as ACPI idle drivers assume), so the choice is the deepest state whose
//! residency fits the predicted idle time and whose exit latency fits the limit.
//!
//! The prediction is the time to the next event, shortened by the typical recent idle
//! length when recent periods agree (Linux's menu governor finds that "typical
//! interval" the same way: drop the longest sample until the rest are close together).
//! An interrupt that is not a timer (a disk, the network, an IPI) is what ends idle
//! early, and that history is the only warning of it.
#![no_std]

/// States kept, deepest last. C1 (HLT) is not in the table; it is the default.
pub const MAX: usize = 6;
/// Samples of history per core.
pub const SAMPLES: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// ACPI C-state type (2, 3; deeper `_CST` entries reuse 3).
    pub kind: u8,
    /// EAX for MWAIT (the C-state and sub-state).
    pub hint: u32,
    /// Worst-case exit latency, microseconds.
    pub latency_us: u32,
}
impl State {
    /// Minimum stay for the state to save energy.
    pub fn residency_us(&self) -> u32 { self.latency_us.saturating_mul(2) }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Table { states: [State; MAX], n: usize }

impl Table {
    pub const fn new() -> Table { Table { states: [State { kind: 0, hint: 0, latency_us: 0 }; MAX], n: 0 } }
    /// Adds a state, keeping the table ordered by exit latency. Refuses duplicates (same
    /// hint), C1, hints MWAIT cannot take, and a full table. Returns whether it was added.
    pub fn add(&mut self, s: State) -> bool {
        if s.kind < 2 || s.hint > 0xFF || self.n == MAX || self.states[..self.n].iter().any(|t| t.hint == s.hint) { return false; }
        let at = self.states[..self.n].iter().position(|t| t.latency_us > s.latency_us).unwrap_or(self.n);
        self.states.copy_within(at..self.n, at + 1);
        self.states[at] = s;
        self.n += 1;
        true
    }
    pub fn states(&self) -> &[State] { &self.states[..self.n] }
    pub fn len(&self) -> usize { self.n }
    pub fn is_empty(&self) -> bool { self.n == 0 }
}

/// How long one core's recent idle periods lasted.
#[derive(Clone, Copy, Debug, Default)]
pub struct History { us: [u32; SAMPLES], at: usize, filled: usize }

impl History {
    pub const fn new() -> History { History { us: [0; SAMPLES], at: 0, filled: 0 } }
    pub fn record(&mut self, us: u32) {
        self.us[self.at] = us;
        self.at = (self.at + 1) % SAMPLES;
        self.filled = (self.filled + 1).min(SAMPLES);
    }
    /// The typical recent idle length, when the samples agree; None otherwise.
    pub fn typical_us(&self) -> Option<u32> {
        if self.filled < SAMPLES { return None; }
        let mut limit = u32::MAX;
        // Up to three passes, each dropping the longest sample left.
        for _ in 0..3 {
            let (mut n, mut sum, mut max) = (0u64, 0u64, 0u32);
            for &v in self.us.iter().filter(|v| **v <= limit) { n += 1; sum += v as u64; max = max.max(v); }
            if n < (SAMPLES as u64 * 3) / 4 { return None; }
            let avg = sum / n;
            let var = self.us.iter().filter(|v| **v <= limit).map(|&v| { let d = v as i64 - avg as i64; (d * d) as u64 }).sum::<u64>() / n;
            // Close together: spread under 20 us, or a standard deviation under a sixth of the mean.
            if var <= 400 || avg * avg > 36 * var { return Some(avg as u32); }
            limit = max.saturating_sub(1);
        }
        None
    }
}

/// The index into `t.states()` of the state to enter, or None for C1 (HLT).
/// `until_event_us`: time to the next timer event; `limit_us`: the longest exit latency allowed.
pub fn choose(t: &Table, h: &History, until_event_us: u32, limit_us: u32) -> Option<usize> {
    let predicted = h.typical_us().map_or(until_event_us, |typ| typ.min(until_event_us));
    t.states().iter().rposition(|s| s.residency_us() <= predicted && s.latency_us <= limit_us)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x17() -> Table {
        // Tiger Lake-H as Dell firmware lists it: C1 (HLT, not added), C6 and C10 through MWAIT.
        let mut t = Table::new();
        assert!(t.add(State { kind: 3, hint: 0x60, latency_us: 253 }));
        assert!(t.add(State { kind: 2, hint: 0x20, latency_us: 85 }));
        assert!(!t.add(State { kind: 1, hint: 0x00, latency_us: 1 }));
        assert!(!t.add(State { kind: 2, hint: 0x20, latency_us: 85 }));
        assert!(!t.add(State { kind: 3, hint: 0x100, latency_us: 400 }));
        t
    }

    #[test]
    fn table_is_ordered_by_latency_and_bounded() {
        let t = x17();
        assert_eq!(t.states().iter().map(|s| s.hint).collect::<heapless_vec::V>(), heapless_vec::V::of(&[0x20, 0x60]));
        let mut full = Table::new();
        for i in 0..MAX as u32 { assert!(full.add(State { kind: 2, hint: i + 1, latency_us: 100 - i })); }
        assert!(!full.add(State { kind: 2, hint: 0x40, latency_us: 5 }));
        assert!(full.states().windows(2).all(|w| w[0].latency_us <= w[1].latency_us));
    }

    #[test]
    fn deepest_state_that_pays_for_itself() {
        let (t, h) = (x17(), History::new());
        // A whole 1 ms tick ahead: C10 (residency 506 us) fits.
        assert_eq!(choose(&t, &h, 1000, u32::MAX), Some(1));
        // 300 us left: only C6 (170 us) pays back.
        assert_eq!(choose(&t, &h, 300, u32::MAX), Some(0));
        // 100 us left: HLT.
        assert_eq!(choose(&t, &h, 100, u32::MAX), None);
        // A latency limit keeps the core out of C10 however long it waits.
        assert_eq!(choose(&t, &h, 1000, 200), Some(0));
        assert_eq!(choose(&Table::new(), &h, 1000, u32::MAX), None);
    }

    #[test]
    fn steady_short_idles_keep_the_core_shallow() {
        let t = x17();
        let mut h = History::new();
        // A device interrupting every ~120 us: deep states would never pay back.
        for v in [118, 121, 125, 119, 122, 120, 124, 117] { h.record(v); }
        assert_eq!(h.typical_us(), Some(120));
        assert_eq!(choose(&t, &h, 1000, u32::MAX), None);
        // Every ~400 us: C6 but not C10.
        for v in [400, 410, 395, 405, 398, 402, 407, 393] { h.record(v); }
        assert_eq!(choose(&t, &h, 1000, u32::MAX), Some(0));
    }

    #[test]
    fn scattered_idles_trust_the_timer() {
        let t = x17();
        let mut h = History::new();
        for v in [5, 900, 40, 700, 300, 1000, 15, 650] { h.record(v); }
        assert_eq!(h.typical_us(), None);
        assert_eq!(choose(&t, &h, 1000, u32::MAX), Some(1));
        // Not enough samples yet: the timer alone decides.
        let mut young = History::new();
        for v in [10, 10, 10] { young.record(v); }
        assert_eq!(young.typical_us(), None);
        assert_eq!(choose(&t, &young, 1000, u32::MAX), Some(1));
    }

    #[test]
    fn one_outlier_is_dropped() {
        let mut h = History::new();
        // Seven ticks that ran out, and one early wake: still the deepest state.
        for v in [998, 1000, 999, 1000, 20, 1000, 997, 999] { h.record(v); }
        assert_eq!(choose(&x17(), &h, 1000, u32::MAX), Some(1));
        let mut h2 = History::new();
        // Seven short periods and one long one: the long one goes, short wins.
        for v in [100, 102, 98, 101, 950, 99, 100, 103] { h2.record(v); }
        assert_eq!(h2.typical_us(), Some(100));
    }

    /// A fixed-size vector for comparing without alloc.
    mod heapless_vec {
        #[derive(Debug, PartialEq, Eq, Default)]
        pub struct V { v: [u32; super::MAX], n: usize }
        impl V { pub fn of(s: &[u32]) -> V { let mut x = V::default(); for &e in s { x.v[x.n] = e; x.n += 1; } x } }
        impl FromIterator<u32> for V { fn from_iter<I: IntoIterator<Item = u32>>(i: I) -> V { let mut x = V::default(); for e in i { x.v[x.n] = e; x.n += 1; } x } }
    }
}
