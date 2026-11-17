//! Minimal, `#![forbid(unsafe_code)]`, `no_std` recursive-descent JSON parser and
//! deterministic serializer.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

const MAX_NESTING_DEPTH: usize = 128;

/// A dynamically-typed JSON value with deterministic object key ordering.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonVal {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<JsonVal>),
    Obj(Vec<(String, JsonVal)>),
}

/// Error returned when parsing invalid JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl JsonError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON error at byte {}: {}", self.offset, self.message)
    }
}

/// Parses a complete JSON document from a UTF-8 string slice.
pub fn parse_json(input: &str) -> Result<JsonVal, JsonError> {
    JsonVal::parse(input)
}

impl JsonVal {
    /// Parses a complete JSON value from `input`.
    pub fn parse(input: &str) -> Result<Self, JsonError> {
        let mut parser = Parser::new(input);
        parser.skip_ws();
        let val = parser.parse_value(0)?;
        parser.skip_ws();
        if !parser.is_eof() {
            return Err(JsonError::new(
                parser.pos,
                "unexpected trailing characters after JSON value",
            ));
        }
        Ok(val)
    }

    /// Serializes this value into a compact, deterministic JSON string.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    /// Appends the compact, deterministic JSON representation of `self` to `out`.
    pub fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Num(n) => write_json_number(*n, out),
            Self::Str(s) => write_json_string(s, out),
            Self::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Self::Obj(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(k, out);
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }

    /// Constructs an empty JSON object.
    pub fn obj() -> Self {
        Self::Obj(Vec::new())
    }

    /// Constructs an empty JSON array.
    pub fn arr() -> Self {
        Self::Arr(Vec::new())
    }

    /// Returns `true` if the value is `JsonVal::Null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Returns the boolean value if this is `JsonVal::Bool`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the number as `f64` if this is `JsonVal::Num`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the number as an exact `i64` if this is an integer `JsonVal::Num` in range.
    pub fn as_i64(&self) -> Option<i64> {
        let n = self.as_f64()?;
        if !n.is_finite() || libm::trunc(n) != n {
            return None;
        }
        if n < -9_007_199_254_740_991.0 || n > 9_007_199_254_740_991.0 {
            return None;
        }
        Some(n as i64)
    }

    /// Returns the number as an exact `u64` if this is a non-negative integer `JsonVal::Num`.
    pub fn as_u64(&self) -> Option<u64> {
        let i = self.as_i64()?;
        if i >= 0 {
            Some(i as u64)
        } else {
            None
        }
    }

    /// Returns the number as an exact `u32` if in range.
    pub fn as_u32(&self) -> Option<u32> {
        u32::try_from(self.as_u64()?).ok()
    }

    /// Returns the number as an exact `u16` if in range.
    pub fn as_u16(&self) -> Option<u16> {
        u16::try_from(self.as_u64()?).ok()
    }

    /// Returns the string slice if this is `JsonVal::Str`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the slice of elements if this is `JsonVal::Arr`.
    pub fn as_arr(&self) -> Option<&[JsonVal]> {
        match self {
            Self::Arr(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Returns the key-value slice if this is `JsonVal::Obj`.
    pub fn as_obj(&self) -> Option<&[(String, JsonVal)]> {
        match self {
            Self::Obj(o) => Some(o.as_slice()),
            _ => None,
        }
    }

    /// Looks up a field by key on a `JsonVal::Obj`.
    pub fn get(&self, key: &str) -> Option<&JsonVal> {
        let entries = self.as_obj()?;
        entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Looks up a string field by key on a `JsonVal::Obj`.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Self::as_str)
    }

    /// Looks up a boolean field by key on a `JsonVal::Obj`.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Self::as_bool)
    }

    /// Looks up an `f64` field by key on a `JsonVal::Obj`.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Self::as_f64)
    }

    /// Looks up an `i64` field by key on a `JsonVal::Obj`.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Self::as_i64)
    }

    /// Looks up a `u64` field by key on a `JsonVal::Obj`.
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(Self::as_u64)
    }

    /// Looks up a `u32` field by key on a `JsonVal::Obj`.
    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(Self::as_u32)
    }

    /// Looks up an array field by key on a `JsonVal::Obj`.
    pub fn get_arr(&self, key: &str) -> Option<&[JsonVal]> {
        self.get(key).and_then(Self::as_arr)
    }

    /// Looks up an object field by key on a `JsonVal::Obj`.
    pub fn get_obj(&self, key: &str) -> Option<&[(String, JsonVal)]> {
        self.get(key).and_then(Self::as_obj)
    }

    /// Inserts or replaces a key in a `JsonVal::Obj`. No-op if `self` is not an object.
    pub fn set(&mut self, key: impl Into<String>, val: JsonVal) {
        if let Self::Obj(entries) = self {
            let key = key.into();
            if let Some((_, existing)) = entries.iter_mut().find(|(k, _)| *k == key) {
                *existing = val;
            } else {
                entries.push((key, val));
            }
        }
    }

    /// Builder-style helper to set a key on a `JsonVal::Obj`.
    pub fn with(mut self, key: impl Into<String>, val: impl Into<JsonVal>) -> Self {
        self.set(key, val.into());
        self
    }

    /// Builder-style helper to set an optional key only when `val` is `Some`.
    pub fn with_opt<T: Into<JsonVal>>(mut self, key: impl Into<String>, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.set(key, v.into());
        }
        self
    }
}

