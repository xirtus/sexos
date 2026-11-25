// SPDX-License-Identifier: GPL-3.0-only
//! Bounded LSP wire framing and Quil's first native diagnostic checks.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const MAX_BODY: usize = 1 << 20;
const MAX_HEADER: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: u32,
    pub character: u32,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: u32,
    pub line: u32,
    pub character: u32,
}

/// A bounded outline for Quil's bundled service. This recognizes declaration
/// heads, not full language syntax; the client still uses LSP documentSymbol.
pub fn symbols(text: &str, language: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line, source) in text.lines().enumerate() {
        if out.len() == 128 { break; }
        let head = source.trim_start();
        let column = source.len() - head.len();
        let choices: &[(&str, u32)] = match language {
            "rust" => &[("pub async fn ", 12), ("async fn ", 12), ("pub fn ", 12), ("fn ", 12), ("pub struct ", 23), ("struct ", 23), ("pub enum ", 10), ("enum ", 10), ("pub trait ", 11), ("trait ", 11), ("impl ", 5), ("pub mod ", 2), ("mod ", 2)],
            "python" => &[("async def ", 12), ("def ", 12), ("class ", 5)],
            "javascript" => &[("export async function ", 12), ("async function ", 12), ("export function ", 12), ("function ", 12), ("export class ", 5), ("class ", 5)],
            _ => &[],
        };
        let Some((prefix, kind)) = choices.iter().find(|(prefix, _)| head.starts_with(prefix)) else { continue };
        let rest = &head[prefix.len()..];
        let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').take(64).collect();
        if name.is_empty() || !name.as_bytes()[0].is_ascii_alphabetic() && name.as_bytes()[0] != b'_' { continue; }
        out.push(Symbol { name, kind: *kind, line: line as u32, character: (source[..column].encode_utf16().count() + prefix.len()) as u32 });
    }
    out
}

/// Resolve one identifier to a unique declaration in the same text snapshot.
/// This bundled helper recognizes declaration heads; it does not resolve imports,
/// scopes, members, or names in other files.
pub fn definition(text: &str, language: &str, cursor: usize) -> Option<Symbol> {
    let bytes = text.as_bytes();
    if cursor > bytes.len() || !text.is_char_boundary(cursor) { return None; }
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = cursor;
    while start > 0 && ident(bytes[start - 1]) { start -= 1; }
    let mut end = cursor;
    while end < bytes.len() && ident(bytes[end]) { end += 1; }
    if start == end { return None; }
    let name = &text[start..end];
    let mut found = None;
    for symbol in symbols(text, language).into_iter().filter(|s| s.name == name) {
        if found.is_some() { return None; }
        found = Some(symbol);
    }
    found
}

/// A small, bounded completion set for the bundled language service. The
/// replacement start is a byte offset in the snapshot supplied by the editor.
pub fn complete(text: &str, language: &str, cursor: usize) -> (usize, Vec<String>) {
    let bytes = text.as_bytes();
    if cursor > bytes.len() || !text.is_char_boundary(cursor) { return (cursor.min(bytes.len()), Vec::new()); }
    let mut start = cursor;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') { start -= 1; }
    let prefix = &text[start..cursor];
    if prefix.is_empty() { return (start, Vec::new()); }
    let keywords: &[&str] = match language {
        "rust" => &["async", "await", "const", "else", "enum", "fn", "for", "if", "impl", "let", "match", "mod", "mut", "pub", "return", "self", "struct", "trait", "use", "where", "while"],
        "python" => &["async", "await", "class", "def", "elif", "else", "except", "for", "from", "if", "import", "in", "None", "pass", "return", "self", "try", "while", "with", "yield"],
        "javascript" => &["async", "await", "class", "const", "else", "export", "for", "from", "function", "if", "import", "let", "return", "this", "var", "while"],
        _ => &[],
    };
    let mut out = Vec::new();
    for &word in keywords {
        if word.starts_with(prefix) && word != prefix { out.push(word.to_string()); }
        if out.len() == 8 { break; }
    }
    (start, out)
}

/// Convert an LSP UTF-16 position into an offset in the open UTF-8 snapshot.
pub fn offset_at_position(text: &str, line: u64, character: u64) -> Option<usize> {
    let mut at = 0usize;
    for _ in 0..line { at = text[at..].find('\n').map(|n| at + n + 1)?; }
    let mut units = 0u64;
    for (delta, c) in text[at..].char_indices() {
        if units == character { return Some(at + delta); }
        if c == '\n' { return None; }
        units += c.len_utf16() as u64;
        if units > character { return None; }
    }
    (units == character).then_some(text.len())
}

pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

/// Takes exactly one JSON-RPC body, leaving later frames buffered. Refuses huge input.
pub fn take_frame(input: &mut Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
    let Some(header_end) = input.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if input.len() > MAX_HEADER {
            Err("LSP header is too long")
        } else {
            Ok(None)
        };
    };
    if header_end > MAX_HEADER {
        return Err("LSP header is too long");
    }
    let header = core::str::from_utf8(&input[..header_end]).map_err(|_| "Invalid LSP header")?;
    let mut length = None;
    for line in header.split("\r\n") {
        if let Some(value) = line.strip_prefix("Content-Length:") {
            if length.is_some() {
                return Err("Duplicate LSP length");
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Invalid LSP length")?,
            );
        }
    }
    let length = length.ok_or("Missing LSP length")?;
    if length > MAX_BODY {
        return Err("LSP message is too large");
    }
    let body_start = header_end + 4;
    let end = body_start + length;
    if input.len() < end {
        return Ok(None);
    }
    let body = input[body_start..end].to_vec();
    input.drain(..end);
    Ok(Some(body))
}

