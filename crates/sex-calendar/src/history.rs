// SPDX-License-Identifier: GPL-3.0-only
//! In-session undo for Date's local event, reply-choice, and clock edits.
//! Server sync changes the event list outside this history, so Date clears it then.
use alloc::vec::Vec;
use crate::{clock::{Alarm, Clock, Timer}, Event, PartStat};

const LIMIT: usize = 32;

#[derive(Clone)]
pub struct EventEdit { pub index: usize, pub before: Option<Event>, pub after: Option<Event> }

#[derive(Default)]
pub struct EventHistory { undo: Vec<EventEdit>, redo: Vec<EventEdit> }

impl EventHistory {
    pub fn clear(&mut self) { self.undo.clear(); self.redo.clear(); }

    pub fn push(&mut self, edit: EventEdit) {
        if edit.before == edit.after { return; }
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push(edit);
        self.redo.clear();
    }

    pub fn undo(&mut self, events: &mut Vec<Event>) -> bool {
        let Some(edit) = self.undo.pop() else { return false };
        if !apply(events, &edit, true) { self.clear(); return false; }
        self.redo.push(edit);
        true
    }

    pub fn redo(&mut self, events: &mut Vec<Event>) -> bool {
        let Some(edit) = self.redo.pop() else { return false };
        if !apply(events, &edit, false) { self.clear(); return false; }
        self.undo.push(edit);
        true
    }
}

fn apply(events: &mut Vec<Event>, edit: &EventEdit, backward: bool) -> bool {
    let (current, wanted) = if backward { (&edit.after, &edit.before) } else { (&edit.before, &edit.after) };
    match (current, wanted) {
        (None, Some(value)) if edit.index <= events.len() => events.insert(edit.index, value.clone()),
        (Some(expected), None) if events.get(edit.index) == Some(expected) => { events.remove(edit.index); },
        (Some(expected), Some(value)) if events.get(edit.index) == Some(expected) => events[edit.index] = value.clone(),
        _ => return false,
    }
    true
}

/// A reply choice is reversible until Enter queues its local `.ics` file.
/// The committed reply is intentionally outside this history: a delivered
/// message cannot be recalled by undoing local calendar state.
#[derive(Clone)]
pub struct ReplyDraft { pub event: Event, pub answer: PartStat }

#[derive(Default)]
pub struct ReplyDraftHistory { current: Option<ReplyDraft>, redo: Option<ReplyDraft> }

impl ReplyDraftHistory {
    pub fn pending(&self) -> Option<&ReplyDraft> { self.current.as_ref() }
    pub fn has_redo(&self) -> bool { self.redo.is_some() }
    pub fn choose(&mut self, event: Event, answer: PartStat) {
        self.current = Some(ReplyDraft { event, answer });
        self.redo = None;
    }
    pub fn cancel(&mut self) -> bool {
        let Some(draft) = self.current.take() else { return false };
        self.redo = Some(draft);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(draft) = self.redo.take() else { return false };
        self.current = Some(draft);
        true
    }
    pub fn take(&mut self) -> Option<ReplyDraft> {
        self.redo = None;
        self.current.take()
    }
    pub fn clear(&mut self) { self.current = None; self.redo = None; }
}

/// Clock edits share one order, including timer starts and cancellations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockEditKind { Zone, Alarm, Timer }

/// A fired alarm or timer changes its own state and invalidates stale history.
#[derive(Clone, PartialEq, Eq)]
pub struct ClockState { home: alloc::string::String, zones: Vec<alloc::string::String>, alarms: Vec<Alarm>, timers: Vec<Timer> }

impl ClockState {
    pub fn from_clock(clock: &Clock) -> Self { Self { home: clock.home.clone(), zones: clock.zones.clone(), alarms: clock.alarms.clone(), timers: clock.timers.clone() } }
    fn matches(&self, clock: &Clock, kind: ClockEditKind) -> bool {
        match kind {
            ClockEditKind::Zone | ClockEditKind::Alarm =>
                self.home == clock.home && self.zones == clock.zones && self.alarms == clock.alarms,
            ClockEditKind::Timer => self.home == clock.home && self.zones == clock.zones &&
                self.alarms == clock.alarms && self.timers == clock.timers,
        }
    }
    fn restore(&self, clock: &mut Clock, kind: ClockEditKind) {
        match kind {
            ClockEditKind::Zone => { clock.home = self.home.clone(); clock.zones = self.zones.clone(); }
            ClockEditKind::Alarm => clock.alarms = self.alarms.clone(),
            ClockEditKind::Timer => clock.timers = self.timers.clone(),
        }
    }
}

#[derive(Default)]
pub struct ClockHistory { undo: Vec<(ClockState, ClockState, ClockEditKind)>, redo: Vec<(ClockState, ClockState, ClockEditKind)> }

