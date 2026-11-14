//! The PDF object model (ISO 32000 7.3) and its serialisation.

use std::fmt::Write as _;

/// An indirect object's number and generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjId(pub u32, pub u16);

/// A PDF object. Streams keep their bytes exactly as stored (still encoded).
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjId),
}

/// A dictionary, keeping key order so rewritten objects stay recognisable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict(pub Vec<(Vec<u8>, Object)>);

/// A stream: its dictionary and its stored (encoded) bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Vec<u8>,
}

impl Dict {
    pub fn new() -> Dict { Dict(Vec::new()) }
    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Object> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn set(&mut self, key: &[u8], value: impl Into<Object>) {
        let value = value.into();
        match self.get_mut(key) {
            Some(v) => *v = value,
            None => self.0.push((key.to_vec(), value)),
        }
    }
    pub fn remove(&mut self, key: &[u8]) -> Option<Object> {
        let i = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(i).1)
    }
    pub fn with(mut self, key: &[u8], value: impl Into<Object>) -> Dict {
        self.set(key, value);
        self
    }
}

impl Object {
    pub fn name(s: &str) -> Object { Object::Name(s.as_bytes().to_vec()) }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            Object::Real(r) => Some(*r as i64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    }
    pub fn as_name(&self) -> Option<&[u8]> {
        if let Object::Name(n) = self { Some(n) } else { None }
    }
    pub fn as_str(&self) -> Option<&[u8]> {
        if let Object::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Object>> {
        if let Object::Array(a) = self { Some(a) } else { None }
    }
    pub fn as_ref(&self) -> Option<ObjId> {
        if let Object::Ref(r) = self { Some(*r) } else { None }
    }
}

impl From<Dict> for Object { fn from(d: Dict) -> Object { Object::Dict(d) } }
impl From<ObjId> for Object { fn from(r: ObjId) -> Object { Object::Ref(r) } }
impl From<i64> for Object { fn from(i: i64) -> Object { Object::Int(i) } }
impl From<f64> for Object { fn from(r: f64) -> Object { Object::Real(r) } }
impl From<f32> for Object { fn from(r: f32) -> Object { Object::Real(r as f64) } }
impl From<bool> for Object { fn from(b: bool) -> Object { Object::Bool(b) } }
impl From<Vec<Object>> for Object { fn from(a: Vec<Object>) -> Object { Object::Array(a) } }
impl From<Stream> for Object { fn from(s: Stream) -> Object { Object::Stream(s) } }

/// Formats a number the way content streams and objects want it: no exponent, few digits.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() { return "0".into(); }
    if (v - v.round()).abs() < 1e-6 { return format!("{}", v.round() as i64); }
    let mut s = format!("{v:.4}");
    while s.ends_with('0') { s.pop(); }
    if s.ends_with('.') { s.pop(); }
    if s == "-0" { s = "0".into(); }
    s
}

fn is_regular(b: u8) -> bool {
    !matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// Serialises one object (never the `n g obj` wrapper).
pub fn write_object(out: &mut Vec<u8>, obj: &Object) {
    match obj {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Object::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Object::Real(r) => out.extend_from_slice(fmt_num(*r).as_bytes()),
        Object::Str(s) => write_string(out, s),
        Object::Name(n) => write_name(out, n),
        Object::Array(a) => {
            out.push(b'[');
            for (i, o) in a.iter().enumerate() {
                if i > 0 { out.push(b' '); }
                write_object(out, o);
            }
            out.push(b']');
        }
        Object::Dict(d) => write_dict(out, d),
        Object::Stream(s) => {
            let mut d = s.dict.clone();
            d.set(b"Length", Object::Int(s.data.len() as i64));
            write_dict(out, &d);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&s.data);
            out.extend_from_slice(b"\nendstream");
        }
        Object::Ref(r) => out.extend_from_slice(format!("{} {} R", r.0, r.1).as_bytes()),
    }
}

fn write_dict(out: &mut Vec<u8>, d: &Dict) {
    out.extend_from_slice(b"<<");
    for (k, v) in &d.0 {
        write_name(out, k);
        out.push(b' ');
        write_object(out, v);
        out.push(b' ');
    }
    out.extend_from_slice(b">>");
}

fn write_name(out: &mut Vec<u8>, n: &[u8]) {
    out.push(b'/');
    for &b in n {
        if b == b'#' || !(0x21..=0x7e).contains(&b) || !is_regular(b) {
            out.extend_from_slice(format!("#{b:02X}").as_bytes());
        } else {
            out.push(b);
        }
    }
}

fn write_string(out: &mut Vec<u8>, s: &[u8]) {
    let binary = s.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t') || b >= 0x7f).count();
    if binary * 4 > s.len().max(1) {
        out.push(b'<');
        let mut h = String::with_capacity(s.len() * 2);
        for b in s { let _ = write!(h, "{b:02X}"); }
        out.extend_from_slice(h.as_bytes());
        out.push(b'>');
        return;
    }
    out.push(b'(');
    for &b in s {
        match b {
            b'(' | b')' | b'\\' => { out.push(b'\\'); out.push(b); }
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x20..=0x7e => out.push(b),
            _ => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
        }
    }
    out.push(b')');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    #[test]
    fn objects_survive_write_then_parse() {
        let d = Dict::new()
            .with(b"Name With#Space", Object::name("A/B"))
            .with(b"S", Object::Str(b"(paren) \\ back\nslash\x01".to_vec()))
            .with(b"Bin", Object::Str(vec![0xFE, 0xFF, 0, 0x41, 0x4E, 0x2D]))
            .with(b"N", Object::Array(vec![Object::Int(-3), Object::Real(0.125), Object::Real(-0.5), Object::Bool(true), Object::Ref(ObjId(12, 3))]));
        let mut out = Vec::new();
        write_object(&mut out, &Object::Dict(d.clone()));
        let back = Parser::new(&out, 0).object(&|_| None, 0).unwrap();
        assert_eq!(back, Object::Dict(d));
        assert_eq!(fmt_num(1.0 / 3.0), "0.3333");
        assert_eq!(fmt_num(-0.00001), "0");
    }

    #[test]
    fn stream_length_fallback() {
        let src = b"7 0 obj << /Length 999 >> stream\nabc\nendstream endobj";
        let (id, o) = Parser::new(src, 0).indirect(&|_| None).unwrap();
        assert_eq!(id, ObjId(7, 0));
        let Object::Stream(s) = o else { panic!() };
        assert_eq!(s.data, b"abc");
    }
}