impl fmt::Display for JsonVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json())
    }
}

impl From<bool> for JsonVal {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i32> for JsonVal {
    fn from(v: i32) -> Self {
        Self::Num(f64::from(v))
    }
}

impl From<i64> for JsonVal {
    fn from(v: i64) -> Self {
        Self::Num(v as f64)
    }
}

impl From<u8> for JsonVal {
    fn from(v: u8) -> Self {
        Self::Num(f64::from(v))
    }
}

impl From<u16> for JsonVal {
    fn from(v: u16) -> Self {
        Self::Num(f64::from(v))
    }
}

impl From<u32> for JsonVal {
    fn from(v: u32) -> Self {
        Self::Num(f64::from(v))
    }
}

impl From<u64> for JsonVal {
    fn from(v: u64) -> Self {
        Self::Num(v as f64)
    }
}

impl From<usize> for JsonVal {
    fn from(v: usize) -> Self {
        Self::Num(v as f64)
    }
}

impl From<f64> for JsonVal {
    fn from(v: f64) -> Self {
        Self::Num(v)
    }
}

impl From<&str> for JsonVal {
    fn from(v: &str) -> Self {
        Self::Str(v.to_string())
    }
}

impl From<String> for JsonVal {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}

impl From<Vec<JsonVal>> for JsonVal {
    fn from(v: Vec<JsonVal>) -> Self {
        Self::Arr(v)
    }
}

impl<T: Into<JsonVal>> From<Option<T>> for JsonVal {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(inner) => inner.into(),
            None => Self::Null,
        }
    }
}

/// Writes an escaped JSON string literal (including surrounding quotes) to `out`.
pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let code = c as u32;
                out.push_str("\\u00");
                out.push(hex_nibble((code >> 4) as u8));
                out.push(hex_nibble((code & 0x0f) as u8));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn hex_nibble(v: u8) -> char {
    match v & 0x0f {
        0..=9 => (b'0' + (v & 0x0f)) as char,
        n => (b'a' + (n - 10)) as char,
    }
}

fn write_json_number(n: f64, out: &mut String) {
    if !n.is_finite() {
        out.push_str("null");
        return;
    }
    if n == 0.0 {
        out.push('0');
        return;
    }
    if libm::trunc(n) == n && (-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&n) {
        out.push_str(&format!("{}", n as i64));
        return;
    }
    out.push_str(&format!("{n}"));
}

