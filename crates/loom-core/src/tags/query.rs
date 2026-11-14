// SPDX-License-Identifier: GPL-3.0-only
//! Smart folders: saved searches over files and their metadata.
//!
//! ```text
//! tag:work rating>=4 type:pdf modified<7d name:*report*
//! (tag:urgent OR color:red) AND NOT type:dir
//! -tag:done size>100M "quarterly plan"
//! ```
//!
//! Terms side by side are ANDed; `OR` joins alternatives; `NOT`, `-` or `!` negates. `NOT`
//! binds tightest, then AND, then OR; parentheses group. Keywords are any case; quote a
//! word (`"or"`) to search for it. Terms:
//!
//! | term | matches |
//! |---|---|
//! | `tag:x` | a tag equal to `x` (any case; wildcards allowed) |
//! | `rating>=4`, `rating:5`, `rating<2` | stars, 0 to 5 |
//! | `color:red`, `label:red` | the colour label (`none` for none) |
//! | `type:pdf`, `ext:pdf` | the extension; `type:` also takes `dir`/`folder`, `file`, `image`, `video`, `audio`, `document`, `archive`, `text`, `code` |
//! | `name:*report*`, `path:docs/*` | wildcards over the name or the whole path, any case; without wildcards, contains |
//! | `size>10M` | bytes, with `K`/`M`/`G`/`T` (powers of 1024) |
//! | `modified<7d`, `modified>1y`, `modified>=2026-01-01`, `modified:2026-03-04` | age in `s`/`min`/`h`/`d`/`w`/`mo`/`y` (`<7d`: within the last 7 days) or a date |
//! | `comment:text` | the comment contains it, any case |
//! | a bare word | the name contains it (or matches it, with wildcards) |

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Color, Meta, Store};
use crate::date::{parse_date, DAY};
use crate::glob::Glob;
use crate::text::{escape, lines, split_unescaped, unescape};

/// A file as a query sees it.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> { pub path: &'a [u8], pub dir: bool, pub size: u64, pub mtime: u64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp { Eq, Ne, Lt, Le, Gt, Ge }

