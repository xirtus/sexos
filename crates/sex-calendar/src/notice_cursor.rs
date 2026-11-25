// SPDX-License-Identifier: GPL-3.0-only
//! Date's durable last-checked minute for reminders and alarms. A catch-up is
//! bounded so a stale clock or an old copy cannot replay years of notices.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::clock::{Alarm, Clock};
use crate::{Calendar, Date};

pub const MAX_CATCHUP_MINUTES: i64 = 7 * 24 * 60;
pub const NOTICE_LEASE_MS: u64 = 10_000;
pub const NOTICE_POLL_MS: u64 = 500;
pub const NOTICE_RETRY_MS: u64 = 5_000;
const HEADER: &str = "# date-notices 1";

/// Poll independently of the window's input stream. A stream of pointer or
/// keyboard events must not postpone a due Bell notice indefinitely.
pub struct NoticeCadence { next: u64 }

impl NoticeCadence {
    pub fn new(now_ticks: u64) -> Self { Self { next: now_ticks.saturating_add(NOTICE_POLL_MS) } }
    pub fn due(&mut self, now_ticks: u64) -> bool {
        if now_ticks < self.next { return false; }
        self.next = now_ticks.saturating_add(NOTICE_POLL_MS);
        true
    }
}

/// Bell transport failures are retried after a short pause, including while a
/// visible Date window is receiving input. This does not acknowledge Bell's
/// own policy or prevent replay after a crash.
#[derive(Default)]
pub struct NoticeRetry { next_ticks: u64 }

impl NoticeRetry {
    pub fn ready(&self, now_ticks: u64) -> bool { now_ticks >= self.next_ticks }
    pub fn failed(&mut self, now_ticks: u64) { self.next_ticks = now_ticks.saturating_add(NOTICE_RETRY_MS); }
    pub fn succeeded(&mut self) { self.next_ticks = 0; }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DueNotice {
    Reminder { event: usize, day: Date, at: i64 },
    Alarm { index: usize, alarm: Alarm, at: i64 },
}

impl DueNotice {
    pub fn at(&self) -> i64 { match self { Self::Reminder { at, .. } | Self::Alarm { at, .. } => *at } }
}

pub struct NoticeDelivery {
    /// Last complete minute. If posting fails at minute M, M remains pending.
    pub cursor: i64,
    pub delivered: Vec<DueNotice>,
    pub failed: bool,
}

/// Post reminders and alarms in due-minute order. A failed transport stops the
/// batch and retains its minute for retry. Earlier successful posts in that
/// same minute can replay, so delivery is at least once, not exactly once.
pub fn deliver_due(cal: &Calendar, clock: &mut Clock, after: i64, upto: i64,
    local_offset: i32, mut post: impl FnMut(&DueNotice) -> bool) -> NoticeDelivery {
    let mut due: Vec<DueNotice> = cal.reminders_due(after, upto, local_offset).into_iter()
        .map(|(event, day, at)| DueNotice::Reminder { event, day, at }).collect();
    let alarm_offset = clock.home_zone().offset(upto);
    for (index, alarm) in clock.alarms.iter().enumerate() {
        if let Some(at) = alarm.due(after, upto, alarm_offset) {
            due.push(DueNotice::Alarm { index, alarm: alarm.clone(), at });
        }
    }
    due.sort_by_key(DueNotice::at);
    let mut delivered = Vec::new();
    for notice in due {
        if !post(&notice) {
            return NoticeDelivery { cursor: notice.at().saturating_sub(1), delivered, failed: true };
        }
        if let DueNotice::Alarm { index, alarm, .. } = &notice {
            if alarm.days == 0 { clock.alarms[*index].enabled = false; }
        }
        delivered.push(notice);
    }
    NoticeDelivery { cursor: upto, delivered, failed: false }
}

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
    use crate::Event;
    use alloc::vec;

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

    #[test]
    fn notice_poll_advances_during_continuous_events_without_duplicate_interval() {
        let mut cadence = NoticeCadence::new(0);
        let mut polls = alloc::vec::Vec::new();
        for ticks in (0..=2_000).step_by(50) {
            // Every iteration represents an input event, so there is never an
            // idle return from a window event wait.
            if cadence.due(ticks) { polls.push(ticks); }
        }
        assert_eq!(polls, [500, 1_000, 1_500, 2_000]);
        assert!(!cadence.due(2_000), "one interval gets one due check");
    }

    #[test]
    fn failed_bell_post_retains_reminder_and_one_time_alarm_until_retry() {
        let day = Date { year: 1970, month: 1, day: 1 };
        let cal = Calendar { events: vec![Event { start: day, time: Some((61, 61)),
            title: String::from("meeting"), alarms: vec![0], ..Event::default() }], calendars: vec![] };
        let mut clock = Clock { alarms: vec![Alarm { minute: 61, days: 0, enabled: true,
            label: String::from("wake") }], ..Clock::default() };
        let blocked = deliver_due(&cal, &mut clock, 60, 61, 0, |_| false);
        assert!(blocked.failed);
        assert_eq!(blocked.cursor, 60);
        assert!(blocked.delivered.is_empty());
        assert!(clock.alarms[0].enabled);
        let mut attempted = vec![];
        let first = deliver_due(&cal, &mut clock, 60, 61, 0, |notice| {
            attempted.push(notice.clone());
            matches!(notice, DueNotice::Reminder { .. })
        });
        assert!(first.failed);
        assert_eq!(first.cursor, 60, "the failed minute must remain pending");
        assert_eq!(first.delivered.len(), 1);
        assert!(clock.alarms[0].enabled, "failed one-time alarm must stay enabled");
        assert!(Clock::parse(&clock.serialize()).alarms[0].enabled);
        assert_eq!(NoticeCursor::recover(&NoticeCursor { minute: first.cursor }.serialize(), 61 * 60_000).minute, 60);
        assert_eq!(attempted.len(), 2);
        let mut retry = NoticeRetry::default();
        retry.failed(61_000);
        assert!(!retry.ready(65_999));
        assert!(retry.ready(66_000));
        let second = deliver_due(&cal, &mut clock, first.cursor, 61, 0, |_| true);
        assert!(!second.failed);
        assert_eq!(second.cursor, 61);
        assert_eq!(second.delivered.len(), 2, "earlier success may replay in the failed minute");
        assert!(!clock.alarms[0].enabled);
        assert!(deliver_due(&cal, &mut clock, second.cursor, 62, 0, |_| true).delivered.is_empty());
    }
}
