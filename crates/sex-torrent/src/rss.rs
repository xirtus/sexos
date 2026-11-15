// SPDX-License-Identifier: GPL-3.0-only
//! RSS 2.0, RSS 1.0 (RDF) and Atom feed reading for torrent feeds, and the state a
//! program keeps per subscribed feed.
//!
//! Real torrent feeds are messy: wrong encodings, HTML pasted into `<description>`
//! without escaping, unknown entities, unclosed tags. So the [`xml`] tokenizer here is
//! tolerant rather than validating: it never fails on bad markup (a stray `<` is text, a
//! mismatched end tag is skipped, unclosed elements close at the end), but it is bounded
//! (input size, element count, nesting depth, attribute count), so hostile input cannot
//! exhaust memory or the stack.
//!
//! [`parse_feed`] turns a feed into [`FeedItem`]s with what a torrent client needs:
//! title, link, guid, date (RFC 822 or RFC 3339, as unix seconds), enclosure, a magnet
//! link (from the link, the enclosure, `torrent:magnetURI`, a Torznab attribute, or
//! found in the description), the info-hash (`torrent:infoHash`, `nyaa:infoHash`, or
//! the magnet's `btih`), size and seeders/leechers when the feed gives them.
//! Namespaced elements are matched by their local name (`nyaa:seeders`, `torrent:seeds`)
//! because feeds disagree on namespace URIs but agree on the names.
//!
//! [`FeedState`] remembers which items were already seen (bounded) and when the feed is
//! next due, honouring the feed's `<ttl>`, backing off after failures, and producing the
//! conditional-GET headers (`If-None-Match`, `If-Modified-Since`).
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{from_hex, InfoHashV1, Millis};

/// A small tolerant XML reader that builds a bounded element tree.
pub mod xml {
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Inputs larger than this are refused.
    pub const MAX_INPUT: usize = 16 << 20;
    /// Elements nested deeper than this are refused.
    pub const MAX_DEPTH: usize = 48;
    /// At most this many elements per document.
    pub const MAX_NODES: usize = 200_000;
    /// Attributes beyond this many on one element are dropped.
    pub const MAX_ATTRS: usize = 64;

    /// Why a document was refused (markup errors are tolerated, limits are not).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Error { TooLarge, TooDeep, TooManyNodes }

    /// One element: its qualified name as written (`nyaa:seeders`), attributes with
    /// entities decoded, child elements, and its direct text (CDATA included).
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Element {
        pub name: String,
        pub attrs: Vec<(String, String)>,
        pub children: Vec<Element>,
        pub text: String,
    }

    /// The part of a qualified name after the colon.
    pub fn local(name: &str) -> &str { name.rsplit(':').next().unwrap_or(name) }

