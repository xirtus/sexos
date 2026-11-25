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
}
