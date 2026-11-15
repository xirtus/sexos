// SPDX-License-Identifier: GPL-3.0-only
//! An in-memory mail server (IMAP + SMTP over one shared store) for host tests and the
//! QEMU gate. It speaks the subset Valentine uses and is strict about it: unknown
//! commands get BAD, wrong passwords get NO, mail is delivered only to known users.
use crate::smtp::dot_unstuff;
use crate::{b64, Conn, Error, Result};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

#[derive(Clone)]
pub struct Stored { pub uid: u32, pub flags: Vec<String>, pub data: Vec<u8> }

#[derive(Default)]
pub struct Store {
    users: BTreeMap<String, String>,
    boxes: BTreeMap<(String, String), (u32, Vec<Stored>)>,
}

pub type Shared = Rc<RefCell<Store>>;

impl Store {
    pub fn shared() -> Shared { Rc::new(RefCell::new(Store::default())) }
    pub fn add_user(&mut self, user: &str, secret: &str) {
        self.users.insert(user.into(), secret.into());
        for b in ["INBOX", "Sent", "Drafts", "Trash"] { self.boxes.insert((user.into(), b.into()), (1, Vec::new())); }
    }
    pub fn deliver(&mut self, user: &str, mailbox: &str, flags: Vec<String>, data: Vec<u8>) -> bool {
        match self.boxes.get_mut(&(user.into(), mailbox.into())) {
            Some((next, msgs)) => { msgs.push(Stored { uid: *next, flags, data }); *next += 1; true }
            None => false,
        }
    }
    pub fn messages(&self, user: &str, mailbox: &str) -> Vec<Stored> {
        self.boxes.get(&(user.into(), mailbox.into())).map(|b| b.1.clone()).unwrap_or_default()
    }
    fn check(&self, user: &str, secret: &str) -> bool { self.users.get(user).map(|s| s == secret).unwrap_or(false) }
    fn sasl(&self, mech: &str, ir: &str) -> Option<String> {
        let raw = b64::decode(ir.as_bytes())?;
        match mech.to_ascii_uppercase().as_str() {
            "PLAIN" => {
                let mut p = raw.split(|&b| b == 0);
                let _authz = p.next()?;
                let (u, s) = (String::from_utf8(p.next()?.to_vec()).ok()?, String::from_utf8(p.next()?.to_vec()).ok()?);
                self.check(&u, &s).then_some(u)
            }
            "XOAUTH2" => {
                let t = String::from_utf8(raw).ok()?;
                let mut user = None; let mut tok = None;
                for kv in t.split('\x01') {
                    if let Some(u) = kv.strip_prefix("user=") { user = Some(u.to_string()); }
                    if let Some(b) = kv.strip_prefix("auth=Bearer ") { tok = Some(b.to_string()); }
                }
                let (u, b) = (user?, tok?);
                self.check(&u, &b).then_some(u)
            }
            _ => None,
        }
    }
}

enum Proto { Imap(Imap), Smtp(Smtp) }

#[derive(Default)]
struct Imap { user: Option<String>, selected: Option<String>, seen_count: usize, idle: Option<String>, append: Option<(String, String, Vec<String>, usize)> }

#[derive(Default)]
struct Smtp { user: Option<String>, from: Option<String>, rcpt: Vec<String>, data: Option<Vec<u8>> }

/// One client connection to the fixture.
pub struct FixtureConn { store: Shared, proto: Proto, input: Vec<u8>, out: VecDeque<u8>, closed: bool }

impl FixtureConn {
    pub fn imap(store: &Shared) -> Self {
        let mut c = Self { store: store.clone(), proto: Proto::Imap(Imap::default()), input: Vec::new(), out: VecDeque::new(), closed: false };
        c.say("* OK [CAPABILITY IMAP4rev1 IDLE SASL-IR AUTH=PLAIN AUTH=XOAUTH2] fixture ready");
        c
    }
    pub fn smtp(store: &Shared) -> Self {
        let mut c = Self { store: store.clone(), proto: Proto::Smtp(Smtp::default()), input: Vec::new(), out: VecDeque::new(), closed: false };
        c.say("220 fixture.example ESMTP ready");
        c
    }
    fn say(&mut self, s: &str) { self.out.extend(s.as_bytes()); self.out.extend(b"\r\n"); }
    fn raw(&mut self, b: &[u8]) { self.out.extend(b); }

