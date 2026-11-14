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
