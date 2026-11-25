// SPDX-License-Identifier: GPL-3.0-only
//! Bounded, in-session undo for Date's local event additions and deletions.
//! Server sync changes the event list outside this history, so Date clears it then.
use alloc::vec::Vec;
use crate::{clock::Clock, Event};

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

/// Only the world-zone list and home zone participate. Alarms and timers can
/// fire between keystrokes, so a whole-clock snapshot would resurrect them.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneState { home: alloc::string::String, zones: Vec<alloc::string::String> }

impl ZoneState {
    pub fn from_clock(clock: &Clock) -> Self { Self { home: clock.home.clone(), zones: clock.zones.clone() } }
    fn restore(&self, clock: &mut Clock) { clock.home = self.home.clone(); clock.zones = self.zones.clone(); }
}

#[derive(Default)]
pub struct ZoneHistory { undo: Vec<(ZoneState, ZoneState)>, redo: Vec<(ZoneState, ZoneState)> }

impl ZoneHistory {
    pub fn push(&mut self, before: ZoneState, after: ZoneState) {
        if before == after { return; }
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push((before, after));
        self.redo.clear();
    }
    pub fn undo(&mut self, clock: &mut Clock) -> bool { self.apply(clock, true) }
    pub fn redo(&mut self, clock: &mut Clock) -> bool { self.apply(clock, false) }
    fn apply(&mut self, clock: &mut Clock, backward: bool) -> bool {
        let source = if backward { &mut self.undo } else { &mut self.redo };
        let Some((before, after)) = source.pop() else { return false };
        let (current, wanted) = if backward { (&after, &before) } else { (&before, &after) };
        if ZoneState::from_clock(clock) != *current {
            self.undo.clear(); self.redo.clear();
            return false;
        }
        wanted.restore(clock);
        if backward { self.redo.push((before, after)); } else { self.undo.push((before, after)); }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use crate::{Date, SyncState};

    fn event(title: &str) -> Event { Event::all_day(Date::new(2026, 3, 16).unwrap(), title) }

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
    fn zone_history_keeps_live_timers_and_refuses_stale_zones() {
        let mut clock = Clock::default();
        let mut history = ZoneHistory::default();
        let before = ZoneState::from_clock(&clock);
        clock.zones.push("America/Chicago".into());
        history.push(before, ZoneState::from_clock(&clock));
        clock.timers.push(crate::clock::Timer { ends_ms: 1000, seconds: 1, label: "tea".into() });
        assert!(history.undo(&mut clock));
        assert!(clock.zones.is_empty());
        assert_eq!(clock.timers.len(), 1);
        assert!(history.redo(&mut clock));
        assert_eq!(clock.zones, vec!["America/Chicago"]);
        clock.home = "Europe/London".into();
        assert!(!history.undo(&mut clock));
        assert_eq!(clock.home, "Europe/London");
        assert!(!history.redo(&mut clock));
    }
}