    fn pump(&mut self) {
        loop {
            // A pending IMAP APPEND literal.
            if let Proto::Imap(Imap { append: Some((_, _, _, n)), .. }) = &self.proto {
                let n = *n;
                if self.input.len() < n + 2 { return; }
                let data: Vec<u8> = self.input.drain(..n).collect();
                self.input.drain(..2);
                let Proto::Imap(st) = &mut self.proto else { unreachable!() };
                let (tag, mbox, flags, _) = st.append.take().unwrap();
                let user = st.user.clone().unwrap_or_default();
                let ok = self.store.borrow_mut().deliver(&user, &mbox, flags, data);
                self.say(&if ok { format!("{tag} OK APPEND completed") } else { format!("{tag} NO [TRYCREATE] no such mailbox") });
                continue;
            }
            let Some(i) = self.input.windows(2).position(|w| w == b"\r\n") else { return };
            let line: Vec<u8> = self.input.drain(..i + 2).take(i).collect();
            let line = String::from_utf8_lossy(&line).into_owned();
            match self.proto { Proto::Imap(_) => self.imap_line(&line), Proto::Smtp(_) => self.smtp_line(&line, &line) }
        }
    }

    fn imap_line(&mut self, line: &str) {
        let Proto::Imap(st) = &mut self.proto else { return };
        if let Some(tag) = st.idle.clone() {
            if line.eq_ignore_ascii_case("DONE") { st.idle = None; self.say(&format!("{tag} OK IDLE terminated")); }
            return;
        }
        let mut words = split_args(line);
        if words.len() < 2 { self.say("* BAD missing command"); return; }
        let tag = words.remove(0);
        let mut cmd = words.remove(0).to_ascii_uppercase();
        if cmd == "UID" && !words.is_empty() { cmd = format!("UID {}", words.remove(0).to_ascii_uppercase()); }
        let user = st.user.clone();
        let authed = |s: &mut Self| -> Option<String> { if user.is_none() { s.say(&format!("{tag} NO authenticate first")); } user.clone() };
        match cmd.as_str() {
            "CAPABILITY" => { self.say("* CAPABILITY IMAP4rev1 IDLE SASL-IR AUTH=PLAIN AUTH=XOAUTH2"); self.say(&format!("{tag} OK done")); }
            "NOOP" => self.say(&format!("{tag} OK done")),
            "AUTHENTICATE" if words.len() == 2 => {
                let who = self.store.borrow().sasl(&words[0], &words[1]);
                match who {
                    Some(u) => { if let Proto::Imap(st) = &mut self.proto { st.user = Some(u); } self.say(&format!("{tag} OK authenticated")); }
                    None => self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] invalid credentials")),
                }
            }
            "LIST" => {
                let Some(u) = authed(self) else { return };
                let names: Vec<String> = self.store.borrow().boxes.keys().filter(|(o, _)| *o == u).map(|(_, b)| b.clone()).collect();
                for n in names {
                    let flag = match n.as_str() { "Sent" => "\\Sent", "Drafts" => "\\Drafts", "Trash" => "\\Trash", _ => "\\HasNoChildren" };
                    self.say(&format!("* LIST ({flag}) \"/\" \"{n}\""));
                }
                self.say(&format!("{tag} OK LIST completed"));
            }
            "SELECT" if words.len() == 1 => {
                let Some(u) = authed(self) else { return };
                let b = unquote(&words[0]);
                let info = self.store.borrow().boxes.get(&(u.clone(), b.clone())).map(|(next, m)| (*next, m.len()));
                match info {
                    Some((next, n)) => {
                        if let Proto::Imap(st) = &mut self.proto { st.selected = Some(b); st.seen_count = n; }
                        self.say("* FLAGS (\\Seen \\Answered \\Flagged \\Deleted \\Draft)");
                        self.say(&format!("* {n} EXISTS"));
                        self.say("* OK [UIDVALIDITY 1700000000] UIDs valid");
                        self.say(&format!("* OK [UIDNEXT {next}] predicted next UID"));
                        self.say(&format!("{tag} OK [READ-WRITE] SELECT completed"));
                    }
                    None => self.say(&format!("{tag} NO no such mailbox")),
                }
            }
            "UID SEARCH" => {
                let Some(msgs) = self.selected_msgs(&tag) else { return };
                let unseen = words.iter().any(|w| w.eq_ignore_ascii_case("UNSEEN"));
                let uids: Vec<String> = msgs.iter().filter(|m| !unseen || !m.flags.iter().any(|f| f == "\\Seen")).map(|m| m.uid.to_string()).collect();
                self.say(&format!("* SEARCH {}", uids.join(" ")).trim_end().to_string());
                self.say(&format!("{tag} OK SEARCH completed"));
            }
            "UID FETCH" if !words.is_empty() => {
                let Some(msgs) = self.selected_msgs(&tag) else { return };
                let want_body = line.to_ascii_uppercase().contains("BODY.PEEK[]") || line.to_ascii_uppercase().contains("BODY[]");
                for (seq, m) in msgs.iter().enumerate() {
                    if !in_set(&words[0], m.uid) { continue; }
                    let head = format!("* {} FETCH (UID {} FLAGS ({}) RFC822.SIZE {}", seq + 1, m.uid, m.flags.join(" "), m.data.len());
                    if want_body {
                        self.raw(format!("{head} BODY[] {{{}}}\r\n", m.data.len()).as_bytes());
                        self.raw(&m.data);
                        self.say(")");
                    } else { self.say(&format!("{head})")); }
                }
                self.say(&format!("{tag} OK FETCH completed"));
            }
            "UID STORE" if words.len() >= 3 => {
                let Some(u) = authed(self) else { return };
                let Proto::Imap(st) = &self.proto else { return };
                let Some(b) = st.selected.clone() else { self.say(&format!("{tag} NO select first")); return };
                let op = words[1].to_ascii_uppercase();
                let flags: Vec<String> = words[2..].join(" ").trim_matches(['(', ')']).split_whitespace().map(Into::into).collect();
                if let Some((_, msgs)) = self.store.borrow_mut().boxes.get_mut(&(u, b)) {
                    for m in msgs.iter_mut().filter(|m| in_set(&words[0], m.uid)) {
                        if op.starts_with('+') { for f in &flags { if !m.flags.contains(f) { m.flags.push(f.clone()); } } }
                        else if op.starts_with('-') { m.flags.retain(|f| !flags.contains(f)); }
                        else { m.flags = flags.clone(); }
                    }
                }
                self.say(&format!("{tag} OK STORE completed"));
            }
            "APPEND" if words.len() >= 2 => {
                let Some(_) = authed(self) else { return };
                let n = words.last().and_then(|w| w.trim_matches(['{', '}', '+']).parse::<usize>().ok());
                let flags: Vec<String> = if words.len() >= 3 { words[1].trim_matches(['(', ')']).split_whitespace().map(Into::into).collect() } else { Vec::new() };
                match n {
                    Some(n) => { if let Proto::Imap(st) = &mut self.proto { st.append = Some((tag, unquote(&words[0]), flags, n)); } self.say("+ Ready for literal data"); }
                    None => self.say(&format!("{tag} BAD literal expected")),
                }
            }
            "IDLE" => {
                if self.selected_msgs(&tag).is_none() { return; }
                if let Proto::Imap(st) = &mut self.proto { st.idle = Some(tag); }
                self.say("+ idling");
            }
            "LOGOUT" => { self.say("* BYE fixture closing"); self.say(&format!("{tag} OK LOGOUT completed")); self.closed = true; }
            _ => self.say(&format!("{tag} BAD unknown command")),
        }
    }

    fn selected_msgs(&mut self, tag: &str) -> Option<Vec<Stored>> {
        let Proto::Imap(st) = &self.proto else { return None };
        match (&st.user, &st.selected) {
            (Some(u), Some(b)) => Some(self.store.borrow().messages(u, b)),
            _ => { self.say(&format!("{tag} NO select a mailbox first")); None }
        }
    }

    /// While idling, report mail that arrived since.
    fn poll(&mut self) {
        let Proto::Imap(st) = &self.proto else { return };
        if st.idle.is_none() { return; }
        let (Some(u), Some(b)) = (st.user.clone(), st.selected.clone()) else { return };
        let n = self.store.borrow().messages(&u, &b).len();
        if n > st.seen_count {
            if let Proto::Imap(st) = &mut self.proto { st.seen_count = n; }
            self.say(&format!("* {n} EXISTS"));
        }
    }

    fn smtp_line(&mut self, line: &str, raw: &str) {
        let Proto::Smtp(st) = &mut self.proto else { return };
        if let Some(data) = &mut st.data {
            if line == "." {
                let body = dot_unstuff(&core::mem::take(data));
                st.data = None;
                let rcpt = core::mem::take(&mut st.rcpt);
                st.from = None;
                let mut store = self.store.borrow_mut();
                for r in &rcpt { store.deliver(r, "INBOX", Vec::new(), body.clone()); }
                drop(store);
                self.say("250 2.0.0 queued");
            } else { data.extend_from_slice(raw.as_bytes()); data.extend_from_slice(b"\r\n"); }
            return;
        }
        let upper = line.to_ascii_uppercase();
        let authed = st.user.is_some();
        if upper.starts_with("EHLO ") || upper.starts_with("HELO ") {
            self.say("250-fixture.example"); self.say("250-AUTH PLAIN XOAUTH2"); self.say("250-8BITMIME"); self.say("250 SMTPUTF8");
        } else if let Some(rest) = upper.strip_prefix("AUTH ") {
            let mut p = rest.split(' ');
            let mech = p.next().unwrap_or("").to_string();
            let ir = line.splitn(3, ' ').nth(2).unwrap_or("").to_string();
            let who = self.store.borrow().sasl(&mech, &ir);
            match who { Some(u) => { if let Proto::Smtp(st) = &mut self.proto { st.user = Some(u); } self.say("235 2.7.0 authenticated"); } None => self.say("535 5.7.8 bad credentials") }
        } else if upper.starts_with("MAIL FROM:") {
            if !authed { self.say("530 5.7.0 authentication required"); return; }
            if let Proto::Smtp(st) = &mut self.proto { st.from = Some(angle(line)); st.rcpt.clear(); }
            self.say("250 2.1.0 ok");
        } else if upper.starts_with("RCPT TO:") {
            let to = angle(line);
            let known = self.store.borrow().users.contains_key(&to);
            if !known { self.say("550 5.1.1 no such user"); return; }
            if let Proto::Smtp(st) = &mut self.proto { st.rcpt.push(to); }
            self.say("250 2.1.5 ok");
        } else if upper == "DATA" {
            let Proto::Smtp(st) = &mut self.proto else { return };
            if st.from.is_none() || st.rcpt.is_empty() { self.say("503 5.5.1 need MAIL and RCPT"); return; }
            st.data = Some(Vec::new());
            self.say("354 end with <CRLF>.<CRLF>");
        } else if upper == "RSET" { if let Proto::Smtp(st) = &mut self.proto { st.from = None; st.rcpt.clear(); } self.say("250 ok"); }
        else if upper == "QUIT" { self.say("221 bye"); self.closed = true; }
        else { self.say("500 5.5.2 unknown command"); }
    }
}