    impl Element {
        /// Local name (no prefix).
        pub fn local(&self) -> &str { local(&self.name) }
        /// Namespace prefix, empty when none.
        pub fn prefix(&self) -> &str { self.name.rsplit_once(':').map(|(p, _)| p).unwrap_or("") }
        /// First child whose local name equals `name`, ignoring ASCII case.
        pub fn child(&self, name: &str) -> Option<&Element> { self.children.iter().find(|c| c.local().eq_ignore_ascii_case(name)) }
        /// Children whose local name equals `name`, ignoring ASCII case.
        pub fn children_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s Element> + 's {
            self.children.iter().filter(move |c| c.local().eq_ignore_ascii_case(name))
        }
        /// An attribute by exact qualified name, else by local name (ASCII case ignored).
        pub fn attr(&self, name: &str) -> Option<&str> {
            self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name))
                .or_else(|| self.attrs.iter().find(|(k, _)| local(k).eq_ignore_ascii_case(name)))
                .map(|(_, v)| v.as_str())
        }
        /// All text below this element (children included), in document order, trimmed.
        pub fn inner_text(&self) -> String {
            let mut out = String::new();
            fn walk(e: &Element, out: &mut String, depth: usize) {
                out.push_str(&e.text);
                if depth < MAX_DEPTH { for c in &e.children { walk(c, out, depth + 1); } }
            }
            walk(self, &mut out, 0);
            String::from(out.trim())
        }
        /// Direct text, trimmed; falls back to [`Element::inner_text`] when there is
        /// markup inside (unescaped HTML in a description).
        pub fn text(&self) -> String {
            if self.children.is_empty() { String::from(self.text.trim()) } else { self.inner_text() }
        }
    }

    /// Decodes the input as UTF-8, or as Latin-1 when it is not valid UTF-8 (old
    /// feeds declare ISO-8859-1 and mean it); a byte-order mark is dropped.
    pub fn decode_text(input: &[u8]) -> String {
        let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
        match core::str::from_utf8(input) {
            Ok(s) => String::from(s),
            Err(_) => input.iter().map(|&b| b as char).collect(),
        }
    }

    /// Appends `s` to `out` with XML/HTML entities decoded. Unknown or malformed
    /// entities stay as written.
    pub fn decode_entities(s: &str, out: &mut String) {
        let mut rest = s;
        while let Some(p) = rest.find('&') {
            out.push_str(&rest[..p]);
            rest = &rest[p..];
            let end = rest.as_bytes().iter().take(12).position(|&c| c == b';');
            let decoded = end.and_then(|e| entity(&rest[1..e]).map(|c| (c, e)));
            match decoded {
                Some((c, e)) => { out.push(c); rest = &rest[e + 1..]; }
                None => { out.push('&'); rest = &rest[1..]; }
            }
        }
        out.push_str(rest);
    }

    fn entity(name: &str) -> Option<char> {
        if let Some(num) = name.strip_prefix('#') {
            let v = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => num.parse::<u32>().ok()?,
            };
            return Some(char::from_u32(v).filter(|&c| c != '\0').unwrap_or('\u{FFFD}'));
        }
        Some(match name {
            "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'',
            "nbsp" => '\u{A0}', "hellip" => '…', "mdash" => '—', "ndash" => '–', "copy" => '©',
            "reg" => '®', "trade" => '™', "laquo" => '«', "raquo" => '»', "lsquo" => '‘',
            "rsquo" => '’', "ldquo" => '“', "rdquo" => '”', "middot" => '·', "times" => '×',
            _ => return None,
        })
    }

    fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
        if from > b.len() { return None; }
        b[from..].windows(pat.len()).position(|w| w == pat).map(|p| p + from)
    }

    fn is_name_start(c: u8) -> bool { c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80 }
    fn is_name(c: u8) -> bool { is_name_start(c) || c.is_ascii_digit() || c == b'-' || c == b'.' }

    /// Parses a document into a synthetic root element (empty name) whose children are
    /// the top-level elements.
    pub fn parse(input: &[u8]) -> Result<Element, Error> {
        if input.len() > MAX_INPUT { return Err(Error::TooLarge); }
        let text = decode_text(input);
        let s = text.as_str();
        let b = s.as_bytes();
        let mut stack: Vec<Element> = alloc::vec![Element::default()];
        let mut nodes = 0usize;
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'<' {
                let end = b[i..].iter().position(|&c| c == b'<').map(|p| p + i).unwrap_or(b.len());
                decode_entities(&s[i..end], &mut stack.last_mut().unwrap().text);
                i = end;
                continue;
            }
            if b[i..].starts_with(b"<!--") {
                i = find(b, i + 4, b"-->").map(|e| e + 3).unwrap_or(b.len());
            } else if b[i..].starts_with(b"<![CDATA[") {
                let end = find(b, i + 9, b"]]>").unwrap_or(b.len());
                stack.last_mut().unwrap().text.push_str(&s[i + 9..end]);
                i = (end + 3).min(b.len());
            } else if b[i..].starts_with(b"<?") {
                i = find(b, i + 2, b"?>").map(|e| e + 2).unwrap_or(b.len());
            } else if b[i..].starts_with(b"<!") {
                // DOCTYPE (with an optional internal subset in brackets) or similar.
                let mut depth = 0i32;
                let mut j = i + 2;
                while j < b.len() {
                    match b[j] { b'[' => depth += 1, b']' => depth -= 1, b'>' if depth <= 0 => break, _ => {} }
                    j += 1;
                }
                i = (j + 1).min(b.len());
            } else if b[i..].starts_with(b"</") {
                let end = b[i..].iter().position(|&c| c == b'>').map(|p| p + i).unwrap_or(b.len());
                let name = s[i + 2..end].trim();
                if let Some(pos) = stack.iter().rposition(|e| e.name.eq_ignore_ascii_case(name)) {
                    if pos > 0 {
                        while stack.len() > pos {
                            let done = stack.pop().unwrap();
                            stack.last_mut().unwrap().children.push(done);
                        }
                    }
                }
                i = (end + 1).min(b.len());
            } else if i + 1 < b.len() && is_name_start(b[i + 1]) {
                let mut j = i + 1;
                while j < b.len() && is_name(b[j]) { j += 1; }
                let mut el = Element { name: String::from(&s[i + 1..j]), ..Element::default() };
                let mut self_closing = false;
                // Attributes until '>'.
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                    if j >= b.len() { break; }
                    match b[j] {
                        b'>' => { j += 1; break; }
                        b'/' => { j += 1; if j < b.len() && b[j] == b'>' { self_closing = true; j += 1; break; } continue; }
                        _ => {}
                    }
                    let ns = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/') { j += 1; }
                    if j == ns { j += 1; continue; }
                    let name = &s[ns..j];
                    while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                    let mut value = String::new();
                    if j < b.len() && b[j] == b'=' {
                        j += 1;
                        while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                        if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                            let q = b[j];
                            let vs = j + 1;
                            let ve = b[vs..].iter().position(|&c| c == q).map(|p| p + vs).unwrap_or(b.len());
                            decode_entities(&s[vs..ve], &mut value);
                            j = (ve + 1).min(b.len());
                        } else {
                            let vs = j;
                            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' { j += 1; }
                            decode_entities(&s[vs..j], &mut value);
                        }
                    }
                    if el.attrs.len() < MAX_ATTRS { el.attrs.push((String::from(name), value)); }
                }
                nodes += 1;
                if nodes > MAX_NODES { return Err(Error::TooManyNodes); }
                if self_closing {
                    stack.last_mut().unwrap().children.push(el);
                } else {
                    if stack.len() > MAX_DEPTH { return Err(Error::TooDeep); }
                    stack.push(el);
                }
                i = j;
            } else {
                // A '<' that does not start markup ("a < b"): keep it as text.
                stack.last_mut().unwrap().text.push('<');
                i += 1;
            }
        }
        while stack.len() > 1 {
            let done = stack.pop().unwrap();
            stack.last_mut().unwrap().children.push(done);
        }
        Ok(stack.pop().unwrap())
    }
}

use xml::Element;

/// A feed's `<enclosure>` (or Atom `link rel="enclosure"`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Enclosure {
    pub url: String,
    pub length: Option<u64>,
    pub mime: Option<String>,
}

/// One feed entry with what a torrent client can use from it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FeedItem {
    pub title: String,
    pub link: Option<String>,
    pub guid: Option<String>,
    /// Publication time, unix seconds.
    pub pub_date: Option<i64>,
    pub description: Option<String>,
    pub enclosure: Option<Enclosure>,
    pub magnet: Option<String>,
    pub info_hash: Option<InfoHashV1>,
    /// Total size in bytes.
    pub size: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub categories: Vec<String>,
    pub comments: Option<String>,
    /// Raw `torznab:attr` / `newznab:attr` name/value pairs, names lower-cased.
    pub attrs: Vec<(String, String)>,
}