/// Useful, deliberately limited syntax diagnostics for the bundled first server.
/// It checks matching delimiters outside strings/comments and trailing whitespace.
pub fn diagnose(text: &str, language: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut stack: Vec<(u8, u32, u32)> = Vec::new();
    let mut line = 0u32;
    let mut col = 0u32;
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut quote = 0u8;
    let mut line_comment = false;
    let mut block_comment = false;
    while i < bytes.len() && out.len() < 16 {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied().unwrap_or(0);
        if b == b'\n' {
            if i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
                out.push(Diagnostic {
                    line,
                    character: col.saturating_sub(1),
                    message: "Trailing whitespace".to_string(),
                });
            }
            line += 1;
            col = 0;
            line_comment = false;
            i += 1;
            continue;
        }
        if line_comment {
            i += 1;
            col += 1;
            continue;
        }
        if block_comment {
            if b == b'*' && next == b'/' {
                block_comment = false;
                i += 2;
                col += 2;
            } else {
                i += 1;
                col += 1;
            }
            continue;
        }
        if quote != 0 {
            if b == b'\\' {
                i = (i + 2).min(bytes.len());
                col += 2;
                continue;
            }
            if b == quote {
                quote = 0;
            }
            i += 1;
            col += 1;
            continue;
        }
        if b == b'/' && next == b'/' && language != "python" {
            line_comment = true;
            i += 2;
            col += 2;
            continue;
        }
        if b == b'/' && next == b'*' && language != "python" {
            block_comment = true;
            i += 2;
            col += 2;
            continue;
        }
        if b == b'#' && language == "python" {
            line_comment = true;
            i += 1;
            col += 1;
            continue;
        }
        if matches!(b, b'"' | b'\'') {
            quote = b;
            i += 1;
            col += 1;
            continue;
        }
        if matches!(b, b'(' | b'[' | b'{') {
            stack.push((b, line, col));
        }
        if matches!(b, b')' | b']' | b'}') {
            let wanted = match b {
                b')' => b'(',
                b']' => b'[',
                _ => b'{',
            };
            if stack.last().is_some_and(|entry| entry.0 == wanted) {
                stack.pop();
            } else {
                out.push(Diagnostic {
                    line,
                    character: col,
                    message: format!("Unmatched {}", b as char),
                });
            }
        }
        i += 1;
        col += 1;
    }
    for (open, line, character) in stack.into_iter().rev().take(16 - out.len()) {
        out.push(Diagnostic {
            line,
            character,
            message: format!("Unclosed {}", open as char),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_and_adjacent_lsp_frames() {
        let a = frame(br#"{"jsonrpc":"2.0","id":1}"#);
        let b = frame(br#"{"method":"textDocument/publishDiagnostics"}"#);
        let mut input = a[..9].to_vec();
        assert_eq!(take_frame(&mut input), Ok(None));
        input.extend_from_slice(&a[9..]);
        input.extend_from_slice(&b);
        assert_eq!(
            take_frame(&mut input).unwrap(),
            Some(br#"{"jsonrpc":"2.0","id":1}"#.to_vec())
        );
        assert_eq!(
            take_frame(&mut input).unwrap(),
            Some(br#"{"method":"textDocument/publishDiagnostics"}"#.to_vec())
        );
        assert!(input.is_empty());
    }

    #[test]
    fn diagnostics_ignore_strings_and_comments() {
        let issues = diagnose("fn main() { // }\n println!(\"}\");\n", "rust");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Unclosed {");
        assert_eq!(issues[0].line, 0);
        assert_eq!(diagnose("print('ok')\n", "python"), Vec::new());
        assert_eq!(diagnose("let x = [1, 2];\n", "javascript"), Vec::new());
    }

    #[test]
    fn completion_respects_prefix_and_utf16_position() {
        let text = "// 🦀\nfn main() {\n  ret\n}";
        let at = offset_at_position(text, 2, 5).unwrap();
        let (start, items) = complete(text, "rust", at);
        assert_eq!(&text[start..at], "ret");
        assert_eq!(items, ["return"]);
        assert_eq!(offset_at_position(text, 0, 4), None); // inside surrogate pair
        assert!(complete(text, "rust", text.len() + 1).1.is_empty());
    }

    #[test]
    fn outline_bounds_and_recognizes_declaration_heads() {
        assert_eq!(symbols("// fn fake() {}\npub fn real() {}\nstruct Thing;\n", "rust").iter().map(|s| (&*s.name, s.line, s.kind)).collect::<Vec<_>>(), [("real", 1, 12), ("Thing", 2, 23)]);
        assert_eq!(symbols("  async def fetch():\n    pass\nclass Client:\n", "python").iter().map(|s| (&*s.name, s.character)).collect::<Vec<_>>(), [("fetch", 12), ("Client", 6)]);
        assert_eq!(symbols("export function go() {}\n", "javascript")[0].name, "go");
        assert!(symbols("fn x() {}\n".repeat(140).as_str(), "rust").len() == 128);
    }

    #[test]
    fn definition_requires_one_same_file_declaration() {
        let text = "// 🦀\nfn helper() {}\nfn main() { helper(); }\n";
        let at = text.rfind("helper();").unwrap() + 3;
        let symbol = definition(text, "rust", at).unwrap();
        assert_eq!((symbol.name.as_str(), symbol.line), ("helper", 1));
        assert_eq!(definition(text, "rust", text.len() + 1), None);
        assert_eq!(definition(text, "python", at), None);
        let ambiguous = "fn helper() {}\nfn helper() {}\nhelper();";
        assert_eq!(definition(ambiguous, "rust", ambiguous.rfind("helper();").unwrap() + 2), None);
    }
}
