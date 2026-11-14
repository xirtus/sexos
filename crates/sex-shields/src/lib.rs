// SPDX-License-Identifier: GPL-3.0-only
//! Shields: ad and tracker blocking for SexOS programs, without any I/O.
//!
//! Reads filter lists in the Adblock Plus / uBlock Origin syntax that EasyList and
//! EasyPrivacy are written in, and answers two questions:
//!
//! * may this request go out? ([`Engine::check`]): network filters (`||ads.example^`,
//!   `/banner/*$image,third-party`, `@@` exceptions, `$domain=`, `$important`), found
//!   through a token index so a request costs a few map lookups, not a list walk;
//! * what should a page hide? ([`Engine::hidden`], [`Engine::css`]): element hiding
//!   (`##.ad-slot`, `example.com##.promo`, `#@#` exceptions). Generic selectors are
//!   kept by the class or id they need, so only those whose class or id is on the
//!   page are sent to its style sheets.
//!
//! Kaleidoscope uses it for every request a page makes, Qupid for the media requests
//! it makes. Both own their lists and settings; this crate only decides.
//!
//! Filters this engine can't honour exactly (regular expressions, `$csp`,
//! `$removeparam`, procedural cosmetics such as `:has-text()`, scriptlets) are left
//! out and counted in [`Engine::skipped`], never guessed at: a guessed filter blocks
//! the wrong things.
#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// What a request is for, as filter options name it (`$script`, `$image`...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    /// The page itself (a tab's main load). Never blocked; `@@...$document` exceptions
    /// turn Shields off for the page.
    Document = 1,
    Subdocument = 1 << 1,
    Script = 1 << 2,
    Stylesheet = 1 << 3,
    Image = 1 << 4,
    Font = 1 << 5,
    Media = 1 << 6,
    Xhr = 1 << 7,
    WebSocket = 1 << 8,
    Ping = 1 << 9,
    Object = 1 << 10,
    Other = 1 << 11,
}

const ALL_BUT_DOCUMENT: u16 = 0x0ffe;

impl Kind {
    /// A guess from the address alone, for a request whose purpose the caller doesn't
    /// know: its file extension, else [`Kind::Other`].
    pub fn guess(url: &str) -> Kind {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let last = path.rsplit('/').next().unwrap_or("");
        let ext = match last.rfind('.') { Some(i) => &last[i + 1..], None => return Kind::Other };
        let mut low = [0u8; 8];
        if ext.len() > low.len() { return Kind::Other; }
        for (d, s) in low.iter_mut().zip(ext.bytes()) { *d = s.to_ascii_lowercase(); }
        match &low[..ext.len()] {
            b"js" | b"mjs" => Kind::Script,
            b"css" => Kind::Stylesheet,
            b"png" | b"jpg" | b"jpeg" | b"gif" | b"webp" | b"svg" | b"ico" | b"avif" | b"bmp" => Kind::Image,
            b"woff" | b"woff2" | b"ttf" | b"otf" | b"eot" => Kind::Font,
            b"mp4" | b"webm" | b"mp3" | b"ogg" | b"m4a" | b"m3u8" | b"ts" | b"m4s" | b"wav" | b"flac" | b"opus" => Kind::Media,
            b"html" | b"htm" => Kind::Subdocument,
            _ => Kind::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Document => "document", Kind::Subdocument => "subdocument", Kind::Script => "script",
            Kind::Stylesheet => "stylesheet", Kind::Image => "image", Kind::Font => "font", Kind::Media => "media",
            Kind::Xhr => "xmlhttprequest", Kind::WebSocket => "websocket", Kind::Ping => "ping",
            Kind::Object => "object", Kind::Other => "other",
        }
    }
}

/// A request to judge: where it goes, the page asking, and what for.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub url: &'a str,
    /// The address of the page (or frame) making the request.
    pub page: &'a str,
    pub kind: Kind,
}

/// The answer for one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No filter matched.
    Pass,
    /// A blocking filter matched; the text is that filter as written in its list.
    Block(u32),
    /// A blocking filter matched but an `@@` exception let it through.
    Excepted(u32),
}

impl Verdict {
    pub fn blocked(self) -> bool { matches!(self, Verdict::Block(_)) }
}

