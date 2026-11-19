//! A bounded Turtle (RDF 1.1) reader for LV2 bundle data.
//!
//! It reads the whole Turtle grammar LV2 bundles use (prefixes, base, `a`,
//! `;` and `,` lists, blank node property lists, collections, long and short
//! string literals with escapes, language tags, datatypes, numbers, booleans)
//! into a flat triple list. It never fetches anything: relative IRIs stay
//! relative so the caller resolves them inside the bundle.
use alloc::{string::String, vec::Vec};

pub const MAX_INPUT: usize = 1024 * 1024;
pub const MAX_TRIPLES: usize = 65_536;
const MAX_DEPTH: usize = 32;

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Size,
    Syntax(usize),
    Prefix(usize),
    Depth,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Iri(String),
    Blank(u32),
    Literal(String),
    Number(f64),
    Bool(bool),
}

impl Term {
    pub fn iri(&self) -> Option<&str> {
        match self {
            Self::Iri(s) => Some(s),
            _ => None,
        }
    }
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Literal(s) => parse_number(s.as_bytes()),
            Self::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Triple {
    pub subject: Term,
    pub predicate: String,
    pub object: Term,
}

#[derive(Default)]
pub struct Graph {
    pub triples: Vec<Triple>,
    blanks: u32,
}

impl Graph {
    /// Parse one document into this graph. Blank node labels are local to each
    /// document, so `_:a` in two files are different nodes.
    pub fn parse(&mut self, text: &[u8]) -> Result<(), Error> {
        if text.len() > MAX_INPUT {
            return Err(Error::Size);
        }
        let mut p = Parser {
            s: text,
            at: 0,
            prefixes: Vec::new(),
            labels: Vec::new(),
            base: String::new(),
            graph: self,
            depth: 0,
        };
        p.document()
    }
    pub fn objects<'s, 'q>(
        &'s self,
        subject: &'q Term,
        predicate: &'q str,
    ) -> impl Iterator<Item = &'s Term> + 'q
    where
        's: 'q,
    {
        self.triples
            .iter()
            .filter(move |t| &t.subject == subject && t.predicate == predicate)
            .map(|t| &t.object)
    }
    pub fn object(&self, subject: &Term, predicate: &str) -> Option<&Term> {
        self.triples
            .iter()
            .find(|t| &t.subject == subject && t.predicate == predicate)
            .map(|t| &t.object)
    }
    pub fn has(&self, subject: &Term, predicate: &str, object: &str) -> bool {
        self.objects(subject, predicate)
            .any(|o| o.iri() == Some(object))
    }
    /// Subjects that have `rdf:type <class>`, in document order, without repeats.
    pub fn instances(&self, class: &str) -> Vec<Term> {
        let mut out: Vec<Term> = Vec::new();
        for t in &self.triples {
            if t.predicate == RDF_TYPE && t.object.iri() == Some(class) && !out.contains(&t.subject)
            {
                out.push(t.subject.clone());
            }
        }
        out
    }
}

struct Parser<'a, 'g> {
    s: &'a [u8],
    at: usize,
    prefixes: Vec<(String, String)>,
    labels: Vec<(String, u32)>,
    base: String,
    graph: &'g mut Graph,
    depth: usize,
}

fn parse_number(s: &[u8]) -> Option<f64> {
    let t = core::str::from_utf8(s).ok()?.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn pn_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.' || b >= 0x80
}

impl<'a, 'g> Parser<'a, 'g> {
    fn err(&self) -> Error {
        Error::Syntax(self.at)
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }
    fn skip(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'#' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.at += 1;
                }
            } else if b.is_ascii_whitespace() {
                self.at += 1;
            } else {
                break;
            }
        }
    }
    fn eat(&mut self, b: u8) -> bool {
        self.skip();
        if self.peek() == Some(b) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, b: u8) -> Result<(), Error> {
        if self.eat(b) {
            Ok(())
        } else {
            Err(self.err())
        }
    }
    fn keyword(&mut self, word: &[u8], case_insensitive: bool) -> bool {
        self.skip();
        let end = self.at + word.len();
        let Some(got) = self.s.get(self.at..end) else {
            return false;
        };
        let same = if case_insensitive {
            got.eq_ignore_ascii_case(word)
        } else {
            got == word
        };
        // Must not be the start of a longer name.
        if same && !self.s.get(end).is_some_and(|b| pn_char(*b) || *b == b':') {
            self.at = end;
            true
        } else {
            false
        }
    }
    fn document(&mut self) -> Result<(), Error> {
        loop {
            self.skip();
            if self.peek().is_none() {
                return Ok(());
            }
            if self.eat(b'@') {
                if self.keyword(b"prefix", false) {
                    self.prefix_decl()?;
                } else if self.keyword(b"base", false) {
                    self.base = self.iriref()?;
                } else {
                    return Err(self.err());
                }
                self.expect(b'.')?;
            } else if self.keyword(b"PREFIX", true) {
                self.prefix_decl()?;
            } else if self.keyword(b"BASE", true) {
                self.base = self.iriref()?;
            } else {
                self.triples()?;
                self.expect(b'.')?;
            }
        }
    }
    fn prefix_decl(&mut self) -> Result<(), Error> {
        self.skip();
        let start = self.at;
        while self.peek().is_some_and(pn_char) {
            self.at += 1;
        }
        let name = String::from_utf8(self.s[start..self.at].to_vec()).map_err(|_| self.err())?;
        self.expect(b':')?;
        let iri = self.iriref()?;
        self.prefixes.retain(|(n, _)| *n != name);
        self.prefixes.push((name, iri));
        Ok(())
    }
    fn iriref(&mut self) -> Result<String, Error> {
        self.expect(b'<')?;
        let mut out = Vec::new();
        loop {
            let b = self.peek().ok_or(self.err())?;
            self.at += 1;
            match b {
                b'>' => break,
                b'\\' => {
                    let c = self.unicode_escape()?;
                    push_char(&mut out, c);
                }
                b'\n' | b'\r' | b' ' | b'"' | b'<' | b'{' | b'}' | b'|' | b'^' | b'`' => {
                    return Err(self.err())
                }
                _ => out.push(b),
            }
        }
        let iri = String::from_utf8(out).map_err(|_| self.err())?;
        Ok(self.resolve(iri))
    }
    /// Only absolute or bundle-relative names occur in LV2 data; a relative IRI
    /// is kept relative (prefixed by @base if one was declared).
    fn resolve(&self, iri: String) -> String {
        if iri.contains(':') || self.base.is_empty() {
            return iri;
        }
        let mut out = self.base.clone();
        out.push_str(&iri);
        out
    }
    fn unicode_escape(&mut self) -> Result<char, Error> {
        let n = match self.peek() {
            Some(b'u') => 4,
            Some(b'U') => 8,
            _ => return Err(self.err()),
        };
        self.at += 1;
        let hex = self.s.get(self.at..self.at + n).ok_or(self.err())?;
        let text = core::str::from_utf8(hex).map_err(|_| self.err())?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err())?;
        self.at += n;
        char::from_u32(v).ok_or(self.err())
    }
    fn prefixed(&mut self) -> Result<String, Error> {
        self.skip();
        let start = self.at;
        while self.peek().is_some_and(pn_char) {
            self.at += 1;
        }
        let prefix = &self.s[start..self.at];
        if self.peek() != Some(b':') {
            return Err(self.err());
        }
        self.at += 1;
        let local_start = self.at;
        while let Some(b) = self.peek() {
            if pn_char(b) || b == b':' || b == b'%' {
                self.at += 1;
            } else if b == b'\\' && self.s.get(self.at + 1).is_some() {
                self.at += 2;
            } else {
                break;
            }
        }
        // A local name never ends in '.', which is the statement terminator.
        while self.at > local_start && self.s[self.at - 1] == b'.' {
            self.at -= 1;
        }
        let mut local = Vec::new();
        let mut i = local_start;
        while i < self.at {
            if self.s[i] == b'\\' {
                i += 1;
            }
            local.push(self.s[i]);
            i += 1;
        }
        let base = self
            .prefixes
            .iter()
            .find(|(n, _)| n.as_bytes() == prefix)
            .map(|(_, iri)| iri.clone())
            .ok_or(Error::Prefix(start))?;
        let mut iri = base;
        iri.push_str(core::str::from_utf8(&local).map_err(|_| self.err())?);
        Ok(iri)
    }
    fn iri(&mut self) -> Result<String, Error> {
        self.skip();
        if self.peek() == Some(b'<') {
            self.iriref()
        } else {
            self.prefixed()
        }
    }
    fn blank_label(&mut self) -> Result<Term, Error> {
        // Caller saw "_:".
        self.at += 2;
        let start = self.at;
        while self.peek().is_some_and(pn_char) {
            self.at += 1;
        }
        while self.at > start && self.s[self.at - 1] == b'.' {
            self.at -= 1;
        }
        if self.at == start {
            return Err(self.err());
        }
        let name = String::from_utf8(self.s[start..self.at].to_vec()).map_err(|_| self.err())?;
        if let Some((_, id)) = self.labels.iter().find(|(n, _)| *n == name) {
            return Ok(Term::Blank(*id));
        }
        let id = self.fresh();
        self.labels.push((name, id));
        Ok(Term::Blank(id))
    }
    fn fresh(&mut self) -> u32 {
        self.graph.blanks += 1;
        self.graph.blanks
    }
    fn emit(&mut self, subject: &Term, predicate: &str, object: Term) -> Result<(), Error> {
        if self.graph.triples.len() >= MAX_TRIPLES {
            return Err(Error::Size);
        }
        self.graph.triples.push(Triple {
            subject: subject.clone(),
            predicate: String::from(predicate),
            object,
        });
        Ok(())
    }
    fn triples(&mut self) -> Result<(), Error> {
        self.skip();
        if self.peek() == Some(b'[') {
            let subject = self.blank_property_list()?;
            self.skip();
            if self.peek() != Some(b'.') {
                self.predicate_objects(&subject)?;
            }
            return Ok(());
        }
        let subject = self.subject()?;
        self.predicate_objects(&subject)
    }
    fn subject(&mut self) -> Result<Term, Error> {
        self.skip();
        match self.peek() {
            Some(b'_') if self.s.get(self.at + 1) == Some(&b':') => self.blank_label(),
            Some(b'(') => self.collection(),
            _ => Ok(Term::Iri(self.iri()?)),
        }
    }
    fn predicate_objects(&mut self, subject: &Term) -> Result<(), Error> {
        loop {
            let predicate = if self.keyword(b"a", false) {
                String::from(RDF_TYPE)
            } else {
                self.iri()?
            };
            loop {
                let object = self.object()?;
                self.emit(subject, &predicate, object)?;
                if !self.eat(b',') {
                    break;
                }
            }
            if !self.eat(b';') {
                return Ok(());
            }
            while self.eat(b';') {}
            self.skip();
            if matches!(self.peek(), Some(b'.') | Some(b']') | None) {
                return Ok(());
            }
        }
    }
    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::Depth);
        }
        Ok(())
    }
    fn blank_property_list(&mut self) -> Result<Term, Error> {
        self.enter()?;
        self.expect(b'[')?;
        let node = Term::Blank(self.fresh());
        if !self.eat(b']') {
            self.predicate_objects(&node)?;
            self.expect(b']')?;
        }
        self.depth -= 1;
        Ok(node)
    }
    fn collection(&mut self) -> Result<Term, Error> {
        self.enter()?;
        self.expect(b'(')?;
        let nil = Term::Iri(alloc::format!("{RDF}nil"));
        let mut head = nil.clone();
        let mut last: Option<Term> = None;
        while !self.eat(b')') {
            let item = self.object()?;
            let cell = Term::Blank(self.fresh());
            self.emit(&cell, &alloc::format!("{RDF}first"), item)?;
            match last {
                Some(prev) => self.emit(&prev, &alloc::format!("{RDF}rest"), cell.clone())?,
                None => head = cell.clone(),
            }
            last = Some(cell);
        }
        if let Some(prev) = last {
            self.emit(&prev, &alloc::format!("{RDF}rest"), nil)?;
        }
        self.depth -= 1;
        Ok(head)
    }
    fn object(&mut self) -> Result<Term, Error> {
        self.skip();
        match self.peek().ok_or(self.err())? {
            b'[' => self.blank_property_list(),
            b'(' => self.collection(),
            b'_' if self.s.get(self.at + 1) == Some(&b':') => self.blank_label(),
            b'"' | b'\'' => self.literal(),
            b'<' => Ok(Term::Iri(self.iriref()?)),
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number(),
            _ => {
                if self.keyword(b"true", false) {
                    Ok(Term::Bool(true))
                } else if self.keyword(b"false", false) {
                    Ok(Term::Bool(false))
                } else {
                    Ok(Term::Iri(self.prefixed()?))
                }
            }
        }
    }
    fn number(&mut self) -> Result<Term, Error> {
        let start = self.at;
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.at += 1;
        }
        let mut digits = 0;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
            digits += 1;
        }
        // '.' is a decimal point only when a digit follows; otherwise it ends the statement.
        if self.peek() == Some(b'.') && self.s.get(self.at + 1).is_some_and(|b| b.is_ascii_digit())
        {
            self.at += 1;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.at += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(self.err());
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.at += 1;
            }
            let e = self.at;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.at += 1;
            }
            if e == self.at {
                return Err(self.err());
            }
        }
        parse_number(&self.s[start..self.at])
            .map(Term::Number)
            .ok_or(Error::Syntax(start))
    }
    fn literal(&mut self) -> Result<Term, Error> {
        let quote = self.peek().ok_or(self.err())?;
        let long = self.s.get(self.at..self.at + 3) == Some(&[quote; 3][..]);
        self.at += if long { 3 } else { 1 };
        let mut out = Vec::new();
        loop {
            let b = self.peek().ok_or(self.err())?;
            if long {
                if self.s.get(self.at..self.at + 3) == Some(&[quote; 3][..]) {
                    self.at += 3;
                    // Up to two more quotes belong to the content.
                    break;
                }
            } else if b == quote {
                self.at += 1;
                break;
            } else if b == b'\n' || b == b'\r' {
                return Err(self.err());
            }
            self.at += 1;
            if b == b'\\' {
                let e = self.peek().ok_or(self.err())?;
                let c = match e {
                    b't' => '\t',
                    b'b' => '\u{8}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b'f' => '\u{c}',
                    b'"' => '"',
                    b'\'' => '\'',
                    b'\\' => '\\',
                    b'u' | b'U' => {
                        let c = self.unicode_escape()?;
                        push_char(&mut out, c);
                        continue;
                    }
                    _ => return Err(self.err()),
                };
                self.at += 1;
                push_char(&mut out, c);
            } else {
                out.push(b);
            }
        }
        let text = String::from_utf8(out).map_err(|_| self.err())?;
        if self.peek() == Some(b'@') {
            self.at += 1;
            while self
                .peek()
                .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                self.at += 1;
            }
            return Ok(Term::Literal(text));
        }
        if self.s.get(self.at..self.at + 2) == Some(b"^^") {
            self.at += 2;
            let datatype = self.iri()?;
            if let Some(kind) = datatype.strip_prefix(XSD) {
                match kind {
                    "integer" | "decimal" | "double" | "float" | "int" | "long" | "short" => {
                        if let Some(v) = parse_number(text.as_bytes()) {
                            return Ok(Term::Number(v));
                        }
                    }
                    "boolean" => return Ok(Term::Bool(text == "true" || text == "1")),
                    _ => (),
                }
            }
        }
        Ok(Term::Literal(text))
    }
}