impl FeedItem {
    /// The identity used for "already seen": guid, else link, else enclosure, else
    /// title. Truncated to 512 bytes.
    pub fn key(&self) -> String {
        let k = self.guid.as_deref().filter(|s| !s.is_empty())
            .or(self.link.as_deref().filter(|s| !s.is_empty()))
            .or(self.enclosure.as_ref().map(|e| e.url.as_str()))
            .unwrap_or(&self.title);
        String::from(truncate(k, 512))
    }
    /// What to hand the engine: the magnet when there is one, else the enclosure, else
    /// the link.
    pub fn torrent_url(&self) -> Option<&str> {
        self.magnet.as_deref()
            .or(self.enclosure.as_ref().map(|e| e.url.as_str()).filter(|u| !u.is_empty()))
            .or(self.link.as_deref())
    }
    /// The value of a Torznab/Newznab attribute by name (case-insensitive).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Which syntax a feed used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FeedKind { #[default] Rss, Atom }

/// A parsed feed.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Feed {
    pub kind: FeedKind,
    pub title: String,
    pub link: Option<String>,
    /// The channel's `<ttl>`: minutes the feed may be cached before refreshing.
    pub ttl_minutes: Option<u32>,
    pub items: Vec<FeedItem>,
}

/// Why a feed could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedError {
    /// The document hit a size/depth limit.
    Xml(xml::Error),
    /// The document has no `rss`, `RDF` or `feed` root.
    NotAFeed,
}

/// At most this many items are taken from one feed.
pub const MAX_ITEMS: usize = 5000;

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    let mut n = max;
    while !s.is_char_boundary(n) { n -= 1; }
    &s[..n]
}

/// Parses an RSS 2.0, RSS 1.0 or Atom document.
pub fn parse_feed(input: &[u8]) -> Result<Feed, FeedError> {
    let root = xml::parse(input).map_err(FeedError::Xml)?;
    if let Some(rss) = root.child("rss").or_else(|| root.child("RDF")) {
        let channel = rss.child("channel");
        let mut feed = Feed { kind: FeedKind::Rss, ..Feed::default() };
        if let Some(ch) = channel {
            feed.title = ch.child("title").map(|t| t.text()).unwrap_or_default();
            feed.link = ch.child("link").map(|l| l.text()).filter(|s| !s.is_empty());
            feed.ttl_minutes = ch.child("ttl").and_then(|t| t.text().parse().ok());
        }
        // RSS 2.0 puts items inside the channel; RSS 1.0 next to it.
        let items = channel.into_iter().flat_map(|c| c.children_named("item")).chain(rss.children_named("item"));
        feed.items = items.take(MAX_ITEMS).map(parse_item).collect();
        return Ok(feed);
    }
    if let Some(f) = root.child("feed") {
        let mut feed = Feed { kind: FeedKind::Atom, ..Feed::default() };
        feed.title = f.child("title").map(|t| t.text()).unwrap_or_default();
        feed.link = f.children_named("link").find(|l| l.attr("rel").map_or(true, |r| r == "alternate"))
            .and_then(|l| l.attr("href")).map(String::from);
        feed.items = f.children_named("entry").take(MAX_ITEMS).map(parse_item).collect();
        return Ok(feed);
    }
    Err(FeedError::NotAFeed)
}

fn parse_u64(s: &str) -> Option<u64> { s.trim().parse().ok() }
fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    s.parse().ok().or_else(|| s.parse::<u64>().ok().map(|v| v.min(u32::MAX as u64) as u32))
}

fn parse_item(e: &Element) -> FeedItem {
    let mut it = FeedItem::default();
    let mut date_primary = None;
    let mut date_updated = None;
    let mut desc_primary = None;
    let mut desc_content = None;
    let mut peers_total = None;
    for c in &e.children {
        let local = c.local().to_ascii_lowercase();
        match local.as_str() {
            "title" => it.title = c.text(),
            "link" => {
                if let Some(href) = c.attr("href") {
                    match c.attr("rel").unwrap_or("alternate") {
                        "enclosure" => it.enclosure = Some(Enclosure {
                            url: String::from(href),
                            length: c.attr("length").and_then(parse_u64).filter(|&n| n > 0),
                            mime: c.attr("type").map(String::from),
                        }),
                        "alternate" | "" => if it.link.is_none() { it.link = Some(String::from(href)) },
                        _ => {}
                    }
                } else {
                    let t = c.text();
                    if !t.is_empty() && it.link.is_none() { it.link = Some(t); }
                }
            }
            "guid" | "id" => { let t = c.text(); if !t.is_empty() { it.guid = Some(t); } }
            "pubdate" | "published" | "issued" => date_primary = parse_date(&c.text()),
            "updated" | "date" | "modified" => date_updated = parse_date(&c.text()),
            "description" | "summary" => desc_primary = Some(c.text()),
            "content" | "encoded" => if c.attr("url").is_none() { desc_content = Some(c.text()) } else { media(&mut it, c) },
            "enclosure" => if let Some(url) = c.attr("url") {
                it.enclosure = Some(Enclosure {
                    url: String::from(url),
                    length: c.attr("length").and_then(parse_u64).filter(|&n| n > 0),
                    mime: c.attr("type").map(String::from),
                });
            },
            "category" => {
                let t = c.attr("term").map(String::from).unwrap_or_else(|| c.text());
                if !t.is_empty() && it.categories.len() < 32 { it.categories.push(t); }
            }
            "comments" => it.comments = Some(c.text()),
            "infohash" => it.info_hash = parse_hash(&c.text()),
            "magneturi" | "magnet" => { let t = c.text(); if t.starts_with("magnet:") { it.magnet = Some(t); } }
            "contentlength" => it.size = parse_u64(&c.text()).or(it.size),
            "size" => it.size = parse_size(&c.text()).or(it.size),
            "seeders" | "seeds" => it.seeders = parse_u32(&c.text()),
            "leechers" => it.leechers = parse_u32(&c.text()),
            "peers" => if c.prefix().eq_ignore_ascii_case("torrent") { it.leechers = parse_u32(&c.text()) },
            "attr" => if let (Some(n), Some(v)) = (c.attr("name"), c.attr("value")) {
                let n = n.to_ascii_lowercase();
                match n.as_str() {
                    "seeders" => it.seeders = parse_u32(v),
                    "leechers" => it.leechers = parse_u32(v),
                    "peers" => peers_total = parse_u32(v),
                    "infohash" => it.info_hash = parse_hash(v).or(it.info_hash),
                    "magneturl" if v.starts_with("magnet:") => it.magnet = Some(String::from(v)),
                    "size" => it.size = parse_u64(v).or(it.size),
                    _ => {}
                }
                if it.attrs.len() < 64 { it.attrs.push((n, String::from(truncate(v, 4096)))); }
            },
            _ => {}
        }
    }
    it.pub_date = date_primary.or(date_updated);
    it.description = desc_primary.or(desc_content).filter(|d| !d.is_empty());
    if let (Some(total), None) = (peers_total, it.leechers) {
        it.leechers = Some(total.saturating_sub(it.seeders.unwrap_or(0)));
    }
    if it.magnet.is_none() {
        it.magnet = [it.link.as_deref(), it.enclosure.as_ref().map(|e| e.url.as_str()), it.guid.as_deref()]
            .into_iter().flatten().find(|u| u.starts_with("magnet:")).map(String::from)
            .or_else(|| it.description.as_deref().and_then(find_magnet));
    }
    if it.info_hash.is_none() { it.info_hash = it.magnet.as_deref().and_then(btih_from_magnet); }
    if it.size.is_none() { it.size = it.enclosure.as_ref().and_then(|e| e.length); }
    it
}

