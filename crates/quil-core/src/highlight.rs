// SPDX-License-Identifier: GPL-3.0-only
//! Syntax colouring until tree-sitter arrives (slice O4): a small lexer per language
//! family that marks keywords, types, strings, numbers and comments line by line. A line
//! can start inside a block comment; the caller carries that state between lines.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Plain, Keyword, Type, String, Number, Comment, Punct, Function }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang { Plain, Rust, C, Js, Python, Shell, Toml, Markdown, Go, Zig }

impl Lang {
    pub fn from_path(path: &[u8]) -> Lang {
        let ext = path.rsplit(|b| *b == b'.').next().unwrap_or(b"");
        match ext {
            b"rs" => Lang::Rust, b"c" | b"h" | b"cc" | b"cpp" | b"hpp" | b"cxx" => Lang::C,
            b"js" | b"mjs" | b"ts" | b"tsx" | b"jsx" | b"json" => Lang::Js, b"py" => Lang::Python,
            b"sh" | b"bash" => Lang::Shell, b"toml" | b"ini" | b"cfg" => Lang::Toml, b"md" | b"markdown" => Lang::Markdown,
            b"go" => Lang::Go, b"zig" => Lang::Zig, _ => Lang::Plain,
        }
    }
    pub fn name(self) -> &'static str {
        match self { Lang::Plain => "Plain Text", Lang::Rust => "Rust", Lang::C => "C/C++", Lang::Js => "JavaScript", Lang::Python => "Python",
            Lang::Shell => "Shell", Lang::Toml => "TOML", Lang::Markdown => "Markdown", Lang::Go => "Go", Lang::Zig => "Zig" }
    }
    /// What Toggle Line Comment puts in front of a line.
    pub fn comment(self) -> &'static [u8] {
        match self { Lang::Python | Lang::Shell | Lang::Toml => b"#", Lang::Markdown | Lang::Plain => b"#", _ => b"//" }
    }
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield"],
            Lang::C => &["auto", "break", "case", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "namespace", "new", "nullptr", "private", "public", "return", "short", "signed", "sizeof", "static", "struct", "switch", "template", "typedef", "union", "unsigned", "void", "volatile", "while", "#include", "#define", "#if", "#endif", "#ifdef", "#ifndef"],
            Lang::Js => &["async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "function", "if", "import", "in", "instanceof", "interface", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined", "var", "void", "while", "yield"],
            Lang::Python => &["and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield"],
            Lang::Shell => &["case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in", "local", "return", "then", "until", "while"],
            Lang::Go => &["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "nil", "true", "false"],
            Lang::Zig => &["const", "var", "fn", "pub", "return", "if", "else", "while", "for", "switch", "struct", "enum", "union", "try", "catch", "defer", "errdefer", "comptime", "true", "false", "null", "undefined"],
            Lang::Toml => &["true", "false"],
            _ => &[],
        }
    }
    fn block_comments(self) -> bool { matches!(self, Lang::Rust | Lang::C | Lang::Js | Lang::Go) }
}

/// End of a complete one-line Rust raw (byte) string, including its delimiters.
/// Multiline raw strings need a richer lexer state and are left to that work.
fn rust_raw_string_end(text: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    if text.get(at) == Some(&b'b') { at += 1; }
    if text.get(at) != Some(&b'r') { return None; }
    at += 1;
    let marks = at;
    while text.get(at) == Some(&b'#') && at - marks < 16 { at += 1; }
    if text.get(at) != Some(&b'"') { return None; }
    let hashes = &text[marks..at];
    at += 1;
    while at < text.len() {
        if text[at] == b'"' && text.get(at + 1..at + 1 + hashes.len()) == Some(hashes) {
            return Some(at + 1 + hashes.len());
        }
        at += 1;
    }
    None
}

