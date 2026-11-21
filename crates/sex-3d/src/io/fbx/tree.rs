// SPDX-License-Identifier: GPL-3.0-only
//! The FBX node tree: binary (6.1 to 7.7) and ASCII readers, a binary writer.
//!
//! An FBX file is a tree of named nodes, each with a list of typed values
//! ("properties") and child nodes. Binary files store it as records with
//! 32-bit offsets (version < 7500) or 64-bit offsets (7500 and up); numeric
//! arrays may be zlib-compressed. ASCII files write the same tree as
//! `Name: value, value { children }` with `*N { a: ... }` arrays.
//!
//! Object names differ between the two: binary writes `Name\0\x01Class`, ASCII
//! writes `Class::Name`. [`object_name`] strips the class from either.
use std::io::{Read, Write as _};

/// One value of a node.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    ArrBool(Vec<bool>),
    ArrI32(Vec<i32>),
    ArrI64(Vec<i64>),
    ArrF32(Vec<f32>),
    ArrF64(Vec<f64>),
}

impl Prop {
    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Prop::Bool(b) => *b as i32 as f64,
            Prop::I16(v) => *v as f64,
            Prop::I32(v) => *v as f64,
            Prop::I64(v) => *v as f64,
            Prop::F32(v) => *v as f64,
            Prop::F64(v) => *v,
            _ => return None,
        })
    }
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Prop::Bool(b) => *b as i64,
            Prop::I16(v) => *v as i64,
            Prop::I32(v) => *v as i64,
            Prop::I64(v) => *v,
            Prop::F32(v) => *v as i64,
            Prop::F64(v) => *v as i64,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Any numeric array (or a lone number) as f64s.
    pub fn to_f64s(&self) -> Option<Vec<f64>> {
        Some(match self {
            Prop::ArrF64(v) => v.clone(),
            Prop::ArrF32(v) => v.iter().map(|&x| x as f64).collect(),
            Prop::ArrI32(v) => v.iter().map(|&x| x as f64).collect(),
            Prop::ArrI64(v) => v.iter().map(|&x| x as f64).collect(),
            Prop::ArrBool(v) => v.iter().map(|&x| x as i32 as f64).collect(),
            p => vec![p.as_f64()?],
        })
    }
    /// Any numeric array (or a lone number) as i64s.
    pub fn to_i64s(&self) -> Option<Vec<i64>> {
        Some(match self {
            Prop::ArrI64(v) => v.clone(),
            Prop::ArrI32(v) => v.iter().map(|&x| x as i64).collect(),
            Prop::ArrF64(v) => v.iter().map(|&x| x as i64).collect(),
            Prop::ArrF32(v) => v.iter().map(|&x| x as i64).collect(),
            Prop::ArrBool(v) => v.iter().map(|&x| x as i64).collect(),
            p => vec![p.as_i64()?],
        })
    }
}

/// One node of the tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(name: &str, props: Vec<Prop>) -> Node {
        Node { name: name.into(), props, children: Vec::new() }
    }
    pub fn with(mut self, child: Node) -> Node {
        self.children.push(child);
        self
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn prop(&self, i: usize) -> Option<&Prop> {
        self.props.get(i)
    }
    pub fn str(&self, i: usize) -> Option<&str> {
        self.props.get(i).and_then(Prop::as_str)
    }
    pub fn i64(&self, i: usize) -> Option<i64> {
        self.props.get(i).and_then(Prop::as_i64)
    }
    /// The first property of child `name` as numbers.
    pub fn child_f64s(&self, name: &str) -> Option<Vec<f64>> {
        self.child(name)?.props.first()?.to_f64s()
    }
    pub fn child_i64s(&self, name: &str) -> Option<Vec<i64>> {
        self.child(name)?.props.first()?.to_i64s()
    }
    pub fn child_str(&self, name: &str) -> Option<&str> {
        self.child(name)?.str(0)
    }
}

/// The object name without its class (`Cube\0\x01Model` or `Model::Cube` → `Cube`).
pub fn object_name(s: &str) -> &str {
    if let Some(i) = s.find("\0\u{1}") {
        &s[..i]
    } else if let Some(i) = s.find("::") {
        &s[i + 2..]
    } else {
        s
    }
}

pub const BINARY_MAGIC: &[u8; 23] = b"Kaydara FBX Binary  \0\x1a\0";