fn media(it: &mut FeedItem, c: &Element) {
    // media:content url="..." type="application/x-bittorrent"
    let url = c.attr("url").unwrap_or("");
    if it.enclosure.is_none() && (c.attr("type").is_some_and(|t| t.contains("bittorrent")) || url.starts_with("magnet:")) {
        it.enclosure = Some(Enclosure { url: String::from(url), length: c.attr("fileSize").and_then(parse_u64), mime: c.attr("type").map(String::from) });
    }
}

/// Finds the first `magnet:?` link in free text (a description) and returns it, cut at
/// whitespace, a quote or markup.
pub fn find_magnet(text: &str) -> Option<String> {
    let start = text.find("magnet:?")?;
    let rest = &text[start..];
    let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>')).unwrap_or(rest.len());
    Some(String::from(truncate(&rest[..end], 8192)))
}

/// A 40-hex or 32-base32 info-hash.
pub fn parse_hash(s: &str) -> Option<InfoHashV1> {
    let s = s.trim();
    match s.len() {
        40 => from_hex::<20>(s).map(InfoHashV1),
        32 => base32_20(s).map(InfoHashV1),
        _ => None,
    }
}

fn base32_20(s: &str) -> Option<[u8; 20]> {
    let mut out = [0u8; 20];
    let (mut acc, mut bits, mut n) = (0u64, 0u32, 0usize);
    for c in s.bytes() {
        let v = match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a', b'2'..=b'7' => c - b'2' + 26, _ => return None } as u64;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 { bits -= 8; if n < 20 { out[n] = (acc >> bits) as u8; } n += 1; acc &= (1 << bits) - 1; }
    }
    (n == 20).then_some(out)
}

/// The v1 info-hash in a magnet link's `xt=urn:btih:` (hex or base32).
pub fn btih_from_magnet(m: &str) -> Option<InfoHashV1> {
    let lower = m.to_ascii_lowercase();
    let p = lower.find("urn:btih:")? + 9;
    let rest = &m[p..];
    let end = rest.find('&').unwrap_or(rest.len());
    parse_hash(&rest[..end])
}

/// Parses a human size: `1.4 GiB`, `700 MB`, `123456` (bytes). Binary and decimal
/// units are both understood.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let num_end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(s.len());
    let (num, unit) = (s[..num_end].replace(',', ""), s[num_end..].trim().to_ascii_lowercase());
    let (int, frac) = num.split_once('.').unwrap_or((&num, ""));
    let int: u128 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let mut frac_milli: u128 = 0;
    for (k, d) in frac.bytes().take(3).enumerate() {
        if !d.is_ascii_digit() { return None; }
        frac_milli += (d - b'0') as u128 * [100, 10, 1][k];
    }
    let mult: u128 = match unit.as_str() {
        "" | "b" | "bytes" => 1,
        "k" | "kb" => 1000, "kib" => 1 << 10,
        "m" | "mb" => 1_000_000, "mib" => 1 << 20,
        "g" | "gb" => 1_000_000_000, "gib" => 1 << 30,
        "t" | "tb" => 1_000_000_000_000, "tib" => 1 << 40,
        _ => return None,
    };
    let v = (int.checked_mul(1000)? + frac_milli).checked_mul(mult)? / 1000;
    u64::try_from(v).ok()
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn valid_date(y: i64, m: u32, d: u32) -> bool {
    let dim = match m { 1 | 3 | 5 | 7 | 8 | 10 | 12 => 31, 4 | 6 | 9 | 11 => 30,
        2 => if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 }, _ => return false };
    (1..=dim).contains(&d) && (1..=9999).contains(&y)
}

