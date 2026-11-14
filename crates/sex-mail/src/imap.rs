// SPDX-License-Identifier: GPL-3.0-only
//! IMAP4rev1 client (RFC 3501) with IDLE (RFC 2177), SASL-IR (RFC 4959) and UIDPLUS-free
//! UID operations: enough for Valentine's folders, threads, flags and push.
use crate::{Auth, Conn, Error, Lines, Result};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token { Atom(String), Str(Vec<u8>), List(Vec<Token>), Nil }

impl Token {
    pub fn text(&self) -> Option<String> {
        match self { Token::Atom(a) => Some(a.clone()), Token::Str(s) => Some(String::from_utf8_lossy(s).into_owned()), _ => None }
    }
    pub fn num(&self) -> Option<u32> { match self { Token::Atom(a) => a.parse().ok(), _ => None } }
}

/// One server response line, literals included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response { pub tag: String, pub tokens: Vec<Token> }

impl Response {
    fn word(&self, i: usize) -> Option<String> { self.tokens.get(i).and_then(Token::text).map(|s| s.to_ascii_uppercase()) }
    /// Text after the status word, for error messages.
    fn rest(&self) -> String {
        self.tokens.iter().skip(1).filter_map(Token::text).collect::<Vec<_>>().join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox { pub name: String, pub flags: Vec<String>, pub delimiter: Option<String> }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selected { pub exists: u32, pub uid_validity: u32, pub uid_next: u32 }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched { pub uid: u32, pub flags: Vec<String>, pub size: u32, pub body: Vec<u8> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event { Exists(u32), Expunge(u32), Flags(u32) }

pub struct Client<C: Conn> { io: Lines<C>, tag: u32, pub capabilities: Vec<String>, pub authenticated: bool }

/// Quote a string as an IMAP quoted string.
pub fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() { if c == '"' || c == '\\' { o.push('\\'); } o.push(c); }
    o.push('"');
    o
}

impl<C: Conn> Client<C> {
    /// Read the greeting.
    pub fn connect(conn: C) -> Result<Self> {
        let mut c = Self { io: Lines::new(conn), tag: 0, capabilities: Vec::new(), authenticated: false };
        let g = c.read_response()?;
        match g.word(0).as_deref() {
            Some("OK") => {}
            Some("PREAUTH") => c.authenticated = true,
            _ => return Err(Error::Refused(g.rest())),
        }
        Ok(c)
    }

    pub fn into_conn(self) -> C { self.io.conn }

    /// Parse one response (line plus literals) into tokens.
    pub fn read_response(&mut self) -> Result<Response> {
        let mut data = Vec::new();
        let mut literals: Vec<Vec<u8>> = Vec::new();
        loop {
            let line = self.io.line()?;
            let lit = literal_len(&line);
            data.extend_from_slice(&line);
            match lit {
                Some(n) => {
                    literals.push(self.io.bytes(n)?);
                    data.extend_from_slice(b"\x00"); // marks where the literal goes
                }
                None => break,
            }
        }
        let mut lits = literals.into_iter();
        let (tag, rest) = match data.iter().position(|&b| b == b' ') { Some(i) => (&data[..i], &data[i + 1..]), None => (&data[..], &[][..]) };
        let mut p = Parser { s: rest, i: 0 };
        let tokens = p.list_until(None, &mut lits);
        Ok(Response { tag: String::from_utf8_lossy(tag).into_owned(), tokens })
    }

    fn next_tag(&mut self) -> String { self.tag += 1; format!("v{}", self.tag) }

    /// Send a command; collect untagged responses until its tagged reply.
    pub fn command(&mut self, cmd: &str) -> Result<Vec<Response>> {
        let tag = self.next_tag();
        self.io.send(format!("{tag} {cmd}\r\n").as_bytes())?;
        self.finish(&tag)
    }

    fn finish(&mut self, tag: &str) -> Result<Vec<Response>> {
        let mut untagged = Vec::new();
        loop {
            let r = self.read_response()?;
            if r.tag == tag {
                return match r.word(0).as_deref() { Some("OK") => Ok(untagged), _ => Err(Error::Refused(r.rest())) };
            }
            if r.tag == "*" {
                if r.word(0).as_deref() == Some("CAPABILITY") { self.capabilities = caps(&r); }
                untagged.push(r);
            }
        }
    }

    pub fn capability(&mut self) -> Result<&[String]> {
        self.command("CAPABILITY")?;
        Ok(&self.capabilities)
    }

    /// Run by Multipass with the secret from Shibari, before Valentine gets the flow.
    pub fn authenticate(&mut self, auth: &Auth) -> Result<()> {
        let (mech, ir) = auth.sasl();
        self.command(&format!("AUTHENTICATE {mech} {ir}"))?;
        self.authenticated = true;
        Ok(())
    }

    pub fn list(&mut self) -> Result<Vec<Mailbox>> {
        let rs = self.command("LIST \"\" \"*\"")?;
        Ok(rs.iter().filter(|r| r.word(0).as_deref() == Some("LIST")).filter_map(|r| {
            let flags = match r.tokens.get(1)? { Token::List(l) => l.iter().filter_map(Token::text).collect(), _ => Vec::new() };
            let delimiter = r.tokens.get(2).and_then(Token::text);
            Some(Mailbox { name: r.tokens.get(3)?.text()?, flags, delimiter })
        }).collect())
    }

    pub fn select(&mut self, mailbox: &str) -> Result<Selected> {
        let rs = self.command(&format!("SELECT {}", quote(mailbox)))?;
        let mut s = Selected::default();
        for r in &rs {
            if r.word(1).as_deref() == Some("EXISTS") { s.exists = r.tokens[0].num().unwrap_or(0); }
            if r.word(0).as_deref() == Some("OK") {
                if let Some(Token::Atom(code)) = r.tokens.get(1) {
                    let n = r.tokens.get(2).and_then(|t| t.text()).and_then(|t| t.trim_end_matches(']').parse().ok()).unwrap_or(0);
                    match code.to_ascii_uppercase().as_str() { "[UIDVALIDITY" => s.uid_validity = n, "[UIDNEXT" => s.uid_next = n, _ => {} }
                }
            }
        }
        Ok(s)
    }

    pub fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        let rs = self.command(&format!("UID SEARCH {criteria}"))?;
        Ok(rs.iter().filter(|r| r.word(0).as_deref() == Some("SEARCH")).flat_map(|r| r.tokens[1..].iter().filter_map(Token::num)).collect())
    }

    /// Fetch whole messages without setting \Seen.
    pub fn uid_fetch(&mut self, set: &str) -> Result<Vec<Fetched>> {
        let rs = self.command(&format!("UID FETCH {set} (UID FLAGS RFC822.SIZE BODY.PEEK[])"))?;
        Ok(rs.iter().filter(|r| r.word(1).as_deref() == Some("FETCH")).filter_map(|r| {
            let Token::List(items) = r.tokens.get(2)? else { return None };
            let mut f = Fetched::default();
            let mut i = 0;
            while i + 1 < items.len() {
                let key = items[i].text().unwrap_or_default().to_ascii_uppercase();
                match key.as_str() {
                    "UID" => f.uid = items[i + 1].num().unwrap_or(0),
                    "RFC822.SIZE" => f.size = items[i + 1].num().unwrap_or(0),
                    "FLAGS" => if let Token::List(l) = &items[i + 1] { f.flags = l.iter().filter_map(Token::text).collect(); },
                    "BODY[]" => if let Token::Str(b) = &items[i + 1] { f.body = b.clone(); },
                    _ => {}
                }
                i += 2;
            }
            Some(f)
        }).collect())
    }

    /// `op` is `+FLAGS`, `-FLAGS` or `FLAGS`.
    pub fn uid_store(&mut self, set: &str, op: &str, flags: &[&str]) -> Result<()> {
        self.command(&format!("UID STORE {set} {op}.SILENT ({})", flags.join(" "))).map(|_| ())
    }

    /// Store a message (a sent copy, a draft).
    pub fn append(&mut self, mailbox: &str, flags: &[&str], message: &[u8]) -> Result<()> {
        let tag = self.next_tag();
        self.io.send(format!("{tag} APPEND {} ({}) {{{}}}\r\n", quote(mailbox), flags.join(" "), message.len()).as_bytes())?;
        let cont = self.read_response()?;
        if cont.tag != "+" { return Err(Error::Refused(cont.rest())); }
        self.io.send(message)?;
        self.io.send(b"\r\n")?;
        self.finish(&tag).map(|_| ())
    }

    pub fn idle_start(&mut self) -> Result<String> {
        let tag = self.next_tag();
        self.io.send(format!("{tag} IDLE\r\n").as_bytes())?;
        let r = self.read_response()?;
        if r.tag != "+" { return Err(Error::Refused(r.rest())); }
        Ok(tag)
    }

    /// Block until the server reports a change while idling.
    pub fn idle_wait(&mut self) -> Result<Event> {
        loop {
            let r = self.read_response()?;
            if r.tag != "*" { continue; }
            let n = r.tokens.first().and_then(Token::num).unwrap_or(0);
            match r.word(1).as_deref() {
                Some("EXISTS") => return Ok(Event::Exists(n)),
                Some("EXPUNGE") => return Ok(Event::Expunge(n)),
                Some("FETCH") => return Ok(Event::Flags(n)),
                _ => {}
            }
        }
    }

    pub fn idle_done(&mut self, tag: &str) -> Result<()> {
        self.io.send(b"DONE\r\n")?;
        self.finish(tag).map(|_| ())
    }

    pub fn logout(&mut self) -> Result<()> { self.command("LOGOUT").map(|_| ()) }
}

fn caps(r: &Response) -> Vec<String> { r.tokens[1..].iter().filter_map(Token::text).map(|s| s.to_ascii_uppercase()).collect() }

/// `... {123}` at the end of a line announces a literal.
pub fn literal_len(line: &[u8]) -> Option<usize> {
    if !line.ends_with(b"}") { return None; }
    let open = line.iter().rposition(|&b| b == b'{')?;
    core::str::from_utf8(&line[open + 1..line.len() - 1]).ok()?.trim_end_matches('+').parse().ok()
}

struct Parser<'a> { s: &'a [u8], i: usize }