impl ClockHistory {
    pub fn push(&mut self, before: ClockState, after: ClockState, kind: ClockEditKind) {
        if match kind {
            ClockEditKind::Zone => before.home == after.home && before.zones == after.zones,
            ClockEditKind::Alarm => before.alarms == after.alarms,
            ClockEditKind::Timer => before.timers == after.timers,
        } { return; }
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push((before, after, kind));
        self.redo.clear();
    }
    #[cfg(test)]
    fn undo(&mut self, clock: &mut Clock) -> Option<ClockEditKind> { self.undo_at(clock, 0) }
    #[cfg(test)]
    fn redo(&mut self, clock: &mut Clock) -> Option<ClockEditKind> { self.redo_at(clock, 0) }
    pub fn undo_at(&mut self, clock: &mut Clock, now_ms: u64) -> Option<ClockEditKind> { self.apply(clock, true, now_ms) }
    pub fn redo_at(&mut self, clock: &mut Clock, now_ms: u64) -> Option<ClockEditKind> { self.apply(clock, false, now_ms) }
    fn apply(&mut self, clock: &mut Clock, backward: bool, now_ms: u64) -> Option<ClockEditKind> {
        let source = if backward { &mut self.undo } else { &mut self.redo };
        let (before, after, kind) = source.pop()?;
        let (current, wanted) = if backward { (&after, &before) } else { (&before, &after) };
        if !current.matches(clock, kind) ||
            (kind == ClockEditKind::Timer && wanted.timers.iter().any(|timer| timer.ends_ms <= now_ms)) {
            self.undo.clear(); self.redo.clear();
            return None;
        }
        wanted.restore(clock, kind);
        if backward { self.redo.push((before, after, kind)); } else { self.undo.push((before, after, kind)); }
        Some(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::{Date, SyncState};

    fn event(title: &str) -> Event { Event::all_day(Date::new(2026, 3, 16).unwrap(), title) }

    #[test]
    fn reply_choice_is_reversible_only_before_commit() {
        let mut history = ReplyDraftHistory::default();
        let invite = event("review");
        history.choose(invite.clone(), PartStat::Accepted);
        assert_eq!(history.pending().unwrap().event, invite);
        assert_eq!(history.pending().unwrap().answer, PartStat::Accepted);
        assert!(history.cancel());
        assert!(history.pending().is_none());
        assert!(history.redo());
        assert_eq!(history.take().unwrap().answer, PartStat::Accepted);
        assert!(!history.redo(), "a queued choice cannot be redone");
        history.choose(invite, PartStat::Declined);
        history.cancel();
        history.choose(event("other"), PartStat::Accepted);
        assert!(!history.redo(), "a new choice replaces the old redo");
    }

    #[test]
    fn addition_deletion_and_redo_chain() {
        let mut events = vec![event("first")];
        let mut history = EventHistory::default();
        let added = event("second");
        events.push(added.clone());
        history.push(EventEdit { index: 1, before: None, after: Some(added.clone()) });
        let deleted = events.remove(0);
        history.push(EventEdit { index: 0, before: Some(deleted), after: None });
        assert!(history.undo(&mut events));
        assert_eq!(events, vec![event("first"), added.clone()]);
        assert!(history.undo(&mut events));
        assert_eq!(events, vec![event("first")]);
        assert!(history.redo(&mut events));
        assert!(history.redo(&mut events));
        assert_eq!(events, vec![added]);
    }

    #[test]
    fn new_edit_clears_redo_and_external_change_refuses_stale_history() {
        let mut events = vec![event("one")];
        let mut history = EventHistory::default();
        history.push(EventEdit { index: 0, before: None, after: Some(event("one")) });
        assert!(history.undo(&mut events));
        assert!(!history.undo(&mut events));
        events.push(event("different"));
        history.push(EventEdit { index: 0, before: None, after: Some(event("different")) });
        assert!(!history.redo(&mut events));
        events[0] = event("external");
        assert!(!history.undo(&mut events));
    }

    #[test]
    fn pending_server_deletion_restores_full_event_state() {
        let mut before = event("meeting");
        before.href = "/dav/meeting.ics".into();
        let mut after = before.clone();
        after.state = SyncState::Deleted;
        let mut events = vec![after.clone()];
        let mut history = EventHistory::default();
        history.push(EventEdit { index: 0, before: Some(before.clone()), after: Some(after.clone()) });
        assert!(history.undo(&mut events));
        assert_eq!(events[0], before);
        assert!(history.redo(&mut events));
        assert_eq!(events[0], after);
    }

    #[test]
    fn clock_history_orders_zone_and_alarm_edits_without_restoring_timers() {
        let mut clock = Clock::default();
        let mut history = ClockHistory::default();
        let before = ClockState::from_clock(&clock);
        clock.zones.push("America/Chicago".into());
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Zone);
        let before = ClockState::from_clock(&clock);
        clock.alarms.push(crate::clock::Alarm { minute: 450, days: 0, enabled: true, label: "wake".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Alarm);
        clock.timers.push(crate::clock::Timer { ends_ms: 1000, seconds: 1, label: "tea".into() });
        assert_eq!(history.undo(&mut clock), Some(ClockEditKind::Alarm));
        assert!(clock.alarms.is_empty());
        assert_eq!(history.undo(&mut clock), Some(ClockEditKind::Zone));
        assert!(clock.zones.is_empty());
        assert_eq!(clock.timers.len(), 1);
        assert_eq!(history.redo(&mut clock), Some(ClockEditKind::Zone));
        assert_eq!(clock.zones, vec!["America/Chicago"]);
        assert_eq!(history.redo(&mut clock), Some(ClockEditKind::Alarm));
        assert_eq!(clock.alarms.len(), 1);
        clock.home = "Europe/London".into();
        assert_eq!(history.undo(&mut clock), None);
        assert_eq!(clock.home, "Europe/London");
        assert_eq!(history.redo(&mut clock), None);
    }

    #[test]
    fn fired_one_time_alarm_cannot_be_restored_by_undo() {
        let mut clock = Clock::default();
        let mut history = ClockHistory::default();
        let before = ClockState::from_clock(&clock);
        clock.alarms.push(crate::clock::Alarm { minute: 450, days: 0, enabled: true, label: "wake".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Alarm);
        clock.alarms[0].enabled = false;
        assert_eq!(history.undo(&mut clock), None);
        assert!(!clock.alarms[0].enabled);
    }

    #[test]
    fn alarm_toggle_and_deletion_are_reversible_in_order() {
        let mut clock = Clock::default();
        clock.alarms.push(crate::clock::Alarm { minute: 540, days: 0, enabled: true, label: "stand up".into() });
        let mut history = ClockHistory::default();
        let before = ClockState::from_clock(&clock);
        clock.alarms[0].enabled = false;
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Alarm);
        let before = ClockState::from_clock(&clock);
        clock.alarms.remove(0);
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Alarm);
        assert_eq!(history.undo(&mut clock), Some(ClockEditKind::Alarm));
        assert!(!clock.alarms[0].enabled);
        assert_eq!(history.undo(&mut clock), Some(ClockEditKind::Alarm));
        assert!(clock.alarms[0].enabled);
        assert_eq!(history.redo(&mut clock), Some(ClockEditKind::Alarm));
        assert_eq!(history.redo(&mut clock), Some(ClockEditKind::Alarm));
        assert!(clock.alarms.is_empty());
    }

    #[test]
    fn timer_start_cancel_undo_redo_keep_deadline_and_saved_state() {
        let mut clock = Clock::default();
        let mut history = ClockHistory::default();
        let before = ClockState::from_clock(&clock);
        clock.timers.push(crate::clock::Timer { ends_ms: 90_000, seconds: 90, label: "tea".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Timer);
        assert_eq!(history.undo_at(&mut clock, 1_000), Some(ClockEditKind::Timer));
        assert!(Clock::parse(&clock.serialize()).timers.is_empty());
        assert_eq!(history.redo_at(&mut clock, 2_000), Some(ClockEditKind::Timer));
        assert_eq!(Clock::parse(&clock.serialize()).timers[0].ends_ms, 90_000);

        let before = ClockState::from_clock(&clock);
        clock.timers.remove(0);
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Timer);
        assert_eq!(history.undo_at(&mut clock, 3_000), Some(ClockEditKind::Timer));
        assert_eq!(clock.timers.len(), 1);
        assert_eq!(history.redo_at(&mut clock, 4_000), Some(ClockEditKind::Timer));
        assert!(clock.timers.is_empty());
    }

    #[test]
    fn elapsed_or_delivered_timer_cannot_be_resurrected() {
        let mut clock = Clock::default();
        let mut history = ClockHistory::default();
        let before = ClockState::from_clock(&clock);
        clock.timers.push(crate::clock::Timer { ends_ms: 5_000, seconds: 5, label: "tea".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Timer);
        assert_eq!(history.undo_at(&mut clock, 1_000), Some(ClockEditKind::Timer));
        assert_eq!(history.redo_at(&mut clock, 5_000), None);
        assert!(clock.timers.is_empty());

        let before = ClockState::from_clock(&clock);
        clock.timers.push(crate::clock::Timer { ends_ms: 10_000, seconds: 10, label: "tea".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Timer);
        clock.deliver_timers(10_000, |_| true);
        assert_eq!(history.undo_at(&mut clock, 11_000), None);
        assert!(clock.timers.is_empty());
    }

    #[test]
    fn timer_and_alarm_edits_undo_in_one_order() {
        let mut clock = Clock::default();
        let mut history = ClockHistory::default();
        let before = ClockState::from_clock(&clock);
        clock.timers.push(crate::clock::Timer { ends_ms: 60_000, seconds: 60, label: "tea".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Timer);
        let before = ClockState::from_clock(&clock);
        clock.alarms.push(crate::clock::Alarm { minute: 450, days: 0, enabled: true, label: "wake".into() });
        history.push(before, ClockState::from_clock(&clock), ClockEditKind::Alarm);
        assert_eq!(history.undo_at(&mut clock, 1_000), Some(ClockEditKind::Alarm));
        assert_eq!(clock.timers.len(), 1);
        assert_eq!(history.undo_at(&mut clock, 1_000), Some(ClockEditKind::Timer));
        assert!(clock.timers.is_empty());
        assert_eq!(history.redo_at(&mut clock, 2_000), Some(ClockEditKind::Timer));
        assert_eq!(history.redo_at(&mut clock, 2_000), Some(ClockEditKind::Alarm));
    }
}
