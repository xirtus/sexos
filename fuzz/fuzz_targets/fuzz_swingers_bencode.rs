// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use libfuzzer_sys::fuzz_target;

extern crate alloc;

// Mirror of sex-torrent::bencode (PR #236 / Swingers BitTorrent client)
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Eof,
    Syntax(usize),
    Integer(usize),
    Depth,
    Trailing(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value<'a> {
    pub kind: Kind<'a>,
    pub raw: &'a [u8],
}

impl<'a> Value<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Value<'a>, Error> {
        decode(input)
    }

    pub fn parse_prefix(input: &'a [u8]) -> Result<(Value<'a>, usize), Error> {
        decode_prefix(input)
    }

    pub fn int(&self) -> Option<i64> {
        if let Kind::Int(n) = self.kind {
            Some(n)
        } else {
            None
        }
    }

    pub fn bytes(&self) -> Option<&'a [u8]> {
        if let Kind::Bytes(b) = self.kind {
            Some(b)
        } else {
            None
        }
    }

    pub fn str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.bytes()?).ok()
    }

    pub fn list(&self) -> Option<&[Value<'a>]> {
        if let Kind::List(l) = &self.kind {
            Some(l)
        } else {
            None
        }
    }

    pub fn dict(&self) -> Option<&[(&'a [u8], Value<'a>)]> {
        if let Kind::Dict(d) = &self.kind {
            Some(d)
        } else {
            None
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&Value<'a>> {
        self.dict()?.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn canonical(&self) -> bool {
        match &self.kind {
            Kind::Int(_) | Kind::Bytes(_) => true,
            Kind::List(l) => l.iter().all(|v| v.canonical()),
            Kind::Dict(d) => {
                d.windows(2).all(|w| w[0].0 < w[1].0) && d.iter().all(|(_, v)| v.canonical())
            }
        }
    }

    pub fn to_item(&self) -> Item {
        match &self.kind {
            Kind::Int(n) => Item::Int(*n),
            Kind::Bytes(b) => Item::Bytes(b.to_vec()),
            Kind::List(l) => Item::List(l.iter().map(|v| v.to_item()).collect()),
            Kind::Dict(d) => {
                Item::Dict(d.iter().map(|(k, v)| (k.to_vec(), v.to_item())).collect())
            }
        }
    }
}

pub fn decode(input: &[u8]) -> Result<Value<'_>, Error> {
    let (v, used) = decode_prefix(input)?;
    if used != input.len() {
        return Err(Error::Trailing(used));
    }
    Ok(v)
}

pub fn decode_prefix(input: &[u8]) -> Result<(Value<'_>, usize), Error> {
    let mut p = Parser { input, pos: 0 };
    let v = p.value(0)?;
    Ok((v, p.pos))
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Result<u8, Error> {
        self.input.get(self.pos).copied().ok_or(Error::Eof)
    }

    fn value(&mut self, depth: usize) -> Result<Value<'a>, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Depth);
        }
        let start = self.pos;
        let kind = match self.peek()? {
            b'i' => {
                self.pos += 1;
                Kind::Int(self.integer(b'e')?)
            }
            b'0'..=b'9' => Kind::Bytes(self.byte_string()?),
            b'l' => {
                self.pos += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    items.push(self.value(depth + 1)?);
                }
                self.pos += 1;
                Kind::List(items)
            }
            b'd' => {
                self.pos += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    if !self.peek()?.is_ascii_digit() {
                        return Err(Error::Syntax(self.pos));
                    }
                    let k = self.byte_string()?;
                    let v = self.value(depth + 1)?;
                    items.push((k, v));
                }
                self.pos += 1;
                Kind::Dict(items)
            }
            _ => return Err(Error::Syntax(self.pos)),
        };
        Ok(Value {
            kind,
            raw: &self.input[start..self.pos],
        })
    }

    fn integer(&mut self, end: u8) -> Result<i64, Error> {
        let start = self.pos;
        let neg = self.peek()? == b'-';
        if neg {
            self.pos += 1;
        }
        let digits = self.pos;
        let mut n: i64 = 0;
        loop {
            let c = self.peek()?;
            if c == end {
                break;
            }
            if !c.is_ascii_digit() {
                return Err(Error::Syntax(self.pos));
            }
            n = n
                .checked_mul(10)
                .and_then(|n| {
                    if neg {
                        n.checked_sub((c - b'0') as i64)
                    } else {
                        n.checked_add((c - b'0') as i64)
                    }
                })
                .ok_or(Error::Integer(start))?;
            self.pos += 1;
        }
        let len = self.pos - digits;
        if len == 0 || (len > 1 && self.input[digits] == b'0') || (neg && n == 0) {
            return Err(Error::Integer(start));
        }
        self.pos += 1;
        Ok(n)
    }

    fn byte_string(&mut self) -> Result<&'a [u8], Error> {
        let at = self.pos;
        let len = self.integer(b':')?;
        if len < 0 {
            return Err(Error::Integer(at));
        }
        let len = len as usize;
        if self.input.len() - self.pos < len {
            return Err(Error::Eof);
        }
        let s = &self.input[self.pos..self.pos + len];
        self.pos += len;
        Ok(s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Item>),
    Dict(BTreeMap<Vec<u8>, Item>),
}