impl Parser<'_> {
    fn list_until(&mut self, close: Option<u8>, lits: &mut impl Iterator<Item = Vec<u8>>) -> Vec<Token> {
        let mut out = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if Some(c) == close { self.i += 1; return out; }
            match c {
                b' ' => self.i += 1,
                b'(' => { self.i += 1; out.push(Token::List(self.list_until(Some(b')'), lits))); }
                b'"' => {
                    self.i += 1;
                    let mut v = Vec::new();
                    while self.i < self.s.len() && self.s[self.i] != b'"' {
                        if self.s[self.i] == b'\\' && self.i + 1 < self.s.len() { self.i += 1; }
                        v.push(self.s[self.i]); self.i += 1;
                    }
                    self.i += 1;
                    out.push(Token::Str(v));
                }
                b'{' => {
                    while self.i < self.s.len() && self.s[self.i] != 0 { self.i += 1; }
                    self.i += 1;
                    out.push(Token::Str(lits.next().unwrap_or_default()));
                }
                _ => {
                    let start = self.i;
                    let mut depth = 0;
                    while self.i < self.s.len() {
                        let b = self.s[self.i];
                        // `BODY[HEADER.FIELDS (A B)]` is one atom; a response code `[UIDNEXT 4]` is not.
                        if b == b'[' && self.i > start { depth += 1; } else if b == b']' && depth > 0 { depth -= 1; }
                        if depth == 0 && (b == b' ' || Some(b) == close || b == b')') { break; }
                        self.i += 1;
                    }
                    let a = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                    out.push(if a.eq_ignore_ascii_case("NIL") { Token::Nil } else { Token::Atom(a) });
                }
            }
        }
        out
    }
}