/// One network filter.
struct Net {
    /// Pattern bytes, lower-case unless `match_case`. `*` any run, `^` a separator.
    pattern: Vec<u8>,
    host_anchor: bool,
    start_anchor: bool,
    end_anchor: bool,
    match_case: bool,
    exception: bool,
    important: bool,
    kinds: u16,
    /// `Some(true)`: third-party requests only; `Some(false)`: first-party only.
    third: Option<bool>,
    domains: Domains,
    /// Exceptions only: `$elemhide`, `$generichide`.
    elemhide: bool,
    generichide: bool,
    /// Where the filter text is kept, for [`Engine::filter_text`].
    text: u32,
}

/// A `domain=`/cosmetic domain list: the filter applies on these sites and not those.
#[derive(Default)]
struct Domains { on: Vec<String>, off: Vec<String> }

impl Domains {
    fn parse(list: &str, sep: char) -> Domains {
        let mut d = Domains::default();
        for e in list.split(sep).map(str::trim).filter(|e| !e.is_empty()) {
            match e.strip_prefix('~') { Some(n) => d.off.push(n.to_ascii_lowercase()), None => d.on.push(e.to_ascii_lowercase()) }
        }
        d
    }
    fn is_empty(&self) -> bool { self.on.is_empty() && self.off.is_empty() }
    /// Whether a page on `host` is covered.
    fn applies(&self, host: &str) -> bool {
        if self.off.iter().any(|d| domain_matches(host, d)) { return false; }
        self.on.is_empty() || self.on.iter().any(|d| domain_matches(host, d))
    }
}

/// `host` is `d` or below it; `d` ending in `.*` is any public suffix (`google.*`).
fn domain_matches(host: &str, d: &str) -> bool {
    if let Some(base) = d.strip_suffix(".*") {
        let Some(suffix) = psl::suffix_str(host) else { return false };
        let Some(rest) = host.strip_suffix(suffix).and_then(|r| r.strip_suffix('.')) else { return false };
        return rest == base || rest.ends_with(base) && rest.as_bytes()[rest.len() - base.len() - 1] == b'.';
    }
    host == d || host.len() > d.len() && host.ends_with(d) && host.as_bytes()[host.len() - d.len() - 1] == b'.'
}

/// One element hiding rule that names its sites.
struct Specific { domains: Domains, selector: String, exception: bool }

/// Filter counts, for a settings page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts { pub network: usize, pub cosmetic: usize, pub skipped: usize }

/// The filters of every list added, ready to answer.
#[derive(Default)]
pub struct Engine {
    nets: Vec<Net>,
    /// Network filters by the hash of their rarest-looking token.
    index: BTreeMap<u32, Vec<u32>>,
    /// Network filters with no usable token: tried on every request.
    untokened: Vec<u32>,
    /// Filter texts, one string, for reporting which filter matched.
    texts: String,
    text_ends: Vec<u32>,
    /// Generic hiding selectors keyed by a class (`.x`) or id (`#x`) they need.
    generic_keyed: BTreeMap<String, Vec<String>>,
    /// Generic hiding selectors that need no particular class or id.
    generic_always: Vec<String>,
    /// Generic selectors excepted everywhere (`#@#.x`).
    generic_off: Vec<String>,
    /// Hiding rules naming sites, by each site named (`google.*` kept as written).
    specific: Vec<Specific>,
    specific_by_domain: BTreeMap<String, Vec<u32>>,
    /// Hiding rules naming only sites they don't apply on (`~a.com##.x`).
    specific_negated: Vec<u32>,
    counts: Counts,
    badfilters: Vec<String>,
}

const SEPARATORS: &[&str] = &["#@?#", "#@$#", "#@#", "#?#", "#$#", "#%#", "##"];

impl Engine {
    pub fn new() -> Engine { Engine::default() }

    /// Adds every filter of a list (text in Adblock Plus syntax, or a hosts file).
    pub fn add_list(&mut self, text: &str) {
        for line in text.lines() { self.add_filter(line); }
        self.apply_badfilters();
    }

    pub fn counts(&self) -> Counts { self.counts }

