// SPDX-License-Identifier: GPL-3.0-only
//! Contacts for Crush: vCard 3.0 (RFC 2426) and 4.0 (RFC 6350) read and written, an
//! address book that searches, sorts and keeps each card's CardDAV state, all tested
//! on the host.
//!
//! Reading unfolds lines, resolves escapes, takes `TYPE=` in both versions' forms
//! (`TYPE=work,voice`, `TYPE=WORK;TYPE=CELL`, bare v2.1 `WORK`), ignores group
//! prefixes (`item1.EMAIL`), decodes v2.1 QUOTED-PRINTABLE, and keeps any property it
//! does not know verbatim so writing the card back loses nothing.
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// A value with its kinds (`work`, `home`, `cell`...), lower case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typed { pub kinds: Vec<String>, pub value: String }

impl Typed {
    pub fn new(kind: &str, value: &str) -> Typed { Typed { kinds: if kind.is_empty() { Vec::new() } else { alloc::vec![kind.into()] }, value: value.into() } }
    pub fn label(&self) -> &str { self.kinds.iter().find(|k| !matches!(k.as_str(), "internet" | "pref" | "voice")).map_or("", |k| k.as_str()) }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Address { pub kinds: Vec<String>, pub po_box: String, pub extended: String, pub street: String, pub city: String, pub region: String, pub code: String, pub country: String }

impl Address {
    /// One line: street, code city, region, country (the empty parts left out).
    pub fn line(&self) -> String {
        let city = [self.code.as_str(), self.city.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        [self.po_box.as_str(), self.extended.as_str(), self.street.as_str(), city.as_str(), self.region.as_str(), self.country.as_str()]
            .iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ")
    }
}

/// Where a card stands against its server copy (CardDAV).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncState { #[default] Clean, New, Modified, Deleted }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub uid: String,
    /// FN: the name as shown.
    pub name: String,
    /// N: family, given, additional, prefixes, suffixes.
    pub n: [String; 5],
    pub nickname: String,
    pub org: String,
    pub title: String,
    pub emails: Vec<Typed>,
    pub phones: Vec<Typed>,
    pub addresses: Vec<Address>,
    pub urls: Vec<String>,
    /// As written: `1990-03-16`, `19900316` or `--0316`.
    pub birthday: String,
    pub note: String,
    pub categories: Vec<String>,
    /// Properties not read, written back as they came (unfolded).
    pub other: Vec<String>,
    /// 3 or 4: the version read, and the one written back.
    pub version: u8,
    pub href: String,
    pub etag: String,
    pub state: SyncState,
}

impl Contact {
    pub fn new(name: &str) -> Contact { Contact { name: name.into(), version: 4, ..Contact::default() } }
    /// The name to show: FN, else N put together, else the organisation or first email.
    pub fn display(&self) -> String {
        if !self.name.trim().is_empty() { return self.name.clone(); }
        let n: Vec<&str> = [&self.n[3], &self.n[1], &self.n[2], &self.n[0], &self.n[4]].iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
        if !n.is_empty() { return n.join(" "); }
        if !self.org.is_empty() { return self.org.clone(); }
        self.emails.first().map(|e| e.value.clone()).unwrap_or_else(|| String::from("(no name)"))
    }
    /// The key to sort by: family name if known, then the shown name, ignoring case.
    pub fn sort_key(&self) -> String {
        let mut k = if self.n[0].is_empty() { self.display() } else { alloc::format!("{} {}", self.n[0], self.n[1]) };
        k.make_ascii_lowercase();
        k
    }
    /// Whether every word of `query` is in the name, an email, a phone number (digits
    /// compared alone), the organisation, a nickname or a note, ignoring case.
    pub fn matches(&self, query: &str) -> bool {
        query.split_whitespace().all(|word| {
            let w = word.to_ascii_lowercase();
            let has = |s: &str| s.to_ascii_lowercase().contains(&w);
            let digits: String = w.chars().filter(|c| c.is_ascii_digit()).collect();
            has(&self.display()) || has(&self.nickname) || has(&self.org) || has(&self.note) || self.n.iter().any(|p| has(p))
                || self.emails.iter().any(|e| has(&e.value))
                || (!digits.is_empty() && digits.len() == w.trim_start_matches('+').len() && self.phones.iter().any(|p| p.value.chars().filter(|c| c.is_ascii_digit()).collect::<String>().contains(&digits)))
                || self.phones.iter().any(|p| has(&p.value))
        })
    }
    /// Marks a change made here, for the next sync.
    pub fn touch(&mut self) {
        if self.state == SyncState::Clean && !self.href.is_empty() { self.state = SyncState::Modified; }
    }
}

/// Joins folded lines (CRLF or LF, then a space or tab).
pub fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match (line.as_bytes().first(), lines.last_mut()) {
            (Some(b' ' | b'\t'), Some(last)) => last.push_str(&line[1..]),
            _ if line.is_empty() => {}
            _ => lines.push(line.into()),
        }
    }
    lines
}

