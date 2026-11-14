// SPDX-License-Identifier: GPL-3.0-only
//! PLY (Stanford polygon files): `ascii`, `binary_little_endian` and
//! `binary_big_endian`.
//!
//! Import reads the `vertex` element's `x y z` and texture coordinates
//! (`s t`, `u v` or `texture_u texture_v`), the `face` element's index list
//! (`vertex_indices` or `vertex_index`, any count/index types) with optional
//! per-corner `texcoord` lists, and `edge` elements as loose edges. Other
//! properties (normals, colours) and elements are skipped. No axis conversion
//! is applied (like Blender's default).
//!
//! Export merges every mesh object in world space into one mesh and writes
//! positions, n-gon faces, per-corner `texcoord` lists (when there are UVs)
//! and loose edges, binary little-endian by default.
use super::{world_meshes, Format, IoError};
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::scene::Scene;

const F: Format = Format::Ply;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Option<Scalar> {
        Some(match s {
            "char" | "int8" => Scalar::I8,
            "uchar" | "uint8" => Scalar::U8,
            "short" | "int16" => Scalar::I16,
            "ushort" | "uint16" => Scalar::U16,
            "int" | "int32" => Scalar::I32,
            "uint" | "uint32" => Scalar::U32,
            "float" | "float32" => Scalar::F32,
            "double" | "float64" => Scalar::F64,
            _ => return None,
        })
    }
    fn size(self) -> usize {
        match self {
            Scalar::I8 | Scalar::U8 => 1,
            Scalar::I16 | Scalar::U16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::F64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Scalar(Scalar),
    List { count: Scalar, item: Scalar },
}

#[derive(Debug)]
struct Element {
    name: String,
    count: usize,
    props: Vec<(String, Kind)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Ascii,
    Little,
    Big,
}

/// Reads scalars from the body in the file's encoding.
struct Body<'a> {
    enc: Encoding,
    bytes: &'a [u8],
    pos: usize,
    /// Current line (ASCII only), 1-based in the whole file.
    line: usize,
}

impl<'a> Body<'a> {
    fn err(&self, msg: impl Into<String>) -> IoError {
        IoError::parse(F, (self.enc == Encoding::Ascii).then_some(self.line), msg)
    }

    /// Next whitespace-separated token (ASCII bodies).
    fn token(&mut self) -> Result<&'a str, IoError> {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            if self.bytes[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        let start = self.pos;
        while self.pos < self.bytes.len() && !self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(self.err("unexpected end of file"));
        }
        std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| self.err("invalid text"))
    }

    fn read(&mut self, t: Scalar) -> Result<f64, IoError> {
        if self.enc == Encoding::Ascii {
            let tok = self.token()?;
            return tok.parse::<f64>().map_err(|_| self.err(format!("invalid number \"{tok}\"")));
        }
        let n = t.size();
        let Some(raw) = self.bytes.get(self.pos..self.pos + n) else {
            return Err(self.err(format!("unexpected end of data at byte {}", self.pos)));
        };
        self.pos += n;
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(raw);
        if self.enc == Encoding::Big {
            b[..n].reverse();
        }
        Ok(match t {
            Scalar::I8 => b[0] as i8 as f64,
            Scalar::U8 => b[0] as f64,
            Scalar::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            Scalar::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            Scalar::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Scalar::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Scalar::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Scalar::F64 => f64::from_le_bytes(b),
        })
    }

    /// Read a list's length, checked against the bytes left.
    fn list_len(&mut self, count: Scalar, item: Scalar) -> Result<usize, IoError> {
        let n = self.read(count)?;
        let left = self.bytes.len().saturating_sub(self.pos);
        let max = if self.enc == Encoding::Ascii { left / 2 + 1 } else { left / item.size() };
        if !(0.0..=max as f64).contains(&n) || n.fract() != 0.0 {
            return Err(self.err(format!("invalid list length {n}")));
        }
        Ok(n as usize)
    }

    fn skip(&mut self, kind: Kind) -> Result<(), IoError> {
        match kind {
            Kind::Scalar(t) => {
                self.read(t)?;
            }
            Kind::List { count, item } => {
                let n = self.list_len(count, item)?;
                for _ in 0..n {
                    self.read(item)?;
                }
            }
        }
        Ok(())
    }
}