struct Parser<'a> {
    bytes: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            bytes: src.as_bytes(),
            src,
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
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

    fn parse_value(&mut self, depth: usize) -> Result<JsonVal, JsonError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(JsonError::new(self.pos, "maximum JSON nesting depth exceeded"));
        }
        self.skip_ws();
        let start = self.pos;
        let Some(b) = self.peek() else {
            return Err(JsonError::new(start, "unexpected end of input"));
        };
        match b {
            b'n' => self.parse_literal(b"null", JsonVal::Null),
            b't' => self.parse_literal(b"true", JsonVal::Bool(true)),
            b'f' => self.parse_literal(b"false", JsonVal::Bool(false)),
            b'"' => self.parse_string().map(JsonVal::Str),
            b'[' => self.parse_array(depth + 1),
            b'{' => self.parse_object(depth + 1),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(JsonError::new(start, "unexpected token in JSON")),
        }
    }

    fn parse_literal(&mut self, lit: &[u8], val: JsonVal) -> Result<JsonVal, JsonError> {
        let start = self.pos;
        if self.bytes.get(start..start + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(JsonError::new(start, "invalid literal"))
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonVal, JsonError> {
        self.bump(); // '['
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(JsonVal::Arr(items));
        }
        loop {
            let val = self.parse_value(depth)?;
            items.push(val);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(JsonVal::Arr(items)),
                Some(_) => {
                    return Err(JsonError::new(
                        self.pos - 1,
                        "expected ',' or ']' in JSON array",
                    ));
                }
                None => {
                    return Err(JsonError::new(self.pos, "unterminated JSON array"));
                }
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonVal, JsonError> {
        self.bump(); // '{'
        self.skip_ws();
        let mut entries = Vec::new();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(JsonVal::Obj(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(JsonError::new(
                    self.pos,
                    "expected string key in JSON object",
                ));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bump() != Some(b':') {
                return Err(JsonError::new(
                    self.pos,
                    "expected ':' after JSON object key",
                ));
            }
            let val = self.parse_value(depth)?;
            entries.push((key, val));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(JsonVal::Obj(entries)),
                Some(_) => {
                    return Err(JsonError::new(
                        self.pos - 1,
                        "expected ',' or '}' in JSON object",
                    ));
                }
                None => {
                    return Err(JsonError::new(self.pos, "unterminated JSON object"));
                }
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        let open_pos = self.pos;
        if self.bump() != Some(b'"') {
            return Err(JsonError::new(open_pos, "expected opening '\"'"));
        }
        let mut out = String::new();
        let mut chunk_start = self.pos;
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    if self.pos > chunk_start {
                        out.push_str(&self.src[chunk_start..self.pos]);
                    }
                    self.bump();
                    return Ok(out);
                }
                b'\\' => {
                    if self.pos > chunk_start {
                        out.push_str(&self.src[chunk_start..self.pos]);
                    }
                    self.bump(); // '\'
                    let esc_pos = self.pos;
                    let esc = self
                        .bump()
                        .ok_or_else(|| JsonError::new(esc_pos, "unterminated string escape"))?;
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
                                // High surrogate must be followed by \uXXXX low surrogate.
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err(JsonError::new(
                                        self.pos,
                                        "unpaired high UTF-16 surrogate",
                                    ));
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(JsonError::new(
                                        self.pos,
                                        "invalid low UTF-16 surrogate",
                                    ));
                                }
                                let cp = 0x10000
                                    + (((u32::from(code) - 0xD800) << 10)
                                        | (u32::from(low) - 0xDC00));
                                let ch = char::from_u32(cp).ok_or_else(|| {
                                    JsonError::new(self.pos, "invalid Unicode scalar")
                                })?;
                                out.push(ch);
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                return Err(JsonError::new(
                                    esc_pos,
                                    "unpaired low UTF-16 surrogate",
                                ));
                            } else {
                                let ch = char::from_u32(u32::from(code)).ok_or_else(|| {
                                    JsonError::new(esc_pos, "invalid Unicode scalar")
                                })?;
                                out.push(ch);
                            }
                        }
                        _ => {
                            return Err(JsonError::new(esc_pos, "invalid JSON escape sequence"));
                        }
                    }
                    chunk_start = self.pos;
                }
                0x00..=0x1F => {
                    return Err(JsonError::new(
                        self.pos,
                        "unescaped control character in JSON string",
                    ));
                }
                _ => {
                    // Advance past UTF-8 codepoint boundary safely.
                    let ch = self.src[self.pos..]
                        .chars()
                        .next()
                        .ok_or_else(|| JsonError::new(self.pos, "invalid UTF-8"))?;
                    self.pos += ch.len_utf8();
                }
            }
        }
        Err(JsonError::new(open_pos, "unterminated JSON string"))
    }

    fn parse_hex4(&mut self) -> Result<u16, JsonError> {
        let start = self.pos;
        if self.pos + 4 > self.bytes.len() {
            return Err(JsonError::new(start, "incomplete \\uXXXX escape"));
        }
        let mut acc: u16 = 0;
        for i in 0..4 {
            let b = self.bytes[self.pos + i];
            let digit = match b {
                b'0'..=b'9' => u16::from(b - b'0'),
                b'a'..=b'f' => u16::from(b - b'a' + 10),
                b'A'..=b'F' => u16::from(b - b'A' + 10),
                _ => return Err(JsonError::new(self.pos + i, "invalid hex digit in \\uXXXX")),
            };
            acc = (acc << 4) | digit;
        }
        self.pos += 4;
        Ok(acc)
    }

    fn parse_number(&mut self) -> Result<JsonVal, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        match self.peek() {
            Some(b'0') => {
                self.bump();
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(JsonError::new(self.pos, "leading zero in JSON number"));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.bump();
                }
            }
            _ => return Err(JsonError::new(self.pos, "invalid JSON number")),
        }
        if self.peek() == Some(b'.') {
            self.bump();
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JsonError::new(
                    self.pos,
                    "expected digit after decimal point",
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.bump();
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump();
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JsonError::new(self.pos, "expected digit in exponent"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump();
            }
        }
        let slice = &self.src[start..self.pos];
        let val: f64 = slice
            .parse()
            .map_err(|_| JsonError::new(start, "failed to parse JSON number"))?;
        if !val.is_finite() {
            return Err(JsonError::new(start, "JSON number overflow"));
        }
        Ok(JsonVal::Num(val))
    }
}
