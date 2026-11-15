// SPDX-License-Identifier: GPL-3.0-only
//! Strings and translations in Fluent's syntax (projectfluent.org), the format Firefox
//! and many Rust programs use, read by Weft's own parser: `fluent-rs` needs `std`, and
//! Weft runs in `no_std` programs too.
//!
//! What it reads: messages and terms (`-brand = SexOS`), attributes (`.tooltip = ...`),
//! multi-line values, comments, placeables with variables (`{ $count }`), terms and
//! other messages (`{ -brand }`, `{ other-message }`), string and number literals, and
//! select expressions on numbers (CLDR plural categories of the language) or strings:
//!
//! ```text
//! files-selected = { $count ->
//!     [0] Nothing selected
//!     [one] One file selected
//!    *[other] { $count } files selected
//! }
//! ```
//!
//! A message missing in the person's language comes from English, then shows its id,
//! so a half-translated program still works.
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

/// A value given to a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg<'a> { Str(&'a str), Num(f64) }

impl<'a> From<&'a str> for Arg<'a> { fn from(s: &'a str) -> Self { Arg::Str(s) } }
impl From<i64> for Arg<'_> { fn from(n: i64) -> Self { Arg::Num(n as f64) } }
impl From<usize> for Arg<'_> { fn from(n: usize) -> Self { Arg::Num(n as f64) } }
impl From<f64> for Arg<'_> { fn from(n: f64) -> Self { Arg::Num(n) } }

#[derive(Clone, Debug, PartialEq)]
enum Part { Text(String), Var(String), Ref(String), Lit(String), Select { selector: String, variants: Vec<(String, Pattern)>, default: usize } }

#[derive(Clone, Debug, PartialEq, Default)]
struct Pattern(Vec<Part>);

#[derive(Clone, Debug, Default)]
struct Message { value: Option<Pattern>, attrs: BTreeMap<String, Pattern> }

/// One language's messages.
#[derive(Clone, Debug, Default)]
pub struct Catalog { messages: BTreeMap<String, Message> }

/// A parse problem: the line and what was wrong. Bad entries are skipped, the rest kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem { pub line: usize, pub what: &'static str }

impl Catalog {
    /// Reads an `.ftl` text; returns the problems it skipped.
    pub fn add_ftl(&mut self, ftl: &str) -> Vec<Problem> {
        let mut problems = Vec::new();
        let lines: Vec<&str> = ftl.lines().collect();
        let mut i = 0;
        let mut current: Option<String> = None;
        while i < lines.len() {
            let line = lines[i];
            let n = i + 1;
            i += 1;
            if line.trim().is_empty() || line.starts_with('#') { if !line.starts_with(' ') { current = if line.trim().is_empty() { current } else { None }; } continue; }
            let indented = line.starts_with(' ') || line.starts_with('\t');
            // Collect continuation lines (indented, not starting a variant or attribute
            // at the message level).
            let mut body = String::new();
            let (key, first) = if indented {
                let t = line.trim_start();
                if let Some(rest) = t.strip_prefix('.') {
                    match rest.split_once('=') { Some((k, v)) => (Some(String::from(".") + k.trim()), v), None => { problems.push(Problem { line: n, what: "attribute without =" }); continue; } }
                } else { problems.push(Problem { line: n, what: "indented line outside a message" }); continue; }
            } else {
                match line.split_once('=') { Some((k, v)) => (Some(k.trim().to_string()), v), None => { problems.push(Problem { line: n, what: "line without =" }); continue; } }
            };
            body.push_str(first.trim_start());
            while i < lines.len() {
                let l = lines[i];
                let t = l.trim_start();
                let cont = l.trim() == "}" || (l.starts_with(' ') || l.starts_with('\t')) && !t.starts_with('.') || l.trim().is_empty() && lines.get(i + 1).map_or(false, |x| x.starts_with(' ') && !x.trim_start().starts_with('.'));
                if !cont { break; }
                body.push('\n');
                body.push_str(t);
                i += 1;
            }
            let body = body.trim_end().to_string();
            let key = key.unwrap();
            let Some(pattern) = parse_pattern(&body) else { problems.push(Problem { line: n, what: "bad placeable" }); continue };
            if let Some(attr) = key.strip_prefix('.') {
                match current.as_ref().and_then(|c| self.messages.get_mut(c)) {
                    Some(m) => { m.attrs.insert(attr.to_string(), pattern); }
                    None => problems.push(Problem { line: n, what: "attribute without a message" }),
                }
            } else {
                if !valid_id(&key) { problems.push(Problem { line: n, what: "bad identifier" }); continue; }
                let value = if body.is_empty() { None } else { Some(pattern) };
                self.messages.insert(key.clone(), Message { value, attrs: BTreeMap::new() });
                current = Some(key);
            }
        }
        problems
    }
    pub fn has(&self, id: &str) -> bool { self.messages.contains_key(id) }
    pub fn len(&self) -> usize { self.messages.len() }
    pub fn is_empty(&self) -> bool { self.messages.is_empty() }
}