    /// Adds one filter line. Returns whether it is honoured.
    pub fn add_filter(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() || line.starts_with('!') || line.starts_with('[') { return false; }
        // Hosts file lines: `0.0.0.0 tracker.example`.
        if let Some(host) = line.strip_prefix("0.0.0.0 ").or_else(|| line.strip_prefix("127.0.0.1 ")) {
            let host = host.split('#').next().unwrap_or("").trim();
            if host.is_empty() || host.contains(char::is_whitespace) || host == "localhost" || host == "0.0.0.0" { return false; }
            let mut f = String::from("||");
            f.push_str(host);
            f.push('^');
            return self.add_filter(&f);
        }
        let ok = match cosmetic_split(line) {
            Some((domains, sep, body)) => self.add_cosmetic(domains, sep, body),
            None => self.add_network(line),
        };
        if !ok { self.counts.skipped += 1; }
        ok
    }

    fn keep_text(&mut self, text: &str) -> u32 {
        self.texts.push_str(text);
        self.text_ends.push(self.texts.len() as u32);
        self.text_ends.len() as u32 - 1
    }

    /// The text of the filter a [`Verdict`] names.
    pub fn filter_text(&self, id: u32) -> &str {
        let net = &self.nets[id as usize];
        let i = net.text as usize;
        let start = if i == 0 { 0 } else { self.text_ends[i - 1] as usize };
        &self.texts[start..self.text_ends[i] as usize]
    }

    fn add_network(&mut self, line: &str) -> bool {
        let mut rest = line;
        let exception = if let Some(r) = rest.strip_prefix("@@") { rest = r; true } else { false };
        // Options follow the last `$`, unless the filter is a /regex/ that holds one.
        let (mut pattern, options) = match rest.rfind('$') {
            Some(i) if !(rest.starts_with('/') && rest[i..].contains('/')) => (&rest[..i], Some(&rest[i + 1..])),
            _ => (rest, None),
        };
        if pattern.len() > 1 && pattern.starts_with('/') && pattern.ends_with('/') { return false; }
        let mut f = Net {
            pattern: Vec::new(), host_anchor: false, start_anchor: false, end_anchor: false, match_case: false,
            exception, important: false, kinds: 0, third: None, domains: Domains::default(),
            elemhide: false, generichide: false, text: 0,
        };
        let (mut on, mut off) = (0u16, 0u16);
        let mut badfilter = false;
        if let Some(options) = options {
            for opt in options.split(',') {
                let opt = opt.trim();
                let (neg, name) = match opt.strip_prefix('~') { Some(n) => (true, n), None => (false, opt) };
                let (key, value) = match name.find('=') { Some(i) => (&name[..i], Some(&name[i + 1..])), None => (name, None) };
                let kind = match key {
                    "script" => Some(Kind::Script as u16),
                    "image" => Some(Kind::Image as u16),
                    "stylesheet" | "css" => Some(Kind::Stylesheet as u16),
                    "object" | "object-subrequest" => Some(Kind::Object as u16),
                    "xmlhttprequest" | "xhr" => Some(Kind::Xhr as u16),
                    "subdocument" | "frame" => Some(Kind::Subdocument as u16),
                    "font" => Some(Kind::Font as u16),
                    "media" => Some(Kind::Media as u16),
                    "websocket" => Some(Kind::WebSocket as u16),
                    "ping" | "beacon" => Some(Kind::Ping as u16),
                    "other" => Some(Kind::Other as u16),
                    "document" | "doc" => Some(Kind::Document as u16),
                    "all" => Some(ALL_BUT_DOCUMENT | Kind::Document as u16),
                    _ => None,
                };
                if let Some(k) = kind {
                    if neg { off |= k } else { on |= k }
                    continue;
                }
                match (key, neg) {
                    ("third-party" | "3p", false) | ("first-party" | "1p", true) => f.third = Some(true),
                    ("third-party" | "3p", true) | ("first-party" | "1p", false) => f.third = Some(false),
                    ("domain" | "from", false) => f.domains = Domains::parse(value.unwrap_or(""), '|'),
                    ("match-case", false) => f.match_case = true,
                    ("important", false) => f.important = true,
                    ("elemhide" | "ehide", false) if exception => f.elemhide = true,
                    ("generichide" | "ghide", false) if exception => f.generichide = true,
                    // Pages can't open pop-ups here; a rule only about them is moot.
                    ("popup", false) => return false,
                    // A blocked request is blocked; there is no stand-in resource yet.
                    ("redirect", false) => {}
                    ("badfilter", false) => badfilter = true,
                    _ => return false,
                }
            }
        }
        if badfilter {
            let mut target = String::from(&line[..line.len() - options.map_or(0, |o| o.len() + 1)]);
            let kept: Vec<&str> = options.unwrap_or("").split(',').filter(|o| o.trim() != "badfilter").collect();
            if !kept.is_empty() { target.push('$'); target.push_str(&kept.join(",")); }
            self.badfilters.push(target);
            return true;
        }
        f.kinds = if on != 0 { on & !off } else if off != 0 { (ALL_BUT_DOCUMENT | Kind::Document as u16) & !off & !(Kind::Document as u16) } else { ALL_BUT_DOCUMENT };
        // A blocking filter for whole pages only: tabs are never blocked.
        if f.kinds == 0 || (!exception && f.kinds == Kind::Document as u16) { return false; }
        if f.elemhide || f.generichide { f.kinds = Kind::Document as u16; }
        if let Some(p) = pattern.strip_prefix("||") { f.host_anchor = true; pattern = p; }
        else if let Some(p) = pattern.strip_prefix('|') { f.start_anchor = true; pattern = p; }
        if let Some(p) = pattern.strip_suffix('|') { f.end_anchor = true; pattern = p; }
        let pattern = pattern.trim_start_matches('*');
        let pattern = if f.end_anchor { pattern } else { pattern.trim_end_matches('*') };
        if pattern.is_empty() && f.domains.is_empty() && f.third.is_none() && !(f.elemhide || f.generichide) { return false; }
        f.pattern = if f.match_case { Vec::from(pattern.as_bytes()) } else { pattern.as_bytes().to_ascii_lowercase() };
        let id = self.nets.len() as u32;
        match best_token(&f.pattern.to_ascii_lowercase(), f.host_anchor || f.start_anchor, f.end_anchor) {
            Some(t) => self.index.entry(t).or_default().push(id),
            None => self.untokened.push(id),
        }
        f.text = self.keep_text(line);
        self.nets.push(f);
        self.counts.network += 1;
        true
    }

