// SPDX-License-Identifier: GPL-3.0-only
//! Bencode (BEP 3): integers `i42e`, byte strings `4:spam`, lists `l...e` and
//! dictionaries `d...e` with keys sorted as raw bytes.
//!
//! Decoding borrows from the input and keeps, for every value, the exact byte span it
//! came from, because an info-hash is the hash of the `info` dictionary *as it appeared*,
//! not as we would re-encode it. Decoding is strict where it matters for safety (depth
//! limit, no leading zeros, no `-0`, lengths checked against the input) and accepts
//! unsorted dictionary keys, which real torrents sometimes have; [`Value::canonical`]
//! reports whether a value was encoded canonically.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Nesting deeper than this is refused (a hostile peer could otherwise exhaust the stack).
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended inside a value.
    Eof,
    /// A byte that cannot start or continue a value here.
    Syntax(usize),
    /// An integer with leading zeros, `-0`, or out of range for `i64`.
    Integer(usize),
    /// Nested deeper than [`MAX_DEPTH`].
    Depth,
    /// Bytes left over after the top-level value.
    Trailing(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    /// Entries in input order (duplicates kept); use [`Value::get`] to look up.
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

/// A decoded value and the span of input it was decoded from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value<'a> {
    pub kind: Kind<'a>,
    /// The exact bytes this value was encoded as.
    pub raw: &'a [u8],
}

impl<'a> Value<'a> {
    pub fn int(&self) -> Option<i64> { if let Kind::Int(n) = self.kind { Some(n) } else { None } }
    pub fn bytes(&self) -> Option<&'a [u8]> { if let Kind::Bytes(b) = self.kind { Some(b) } else { None } }
    pub fn str(&self) -> Option<&'a str> { core::str::from_utf8(self.bytes()?).ok() }
    pub fn list(&self) -> Option<&[Value<'a>]> { if let Kind::List(l) = &self.kind { Some(l) } else { None } }
    pub fn dict(&self) -> Option<&[(&'a [u8], Value<'a>)]> { if let Kind::Dict(d) = &self.kind { Some(d) } else { None } }
    /// The first entry with this key, if this is a dictionary.
    pub fn get(&self, key: &[u8]) -> Option<&Value<'a>> {
        self.dict()?.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
    /// Whether this value (recursively) is canonical: sorted, unique dictionary keys.
    pub fn canonical(&self) -> bool {
        match &self.kind {
            Kind::Int(_) | Kind::Bytes(_) => true,
            Kind::List(l) => l.iter().all(|v| v.canonical()),
            Kind::Dict(d) => d.windows(2).all(|w| w[0].0 < w[1].0) && d.iter().all(|(_, v)| v.canonical()),
        }
    }
    /// Converts to an owned [`Item`].
    pub fn to_item(&self) -> Item {
        match &self.kind {
            Kind::Int(n) => Item::Int(*n),
            Kind::Bytes(b) => Item::Bytes(b.to_vec()),
            Kind::List(l) => Item::List(l.iter().map(|v| v.to_item()).collect()),
            Kind::Dict(d) => Item::Dict(d.iter().map(|(k, v)| (k.to_vec(), v.to_item())).collect()),
        }
    }
}

/// Decodes exactly one value filling all of `input`.
pub fn decode(input: &[u8]) -> Result<Value<'_>, Error> {
    let (v, used) = decode_prefix(input)?;
    if used != input.len() { return Err(Error::Trailing(used)); }
    Ok(v)
}

/// Decodes one value from the start of `input`, returning it and the bytes used. The
/// extension protocol's `ut_metadata` messages put raw data after a bencoded dictionary.
pub fn decode_prefix(input: &[u8]) -> Result<(Value<'_>, usize), Error> {
    let mut p = Parser { input, pos: 0 };
    let v = p.value(0)?;
    Ok((v, p.pos))
}

struct Parser<'a> { input: &'a [u8], pos: usize }

impl<'a> Parser<'a> {
    fn peek(&self) -> Result<u8, Error> { self.input.get(self.pos).copied().ok_or(Error::Eof) }

    fn value(&mut self, depth: usize) -> Result<Value<'a>, Error> {
        if depth > MAX_DEPTH { return Err(Error::Depth); }
        let start = self.pos;
        let kind = match self.peek()? {
            b'i' => { self.pos += 1; Kind::Int(self.integer(b'e')?) }
            b'0'..=b'9' => Kind::Bytes(self.byte_string()?),
            b'l' => {
                self.pos += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' { items.push(self.value(depth + 1)?); }
                self.pos += 1;
                Kind::List(items)
            }
            b'd' => {
                self.pos += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    if !self.peek()?.is_ascii_digit() { return Err(Error::Syntax(self.pos)); }
                    let k = self.byte_string()?;
                    let v = self.value(depth + 1)?;
                    items.push((k, v));
                }
                self.pos += 1;
                Kind::Dict(items)
            }
            _ => return Err(Error::Syntax(self.pos)),
        };
        Ok(Value { kind, raw: &self.input[start..self.pos] })
    }

    /// Reads a decimal integer up to `end` (consumed).
    fn integer(&mut self, end: u8) -> Result<i64, Error> {
        let start = self.pos;
        let neg = self.peek()? == b'-';
        if neg { self.pos += 1; }
        let digits = self.pos;
        let mut n: i64 = 0;
        loop {
            let c = self.peek()?;
            if c == end { break; }
            if !c.is_ascii_digit() { return Err(Error::Syntax(self.pos)); }
            n = n.checked_mul(10).and_then(|n| if neg { n.checked_sub((c - b'0') as i64) } else { n.checked_add((c - b'0') as i64) }).ok_or(Error::Integer(start))?;
            self.pos += 1;
        }
        let len = self.pos - digits;
        if len == 0 || (len > 1 && self.input[digits] == b'0') || (neg && n == 0) { return Err(Error::Integer(start)); }
        self.pos += 1;
        Ok(n)
    }

    fn byte_string(&mut self) -> Result<&'a [u8], Error> {
        let at = self.pos;
        let len = self.integer(b':')?;
        if len < 0 { return Err(Error::Integer(at)); }
        let len = len as usize;
        if self.input.len() - self.pos < len { return Err(Error::Eof); }
        let s = &self.input[self.pos..self.pos + len];
        self.pos += len;
        Ok(s)
    }
}

