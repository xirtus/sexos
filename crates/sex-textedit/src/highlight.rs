// SPDX-License-Identifier: GPL-3.0-only
//! Syntax colouring without tree-sitter: small hand-written lexers, one line at a time,
//! carrying only what crosses lines (a block comment, a multi-line string, a fenced code
//! block). Colouring a line needs the state at its start, which [`Highlighter`] keeps per
//! line so a long file is lexed once and an edit relexes from its line on.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Plain, Keyword, Type, String, Number, Comment, Punct, Macro, Heading, Emphasis, Code, Link, Key }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang { Plain, Rust, C, Js, Markdown, Toml }

impl Lang {
    /// From a file name's extension.
    pub fn from_name(name: &[u8]) -> Lang {
        let ext = name.rsplit(|b| *b == b'.').next().unwrap_or(b"");
        if !name.contains(&b'.') { return Lang::Plain; }
        match ext.to_ascii_lowercase().as_slice() {
            b"rs" => Lang::Rust,
            b"c" | b"h" | b"cc" | b"cpp" | b"hpp" | b"cxx" => Lang::C,
            b"js" | b"mjs" | b"ts" | b"jsx" | b"tsx" | b"json" => Lang::Js,
            b"md" | b"markdown" => Lang::Markdown,
            b"toml" => Lang::Toml,
            _ => Lang::Plain,
        }
    }
    pub fn name(self) -> &'static str {
        match self { Lang::Plain => "Text", Lang::Rust => "Rust", Lang::C => "C", Lang::Js => "JavaScript", Lang::Markdown => "Markdown", Lang::Toml => "TOML" }
    }
}

/// What carries from one line to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State { block_comment: u8, string: Option<u8>, fence: bool }

/// A coloured run: `start..end` bytes of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span { pub start: usize, pub end: usize, pub kind: Kind }

