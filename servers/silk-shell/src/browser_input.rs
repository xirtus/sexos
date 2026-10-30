// SPDX-License-Identifier: GPL-3.0-only
//! Bounded retention of explicit R/C down commands for the existing BOOT route.
//! Queue membership is data; the shell's held Domain capability authorizes send.
const CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Sent(u8),
    Blocked,
    Failed(u64),
    Retired(usize),
}

pub struct Queue {
    codes: [u8; CAPACITY],
    head: usize,
    len: usize,
}
impl Queue {
    pub const fn new() -> Self {
        Self { codes: [0; CAPACITY], head: 0, len: 0 }
    }
    pub fn len(&self) -> usize { self.len }
    /// Invalid/full input leaves every retained command unchanged.
    pub fn push(&mut self, code: u8) -> bool {
        if !matches!(code, 0x13 | 0x2E) || self.len == CAPACITY { return false; }
        self.codes[(self.head + self.len) % CAPACITY] = code;
        self.len += 1;
        true
    }
    fn clear(&mut self) {
        self.codes.fill(0);
        self.head = 0;
        self.len = 0;
    }
    /// Attempts at most one send. Full preserves the exact front command;
    /// terminal identity loss and hard rejection discard all retained commands.
    pub fn poll(&mut self, live: bool, full_error: u64,
        mut send: impl FnMut(u8) -> Result<(), u64>) -> Outcome {
        if !live {
            let discarded = self.len;
            self.clear();
            return Outcome::Retired(discarded);
        }
        if self.len == 0 { return Outcome::Idle; }
        let code = self.codes[self.head];
        match send(code) {
            Ok(()) => {
                self.codes[self.head] = 0;
                self.head = (self.head + 1) % CAPACITY;
                self.len -= 1;
                Outcome::Sent(code)
            }
            Err(error) if error == full_error => Outcome::Blocked,
            Err(error) => { self.clear(); Outcome::Failed(error) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FULL: u64 = 9;
    fn sent(q: &mut Queue, expected: u8) {
        let mut attempts = 0;
        assert_eq!(q.poll(true, FULL, |code| {
            attempts += 1; assert_eq!(code, expected); Ok(())
        }), Outcome::Sent(expected));
        assert_eq!(attempts, 1);
    }
    #[test] fn fifo_preserves_every_explicit_command() {
        let mut q = Queue::new();
        for code in [0x13, 0x2E, 0x2E, 0x13] { assert!(q.push(code)); }
        for code in [0x13, 0x2E, 0x2E, 0x13] { sent(&mut q, code); }
        assert_eq!(q.len(), 0);
        assert_eq!(q.poll(true, FULL, |_| panic!("empty queue sent")), Outcome::Idle);
    }
    #[test] fn exact_capacity_and_invalid_input_never_overwrite() {
        let mut q = Queue::new();
        for n in 0..CAPACITY { assert!(q.push(if n % 2 == 0 { 0x13 } else { 0x2E })); }
        let codes = q.codes; let head = q.head;
        assert!(!q.push(0x13)); assert!(!q.push(0x2E)); assert!(!q.push(0x93));
        assert_eq!(q.codes, codes); assert_eq!(q.head, head); assert_eq!(q.len(), CAPACITY);
        for n in 0..CAPACITY { sent(&mut q, if n % 2 == 0 { 0x13 } else { 0x2E }); }
    }
    #[test] fn only_r_and_c_down_codes_are_accepted() {
        for code in 0..=u8::MAX {
            let mut q = Queue::new();
            assert_eq!(q.push(code), matches!(code, 0x13 | 0x2E));
            assert_eq!(q.len(), usize::from(matches!(code, 0x13 | 0x2E)));
            if q.len() == 0 { assert_eq!(q.codes, [0; CAPACITY]); assert_eq!(q.head, 0); }
        }
    }
    #[test] fn full_retains_exact_front_and_entire_queue() {
        let mut q = Queue::new(); assert!(q.push(0x13)); assert!(q.push(0x2E));
        let codes = q.codes;
        for _ in 0..3 {
            let mut attempts = 0;
            assert_eq!(q.poll(true, FULL, |code| {
                attempts += 1; assert_eq!(code, 0x13); Err(FULL)
            }), Outcome::Blocked);
            assert_eq!(attempts, 1); assert_eq!(q.codes, codes); assert_eq!(q.len(), 2); assert_eq!(q.head, 0);
        }
        sent(&mut q, 0x13); sent(&mut q, 0x2E);
    }
    #[test] fn successful_pop_then_wrap_keeps_fifo() {
        let mut q = Queue::new();
        for _ in 0..CAPACITY { assert!(q.push(0x13)); }
        for _ in 0..5 { sent(&mut q, 0x13); }
        for _ in 0..5 { assert!(q.push(0x2E)); }
        assert!(!q.push(0x13));
        for _ in 0..3 { sent(&mut q, 0x13); }
        for _ in 0..5 { sent(&mut q, 0x2E); }
        assert_eq!(q.len(), 0);
    }
    #[test] fn hard_rejection_discards_all_without_retry_and_allows_new_input() {
        let mut q = Queue::new(); assert!(q.push(0x13)); assert!(q.push(0x2E));
        assert_eq!(q.poll(true, FULL, |code| { assert_eq!(code, 0x13); Err(42) }), Outcome::Failed(42));
        assert_eq!(q.len(), 0); assert_eq!(q.codes, [0; CAPACITY]);
        assert_eq!(q.poll(true, FULL, |_| panic!("rejected command retried")), Outcome::Idle);
        assert!(q.push(0x2E)); sent(&mut q, 0x2E);
    }
    #[test] fn terminal_clear_happens_before_any_send() {
        let mut q = Queue::new(); assert!(q.push(0x13)); assert!(q.push(0x2E));
        assert_eq!(q.poll(false, FULL, |_| panic!("dead identity sent")), Outcome::Retired(2));
        assert_eq!(q.len(), 0); assert_eq!(q.codes, [0; CAPACITY]);
        assert_eq!(q.poll(false, FULL, |_| panic!("dead identity sent")), Outcome::Retired(0));
        assert_eq!(q.poll(true, FULL, |_| panic!("retired command retried")), Outcome::Idle);
        assert!(q.push(0x13)); sent(&mut q, 0x13);
    }
}