impl Conn for FixtureConn {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        if self.closed { return Err(Error::Io); }
        self.input.extend_from_slice(data);
        self.pump();
        Ok(())
    }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.out.is_empty() { self.poll(); }
        if self.out.is_empty() { return if self.closed { Ok(0) } else { Err(Error::Io) }; }
        let n = buf.len().min(self.out.len());
        for b in buf.iter_mut().take(n) { *b = self.out.pop_front().unwrap(); }
        Ok(n)
    }
}

fn angle(line: &str) -> String {
    match (line.find('<'), line.rfind('>')) { (Some(s), Some(e)) if s < e => line[s + 1..e].to_string(), _ => String::new() }
}

fn unquote(s: &str) -> String { s.trim_matches('"').replace("\\\"", "\"").replace("\\\\", "\\") }

/// Split IMAP arguments, keeping quoted strings and parenthesised groups whole.
fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut depth) = (false, 0);
    for c in line.chars() {
        match c {
            '"' => { quoted = !quoted; cur.push(c); }
            '(' if !quoted => { depth += 1; cur.push(c); }
            ')' if !quoted => { depth -= 1; cur.push(c); }
            ' ' if !quoted && depth == 0 => { if !cur.is_empty() { out.push(core::mem::take(&mut cur)); } }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

/// `1:*`, `3`, `2,5:7`.
fn in_set(set: &str, uid: u32) -> bool {
    set.split(',').any(|r| match r.split_once(':') {
        Some((a, b)) => {
            let lo = a.parse().unwrap_or(1);
            let hi = if b == "*" { u32::MAX } else { b.parse().unwrap_or(0) };
            (lo.min(hi)..=lo.max(hi)).contains(&uid)
        }
        None => r.parse() == Ok(uid),
    })
}
