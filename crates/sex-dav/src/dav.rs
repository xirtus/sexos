// SPDX-License-Identifier: GPL-3.0-only
//! Request bodies and multistatus replies.
use crate::xml::{self, Element};
use crate::{unpercent, Error};
use alloc::string::String;
use alloc::vec::Vec;

pub const DAV: &str = "DAV:";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
pub const CS: &str = "http://calendarserver.org/ns/";
pub const APPLE: &str = "http://apple.com/ns/ical/";

/// Calendars (VEVENTs in `.ics` resources) or address books (vCards in `.vcf`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Calendar, Contacts }

impl Kind {
    pub fn content_type(self) -> &'static str { match self { Kind::Calendar => "text/calendar; charset=utf-8", Kind::Contacts => "text/vcard; charset=utf-8" } }
    pub fn extension(self) -> &'static str { match self { Kind::Calendar => ".ics", Kind::Contacts => ".vcf" } }
    fn data_prop(self) -> &'static str { match self { Kind::Calendar => "<C:calendar-data/>", Kind::Contacts => "<C:address-data/>" } }
    fn ns(self) -> &'static str { match self { Kind::Calendar => CALDAV, Kind::Contacts => CARDDAV } }
}

const HEAD: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";

/// PROPFIND for a collection's change markers (Depth 0): getctag, sync-token,
/// displayname, resourcetype.
pub fn propfind_state() -> String {
    alloc::format!("{HEAD}<D:propfind xmlns:D=\"DAV:\" xmlns:CS=\"{CS}\"><D:prop><D:resourcetype/><D:displayname/><CS:getctag/><D:sync-token/></D:prop></D:propfind>")
}

/// PROPFIND for discovery (Depth 1 on the account's address): which children are
/// calendars or address books, their names and colours.
pub fn propfind_collections() -> String {
    alloc::format!("{HEAD}<D:propfind xmlns:D=\"DAV:\" xmlns:CS=\"{CS}\" xmlns:A=\"{APPLE}\"><D:prop><D:resourcetype/><D:displayname/><A:calendar-color/><CS:getctag/></D:prop></D:propfind>")
}

/// REPORT calendar-query for every VEVENT, or addressbook-query for every vCard,
/// with each resource's etag and data (Depth 1).
pub fn query(kind: Kind) -> String {
    match kind {
        Kind::Calendar => alloc::format!("{HEAD}<C:calendar-query xmlns:D=\"DAV:\" xmlns:C=\"{CALDAV}\"><D:prop><D:getetag/><C:calendar-data/></D:prop><C:filter><C:comp-filter name=\"VCALENDAR\"><C:comp-filter name=\"VEVENT\"/></C:comp-filter></C:filter></C:calendar-query>"),
        Kind::Contacts => alloc::format!("{HEAD}<C:addressbook-query xmlns:D=\"DAV:\" xmlns:C=\"{CARDDAV}\"><D:prop><D:getetag/><C:address-data/></D:prop></C:addressbook-query>"),
    }
}

/// REPORT sync-collection (RFC 6578) from `token` ("" for the first time).
pub fn sync_collection(kind: Kind, token: &str) -> String {
    alloc::format!("{HEAD}<D:sync-collection xmlns:D=\"DAV:\" xmlns:C=\"{}\"><D:sync-token>{}</D:sync-token><D:sync-level>1</D:sync-level><D:prop><D:getetag/>{}</D:prop></D:sync-collection>",
        kind.ns(), xml::escape(token), kind.data_prop())
}

/// One `<D:response>` of a multistatus: a resource and the properties found on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// The href as sent, percent-decoded.
    pub href: String,
    /// The status of the response itself (404 in a sync-collection is a removal), or
    /// of its found properties.
    pub status: u16,
    pub etag: Option<String>,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub display_name: Option<String>,
    pub color: Option<String>,
    /// Calendar data or address data.
    pub data: Option<String>,
    pub is_calendar: bool,
    pub is_addressbook: bool,
    pub is_collection: bool,
}

/// A multistatus: its responses and the collection's new sync-token, if given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Multistatus { pub responses: Vec<Response>, pub sync_token: Option<String> }

/// `HTTP/1.1 404 Not Found` → 404.
fn status_code(text: &str) -> u16 { text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0) }

pub fn parse_multistatus(body: &str) -> Result<Multistatus, Error> {
    let root = xml::parse(body)?;
    if !root.is(DAV, "multistatus") { return Err(Error::Xml(xml::Error::Syntax)); }
    let mut out = Multistatus { responses: Vec::new(), sync_token: root.child(DAV, "sync-token").map(|t| String::from(t.text().trim())) };
    for r in root.children_named(DAV, "response") {
        let Some(href) = r.child(DAV, "href") else { continue };
        let mut resp = Response { href: unpercent(href.text().trim()), status: r.child(DAV, "status").map_or(0, |s| status_code(&s.text())), ..Response::default() };
        for ps in r.children_named(DAV, "propstat") {
            let code = ps.child(DAV, "status").map_or(200, |s| status_code(&s.text()));
            if !(200..300).contains(&code) { continue; }
            if resp.status == 0 { resp.status = code; }
            let Some(prop) = ps.child(DAV, "prop") else { continue };
            read_props(prop, &mut resp);
        }
        if resp.status == 0 { resp.status = 200; }
        out.responses.push(resp);
    }
    Ok(out)
}

fn read_props(prop: &Element, r: &mut Response) {
    let text = |e: &Element| String::from(e.text().trim());
    for p in prop.elements() {
        match (p.ns.as_str(), p.name.as_str()) {
            (DAV, "getetag") => r.etag = Some(text(p)),
            (CS, "getctag") => r.ctag = Some(text(p)),
            (DAV, "sync-token") => r.sync_token = Some(text(p)),
            (DAV, "displayname") => r.display_name = Some(text(p)),
            (APPLE, "calendar-color") => r.color = Some(text(p)),
            // Data keeps its own line breaks: only the XML around it is trimmed.
            (CALDAV, "calendar-data") | (CARDDAV, "address-data") => r.data = Some(String::from(p.text().trim_start_matches(['\r', '\n', ' ']))),
            (DAV, "resourcetype") => {
                r.is_collection = p.child(DAV, "collection").is_some();
                r.is_calendar = p.child(CALDAV, "calendar").is_some();
                r.is_addressbook = p.child(CARDDAV, "addressbook").is_some();
            }
            _ => {}
        }
    }
}

/// `#RRGGBB` or `#RRGGBBAA` (Apple's calendar-color) as 0xRRGGBB.
pub fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 { return None; }
    u32::from_str_radix(&hex[..6], 16).ok()
}
