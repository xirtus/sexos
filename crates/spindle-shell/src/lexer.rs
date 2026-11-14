// SPDX-License-Identifier: GPL-3.0-only
//! Splits a command line into tokens. Never fails: a bad piece becomes an `Error`
//! token, so highlighting and completion work on half-typed lines too.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokKind {
    /// A bare word: command names, flags, numbers, paths, column names.
    Word,
    /// A quoted string; `text` holds it with escapes applied.
    Str,
    /// A backtick word: a bare word that may hold spaces.
    Backtick,
    /// `$name.path`; `text` is everything after the `$`.
    Var,
    Pipe,
    Semi,
    Newline,
    Amp,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Colon,
    Comma,
    /// `==`, `!=`, `<`, `<=`, `>`, `>=`, `=~`, `!~`, `=`, `>>`.
    Op,
    Comment,
    /// Something that is not Spindle syntax; `text` says why.
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokKind,
    pub text: String,
    pub span: Span,
}

const OPS: &[&str] = &[">>", ">=", "<=", "==", "!=", "=~", "!~", ">", "<", "="];

fn op_at(rest: &str) -> Option<&'static str> {
    OPS.iter().copied().find(|op| rest.starts_with(op))
}

fn ends_word(c: char, rest: &str) -> bool {
    if c.is_whitespace() { return true; }
    match c {
        '|' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '&' | '"' | '\'' | '`' => true,
        '<' | '>' | '=' => true,
        '!' => rest[1..].starts_with('=') || rest[1..].starts_with('~'),
        // `a: 1` in a record; `a:b` stays one word (split later in records).
        ':' => rest[1..].chars().next().map(|n| n.is_whitespace() || n == '}' || n == ',').unwrap_or(true),
        _ => false,
    }
}

fn is_ident(c: char) -> bool { c.is_alphanumeric() || c == '_' || c == '-' }

pub fn lex(src: &str) -> Vec<Token> {
    let mut toks = Vec::new();
    let mut i = 0;
    let b = src.as_bytes();
    while i < src.len() {
        let rest = &src[i..];
        let c = rest.chars().next().unwrap();
        let start = i;
        let single = |kind| Token { kind, text: String::from(&src[start..start + c.len_utf8()]), span: Span::new(start, start + c.len_utf8()) };
        match c {
            '\n' => { toks.push(single(TokKind::Newline)); i += 1; }
            c if c.is_whitespace() => { i += c.len_utf8(); }
            '#' => {
                let end = rest.find('\n').map(|n| i + n).unwrap_or(src.len());
                toks.push(Token { kind: TokKind::Comment, text: String::from(&src[i..end]), span: Span::new(i, end) });
                i = end;
            }
            '|' => { toks.push(single(TokKind::Pipe)); i += 1; }
            ';' => { toks.push(single(TokKind::Semi)); i += 1; }
            '(' => { toks.push(single(TokKind::LParen)); i += 1; }
            ')' => { toks.push(single(TokKind::RParen)); i += 1; }
            '[' => { toks.push(single(TokKind::LBracket)); i += 1; }
            ']' => { toks.push(single(TokKind::RBracket)); i += 1; }
            '{' => { toks.push(single(TokKind::LBrace)); i += 1; }
            '}' => { toks.push(single(TokKind::RBrace)); i += 1; }
            ',' => { toks.push(single(TokKind::Comma)); i += 1; }
            ':' => { toks.push(single(TokKind::Colon)); i += 1; }
            '&' => {
                if rest.starts_with("&&") {
                    toks.push(Token { kind: TokKind::Error, text: String::from("`&&` is not Spindle syntax: use `;` to run one command after another, or `and` in a condition"), span: Span::new(i, i + 2) });
                    i += 2;
                } else { toks.push(single(TokKind::Amp)); i += 1; }
            }
            '"' | '\'' => {
                let (tok, next) = lex_string(src, i, c);
                toks.push(tok);
                i = next;
            }
            '`' => match rest[1..].find('`') {
                Some(n) => {
                    toks.push(Token { kind: TokKind::Backtick, text: String::from(&rest[1..1 + n]), span: Span::new(i, i + n + 2) });
                    i += n + 2;
                }
                None => {
                    toks.push(Token { kind: TokKind::Error, text: String::from("this backtick word is not closed"), span: Span::new(i, src.len()) });
                    i = src.len();
                }
            },
            '$' => {
                let mut j = i + 1;
                while j < src.len() {
                    let ch = src[j..].chars().next().unwrap();
                    if is_ident(ch) { j += ch.len_utf8(); continue; }
                    if ch == '.' && src[j + 1..].chars().next().map(is_ident).unwrap_or(false) { j += 1; continue; }
                    break;
                }
                if j == i + 1 {
                    toks.push(Token { kind: TokKind::Error, text: String::from("`$` needs a variable name after it, like `$in` or `$x`"), span: Span::new(i, i + 1) });
                    i += 1;
                } else {
                    toks.push(Token { kind: TokKind::Var, text: String::from(&src[i + 1..j]), span: Span::new(i, j) });
                    i = j;
                }
            }
            _ => {
                if let Some(op) = op_at(rest) {
                    toks.push(Token { kind: TokKind::Op, text: String::from(op), span: Span::new(i, i + op.len()) });
                    i += op.len();
                    continue;
                }
                let mut j = i;
                while j < src.len() {
                    let ch = src[j..].chars().next().unwrap();
                    if j > i && ends_word(ch, &src[j..]) { break; }
                    j += ch.len_utf8();
                }
                let _ = b;
                toks.push(Token { kind: TokKind::Word, text: String::from(&src[i..j]), span: Span::new(i, j) });
                i = j;
            }
        }
    }
    toks
}