/// A parsed file: its version (7400 for 7.4) and top-level nodes.
#[derive(Debug, Default)]
pub struct Document {
    pub version: u32,
    pub binary: bool,
    pub nodes: Vec<Node>,
}

impl Document {
    pub fn get(&self, name: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.name == name)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Document, String> {
    if bytes.starts_with(BINARY_MAGIC) {
        parse_binary(bytes)
    } else {
        parse_ascii(bytes)
    }
}

// ---------------------------------------------------------------- binary

/// Largest array the reader accepts (elements), so a corrupt count cannot
/// make it allocate gigabytes.
const MAX_ARRAY: usize = 1 << 28;

struct Bin<'a> {
    b: &'a [u8],
    pos: usize,
    wide: bool,
}

impl<'a> Bin<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| format!("truncated at byte {}", self.pos))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn off(&mut self) -> Result<u64, String> {
        if self.wide {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    /// One node, or `None` at a list-ending null record.
    fn node(&mut self, depth: usize) -> Result<Option<Node>, String> {
        if depth > 64 {
            return Err("nodes nested too deeply".into());
        }
        let start = self.pos;
        let end = self.off()?;
        let nprops = self.off()?;
        let _plen = self.off()?;
        let nlen = self.u8()? as usize;
        if end == 0 && nprops == 0 && nlen == 0 {
            return Ok(None);
        }
        let end = usize::try_from(end).map_err(|_| "bad node offset")?;
        if end <= start || end > self.b.len() {
            return Err(format!("node at byte {start} ends outside the file"));
        }
        let name = String::from_utf8_lossy(self.take(nlen)?).into_owned();
        if nprops > (end - self.pos) as u64 {
            return Err(format!("node {name}: too many properties"));
        }
        let mut props = Vec::with_capacity(nprops as usize);
        for _ in 0..nprops {
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.pos < end {
            match self.node(depth + 1)? {
                Some(c) => children.push(c),
                None => break,
            }
        }
        self.pos = end;
        Ok(Some(Node { name, props, children }))
    }

    fn prop(&mut self) -> Result<Prop, String> {
        let t = self.u8()?;
        Ok(match t {
            b'C' => Prop::Bool(self.u8()? != 0),
            b'Y' => Prop::I16(i16::from_le_bytes(self.take(2)?.try_into().unwrap())),
            b'I' => Prop::I32(self.u32()? as i32),
            b'L' => Prop::I64(self.u64()? as i64),
            b'F' => Prop::F32(f32::from_bits(self.u32()?)),
            b'D' => Prop::F64(f64::from_bits(self.u64()?)),
            b'S' => {
                let n = self.u32()? as usize;
                Prop::Str(String::from_utf8_lossy(self.take(n)?).into_owned())
            }
            b'R' => {
                let n = self.u32()? as usize;
                Prop::Raw(self.take(n)?.to_vec())
            }
            b'b' | b'c' | b'i' | b'l' | b'f' | b'd' => {
                let len = self.u32()? as usize;
                let enc = self.u32()?;
                let clen = self.u32()? as usize;
                let elem = match t {
                    b'b' | b'c' => 1,
                    b'i' | b'f' => 4,
                    _ => 8,
                };
                if len > MAX_ARRAY {
                    return Err(format!("array of {len} elements is too large"));
                }
                let raw = self.take(clen)?;
                let data: Vec<u8> = match enc {
                    0 => raw.to_vec(),
                    1 => {
                        let mut out = Vec::with_capacity(len * elem);
                        flate2::read::ZlibDecoder::new(raw).take((len * elem) as u64 + 1).read_to_end(&mut out).map_err(|e| format!("bad compressed array: {e}"))?;
                        out
                    }
                    e => return Err(format!("unknown array encoding {e}")),
                };
                if data.len() != len * elem {
                    return Err(format!("array holds {} bytes, expected {}", data.len(), len * elem));
                }
                match t {
                    b'b' | b'c' => Prop::ArrBool(data.iter().map(|&x| x != 0).collect()),
                    b'i' => Prop::ArrI32(data.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect()),
                    b'l' => Prop::ArrI64(data.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect()),
                    b'f' => Prop::ArrF32(data.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()),
                    _ => Prop::ArrF64(data.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect()),
                }
            }
            t => return Err(format!("unknown property type {:?} at byte {}", t as char, self.pos - 1)),
        })
    }
}

fn parse_binary(bytes: &[u8]) -> Result<Document, String> {
    if bytes.len() < 27 {
        return Err("file too short".into());
    }
    let version = u32::from_le_bytes(bytes[23..27].try_into().unwrap());
    let mut r = Bin { b: bytes, pos: 27, wide: version >= 7500 };
    let mut nodes = Vec::new();
    while r.pos < bytes.len() {
        match r.node(0)? {
            Some(n) => nodes.push(n),
            None => break,
        }
    }
    Ok(Document { version, binary: true, nodes })
}

// ---------------------------------------------------------------- ASCII

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Key(String),
    Str(String),
    Word(String),
    Count(usize),
    Open,
    Close,
    Comma,
}