fn hms(t: &str) -> Option<i64> {
    let mut parts = t.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let s: i64 = parts.next().map(|s| s.split('.').next().unwrap_or("0").parse().ok()).unwrap_or(Some(0))?;
    (h < 24 && m < 60 && s <= 60 && parts.next().is_none()).then_some(h * 3600 + m * 60 + s)
}

fn offset(z: &str) -> Option<i64> {
    let z = z.trim();
    Some(match z.to_ascii_uppercase().as_str() {
        "" | "Z" | "GMT" | "UT" | "UTC" => 0,
        "EST" => -5 * 3600, "EDT" => -4 * 3600, "CST" => -6 * 3600, "CDT" => -5 * 3600,
        "MST" => -7 * 3600, "MDT" => -6 * 3600, "PST" => -8 * 3600, "PDT" => -7 * 3600,
        "CET" => 3600, "CEST" => 7200, "JST" => 9 * 3600,
        _ => {
            let (sign, rest) = match z.as_bytes().first()? { b'+' => (1, &z[1..]), b'-' => (-1, &z[1..]), _ => return None };
            let digits: String = rest.chars().filter(|c| *c != ':').collect();
            if digits.len() != 4 || !digits.bytes().all(|c| c.is_ascii_digit()) { return None; }
            let h: i64 = digits[..2].parse().ok()?;
            let m: i64 = digits[2..].parse().ok()?;
            sign * (h * 3600 + m * 60)
        }
    })
}

/// Parses an RFC 822 / RFC 2822 date (`Sat, 14 Nov 2026 10:00:00 +0000`) to unix
/// seconds. Weekday optional, seconds optional, 2-digit years and US zone names allowed.
pub fn parse_rfc822(s: &str) -> Option<i64> {
    let mut toks = s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).peekable();
    if toks.peek()?.chars().next()?.is_ascii_alphabetic() { toks.next(); }
    let day: u32 = toks.next()?.parse().ok()?;
    let mon = toks.next()?.to_ascii_lowercase();
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter().position(|m| mon.starts_with(m))? as u32 + 1;
    let y: i64 = toks.next()?.parse().ok()?;
    let year = match y { 0..=49 => 2000 + y, 50..=999 => 1900 + y, _ => y };
    let secs = toks.next().map(hms).unwrap_or(Some(0))?;
    let off = offset(toks.next().unwrap_or(""))?;
    if !valid_date(year, month, day) { return None; }
    Some(days_from_civil(year, month, day) * 86_400 + secs - off)
}

/// Parses an RFC 3339 / ISO 8601 date-time (`2026-11-14T10:00:00Z`, fractional seconds
/// and offsets allowed; a space instead of `T` and a bare date are tolerated).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 10 || !s.is_char_boundary(10) { return None; }
    let (date, rest) = s.split_at(10);
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: u32 = d.next()?.parse().ok()?;
    let dd: u32 = d.next()?.parse().ok()?;
    if !valid_date(y, m, dd) { return None; }
    let rest = rest.trim_start_matches(['T', 't', ' ']);
    let (secs, off) = if rest.is_empty() { (0, 0) } else {
        let zpos = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
        let (t, z) = rest.split_at(zpos);
        (hms(t.trim())?, offset(z)?)
    };
    Some(days_from_civil(y, m, dd) * 86_400 + secs - off)
}

/// Either date format.
pub fn parse_date(s: &str) -> Option<i64> { parse_rfc822(s).or_else(|| parse_rfc3339(s)) }

/// Per-feed state the program keeps between refreshes.
#[derive(Clone, Debug)]
pub struct FeedState {
    pub url: String,
    /// The user's refresh interval.
    pub interval_ms: u64,
    /// The feed's own `<ttl>` (clamped to a day), if it gave one.
    pub ttl_ms: Option<u64>,
    pub last_attempt: Option<Millis>,
    pub last_success: Option<Millis>,
    /// Failures in a row (drives the backoff).
    pub failures: u32,
    /// `ETag` and `Last-Modified` from the last 200 response.
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// How many item keys to remember.
    pub seen_cap: usize,
    seen: BTreeMap<String, u64>,
    generation: u64,
}

/// Feeds are never polled more often than this.
pub const MIN_INTERVAL_MS: u64 = 60_000;
/// Backoff after failures never waits longer than this.
pub const MAX_BACKOFF_MS: u64 = 6 * 3600 * 1000;

