// SPDX-License-Identifier: GPL-3.0-only
//! Minimal `#![no_std]` + `alloc` JSON parser and encoder.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonError {
    UnexpectedEof,
    UnexpectedChar(usize),
    InvalidEscape(usize),
    InvalidNumber(usize),
    DepthExceeded,
    TrailingCharacters(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }

    pub fn int(n: i64) -> Self {
        let mut s = String::new();
        let _ = write!(&mut s, "{}", n);
        Self::Number(s)
    }

    pub fn uint(n: u64) -> Self {
        let mut s = String::new();
        let _ = write!(&mut s, "{}", n);
        Self::Number(s)
    }

    pub fn object(entries: Vec<(String, JsonValue)>) -> Self {
        Self::Object(entries)
    }

    pub fn array(items: Vec<JsonValue>) -> Self {
        Self::Array(items)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(s) => s.parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(s) => s.parse::<u64>().ok(),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(entries) => entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_path(&self, path: &[&str]) -> Option<&JsonValue> {
        let mut cur = self;
        for segment in path {
            cur = cur.get(segment)?;
        }
        Some(cur)
    }

    pub fn index(&self, idx: usize) -> Option<&JsonValue> {
        self.as_array().and_then(|arr| arr.get(idx))
    }

    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        encode_into(self, &mut out);
        out
    }
}

pub fn escape_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn encode(val: &JsonValue) -> String {
    val.to_json_string()
}

pub fn encode_into(val: &JsonValue, out: &mut String) {
    match val {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(true) => out.push_str("true"),
        JsonValue::Bool(false) => out.push_str("false"),
        JsonValue::Number(n) => out.push_str(n),
        JsonValue::Str(s) => escape_string(s, out),
        JsonValue::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_into(item, out);
            }
            out.push(']');
        }
        JsonValue::Object(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape_string(k, out);
                out.push(':');
                encode_into(v, out);
            }
            out.push('}');
        }
    }
}