/// Parse the header; returns the elements, the encoding, the body's byte
/// offset and the body's first line number.
fn parse_header(bytes: &[u8]) -> Result<(Vec<Element>, Encoding, usize, usize), IoError> {
    if !bytes.starts_with(b"ply") {
        return Err(IoError::at_line(F, 1, "missing \"ply\" magic"));
    }
    let mut elements: Vec<Element> = Vec::new();
    let mut enc = None;
    let mut pos = 0;
    let mut line = 0;
    loop {
        line += 1;
        let Some(nl) = bytes[pos..].iter().position(|&c| c == b'\n') else {
            return Err(IoError::at_line(F, line, "header has no \"end_header\""));
        };
        let text = String::from_utf8_lossy(&bytes[pos..pos + nl]);
        pos += nl + 1;
        let mut toks = text.split_whitespace();
        let err = |m: String| IoError::at_line(F, line, m);
        match toks.next() {
            Some("end_header") => break,
            Some("format") => {
                enc = Some(match toks.next() {
                    Some("ascii") => Encoding::Ascii,
                    Some("binary_little_endian") => Encoding::Little,
                    Some("binary_big_endian") => Encoding::Big,
                    other => return Err(err(format!("unknown format \"{}\"", other.unwrap_or("")))),
                })
            }
            Some("element") => {
                let name = toks.next().ok_or_else(|| err("element without a name".into()))?;
                let count = toks.next().and_then(|c| c.parse().ok()).ok_or_else(|| err(format!("element \"{name}\" has no valid count")))?;
                elements.push(Element { name: name.into(), count, props: Vec::new() });
            }
            Some("property") => {
                let el = elements.last_mut().ok_or_else(|| err("property before any element".into()))?;
                let t = toks.next().unwrap_or("");
                let kind = if t == "list" {
                    let (c, i) = (toks.next().unwrap_or(""), toks.next().unwrap_or(""));
                    match (Scalar::parse(c), Scalar::parse(i)) {
                        (Some(count), Some(item)) => Kind::List { count, item },
                        _ => return Err(err(format!("unknown list types \"{c} {i}\""))),
                    }
                } else {
                    Kind::Scalar(Scalar::parse(t).ok_or_else(|| err(format!("unknown property type \"{t}\"")))?)
                };
                let name = toks.next().ok_or_else(|| err("property without a name".into()))?;
                el.props.push((name.into(), kind));
            }
            Some("ply" | "comment" | "obj_info") | None => {}
            Some(other) => return Err(err(format!("unexpected header keyword \"{other}\""))),
        }
    }
    let enc = enc.ok_or_else(|| IoError::parse(F, None, "header has no \"format\" line"))?;
    Ok((elements, enc, pos, line + 1))
}

/// Import a PLY file as one mesh object.
pub fn import_into(scene: &mut Scene, bytes: &[u8]) -> Result<Vec<usize>, IoError> {
    let (elements, enc, start, line) = parse_header(bytes)?;
    let mut body = Body { enc, bytes, pos: start, line };
    let vertex_count = elements.iter().find(|e| e.name == "vertex").map_or(0, |e| e.count);
    let mut mesh = Mesh::new();
    let mut vertex_uvs: Vec<Vec2> = Vec::new();
    let mut corner_uvs = false;
    let check = |body: &Body, v: f64| -> Result<u32, IoError> {
        if v < 0.0 || v >= vertex_count as f64 || v.fract() != 0.0 {
            return Err(body.err(format!("vertex index {v} out of range ({vertex_count} vertices)")));
        }
        Ok(v as u32)
    };
    for el in &elements {
        let find = |names: &[&str]| el.props.iter().position(|(n, k)| names.contains(&n.as_str()) && matches!(k, Kind::Scalar(_)));
        match el.name.as_str() {
            "vertex" => {
                let xyz = [find(&["x"]), find(&["y"]), find(&["z"])];
                let uv = [find(&["s", "u", "texture_u", "texture_s"]), find(&["t", "v", "texture_v", "texture_t"])];
                let has_uv = uv[0].is_some() && uv[1].is_some();
                mesh.positions.reserve(el.count);
                let mut row = vec![0.0f64; el.props.len()];
                for _ in 0..el.count {
                    for (i, (_, k)) in el.props.iter().enumerate() {
                        match *k {
                            Kind::Scalar(t) => row[i] = body.read(t)?,
                            list => body.skip(list)?,
                        }
                    }
                    let c = |p: Option<usize>| p.map_or(0.0, |i| row[i] as f32);
                    mesh.positions.push(Vec3::new(c(xyz[0]), c(xyz[1]), c(xyz[2])));
                    if has_uv {
                        vertex_uvs.push(Vec2::new(c(uv[0]), c(uv[1])));
                    }
                }
            }
            "face" => {
                mesh.faces.reserve(el.count);
                for _ in 0..el.count {
                    let mut face = Face::default();
                    for (name, k) in &el.props {
                        match (name.as_str(), *k) {
                            ("vertex_indices" | "vertex_index", Kind::List { count, item }) => {
                                let n = body.list_len(count, item)?;
                                face.verts.reserve(n);
                                for _ in 0..n {
                                    let v = body.read(item)?;
                                    face.verts.push(check(&body, v)?);
                                }
                            }
                            ("texcoord", Kind::List { count, item }) => {
                                let n = body.list_len(count, item)?;
                                let mut vals = Vec::with_capacity(n);
                                for _ in 0..n {
                                    vals.push(body.read(item)? as f32);
                                }
                                face.uvs = vals.chunks_exact(2).map(|c| Vec2::new(c[0], c[1])).collect();
                                corner_uvs = true;
                            }
                            (_, kind) => body.skip(kind)?,
                        }
                    }
                    if face.uvs.len() != face.verts.len() {
                        face.uvs.clear();
                    }
                    if face.verts.len() >= 3 {
                        mesh.faces.push(face);
                    }
                }
            }
            "edge" => {
                let ends = [find(&["vertex1"]), find(&["vertex2"])];
                let mut row = vec![0.0f64; el.props.len()];
                for _ in 0..el.count {
                    for (i, (_, k)) in el.props.iter().enumerate() {
                        match *k {
                            Kind::Scalar(t) => row[i] = body.read(t)?,
                            list => body.skip(list)?,
                        }
                    }
                    if let [Some(a), Some(b)] = ends {
                        let e = [check(&body, row[a])?, check(&body, row[b])?];
                        if e[0] != e[1] {
                            mesh.loose_edges.push(e);
                        }
                    }
                }
            }
            _ => {
                for _ in 0..el.count {
                    for &(_, k) in &el.props {
                        body.skip(k)?;
                    }
                }
            }
        }
    }
    // Per-vertex texture coordinates become per-corner UVs.
    if !corner_uvs && vertex_uvs.len() == mesh.positions.len() && !vertex_uvs.is_empty() {
        for f in &mut mesh.faces {
            f.uvs = f.verts.iter().map(|&v| vertex_uvs[v as usize]).collect();
        }
    }
    Ok(vec![scene.add_mesh_object("PLY", mesh)])
}