fn push_char(out: &mut Vec<u8>, c: char) {
    let mut b = [0; 4];
    out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    const LV2: &str = "http://lv2plug.in/ns/lv2core#";

    #[test]
    fn reads_lv2_plugin_description() {
        let text = br#"
@prefix doap:  <http://usefulinc.com/ns/doap#> .
@prefix lv2:   <http://lv2plug.in/ns/lv2core#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
# comment with "quotes" and <angles>
<http://lv2plug.in/plugins/eg-amp>
	a lv2:Plugin ,
		lv2:AmplifierPlugin ;
	doap:name "Simple Amplifier" , "Einfacher Verstaerker"@de-at ;
	lv2:optionalFeature lv2:hardRTCapable ;
	lv2:port [
		a lv2:InputPort , lv2:ControlPort ;
		lv2:index 0 ;
		lv2:symbol "gain" ;
		lv2:default 0.0 ;
		lv2:minimum -90.0 ;
		lv2:maximum 24.0 ;
		lv2:scalePoint [ rdfs:label "+5" ; rdf:value 5.0 ] , [ rdfs:label "0" ; rdf:value 0.0 ]
	] , [
		a lv2:AudioPort , lv2:InputPort ;
		lv2:index 1 ;
		lv2:symbol "in"
	] .
"#;
        let mut g = Graph::default();
        // rdf: prefix used above is undeclared; real files declare it.
        assert!(matches!(g.parse(text), Err(Error::Prefix(_))));
        let fixed = [b"@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n".as_slice(), text].concat();
        let mut g = Graph::default();
        g.parse(&fixed).unwrap();
        let plugin = Term::Iri("http://lv2plug.in/plugins/eg-amp".into());
        assert_eq!(g.instances(&alloc::format!("{LV2}Plugin")), alloc::vec![plugin.clone()]);
        let ports: Vec<_> = g.objects(&plugin, &alloc::format!("{LV2}port")).cloned().collect();
        assert_eq!(ports.len(), 2);
        let min = g.object(&ports[0], &alloc::format!("{LV2}minimum")).unwrap();
        assert_eq!(min.number(), Some(-90.0));
        assert!(g.has(&ports[1], RDF_TYPE, &alloc::format!("{LV2}AudioPort")));
        assert_eq!(
            g.object(&plugin, "http://usefulinc.com/ns/doap#name"),
            Some(&Term::Literal("Simple Amplifier".into()))
        );
    }
    #[test]
    fn manifest_forms_and_literals() {
        let text = br#"PREFIX lv2: <http://lv2plug.in/ns/lv2core#>
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:x> a lv2:Plugin; lv2:binary <amp.so>; rdfs:seeAlso <amp.ttl>, <presets.ttl>.
<urn:x> lv2:default "0.5"^^xsd:float ; lv2:x """multi
line "quoted" """ ; lv2:y 'single\tq' ; lv2:z true ; lv2:w 1e3 ; lv2:list ( 1 2 ) .
_:b1 lv2:index 3 . _:b1 lv2:symbol "s" .
"#;
        let mut g = Graph::default();
        g.parse(text).unwrap();
        let x = Term::Iri("urn:x".into());
        let lv2 = |s: &str| alloc::format!("{LV2}{s}");
        assert_eq!(g.object(&x, &lv2("binary")).unwrap().iri(), Some("amp.so"));
        assert_eq!(g.objects(&x, "http://www.w3.org/2000/01/rdf-schema#seeAlso").count(), 2);
        assert_eq!(g.object(&x, &lv2("default")), Some(&Term::Number(0.5)));
        assert_eq!(
            g.object(&x, &lv2("x")),
            Some(&Term::Literal("multi\nline \"quoted\" ".into()))
        );
        assert_eq!(g.object(&x, &lv2("y")), Some(&Term::Literal("single\tq".into())));
        assert_eq!(g.object(&x, &lv2("z")), Some(&Term::Bool(true)));
        assert_eq!(g.object(&x, &lv2("w")), Some(&Term::Number(1000.0)));
        let b = g.triples.iter().find(|t| t.predicate == lv2("index")).unwrap().subject.clone();
        assert_eq!(g.object(&b, &lv2("symbol")), Some(&Term::Literal("s".into())));
    }
    #[test]
    fn hostile_inputs_fail_cleanly() {
        for bad in [
            &b"<urn:a> <urn:b> \"unterminated ."[..],
            b"<urn:a> <urn:b> [ <urn:c> 1 .",
            b"@prefix x <urn:> .",
            b"<urn:a> <urn:b> 1",
            b"<urn:a> <urn:b> \"\\q\" .",
        ] {
            let mut g = Graph::default();
            assert!(g.parse(bad).is_err(), "{:?}", core::str::from_utf8(bad));
        }
        let deep = [b"<urn:a> <urn:b> ".as_slice(), &b"[ <urn:p> ".repeat(40), &[b']'; 40], b" ."].concat();
        let mut g = Graph::default();
        assert_eq!(g.parse(&deep), Err(Error::Depth));
    }
}
