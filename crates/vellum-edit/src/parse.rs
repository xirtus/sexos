//! A forgiving tokenizer and object parser over the raw file bytes.

use crate::cos::{Dict, ObjId, Object, Stream};

pub fn is_white(b: u8) -> bool { matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ') }
pub fn is_delim(b: u8) -> bool { matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') }

/// A cursor over the bytes.
pub struct Parser<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

/// How a stream's `/Length` given as an indirect reference gets resolved.
pub type LengthResolver<'r> = &'r dyn Fn(ObjId) -> Option<i64>;

impl<'a> Parser<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Parser<'a> { Parser { data, pos } }

    fn peek(&self) -> Option<u8> { self.data.get(self.pos).copied() }

    pub fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' { break; }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Reads a bare keyword or number token (no delimiters).
    pub fn word(&mut self) -> &'a [u8] {
        self.skip_ws();
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_white(b) || is_delim(b) { break; }
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    pub fn expect_word(&mut self, w: &[u8]) -> bool {
        let save = self.pos;
        if self.word() == w { true } else { self.pos = save; false }
    }

    pub fn uint(&mut self) -> Option<u64> {
        let save = self.pos;
        let w = self.word();
        if !w.is_empty() && w.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(w).ok()?.parse().ok()
        } else {
            self.pos = save;
            None
        }
    }

    /// Parses `n g obj <object> endobj` at the cursor.
    pub fn indirect(&mut self, len: LengthResolver) -> Option<(ObjId, Object)> {
        let num = self.uint()?;
        let gen = self.uint()?;
        if !self.expect_word(b"obj") { return None; }
        let obj = self.object(len, 0)?;
        Some((ObjId(num as u32, gen as u16), obj))
    }

    /// Parses one object; dictionaries followed by `stream` become streams.
    pub fn object(&mut self, len: LengthResolver, depth: usize) -> Option<Object> {
        if depth > 64 { return None; }
        self.skip_ws();
        let b = self.peek()?;
        match b {
            b'/' => { self.pos += 1; Some(Object::Name(self.name_body())) }
            b'(' => { self.pos += 1; Some(Object::Str(self.literal())) }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let dict = self.dict_body(len, depth)?;
                let save = self.pos;
                if self.expect_word(b"stream") {
                    Some(Object::Stream(self.stream_body(dict, len)?))
                } else {
                    self.pos = save;
                    Some(Object::Dict(dict))
                }
            }
            b'<' => { self.pos += 1; Some(Object::Str(self.hex())) }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek()? {
                        b']' => { self.pos += 1; break; }
                        _ => items.push(self.object(len, depth + 1)?),
                    }
                }
                Some(Object::Array(items))
            }
            _ => {
                let save = self.pos;
                let w = self.word();
                match w {
                    b"true" => Some(Object::Bool(true)),
                    b"false" => Some(Object::Bool(false)),
                    b"null" => Some(Object::Null),
                    _ => {
                        let n = parse_number(w)?;
                        if let Object::Int(i) = n {
                            // `n g R`?
                            if i >= 0 {
                                let after = self.pos;
                                if let Some(g) = self.uint() {
                                    if self.expect_word(b"R") {
                                        return Some(Object::Ref(ObjId(i as u32, g as u16)));
                                    }
                                }
                                self.pos = after;
                            }
                        }
                        if w.is_empty() { self.pos = save; return None; }
                        Some(n)
                    }
                }
            }
        }
    }

    fn name_body(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            if is_white(b) || is_delim(b) { break; }
            self.pos += 1;
            if b == b'#' {
                let h = self.data.get(self.pos..self.pos + 2).and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = h { out.push(v); self.pos += 2; continue; }
            }
            out.push(b);
        }
        out
    }

    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'(' => { depth += 1; out.push(b); }
                b')' => { depth -= 1; if depth == 0 { break; } out.push(b); }
                b'\\' => {
                    let Some(e) = self.peek() else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => { if self.peek() == Some(b'\n') { self.pos += 1; } }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => { v = v * 8 + (d - b'0') as u32; self.pos += 1; }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'>' { break; }
            let v = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => continue,
            };
            match hi.take() { Some(h) => out.push(h << 4 | v), None => hi = Some(v) }
        }
        if let Some(h) = hi { out.push(h << 4); }
        out
    }

    fn dict_body(&mut self, len: LengthResolver, depth: usize) -> Option<Dict> {
        let mut d = Dict::new();
        loop {
            self.skip_ws();
            match self.peek()? {
                b'>' => {
                    self.pos += 1;
                    if self.peek() == Some(b'>') { self.pos += 1; }
                    return Some(d);
                }
                b'/' => {
                    self.pos += 1;
                    let k = self.name_body();
                    let v = self.object(len, depth + 1)?;
                    // A null value means the key is absent (7.3.7).
                    if v != Object::Null { d.0.retain(|(kk, _)| kk != &k); d.0.push((k, v)); }
                }
                _ => {
                    // Junk inside a dictionary: skip one token.
                    let w = self.word();
                    if w.is_empty() { self.pos += 1; }
                }
            }
        }
    }

    fn stream_body(&mut self, dict: Dict, len: LengthResolver) -> Option<Stream> {
        // EOL after `stream`: CRLF or LF (CR alone tolerated).
        if self.peek() == Some(b'\r') { self.pos += 1; }
        if self.peek() == Some(b'\n') { self.pos += 1; }
        let start = self.pos;
        let declared = match dict.get(b"Length") {
            Some(Object::Int(n)) => Some(*n),
            Some(Object::Ref(r)) => len(*r),
            _ => None,
        };
        if let Some(n) = declared {
            if n >= 0 {
                let end = start.checked_add(n as usize)?;
                if end <= self.data.len() {
                    let mut p = Parser::new(self.data, end);
                    if p.expect_word(b"endstream") {
                        self.pos = p.pos;
                        return Some(Stream { dict, data: self.data[start..end].to_vec() });
                    }
                }
            }
        }
        // Wrong or missing length: search for `endstream`.
        let rel = find(&self.data[start..], b"endstream")?;
        let mut end = start + rel;
        if end > start && self.data[end - 1] == b'\n' { end -= 1; }
        if end > start && self.data[end - 1] == b'\r' { end -= 1; }
        self.pos = start + rel + b"endstream".len();
        Some(Stream { dict, data: self.data[start..end].to_vec() })
    }
}

pub fn parse_number(w: &[u8]) -> Option<Object> {
    let s = std::str::from_utf8(w).ok()?;
    if s.is_empty() { return None; }
    if let Ok(i) = s.parse::<i64>() { return Some(Object::Int(i)); }
    // Reals: `.5`, `-.5`, `4.`, also tolerate `--5` written by some producers.
    let t = s.trim_start_matches('+');
    let t = if t.starts_with("--") { &t[1..] } else { t };
    if t.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-') && t.bytes().any(|b| b.is_ascii_digit()) {
        return t.parse::<f64>().ok().map(Object::Real);
    }
    None
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}
