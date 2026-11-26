// SPDX-License-Identifier: GPL-3.0-only
//! Review the losing local copies Date keeps when CalDAV reports a conflict.
use alloc::vec::Vec;
use crate::{Calendar, SyncState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice { Local, Server }

fn is_note(cal: &Calendar, index: usize) -> bool {
    let Some(event) = cal.events.get(index) else { return false };
    event.uid.ends_with("-conflict") && cal.calendars.iter().any(|c| {
        c.id == event.calendar && c.name == "Conflicts" && c.url.is_empty()
    })
}

/// The next retained local copy, ordered by date and then UID. Wraps after the last.
/// Returns its event index, one-based position and total count.
pub fn next(cal: &Calendar, after: Option<usize>) -> Option<(usize, usize, usize)> {
    let mut notes: Vec<usize> = (0..cal.events.len()).filter(|&i| is_note(cal, i)).collect();
    notes.sort_by(|&a, &b| {
        let a = &cal.events[a];
        let b = &cal.events[b];
        (a.start, &a.uid).cmp(&(b.start, &b.uid))
    });
    if notes.is_empty() { return None; }
    let position = after.and_then(|old| notes.iter().position(|&i| i == old)).map_or(0, |n| (n + 1) % notes.len());
    Some((notes[position], position + 1, notes.len()))
}

/// The winning server event, when it remains in the synced calendar.
pub fn server_copy(cal: &Calendar, note_index: usize) -> Option<usize> {
    let note = cal.events.get(note_index)?;
    if !is_note(cal, note_index) { return None; }
    let uid = note.uid.strip_suffix("-conflict")?;
    let source_calendar = note.conflict_source_calendar?;
    if note.conflict_source_href.is_empty() || !cal.calendars.iter().any(|c| c.id == source_calendar && !c.url.is_empty()) { return None; }
    let mut matches = cal.events.iter().enumerate().filter(|(_, e)| e.uid == uid
        && e.recurrence_id == note.recurrence_id && e.calendar == source_calendar
        && e.href == note.conflict_source_href && e.state != SyncState::Deleted);
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

/// Prepare a resolved calendar without touching the source. The caller must
/// stage and sync this complete copy before replacing its in-memory calendar.
pub fn resolve(cal: &Calendar, note_index: usize, choice: Choice) -> Result<Calendar, &'static str> {
    if !is_note(cal, note_index) { return Err("This is not a retained CalDAV conflict"); }
    let server = server_copy(cal, note_index);
    if choice == Choice::Local && server.is_none() {
        return Err("No unique matching server event remains; the local copy cannot replace it");
    }
    let mut next = cal.clone();
    if let (Choice::Local, Some(index)) = (choice, server) {
        let mut local = next.events[note_index].clone();
        let current = &next.events[index];
        local.uid = current.uid.clone();
        local.calendar = current.calendar;
        local.href = current.href.clone();
        local.etag = current.etag.clone();
        local.conflict_source_calendar = None;
        local.conflict_source_href.clear();
        local.title = local.title.strip_prefix("Conflict: ").unwrap_or(&local.title).into();
        local.state = SyncState::Modified;
        next.events[index] = local;
    }
    next.events.remove(note_index);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Date, Event};

    #[test]
    fn review_cycles_local_notes_and_pairs_server_versions() {
        let mut cal = Calendar::default();
        let server = cal.add_calendar("Home", "/dav/home/");
        let notes = cal.add_calendar("Conflicts", "");
        let mut remote = Event::default();
        remote.uid = "a".into(); remote.title = "Server title".into();
        remote.calendar = server; remote.href = "/dav/home/a.ics".into();
        remote.start = Date::new(2026, 11, 25).unwrap();
        cal.events.push(remote);
        for (uid, day) in [("b-conflict", 26), ("a-conflict", 25)] {
            let mut e = Event::default();
            e.uid = uid.into(); e.title = "Conflict: local title".into();
            e.calendar = notes; e.start = Date::new(2026, 11, day).unwrap();
            if uid == "a-conflict" { e.conflict_source_calendar = Some(server); e.conflict_source_href = "/dav/home/a.ics".into(); }
            cal.events.push(e);
        }
        assert_eq!(next(&cal, None), Some((2, 1, 2)));
        assert_eq!(server_copy(&cal, 2), Some(0));
        assert_eq!(next(&cal, Some(2)), Some((1, 2, 2)));
        assert_eq!(server_copy(&cal, 1), None);
        assert_eq!(next(&cal, Some(1)), Some((2, 1, 2)));
        cal.events[2].calendar = server;
        assert_eq!(next(&cal, None), Some((1, 1, 1)));
    }

    #[test]
    fn local_resolution_reuses_current_server_identity_and_server_choice_keeps_server() {
        let mut cal = Calendar::default();
        let home = cal.add_calendar("Home", "/dav/home/");
        let notes = cal.add_calendar("Conflicts", "");
        let mut remote = Event::default();
        remote.uid = "visit".into(); remote.title = "Server revised".into();
        remote.calendar = home; remote.href = "/dav/home/visit.ics".into(); remote.etag = "new-etag".into();
        cal.events.push(remote.clone());
        let mut local = remote.clone();
        local.uid = "visit-conflict".into(); local.title = "Conflict: Local choice".into();
        local.calendar = notes; local.href.clear(); local.etag.clear();
        local.conflict_source_calendar = Some(home); local.conflict_source_href = remote.href.clone();
        cal.events.push(local);
        let kept = resolve(&cal, 1, Choice::Local).unwrap();
        assert_eq!(kept.events.len(), 1);
        assert_eq!(kept.events[0].title, "Local choice");
        assert_eq!(kept.events[0].href, remote.href);
        assert_eq!(kept.events[0].etag, remote.etag);
        assert_eq!(kept.events[0].state, SyncState::Modified);
        assert_eq!(kept.events[0].conflict_source_calendar, None);
        let server = resolve(&cal, 1, Choice::Server).unwrap();
        assert_eq!(server.events, alloc::vec![remote]);
        assert_eq!(cal.events.len(), 2, "resolution must stage a copy");
        cal.events.remove(0);
        assert!(resolve(&cal, 0, Choice::Local).is_err());
        assert!(resolve(&cal, 0, Choice::Server).unwrap().events.is_empty());
        let mut ambiguous = Calendar::default();
        let home = ambiguous.add_calendar("Home", "/dav/home/");
        let other = ambiguous.add_calendar("Other", "/dav/other/");
        let notes = ambiguous.add_calendar("Conflicts", "");
        let mut event = Event::default(); event.uid = "same".into(); event.href = "/dav/same.ics".into();
        event.calendar = home; ambiguous.events.push(event.clone());
        event.calendar = other; ambiguous.events.push(event.clone());
        event.uid = "same-conflict".into(); event.calendar = notes;
        event.conflict_source_calendar = Some(home); event.conflict_source_href = "/dav/same.ics".into(); ambiguous.events.push(event);
        assert_eq!(server_copy(&ambiguous, 2), Some(0));
        ambiguous.events.remove(0);
        assert!(resolve(&ambiguous, 1, Choice::Local).is_err(), "a matching UID in another calendar must not be overwritten");
        assert_eq!(resolve(&ambiguous, 1, Choice::Server).unwrap().events.len(), 1);
        let reloaded = Calendar::parse(&ambiguous.serialize());
        assert_eq!(reloaded.events[1].conflict_source_calendar, Some(home));
        assert_eq!(reloaded.events[1].conflict_source_href, "/dav/same.ics");
    }
}