    fn apply_badfilters(&mut self) {
        if self.badfilters.is_empty() { return; }
        let bad = core::mem::take(&mut self.badfilters);
        let dead: Vec<u32> = (0..self.nets.len() as u32).filter(|&i| bad.iter().any(|b| b == self.filter_text(i))).collect();
        if dead.is_empty() { return; }
        for list in self.index.values_mut() { list.retain(|i| !dead.contains(i)); }
        self.untokened.retain(|i| !dead.contains(i));
        self.counts.network -= dead.len();
    }

    fn add_cosmetic(&mut self, domains: &str, sep: &str, selector: &str) -> bool {
        let exception = sep.starts_with("#@");
        // Procedural (`#?#`), style injection (`#$#`), scriptlets and HTML filters.
        if sep != "##" && sep != "#@#" { return false; }
        let selector = selector.trim();
        if selector.is_empty() || selector.starts_with("+js(") || selector.starts_with('^') || !plain_selector(selector) { return false; }
        let domains = Domains::parse(domains, ',');
        self.counts.cosmetic += 1;
        if domains.is_empty() {
            if exception { self.generic_off.push(String::from(selector)); return true; }
            match selector_key(selector) {
                Some(key) => self.generic_keyed.entry(key).or_default().push(String::from(selector)),
                None => self.generic_always.push(String::from(selector)),
            }
            return true;
        }
        let id = self.specific.len() as u32;
        if domains.on.is_empty() { self.specific_negated.push(id); }
        for d in &domains.on { self.specific_by_domain.entry(d.clone()).or_default().push(id); }
        self.specific.push(Specific { domains, selector: String::from(selector), exception });
        true
    }