fn lex(text: &str) -> Result<Vec<(Tok, usize)>, String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                out.push((Tok::Open, line));
                i += 1;
            }
            b'}' => {
                out.push((Tok::Close, line));
                i += 1;
            }
            b',' => {
                out.push((Tok::Comma, line));
                i += 1;
            }
            b'"' => {
                let s = i + 1;
                i = s;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if i >= b.len() {
                    return Err(format!("line {line}: unterminated string"));
                }
                let raw = &text[s..i];
                out.push((Tok::Str(raw.replace("&quot;", "\"").replace("&cr;", "\r").replace("&lf;", "\n")), line));
                i += 1;
            }
            b'*' => {
                let s = i + 1;
                i = s;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let n = text[s..i].parse::<usize>().map_err(|_| format!("line {line}: bad array count"))?;
                out.push((Tok::Count(n), line));
            }
            _ => {
                let s = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b',' | b'{' | b'}' | b'"' | b';' | b':') {
                    i += 1;
                }
                if i == s {
                    return Err(format!("line {line}: unexpected character {:?}", c as char));
                }
                let w = text[s..i].to_string();
                if i < b.len() && b[i] == b':' {
                    i += 1;
                    out.push((Tok::Key(w), line));
                } else {
                    out.push((Tok::Word(w), line));
                }
            }
        }
    }
    Ok(out)
}

fn number(w: &str) -> Option<Prop> {
    if let Ok(v) = w.parse::<i64>() {
        return Some(Prop::I64(v));
    }
    w.parse::<f64>().ok().map(Prop::F64)
}

struct Ascii {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

impl Ascii {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }
    fn line(&self) -> usize {
        self.toks.get(self.pos.min(self.toks.len().saturating_sub(1))).map_or(0, |t| t.1)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.0.clone());
        self.pos += 1;
        t
    }
    fn err(&self, m: &str) -> String {
        format!("line {}: {m}", self.line())
    }

    fn value(&mut self) -> Result<Prop, String> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(Prop::Str(s)),
            Some(Tok::Word(w)) => Ok(number(&w).unwrap_or(Prop::Str(w))),
            Some(Tok::Count(n)) => {
                if self.next() != Some(Tok::Open) {
                    return Err(self.err("expected { after array count"));
                }
                if !matches!(self.next(), Some(Tok::Key(_))) {
                    return Err(self.err("expected a: in array"));
                }
                let mut ints = Vec::with_capacity(n.min(MAX_ARRAY));
                let mut floats: Option<Vec<f64>> = None;
                loop {
                    match self.next() {
                        Some(Tok::Word(w)) => match number(&w) {
                            Some(Prop::I64(v)) => match &mut floats {
                                Some(f) => f.push(v as f64),
                                None => ints.push(v),
                            },
                            Some(Prop::F64(v)) => {
                                let f = floats.get_or_insert_with(|| ints.iter().map(|&i| i as f64).collect());
                                f.push(v);
                            }
                            _ => return Err(self.err(&format!("bad number {w:?} in array"))),
                        },
                        Some(Tok::Comma) => {}
                        Some(Tok::Close) => break,
                        _ => return Err(self.err("unterminated array")),
                    }
                }
                Ok(match floats {
                    Some(f) => Prop::ArrF64(f),
                    None => Prop::ArrI64(ints),
                })
            }
            _ => Err(self.err("expected a value")),
        }
    }

    /// Nodes until `}` (when `nested`) or the end.
    fn nodes(&mut self, nested: bool, depth: usize) -> Result<Vec<Node>, String> {
        if depth > 64 {
            return Err(self.err("nodes nested too deeply"));
        }
        let mut out = Vec::new();
        loop {
            match self.next() {
                None if !nested => return Ok(out),
                None => return Err("unexpected end of file (missing })".into()),
                Some(Tok::Close) if nested => return Ok(out),
                Some(Tok::Key(name)) => {
                    let mut node = Node { name, ..Default::default() };
                    if matches!(self.peek(), Some(Tok::Str(_) | Tok::Word(_) | Tok::Count(_))) {
                        node.props.push(self.value()?);
                        while self.peek() == Some(&Tok::Comma) {
                            self.pos += 1;
                            // Trailing commas on property lists happen.
                            if matches!(self.peek(), Some(Tok::Str(_) | Tok::Word(_) | Tok::Count(_))) {
                                node.props.push(self.value()?);
                            }
                        }
                    }
                    if self.peek() == Some(&Tok::Open) {
                        self.pos += 1;
                        node.children = self.nodes(true, depth + 1)?;
                    }
                    out.push(node);
                }
                Some(t) => return Err(self.err(&format!("unexpected {t:?}"))),
            }
        }
    }
}