/// A TEXT value with escapes resolved.
pub fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() { Some('n' | 'N') => out.push('\n'), Some(c) => out.push(c), None => {} }
    }
    out
}

/// Splits a structured value at unescaped `sep`, each part unescaped.
fn parts(value: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' { cur.push(c); if let Some(n) = chars.next() { cur.push(n); } continue; }
        if c == sep { out.push(unescape(&cur)); cur.clear(); continue; }
        cur.push(c);
    }
    out.push(unescape(&cur));
    out
}

fn qp_decode(value: &str) -> String {
    let b = value.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if let Some(v) = b.get(i + 1..i + 3).and_then(|h| core::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One content line: name (upper case, group dropped), parameters, value.
fn split_line(line: &str) -> Option<(String, Vec<(String, String)>, &str)> {
    let mut quoted = false;
    let colon = line.char_indices().find(|&(_, c)| { if c == '"' { quoted = !quoted; } c == ':' && !quoted })?.0;
    let mut items = line[..colon].split(';');
    let name = items.next()?.trim();
    let name = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase();
    let params = items.map(|p| match p.split_once('=') {
        Some((k, v)) => (k.trim().to_ascii_uppercase(), String::from(v.trim_matches('"'))),
        // vCard 2.1: a bare parameter is a TYPE.
        None => (String::from("TYPE"), String::from(p.trim())),
    }).collect();
    Some((name, params, &line[colon + 1..]))
}

fn kinds(params: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (k, v) in params {
        if k != "TYPE" { if k == "PREF" { out.push("pref".into()); } continue; }
        for t in v.split(',') {
            let t = t.trim().to_ascii_lowercase();
            if !t.is_empty() && !out.contains(&t) { out.push(t); }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { NotVcard }

/// The cards of a `.vcf` file in order. A card left open at the end is skipped.
pub fn parse(text: &str) -> Result<Vec<Contact>, Error> {
    let lines = unfold(text);
    if !lines.iter().any(|l| l.trim().eq_ignore_ascii_case("BEGIN:VCARD")) { return Err(Error::NotVcard); }
    let mut out = Vec::new();
    let mut cur: Option<Contact> = None;
    for line in &lines {
        let Some((name, params, raw)) = split_line(line) else { continue };
        let qp = params.iter().any(|(k, v)| k == "ENCODING" && v.eq_ignore_ascii_case("QUOTED-PRINTABLE"));
        let decoded;
        let value = if qp { decoded = qp_decode(raw); decoded.as_str() } else { raw };
        match (name.as_str(), cur.as_mut()) {
            ("BEGIN", None) if value.trim().eq_ignore_ascii_case("VCARD") => cur = Some(Contact { version: 3, ..Contact::default() }),
            ("END", Some(_)) if value.trim().eq_ignore_ascii_case("VCARD") => out.push(cur.take().unwrap()),
            (_, None) => {}
            ("VERSION", Some(c)) => c.version = if value.trim().starts_with('4') { 4 } else { 3 },
            ("UID", Some(c)) => c.uid = String::from(value.trim().trim_start_matches("urn:uuid:")),
            ("FN", Some(c)) => c.name = unescape(value),
            ("N", Some(c)) => { for (i, p) in parts(value, ';').into_iter().take(5).enumerate() { c.n[i] = p; } }
            ("NICKNAME", Some(c)) => c.nickname = unescape(value),
            ("ORG", Some(c)) => c.org = parts(value, ';').into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", "),
            ("TITLE", Some(c)) => c.title = unescape(value),
            ("EMAIL", Some(c)) => c.emails.push(Typed { kinds: kinds(&params), value: String::from(unescape(value).trim()) }),
            ("TEL", Some(c)) => c.phones.push(Typed { kinds: kinds(&params), value: String::from(unescape(value).trim().trim_start_matches("tel:")) }),
            ("ADR", Some(c)) => {
                let p = parts(value, ';');
                let g = |i: usize| p.get(i).cloned().unwrap_or_default();
                c.addresses.push(Address { kinds: kinds(&params), po_box: g(0), extended: g(1), street: g(2), city: g(3), region: g(4), code: g(5), country: g(6) });
            }
            ("URL", Some(c)) => c.urls.push(unescape(value)),
            ("BDAY", Some(c)) => c.birthday = String::from(value.trim()),
            ("NOTE", Some(c)) => c.note = unescape(value),
            ("CATEGORIES", Some(c)) => c.categories = parts(value, ',').into_iter().filter(|p| !p.is_empty()).collect(),
            ("PRODID" | "REV", Some(_)) => {}
            (_, Some(c)) => c.other.push(line.clone()),
        }
    }
    Ok(out)
}

fn line(out: &mut String, text: &str) {
    let mut width = 0;
    for ch in text.chars() {
        if width + ch.len_utf8() > 75 { out.push_str("\r\n "); width = 1; }
        out.push(ch);
        width += ch.len_utf8();
    }
    out.push_str("\r\n");
}

fn esc(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c { '\\' => out.push_str("\\\\"), ';' => out.push_str("\\;"), ',' => out.push_str("\\,"), '\n' => out.push_str("\\n"), '\r' => {}, c => out.push(c) }
    }
    out
}

fn type_param(kinds: &[String], version: u8) -> String {
    let kinds: Vec<&str> = kinds.iter().map(|k| k.as_str()).filter(|k| !k.is_empty()).collect();
    if kinds.is_empty() { return String::new(); }
    if version >= 4 { alloc::format!(";TYPE={}", kinds.join(",")) } else { alloc::format!(";TYPE={}", kinds.iter().map(|k| k.to_ascii_uppercase()).collect::<Vec<_>>().join(",")) }
}

/// One card, in the version it was read (or 4 for a new one).
pub fn write(out: &mut String, c: &Contact) {
    let v = if c.version == 3 { 3 } else { 4 };
    line(out, "BEGIN:VCARD");
    line(out, if v == 3 { "VERSION:3.0" } else { "VERSION:4.0" });
    line(out, "PRODID:-//SexOS//Crush//EN");
    if !c.uid.is_empty() { line(out, &alloc::format!("UID:{}", c.uid)); }
    line(out, &alloc::format!("FN:{}", esc(&c.display())));
    // N is required in 3.0; written whenever any part is known.
    if v == 3 || c.n.iter().any(|p| !p.is_empty()) {
        line(out, &alloc::format!("N:{}", c.n.iter().map(|p| esc(p)).collect::<Vec<_>>().join(";")));
    }
    if !c.nickname.is_empty() { line(out, &alloc::format!("NICKNAME:{}", esc(&c.nickname))); }
    if !c.org.is_empty() { line(out, &alloc::format!("ORG:{}", esc(&c.org))); }
    if !c.title.is_empty() { line(out, &alloc::format!("TITLE:{}", esc(&c.title))); }
    for e in &c.emails { line(out, &alloc::format!("EMAIL{}:{}", type_param(&e.kinds, v), e.value)); }
    for p in &c.phones { line(out, &alloc::format!("TEL{}:{}", type_param(&p.kinds, v), p.value)); }
    for a in &c.addresses {
        let f = [&a.po_box, &a.extended, &a.street, &a.city, &a.region, &a.code, &a.country].iter().map(|s| esc(s)).collect::<Vec<_>>().join(";");
        line(out, &alloc::format!("ADR{}:{f}", type_param(&a.kinds, v)));
    }
    for u in &c.urls { line(out, &alloc::format!("URL:{u}")); }
    if !c.birthday.is_empty() { line(out, &alloc::format!("BDAY:{}", c.birthday)); }
    if !c.note.is_empty() { line(out, &alloc::format!("NOTE:{}", esc(&c.note))); }
    if !c.categories.is_empty() { line(out, &alloc::format!("CATEGORIES:{}", c.categories.iter().map(|s| esc(s)).collect::<Vec<_>>().join(","))); }
    for o in &c.other { line(out, o); }
    line(out, "END:VCARD");
}

pub fn export<'a>(contacts: impl IntoIterator<Item = &'a Contact>) -> String {
    let mut out = String::new();
    for c in contacts { write(&mut out, c); }
    out
}

/// The address book: the cards, kept in the program's folder as one vCard file with
/// each card's CardDAV state in X-SEXOS- properties (never sent to a server), and the
/// server collection's address and change markers in its first lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book { pub contacts: Vec<Contact>, pub url: String, pub ctag: String, pub sync_token: String }

const META: [&str; 3] = ["X-SEXOS-HREF", "X-SEXOS-ETAG", "X-SEXOS-STATE"];

impl Book {
    pub fn parse(text: &str) -> Book {
        let mut book = Book::default();
        for l in text.lines().take_while(|l| l.starts_with('#')) {
            if let Some((k, v)) = l[1..].trim().split_once('=') {
                match k.trim() { "url" => book.url = v.trim().into(), "ctag" => book.ctag = v.trim().into(), "sync-token" => book.sync_token = v.trim().into(), _ => {} }
            }
        }
        book.contacts = parse(text).unwrap_or_default();
        for c in &mut book.contacts {
            let mut kept = Vec::new();
            for o in core::mem::take(&mut c.other) {
                match o.split_once(':') {
                    Some((k, v)) if k == META[0] => c.href = v.into(),
                    Some((k, v)) if k == META[1] => c.etag = v.into(),
                    Some((k, v)) if k == META[2] => c.state = match v { "new" => SyncState::New, "mod" => SyncState::Modified, "del" => SyncState::Deleted, _ => SyncState::Clean },
                    _ => kept.push(o),
                }
            }
            c.other = kept;
        }
        book
    }
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        let _ = write!(out, "# sex-vcard book 1\n# url={}\n# ctag={}\n# sync-token={}\n", self.url, self.ctag, self.sync_token);
        for c in &self.contacts {
            let mut c2 = c.clone();
            if !c.href.is_empty() { c2.other.push(alloc::format!("{}:{}", META[0], c.href)); }
            if !c.etag.is_empty() { c2.other.push(alloc::format!("{}:{}", META[1], c.etag)); }
            match c.state { SyncState::Clean => {}, SyncState::New => c2.other.push(alloc::format!("{}:new", META[2])), SyncState::Modified => c2.other.push(alloc::format!("{}:mod", META[2])), SyncState::Deleted => c2.other.push(alloc::format!("{}:del", META[2])) }
            write(&mut out, &c2);
        }
        out
    }
    /// Indices of the cards shown for `query` (all for an empty one), sorted by name;
    /// cards waiting to be deleted on the server are left out.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let mut found: Vec<usize> = (0..self.contacts.len()).filter(|&i| self.contacts[i].state != SyncState::Deleted && self.contacts[i].matches(query)).collect();
        found.sort_by_cached_key(|&i| self.contacts[i].sort_key());
        found
    }
    /// Adds cards not already here (by UID, or without one by name and first email).
    /// Returns how many were new.
    pub fn merge(&mut self, cards: Vec<Contact>) -> usize {
        let mut added = 0;
        for c in cards {
            let same = |o: &Contact| if !c.uid.is_empty() && !o.uid.is_empty() { o.uid == c.uid } else { o.display() == c.display() && o.emails.first() == c.emails.first() };
            if !self.contacts.iter().any(same) { self.contacts.push(c); added += 1; }
        }
        added
    }
}