impl FeedState {
    /// A new feed, due immediately.
    pub fn new(url: &str, interval_ms: u64) -> FeedState {
        FeedState {
            url: String::from(url), interval_ms, ttl_ms: None, last_attempt: None, last_success: None,
            failures: 0, etag: None, last_modified: None, seen_cap: 4096, seen: BTreeMap::new(), generation: 0,
        }
    }
    /// The interval in effect: the user's, at least the feed's TTL and the minimum.
    pub fn effective_interval(&self) -> u64 {
        self.interval_ms.max(self.ttl_ms.unwrap_or(0)).max(MIN_INTERVAL_MS)
    }
    /// When the next fetch should happen (0 = now).
    pub fn next_due(&self) -> Millis {
        let Some(last) = self.last_attempt else { return 0 };
        let base = self.effective_interval();
        let wait = if self.failures == 0 { base } else {
            base.saturating_mul(1u64 << self.failures.min(10)).min(MAX_BACKOFF_MS.max(base))
        };
        last.saturating_add(wait)
    }
    pub fn is_due(&self, now: Millis) -> bool { now >= self.next_due() }
    /// Headers for a conditional GET.
    pub fn request_headers(&self) -> Vec<(&'static str, String)> {
        let mut h = Vec::new();
        if let Some(e) = &self.etag { h.push(("If-None-Match", e.clone())); }
        if let Some(m) = &self.last_modified { h.push(("If-Modified-Since", m.clone())); }
        h
    }
    /// Whether an item key has been seen.
    pub fn is_seen(&self, key: &str) -> bool { self.seen.contains_key(truncate(key, 512)) }
    /// Number of remembered keys.
    pub fn seen_len(&self) -> usize { self.seen.len() }
    /// Marks a key seen.
    pub fn mark_seen(&mut self, key: &str) {
        self.seen.insert(String::from(truncate(key, 512)), self.generation);
        self.evict();
    }
    fn evict(&mut self) {
        if self.seen.len() <= self.seen_cap { return; }
        let mut gens: Vec<u64> = self.seen.values().copied().collect();
        gens.sort_unstable();
        let cut = gens[self.seen.len() - self.seen_cap];
        // Drop the oldest generations; keep at least the newest generation whole.
        let cut = cut.min(self.generation);
        self.seen.retain(|_, g| *g >= cut);
        if self.seen.len() > self.seen_cap {
            let excess = self.seen.len() - self.seen_cap;
            let old: Vec<String> = self.seen.iter().filter(|(_, g)| **g == cut && cut < self.generation).map(|(k, _)| k.clone()).take(excess).collect();
            for k in old { self.seen.remove(&k); }
        }
    }
    /// Records a fetch that returned this feed; returns the indices of items not seen
    /// before (in feed order) and marks every current item seen.
    pub fn on_success(&mut self, now: Millis, feed: &Feed, etag: Option<&str>, last_modified: Option<&str>) -> Vec<usize> {
        self.last_attempt = Some(now);
        self.last_success = Some(now);
        self.failures = 0;
        self.ttl_ms = feed.ttl_minutes.map(|m| (m as u64).min(24 * 60) * 60_000);
        self.etag = etag.map(|s| String::from(truncate(s, 256)));
        self.last_modified = last_modified.map(|s| String::from(truncate(s, 64)));
        self.generation += 1;
        self.seen_cap = self.seen_cap.max(feed.items.len() * 2);
        let mut fresh = Vec::new();
        for (i, it) in feed.items.iter().enumerate() {
            let k = it.key();
            if self.seen.insert(k, self.generation).is_none() { fresh.push(i); }
        }
        self.evict();
        fresh
    }
    /// Records an HTTP 304 Not Modified.
    pub fn on_not_modified(&mut self, now: Millis) {
        self.last_attempt = Some(now);
        self.last_success = Some(now);
        self.failures = 0;
    }
    /// Records a failed fetch (network error, bad status, unparsable body).
    pub fn on_failure(&mut self, now: Millis) {
        self.last_attempt = Some(now);
        self.failures = self.failures.saturating_add(1);
    }
}

