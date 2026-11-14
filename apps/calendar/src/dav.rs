// SPDX-License-Identifier: GPL-3.0-only
//! CalDAV for Date: the direct-HTTP transport sex-dav talks through, and the mapping
//! between Date's events and sex-dav's sync items (one item per server resource, a
//! resource holding an event and the occurrences it moves).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::Window;
use sex_calendar::{ics, Calendar, Event, SyncState};
use sex_dav::dav::Kind;
use sex_dav::sync::{self, Collection, Item, State};
use sex_dav::{DavAccount, DavResponse, DavTransport, Error, FileAccount};

/// The largest reply read: a calendar's whole contents in one REPORT.
const REPLY_MAX: usize = 4 << 20;

/// Requests over plain HTTP(S) from this program, with the account file's credentials.
// TODO(Multipass): replace with a MultipassTransport over sex_pdx::accounts (OP 0x2B0)
// once the account service is in the boot catalog; then no password lives in dav.txt.
pub struct DirectHttp<'a> { pub window: &'a mut Window, pub origin: String, pub auth: Option<String> }

impl DavTransport for DirectHttp<'_> {
    fn request(&mut self, method: &str, path: &str, content_type: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<DavResponse, Error> {
        let url = format!("{}{}", self.origin, path);
        let mut all: Vec<(&str, &str)> = Vec::with_capacity(headers.len() + 3);
        all.extend_from_slice(headers);
        if let Some(a) = &self.auth { all.push(("Authorization", a.as_str())); }
        if !content_type.is_empty() { all.push(("Content-Type", content_type)); }
        let req = sex_fetch::Request { method, url: &url, headers: &all, body, trusted_origin: false, referrer: None };
        match sex_app::net::request(self.window, &req, None, REPLY_MAX) {
            Ok(r) => {
                sex_pdx::serial_record!("[date.dav.request] method={} path={} status={}", method, path, r.head.status);
                Ok(DavResponse { status: r.head.status, etag: r.head.get("etag").map(String::from), body: r.body })
            }
            Err(e) => {
                sex_pdx::serial_record!("[date.dav.request] method={} path={} error={:?}", method, path, e);
                Err(Error::Transport(e as u32))
            }
        }
    }
}

pub fn transport<'a>(window: &'a mut Window, account: &FileAccount) -> Option<(DirectHttp<'a>, String)> {
    let (origin, path) = sex_dav::split_url(account.base_url())?;
    Some((DirectHttp { window, origin: origin.into(), auth: account.auth_header() }, path.into()))
}

/// What one calendar's sync did.
pub struct Outcome { pub report: sync::Report }

fn item_state(events: &[&Event]) -> State {
    if events.iter().all(|e| e.state == SyncState::Deleted) { State::Deleted }
    else if events.iter().any(|e| e.state == SyncState::New) && events.iter().all(|e| e.href.is_empty()) { State::New }
    else if events.iter().any(|e| e.state != SyncState::Clean) { State::Modified }
    else { State::Clean }
}

/// Syncs calendar `id` (whose `url` is a collection path) both ways and puts the
/// result back into `cal`. The report's conflicts are the caller's to keep as notes.
pub fn sync_calendar(t: &mut dyn DavTransport, cal: &mut Calendar, id: u16, stamp: &str) -> Result<Outcome, Error> {
    let info = cal.info(id);
    // One item per resource: events sharing an href, or a new event alone.
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    for (i, e) in cal.events.iter().enumerate().filter(|(_, e)| e.calendar == id) {
        let key = if e.href.is_empty() { format!("uid:{}", e.uid) } else { e.href.clone() };
        match groups.iter_mut().find(|g| g.0 == key) { Some(g) => g.2.push(i), None => groups.push((key, e.uid.clone(), alloc::vec![i])) }
    }
    let mut items: Vec<Item> = groups.iter().map(|(_, uid, idx)| {
        let events: Vec<&Event> = idx.iter().map(|&i| &cal.events[i]).collect();
        Item { href: events[0].href.clone(), etag: events[0].etag.clone(), uid: uid.clone(), data: ics::export(events.iter().copied(), None, stamp), state: item_state(&events) }
    }).collect();
    let mut col = Collection { path: info.url.clone(), ctag: info.ctag.clone(), sync_token: info.sync_token.clone() };
    let report = sync::sync(t, Kind::Calendar, &mut col, &mut items)?;
    if !report.unchanged {
        cal.events.retain(|e| e.calendar != id);
        for item in &items {
            let Ok(events) = ics::import(&item.data) else { continue };
            for mut e in events {
                e.calendar = id;
                e.href = item.href.clone();
                e.etag = item.etag.clone();
                e.state = match item.state { State::Clean => SyncState::Clean, State::New => SyncState::New, State::Modified => SyncState::Modified, State::Deleted => SyncState::Deleted };
                cal.events.push(e);
            }
        }
    }
    let c = cal.info_mut(id);
    c.ctag = col.ctag;
    c.sync_token = col.sync_token;
    Ok(Outcome { report })
}