impl Cmp {
    fn test<T: Ord>(self, a: T, b: T) -> bool {
        match self { Cmp::Eq => a == b, Cmp::Ne => a != b, Cmp::Lt => a < b, Cmp::Le => a <= b, Cmp::Gt => a > b, Cmp::Ge => a >= b }
    }
    fn text(self) -> &'static str {
        match self { Cmp::Eq => ":", Cmp::Ne => "!=", Cmp::Lt => "<", Cmp::Le => "<=", Cmp::Gt => ">", Cmp::Ge => ">=" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Dir, File, Image, Video, Audio, Document, Archive, Text, Code }

impl Kind {
    fn parse(s: &[u8]) -> Option<Kind> {
        let s = s.to_ascii_lowercase();
        Some(match &s[..] {
            b"dir" | b"folder" | b"directory" => Kind::Dir,
            b"file" => Kind::File,
            b"image" | b"picture" | b"photo" => Kind::Image,
            b"video" | b"movie" => Kind::Video,
            b"audio" | b"music" | b"sound" => Kind::Audio,
            b"document" | b"doc" => Kind::Document,
            b"archive" => Kind::Archive,
            b"text" => Kind::Text,
            b"code" => Kind::Code,
            _ => return None,
        })
    }
    fn exts(self) -> &'static [&'static str] {
        match self {
            Kind::Image => &["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tif", "tiff", "ico", "qoi", "heic", "avif"],
            Kind::Video => &["mp4", "mkv", "webm", "avi", "mov", "m4v", "mpg", "mpeg", "ogv"],
            Kind::Audio => &["mp3", "ogg", "oga", "opus", "flac", "wav", "m4a", "aac", "mid", "midi"],
            Kind::Document => &["pdf", "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "epub", "djvu"],
            Kind::Archive => &["zip", "tar", "gz", "tgz", "xz", "bz2", "zst", "7z", "rar", "iso"],
            Kind::Text => &["txt", "md", "log", "csv", "ini", "cfg", "conf", "toml", "json", "xml", "yaml", "yml"],
            Kind::Code => &["rs", "c", "h", "cpp", "hpp", "py", "js", "ts", "go", "java", "lua", "sh", "zig", "html", "css", "wat"],
            Kind::Dir | Kind::File => &[],
        }
    }
    fn test(self, c: &Candidate) -> bool {
        match self {
            Kind::Dir => c.dir,
            Kind::File => !c.dir,
            _ => !c.dir && self.exts().iter().any(|e| ext(c.path).eq_ignore_ascii_case(e.as_bytes())),
        }
    }
}

fn ext(path: &[u8]) -> &[u8] { crate::rename::split_ext(crate::base_name(path), false).1 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Tag(Glob),
    Rating(Cmp, u8),
    Color(Color),
    Kind(Kind),
    Ext(Vec<u8>),
    Name(Glob),
    Path(Glob),
    Size(Cmp, u64),
    /// Age in seconds against `now - mtime`.
    Age(Cmp, u64),
    /// A calendar day, by the second it starts at.
    Day(Cmp, u64),
    Comment(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// The empty query: everything.
    All,
    Term(String, Term),
    Not(Box<Query>),
    And(Box<Query>, Box<Query>),
    Or(Box<Query>, Box<Query>),
}

/// Where (byte offset in the query) and why it couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError { pub at: usize, pub why: &'static str }

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok { Open, Close, And, Or, Not, Word(Vec<u8>) }

fn lex(s: &[u8]) -> Result<Vec<(usize, Tok)>, ParseError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c.is_ascii_whitespace() { i += 1; continue; }
        if c == b'(' { out.push((i, Tok::Open)); i += 1; continue; }
        if c == b')' { out.push((i, Tok::Close)); i += 1; continue; }
        let start = i;
        if (c == b'-' || c == b'!') && i + 1 < s.len() && !s[i + 1].is_ascii_whitespace() && s[i + 1] != b')' {
            out.push((i, Tok::Not));
            i += 1;
            continue;
        }
        // A word, with quoted stretches kept whole (quotes dropped).
        let mut w = Vec::new();
        let mut quoted = false;
        while i < s.len() {
            let b = s[i];
            if b == b'"' {
                let end = s[i + 1..].iter().position(|x| *x == b'"').ok_or(ParseError { at: i, why: "unclosed quote" })?;
                w.extend_from_slice(&s[i + 1..i + 1 + end]);
                i += end + 2;
                quoted = true;
                continue;
            }
            if b.is_ascii_whitespace() || b == b'(' || b == b')' { break; }
            w.push(b);
            i += 1;
        }
        let tok = if quoted { Tok::Word(w) } else if w.eq_ignore_ascii_case(b"and") || w == b"&&" { Tok::And }
            else if w.eq_ignore_ascii_case(b"or") || w == b"||" { Tok::Or }
            else if w.eq_ignore_ascii_case(b"not") { Tok::Not }
            else { Tok::Word(w) };
        out.push((start, tok));
    }
    Ok(out)
}

struct Parser<'a> { toks: &'a [(usize, Tok)], k: usize, len: usize }

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> { self.toks.get(self.k).map(|t| &t.1) }
    fn at(&self) -> usize { self.toks.get(self.k).map(|t| t.0).unwrap_or(self.len) }
    fn or(&mut self) -> Result<Query, ParseError> {
        let mut q = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.k += 1;
            q = Query::Or(Box::new(q), Box::new(self.and()?));
        }
        Ok(q)
    }
    fn and(&mut self) -> Result<Query, ParseError> {
        let mut q = self.unary()?;
        loop {
            match self.peek() {
                Some(Tok::And) => { self.k += 1; }
                Some(Tok::Open | Tok::Not | Tok::Word(_)) => {}
                _ => return Ok(q),
            }
            q = Query::And(Box::new(q), Box::new(self.unary()?));
        }
    }
    fn unary(&mut self) -> Result<Query, ParseError> {
        let at = self.at();
        match self.peek().cloned() {
            Some(Tok::Not) => { self.k += 1; Ok(Query::Not(Box::new(self.unary()?))) }
            Some(Tok::Open) => {
                self.k += 1;
                let q = self.or()?;
                if self.peek() != Some(&Tok::Close) { return Err(ParseError { at: self.at(), why: "expected )" }); }
                self.k += 1;
                Ok(q)
            }
            Some(Tok::Word(w)) => { self.k += 1; let t = term(&w, at)?; Ok(Query::Term(String::from_utf8_lossy(&w).into_owned(), t)) }
            Some(Tok::Close) => Err(ParseError { at, why: "unexpected )" }),
            Some(_) => Err(ParseError { at, why: "expected a term" }),
            None => Err(ParseError { at, why: "query ends too soon" }),
        }
    }
}