fn lex_string(src: &str, start: usize, quote: char) -> (Token, usize) {
    let mut out = String::new();
    let mut chars = src[start + 1..].char_indices();
    while let Some((k, c)) = chars.next() {
        let at = start + 1 + k;
        if c == quote {
            return (Token { kind: TokKind::Str, text: out, span: Span::new(start, at + 1) }, at + 1);
        }
        if c == '\\' && quote == '"' {
            let Some((_, e)) = chars.next() else { break };
            let ch = match e {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '0' => '\0',
                'e' => '\x1b',
                '\\' => '\\',
                '"' => '"',
                '\'' => '\'',
                '$' => '$',
                'u' => {
                    let tail = &src[at + 2..];
                    let parsed = tail.strip_prefix('{').and_then(|t| t.find('}').map(|end| &t[..end]));
                    match parsed.and_then(|hex| u32::from_str_radix(hex, 16).ok()).and_then(char::from_u32) {
                        Some(ch) => {
                            let hex_len = parsed.unwrap().len() + 2;
                            for _ in 0..hex_len { chars.next(); }
                            ch
                        }
                        None => {
                            let end = at + 2 + tail.find('}').map(|n| n + 1).unwrap_or(0);
                            return (Token { kind: TokKind::Error, text: String::from("`\\u` needs a hex code point in braces, like `\\u{1F600}`"), span: Span::new(at, end.max(at + 2)) }, skip_to_close(src, at, quote));
                        }
                    }
                }
                other => {
                    let mut msg = String::from("unknown escape `\\");
                    msg.push(other);
                    msg.push_str("`: use \\n \\t \\r \\0 \\e \\\\ \\\" \\' or \\u{..}");
                    return (Token { kind: TokKind::Error, text: msg, span: Span::new(at, at + 1 + other.len_utf8()) }, skip_to_close(src, at + 1 + other.len_utf8(), quote));
                }
            };
            out.push(ch);
            continue;
        }
        out.push(c);
    }
    (Token { kind: TokKind::Error, text: String::from("this string is not closed"), span: Span::new(start, src.len()) }, src.len())
}

/// Where lexing resumes after a bad escape: past the closing quote.
fn skip_to_close(src: &str, from: usize, quote: char) -> usize {
    let mut esc = false;
    for (k, c) in src[from..].char_indices() {
        if esc { esc = false; continue; }
        if c == '\\' { esc = true; continue; }
        if c == quote { return from + k + 1; }
    }
    src.len()
}
