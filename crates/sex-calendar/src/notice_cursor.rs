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
const RECEIPT_HEADER: &str = "# date-receipts 1";
pub const RECEIPT_LIMIT: usize = 256;

/// Date-owned durable receipts for UID-backed reminder keys. This bounds
/// same-minute replay after a cursor save fails, including after a restart.
/// The oldest entry is evicted at the limit; it is not an exactly-once store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiptJournal { entries: Vec<(i64, u128)> }
impl ReceiptJournal {
    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next() != Some(RECEIPT_HEADER) { return None; }
        let mut entries = Vec::new();
        for line in lines {
            if entries.len() == RECEIPT_LIMIT { return None; }
            let (minute, digest) = line.split_once(' ')?;
            if digest.len() != 32 { return None; }
            let minute = minute.parse::<i64>().ok()?;
            let digest = u128::from_str_radix(digest, 16).ok()?;
            if minute < 0 || entries.iter().any(|&(m, k)| m == minute && k == digest) { return None; }
            entries.push((minute, digest));
        }
        Some(Self { entries })
    }
    pub fn serialize(&self) -> String {
        let mut text = String::from(RECEIPT_HEADER);
        text.push('\n');
        for &(minute, digest) in &self.entries {
            text.push_str(&format!("{minute} {digest:032x}\n"));
        }
        text
    }
    pub fn contains(&self, minute: i64, digest: u128) -> bool {
        self.entries.contains(&(minute, digest))
    }
    /// Persist only after Shell's handled receipt. A failed file commit leaves
    /// the in-memory journal unchanged and Date retries the due minute.
    pub fn record_after_ack(&mut self, minute: i64, digest: u128,
        mut save: impl FnMut(&str) -> bool) -> bool {
        if self.contains(minute, digest) { return true; }
        let mut next = self.clone();
        next.entries.retain(|&(at, _)| at >= minute.saturating_sub(MAX_CATCHUP_MINUTES));
        if next.entries.len() == RECEIPT_LIMIT { next.entries.remove(0); }
        next.entries.push((minute, digest));
        if !save(&next.serialize()) { return false; }
        *self = next;
        true
    }
}

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

    /// Stable source identity for a reminder occurrence. A UID is required:
    /// the event's vector index changes after import, sorting or sync. The
    /// due UTC minute distinguishes multiple alarms on one occurrence.
    /// Length-prefixing the UID keeps arbitrary iCalendar UIDs unambiguous.
    /// Clock alarms have no persisted ID yet and deliberately return None.
    pub fn semantic_key(&self, cal: &Calendar) -> Option<String> {
        let Self::Reminder { event, day, at } = self else { return None };
        let source = cal.events.get(*event)?;
        if source.uid.is_empty() { return None; }
        Some(format!("date:reminder:{}:{}:{}:{}:{}", source.calendar,
            source.uid.len(), source.uid, day.days(), at))
    }
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

    /// A missing or damaged cursor with saved notices must replay a bounded
    /// window. Otherwise a failed first cursor write followed by a reopen
    /// could silently discard the notice. No delivery starts until `save` has
    /// committed this baseline.
    pub fn bootstrap(raw: Option<&str>, now_ms: u64, has_saved_state: bool,
        mut save: impl FnMut(Self) -> bool) -> (Self, bool) {
        if let Some(valid) = raw.and_then(|text| Self::decode(text, now_ms)) { return (valid, true); }
        let now = Self::current(now_ms);
        let baseline = if has_saved_state {
            Self { minute: now.minute.saturating_sub(MAX_CATCHUP_MINUTES).max(0) }
        } else { now };
        let ready = save(baseline);
        (baseline, ready)
    }

    pub fn decode(text: &str, now_ms: u64) -> Option<Self> {
        let now = Self::current(now_ms);
        let mut lines = text.lines();
        let prior = if lines.next() == Some(HEADER) && lines.clone().count() == 1 {
            lines.next().and_then(|s| s.parse::<i64>().ok())
        } else { None };
        let prior = prior.filter(|m| *m >= 0 && *m <= now.minute)?;
        Some(Self { minute: prior.max(now.minute.saturating_sub(MAX_CATCHUP_MINUTES)) })
    }

    /// Use a well-formed previous minute, never one in the future. An old cursor
    /// is clipped to seven days before `now` so catch-up stays bounded.
    pub fn recover(text: &str, now_ms: u64) -> Self {
        Self::decode(text, now_ms).unwrap_or_else(|| Self::current(now_ms))
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

    #[test]
    fn reminder_key_survives_reorder_and_separates_occurrences() {
        let day = Date { year: 2026, month: 11, day: 25 };
        let first = Event { uid: String::from("uid:one"), calendar: 2, ..Event::all_day(day, "first") };
        let other = Event { uid: String::from("uid:two"), calendar: 2, ..Event::all_day(day, "other") };
        let cal = Calendar { events: vec![first.clone(), other.clone()], calendars: vec![] };
        let due = DueNotice::Reminder { event: 0, day, at: 1234 };
        let key = due.semantic_key(&cal).unwrap();
        let reordered = Calendar { events: vec![other, first], calendars: vec![] };
        assert_eq!(key, DueNotice::Reminder { event: 1, day, at: 1234 }.semantic_key(&reordered).unwrap());
        assert_ne!(key, DueNotice::Reminder { event: 1, day, at: 1235 }.semantic_key(&reordered).unwrap());
        assert_ne!(key, DueNotice::Reminder { event: 1, day: Date { day: 26, ..day }, at: 1234 }.semantic_key(&reordered).unwrap());
        assert_ne!(key, DueNotice::Reminder { event: 0, day, at: 1234 }.semantic_key(&reordered).unwrap());
        assert!(DueNotice::Alarm { index: 0, alarm: Alarm { minute: 10, days: 0, enabled: true,
            label: String::from("wake") }, at: 1234 }.semantic_key(&cal).is_none());
        assert!(DueNotice::Reminder { event: 99, day, at: 1234 }.semantic_key(&cal).is_none());
    }

    #[test]
    fn failed_initial_cursor_save_reopens_from_bounded_replay_floor() {
        let now = 20_000 * 60_000u64;
        let (first, ready) = NoticeCursor::bootstrap(None, now, false, |_| false);
        assert_eq!(first.minute, 20_000);
        assert!(!ready, "delivery must pause until the baseline is durable");
        // A saved alarm or event exists by the time the window closes. The
        // next run cannot distinguish a failed first save from a lost cursor.
        let mut saved = None;
        let reopened_at = now + 2 * 60_000;
        let (reopened, ready) = NoticeCursor::bootstrap(None, reopened_at, true, |cursor| {
            saved = Some(cursor.serialize());
            true
        });
        assert!(ready);
        assert_eq!(reopened.minute, 20_002 - MAX_CATCHUP_MINUTES);
        assert_eq!(NoticeCursor::decode(saved.as_deref().unwrap(), reopened_at), Some(reopened));
        let cal = Calendar { events: vec![], calendars: vec![] };
        let mut clock = Clock { alarms: vec![Alarm { minute: 20_001i64.rem_euclid(1440) as u16,
            days: 0, enabled: true, label: String::from("first notice") }], ..Clock::default() };
        let delivery = deliver_due(&cal, &mut clock, reopened.minute, 20_002, 0, |_| true);
        assert_eq!(delivery.delivered.len(), 1, "the failed first notice remains in catch-up");
        assert!(!clock.alarms[0].enabled);
    }

    #[test]
    fn receipt_commit_suppresses_replay_after_restart() {
        let mut journal = ReceiptJournal::default();
        assert!(!journal.record_after_ack(100, 42, |_| false));
        assert!(!journal.contains(100, 42));
        let mut saved = String::new();
        assert!(journal.record_after_ack(100, 42, |text| { saved = text.into(); true }));
        let mut reopened = ReceiptJournal::parse(&saved).unwrap();
        assert!(reopened.contains(100, 42));
        assert!(reopened.record_after_ack(100, 42, |_| panic!("duplicate must not write")));
        assert!(!reopened.contains(101, 42));
        assert!(!reopened.contains(100, 43));
        assert!(ReceiptJournal::parse("# date-receipts 1\n100 bad\n").is_none());
    }

    #[test]
    fn receipt_journal_is_bounded_and_rejects_partial_file() {
        let mut journal = ReceiptJournal::default();
        for key in 0..RECEIPT_LIMIT + 1 {
            assert!(journal.record_after_ack(100, key as u128, |_| true));
        }
        assert!(!journal.contains(100, 0));
        assert!(journal.contains(100, RECEIPT_LIMIT as u128));
        assert_eq!(ReceiptJournal::parse(&journal.serialize()), Some(journal));
        assert!(ReceiptJournal::parse("# date-receipts 1\n100 0000000000000000000000000000002a\n101 ").is_none());
    }
}