fn split_op(w: &[u8]) -> Option<(&[u8], Cmp, &[u8])> {
    let i = w.iter().position(|b| matches!(b, b':' | b'=' | b'<' | b'>' | b'!'))?;
    let key = &w[..i];
    if key.is_empty() || !key.iter().all(|b| b.is_ascii_alphabetic()) { return None; }
    let rest = &w[i..];
    let (cmp, n) = if rest.starts_with(b">=") { (Cmp::Ge, 2) } else if rest.starts_with(b"<=") { (Cmp::Le, 2) } else if rest.starts_with(b"!=") { (Cmp::Ne, 2) }
        else if rest.starts_with(b">") { (Cmp::Gt, 1) } else if rest.starts_with(b"<") { (Cmp::Lt, 1) } else if rest.starts_with(b"=") || rest.starts_with(b":") { (Cmp::Eq, 1) }
        else { return None };
    // `rating:>=4` reads like `rating>=4`.
    let mut cmp = cmp;
    let mut value = &rest[n..];
    if cmp == Cmp::Eq && rest[0] == b':' {
        for (p, c) in [(&b">="[..], Cmp::Ge), (b"<=", Cmp::Le), (b"!=", Cmp::Ne), (b">", Cmp::Gt), (b"<", Cmp::Lt), (b"=", Cmp::Eq)] {
            if let Some(v) = value.strip_prefix(p) { cmp = c; value = v; break; }
        }
    }
    Some((key, cmp, value))
}

fn number(s: &[u8]) -> Option<u64> { crate::text::parse_u64(s) }

fn size(s: &[u8]) -> Option<u64> {
    let s = s.strip_suffix(b"B").or_else(|| s.strip_suffix(b"b")).unwrap_or(s);
    let (digits, mul) = match s.last()?.to_ascii_uppercase() {
        b'K' => (&s[..s.len() - 1], 1u64 << 10),
        b'M' => (&s[..s.len() - 1], 1 << 20),
        b'G' => (&s[..s.len() - 1], 1 << 30),
        b'T' => (&s[..s.len() - 1], 1 << 40),
        _ => (s, 1),
    };
    number(digits)?.checked_mul(mul)
}

fn age(s: &[u8]) -> Option<u64> {
    let i = s.iter().position(|b| !b.is_ascii_digit())?;
    let n = number(&s[..i])?;
    let unit: u64 = match &s[i..].to_ascii_lowercase()[..] {
        b"s" => 1, b"min" => 60, b"h" => 3600, b"d" => DAY, b"w" => 7 * DAY, b"mo" => 30 * DAY, b"y" => 365 * DAY,
        _ => return None,
    };
    n.checked_mul(unit)
}

