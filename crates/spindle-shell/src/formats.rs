// SPDX-License-Identifier: GPL-3.0-only
//! JSON, CSV/TSV and TOML, written by hand so the crate needs nothing but `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::units;
use crate::value::{Record, Value};

// ---------------------------------------------------------------- JSON

pub fn from_json(text: &str) -> Result<Value, String> {
    let mut p = Json { s: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() { return Err(p.err("extra text after the JSON value")); }
    Ok(v)
}

struct Json<'a> {
    s: &'a [u8],
    i: usize,
}

impl Json<'_> {
    fn err(&self, what: &str) -> String {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|&&c| c == b'\n').count() + 1;
        format!("{} (JSON line {})", what, line)
    }
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') { self.i += 1; }
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.s.get(self.i) == Some(&c) { self.i += 1; true } else { false }
    }
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 { return Err(self.err("JSON nested too deep")); }
        match self.s.get(self.i) {
            None => Err(self.err("JSON ended early")),
            Some(b'{') => {
                self.i += 1;
                let mut rec = Record::new();
                self.ws();
                if self.eat(b'}') { return Ok(Value::Record(rec)); }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') { return Err(self.err("expected a key in quotes")); }
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b':') { return Err(self.err("expected `:` after a key")); }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    rec.insert(key, v);
                    self.ws();
                    if self.eat(b',') { continue; }
                    if self.eat(b'}') { return Ok(Value::Record(rec)); }
                    return Err(self.err("expected `,` or `}`"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b']') { return Ok(Value::List(items)); }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b',') { continue; }
                    if self.eat(b']') { return Ok(Value::List(items)); }
                    return Err(self.err("expected `,` or `]`"));
                }
            }
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') if self.s[self.i..].starts_with(b"true") => { self.i += 4; Ok(Value::Bool(true)) }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => { self.i += 5; Ok(Value::Bool(false)) }
            Some(b'n') if self.s[self.i..].starts_with(b"null") => { self.i += 4; Ok(Value::Nothing) }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                self.i += 1;
                let mut float = false;
                while let Some(&c) = self.s.get(self.i) {
                    match c {
                        b'0'..=b'9' => {}
                        b'.' | b'e' | b'E' | b'+' | b'-' => float = true,
                        _ => break,
                    }
                    self.i += 1;
                }
                let t = core::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                if !float { if let Ok(n) = t.parse::<i64>() { return Ok(Value::Int(n)); } }
                t.parse::<f64>().map(Value::Float).map_err(|_| self.err("bad number"))
            }
            Some(_) => Err(self.err("unexpected character")),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return Err(self.err("string not closed")) };
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("string is not UTF-8")),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return Err(self.err("string not closed")) };
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n', b't' => '\t', b'r' => '\r', b'b' => '\u{8}', b'f' => '\u{c}',
                        b'/' => '/', b'\\' => '\\', b'"' => '"',
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("bad escape in string")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let t = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let v = u32::from_str_radix(core::str::from_utf8(t).unwrap_or("x"), 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }
}

/// JSON text; `indent` 0 writes one line.
pub fn to_json(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_json(v, indent, 0, &mut out);
    out
}

fn json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_json(v: &Value, indent: usize, level: usize, out: &mut String) {
    let nl = |out: &mut String, level: usize| {
        if indent > 0 {
            out.push('\n');
            for _ in 0..indent * level { out.push(' '); }
        }
    };
    match v {
        Value::Nothing => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) | Value::Filesize(i) | Value::Duration(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&if f.is_finite() { units::format_float(*f) } else { String::from("null") }),
        Value::Date(d) => json_string(&units::format_date(*d), out),
        Value::String(s) => json_string(s, out),
        Value::Binary(b) => write_json(&Value::List(b.iter().map(|x| Value::Int(*x as i64)).collect()), indent, level, out),
        Value::Error(e) => json_string(&e.message, out),
        Value::List(items) => {
            if items.is_empty() { out.push_str("[]"); return; }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 { out.push(','); }
                nl(out, level + 1);
                write_json(item, indent, level + 1, out);
            }
            nl(out, level);
            out.push(']');
        }
        Value::Record(r) => {
            if r.is_empty() { out.push_str("{}"); return; }
            out.push('{');
            for (i, (k, val)) in r.iter().enumerate() {
                if i > 0 { out.push(','); }
                nl(out, level + 1);
                json_string(k, out);
                out.push(':');
                if indent > 0 { out.push(' '); }
                write_json(val, indent, level + 1, out);
            }
            nl(out, level);
            out.push('}');
        }
    }
}

