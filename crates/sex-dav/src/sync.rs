// SPDX-License-Identifier: GPL-3.0-only
//! Two-way sync of one collection.
//!
//! 1. PROPFIND (Depth 0) reads the collection's ctag and sync-token. When the ctag is
//!    the one seen last time and nothing changed here, there is nothing to do.
//! 2. Pull: REPORT sync-collection from the last sync-token (or, without one or when
//!    the server refuses it, a calendar-query / addressbook-query for everything).
//!    A resource whose etag differs from ours replaces our copy; one gone from the
//!    server goes here too.
//! 3. Push: new items are PUT with `If-None-Match: *`, changed ones with
//!    `If-Match: <etag>`, removed ones DELETEd with `If-Match`.
//!
//! The conflict rule: the server wins, always. When our copy had changed too (an etag
//! mismatch on pull, or 412 Precondition Failed on push), the server's copy replaces
//! it and ours comes back in [`Report::conflicts`] for the program to keep as a note.
use crate::dav::{self, Kind, Multistatus};
use crate::{resolve, resource_name, unpercent, DavTransport, Error};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Clean, New, Modified, Deleted }

/// One resource as the program holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Its path on the server; empty for one made here and not yet sent.
    pub href: String,
    pub etag: String,
    pub uid: String,
    /// The iCalendar or vCard text.
    pub data: String,
    pub state: State,
}

impl Item {
    pub fn new(uid: &str, data: String) -> Item { Item { href: String::new(), etag: String::new(), uid: uid.into(), data, state: State::New } }
}

/// A collection and what was seen of it last time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collection { pub path: String, pub ctag: String, pub sync_token: String }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Nothing changed on either side.
    pub unchanged: bool,
    /// The pull used the sync-token (an incremental REPORT), not a full query.
    pub incremental: bool,
    pub pulled: usize,
    pub removed: usize,
    pub pushed: usize,
    pub deleted: usize,
    /// Pushes the server refused for another reason than a conflict; they stay
    /// pending for next time.
    pub failed: usize,
    /// Our copies of items the server's copy replaced: keep them as notes.
    pub conflicts: Vec<String>,
}

/// The UID property of an iCalendar or vCard text, folded lines joined.
pub fn find_uid(data: &str) -> String {
    let mut lines = data.split('\n').map(|l| l.trim_end_matches('\r')).peekable();
    while let Some(l) = lines.next() {
        let (name, value) = match l.split_once(':') { Some(p) => p, None => continue };
        if !name.split(';').next().unwrap_or("").eq_ignore_ascii_case("UID") { continue; }
        let mut uid = String::from(value);
        while let Some(next) = lines.peek().filter(|n| n.starts_with(' ') || n.starts_with('\t')) { uid.push_str(&next[1..]); lines.next(); }
        return String::from(uid.trim());
    }
    String::new()
}

fn same(a: &str, b: &str) -> bool { unpercent(a).trim_end_matches('/') == unpercent(b).trim_end_matches('/') }

fn multistatus(t: &mut dyn DavTransport, method: &str, path: &str, depth: &str, body: &str) -> Result<Multistatus, Error> {
    let r = t.request(method, path, "application/xml; charset=utf-8", &[("Depth", depth)], body.as_bytes())?;
    if r.status != 207 { return Err(Error::Status(r.status)); }
    dav::parse_multistatus(&String::from_utf8_lossy(&r.body))
}

/// A calendar or address book found by [`discover`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found { pub path: String, pub name: String, pub color: Option<u32> }

/// The calendars (or address books) at `path` and directly below it (Depth 1).
pub fn discover(t: &mut dyn DavTransport, path: &str, kind: Kind) -> Result<Vec<Found>, Error> {
    let ms = multistatus(t, "PROPFIND", path, "1", &dav::propfind_collections())?;
    Ok(ms.responses.into_iter().filter(|r| match kind { Kind::Calendar => r.is_calendar, Kind::Contacts => r.is_addressbook }).map(|r| {
        let path = resolve(path, &r.href);
        let name = r.display_name.filter(|n| !n.is_empty()).unwrap_or_else(|| String::from(path.trim_end_matches('/').rsplit('/').next().unwrap_or("")));
        Found { path, name, color: r.color.as_deref().and_then(dav::parse_color) }
    }).collect())
}

fn get(t: &mut dyn DavTransport, href: &str) -> Result<Option<(String, String)>, Error> {
    let r = t.request("GET", href, "", &[], b"")?;
    Ok((200..300).contains(&r.status).then(|| (r.etag.unwrap_or_default(), String::from_utf8_lossy(&r.body).into_owned())))
}

/// Puts the server's copy in place of ours, noting ours if it had changed.
fn take_server(items: &mut Vec<Item>, at: Option<usize>, href: &str, etag: String, data: String, report: &mut Report) {
    let item = Item { href: href.into(), etag, uid: find_uid(&data), data, state: State::Clean };
    match at {
        Some(i) => {
            if items[i].state == State::Modified || items[i].state == State::New { report.conflicts.push(core::mem::take(&mut items[i].data)); }
            items[i] = item;
        }
        None => items.push(item),
    }
}