fn term(w: &[u8], at: usize) -> Result<Term, ParseError> {
    let err = |why| ParseError { at, why };
    let Some((key, cmp, value)) = split_op(w) else { return Ok(Term::Name(Glob::new(&contains(w), true))) };
    let key = key.to_ascii_lowercase();
    let eq_only = |c: Cmp| if c == Cmp::Eq { Ok(()) } else { Err(err("this term only takes ':'")) };
    if value.is_empty() { return Err(err("missing value")); }
    Ok(match &key[..] {
        b"tag" => { eq_only(cmp)?; Term::Tag(Glob::new(value, true)) }
        b"rating" | b"stars" => {
            let n = number(value).filter(|n| *n <= 5).ok_or(err("rating is 0 to 5"))?;
            Term::Rating(cmp, n as u8)
        }
        b"color" | b"colour" | b"label" => { eq_only(cmp)?; Term::Color(Color::parse(value).ok_or(err("unknown colour"))?) }
        b"type" | b"kind" => { eq_only(cmp)?; Kind::parse(value).map(Term::Kind).unwrap_or_else(|| Term::Ext(value.to_ascii_lowercase())) }
        b"ext" => { eq_only(cmp)?; Term::Ext(value.strip_prefix(b".").unwrap_or(value).to_ascii_lowercase()) }
        b"name" => { eq_only(cmp)?; Term::Name(Glob::new(&contains(value), true)) }
        b"path" | b"in" => { eq_only(cmp)?; Term::Path(Glob::new(&contains(value), true)) }
        b"size" => Term::Size(cmp, size(value).ok_or(err("size is a number with K, M, G or T"))?),
        b"modified" | b"mtime" | b"date" | b"changed" => {
            if let Some(d) = parse_date(value) { Term::Day(cmp, d) }
            else {
                let secs = age(value).ok_or(err("expected an age like 7d or a date like 2026-01-31"))?;
                Term::Age(if cmp == Cmp::Eq { Cmp::Le } else { cmp }, secs)
            }
        }
        b"comment" | b"note" => { eq_only(cmp)?; Term::Comment(value.to_ascii_lowercase()) }
        _ => return Err(err("unknown term")),
    })
}

/// A plain word means "contains"; with wildcards it is the pattern itself.
fn contains(v: &[u8]) -> Vec<u8> {
    if v.iter().any(|b| matches!(b, b'*' | b'?' | b'[')) { return v.to_vec(); }
    let mut p = alloc::vec![b'*'];
    for &b in v { if b == b'\\' { p.push(b'\\'); } p.push(b); }
    p.push(b'*');
    p
}

pub fn parse(s: &str) -> Result<Query, ParseError> {
    let toks = lex(s.as_bytes())?;
    if toks.is_empty() { return Ok(Query::All); }
    let mut p = Parser { toks: &toks, k: 0, len: s.len() };
    let q = p.or()?;
    if p.k != toks.len() { return Err(ParseError { at: p.at(), why: "unexpected )" }); }
    Ok(q)
}

fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

impl Query {
    pub fn matches(&self, c: &Candidate, meta: Option<&Meta>, now: u64) -> bool {
        match self {
            Query::All => true,
            Query::Not(q) => !q.matches(c, meta, now),
            Query::And(a, b) => a.matches(c, meta, now) && b.matches(c, meta, now),
            Query::Or(a, b) => a.matches(c, meta, now) || b.matches(c, meta, now),
            Query::Term(_, t) => match t {
                Term::Tag(g) => meta.is_some_and(|m| m.tags.iter().any(|x| g.matches(x))),
                Term::Rating(cmp, n) => cmp.test(meta.map_or(0, |m| m.rating), *n),
                Term::Color(col) => meta.map_or(Color::None, |m| m.color) == *col,
                Term::Kind(k) => k.test(c),
                Term::Ext(e) => !c.dir && ext(c.path).eq_ignore_ascii_case(e),
                Term::Name(g) => g.matches(crate::base_name(c.path)),
                Term::Path(g) => g.matches(c.path),
                Term::Size(cmp, n) => cmp.test(c.size, *n),
                Term::Age(cmp, secs) => cmp.test(now.saturating_sub(c.mtime), *secs),
                Term::Day(cmp, d) => match cmp {
                    Cmp::Eq => c.mtime >= *d && c.mtime < d + DAY,
                    Cmp::Ne => c.mtime < *d || c.mtime >= d + DAY,
                    Cmp::Lt => c.mtime < *d,
                    Cmp::Le => c.mtime < d + DAY,
                    Cmp::Gt => c.mtime >= d + DAY,
                    Cmp::Ge => c.mtime >= *d,
                },
                Term::Comment(s) => meta.is_some_and(|m| contains_ci(&m.comment, s)),
            },
        }
    }

    /// The indexes of the candidates that match, looking up each one's metadata.
    pub fn select(&self, cands: &[Candidate], store: &Store, now: u64) -> Vec<usize> {
        (0..cands.len()).filter(|&i| self.matches(&cands[i], store.get(cands[i].path), now)).collect()
    }