    /// Judges one request. A page an `@@...$document` exception covers passes all.
    pub fn check(&self, r: &Request) -> Verdict {
        let Some(url) = Target::parse(r.url) else { return Verdict::Pass };
        let page_host = Target::parse(r.page).map(|p| String::from(p.host())).unwrap_or_default();
        let third = match (psl::domain_str(url.host()), psl::domain_str(&page_host)) {
            (Some(a), Some(b)) => a != b,
            _ => url.host() != page_host,
        };
        let kind = r.kind as u16;
        // `@@||site^$document`: the person's lists trust this page with everything.
        if self.page_exceptions(r.page).0 { return Verdict::Pass; }
        let mut candidates: Vec<u32> = Vec::new();
        for t in tokens(url.low.as_bytes()) {
            if let Some(list) = self.index.get(&t) { candidates.extend_from_slice(list); }
        }
        candidates.sort_unstable();
        candidates.dedup();
        let (mut block, mut except, mut important) = (None, None, None);
        for &id in candidates.iter().chain(self.untokened.iter()) {
            let f = &self.nets[id as usize];
            if f.kinds & kind == 0 { continue; }
            if f.exception && except.is_some() || !f.exception && block.is_some() && !f.important { continue; }
            if f.third.is_some_and(|t| t != third) { continue; }
            if !f.domains.is_empty() && !f.domains.applies(&page_host) { continue; }
            if !f.matches(&url) { continue; }
            if f.exception { except = Some(id) } else if f.important { important = Some(id); break } else { block = Some(id) }
        }
        match (important, block, except) {
            (Some(b), _, _) => Verdict::Block(b),
            (None, Some(b), Some(_)) => Verdict::Excepted(b),
            (None, Some(b), None) => Verdict::Block(b),
            _ => Verdict::Pass,
        }
    }

    /// What an `@@...$document`/`$elemhide`/`$generichide` exception allows on a page:
    /// (requests all allowed, no hiding at all, no generic hiding).
    pub fn page_exceptions(&self, page: &str) -> (bool, bool, bool) {
        let Some(url) = Target::parse(page) else { return (false, false, false) };
        let host = String::from(url.host());
        let (mut all, mut elem, mut generic) = (false, false, false);
        let mut candidates: Vec<u32> = Vec::new();
        for t in tokens(url.low.as_bytes()) {
            if let Some(list) = self.index.get(&t) { candidates.extend_from_slice(list); }
        }
        candidates.sort_unstable();
        candidates.dedup();
        for &id in candidates.iter().chain(self.untokened.iter()) {
            let f = &self.nets[id as usize];
            if !f.exception || f.kinds & Kind::Document as u16 == 0 { continue; }
            if !f.domains.is_empty() && !f.domains.applies(&host) { continue; }
            if !f.matches(&url) { continue; }
            if f.elemhide { elem = true } else if f.generichide { generic = true } else { all = true; elem = true }
        }
        (all, elem, generic)
    }

    /// The selectors to hide on `page`, given which classes and ids its document has.
    pub fn hidden<'a>(&'a self, page: &str, has_class: &dyn Fn(&str) -> bool, has_id: &dyn Fn(&str) -> bool) -> Vec<&'a str> {
        let Some(url) = Target::parse(page) else { return Vec::new() };
        let host = String::from(url.host());
        let (_, no_hiding, no_generic) = self.page_exceptions(page);
        if no_hiding { return Vec::new(); }
        let mut out: Vec<&str> = Vec::new();
        let mut unhide: Vec<&str> = self.generic_off.iter().map(String::as_str).collect();
        // Site rules: every suffix of the host, and the `name.*` forms.
        let mut ids: Vec<u32> = self.specific_negated.clone();
        let suffix = psl::suffix_str(&host).unwrap_or("");
        let mut h = host.as_str();
        loop {
            if let Some(l) = self.specific_by_domain.get(h) { ids.extend_from_slice(l); }
            if let Some(base) = h.strip_suffix(suffix).and_then(|b| b.strip_suffix('.')).filter(|b| !b.is_empty()) {
                let mut entity = String::from(base);
                entity.push_str(".*");
                if let Some(l) = self.specific_by_domain.get(entity.as_str()) { ids.extend_from_slice(l); }
            }
            match h.find('.') { Some(i) => h = &h[i + 1..], None => break }
        }
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let s = &self.specific[id as usize];
            if !s.domains.applies(&host) { continue; }
            if s.exception { unhide.push(&s.selector) } else { out.push(&s.selector) }
        }
        if !no_generic {
            out.extend(self.generic_always.iter().map(String::as_str));
            for (key, sels) in &self.generic_keyed {
                let present = match key.as_bytes()[0] { b'.' => has_class(&key[1..]), _ => has_id(&key[1..]) };
                if present { out.extend(sels.iter().map(String::as_str)); }
            }
        }
        out.retain(|s| !unhide.contains(s));
        out.sort_unstable();
        out.dedup();
        out
    }

    /// [`Engine::hidden`] as a style sheet: one rule per selector, so a selector the
    /// page's style engine can't read drops only itself.
    pub fn css(&self, page: &str, has_class: &dyn Fn(&str) -> bool, has_id: &dyn Fn(&str) -> bool) -> String {
        let mut css = String::new();
        for s in self.hidden(page, has_class, has_id) {
            css.push_str(s);
            css.push_str("{display:none!important}\n");
        }
        css
    }
}

