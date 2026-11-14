// SPDX-License-Identifier: GPL-3.0-only
//! The XML reader, the request bodies and two-way sync against a small in-memory
//! CalDAV/CardDAV server that keeps ctags, etags and sync-tokens the way real ones do.
use sex_dav::dav::{self, Kind};
use sex_dav::sync::{self, Collection, Item, State};
use sex_dav::{xml, DavAccount, DavResponse, DavTransport, Error, FileAccount};
use std::collections::BTreeMap;

#[derive(Default)]
struct Server {
    /// path → (etag number, data)
    items: BTreeMap<String, (u32, String)>,
    /// Every change: (sync-token number, path, removed).
    log: Vec<(u32, String, bool)>,
    version: u32,
    collection: String,
    supports_sync: bool,
    requests: Vec<String>,
}

fn tag(n: u32) -> String { format!("\"e{n}\"") }

impl Server {
    fn new(collection: &str, supports_sync: bool) -> Server { Server { collection: collection.into(), supports_sync, ..Default::default() } }
    fn put_here(&mut self, name: &str, data: &str) {
        self.version += 1;
        let path = format!("{}{name}", self.collection);
        self.items.insert(path.clone(), (self.version, data.into()));
        self.log.push((self.version, path, false));
    }
    fn delete_here(&mut self, name: &str) {
        self.version += 1;
        let path = format!("{}{name}", self.collection);
        self.items.remove(&path);
        self.log.push((self.version, path, true));
    }
    fn response(&self, path: &str, props: &str) -> String { format!("<d:response><d:href>{}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>", path.replace('@', "%40")) }
    fn ms(&self, inner: &str, token: bool) -> DavResponse {
        let t = if token { format!("<d:sync-token>http://x/sync/{}</d:sync-token>", self.version) } else { String::new() };
        DavResponse { status: 207, etag: None, body: format!("<?xml version=\"1.0\"?>\n<d:multistatus xmlns:d=\"DAV:\" xmlns:cs=\"http://calendarserver.org/ns/\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">{inner}{t}</d:multistatus>").into_bytes() }
    }
    fn data(&self, (n, d): &(u32, String)) -> String { format!("<d:getetag>{}</d:getetag><c:calendar-data><![CDATA[{d}]]></c:calendar-data>", xml::escape(&tag(*n))) }
}

impl DavTransport for Server {
    fn request(&mut self, method: &str, path: &str, _ct: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<DavResponse, Error> {
        let h = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_string());
        let body = String::from_utf8_lossy(body).into_owned();
        self.requests.push(format!("{method} {path}"));
        let path = sex_dav::unpercent(path);
        match method {
            "PROPFIND" if h("Depth").as_deref() == Some("1") => {
                let me = self.response("/dav/", "<d:resourcetype><d:collection/></d:resourcetype>");
                let cal = self.response(&self.collection, "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Home &amp; work</d:displayname><x:calendar-color xmlns:x=\"http://apple.com/ns/ical/\">#3574E8FF</x:calendar-color>");
                Ok(self.ms(&(me + &cal), false))
            }
            "PROPFIND" => {
                let token = if self.supports_sync { format!("<d:sync-token>http://x/sync/{}</d:sync-token>", self.version) } else { String::new() };
                let inner = self.response(&self.collection, &format!("<cs:getctag>c{}</cs:getctag>{token}", self.version));
                Ok(self.ms(&inner, false))
            }
            "REPORT" if body.contains("sync-collection") => {
                if !self.supports_sync { return Ok(DavResponse { status: 501, ..Default::default() }); }
                let from: u32 = body.split("sync/").nth(1).and_then(|s| s.split('<').next()).and_then(|s| s.parse().ok()).unwrap_or(0);
                let mut inner = String::new();
                let mut done = Vec::new();
                for (v, p, removed) in self.log.iter().rev() {
                    if *v <= from || done.contains(p) { continue; }
                    done.push(p.clone());
                    if *removed { inner += &format!("<d:response><d:href>{p}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>"); }
                    else if let Some(it) = self.items.get(p) { inner += &self.response(p, &self.data(it)); }
                }
                Ok(self.ms(&inner, true))
            }
            "REPORT" => {
                let inner: String = self.items.iter().map(|(p, it)| self.response(p, &self.data(it))).collect();
                Ok(self.ms(&inner, false))
            }
            "GET" => Ok(match self.items.get(&path) {
                Some((n, d)) => DavResponse { status: 200, etag: Some(tag(*n)), body: d.clone().into_bytes() },
                None => DavResponse { status: 404, ..Default::default() },
            }),
            "PUT" => {
                let current = self.items.get(&path).map(|(n, _)| tag(*n));
                if h("If-None-Match").is_some() && current.is_some() { return Ok(DavResponse { status: 412, ..Default::default() }); }
                if let Some(m) = h("If-Match") { if current.as_deref() != Some(m.as_str()) { return Ok(DavResponse { status: 412, ..Default::default() }); } }
                let name = path.strip_prefix(&self.collection).unwrap().to_string();
                self.put_here(&name, &body);
                Ok(DavResponse { status: if current.is_some() { 204 } else { 201 }, etag: Some(tag(self.version)), body: Vec::new() })
            }
            "DELETE" => {
                let current = self.items.get(&path).map(|(n, _)| tag(*n));
                let Some(cur) = current else { return Ok(DavResponse { status: 404, ..Default::default() }) };
                if let Some(m) = h("If-Match") { if cur != m { return Ok(DavResponse { status: 412, ..Default::default() }); } }
                let name = path.strip_prefix(&self.collection).unwrap().to_string();
                self.delete_here(&name);
                Ok(DavResponse { status: 204, ..Default::default() })
            }
            _ => Ok(DavResponse { status: 405, ..Default::default() }),
        }
    }
}

fn ev(uid: &str, title: &str) -> String { format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:20261120T090000Z\r\nSUMMARY:{title}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n") }
fn titles(items: &[Item]) -> Vec<String> {
    let mut t: Vec<String> = items.iter().map(|i| i.data.split("SUMMARY:").nth(1).unwrap().split('\r').next().unwrap().to_string()).collect();
    t.sort();
    t
}

fn both_ways(supports_sync: bool) {
    let mut s = Server::new("/dav/cal/me@x/home/", supports_sync);
    s.put_here("a.ics", &ev("a", "From server"));
    let found = sync::discover(&mut s, "/dav/", Kind::Calendar).unwrap();
    assert_eq!(found, [sync::Found { path: "/dav/cal/me@x/home/".into(), name: "Home & work".into(), color: Some(0x3574E8) }]);
    let mut col = Collection { path: found[0].path.clone(), ..Default::default() };
    let mut items = vec![Item::new("b@here", ev("b@here", "Made here"))];
    let r = sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    assert_eq!((r.pulled, r.pushed, r.incremental), (1, 1, false));
    assert_eq!(titles(&items), ["From server", "Made here"]);
    assert!(items.iter().all(|i| i.state == State::Clean && !i.etag.is_empty()));
    assert_eq!(items[0].uid, "b@here");
    assert_eq!(s.items.get("/dav/cal/me@x/home/b@here.ics").unwrap().1, ev("b@here", "Made here"));
    // Nothing new on either side: one PROPFIND and done (once the ctag is settled).
    sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    let n = s.requests.len();
    assert!(sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap().unchanged);
    assert_eq!(s.requests.len(), n + 1);
    // A change and a removal on the server come down.
    s.put_here("a.ics", &ev("a", "Changed on server"));
    s.put_here("c.ics", &ev("c", "Third"));
    s.delete_here("b@here.ics");
    let r = sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    assert_eq!((r.pulled, r.removed, r.incremental), (2, 1, supports_sync));
    assert_eq!(titles(&items), ["Changed on server", "Third"]);
    // Changed here and there: the server wins, ours comes back as a conflict note.
    let a = items.iter().position(|i| i.uid == "a").unwrap();
    items[a].data = ev("a", "Changed here");
    items[a].state = State::Modified;
    s.put_here("a.ics", &ev("a", "Changed there"));
    let r = sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    assert_eq!(r.conflicts, [ev("a", "Changed here")]);
    assert_eq!(titles(&items), ["Changed there", "Third"]);
    // A change here alone goes up with If-Match; a removal here is deleted there.
    let c = items.iter().position(|i| i.uid == "c").unwrap();
    items[c].data = ev("c", "Third, edited");
    items[c].state = State::Modified;
    let a = items.iter().position(|i| i.uid == "a").unwrap();
    items[a].state = State::Deleted;
    let r = sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    assert_eq!((r.pushed, r.deleted, r.conflicts.len()), (1, 1, 0));
    assert_eq!(s.items.keys().collect::<Vec<_>>(), ["/dav/cal/me@x/home/c.ics"]);
    assert_eq!(titles(&items), ["Third, edited"]);
}

#[test]
fn sync_both_ways_with_sync_token() { both_ways(true); }
#[test]
fn sync_both_ways_with_ctag_and_full_queries() { both_ways(false); }

#[test]
fn push_conflict_412_keeps_note() {
    let mut s = Server::new("/c/", false);
    s.put_here("x.ics", &ev("x", "One"));
    let mut col = Collection { path: "/c/".into(), ..Default::default() };
    let mut items = Vec::new();
    sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    // The server changes without its ctag moving (a lazy server); our If-Match fails.
    s.items.get_mut("/c/x.ics").unwrap().0 = 99;
    s.items.get_mut("/c/x.ics").unwrap().1 = ev("x", "Server's");
    col.ctag = format!("c{}", s.version);
    items[0].data = ev("x", "Ours");
    items[0].state = State::Modified;
    let r = sync::sync(&mut s, Kind::Calendar, &mut col, &mut items).unwrap();
    assert_eq!((r.pushed, r.conflicts.clone()), (0, vec![ev("x", "Ours")]));
    assert_eq!((titles(&items), items[0].state, items[0].etag.as_str()), (vec!["Server's".to_string()], State::Clean, "\"e99\""));
}

#[test]
fn xml_reader() {
    let doc = xml::parse("<?xml version='1.0'?><!-- hi --><a:x xmlns:a=\"urn:a\" xmlns=\"urn:d\"><y k='v &amp; w'>t&lt;1&#x41;&#66;</y><a:z/><![CDATA[<raw>]]></a:x>").unwrap();
    assert!(doc.is("urn:a", "x"));
    let y = doc.child("urn:d", "y").unwrap();
    assert_eq!((y.text().as_str(), y.attrs[0].1.as_str()), ("t<1AB", "v & w"));
    assert!(doc.child("urn:a", "z").is_some());
    assert_eq!(doc.text(), "t<1AB<raw>");
    assert_eq!(xml::parse("<a><b></a></b>"), Err(xml::Error::Unbalanced));
    assert_eq!(xml::parse("<!DOCTYPE x [<!ENTITY e 'boom'>]><x>&e;</x>"), Err(xml::Error::Doctype));
    assert_eq!(xml::parse("<p:x/>"), Err(xml::Error::Syntax));
    assert_eq!(xml::parse("<x>&bogus;</x>"), Err(xml::Error::Syntax));
    let deep = "<a>".repeat(100) + &"</a>".repeat(100);
    assert_eq!(xml::parse(&deep), Err(xml::Error::TooDeep));
}

#[test]
fn bodies_and_multistatus() {
    for body in [dav::propfind_state(), dav::propfind_collections(), dav::query(Kind::Calendar), dav::query(Kind::Contacts), dav::sync_collection(Kind::Contacts, "t<1>")] {
        xml::parse(&body).unwrap();
    }
    assert!(dav::query(Kind::Contacts).contains("addressbook-query") && dav::sync_collection(Kind::Contacts, "t<1>").contains("t&lt;1&gt;"));
    let ms = dav::parse_multistatus(r#"<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
      <response><href>/ab/a%20b.vcf</href><propstat><prop><getetag>"1"</getetag><C:address-data>BEGIN:VCARD&#13;
UID:1&#13;
END:VCARD&#13;
</C:address-data></prop><status>HTTP/1.1 200 OK</status></propstat>
      <propstat><prop><displayname/></prop><status>HTTP/1.1 404 Not Found</status></propstat></response>
      <response><href>/ab/gone.vcf</href><status>HTTP/1.1 404 Not Found</status></response>
      <sync-token>tok-2</sync-token></multistatus>"#).unwrap();
    assert_eq!(ms.sync_token.as_deref(), Some("tok-2"));
    assert_eq!((ms.responses[0].href.as_str(), ms.responses[0].status, ms.responses[0].etag.as_deref(), ms.responses[0].display_name.as_deref()), ("/ab/a b.vcf", 200, Some("\"1\""), None));
    assert_eq!(sync::find_uid(ms.responses[0].data.as_deref().unwrap()), "1");
    assert_eq!(ms.responses[1].status, 404);
}

#[test]
fn accounts_and_addresses() {
    let a = FileAccount::parse("# mine\nurl=http://10.0.2.2:8080/dav/\nuser=Aladdin\npassword=open sesame\nemail=me@x\n").unwrap();
    assert_eq!(a.auth_header().as_deref(), Some("Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="));
    assert_eq!(a.base_url(), "http://10.0.2.2:8080/dav/");
    assert!(FileAccount::parse("url=ftp://x/").is_none());
    assert_eq!(sex_dav::split_url("https://h:1/a/b"), Some(("https://h:1", "/a/b")));
    assert_eq!(sex_dav::split_url("http://h"), Some(("http://h", "/")));
    assert_eq!(sex_dav::resolve("/a/b/", "c.ics"), "/a/b/c.ics");
    assert_eq!(sex_dav::resolve("/a/b/", "http://h/x/"), "/x/");
    assert_eq!(sex_dav::resource_name("a b/c@d", ".vcf"), "a_b_c@d.vcf");
    assert_eq!(sex_dav::base64(b"ab"), "YWI=");
    assert_eq!(sync::find_uid("BEGIN:VEVENT\r\nUID:abc\r\n def\r\nEND:VEVENT"), "abcdef");
}
