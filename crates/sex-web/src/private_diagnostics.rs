// SPDX-License-Identifier: GPL-3.0-only
//! Browser diagnostic policy. Once a private tab exists, asynchronous work can
//! complete after another tab is shown, so content-bearing records stay off for
//! the rest of this browser process. Structural counts use separate call sites.

use core::sync::atomic::{AtomicBool, Ordering};

pub struct Policy { private_seen: AtomicBool }

impl Default for Policy { fn default() -> Self { Self::new() } }

impl Policy {
    pub const fn new() -> Self { Self { private_seen: AtomicBool::new(false) } }
    pub fn note_private(&self) { self.private_seen.store(true, Ordering::Release); }
    pub fn allow_content(&self) -> bool { !self.private_seen.load(Ordering::Acquire) }
    pub fn address<'a>(&self, value: &'a str) -> &'a str {
        if self.allow_content() { value } else { "<private>" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_history_remains_redacted_after_tab_switch_and_close() {
        let policy = Policy::new();
        assert!(policy.allow_content());
        assert_eq!(policy.address("https://public.example/path"), "https://public.example/path");
        policy.note_private();
        assert!(!policy.allow_content());
        assert_eq!(policy.address("https://secret.example/path?token=x"), "<private>");
        // Public tabs and completed private requests can now interleave. The
        // policy cannot be reset by changing the currently shown tab.
        assert_eq!(policy.address("https://public.example/path"), "<private>");
        policy.note_private();
        assert!(!policy.allow_content());
    }
}