/// Splits `domains##selector` at its separator, if the line is a cosmetic filter.
fn cosmetic_split(line: &str) -> Option<(&str, &'static str, &str)> {
    let mut from = 0;
    while let Some(i) = line[from..].find('#').map(|i| i + from) {
        for sep in SEPARATORS {
            if line[i..].starts_with(sep) {
                let domains = &line[..i];
                // Network filters may hold `#` in a URL; a domain list can't hold these.
                if domains.contains(['/', '|', '$', '^', '*']) && !domains.ends_with(".*") { return None; }
                return Some((domains, sep, &line[i + sep.len()..]));
            }
        }
        from = i + 1;
    }
    None
}

/// Whether a hiding selector is plain CSS (no uBlock or Adblock Plus extensions).
fn plain_selector(s: &str) -> bool {
    const EXTENDED: &[&str] = &[":has(", ":has-text(", ":-abp-", ":xpath(", ":style(", ":matches-css", ":upward(", ":remove(", ":watch-attr(", ":min-text-length(", ":matches-path(", ":matches-attr(", ":others(", ":if(", ":if-not(", ":contains(", ":nth-ancestor(", ":remove-attr(", ":remove-class(", ":matches-media(", ":matches-prop("];
    !s.contains('{') && !EXTENDED.iter().any(|e| s.contains(e))
}

/// A class (`.x`) or id (`#x`) every element the selector matches needs to exist on
/// the page: the first one outside brackets and parentheses.
fn selector_key(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0i32, 0);
    while i < b.len() {
        match b[i] {
            b'[' | b'(' => depth += 1,
            b']' | b')' => depth -= 1,
            b'\\' => i += 1,
            b'"' | b'\'' => { let q = b[i]; i += 1; while i < b.len() && b[i] != q { i += 1; } }
            c @ (b'.' | b'#') if depth == 0 => {
                let start = i + 1;
                let mut e = start;
                while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'-' || b[e] == b'_' || b[e] >= 0x80) { e += 1; }
                // An escape inside the name: the key would be wrong, so use none.
                if e > start && b.get(e) != Some(&b'\\') {
                    let mut k = String::new();
                    k.push(c as char);
                    k.push_str(&s[start..e]);
                    return Some(k);
                }
                return None;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// An address cut up for matching.
struct Target<'a> {
    raw: &'a str,
    low: String,
    host: core::ops::Range<usize>,
}

impl<'a> Target<'a> {
    fn parse(url: &'a str) -> Option<Target<'a>> {
        let url = url.split('#').next().unwrap_or(url);
        let scheme_end = url.find("://")?;
        let start = scheme_end + 3;
        let after = &url[start..];
        let end = start + after.find(['/', '?']).unwrap_or(after.len());
        let auth = &url[start..end];
        let host_start = start + auth.rfind('@').map_or(0, |i| i + 1);
        let host_end = if url[host_start..end].starts_with('[') { url[host_start..end].find(']').map_or(end, |i| host_start + i + 1) } else { url[host_start..end].find(':').map_or(end, |i| host_start + i) };
        Some(Target { raw: url, low: url.to_ascii_lowercase(), host: host_start..host_end })
    }
    fn host(&self) -> &str { self.low[self.host.clone()].trim_end_matches('.') }
}

impl Net {
    fn matches(&self, t: &Target) -> bool {
        let s = if self.match_case { t.raw.as_bytes() } else { t.low.as_bytes() };
        let p = &self.pattern[..];
        if self.start_anchor { return glob(p, s, 0, self.end_anchor); }
        if self.host_anchor {
            let h = t.host.clone();
            if glob(p, s, h.start, self.end_anchor) { return true; }
            return (h.start + 1..h.end).any(|i| s[i - 1] == b'.' && glob(p, s, i, self.end_anchor));
        }
        match p.first() {
            None => true,
            Some(&c) if c != b'^' && c != b'*' => (0..s.len()).any(|i| s[i] == c && glob(p, s, i, self.end_anchor)),
            _ => (0..=s.len()).any(|i| glob(p, s, i, self.end_anchor)),
        }
    }
}

/// A separator for `^`: anything but a letter, digit or `_ - . %`.
fn separator(c: u8) -> bool { !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'%')) }