fn valid_id(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    s.chars().next().map_or(false, |c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn parse_pattern(s: &str) -> Option<Pattern> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            // Find the matching brace.
            let mut depth = 0;
            let mut j = i;
            let mut in_str = false;
            while j < b.len() {
                match b[j] {
                    b'"' => in_str = !in_str,
                    b'{' if !in_str => depth += 1,
                    b'}' if !in_str => { depth -= 1; if depth == 0 { break; } }
                    _ => {}
                }
                j += 1;
            }
            if j >= b.len() { return None; }
            if !text.is_empty() { parts.push(Part::Text(core::mem::take(&mut text))); }
            parts.push(parse_placeable(s[i + 1..j].trim())?);
            i = j + 1;
        } else {
            let ch = s[i..].chars().next()?;
            text.push(ch);
            i += ch.len_utf8();
        }
    }
    if !text.is_empty() { parts.push(Part::Text(text)); }
    Some(Pattern(parts))
}

fn parse_placeable(s: &str) -> Option<Part> {
    if let Some((sel, rest)) = s.split_once("->") {
        let selector = sel.trim().strip_prefix('$')?.to_string();
        let mut variants = Vec::new();
        let mut default = None;
        let mut cur: Option<(String, String, bool)> = None;
        for line in rest.lines() {
            let t = line.trim();
            if t.is_empty() { if let Some(c) = cur.as_mut() { c.1.push('\n'); } continue; }
            let (star, t2) = match t.strip_prefix('*') { Some(r) => (true, r), None => (false, t) };
            if t2.starts_with('[') {
                if let Some(end) = t2.find(']') {
                    if let Some((k, v, d)) = cur.take() { if d { default = Some(variants.len()); } variants.push((k, parse_pattern(v.trim())?)); }
                    cur = Some((t2[1..end].trim().to_string(), t2[end + 1..].trim_start().to_string(), star));
                    continue;
                }
            }
            if let Some(c) = cur.as_mut() { c.1.push('\n'); c.1.push_str(t); }
        }
        if let Some((k, v, d)) = cur.take() { if d { default = Some(variants.len()); } variants.push((k, parse_pattern(v.trim())?)); }
        return Some(Part::Select { selector, variants, default: default? });
    }
    if let Some(v) = s.strip_prefix('$') { return valid_id(v).then(|| Part::Var(v.to_string())); }
    if let Some(l) = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) { return Some(Part::Lit(l.replace("\\\"", "\""))); }
    if s.parse::<f64>().is_ok() { return Some(Part::Lit(s.to_string())); }
    valid_id(s.split('.').next()?).then(|| Part::Ref(s.to_string()))
}