/// An owned value, for building messages. Dictionaries are `BTreeMap`s, so encoding is
/// always canonical (keys sorted as raw bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Item>),
    Dict(BTreeMap<Vec<u8>, Item>),
}

impl Item {
    pub fn encode(&self) -> Vec<u8> { let mut out = Vec::new(); self.encode_into(&mut out); out }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Item::Int(n) => { out.push(b'i'); push_decimal(out, *n); out.push(b'e'); }
            Item::Bytes(b) => { push_decimal(out, b.len() as i64); out.push(b':'); out.extend_from_slice(b); }
            Item::List(l) => { out.push(b'l'); for i in l { i.encode_into(out); } out.push(b'e'); }
            Item::Dict(d) => {
                out.push(b'd');
                for (k, v) in d { push_decimal(out, k.len() as i64); out.push(b':'); out.extend_from_slice(k); v.encode_into(out); }
                out.push(b'e');
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&Item> { if let Item::Dict(d) = self { d.get(key) } else { None } }
}

impl From<i64> for Item { fn from(n: i64) -> Self { Item::Int(n) } }
impl From<&[u8]> for Item { fn from(b: &[u8]) -> Self { Item::Bytes(b.to_vec()) } }
impl From<&str> for Item { fn from(s: &str) -> Self { Item::Bytes(s.as_bytes().to_vec()) } }
impl From<Vec<u8>> for Item { fn from(b: Vec<u8>) -> Self { Item::Bytes(b) } }

/// Builds a dictionary: `dict([(b"a", 1.into()), ...])`.
pub fn dict<const N: usize>(entries: [(&[u8], Item); N]) -> Item {
    Item::Dict(entries.into_iter().map(|(k, v)| (k.to_vec(), v)).collect())
}

fn push_decimal(out: &mut Vec<u8>, n: i64) {
    if n < 0 { out.push(b'-'); }
    let mut m = n.unsigned_abs();
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop { i -= 1; buf[i] = b'0' + (m % 10) as u8; m /= 10; if m == 0 { break; } }
    out.extend_from_slice(&buf[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goldens() {
        assert_eq!(decode(b"i42e").unwrap().int(), Some(42));
        assert_eq!(decode(b"i-7e").unwrap().int(), Some(-7));
        assert_eq!(decode(b"4:spam").unwrap().bytes(), Some(&b"spam"[..]));
        assert_eq!(decode(b"0:").unwrap().bytes(), Some(&b""[..]));
        let v = decode(b"d3:cow3:moo4:spaml1:a1:bee").unwrap();
        assert_eq!(v.get(b"cow").unwrap().str(), Some("moo"));
        assert_eq!(v.get(b"spam").unwrap().list().unwrap().len(), 2);
        assert!(v.canonical());
        assert_eq!(v.to_item().encode(), b"d3:cow3:moo4:spaml1:a1:bee");
    }

    #[test]
    fn spans_are_exact() {
        let input = b"d4:infod6:lengthi5e4:name1:xe1:zi0ee";
        let v = decode(input).unwrap();
        assert_eq!(v.get(b"info").unwrap().raw, b"d6:lengthi5e4:name1:xe");
    }

    #[test]
    fn refuses_bad_input() {
        assert_eq!(decode(b"i03e"), Err(Error::Integer(1)));
        assert_eq!(decode(b"i-0e"), Err(Error::Integer(1)));
        assert_eq!(decode(b"ie"), Err(Error::Integer(1)));
        assert!(decode(b"i99999999999999999999e").is_err());
        assert_eq!(decode(b"5:abc"), Err(Error::Eof));
        assert_eq!(decode(b"i1ei2e"), Err(Error::Trailing(3)));
        assert!(decode(b"di1ei2ee").is_err());
        assert_eq!(decode(b"l").unwrap_err(), Error::Eof);
        let deep: Vec<u8> = core::iter::repeat(b'l').take(100).chain(core::iter::repeat(b'e').take(100)).collect();
        assert_eq!(decode(&deep), Err(Error::Depth));
    }

    #[test]
    fn unsorted_dicts_decode_but_are_not_canonical() {
        let v = decode(b"d1:bi1e1:ai2ee").unwrap();
        assert_eq!(v.get(b"a").unwrap().int(), Some(2));
        assert!(!v.canonical());
    }

    #[test]
    fn prefix_leaves_trailing_data() {
        let (v, n) = decode_prefix(b"d8:msg_typei1e5:piecei0eeRAW").unwrap();
        assert_eq!(v.get(b"piece").unwrap().int(), Some(0));
        assert_eq!(n, 25);
    }

    #[test]
    fn encode_is_sorted() {
        let d = dict([(b"zeta", 1.into()), (b"alpha", "x".into()), (b"mid", Item::List(alloc::vec![(-3).into()]))]);
        assert_eq!(d.encode(), b"d5:alpha1:x3:midli-3ee4:zetai1ee");
        assert_eq!(push(i64::MIN), b"i-9223372036854775808e");
        fn push(n: i64) -> Vec<u8> { Item::Int(n).encode() }
    }
}