const RUST_KW: &[&str] = &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield"];
const RUST_TY: &[&str] = &["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result", "Box", "Some", "None", "Ok", "Err"];
const C_KW: &[&str] = &["auto", "break", "case", "class", "const", "continue", "default", "delete", "do", "else", "enum", "extern", "for", "goto", "if", "inline", "namespace", "new", "nullptr", "private", "protected", "public", "register", "return", "sizeof", "static", "struct", "switch", "template", "this", "typedef", "union", "using", "virtual", "volatile", "while", "true", "false", "NULL"];
const C_TY: &[&str] = &["void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "bool", "size_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t", "FILE"];
const JS_KW: &[&str] = &["async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "with", "yield"];
const JS_TY: &[&str] = &["Array", "Object", "String", "Number", "Boolean", "Promise", "Map", "Set", "Math", "JSON", "console", "window", "document"];

fn ident(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 }

/// Colours one line given the state at its start; returns the spans and the state at its end.
pub fn line(lang: Lang, text: &[u8], state: State) -> (Vec<Span>, State) {
    match lang {
        Lang::Plain => (Vec::new(), state),
        Lang::Markdown => markdown(text, state),
        Lang::Toml => toml(text, state),
        Lang::Rust => code(text, state, RUST_KW, RUST_TY, b"//", true),
        Lang::C => code(text, state, C_KW, C_TY, b"//", false),
        Lang::Js => code(text, state, JS_KW, JS_TY, b"//", false),
    }
}

fn code(t: &[u8], mut st: State, kw: &[&str], ty: &[&str], line_comment: &[u8], rust: bool) -> (Vec<Span>, State) {
    let mut out = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<Span>, start: usize, end: usize, kind: Kind| if end > start { out.push(Span { start, end, kind }) };
    // Continuing a block comment or a string from the line before.
    if st.block_comment > 0 {
        let start = 0;
        while i < t.len() && st.block_comment > 0 {
            if t[i..].starts_with(b"*/") { st.block_comment -= 1; i += 2; }
            else if rust && t[i..].starts_with(b"/*") { st.block_comment += 1; i += 2; }
            else { i += 1; }
        }
        push(&mut out, start, i, Kind::Comment);
    } else if let Some(q) = st.string {
        let start = 0;
        let (end, closed) = scan_string(t, 0, q);
        i = end;
        if closed { st.string = None; }
        push(&mut out, start, i, Kind::String);
    }
    while i < t.len() {
        let b = t[i];
        let start = i;
        if t[i..].starts_with(line_comment) { push(&mut out, i, t.len(), Kind::Comment); i = t.len(); }
        else if t[i..].starts_with(b"/*") {
            st.block_comment = 1;
            i += 2;
            while i < t.len() && st.block_comment > 0 {
                if t[i..].starts_with(b"*/") { st.block_comment -= 1; i += 2; }
                else if rust && t[i..].starts_with(b"/*") { st.block_comment += 1; i += 2; }
                else { i += 1; }
            }
            push(&mut out, start, i, Kind::Comment);
        } else if b == b'"' || (b == b'`' && !rust) || (b == b'\'' && !rust) {
            let (end, closed) = scan_string(t, i + 1, b);
            i = end;
            // JS template literals and Rust strings may span lines; C and JS quotes do not.
            if !closed && (b == b'`' || (rust && b == b'"')) { st.string = Some(b); }
            push(&mut out, start, i, Kind::String);
        } else if rust && b == b'\'' {
            // A char literal ('a', '\n') or a lifetime ('a): only the former closes.
            let close = if t.get(i + 1) == Some(&b'\\') { t[i + 2..].iter().position(|c| *c == b'\'').map(|p| i + 3 + p) } else if t.get(i + 2) == Some(&b'\'') { Some(i + 3) } else { None };
            match close {
                Some(end) => { i = end; push(&mut out, start, i, Kind::String); }
                None => { i += 1; while i < t.len() && ident(t[i]) { i += 1; } push(&mut out, start, i, Kind::Type); }
            }
        } else if b.is_ascii_digit() {
            while i < t.len() && (t[i].is_ascii_alphanumeric() || t[i] == b'_' || (t[i] == b'.' && t.get(i + 1).is_some_and(u8::is_ascii_digit))) { i += 1; }
            push(&mut out, start, i, Kind::Number);
        } else if b == b'#' && !rust && t[..i].iter().all(|c| *c == b' ' || *c == b'\t') {
            push(&mut out, i, t.len(), Kind::Macro); i = t.len();
        } else if rust && b == b'#' && t.get(i + 1).is_some_and(|c| *c == b'[' || *c == b'!') {
            let end = t[i..].iter().position(|c| *c == b']').map_or(t.len(), |p| i + p + 1);
            push(&mut out, i, end, Kind::Macro); i = end;
        } else if ident(b) {
            while i < t.len() && ident(t[i]) { i += 1; }
            let word = core::str::from_utf8(&t[start..i]).unwrap_or("");
            let kind = if rust && t.get(i) == Some(&b'!') { i += 1; Kind::Macro }
                else if kw.contains(&word) { Kind::Keyword }
                else if ty.contains(&word) || (rust && word.as_bytes()[0].is_ascii_uppercase()) { Kind::Type }
                else { Kind::Plain };
            if kind != Kind::Plain { push(&mut out, start, i, kind); }
        } else if b"{}()[];,.:=<>+-*/&|!?%^~".contains(&b) {
            push(&mut out, i, i + 1, Kind::Punct); i += 1;
        } else { i += 1; }
    }
    (out, st)
}

/// From `i` (just after an opening quote) to just after the closing one; `false` if the
/// line ended first. Backslash escapes the next byte.
fn scan_string(t: &[u8], mut i: usize, q: u8) -> (usize, bool) {
    while i < t.len() {
        if t[i] == b'\\' { i += 2; continue; }
        if t[i] == q { return (i + 1, true); }
        i += 1;
    }
    (t.len(), false)
}

fn markdown(t: &[u8], mut st: State) -> (Vec<Span>, State) {
    let mut out = Vec::new();
    let trimmed = t.trim_ascii_start();
    if trimmed.starts_with(b"```") || trimmed.starts_with(b"~~~") {
        st.fence = !st.fence;
        out.push(Span { start: 0, end: t.len(), kind: Kind::Code });
        return (out, st);
    }
    if st.fence { if !t.is_empty() { out.push(Span { start: 0, end: t.len(), kind: Kind::Code }); } return (out, st); }
    if trimmed.starts_with(b"#") { out.push(Span { start: 0, end: t.len(), kind: Kind::Heading }); return (out, st); }
    if trimmed.starts_with(b">") { out.push(Span { start: 0, end: t.len(), kind: Kind::Comment }); return (out, st); }
    let lead = t.len() - trimmed.len();
    if trimmed.starts_with(b"- ") || trimmed.starts_with(b"* ") || trimmed.starts_with(b"+ ") { out.push(Span { start: lead, end: lead + 1, kind: Kind::Punct }); }
    let mut i = 0;
    while i < t.len() {
        match t[i] {
            b'`' => { let end = t[i + 1..].iter().position(|c| *c == b'`').map_or(t.len(), |p| i + p + 2); out.push(Span { start: i, end, kind: Kind::Code }); i = end; }
            b'*' | b'_' if t.get(i + 1).is_some_and(|c| !c.is_ascii_whitespace()) && (i == 0 || !ident(t[i - 1])) => {
                let m = t[i];
                let double = t.get(i + 1) == Some(&m);
                let from = i + 1 + double as usize;
                let close = t[from..].iter().position(|c| *c == m).map(|p| from + p + 1 + double as usize);
                match close { Some(end) if end <= t.len() => { out.push(Span { start: i, end, kind: Kind::Emphasis }); i = end; } _ => i += 1 }
            }
            b'[' => {
                // [text](url)
                if let Some(close) = t[i..].iter().position(|c| *c == b']') {
                    let c = i + close;
                    if t.get(c + 1) == Some(&b'(') {
                        let end = t[c..].iter().position(|x| *x == b')').map_or(t.len(), |p| c + p + 1);
                        out.push(Span { start: i, end, kind: Kind::Link });
                        i = end;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    (out, st)
}

fn toml(t: &[u8], mut st: State) -> (Vec<Span>, State) {
    let mut out = Vec::new();
    let mut i = 0;
    if let Some(q) = st.string {
        let close = t.windows(3).position(|w| w == [q; 3]).map(|p| p + 3);
        let end = close.unwrap_or(t.len());
        out.push(Span { start: 0, end, kind: Kind::String });
        if close.is_some() { st.string = None; }
        i = end;
    }
    let trimmed_at = t.len() - t.trim_ascii_start().len();
    if i == 0 && t[trimmed_at..].starts_with(b"[") { out.push(Span { start: trimmed_at, end: t.len(), kind: Kind::Heading }); return (out, st); }
    // A key up to `=`.
    if i == 0 {
        if let Some(eq) = t.iter().position(|b| *b == b'=') {
            if !t[..eq].contains(&b'#') && !t[..eq].contains(&b'"') {
                let key_end = t[..eq].trim_ascii_end().len();
                if key_end > trimmed_at { out.push(Span { start: trimmed_at, end: key_end, kind: Kind::Key }); }
                i = eq + 1;
            }
        }
    }
    while i < t.len() {
        let b = t[i];
        let start = i;
        if b == b'#' { out.push(Span { start: i, end: t.len(), kind: Kind::Comment }); break; }
        if b == b'"' || b == b'\'' {
            if t[i..].starts_with(&[b; 3]) {
                let close = t[i + 3..].windows(3).position(|w| w == [b; 3]).map(|p| i + 3 + p + 3);
                i = close.unwrap_or(t.len());
                if close.is_none() { st.string = Some(b); }
            } else { i = scan_string(t, i + 1, b).0; }
            out.push(Span { start, end: i, kind: Kind::String });
        } else if b.is_ascii_digit() || (b == b'-' && t.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            i += 1;
            while i < t.len() && (t[i].is_ascii_alphanumeric() || b"._-:+".contains(&t[i])) { i += 1; }
            out.push(Span { start, end: i, kind: Kind::Number });
        } else if ident(b) {
            while i < t.len() && ident(t[i]) { i += 1; }
            if matches!(&t[start..i], b"true" | b"false") { out.push(Span { start, end: i, kind: Kind::Keyword }); }
        } else { i += 1; }
    }
    (out, st)
}

/// Keeps each line's starting state, so colouring line N needs no lexing above it twice.
pub struct Highlighter {
    pub lang: Lang,
    /// `starts[n]` is the state at the start of line `n`, for the lines known so far.
    starts: Vec<State>,
}

impl Highlighter {
    pub fn new(lang: Lang) -> Self { Highlighter { lang, starts: alloc::vec![State::default()] } }
    /// The text changed at line `from` or below: forget what follows it.
    pub fn invalidate(&mut self, from: usize) { self.starts.truncate(from.max(0) + 1); }
    /// Spans for lines `first..first + count` of `text`, one list per line.
    pub fn visible(&mut self, text: &[u8], first: usize, count: usize) -> Vec<Vec<Span>> {
        let lines: Vec<&[u8]> = text.split(|b| *b == b'\n').collect();
        let last = (first + count).min(lines.len());
        if self.lang == Lang::Plain || first >= last { return alloc::vec![Vec::new(); last.saturating_sub(first)]; }
        while self.starts.len() <= first {
            let k = self.starts.len() - 1;
            let (_, next) = line(self.lang, lines[k], self.starts[k]);
            self.starts.push(next);
        }
        let mut st = self.starts[first];
        let mut out = Vec::with_capacity(last - first);
        for (k, l) in lines.iter().enumerate().take(last).skip(first) {
            if self.starts.len() == k { self.starts.push(st); }
            let (spans, next) = line(self.lang, l, st);
            out.push(spans);
            st = next;
        }
        out
    }
}