// ---------------------------------------------------------------- CSV / TSV

/// A cell's text as a value: integers and floats become numbers.
fn infer(cell: &str) -> Value {
    let t = cell.trim();
    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit() || c == b'-' || c == b'+') {
        if let Ok(i) = t.parse::<i64>() { return Value::Int(i); }
    }
    if !t.is_empty() && t.bytes().any(|c| c.is_ascii_digit()) && t.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E')) {
        if let Ok(f) = t.parse::<f64>() { return Value::Float(f); }
    }
    match t {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::String(cell.to_string()),
    }
}

fn csv_rows(text: &str, sep: char) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') { chars.next(); cell.push('"'); } else { quoted = false; }
            } else { cell.push(c); }
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            c if c == sep => row.push(core::mem::take(&mut cell)),
            '\r' => {}
            '\n' => {
                row.push(core::mem::take(&mut cell));
                rows.push(core::mem::take(&mut row));
                any = false;
            }
            c => cell.push(c),
        }
    }
    if quoted { return Err(String::from("a quoted cell is not closed")); }
    if any { row.push(cell); rows.push(row); }
    Ok(rows)
}

pub fn from_csv(text: &str, sep: char, headers: bool) -> Result<Value, String> {
    let mut rows = csv_rows(text, sep)?.into_iter().filter(|r| !(r.len() == 1 && r[0].is_empty()));
    let header: Vec<String> = if headers {
        match rows.next() { Some(h) => h.into_iter().map(|s| s.trim().to_string()).collect(), None => return Ok(Value::List(Vec::new())) }
    } else { Vec::new() };
    let mut out = Vec::new();
    for r in rows {
        let mut rec = Record::new();
        for (i, cell) in r.iter().enumerate() {
            let name = header.get(i).cloned().unwrap_or_else(|| format!("column{}", i));
            rec.insert(name, infer(cell));
        }
        for name in header.iter().skip(r.len()) { rec.insert(name.clone(), Value::Nothing); }
        out.push(Value::Record(rec));
    }
    Ok(Value::List(out))
}