fn parse_ascii(bytes: &[u8]) -> Result<Document, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "neither a binary FBX nor UTF-8 text".to_string())?;
    let mut p = Ascii { toks: lex(text)?, pos: 0 };
    let nodes = p.nodes(false, 0)?;
    let version = nodes.iter().find(|n| n.name == "FBXHeaderExtension").and_then(|h| h.child("FBXVersion")).and_then(|v| v.i64(0)).unwrap_or(7400) as u32;
    if nodes.is_empty() {
        return Err("no FBX nodes".into());
    }
    Ok(Document { version, binary: false, nodes })
}

// ---------------------------------------------------------------- writer

/// Arrays at least this long (bytes) are zlib-compressed, like Blender does.
const COMPRESS_MIN: usize = 128;

fn write_prop(out: &mut Vec<u8>, p: &Prop) {
    fn arr(out: &mut Vec<u8>, t: u8, n: usize, data: Vec<u8>) {
        out.push(t);
        out.extend((n as u32).to_le_bytes());
        if data.len() >= COMPRESS_MIN {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(&data).expect("in-memory zlib");
            let c = z.finish().expect("in-memory zlib");
            out.extend(1u32.to_le_bytes());
            out.extend((c.len() as u32).to_le_bytes());
            out.extend(c);
        } else {
            out.extend(0u32.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        }
    }
    match p {
        Prop::Bool(b) => out.extend([b'C', *b as u8]),
        Prop::I16(v) => {
            out.push(b'Y');
            out.extend(v.to_le_bytes());
        }
        Prop::I32(v) => {
            out.push(b'I');
            out.extend(v.to_le_bytes());
        }
        Prop::I64(v) => {
            out.push(b'L');
            out.extend(v.to_le_bytes());
        }
        Prop::F32(v) => {
            out.push(b'F');
            out.extend(v.to_le_bytes());
        }
        Prop::F64(v) => {
            out.push(b'D');
            out.extend(v.to_le_bytes());
        }
        Prop::Str(s) => {
            out.push(b'S');
            out.extend((s.len() as u32).to_le_bytes());
            out.extend(s.as_bytes());
        }
        Prop::Raw(r) => {
            out.push(b'R');
            out.extend((r.len() as u32).to_le_bytes());
            out.extend(r);
        }
        Prop::ArrBool(v) => arr(out, b'b', v.len(), v.iter().map(|&x| x as u8).collect()),
        Prop::ArrI32(v) => arr(out, b'i', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Prop::ArrI64(v) => arr(out, b'l', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Prop::ArrF32(v) => arr(out, b'f', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Prop::ArrF64(v) => arr(out, b'd', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
    }
}

fn write_node(out: &mut Vec<u8>, n: &Node) {
    let start = out.len();
    out.extend([0u8; 12]);
    out.push(n.name.len() as u8);
    out.extend(n.name.as_bytes());
    let pstart = out.len();
    for p in &n.props {
        write_prop(out, p);
    }
    let plen = out.len() - pstart;
    // Blender and the FBX SDK end every node with children, or without
    // properties, with a null record.
    if !n.children.is_empty() || n.props.is_empty() {
        for c in &n.children {
            write_node(out, c);
        }
        out.extend([0u8; 13]);
    }
    let end = out.len() as u32;
    out[start..start + 4].copy_from_slice(&end.to_le_bytes());
    out[start + 4..start + 8].copy_from_slice(&(n.props.len() as u32).to_le_bytes());
    out[start + 8..start + 12].copy_from_slice(&(plen as u32).to_le_bytes());
}

/// The 16-byte file id and footer ids every FBX SDK writer emits (fixed values,
/// matching the fixed `CreationTime` the writer uses).
const FILE_ID: [u8; 16] = [0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1];
const FOOT_ID: [u8; 16] = [0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e];
const FOOT_MAGIC: [u8; 16] = [0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b];

pub fn file_id() -> Prop {
    Prop::Raw(FILE_ID.to_vec())
}

/// A binary FBX 7.4 file (32-bit offsets) holding `nodes`.
pub fn write_binary(nodes: &[Node]) -> Vec<u8> {
    let version = 7400u32;
    let mut out = Vec::new();
    out.extend(BINARY_MAGIC);
    out.extend(version.to_le_bytes());
    for n in nodes {
        write_node(&mut out, n);
    }
    out.extend([0u8; 13]);
    out.extend(FOOT_ID);
    out.extend([0u8; 4]);
    let pad = (16 - out.len() % 16) % 16;
    out.extend(std::iter::repeat_n(0u8, if pad == 0 { 16 } else { pad }));
    out.extend(version.to_le_bytes());
    out.extend([0u8; 120]);
    out.extend(FOOT_MAGIC);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_round_trip_with_compressed_arrays() {
        let big: Vec<f64> = (0..500).map(|i| i as f64 * 0.5).collect();
        let tree = vec![Node::new("Objects", vec![]).with(Node::new("Geometry", vec![Prop::I64(7), Prop::Str("Cube\0\u{1}Geometry".into()), Prop::Str("Mesh".into())]).with(Node::new("Vertices", vec![Prop::ArrF64(big.clone())])).with(Node::new("PolygonVertexIndex", vec![Prop::ArrI32(vec![0, 1, -3])])))];
        let bytes = write_binary(&tree);
        let doc = parse(&bytes).unwrap();
        assert!(doc.binary);
        assert_eq!(doc.version, 7400);
        let g = doc.get("Objects").unwrap().child("Geometry").unwrap();
        assert_eq!(object_name(g.str(1).unwrap()), "Cube");
        assert_eq!(g.child_f64s("Vertices").unwrap(), big);
        assert_eq!(g.child_i64s("PolygonVertexIndex").unwrap(), vec![0, 1, -3]);
    }

    #[test]
    fn ascii_tree() {
        let text = r#"; FBX 7.3.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7300
}
Objects:  {
	Geometry: 123, "Geometry::Box", "Mesh" {
		Vertices: *6 {
			a: 0,0,0,1.5,
			2,-3e-1
		}
		PolygonVertexIndex: *3 {
			a: 0,1,-3
		}
		Shading: T
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",1,2,3
		}
	}
}
"#;
        let doc = parse(text.as_bytes()).unwrap();
        assert_eq!(doc.version, 7300);
        let g = doc.get("Objects").unwrap().child("Geometry").unwrap();
        assert_eq!(g.i64(0), Some(123));
        assert_eq!(object_name(g.str(1).unwrap()), "Box");
        assert_eq!(g.child_f64s("Vertices").unwrap(), vec![0.0, 0.0, 0.0, 1.5, 2.0, -0.3]);
        assert_eq!(g.child_i64s("PolygonVertexIndex").unwrap(), vec![0, 1, -3]);
        assert_eq!(g.child_str("Shading"), Some("T"));
        let p = g.child("Properties70").unwrap().child("P").unwrap();
        assert_eq!(p.props.len(), 7);
        assert_eq!(p.props[6].as_f64(), Some(3.0));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"Kaydara FBX Binary  \0\x1a\0\xe8\x1c\0\0\xff\xff\xff\x7f\0\0\0\0").is_err());
        assert!(parse(&[0xff, 0xfe, 0x00]).is_err());
    }
}
