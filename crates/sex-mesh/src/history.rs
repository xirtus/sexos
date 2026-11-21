// SPDX-License-Identifier: GPL-3.0-only
//! A fixed ring of the last `N` samples of one measurement, oldest first.

pub struct History<const N: usize> {
    values: [u32; N],
    start: usize,
    len: usize,
}

impl<const N: usize> Default for History<N> {
    fn default() -> Self { Self::new() }
}

impl<const N: usize> History<N> {
    pub const fn new() -> Self { Self { values: [0; N], start: 0, len: 0 } }
    /// Adds the newest sample; past `N` the oldest is forgotten.
    pub fn push(&mut self, value: u32) {
        if N == 0 { return; }
        let end = (self.start + self.len) % N;
        self.values[end] = value;
        if self.len < N { self.len += 1; } else { self.start = (self.start + 1) % N; }
    }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub const fn capacity(&self) -> usize { N }
    /// The `i`th sample, oldest first.
    pub fn get(&self, i: usize) -> Option<u32> { (i < self.len).then(|| self.values[(self.start + i) % N]) }
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ { (0..self.len).map(move |i| self.values[(self.start + i) % N]) }
    pub fn latest(&self) -> Option<u32> { self.len.checked_sub(1).and_then(|i| self.get(i)) }
    pub fn max(&self) -> u32 { self.iter().max().unwrap_or(0) }
    pub fn clear(&mut self) { self.start = 0; self.len = 0; }
}

/// How busy a span was, in permille, from the idle cycles counted over it: `cores`
/// cores each ran `span` cycles, `idle` of all of them were idle.
pub fn busy_permille(idle: u64, span: u64, cores: u64) -> u32 {
    let all = span.saturating_mul(cores.max(1));
    if all == 0 { return 0; }
    1000 - ((idle as u128 * 1000 / all as u128).min(1000) as u32)
}

/// `part` of `whole` in permille, 0 when `whole` is 0.
pub fn permille(part: u64, whole: u64) -> u32 {
    if whole == 0 { 0 } else { (part as u128 * 1000 / whole as u128).min(1000) as u32 }
}

/// Events per second from a count that grew by `delta` over `ms` milliseconds.
pub fn per_second(delta: u64, ms: u64) -> u64 { (delta as u128 * 1000 / ms.max(1) as u128).min(u64::MAX as u128) as u64 }

/// The top of a graph whose values have no natural ceiling: the largest value rounded
/// up to 1, 2 or 5 times a power of ten, so the scale reads well and seldom jumps.
pub fn nice_ceiling(max: u64) -> u64 {
    let mut step = 1u64;
    loop {
        for m in [1, 2, 5] {
            let top = step.saturating_mul(m);
            if top >= max || top == u64::MAX { return top; }
        }
        if step > u64::MAX / 10 { return u64::MAX; }
        step *= 10;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_the_newest_in_order() {
        let mut h: History<3> = History::new();
        assert!(h.is_empty() && h.latest().is_none() && h.max() == 0);
        for v in 1..=5 { h.push(v); }
        assert_eq!(h.len(), 3);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![3, 4, 5]);
        assert_eq!((h.get(0), h.get(2), h.get(3)), (Some(3), Some(5), None));
        assert_eq!((h.latest(), h.max()), (Some(5), 5));
        h.clear();
        assert!(h.is_empty());
        h.push(9);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![9]);
        let mut none: History<0> = History::new();
        none.push(1);
        assert!(none.is_empty());
    }

    #[test]
    fn busy_share_from_idle_cycles() {
        assert_eq!(busy_permille(0, 1000, 1), 1000);
        assert_eq!(busy_permille(1000, 1000, 1), 0);
        // Four cores, one idle the whole span: three quarters busy.
        assert_eq!(busy_permille(1000, 1000, 4), 750);
        // Idle beyond the span (clock skew) is never negative.
        assert_eq!(busy_permille(5000, 1000, 2), 0);
        assert_eq!(busy_permille(10, 0, 4), 0);
    }

    #[test]
    fn shares_rates_and_ceilings() {
        assert_eq!((permille(1, 4), permille(5, 0), permille(9, 3)), (250, 0, 1000));
        assert_eq!((per_second(50, 500), per_second(7, 0)), (100, 7000));
        assert_eq!([0, 1, 2, 3, 6, 10, 11, 99, 4000].map(nice_ceiling), [1, 1, 2, 5, 10, 10, 20, 100, 5000]);
        assert_eq!(nice_ceiling(u64::MAX), u64::MAX);
    }
}