/// End of a complete one-line Python f-string, including `f`, `fr`, or `rf`.
/// Triple-quoted and multiline strings need cross-line lexer state.
fn python_f_string_end(text: &[u8], start: usize) -> Option<usize> {
    let first = text.get(start)?.to_ascii_lowercase();
    let mut at = start + 1;
    match first {
        b'f' => { if text.get(at).is_some_and(|b| b.to_ascii_lowercase() == b'r') { at += 1; } }
        b'r' => { if text.get(at).is_some_and(|b| b.to_ascii_lowercase() == b'f') { at += 1; } else { return None; } }
        _ => return None,
    }
    let quote = *text.get(at)?;
    if !matches!(quote, b'\'' | b'"') || text.get(at + 1) == Some(&quote) && text.get(at + 2) == Some(&quote) {
        return None;
    }
    at += 1;
    while at < text.len() {
        if text[at] == b'\\' { at = (at + 2).min(text.len()); continue; }
        if text[at] == quote { return Some(at + 1); }
        at += 1;
    }
    None
}

/// Color simple references in a complete, one-line f-string. Escaped braces
/// and expressions needing Python parsing stay within the string span.
fn python_inline_f_string(text: &[u8], start: usize, end: usize, out: &mut Vec<(usize, usize, Kind)>) {
    let quote = (start..end).find(|&at| matches!(text[at], b'\'' | b'"')).unwrap();
    let (mut at, mut segment) = (quote + 1, start);
    while at + 1 < end {
        if text[at] == b'\\' { at = (at + 2).min(end - 1); continue; }
        if text[at..].starts_with(b"{{") { at += 2; continue; }
        if text[at] == b'{' {
            let begin = at + 1;
            let mut after = begin;
            if text.get(after).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') {
                after += 1;
                while text.get(after).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') { after += 1; }
                if after < end - 1 && text[after] == b'}' {
                    if at > segment { out.push((segment, at, Kind::String)); }
                    out.push((at, begin, Kind::Punct));
                    let word = core::str::from_utf8(&text[begin..after]).unwrap_or("");
                    let kind = if Lang::Python.keywords().contains(&word) { Kind::Keyword }
                        else if text[begin].is_ascii_uppercase() { Kind::Type } else { Kind::Plain };
                    out.push((begin, after, kind));
                    out.push((after, after + 1, Kind::Punct));
                    segment = after + 1;
                    at = segment;
                    continue;
                }
            }
        }
        at += 1;
    }
    out.push((segment, end, Kind::String));
}

