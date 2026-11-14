// SPDX-License-Identifier: GPL-3.0-only
//! Idle is held off while media plays (workstream B7).
//!
//! Before the idle policy dims, locks or blanks, the Shell asks Bell whether a program's
//! sound stream is playing (`sound_mixer::STREAMING`). A yes counts as input: the idle
//! clock starts again. Bell's answer comes back later, so the step waits for it, but
//! not for long: with no answer the policy goes ahead. An answer holds for a while, so
//! the Shell asks at most once per `RECHECK_MS` while nothing plays.

/// How long the policy waits for Bell's answer.
pub const ANSWER_MS: u64 = 2_000;
/// How long a "nothing plays" answer holds.
pub const RECHECK_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Go ahead with the idle step.
    Proceed,
    /// Ask Bell first (call `asked` once the request went out).
    Ask,
    /// An answer is on its way: do nothing yet.
    Wait,
}

pub struct Inhibit { asked_at: Option<u64>, checked_at: Option<u64> }

impl Inhibit {
    pub const fn new() -> Self { Self { asked_at: None, checked_at: None } }

    /// What to do when the idle policy has a step `due` at `now`.
    pub fn step(&mut self, now: u64, due: bool) -> Step {
        if !due { return Step::Proceed; }
        if let Some(t) = self.asked_at {
            if now.saturating_sub(t) < ANSWER_MS { return Step::Wait; }
            // Bell did not answer: go ahead, and don't ask again for a while.
            self.asked_at = None;
            self.checked_at = Some(now);
            return Step::Proceed;
        }
        match self.checked_at {
            Some(t) if now.saturating_sub(t) < RECHECK_MS => Step::Proceed,
            _ => Step::Ask,
        }
    }

    pub fn asked(&mut self, now: u64) { self.asked_at = Some(now); }

    /// Bell's answer. True when media plays: the caller restarts the idle clock.
    pub fn answer(&mut self, now: u64, playing: bool) -> bool {
        if self.asked_at.take().is_none() { return false; }
        // A yes is asked again next time the policy is due; a no holds for a while.
        self.checked_at = if playing { None } else { Some(now) };
        playing
    }

    pub fn waiting(&self) -> bool { self.asked_at.is_some() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_due_needs_no_question() {
        let mut i = Inhibit::new();
        assert_eq!(i.step(100, false), Step::Proceed);
    }

    #[test]
    fn playing_holds_idle_off_every_time() {
        let mut i = Inhibit::new();
        assert_eq!(i.step(60_000, true), Step::Ask);
        i.asked(60_000);
        assert_eq!(i.step(60_500, true), Step::Wait);
        assert!(i.answer(60_600, true), "media plays: idle starts again");
        // A minute later the policy is due again and asks again.
        assert_eq!(i.step(120_600, true), Step::Ask);
    }

    #[test]
    fn silence_lets_idle_go_ahead_and_holds_for_a_while() {
        let mut i = Inhibit::new();
        assert_eq!(i.step(60_000, true), Step::Ask);
        i.asked(60_000);
        assert!(!i.answer(60_100, false));
        assert_eq!(i.step(60_200, true), Step::Proceed);
        assert_eq!(i.step(60_100 + RECHECK_MS, true), Step::Ask);
    }

    #[test]
    fn no_answer_goes_ahead() {
        let mut i = Inhibit::new();
        assert_eq!(i.step(0, true), Step::Ask);
        i.asked(1);
        assert_eq!(i.step(1 + ANSWER_MS - 1, true), Step::Wait);
        assert_eq!(i.step(1 + ANSWER_MS, true), Step::Proceed);
        assert!(!i.answer(1 + ANSWER_MS + 5, true), "a late answer changes nothing");
    }
}