impl Item {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Item::Int(n) => {
                out.push(b'i');
                push_decimal(out, *n);
                out.push(b'e');
            }
            Item::Bytes(b) => {
                push_decimal(out, b.len() as i64);
                out.push(b':');
                out.extend_from_slice(b);
            }
            Item::List(l) => {
                out.push(b'l');
                for i in l {
                    i.encode_into(out);
                }
                out.push(b'e');
            }
            Item::Dict(d) => {
                out.push(b'd');
                for (k, v) in d {
                    push_decimal(out, k.len() as i64);
                    out.push(b':');
                    out.extend_from_slice(k);
                    v.encode_into(out);
                }
                out.push(b'e');
            }
        }
    }
}

fn push_decimal(out: &mut Vec<u8>, n: i64) {
    if n < 0 {
        out.push(b'-');
    }
    let mut m = n.unsigned_abs();
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

pub fn sanitize_component(raw: &[u8]) -> Option<alloc::string::String> {
    let s = alloc::string::String::from_utf8_lossy(raw);
    let mut out = alloc::string::String::with_capacity(s.len().min(255));
    for c in s.chars() {
        let c = if c.is_control() || c == '/' || c == '\\' { '_' } else { c };
        if out.len() + c.len_utf8() > 255 { break; }
        out.push(c);
    }
    if out.is_empty() || out == "." || out == ".." { None } else { Some(out) }
}

fn exercise_metainfo(val: &Value<'_>) {
    if let Some(dict) = val.dict() {
        let _ = val.get(b"announce").and_then(|a| a.str());
        let _ = val.get(b"comment").and_then(|c| c.str());
        let _ = val.get(b"created by").and_then(|c| c.str());
        let _ = val.get(b"creation date").and_then(|d| d.int());

        if let Some(info) = val.get(b"info") {
            let _ = info.get(b"name").and_then(|n| n.str());
            let pl = info.get(b"piece length").and_then(|pl| pl.int());
            let _ = info.get(b"pieces").and_then(|p| p.bytes());
            let _ = info.get(b"length").and_then(|l| l.int());

            if let Some(p_len) = pl {
                if p_len > 0 && p_len <= (1 << 29) {
                    let _pow2 = (p_len as u64).is_power_of_two();
                }
            }

            if let Some(files) = info.get(b"files").and_then(|f| f.list()) {
                let mut total_len: u64 = 0;
                for file in files.iter().take(16) {
                    if let Some(l) = file.get(b"length").and_then(|l| l.int()) {
                        if l >= 0 {
                            total_len = total_len.saturating_add(l as u64);
                        }
                    }
                    if let Some(path_list) = file.get(b"path").and_then(|p| p.list()) {
                        for p in path_list.iter().take(8) {
                            if let Some(bytes) = p.bytes() {
                                let _ = sanitize_component(bytes);
                            }
                        }
                    }
                }
                let _ = total_len;
            }
        }
        let _ = dict.len();
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Full value parsing
    if let Ok(val) = Value::parse(data) {
        let canonical = val.canonical();
        exercise_metainfo(&val);

        // Conversion to owned Item and canonical re-encoding round-trip
        let item = val.to_item();
        let encoded = item.encode();

        if canonical {
            // Canonical inputs must round-trip exactly
            if let Ok(redecoded) = Value::parse(&encoded) {
                assert!(redecoded.canonical(), "Canonical item did not re-decode canonically");
            }
        }
    }

    // 2. Prefix parsing (ut_metadata bencode dictionary with trailing raw payload)
    if let Ok((val, used)) = Value::parse_prefix(data) {
        assert!(used <= data.len());
        exercise_metainfo(&val);
    }

    // 3. Path component sanitization on arbitrary data
    let _ = sanitize_component(data);

    // 4. Targeted high-yield bencode edge-cases
    let _ = Value::parse(b"i-9223372036854775808e"); // i64::MIN
    let _ = Value::parse(b"i9223372036854775807e");  // i64::MAX
    let _ = Value::parse(b"i-0e");                   // negative zero
    let _ = Value::parse(b"i0e");                    // zero
    let _ = Value::parse(b"i00e");                   // leading zero
    let _ = Value::parse(b"99999999999999999999:a"); // usize length overflow
    let _ = Value::parse(b"d1:\xFF1:ae");            // non-UTF8 dict key

    // Deeply nested input check (> MAX_DEPTH = 64)
    let mut deep = Vec::with_capacity(140);
    for _ in 0..70 { deep.push(b'l'); }
    for _ in 0..70 { deep.push(b'e'); }
    let _ = Value::parse(&deep);

    // Metainfo piece length edge cases
    let _ = Value::parse(b"d4:infod12:piece lengthi0e6:pieces0:4:name4:testee");
    let _ = Value::parse(b"d4:infod12:piece lengthi-1e6:pieces0:4:name4:testee");
    let _ = Value::parse(b"d4:infod12:piece lengthi536870913e6:pieces0:4:name4:testee");
});
