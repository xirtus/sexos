// SPDX-License-Identifier: GPL-3.0-only
//! Key repeat for held keys, the same for PS/2 and USB keyboards.
//!
//! Shell arms it when a key press reaches a program and replays the press
//! while the key stays down: first after `delay` ms, then every `interval` ms.
//! A PS/2 keyboard's own typematic presses of the armed key are swallowed, so
//! the configured rate applies to every keyboard. The last key pressed is the
//! one that repeats; releasing it stops the repeat.

/// Defaults: 500 ms before the first repeat, then 30 per second.
pub const DELAY_MS: u64 = 500;
pub const INTERVAL_MS: u64 = 33;
/// Settings ranges (ms).
pub const DELAY_RANGE: (u64, u64) = (150, 2000);
pub const INTERVAL_RANGE: (u64, u64) = (10, 500);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Repeat {
    held: Option<(u8, u64)>,
    delay: u64,
    interval: u64,
    /// Off: held keys type once, as before.
    enabled: bool,
}

impl Repeat {
    pub const fn new() -> Self { Self { held: None, delay: DELAY_MS, interval: INTERVAL_MS, enabled: true } }

    /// Sets delay and interval (clamped to the ranges); 0 turns repeat off.
    pub fn configure(&mut self, delay_ms: u64, interval_ms: u64) {
        self.enabled = delay_ms != 0 && interval_ms != 0;
        self.delay = delay_ms.clamp(DELAY_RANGE.0, DELAY_RANGE.1);
        self.interval = interval_ms.clamp(INTERVAL_RANGE.0, INTERVAL_RANGE.1);
        if !self.enabled { self.held = None; }
    }
    pub fn delay(&self) -> u64 { self.delay }
    pub fn interval(&self) -> u64 { self.interval }

    /// True when `code` is the armed key: a press of it is a hardware repeat.
    pub fn is_held(&self, code: u8) -> bool { matches!(self.held, Some((c, _)) if c == code) }
    /// A key held and waiting to repeat; Shell keeps its loop awake for it.
    pub fn pending(&self) -> bool { self.held.is_some() }

    /// A press that reached a program: it repeats from now on.
    pub fn press(&mut self, code: u8, now: u64) {
        if self.enabled { self.held = Some((code, now.saturating_add(self.delay))); }
    }
    /// Any key going up; only the armed key stops the repeat.
    pub fn release(&mut self, code: u8) { if self.is_held(code) { self.held = None; } }
    /// Focus moved, a window closed, or the keyboard went away.
    pub fn cancel(&mut self) { self.held = None; }

    /// The key to press again now, at most one per call. A late poll never
    /// bursts: the next repeat is one interval after this one.
    pub fn due(&mut self, now: u64) -> Option<u8> {
        let (code, next) = self.held?;
        if now < next { return None; }
        self.held = Some((code, now.saturating_add(self.interval)));
        Some(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_key_repeats_after_the_delay_at_the_rate_and_stops_on_release() {
        let mut r = Repeat::new();
        r.press(0x1e, 1000);
        assert_eq!(r.due(1499), None);
        assert_eq!(r.due(1500), Some(0x1e));
        assert_eq!(r.due(1532), None);
        assert_eq!(r.due(1533), Some(0x1e));
        // A late poll gives one repeat, not a burst.
        assert_eq!(r.due(3000), Some(0x1e));
        assert_eq!(r.due(3001), None);
        r.release(0x30);
        assert!(r.is_held(0x1e));
        r.release(0x1e);
        assert_eq!(r.due(9000), None);
        assert!(!r.pending());
    }
    #[test]
    fn last_key_wins_and_settings_clamp_or_turn_it_off() {
        let mut r = Repeat::new();
        r.press(0x1e, 0);
        r.press(0x1f, 100);
        assert!(!r.is_held(0x1e));
        assert_eq!(r.due(600), Some(0x1f));
        r.configure(5, 5000);
        assert_eq!((r.delay(), r.interval()), (DELAY_RANGE.0, INTERVAL_RANGE.1));
        r.configure(0, 33);
        r.press(0x1e, 0);
        assert!(!r.pending());
        r.configure(250, 20);
        r.press(0x1e, 0);
        assert_eq!(r.due(250), Some(0x1e));
    }
}