    /// The query fully bracketed, to show how it was read.
    pub fn explain(&self) -> String {
        match self {
            Query::All => String::from("*"),
            Query::Term(s, _) => s.clone(),
            Query::Not(q) => alloc::format!("NOT {}", q.explain()),
            Query::And(a, b) => alloc::format!("({} AND {})", a.explain(), b.explain()),
            Query::Or(a, b) => alloc::format!("({} OR {})", a.explain(), b.explain()),
        }
    }
}

impl Cmp { pub fn symbol(self) -> &'static str { self.text() } }

/// Saved searches, shown as folders.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SmartFolders { pub list: Vec<(Vec<u8>, String)> }

const HEADER: &[u8] = b"loom-smart 1";

impl SmartFolders {
    /// Saves (or replaces) a search under `name`, if it parses.
    pub fn save(&mut self, name: &[u8], query: &str) -> Result<(), ParseError> {
        parse(query)?;
        match self.list.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = String::from(query),
            None => self.list.push((name.to_vec(), String::from(query))),
        }
        Ok(())
    }
    pub fn remove(&mut self, name: &[u8]) { self.list.retain(|(n, _)| n != name); }
    pub fn get(&self, name: &[u8]) -> Option<Query> { self.list.iter().find(|(n, _)| n == name).and_then(|(_, q)| parse(q).ok()) }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = HEADER.to_vec();
        out.push(b'\n');
        for (n, q) in &self.list {
            escape(n, &mut out);
            out.push(b'\t');
            escape(q.as_bytes(), &mut out);
            out.push(b'\n');
        }
        out
    }
    pub fn from_bytes(raw: &[u8]) -> Option<SmartFolders> {
        let mut it = lines(raw);
        if it.next()? != HEADER { return None; }
        let list = it.filter_map(|l| {
            let f = split_unescaped(l, b'\t');
            if f.len() != 2 { return None; }
            Some((unescape(f[0]), String::from_utf8(unescape(f[1])).ok()?))
        }).collect();
        Some(SmartFolders { list })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000;

    fn explain(s: &str) -> String { parse(s).unwrap().explain() }

    #[test]
    fn precedence() {
        assert_eq!(explain("a b c"), "((a AND b) AND c)");
        assert_eq!(explain("a OR b c"), "(a OR (b AND c))");
        assert_eq!(explain("a b OR c"), "((a AND b) OR c)");
        assert_eq!(explain("NOT a OR b"), "(NOT a OR b)");
        assert_eq!(explain("NOT (a OR b)"), "NOT (a OR b)");
        assert_eq!(explain("-a !b not c"), "((NOT a AND NOT b) AND NOT c)");
        assert_eq!(explain("a and b or c and not d"), "((a AND b) OR (c AND NOT d))");
        assert_eq!(explain("(a OR b) AND (c OR d)"), "((a OR b) AND (c OR d))");
        assert_eq!(explain("a || b && c"), "(a OR (b AND c))");
        assert_eq!(explain("\"or\" x"), "(or AND x)");
        assert_eq!(explain("tag:work rating>=4 type:pdf modified<7d name:*report*"),
            "((((tag:work AND rating>=4) AND type:pdf) AND modified<7d) AND name:*report*)");
        assert_eq!(parse("   "), Ok(Query::All));
        assert_eq!(parse("rating:>=4").unwrap(), parse("rating>=4").unwrap().clone_with_text("rating:>=4"));
    }

    impl Query {
        fn clone_with_text(&self, s: &str) -> Query {
            match self { Query::Term(_, t) => Query::Term(String::from(s), t.clone()), q => q.clone() }
        }
    }

    #[test]
    fn errors() {
        assert_eq!(parse("(a OR b").unwrap_err().why, "expected )");
        assert_eq!(parse("a)").unwrap_err(), ParseError { at: 1, why: "unexpected )" });
        assert_eq!(parse("a OR").unwrap_err().why, "query ends too soon");
        assert_eq!(parse("rating>9").unwrap_err().why, "rating is 0 to 5");
        assert_eq!(parse("x tag>3").unwrap_err().at, 2);
        assert_eq!(parse("color:mauve").unwrap_err().why, "unknown colour");
        assert_eq!(parse("size>lots").unwrap_err().why, "size is a number with K, M, G or T");
        assert_eq!(parse("modified<7 days").unwrap_err().at, 0);
        assert_eq!(parse("bogus:1").unwrap_err().why, "unknown term");
        assert_eq!(parse("name:\"open").unwrap_err().why, "unclosed quote");
        assert_eq!(parse("tag:").unwrap_err().why, "missing value");
    }

    fn cand(path: &str, dir: bool, size: u64, age: u64) -> Candidate<'_> { Candidate { path: path.as_bytes(), dir, size, mtime: NOW - age } }

    #[test]
    fn evaluates() {
        let mut store = Store::new();
        store.tag(b"w/q3-report.pdf", b"work");
        store.rate(b"w/q3-report.pdf", 4);
        store.tag(b"w/old-report.pdf", b"Work");
        store.rate(b"w/old-report.pdf", 5);
        store.tag(b"w/report.txt", b"work");
        store.rate(b"w/report.txt", 5);
        store.label(b"p/cat.jpg", Color::Red);
        store.comment(b"p/cat.jpg", b"The Cat on the mat");
        let c = [
            cand("w/q3-report.pdf", false, 2_000_000, 2 * DAY),
            cand("w/old-report.pdf", false, 1000, 40 * DAY),
            cand("w/report.txt", false, 10, DAY),
            cand("p/cat.jpg", false, 3 << 20, 400 * DAY),
            cand("p", true, 0, 0),
        ];
        let sel = |q: &str| parse(q).unwrap().select(&c, &store, NOW);
        assert_eq!(sel("tag:work rating>=4 type:pdf modified<7d name:*report*"), [0]);
        assert_eq!(sel("tag:work"), [0, 1, 2]);
        assert_eq!(sel("tag:WO*"), [0, 1, 2]);
        assert_eq!(sel("rating:5"), [1, 2]);
        assert_eq!(sel("rating<1"), [3, 4]);
        assert_eq!(sel("type:pdf OR type:image"), [0, 1, 3]);
        assert_eq!(sel("type:dir"), [4]);
        assert_eq!(sel("NOT type:dir size>1M"), [0, 3]);
        assert_eq!(sel("size>=3M"), [3]);
        assert_eq!(sel("modified>30d"), [1, 3]);
        assert_eq!(sel("modified>1y"), [3]);
        assert_eq!(sel("color:red OR -tag:work -type:folder"), [3]);
        assert_eq!(sel("comment:cat\\ on"), []);
        assert_eq!(sel("comment:\"cat on\""), [3]);
        assert_eq!(sel("report"), [0, 1, 2]);
        assert_eq!(sel("REPORT ext:.txt"), [2]);
        assert_eq!(sel("path:w/* !name:old*"), [0, 2]);
        assert_eq!(sel("(tag:work OR color:red) AND NOT (rating:5 OR ext:jpg)"), [0]);
        assert_eq!(sel(""), [0, 1, 2, 3, 4]);
        // Dates: the day it was modified, before and after.
        let day = crate::date::from_secs(NOW - DAY);
        let d = alloc::format!("{:04}-{:02}-{:02}", day.year, day.month, day.day);
        assert_eq!(sel(&alloc::format!("modified:{d}")), [2]);
        assert_eq!(sel(&alloc::format!("modified<{d} type:file")), [0, 1, 3]);
        assert_eq!(sel(&alloc::format!("modified>{d}")), [4]);
    }

    #[test]
    fn smart_folders_round_trip() {
        let mut s = SmartFolders::default();
        s.save(b"Hot work", "tag:work rating>=4").unwrap();
        s.save(b"Big\tstuff", "size>1G OR (type:video modified<30d)").unwrap();
        assert!(s.save(b"bad", "(oops").is_err());
        s.save(b"Hot work", "tag:work rating:5").unwrap();
        assert_eq!(s.list.len(), 2);
        let back = SmartFolders::from_bytes(&s.to_bytes()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get(b"Hot work").unwrap().explain(), "(tag:work AND rating:5)");
        s.remove(b"Hot work");
        assert_eq!(s.list.len(), 1);
    }
}