pub fn parse(input: &str) -> Result<JsonValue, JsonError> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        pos: 0,
    };
    let val = parser.parse_value(0)?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(JsonError::TrailingCharacters(parser.pos));
    }
    Ok(val)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::DepthExceeded);
        }
        self.skip_ws();
        let b = self.peek().ok_or(JsonError::UnexpectedEof)?;
        match b {
            b'n' => self.expect_literal(b"null").map(|_| JsonValue::Null),
            b't' => self.expect_literal(b"true").map(|_| JsonValue::Bool(true)),
            b'f' => self
                .expect_literal(b"false")
                .map(|_| JsonValue::Bool(false)),
            b'"' => self.parse_string().map(JsonValue::Str),
            b'[' => self.parse_array(depth + 1),
            b'{' => self.parse_object(depth + 1),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(JsonError::UnexpectedChar(self.pos)),
        }
    }

    fn expect_literal(&mut self, lit: &[u8]) -> Result<(), JsonError> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(JsonError::UnexpectedChar(self.pos))
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, JsonError> {
        let start = self.pos;
        if self.pos + 4 > self.bytes.len() {
            return Err(JsonError::UnexpectedEof);
        }
        let mut val = 0u16;
        for _ in 0..4 {
            let b = self.next().ok_or(JsonError::UnexpectedEof)?;
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u16,
                b'a'..=b'f' => (b - b'a' + 10) as u16,
                b'A'..=b'F' => (b - b'A' + 10) as u16,
                _ => return Err(JsonError::InvalidEscape(start)),
            };
            val = (val << 4) | digit;
        }
        Ok(val)
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        if self.next() != Some(b'"') {
            return Err(JsonError::UnexpectedChar(self.pos));
        }
        let mut out = String::new();
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let esc_pos = self.pos;
                    let esc = self.next().ok_or(JsonError::UnexpectedEof)?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                // High surrogate: must be followed by \uXXXX low surrogate.
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err(JsonError::InvalidEscape(esc_pos));
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(JsonError::InvalidEscape(esc_pos));
                                }
                                let cp = 0x10000
                                    + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                let ch =
                                    char::from_u32(cp).ok_or(JsonError::InvalidEscape(esc_pos))?;
                                out.push(ch);
                            } else {
                                let ch = char::from_u32(code as u32)
                                    .ok_or(JsonError::InvalidEscape(esc_pos))?;
                                out.push(ch);
                            }
                        }
                        _ => return Err(JsonError::InvalidEscape(esc_pos)),
                    }
                }
                c if c < 0x20 => return Err(JsonError::UnexpectedChar(self.pos)),
                _ => {
                    // Copy a contiguous run of valid unescaped UTF-8 bytes.
                    let start = self.pos;
                    while let Some(ch_b) = self.peek() {
                        if ch_b == b'"' || ch_b == b'\\' || ch_b < 0x20 {
                            break;
                        }
                        self.pos += 1;
                    }
                    let slice = core::str::from_utf8(&self.bytes[start..self.pos])
                        .map_err(|_| JsonError::UnexpectedChar(start))?;
                    out.push_str(slice);
                }
            }
        }
        Err(JsonError::UnexpectedEof)
    }

    fn parse_number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(JsonError::InvalidNumber(start)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JsonError::InvalidNumber(start));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JsonError::InvalidNumber(start));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let s = core::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| JsonError::InvalidNumber(start))?;
        Ok(JsonValue::Number(String::from(s)))
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value(depth)?);
            self.skip_ws();
            match self.next() {
                Some(b',') => continue,
                Some(b']') => return Ok(JsonValue::Array(items)),
                Some(_) => return Err(JsonError::UnexpectedChar(self.pos - 1)),
                None => return Err(JsonError::UnexpectedEof),
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        self.pos += 1; // '{'
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(JsonError::UnexpectedChar(self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.next() != Some(b':') {
                return Err(JsonError::UnexpectedChar(self.pos.saturating_sub(1)));
            }
            let value = self.parse_value(depth)?;
            entries.push((key, value));
            self.skip_ws();
            match self.next() {
                Some(b',') => continue,
                Some(b'}') => return Ok(JsonValue::Object(entries)),
                Some(_) => return Err(JsonError::UnexpectedChar(self.pos - 1)),
                None => return Err(JsonError::UnexpectedEof),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn roundtrip_complex_json_and_escapes() {
        let obj = JsonValue::object(vec![
            ("text".into(), JsonValue::str("line1\n\"quoted\"\t\\slash")),
            ("flag".into(), JsonValue::Bool(true)),
            ("count".into(), JsonValue::int(-42)),
            ("none".into(), JsonValue::Null),
            (
                "nested".into(),
                JsonValue::array(vec![
                    JsonValue::uint(1),
                    JsonValue::object(vec![("ok".into(), JsonValue::Bool(false))]),
                ]),
            ),
        ]);
        let encoded = encode(&obj);
        let decoded = parse(&encoded).unwrap();
        assert_eq!(decoded, obj);
        assert_eq!(decoded.get("count").and_then(JsonValue::as_i64), Some(-42));
        assert_eq!(
            decoded
                .get("nested")
                .and_then(|a| a.index(1))
                .and_then(|o| o.get("ok"))
                .and_then(JsonValue::as_bool),
            Some(false)
        );
    }

    #[test]
    fn parses_unicode_and_surrogate_pairs() {
        let parsed = parse(r#"{"emoji":"\uD83D\uDE80","euro":"\u20AC"}"#).unwrap();
        assert_eq!(parsed.get("emoji").and_then(JsonValue::as_str), Some("🚀"));
        assert_eq!(parsed.get("euro").and_then(JsonValue::as_str), Some("€"));
    }

    #[test]
    fn rejects_excessive_nesting() {
        let mut s = String::new();
        for _ in 0..80 {
            s.push('[');
        }
        for _ in 0..80 {
            s.push(']');
        }
        assert_eq!(parse(&s), Err(JsonError::DepthExceeded));
    }
}