/// Whether pattern `p` matches `s` starting at `at` (to the end when `end`).
fn glob(p: &[u8], s: &[u8], at: usize, end: bool) -> bool {
    let (mut pi, mut si) = (0, at);
    // The last `*` seen and where in `s` it was, to back up to.
    let mut star: Option<(usize, usize)> = None;
    loop {
        if pi == p.len() {
            if !end || si == s.len() { return true; }
        } else {
            match p[pi] {
                b'*' => { star = Some((pi, si)); pi += 1; continue; }
                b'^' if si == s.len() => { pi += 1; continue; }
                b'^' if separator(s[si]) => { pi += 1; si += 1; continue; }
                c if si < s.len() && s[si] == c => { pi += 1; si += 1; continue; }
                _ => {}
            }
        }
        match star {
            Some((sp, ss)) if ss < s.len() => { star = Some((sp, ss + 1)); pi = sp + 1; si = ss + 1; }
            _ => return false,
        }
    }
}

fn token_byte(c: u8) -> bool { c.is_ascii_alphanumeric() || c == b'%' }

fn hash(t: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &c in t { h = (h ^ c as u32).wrapping_mul(0x0100_0193); }
    h
}

/// The hashes of an address's tokens (runs of letters and digits).
fn tokens(s: &[u8]) -> impl Iterator<Item = u32> + '_ {
    s.split(|&c| !token_byte(c)).filter(|t| t.len() > 1).map(hash)
}

/// Common tokens are poor keys: nearly every address has them.
const COMMON: &[&[u8]] = &[b"http", b"https", b"www", b"com", b"js", b"html", b"net", b"org", b"img", b"ad", b"ads"];

/// The filter's best index key: its longest whole token (one a matching address must
/// hold exactly), avoiding the commonest. `None` if the pattern has no whole token.
fn best_token(p: &[u8], anchored_start: bool, anchored_end: bool) -> Option<u32> {
    let mut best: Option<(&[u8], i32)> = None;
    let mut i = 0;
    while i < p.len() {
        if !token_byte(p[i]) { i += 1; continue; }
        let start = i;
        while i < p.len() && token_byte(p[i]) { i += 1; }
        let t = &p[start..i];
        let whole_left = if start == 0 { anchored_start } else { p[start - 1] != b'*' };
        let whole_right = if i == p.len() { anchored_end } else { p[i] != b'*' };
        if !whole_left || !whole_right || t.len() < 2 { continue; }
        let score = t.len() as i32 - if COMMON.contains(&t) { 100 } else { 0 };
        if best.is_none_or(|(_, s)| score > s) { best = Some((t, score)); }
    }
    best.map(|(t, _)| hash(t))
}

/// Shields' own small list, for before any list is downloaded: the commonest ad and
/// tracking hosts and ad slots. Written for SexOS (GPL-3.0).
pub const STARTER_LIST: &str = include_str!("starter.txt");

/// Where Kaleidoscope downloads the full lists from (EasyList and EasyPrivacy,
/// GPL-3.0 / CC BY-SA 3.0, the same lists Brave and Adblock Plus start from).
pub const LISTS: &[(&str, &str)] = &[
    ("easylist.txt", "https://raw.githubusercontent.com/easylist/easylist/gh-pages/easylist.txt"),
    ("easyprivacy.txt", "https://raw.githubusercontent.com/easylist/easylist/gh-pages/easyprivacy.txt"),
];
