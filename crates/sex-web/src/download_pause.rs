// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! One live download's pause state and committed file offset. A paused transfer
//! cannot reserve a write; its next write resumes at exactly the prior offset.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transfer { paused: bool, written: u64, limit: u64 }

impl Transfer {
    pub const fn new(limit: u64) -> Self { Self { paused: false, written: 0, limit } }
    pub const fn paused(&self) -> bool { self.paused }
    pub const fn written(&self) -> u64 { self.written }
    pub fn toggle(&mut self) -> bool { self.paused = !self.paused; self.paused }
    /// The next file offset, only while the transfer is moving and within its cap.
    pub fn write_offset(&self, bytes: usize) -> Option<u64> {
        if self.paused { return None; }
        self.written.checked_add(bytes as u64).filter(|n| *n <= self.limit).map(|_| self.written)
    }
    /// Called only after Files acknowledged the whole write.
    pub fn committed(&mut self, bytes: usize) { self.written += bytes as u64; }
}

#[cfg(test)]
mod tests {
    use super::Transfer;

    #[test]
    fn pause_keeps_exact_file_offset_and_limit_through_resume() {
        let mut transfer = Transfer::new(12);
        assert_eq!(transfer.write_offset(5), Some(0));
        transfer.committed(5);
        assert!(transfer.toggle());
        assert_eq!(transfer.write_offset(1), None);
        assert_eq!(transfer.written(), 5);
        assert!(!transfer.toggle());
        assert_eq!(transfer.write_offset(7), Some(5));
        transfer.committed(7);
        assert_eq!(transfer.written(), 12);
        assert_eq!(transfer.write_offset(1), None);
    }
}