/// CLDR plural category of an integer `n` in language `lang` (the cardinal rules for
/// the languages SexOS ships first; others use `one`/`other` like English).
pub fn plural(lang: &str, n: f64) -> &'static str {
    let int = (n - libm::trunc(n)) == 0.0 && n >= 0.0;
    let i = n as u64;
    let base = lang.split(['-', '_']).next().unwrap_or(lang);
    match base {
        "ja" | "zh" | "ko" | "th" | "vi" | "id" | "ms" | "tr" => "other",
        "fr" | "pt" => if int && i <= 1 || !int && n < 2.0 { "one" } else { "other" },
        "ru" | "uk" | "be" | "sr" | "hr" | "bs" => {
            if !int { return "other"; }
            let (m10, m100) = (i % 10, i % 100);
            if m10 == 1 && m100 != 11 { "one" } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) { "few" } else { "many" }
        }
        "pl" => {
            if !int { return "other"; }
            let (m10, m100) = (i % 10, i % 100);
            if i == 1 { "one" } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) { "few" } else { "many" }
        }
        "cs" | "sk" => if !int { "many" } else if i == 1 { "one" } else if (2..=4).contains(&i) { "few" } else { "other" },
        "ar" => {
            if !int { return "other"; }
            let m100 = i % 100;
            match i { 0 => "zero", 1 => "one", 2 => "two", _ if (3..=10).contains(&m100) => "few", _ if (11..=99).contains(&m100) => "many", _ => "other" }
        }
        "he" => if int && i == 1 { "one" } else if int && i == 2 { "two" } else { "other" },
        _ => if int && i == 1 { "one" } else { "other" },
    }
}

/// The program's strings: its language's catalogue over English.
#[derive(Clone, Debug)]
pub struct Strings {
    pub language: String,
    local: Catalog,
    english: Catalog,
}

/// Weft's own strings (buttons, menus, the command palette, dialogs).
pub const WEFT_EN: &str = include_str!("../i18n/en.ftl");
pub const WEFT_DE: &str = include_str!("../i18n/de.ftl");

impl Default for Strings {
    fn default() -> Self { Strings::new("en") }
}

impl Strings {
    /// Strings in `language` (a BCP 47 tag such as `de` or `pt-BR`), with Weft's own.
    pub fn new(language: &str) -> Strings {
        let mut s = Strings { language: language.into(), local: Catalog::default(), english: Catalog::default() };
        s.english.add_ftl(WEFT_EN);
        if language.starts_with("de") { s.local.add_ftl(WEFT_DE); }
        s
    }
    /// Adds a catalogue: `language` "en" fills the English fallback, the person's
    /// language fills the local one, any other is ignored.
    pub fn add(&mut self, language: &str, ftl: &str) -> Vec<Problem> {
        let base = |l: &str| String::from(l.split(['-', '_']).next().unwrap_or(l));
        if base(language) == base(&self.language) && base(language) != "en" { self.local.add_ftl(ftl) }
        else if base(language) == "en" { self.english.add_ftl(ftl) }
        else { Vec::new() }
    }
    /// Message `id` (or `id.attribute`), formatted with `args`.
    pub fn format(&self, id: &str, args: &[(&str, Arg)]) -> String {
        let mut out = String::new();
        if !self.write(&mut out, id, args, 0) { out.clear(); out.push_str(id); }
        out
    }
    pub fn get(&self, id: &str) -> String { self.format(id, &[]) }

    fn find(&self, id: &str) -> Option<(&Pattern, &Catalog)> {
        let (msg, attr) = match id.split_once('.') { Some((m, a)) => (m, Some(a)), None => (id, None) };
        for cat in [&self.local, &self.english] {
            if let Some(m) = cat.messages.get(msg) {
                let p = match attr { Some(a) => m.attrs.get(a), None => m.value.as_ref() };
                if let Some(p) = p { return Some((p, cat)); }
            }
        }
        None
    }

    fn write(&self, out: &mut String, id: &str, args: &[(&str, Arg)], depth: u8) -> bool {
        if depth > 8 { return false; }
        let Some((p, _)) = self.find(id) else { return false };
        self.pattern(out, p, args, depth);
        true
    }

