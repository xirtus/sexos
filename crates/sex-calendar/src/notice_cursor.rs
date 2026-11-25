// SPDX-License-Identifier: GPL-3.0-only
//! Date's durable last-checked minute for reminders and alarms. A catch-up is
//! bounded so a stale clock or an old copy cannot replay years of notices.
use alloc::format;
use alloc::string::String;

pub const MAX_CATCHUP_MINUTES: i64 = 7 * 24 * 60;
pub const NOTICE_LEASE_MS: u64 = 10_000;
const HEADER: &str = "# date-notices 1";

/// A visible Date run or parked worker refreshes this timestamp twice a
/// second. A stopped run gives up ownership after this short recovery window.
pub fn lease_live(seen_ms: u64, now_ms: u64) -> bool {
    seen_ms != 0 && seen_ms <= now_ms && now_ms - seen_ms <= NOTICE_LEASE_MS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeCursor { pub minute: i64 }

impl NoticeCursor {
    pub fn current(now_ms: u64) -> Self { Self { minute: (now_ms / 60_000) as i64 } }

    /// Use a well-formed previous minute, never one in the future. An old cursor
    /// is clipped to seven days before `now` so catch-up stays bounded.
    pub fn recover(text: &str, now_ms: u64) -> Self {
        let now = Self::current(now_ms);
        let mut lines = text.lines();
        let prior = if lines.next() == Some(HEADER) && lines.clone().count() == 1 {
            lines.next().and_then(|s| s.parse::<i64>().ok())
        } else { None };
        let Some(prior) = prior.filter(|m| *m >= 0 && *m <= now.minute) else { return now };
        Self { minute: prior.max(now.minute.saturating_sub(MAX_CATCHUP_MINUTES)) }
    }

    pub fn serialize(self) -> String { format!("{HEADER}\n{}\n", self.minute) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_recent_minute_and_bounds_old_catchup() {
        let now = 30_000 * 60_000u64;
        let recent = NoticeCursor { minute: 29_995 };
        assert_eq!(NoticeCursor::recover(&recent.serialize(), now), recent);
        assert_eq!(NoticeCursor::recover(&NoticeCursor { minute: 1 }.serialize(), now).minute,
            30_000 - MAX_CATCHUP_MINUTES);
    }

    #[test]
    fn invalid_or_future_cursor_uses_current_minute() {
        let now = 30_000 * 60_000u64;
        for text in ["", "30000", "# date-notices 2\n29999\n", "# date-notices 1\n-1\n",
            "# date-notices 1\n30001\n", "# date-notices 1\n29999\nmore\n"] {
            assert_eq!(NoticeCursor::recover(text, now), NoticeCursor::current(now), "{text:?}");
        }
    }

    #[test]
    fn crashed_handoff_and_window_leases_expire() {
        let now = 100_000;
        assert!(lease_live(now, now));
        assert!(lease_live(now - NOTICE_LEASE_MS, now));
        assert!(!lease_live(now - NOTICE_LEASE_MS - 1, now));
        assert!(!lease_live(now + 1, now));
        assert!(!lease_live(0, now));
    }
}