/// Split only simple `${identifier}` substitutions in a complete one-line JS
/// template. Complex expressions stay string-colored until a parser is used.
fn js_inline_template(text: &[u8], start: usize, out: &mut Vec<(usize, usize, Kind)>) -> Option<usize> {
    let mut end = start + 1;
    while end < text.len() {
        if text[end] == b'\\' { end = (end + 2).min(text.len()); continue; }
        if text[end] == b'`' { break; }
        end += 1;
    }
    if end == text.len() { return None; }
    let (mut i, mut segment) = (start + 1, start);
    while i < end {
        if text[i] == b'\\' { i = (i + 2).min(end); continue; }
        if text[i..end].starts_with(b"${") {
            let begin = i + 2;
            let mut after = begin;
            if text.get(after).is_some_and(|b| b.is_ascii_alphabetic() || matches!(*b, b'_' | b'$')) {
                after += 1;
                while text.get(after).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'$')) { after += 1; }
                if after < end && text[after] == b'}' {
                    if i > segment { out.push((segment, i, Kind::String)); }
                    out.push((i, begin, Kind::Punct));
                    let word = core::str::from_utf8(&text[begin..after]).unwrap_or("");
                    let kind = if Lang::Js.keywords().contains(&word) { Kind::Keyword }
                        else if text[begin].is_ascii_uppercase() { Kind::Type } else { Kind::Plain };
                    out.push((begin, after, kind));
                    out.push((after, after + 1, Kind::Punct));
                    segment = after + 1;
                    i = segment;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push((segment, end + 1, Kind::String));
    Some(end + 1)
}

/// Colours one line. `in_comment` says whether it starts inside a block comment; the
/// return value says whether the next line does.
pub fn line(lang: Lang, text: &[u8], in_comment: bool, out: &mut Vec<(usize, usize, Kind)>) -> bool {
    out.clear();
    let n = text.len();
    let mut i = 0;
    let mut in_c = in_comment;
    let kws = lang.keywords();
    let push = |out: &mut Vec<(usize, usize, Kind)>, a: usize, b: usize, k: Kind| if b > a { out.push((a, b, k)) };
    if lang == Lang::Markdown {
        let k = if text.starts_with(b"#") { Kind::Keyword } else if text.starts_with(b"```") || text.starts_with(b"    ") { Kind::String } else if text.starts_with(b"> ") { Kind::Comment } else { Kind::Plain };
        push(out, 0, n, k);
        return false;
    }
    while i < n {
        if in_c {
            let end = text[i..].windows(2).position(|w| w == b"*/").map(|p| i + p + 2);
            match end { Some(e) => { push(out, i, e, Kind::Comment); i = e; in_c = false; } None => { push(out, i, n, Kind::Comment); return true; } }
            continue;
        }
        let c = text[i];
        if lang == Lang::Rust {
            if let Some(end) = rust_raw_string_end(text, i) {
                push(out, i, end, Kind::String);
                i = end;
                continue;
            }
        }
        if lang == Lang::Python {
            if let Some(end) = python_f_string_end(text, i) {
                python_inline_f_string(text, i, end, out);
                i = end;
                continue;
            }
        }
        if lang == Lang::Js && c == b'`' {
            if let Some(end) = js_inline_template(text, i, out) { i = end; continue; }
        }
        let line_comment = match lang {
            Lang::Python | Lang::Shell | Lang::Toml => c == b'#',
            Lang::Plain => false,
            _ => text[i..].starts_with(b"//"),
        };
        if line_comment { push(out, i, n, Kind::Comment); return false; }
        if lang.block_comments() && text[i..].starts_with(b"/*") { let s = i; i += 2; let end = text[i..].windows(2).position(|w| w == b"*/").map(|p| i + p + 2); match end { Some(e) => { push(out, s, e, Kind::Comment); i = e; in_c = false; } None => { push(out, s, n, Kind::Comment); return true; } } continue; }
        if lang != Lang::Plain && (c == b'"' || c == b'\'' || c == b'`') {
            // A Rust lifetime ('a) is not a string.
            if lang == Lang::Rust && c == b'\'' && text.get(i + 2).is_some_and(|b| *b != b'\'') && text.get(i + 1).is_some_and(|b| b.is_ascii_alphabetic()) {
                let s = i; i += 1; while i < n && (text[i].is_ascii_alphanumeric() || text[i] == b'_') { i += 1; } push(out, s, i, Kind::Type); continue;
            }
            let s = i; i += 1;
            while i < n && text[i] != c { if text[i] == b'\\' { i += 1; } i += 1; }
            i = (i + 1).min(n);
            push(out, s, i, Kind::String);
            continue;
        }
        if c.is_ascii_digit() {
            let s = i;
            while i < n && (text[i].is_ascii_alphanumeric() || text[i] == b'_' || text[i] == b'.') { i += 1; }
            push(out, s, i, Kind::Number);
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'#' || c >= 0x80 {
            let s = i;
            i += 1;
            while i < n && (text[i].is_ascii_alphanumeric() || text[i] == b'_' || text[i] >= 0x80) { i += 1; }
            if lang == Lang::Rust && text.get(i) == Some(&b'!') { i += 1; push(out, s, i, Kind::Function); continue; }
            let word = core::str::from_utf8(&text[s..i]).unwrap_or("");
            let kind = if kws.contains(&word) { Kind::Keyword }
                else if lang != Lang::Plain && word.as_bytes()[0].is_ascii_uppercase() { Kind::Type }
                else if lang != Lang::Plain && text.get(i) == Some(&b'(') { Kind::Function }
                else { Kind::Plain };
            push(out, s, i, kind);
            continue;
        }
        if lang != Lang::Plain && c.is_ascii_punctuation() { push(out, i, i + 1, Kind::Punct); }
        i += 1;
    }
    in_c
}