/// Writes values as text or little-endian bytes.
struct Out {
    ascii: bool,
    buf: Vec<u8>,
}

impl Out {
    fn f32(&mut self, v: f32) {
        if self.ascii {
            self.buf.extend_from_slice(format!("{} ", if v == 0.0 { 0.0 } else { v }).as_bytes());
        } else {
            self.buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    fn u32(&mut self, v: u32) {
        if self.ascii {
            self.buf.extend_from_slice(format!("{v} ").as_bytes());
        } else {
            self.buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    /// A list length: one byte, or four when `wide`.
    fn len(&mut self, n: usize, wide: bool) {
        if self.ascii || wide {
            self.u32(n as u32);
        } else {
            self.buf.push(n as u8);
        }
    }
    fn end_row(&mut self) {
        if self.ascii {
            if self.buf.last() == Some(&b' ') {
                self.buf.pop();
            }
            self.buf.push(b'\n');
        }
    }
}

fn export(scene: &Scene, ascii: bool) -> Vec<u8> {
    let mut m = Mesh::new();
    for w in world_meshes(scene) {
        m.append(&w.mesh);
    }
    let any_uv = m.faces.iter().any(|f| !f.uvs.is_empty());
    let wide = m.faces.iter().any(|f| f.verts.len() > 255);
    let count_t = if wide { "uint" } else { "uchar" };
    let mut header = format!("ply\nformat {} 1.0\ncomment Polyamour\n", if ascii { "ascii" } else { "binary_little_endian" });
    header += &format!("element vertex {}\nproperty float x\nproperty float y\nproperty float z\n", m.positions.len());
    header += &format!("element face {}\nproperty list {count_t} uint vertex_indices\n", m.faces.len());
    if any_uv {
        header += &format!("property list {count_t} float texcoord\n");
    }
    if !m.loose_edges.is_empty() {
        header += &format!("element edge {}\nproperty uint vertex1\nproperty uint vertex2\n", m.loose_edges.len());
    }
    header += "end_header\n";
    let mut o = Out { ascii, buf: header.into_bytes() };
    for p in &m.positions {
        o.f32(p.x);
        o.f32(p.y);
        o.f32(p.z);
        o.end_row();
    }
    for f in &m.faces {
        o.len(f.verts.len(), wide);
        for &v in &f.verts {
            o.u32(v);
        }
        if any_uv {
            o.len(f.uvs.len() * 2, wide);
            for uv in &f.uvs {
                o.f32(uv.x);
                o.f32(uv.y);
            }
        }
        o.end_row();
    }
    for e in &m.loose_edges {
        o.u32(e[0]);
        o.u32(e[1]);
        o.end_row();
    }
    o.buf
}

/// Binary little-endian PLY of every mesh object, merged, in world space.
pub fn export_binary(scene: &Scene) -> Vec<u8> {
    export(scene, false)
}

/// ASCII PLY of every mesh object, merged, in world space.
pub fn export_ascii(scene: &Scene) -> Vec<u8> {
    export(scene, true)
}
