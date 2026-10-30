// SPDX-License-Identifier: GPL-3.0-only
//! Bounded retention of Browser command, import-name and reader down input on the existing BOOT route.
//! Queue membership is data; the shell's held Domain capability authorizes send.
const CAPACITY: usize = 8;
/// PS/2 navigation arrives as its set1 byte; USB uses normalized codes.
/// Only the focused app route consumes these equivalent representations.
pub fn command(code:u8)->Option<u8> {
 let code=match code {0x47=>0x66,0x48=>0x67,0x49=>0x68,0x4F=>0x6B,0x50=>0x6C,0x51=>0x6D,_=>code};
 if printable(code)||matches!(code,0x01|0x0E|0x1C|0x30|0x15|0x26|0x14|0x12|0x10|0x13|0x2E|0x20|0x2D|0x23|0x24|0x2F|0x67|0x6C|0x68|0x6D|0x66|0x6B){Some(code)}else{None}
}
/// Only safe unshifted import-name characters; interpretation stays app-local.
fn printable(code:u8)->bool{matches!(code,0x02..=0x0C|0x10..=0x19|0x1E..=0x26|0x2C..=0x32|0x34)}



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
        let Some(code)=command(code) else{return false;};
        if self.len == CAPACITY { return false; }
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
    #[test] fn only_safe_name_commands_and_reader_down_codes_are_accepted() {
        for code in 0..=u8::MAX {
            let mut q = Queue::new();
            assert_eq!(q.push(code), matches!(code, 0x01 | 0x0E | 0x1C | 0x02..=0x0C | 0x10..=0x19 | 0x1E..=0x26 | 0x2C..=0x32 | 0x34 | 0x30 | 0x15 | 0x26 | 0x14 | 0x12 | 0x10 | 0x13 | 0x2E | 0x20 | 0x2D | 0x23 | 0x24 | 0x2F | 0x67 | 0x6C | 0x68 | 0x6D | 0x66 | 0x6B | 0x47 | 0x48 | 0x49 | 0x4F | 0x50 | 0x51));
            assert_eq!(q.len(), usize::from(matches!(code, 0x01 | 0x0E | 0x1C | 0x02..=0x0C | 0x10..=0x19 | 0x1E..=0x26 | 0x2C..=0x32 | 0x34 | 0x30 | 0x15 | 0x26 | 0x14 | 0x12 | 0x10 | 0x13 | 0x2E | 0x20 | 0x2D | 0x23 | 0x24 | 0x2F | 0x67 | 0x6C | 0x68 | 0x6D | 0x66 | 0x6B | 0x47 | 0x48 | 0x49 | 0x4F | 0x50 | 0x51)));
            if q.len() == 0 { assert_eq!(q.codes, [0; CAPACITY]); assert_eq!(q.head, 0); }
        }
    }
    #[test] fn ps2_and_usb_navigation_share_canonical_retained_delivery() {
        for (raw,normalized) in [(0x47,0x66),(0x48,0x67),(0x49,0x68),(0x4F,0x6B),(0x50,0x6C),(0x51,0x6D)] {
            let mut q=Queue::new();assert!(q.push(raw));assert!(q.push(normalized));
            assert_eq!(q.poll(true,FULL,|code|{assert_eq!(code,normalized);Err(FULL)}),Outcome::Blocked);
            assert_eq!(q.len(),2);sent(&mut q,normalized);sent(&mut q,normalized);
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