fn csv_cell(v: &Value, sep: char) -> String {
    let t = v.to_abbreviated();
    if t.contains(sep) || t.contains('"') || t.contains('\n') {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else { t }
}

pub fn to_csv(v: &Value, sep: char) -> Result<String, String> {
    let rows: Vec<Value> = match v {
        Value::List(items) => items.clone(),
        Value::Record(_) => alloc::vec![v.clone()],
        other => return Err(format!("can't write a {} as CSV; it needs a table or a record", other.type_name())),
    };
    let cols = Value::List(rows.clone()).columns();
    let mut out = String::new();
    let sep_s = sep.to_string();
    if cols.is_empty() {
        for r in &rows { out.push_str(&csv_cell(r, sep)); out.push('\n'); }
        return Ok(out);
    }
    out.push_str(&cols.iter().map(|c| csv_cell(&Value::String(c.clone()), sep)).collect::<Vec<_>>().join(&sep_s));
    out.push('\n');
    for r in &rows {
        let Value::Record(rec) = r else { return Err(String::from("every row needs to be a record")) };
        let cells: Vec<String> = cols.iter().map(|c| rec.get(c).map(|v| csv_cell(v, sep)).unwrap_or_default()).collect();
        out.push_str(&cells.join(&sep_s));
        out.push('\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------- TOML

/// TOML: tables, arrays of tables, dotted keys, strings, numbers, booleans, arrays,
/// inline tables, dates (as dates when they parse, otherwise strings).
pub fn from_toml(text: &str) -> Result<Value, String> {
    let mut root = Value::Record(Record::new());
    let mut current: Vec<String> = Vec::new();
    let mut p = Toml { s: text.as_bytes(), i: 0, line: 1 };
    loop {
        p.ws_lines();
        if p.i >= p.s.len() { break; }
        if p.peek() == Some(b'[') {
            let array = p.s[p.i..].starts_with(b"[[");
            p.i += if array { 2 } else { 1 };
            p.ws();
            let path = p.key_path()?;
            p.ws();
            let close: &[u8] = if array { b"]]" } else { b"]" };
            if !p.s[p.i..].starts_with(close) { return Err(p.err("expected `]` after the table name")); }
            p.i += close.len();
            if array {
                let (last, parent) = path.split_last().unwrap();
                let parent_rec = table_at(&mut root, parent, &p)?;
                match parent_rec.get_mut(last) {
                    Some(Value::List(items)) => items.push(Value::Record(Record::new())),
                    Some(_) => return Err(p.err("that key is not an array of tables")),
                    None => parent_rec.insert(last.clone(), Value::List(alloc::vec![Value::Record(Record::new())])),
                }
            } else {
                table_at(&mut root, &path, &p)?;
            }
            current = path;
            p.end_line()?;
            continue;
        }
        let path = p.key_path()?;
        p.ws();
        if p.peek() != Some(b'=') { return Err(p.err("expected `=` after the key")); }
        p.i += 1;
        p.ws();
        let v = p.value()?;
        let (last, parents) = path.split_last().unwrap();
        let mut full = current.clone();
        full.extend(parents.iter().cloned());
        let rec = table_at(&mut root, &full, &p)?;
        if rec.contains(last) { return Err(p.err(&format!("key `{}` is set twice", last))); }
        rec.insert(last.clone(), v);
        p.end_line()?;
    }
    Ok(root)
}

/// The record at `path` below `root`, made if missing; in an array of tables, its last one.
fn table_at<'a>(root: &'a mut Value, path: &[String], p: &Toml) -> Result<&'a mut Record, String> {
    let mut cur = root;
    for key in path {
        let rec = match cur { Value::Record(r) => r, _ => return Err(p.err("that key is not a table")) };
        if !rec.contains(key) { rec.insert(key.clone(), Value::Record(Record::new())); }
        let next = rec.get_mut(key).unwrap();
        cur = match next {
            Value::List(items) => items.last_mut().ok_or_else(|| p.err("empty array of tables"))?,
            other => other,
        };
    }
    match cur { Value::Record(r) => Ok(r), _ => Err(p.err("that key is not a table")) }
}

struct Toml<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

impl Toml<'_> {
    fn err(&self, what: &str) -> String { format!("{} (TOML line {})", what, self.line) }
    fn peek(&self) -> Option<u8> { self.s.get(self.i).copied() }
    fn ws(&mut self) { while matches!(self.peek(), Some(b' ' | b'\t')) { self.i += 1; } }
    fn comment(&mut self) { if self.peek() == Some(b'#') { while !matches!(self.peek(), None | Some(b'\n')) { self.i += 1; } } }
    fn ws_lines(&mut self) {
        loop {
            self.ws();
            self.comment();
            match self.peek() {
                Some(b'\n') => { self.i += 1; self.line += 1; }
                Some(b'\r') => self.i += 1,
                _ => break,
            }
        }
    }
    fn end_line(&mut self) -> Result<(), String> {
        self.ws();
        self.comment();
        match self.peek() {
            None => Ok(()),
            Some(b'\r') | Some(b'\n') => Ok(()),
            _ => Err(self.err("unexpected text at the end of the line")),
        }
    }
    fn key_path(&mut self) -> Result<Vec<String>, String> {
        let mut parts = Vec::new();
        loop {
            self.ws();
            let key = match self.peek() {
                Some(b'"') => self.basic_string()?,
                Some(b'\'') => self.literal_string()?,
                _ => {
                    let start = self.i;
                    while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-') { self.i += 1; }
                    if start == self.i { return Err(self.err("expected a key")); }
                    String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
                }
            };
            parts.push(key);
            self.ws();
            if self.peek() == Some(b'.') { self.i += 1; continue; }
            return Ok(parts);
        }
    }
    fn basic_string(&mut self) -> Result<String, String> {
        if self.s[self.i..].starts_with(b"\"\"\"") {
            self.i += 3;
            if self.peek() == Some(b'\n') { self.i += 1; self.line += 1; }
            let start = self.i;
            while self.i < self.s.len() && !self.s[self.i..].starts_with(b"\"\"\"") {
                if self.s[self.i] == b'\n' { self.line += 1; }
                self.i += 1;
            }
            if self.i >= self.s.len() { return Err(self.err("multi-line string not closed")); }
            let raw = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
            self.i += 3;
            return Ok(unescape(&raw));
        }
        self.i += 1;
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'\\' { self.i += 2; continue; }
            if c == b'"' { break; }
            if c == b'\n' { return Err(self.err("string not closed")); }
            self.i += 1;
        }
        if self.peek() != Some(b'"') { return Err(self.err("string not closed")); }
        let raw = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.i += 1;
        Ok(unescape(&raw))
    }
    fn literal_string(&mut self) -> Result<String, String> {
        let triple = self.s[self.i..].starts_with(b"'''");
        self.i += if triple { 3 } else { 1 };
        let start = self.i;
        let close: &[u8] = if triple { b"'''" } else { b"'" };
        while self.i < self.s.len() && !self.s[self.i..].starts_with(close) {
            if self.s[self.i] == b'\n' { if !triple { return Err(self.err("string not closed")); } self.line += 1; }
            self.i += 1;
        }
        if self.i >= self.s.len() { return Err(self.err("string not closed")); }
        let s = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.i += close.len();
        Ok(s)
    }
    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'"') => Ok(Value::String(self.basic_string()?)),
            Some(b'\'') => Ok(Value::String(self.literal_string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws_lines();
                    if self.peek() == Some(b']') { self.i += 1; return Ok(Value::List(items)); }
                    items.push(self.value()?);
                    self.ws_lines();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => { self.i += 1; return Ok(Value::List(items)); }
                        _ => return Err(self.err("expected `,` or `]` in the array")),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut rec = Value::Record(Record::new());
                loop {
                    self.ws();
                    if self.peek() == Some(b'}') { self.i += 1; return Ok(rec); }
                    let path = self.key_path()?;
                    self.ws();
                    if self.peek() != Some(b'=') { return Err(self.err("expected `=` in the inline table")); }
                    self.i += 1;
                    self.ws();
                    let v = self.value()?;
                    let (last, parents) = path.split_last().unwrap();
                    table_at(&mut rec, parents, self)?.insert(last.clone(), v);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => { self.i += 1; return Ok(rec); }
                        _ => return Err(self.err("expected `,` or `}` in the inline table")),
                    }
                }
            }
            _ => {
                let start = self.i;
                while matches!(self.peek(), Some(c) if !matches!(c, b',' | b']' | b'}' | b'\n' | b'\r' | b'#')) { self.i += 1; }
                let raw = core::str::from_utf8(&self.s[start..self.i]).unwrap_or("").trim_end();
                self.i = start + raw.len();
                match raw {
                    "true" => return Ok(Value::Bool(true)),
                    "false" => return Ok(Value::Bool(false)),
                    "inf" | "+inf" => return Ok(Value::Float(f64::INFINITY)),
                    "-inf" => return Ok(Value::Float(f64::NEG_INFINITY)),
                    "nan" | "+nan" | "-nan" => return Ok(Value::Float(f64::NAN)),
                    _ => {}
                }
                if let Some(d) = units::parse_date(raw) { return Ok(Value::Date(d)); }
                let clean: String = raw.chars().filter(|c| *c != '_').collect();
                if let Some(v) = units::parse_number(&clean).filter(|v| matches!(v, Value::Int(_) | Value::Float(_))) { return Ok(v); }
                if raw.is_empty() { return Err(self.err("expected a value")); }
                Ok(Value::String(raw.to_string()))
            }
        }
    }
}

fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(u @ ('u' | 'U')) => {
                let hex: String = chars.by_ref().take(if u == 'u' { 4 } else { 8 }).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) { out.push(ch); }
            }
            Some(o) => { out.push('\\'); out.push(o); }
            None => out.push('\\'),
        }
    }
    out
}
