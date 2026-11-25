// SPDX-License-Identifier: GPL-3.0-only
//! Review the losing local copies Date keeps when CalDAV reports a conflict.
use alloc::vec::Vec;
use crate::{Calendar, SyncState};

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
    cal.events.iter().position(|e| e.uid == uid && e.calendar != note.calendar
        && !e.href.is_empty() && e.state != SyncState::Deleted)
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
}