/// Syncs `items` with the collection both ways. See the module's text for the rules.
pub fn sync(t: &mut dyn DavTransport, kind: Kind, col: &mut Collection, items: &mut Vec<Item>) -> Result<Report, Error> {
    let mut report = Report::default();
    let state = multistatus(t, "PROPFIND", &col.path, "0", &dav::propfind_state())?;
    let me = state.responses.iter().find(|r| same(&resolve(&col.path, &r.href), &col.path)).or(state.responses.first());
    let ctag = me.and_then(|r| r.ctag.clone()).unwrap_or_default();
    let token = me.and_then(|r| r.sync_token.clone()).unwrap_or_default();
    let local = items.iter().any(|i| i.state != State::Clean);
    let same_marker = (!ctag.is_empty() && ctag == col.ctag) || (ctag.is_empty() && !token.is_empty() && token == col.sync_token);
    if same_marker && !local {
        report.unchanged = true;
        return Ok(report);
    }

    // Pull.
    if !same_marker {
        let mut incremental = None;
        if !col.sync_token.is_empty() {
            match multistatus(t, "REPORT", &col.path, "1", &dav::sync_collection(kind, &col.sync_token)) {
                Ok(ms) => incremental = Some(ms),
                // An expired or unknown token: start again with everything.
                Err(Error::Status(_)) => {}
                Err(e) => return Err(e),
            }
        }
        report.incremental = incremental.is_some();
        let full = incremental.is_none();
        let ms = match incremental { Some(ms) => ms, None => multistatus(t, "REPORT", &col.path, "1", &dav::query(kind))? };
        let mut seen: Vec<String> = Vec::new();
        for r in &ms.responses {
            let href = resolve(&col.path, &r.href);
            if same(&href, &col.path) { continue; }
            let at = items.iter().position(|i| !i.href.is_empty() && same(&i.href, &href));
            if r.status == 404 {
                if let Some(i) = at {
                    let gone = items.remove(i);
                    if gone.state == State::Modified { report.conflicts.push(gone.data); }
                    report.removed += 1;
                }
                continue;
            }
            seen.push(href.clone());
            let etag = r.etag.clone().unwrap_or_default();
            if let Some(i) = at { if !etag.is_empty() && items[i].etag == etag { continue; } }
            let (etag, data) = match &r.data {
                Some(d) if !d.trim().is_empty() => (etag, d.clone()),
                _ => match get(t, &href)? { Some((e, d)) => (if etag.is_empty() { e } else { etag }, d), None => continue },
            };
            take_server(items, at, &href, etag, data, &mut report);
            report.pulled += 1;
        }
        if full {
            // What we hold from the server and it no longer lists is gone.
            let before = items.len();
            let mut kept = Vec::with_capacity(before);
            for i in items.drain(..) {
                if i.href.is_empty() || i.state == State::New || seen.iter().any(|s| same(s, &i.href)) { kept.push(i); continue; }
                if i.state == State::Modified { report.conflicts.push(i.data); }
                report.removed += 1;
            }
            *items = kept;
        }
        col.sync_token = ms.sync_token.filter(|t| !t.is_empty()).unwrap_or(token);
    }

    // Push.
    let mut i = 0;
    while i < items.len() {
        let state = items[i].state;
        if state == State::Clean { i += 1; continue; }
        if items[i].href.is_empty() {
            let mut path = String::from(&col.path);
            if !path.ends_with('/') { path.push('/'); }
            path.push_str(&resource_name(&items[i].uid, kind.extension()));
            items[i].href = path;
        }
        let href = items[i].href.clone();
        let etag = items[i].etag.clone();
        let r = match state {
            State::Deleted => {
                let h: &[(&str, &str)] = if etag.is_empty() { &[] } else { &[("If-Match", etag.as_str())] };
                t.request("DELETE", &href, "", h, b"")?
            }
            _ => {
                let h: [(&str, &str); 1] = if state == State::New || etag.is_empty() { [("If-None-Match", "*")] } else { [("If-Match", etag.as_str())] };
                // A changed item we hold no etag for is overwritten.
                let h: &[(&str, &str)] = if state == State::Modified && etag.is_empty() { &[] } else { &h };
                t.request("PUT", &href, kind.content_type(), h, items[i].data.as_bytes())?
            }
        };
        match (state, r.status) {
            (State::Deleted, s) if (200..300).contains(&s) || s == 404 || s == 410 => { items.remove(i); report.deleted += 1; continue; }
            (_, s) if (200..300).contains(&s) => {
                items[i].state = State::Clean;
                items[i].etag = r.etag.unwrap_or_default();
                report.pushed += 1;
            }
            (State::Modified, 404 | 410) => { let gone = items.remove(i); report.conflicts.push(gone.data); continue; }
            (_, 412) => {
                // Changed on the server meanwhile: its copy wins.
                match get(t, &href)? {
                    Some((e, d)) => {
                        let at = Some(i);
                        if state == State::Deleted { items[i].state = State::Clean; }
                        take_server(items, at, &href, e, d, &mut report);
                    }
                    None => { let gone = items.remove(i); if state != State::Deleted { report.conflicts.push(gone.data); } continue; }
                }
            }
            _ => report.failed += 1,
        }
        i += 1;
    }
    // Our own writes changed the ctag: look again next time rather than trust it.
    col.ctag = if report.pushed + report.deleted > 0 { String::new() } else { ctag };
    Ok(report)
}