    fn pattern(&self, out: &mut String, p: &Pattern, args: &[(&str, Arg)], depth: u8) {
        for part in &p.0 {
            match part {
                Part::Text(t) | Part::Lit(t) => out.push_str(t),
                Part::Var(v) => match args.iter().find(|(k, _)| k == v) {
                    Some((_, Arg::Str(s))) => out.push_str(s),
                    Some((_, Arg::Num(n))) => number(out, *n),
                    None => { out.push('{'); out.push('$'); out.push_str(v); out.push('}'); }
                },
                Part::Ref(r) => { if !self.write(out, r, args, depth + 1) { out.push_str(r); } }
                Part::Select { selector, variants, default } => {
                    let chosen = match args.iter().find(|(k, _)| k == selector) {
                        Some((_, Arg::Num(n))) => {
                            let mut exact = String::new();
                            number(&mut exact, *n);
                            let cat = plural(&self.language, *n);
                            variants.iter().position(|(k, _)| *k == exact).or_else(|| variants.iter().position(|(k, _)| k == cat))
                        }
                        Some((_, Arg::Str(s))) => variants.iter().position(|(k, _)| k == s),
                        None => None,
                    };
                    let (_, pat) = &variants[chosen.unwrap_or(*default)];
                    self.pattern(out, pat, args, depth + 1);
                }
            }
        }
    }
}

fn number(out: &mut String, n: f64) {
    if (n - libm::trunc(n)) == 0.0 && n.abs() < 1e15 { let _ = write!(out, "{}", n as i64); } else { let _ = write!(out, "{}", n); }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FTL: &str = "\
# Comment
-brand = SexOS
hello = Hello, { $name }!
welcome = Welcome to { -brand }
    on two lines.
files = { $count ->
    [0] Nothing selected
    [one] One file selected
   *[other] { $count } files selected
}
save = Save
    .tooltip = Write the file to disk
braces = Literal {\"{\"} brace
";
    #[test]
    fn messages_terms_attributes_and_plurals() {
        let mut s = Strings::new("en");
        assert!(s.add("en", FTL).is_empty());
        assert_eq!(s.format("hello", &[("name", "Andreas".into())]), "Hello, Andreas!");
        assert_eq!(s.get("welcome"), "Welcome to SexOS\non two lines.");
        assert_eq!(s.format("files", &[("count", 0usize.into())]), "Nothing selected");
        assert_eq!(s.format("files", &[("count", 1usize.into())]), "One file selected");
        assert_eq!(s.format("files", &[("count", 7usize.into())]), "7 files selected");
        assert_eq!(s.get("save.tooltip"), "Write the file to disk");
        assert_eq!(s.get("braces"), "Literal { brace");
        assert_eq!(s.get("missing-id"), "missing-id");
    }
    #[test]
    fn german_falls_back_to_english_and_weft_ships_german() {
        let mut s = Strings::new("de");
        s.add("en", "only-en = Only English\n");
        s.add("de", "hello = Hallo\n");
        assert_eq!(s.get("hello"), "Hallo");
        assert_eq!(s.get("only-en"), "Only English");
        assert_eq!(s.get("weft-ok"), "OK");
        assert_eq!(s.get("weft-cancel"), "Abbrechen");
    }
    #[test]
    fn plural_rules() {
        assert_eq!(plural("en", 1.0), "one");
        assert_eq!(plural("fr", 0.0), "one");
        assert_eq!(plural("ru", 21.0), "one");
        assert_eq!(plural("ru", 3.0), "few");
        assert_eq!(plural("ru", 11.0), "many");
        assert_eq!(plural("pl", 22.0), "few");
        assert_eq!(plural("pl", 25.0), "many");
        assert_eq!(plural("ar", 0.0), "zero");
        assert_eq!(plural("ar", 105.0), "few");
        assert_eq!(plural("ja", 1.0), "other");
    }
    #[test]
    fn every_weft_message_has_a_german_translation() {
        let mut en = Catalog::default();
        assert!(en.add_ftl(WEFT_EN).is_empty());
        let mut de = Catalog::default();
        assert!(de.add_ftl(WEFT_DE).is_empty());
        for id in en.messages.keys() { assert!(de.has(id), "{id} lacks German"); }
    }
}
