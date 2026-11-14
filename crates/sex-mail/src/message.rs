// SPDX-License-Identifier: GPL-3.0-only
//! RFC 5322 messages and MIME (RFC 2045-2047): headers with folding and encoded words,
//! multipart bodies, quoted-printable and base64, text and HTML parts, attachments.
use crate::b64;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(pub Vec<(String, String)>);

impl Headers {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    /// A header with RFC 2047 encoded words decoded.
    pub fn text(&self, name: &str) -> Option<String> { self.get(name).map(decode_words) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment { pub filename: String, pub content_type: String, pub data: Vec<u8> }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub headers: Headers,
    pub text: Option<String>,
    /// HTML as sent. Valentine renders it with sex-html with remote content blocked.
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

impl Message {
    pub fn subject(&self) -> String { self.headers.text("subject").unwrap_or_default() }
    pub fn from(&self) -> String { self.headers.text("from").unwrap_or_default() }
    pub fn message_id(&self) -> Option<String> { self.headers.get("message-id").and_then(|v| ids(v).into_iter().next()) }
    pub fn in_reply_to(&self) -> Option<String> { self.headers.get("in-reply-to").and_then(|v| ids(v).into_iter().next()) }
    pub fn references(&self) -> Vec<String> { self.headers.get("references").map(ids).unwrap_or_default() }
    /// Whether the HTML part asks for anything remote (images, styles, frames).
    pub fn has_remote_content(&self) -> bool {
        self.html.as_deref().map(|h| {
            let l = h.to_ascii_lowercase();
            ["src=\"http", "src='http", "src=http", "url(http", "href=\"http"].iter().any(|p| l.contains(p) && !p.starts_with("href"))
                || l.contains("<link") && l.contains("http")
        }).unwrap_or(false)
    }
}

/// `<a@b> <c@d>` → ["a@b", "c@d"].
pub fn ids(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = v;
    while let Some(s) = rest.find('<') {
        let Some(e) = rest[s..].find('>') else { break };
        out.push(rest[s + 1..s + e].trim().to_string());
        rest = &rest[s + e + 1..];
    }
    out
}

/// Split a raw message into unfolded headers and body.
pub fn split(raw: &[u8]) -> (Headers, &[u8]) {
    let (head, body) = match find(raw, b"\r\n\r\n") {
        Some(i) => (&raw[..i], &raw[i + 4..]),
        None => match find(raw, b"\n\n") { Some(i) => (&raw[..i], &raw[i + 2..]), None => (raw, &raw[raw.len()..]) },
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in String::from_utf8_lossy(head).split('\n') {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some(last) = headers.last_mut() { last.1.push(' '); last.1.push_str(line.trim()); }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    (Headers(headers), body)
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> { h.windows(n.len()).position(|w| w == n) }

pub fn parse(raw: &[u8]) -> Message {
    let (headers, body) = split(raw);
    let mut m = Message { headers: headers.clone(), ..Default::default() };
    walk(&headers, body, &mut m, 0);
    m
}

/// `type/subtype; a=b; c="d"` → ("type/subtype", params).
pub fn content_type(v: Option<&str>) -> (String, Vec<(String, String)>) {
    let v = v.unwrap_or("text/plain; charset=us-ascii");
    let mut parts = v.split(';');
    let t = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let params = parts.filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').to_string())).collect();
    (if t.is_empty() { "text/plain".into() } else { t }, params)
}

fn param<'a>(p: &'a [(String, String)], k: &str) -> Option<&'a str> { p.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()) }

fn walk(h: &Headers, body: &[u8], m: &mut Message, depth: usize) {
    let (ct, params) = content_type(h.get("content-type"));
    if ct.starts_with("multipart/") && depth < 8 {
        if let Some(b) = param(&params, "boundary") {
            let delim = format!("--{b}");
            let mut parts = Vec::new();
            let mut start: Option<usize> = None;
            let mut i = 0;
            while let Some(off) = find(&body[i..], delim.as_bytes()) {
                let at = i + off;
                let line_start = at == 0 || body[at - 1] == b'\n';
                if line_start {
                    if let Some(s) = start {
                        let mut end = at;
                        if end >= 2 && &body[end - 2..end] == b"\r\n" { end -= 2; } else if end >= 1 && body[end - 1] == b'\n' { end -= 1; }
                        parts.push(&body[s..end]);
                    }
                    let after = at + delim.len();
                    if body[after..].starts_with(b"--") { break; }
                    let nl = find(&body[after..], b"\n").map(|n| after + n + 1).unwrap_or(body.len());
                    start = Some(nl);
                    i = nl;
                } else { i = at + delim.len(); }
            }
            for p in parts {
                let (ph, pb) = split(p);
                walk(&ph, pb, m, depth + 1);
            }
            return;
        }
    }
    let data = transfer_decode(h.get("content-transfer-encoding"), body);
    let disposition = h.get("content-disposition").unwrap_or("");
    let (_, dparams) = content_type(Some(disposition));
    let filename = param(&dparams, "filename").or(param(&params, "name")).map(decode_words);
    let attachment = disposition.to_ascii_lowercase().starts_with("attachment") || (filename.is_some() && !ct.starts_with("text/"));
    if !attachment && ct == "text/plain" && m.text.is_none() {
        m.text = Some(charset_decode(param(&params, "charset"), &data));
    } else if !attachment && ct == "text/html" && m.html.is_none() {
        m.html = Some(charset_decode(param(&params, "charset"), &data));
    } else if ct == "message/rfc822" && !attachment {
        m.attachments.push(Attachment { filename: "message.eml".into(), content_type: ct, data });
    } else {
        m.attachments.push(Attachment { filename: filename.unwrap_or_else(|| "attachment".into()), content_type: ct, data });
    }
}

pub fn transfer_decode(enc: Option<&str>, body: &[u8]) -> Vec<u8> {
    match enc.map(|e| e.trim().to_ascii_lowercase()).as_deref() {
        Some("base64") => b64::decode(body).unwrap_or_default(),
        Some("quoted-printable") => qp_decode(body, false),
        _ => body.to_vec(),
    }
}

pub fn qp_decode(s: &[u8], underscore_space: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'=' if s.get(i + 1) == Some(&b'\r') && s.get(i + 2) == Some(&b'\n') => i += 2,
            b'=' if s.get(i + 1) == Some(&b'\n') => i += 1,
            b'=' if i + 2 < s.len() => {
                let hex = core::str::from_utf8(&s[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex { Some(v) => { out.push(v); i += 2; } None => out.push(b'=') }
            }
            b'_' if underscore_space => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    out
}

pub fn charset_decode(charset: Option<&str>, data: &[u8]) -> String {
    match charset.map(|c| c.to_ascii_lowercase()).as_deref() {
        Some("iso-8859-1") | Some("latin1") | Some("windows-1252") => data.iter().map(|&b| b as char).collect(),
        _ => String::from_utf8_lossy(data).into_owned(),
    }
}

/// RFC 2047: `=?UTF-8?B?...?=` and `=?ISO-8859-1?Q?...?=`; whitespace between words is dropped.
pub fn decode_words(v: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    let mut last_was_word = false;
    while let Some(s) = rest.find("=?") {
        let (before, tail) = rest.split_at(s);
        let decoded = (|| {
            let inner = &tail[2..];
            let (charset, inner) = inner.split_once('?')?;
            let (enc, inner) = inner.split_once('?')?;
            let end = inner.find("?=")?;
            let text = &inner[..end];
            let bytes = match enc { "B" | "b" => b64::decode(text.as_bytes())?, "Q" | "q" => qp_decode(text.as_bytes(), true), _ => return None };
            Some((charset_decode(Some(charset.split('*').next().unwrap_or(charset)), &bytes), 2 + charset.len() + 1 + enc.len() + 1 + end + 2))
        })();
        match decoded {
            Some((text, used)) => {
                if !(last_was_word && before.trim().is_empty()) { out.push_str(before); }
                out.push_str(&text);
                rest = &tail[used..];
                last_was_word = true;
            }
            None => { out.push_str(before); out.push_str("=?"); rest = &tail[2..]; last_was_word = false; }
        }
    }
    out.push_str(rest);
    out
}

/// Encode a header value as one UTF-8 base64 word when it is not plain ASCII.
pub fn encode_word(v: &str) -> String {
    if v.is_ascii() { v.into() } else { format!("=?UTF-8?B?{}?=", b64::encode(v.as_bytes())) }
}

/// A plain-text message to send.
pub struct Draft<'a> {
    pub from: &'a str,
    pub to: &'a [&'a str],
    pub subject: &'a str,
    pub body: &'a str,
    pub message_id: &'a str,
    pub date: &'a str,
    pub in_reply_to: Option<&'a str>,
    pub references: &'a [&'a str],
}

pub fn compose(d: &Draft) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&format!("From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}>\r\n", d.from, d.to.join(", "), encode_word(d.subject), d.date, d.message_id));
    if let Some(r) = d.in_reply_to { s.push_str(&format!("In-Reply-To: <{r}>\r\n")); }
    if !d.references.is_empty() {
        let refs: Vec<String> = d.references.iter().map(|r| format!("<{r}>")).collect();
        s.push_str(&format!("References: {}\r\n", refs.join(" ")));
    }
    s.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\nUser-Agent: Valentine (SexOS)\r\n\r\n");
    for line in d.body.split('\n') { s.push_str(line.trim_end_matches('\r')); s.push_str("\r\n"); }
    s.into_bytes()
}

/// The address part of `Name <a@b>` or `a@b`.
pub fn address(v: &str) -> &str {
    match (v.rfind('<'), v.rfind('>')) { (Some(s), Some(e)) if s < e => v[s + 1..e].trim(), _ => v.trim() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTI: &str = "From: =?UTF-8?Q?Ren=C3=A9e?= <renee@example.org>\r\nTo: me@example.org\r\nSubject: =?UTF-8?B?Q2Fmw6k=?=\r\n =?UTF-8?Q?_ce_soir?=\r\nMessage-ID: <m2@example.org>\r\nIn-Reply-To: <m1@example.org>\r\nReferences: <m0@example.org>\r\n <m1@example.org>\r\nContent-Type: multipart/mixed; boundary=\"outer\"\r\n\r\npreamble\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nSee you at the caf=C3=A9 =\r\ntonight.\r\n--inner\r\nContent-Type: text/html\r\n\r\n<p>See you <img src=\"https://tracker.example/p.gif\"></p>\r\n--inner--\r\n--outer\r\nContent-Type: application/pdf; name=\"menu.pdf\"\r\nContent-Disposition: attachment; filename=\"menu.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0x\r\nLjQ=\r\n--outer--\r\nepilogue\r\n";

    #[test]
    fn multipart_with_encodings() {
        let m = parse(MULTI.as_bytes());
        assert_eq!(m.subject(), "Café ce soir");
        assert_eq!(m.from(), "Renée <renee@example.org>");
        assert_eq!(m.text.as_deref(), Some("See you at the café tonight."));
        assert!(m.html.as_deref().unwrap().contains("<p>"));
        assert!(m.has_remote_content());
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].filename, "menu.pdf");
        assert_eq!(m.attachments[0].data, b"%PDF-1.4");
        assert_eq!(m.message_id().as_deref(), Some("m2@example.org"));
        assert_eq!(m.in_reply_to().as_deref(), Some("m1@example.org"));
        assert_eq!(m.references(), ["m0@example.org", "m1@example.org"]);
    }

    #[test]
    fn compose_round_trips() {
        let raw = compose(&Draft { from: "Me <me@example.org>", to: &["you@example.org"], subject: "Grüße", body: "line one\nline two", message_id: "x1@example.org", date: "Sat, 14 Nov 2026 02:00:00 +0000", in_reply_to: Some("p@example.org"), references: &["p@example.org"] });
        let m = parse(&raw);
        assert_eq!(m.subject(), "Grüße");
        assert_eq!(m.text.as_deref(), Some("line one\r\nline two\r\n"));
        assert_eq!(m.in_reply_to().as_deref(), Some("p@example.org"));
        assert!(!m.has_remote_content());
        assert_eq!(address("Me <me@example.org>"), "me@example.org");
    }
}