impl core::fmt::Display for FeedError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self { FeedError::Xml(e) => write!(f, "feed too large or deep: {:?}", e), FeedError::NotAFeed => f.write_str("not an RSS or Atom feed") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    const NYAA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:atom="http://www.w3.org/2005/Atom" xmlns:nyaa="https://nyaa.si/xmlns/nyaa" version="2.0">
  <channel>
    <title>Nyaa - Home - Torrent File RSS</title>
    <description>RSS Feed for Home</description>
    <link>https://nyaa.si/</link>
    <ttl>15</ttl>
    <atom:link href="https://nyaa.si/?page=rss" rel="self" type="application/rss+xml" />
    <item>
      <title>[SubsPlease] Frieren - 05 (1080p) [A1B2C3D4].mkv</title>
      <link>https://nyaa.si/download/1888888.torrent</link>
      <guid isPermaLink="true">https://nyaa.si/view/1888888</guid>
      <pubDate>Sat, 14 Nov 2026 10:00:00 -0000</pubDate>
      <nyaa:seeders>1234</nyaa:seeders>
      <nyaa:leechers>56</nyaa:leechers>
      <nyaa:downloads>7890</nyaa:downloads>
      <nyaa:infoHash>0123456789abcdef0123456789abcdef01234567</nyaa:infoHash>
      <nyaa:categoryId>1_2</nyaa:categoryId>
      <nyaa:category>Anime - English-translated</nyaa:category>
      <nyaa:size>1.4 GiB</nyaa:size>
      <nyaa:comments>0</nyaa:comments>
      <description><![CDATA[<a href="https://nyaa.si/view/1888888">#1888888 | [SubsPlease] Frieren - 05</a> | 1.4 GiB | Anime]]></description>
    </item>
    <item>
      <title>Second &amp; &#8220;quoted&#x201D; &bogus; item</title>
      <link>https://nyaa.si/download/1888887.torrent</link>
      <guid isPermaLink="true">https://nyaa.si/view/1888887</guid>
      <pubDate>Fri, 13 Nov 2026 23:59:59 GMT</pubDate>
      <nyaa:size>700 MiB</nyaa:size>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn nyaa_feed() {
        let f = parse_feed(NYAA.as_bytes()).unwrap();
        assert_eq!(f.kind, FeedKind::Rss);
        assert_eq!(f.title, "Nyaa - Home - Torrent File RSS");
        assert_eq!(f.ttl_minutes, Some(15));
        assert_eq!(f.items.len(), 2);
        let a = &f.items[0];
        assert_eq!(a.title, "[SubsPlease] Frieren - 05 (1080p) [A1B2C3D4].mkv");
        assert_eq!(a.link.as_deref(), Some("https://nyaa.si/download/1888888.torrent"));
        assert_eq!(a.guid.as_deref(), Some("https://nyaa.si/view/1888888"));
        assert_eq!(a.pub_date, Some(days_from_civil(2026, 11, 14) * 86400 + 36000));
        assert_eq!(a.seeders, Some(1234));
        assert_eq!(a.leechers, Some(56));
        assert_eq!(a.size, Some(1503238553)); // 1.4 * 2^30, floored
        assert_eq!(crate::to_hex(&a.info_hash.unwrap().0), "0123456789abcdef0123456789abcdef01234567");
        assert!(a.description.as_ref().unwrap().contains("<a href="));
        assert_eq!(a.torrent_url(), Some("https://nyaa.si/download/1888888.torrent"));
        let b = &f.items[1];
        assert_eq!(b.title, "Second & \u{201C}quoted\u{201D} &bogus; item");
        assert_eq!(b.size, Some(700 << 20));
        assert_eq!(b.pub_date, Some(days_from_civil(2026, 11, 14) * 86400 - 1));
    }

    #[test]
    fn ezrss_torrent_namespace_and_enclosure() {
        let doc = r#"<rss version="2.0" xmlns:torrent="http://xmlns.ezrss.it/0.1/"><channel><title>ez</title>
<item><title>Show.Name.S02E03.1080p.WEB.h264-GRP</title>
<link>magnet:?xt=urn:btih:ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD&amp;dn=Show</link>
<enclosure url="https://ex.org/t/123.torrent" length="2147483648" type="application/x-bittorrent"/>
<torrent:contentLength>2147483648</torrent:contentLength>
<torrent:infoHash>abcdefabcdefabcdefabcdefabcdefabcdefabcd</torrent:infoHash>
<torrent:magnetURI><![CDATA[magnet:?xt=urn:btih:ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD&dn=Show]]></torrent:magnetURI>
<torrent:seeds>42</torrent:seeds><torrent:peers>7</torrent:peers>
<pubDate>Mon, 02 Nov 2026 08:30 EST</pubDate></item></channel></rss>"#;
        let f = parse_feed(doc.as_bytes()).unwrap();
        let a = &f.items[0];
        assert_eq!(a.magnet.as_deref(), Some("magnet:?xt=urn:btih:ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD&dn=Show"));
        assert_eq!(a.enclosure.as_ref().unwrap().length, Some(2147483648));
        assert_eq!(a.size, Some(2147483648));
        assert_eq!((a.seeders, a.leechers), (Some(42), Some(7)));
        assert_eq!(a.pub_date, Some(days_from_civil(2026, 11, 2) * 86400 + 8 * 3600 + 1800 + 5 * 3600));
        assert!(a.torrent_url().unwrap().starts_with("magnet:"));
        // The link (no guid) is the key.
        assert!(a.key().starts_with("magnet:"));
    }

    #[test]
    fn atom_feed() {
        let doc = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title type="text">Releases</title>
<link rel="self" href="https://ex.org/atom"/><link href="https://ex.org/"/>
<entry><title>Movie.2026.2160p.BluRay.x265</title><id>urn:uuid:1225c695-cfb8-4ebb-aaaa-80da344efa6a</id>
<updated>2026-11-14T09:30:00.123+01:00</updated>
<link rel="alternate" href="https://ex.org/m/1"/>
<link rel="enclosure" type="application/x-bittorrent" length="5000" href="https://ex.org/m/1.torrent"/>
<category term="Movies"/>
<summary type="html">Get it: &lt;a href="magnet:?xt=urn:btih:MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43U&amp;dn=x"&gt;magnet&lt;/a&gt;</summary>
</entry></feed>"#;
        let f = parse_feed(doc.as_bytes()).unwrap();
        assert_eq!(f.kind, FeedKind::Atom);
        assert_eq!(f.link.as_deref(), Some("https://ex.org/"));
        let a = &f.items[0];
        assert_eq!(a.link.as_deref(), Some("https://ex.org/m/1"));
        assert_eq!(a.enclosure.as_ref().unwrap().url, "https://ex.org/m/1.torrent");
        assert_eq!(a.categories, alloc::vec![String::from("Movies")]);
        assert_eq!(a.pub_date, Some(days_from_civil(2026, 11, 14) * 86400 + 8 * 3600 + 1800));
        assert_eq!(a.magnet.as_deref(), Some("magnet:?xt=urn:btih:MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43U&dn=x"));
        // Base32 "abcdefghijklmnopqrst".
        assert_eq!(&a.info_hash.unwrap().0, b"abcdefghijklmnopqrst");
        assert_eq!(a.key(), "urn:uuid:1225c695-cfb8-4ebb-aaaa-80da344efa6a");
    }

    #[test]
    fn rdf_feed_and_torznab_attrs() {
        let doc = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/">
<channel><title>RDF</title></channel>
<item><title>One</title><link>https://a/1</link>
<torznab:attr name="seeders" value="10"/><torznab:attr name="peers" value="25"/>
<torznab:attr name="infohash" value="00112233445566778899AABBCCDDEEFF00112233"/></item></rdf:RDF>"#;
        let f = parse_feed(doc.as_bytes()).unwrap();
        assert_eq!(f.items.len(), 1);
        let a = &f.items[0];
        assert_eq!((a.seeders, a.leechers), (Some(10), Some(15)));
        assert_eq!(a.attr("PEERS"), Some("25"));
        assert!(a.info_hash.is_some());
    }

    #[test]
    fn tolerant_markup() {
        let doc = "\u{FEFF}<!DOCTYPE rss [<!ENTITY x \"y\">]><rss><channel><title>a < b</title>\
<item><title>Broken <b>bold</title><description>unclosed <p>para</description></item>\
<item><title>Ünïcödé</title></channel>";
        let f = parse_feed(doc.as_bytes()).unwrap();
        assert_eq!(f.title, "a < b");
        assert_eq!(f.items.len(), 2);
        assert_eq!(f.items[0].title, "Broken bold");
        assert_eq!(f.items[0].description.as_deref(), Some("unclosed para"));
        assert_eq!(f.items[1].title, "Ünïcödé");
    }

    #[test]
    fn latin1_fallback() {
        let mut doc = b"<rss><channel><item><title>Caf".to_vec();
        doc.push(0xE9);
        doc.extend_from_slice(b"</title></item></channel></rss>");
        assert_eq!(parse_feed(&doc).unwrap().items[0].title, "Café");
    }

    #[test]
    fn not_a_feed_and_limits() {
        assert_eq!(parse_feed(b"<html><body>hi</body></html>"), Err(FeedError::NotAFeed));
        assert_eq!(parse_feed(b""), Err(FeedError::NotAFeed));
        let deep = "<a>".repeat(10_000);
        assert_eq!(parse_feed(deep.as_bytes()), Err(FeedError::Xml(xml::Error::TooDeep)));
        // Many self-closing elements hit the node limit, not memory exhaustion.
        let many = format!("<rss>{}</rss>", "<x/>".repeat(xml::MAX_NODES + 1));
        assert_eq!(parse_feed(many.as_bytes()), Err(FeedError::Xml(xml::Error::TooManyNodes)));
    }

    #[test]
    fn garbage_never_panics() {
        let samples: [&[u8]; 12] = [b"<", b"<!", b"<!--", b"<![CDATA[", b"<?", b"</", b"<a b='", b"<a b=\"x",
            b"&#xFFFFFFFF;&#0;&#;&", b"<rss><channel><item><link href=", b"<a/ / / >", b"<\xff\xfe>"];
        for s in samples { let _ = parse_feed(s); }
        let mut seed = 12345u32;
        for _ in 0..300 {
            let mut v = Vec::new();
            for _ in 0..200 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let alpha = b"<>/=\"' a&#;!-[]?rssitemchanneltitle:x";
                v.push(alpha[(seed >> 16) as usize % alpha.len()]);
            }
            let _ = parse_feed(&v);
        }
    }

    #[test]
    fn dates() {
        let base = days_from_civil(2026, 11, 14) * 86400;
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(parse_rfc822("Sat, 14 Nov 2026 10:00:00 +0200"), Some(base + 8 * 3600));
        assert_eq!(parse_rfc822("14 Nov 26 10:00 Z"), Some(base + 10 * 3600));
        assert_eq!(parse_rfc822("Saturday, 14 November 2026"), Some(base));
        assert_eq!(parse_rfc822("31 Feb 2026 10:00:00 GMT"), None);
        assert_eq!(parse_rfc822("Sat, 14 Nov 2026 10:00:00 +02"), None);
        assert_eq!(parse_rfc3339("2026-11-14T00:00:00Z"), Some(base));
        assert_eq!(parse_rfc3339("2026-11-14 01:00:00-01:00"), Some(base + 2 * 3600));
        assert_eq!(parse_rfc3339("2026-11-14"), Some(base));
        assert_eq!(parse_date("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_date("garbage"), None);
        assert_eq!(parse_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
    }

    #[test]
    fn sizes_and_hashes() {
        assert_eq!(parse_size("123456"), Some(123456));
        assert_eq!(parse_size("1.5 GB"), Some(1_500_000_000));
        assert_eq!(parse_size("1,024 KiB"), Some(1 << 20));
        assert_eq!(parse_size("2 TiB"), Some(2 << 40));
        assert_eq!(parse_size("99999999999999999999 TiB"), None);
        assert_eq!(parse_size("12 parsecs"), None);
        assert!(parse_hash("xyz").is_none());
        assert!(parse_hash("g123456789abcdef0123456789abcdef01234567").is_none());
        assert_eq!(btih_from_magnet("magnet:?dn=a&XT=URN:BTIH:0123456789abcdef0123456789abcdef01234567&tr=x").map(|h| h.0[0]), Some(0x01));
        assert_eq!(find_magnet("see magnet:?xt=urn:btih:aa&dn=b c"), Some(String::from("magnet:?xt=urn:btih:aa&dn=b")));
    }

    #[test]
    fn feed_state_seen_and_schedule() {
        let f = parse_feed(NYAA.as_bytes()).unwrap();
        let mut st = FeedState::new("https://nyaa.si/?page=rss", 5 * 60_000);
        assert!(st.is_due(0));
        assert_eq!(st.on_success(1000, &f, Some("\"abc\""), Some("Sat, 14 Nov 2026 10:00:00 GMT")), alloc::vec![0, 1]);
        assert_eq!(st.on_success(2000, &f, None, None), Vec::<usize>::new());
        // TTL 15 minutes overrides the 5-minute interval.
        assert_eq!(st.effective_interval(), 15 * 60_000);
        assert!(!st.is_due(2000 + 14 * 60_000));
        assert!(st.is_due(2000 + 15 * 60_000));
        st.on_failure(10_000_000);
        assert_eq!(st.next_due(), 10_000_000 + 30 * 60_000);
        st.on_failure(20_000_000);
        assert_eq!(st.next_due(), 20_000_000 + 60 * 60_000);
        for _ in 0..40 { st.on_failure(0); }
        assert_eq!(st.next_due(), MAX_BACKOFF_MS);
        st.on_not_modified(5);
        assert_eq!(st.failures, 0);
        assert!(st.is_seen("https://nyaa.si/view/1888888"));
        assert_eq!(st.request_headers().len(), 0); // the second success had no validators
    }

    #[test]
    fn seen_set_is_bounded_but_keeps_current_items() {
        let mut st = FeedState::new("u", 0);
        st.seen_cap = 10;
        for round in 0..20u32 {
            let feed = Feed { items: (0..4).map(|i| FeedItem { guid: Some(format!("g{}-{}", round, i)), ..FeedItem::default() }).collect(), ..Feed::default() };
            assert_eq!(st.on_success(round as u64, &feed, None, None).len(), 4);
            // The current items are remembered, so they are not new again.
            assert!(st.on_success(round as u64, &feed, None, None).is_empty());
            assert!(st.seen_len() <= 10);
        }
    }
}
