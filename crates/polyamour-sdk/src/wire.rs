// SPDX-License-Identifier: GPL-3.0-only
//! Pure-Rust `#![no_std]` `polyops 1` wire encoder and decoder, replacing
//! `apps/polyamour/python/polyamour/polyops.py`.

use crate::math::{Vec2, Vec3};
use alloc::{
    collections::BTreeSet,
    format,
    string::{String, ToString},
    vec::Vec,
};
use base64::Engine;
use core::fmt::Write as _;

pub const VERSION: u32 = 1;
pub const BASE64_ABOVE: usize = 64;

/// Decoded mesh geometry and attributes transferred across `polyops 1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    pub faces: Vec<Vec<u32>>,
    pub loose_edges: Vec<[u32; 2]>,
    pub seams: Vec<[u32; 2]>,
    pub sharp: Vec<[u32; 2]>,
    /// Per-face-corner UVs (empty if unmapped).
    pub uvs: Vec<Vec<Vec2>>,
    /// Per-face smooth shading flags (empty defaults to flat).
    pub smooth: Vec<bool>,
    /// Per-face material slot indices (empty defaults to slot 0).
    pub materials: Vec<u16>,
}

impl MeshData {
    /// Build a `MeshData` from Blender-style `from_pydata(verts, edges, faces)` arguments,
    /// dropping edges already belonging to a polygon (matching `polyops.py`).
    pub fn from_pydata(verts: &[Vec3], edges: &[[u32; 2]], faces: &[Vec<u32>]) -> Result<Self, String> {
        let n = verts.len() as u32;
        let mut used = BTreeSet::new();
        for f in faces {
            if f.len() < 3 {
                return Err(format!("a face needs at least 3 vertices, got {}", f.len()));
            }
            for (idx, &v) in f.iter().enumerate() {
                if v >= n {
                    return Err(format!("face uses vertex {v} outside 0..{}", n.saturating_sub(1)));
                }
                let next = f[(idx + 1) % f.len()];
                used.insert(if v < next { (v, next) } else { (next, v) });
            }
        }
        let mut loose_edges = Vec::new();
        for &[a, b] in edges {
            if a >= n || b >= n {
                return Err(format!("edge ({a}, {b}) uses a vertex outside 0..{}", n.saturating_sub(1)));
            }
            let key = if a < b { (a, b) } else { (b, a) };
            if a != b && !used.contains(&key) {
                loose_edges.push([a, b]);
            }
        }
        Ok(Self {
            positions: verts.to_vec(),
            faces: faces.to_vec(),
            loose_edges,
            seams: Vec::new(),
            sharp: Vec::new(),
            uvs: Vec::new(),
            smooth: Vec::new(),
            materials: Vec::new(),
        })
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
}

/// Quote `s` as one `polyops` token when it is not a safe bare word.
pub fn quote(s: &str) -> String {
    let bare = !s.is_empty()
        && s != "none"
        && !s.starts_with(['$', '#', '@'])
        && s.chars().all(|c| c.is_alphanumeric() || "_-.+:/!?%&*<>^~'[]()".contains(c));
    if bare {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub fn fmt_f32(v: f32) -> Result<String, String> {
    if !v.is_finite() {
        return Err(format!("polyops numbers must be finite, got {v}"));
    }
    Ok(format!("{}", v + 0.0))
}

pub fn fmt_f32_list(values: &[f32]) -> Result<String, String> {
    let mut parts = Vec::with_capacity(values.len());
    for &v in values {
        parts.push(fmt_f32(v)?);
    }
    Ok(parts.join(","))
}

pub fn b64_floats(values: &[f32]) -> Result<String, String> {
    if values.iter().any(|v| !v.is_finite()) {
        return Err("polyops base64 float array contains a non-finite value".into());
    }
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    Ok(format!("b64:{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub fn b64_uints(values: &[u32]) -> String {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    format!("b64:{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Encode [`MeshData`] as `verts=... [faces=...] [edges=...] [uvs=...] [smooth=...] [mat=...]`
/// switching automatically to binary little-endian base64 above 64 vertices.
pub fn encode_mesh_options(mesh: &MeshData) -> Result<String, String> {
    let b64 = mesh.positions.len() > BASE64_ABOVE;
    let mut out = String::new();
    let flat_pos: Vec<f32> = mesh.positions.iter().flat_map(|p| p.to_array()).collect();
    let verts = if b64 { b64_floats(&flat_pos)? } else { fmt_f32_list(&flat_pos)? };
    let _ = write!(out, "verts={verts}");

    if !mesh.faces.is_empty() {
        let faces = if b64 {
            let mut runs = Vec::new();
            for f in &mesh.faces {
                runs.push(f.len() as u32);
                runs.extend_from_slice(f);
            }
            b64_uints(&runs)
        } else {
            mesh.faces
                .iter()
                .map(|f| f.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","))
                .collect::<Vec<_>>()
                .join(";")
        };
        let _ = write!(out, " faces={faces}");
    }

    if !mesh.loose_edges.is_empty() {
        let edges = if b64 {
            let flat: Vec<u32> = mesh.loose_edges.iter().flat_map(|e| e.iter().copied()).collect();
            b64_uints(&flat)
        } else {
            mesh.loose_edges
                .iter()
                .map(|[a, b]| format!("{a},{b}"))
                .collect::<Vec<_>>()
                .join(";")
        };
        let _ = write!(out, " edges={edges}");
    }

    if !mesh.uvs.is_empty() && mesh.uvs.len() == mesh.faces.len() {
        let flat_uvs: Vec<f32> = mesh
            .uvs
            .iter()
            .flat_map(|f| f.iter().flat_map(|u| u.to_array()))
            .collect();
        let uvs = if b64 { b64_floats(&flat_uvs)? } else { fmt_f32_list(&flat_uvs)? };
        let _ = write!(out, " uvs={uvs}");
    }

    if mesh.smooth.iter().any(|&s| s) {
        if mesh.smooth.len() == mesh.faces.len() && mesh.smooth.iter().all(|&s| s) {
            out.push_str(" smooth=1");
        } else if mesh.smooth.len() == mesh.faces.len() {
            let bits: String = mesh.smooth.iter().map(|&s| if s { '1' } else { '0' }).collect();
            let _ = write!(out, " smooth={bits}");
        }
    }

    if mesh.materials.iter().any(|&m| m != 0) && mesh.materials.len() == mesh.faces.len() {
        let slots = mesh.materials.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(",");
        let _ = write!(out, " mat={slots}");
    }

    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub key: Option<String>,
    pub text: String,
    pub quoted: bool,
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn tokenize(line: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut it = line.chars().peekable();
    loop {
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
        match it.peek() {
            None | Some('#') => break,
            _ => {}
        }
        let mut tok = Token { key: None, text: String::new(), quoted: false };
        while let Some(&c) = it.peek() {
            if c.is_whitespace() {
                break;
            }
            it.next();
            match c {
                '"' => {
                    tok.quoted = true;
                    loop {
                        match it.next() {
                            None => return Err("unterminated quoted string".into()),
                            Some('"') => break,
                            Some('\\') => match it.next() {
                                Some('"') => tok.text.push('"'),
                                Some('\\') => tok.text.push('\\'),
                                Some('n') => tok.text.push('\n'),
                                Some('t') => tok.text.push('\t'),
                                Some('r') => tok.text.push('\r'),
                                Some(e) => return Err(format!("unknown escape \\{e}")),
                                None => return Err("unterminated quoted string".into()),
                            },
                            Some(ch) => tok.text.push(ch),
                        }
                    }
                }
                '=' if tok.key.is_none() && !tok.quoted && is_ident(&tok.text) => {
                    tok.key = Some(core::mem::take(&mut tok.text));
                }
                ch => tok.text.push(ch),
            }
        }
        out.push(tok);
    }
    Ok(out)
}

pub fn parse_floats(text: &str) -> Result<Vec<f32>, String> {
    if let Some(b64) = text.strip_prefix("b64:") {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("bad base64: {e}"))?;
        if raw.len() % 4 != 0 {
            return Err("base64 float buffer is not a multiple of 4 bytes".into());
        }
        let vals: Vec<f32> = raw
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if vals.iter().any(|x| !x.is_finite()) {
            return Err("non-finite float in base64 array".into());
        }
        return Ok(vals);
    }
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|s| {
            s.trim()
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("'{s}' is not a finite float"))
        })
        .collect()
}

pub fn parse_uints(text: &str) -> Result<Vec<u32>, String> {
    if let Some(b64) = text.strip_prefix("b64:") {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("bad base64: {e}"))?;
        if raw.len() % 4 != 0 {
            return Err("base64 u32 buffer is not a multiple of 4 bytes".into());
        }
        return Ok(raw
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect());
    }
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|s| s.trim().parse::<u32>().map_err(|_| format!("'{s}' is not a u32")))
        .collect()
}

pub fn parse_polygons(text: &str) -> Result<Vec<Vec<u32>>, String> {
    if text.starts_with("b64:") {
        let flat = parse_uints(text)?;
        let mut out = Vec::new();
        let mut i = 0;
        while i < flat.len() {
            let len = flat[i] as usize;
            let end = i + 1 + len;
            if end > flat.len() {
                return Err("truncated base64 polygon run".into());
            }
            out.push(flat[i + 1..end].to_vec());
            i = end;
        }
        return Ok(out);
    }
    text.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(parse_uints)
        .collect()
}

/// Decode a `mesh_get` / `mesh_to_options` option string back into [`MeshData`].
pub fn decode_mesh_options(text: &str) -> Result<MeshData, String> {
    let toks = tokenize(text)?;
    let mut mesh = MeshData::default();
    let mut raw_uvs: Option<String> = None;
    let mut raw_smooth: Option<String> = None;
    let mut raw_mat: Option<String> = None;

    for tok in toks {
        let Some(key) = tok.key else {
            return Err(format!("unexpected positional token '{}' in mesh options", tok.text));
        };
        match key.as_str() {
            "verts" => {
                let vals = parse_floats(&tok.text)?;
                if vals.len() % 3 != 0 {
                    return Err("verts length is not a multiple of 3".into());
                }
                mesh.positions = vals.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
            }
            "faces" => {
                mesh.faces = parse_polygons(&tok.text)?;
            }
            "edges" | "seams" | "sharp" => {
                let flat = if tok.text.starts_with("b64:") {
                    parse_uints(&tok.text)?
                } else {
                    let mut out = Vec::new();
                    for pair in tok.text.split(';').filter(|p| !p.trim().is_empty()) {
                        out.extend(parse_uints(pair)?);
                    }
                    out
                };
                if flat.len() % 2 != 0 {
                    return Err(format!("{key} length is not a multiple of 2"));
                }
                let pairs: Vec<[u32; 2]> = flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
                match key.as_str() {
                    "edges" => mesh.loose_edges = pairs,
                    "seams" => mesh.seams = pairs,
                    _ => mesh.sharp = pairs,
                }
            }
            "uvs" => raw_uvs = Some(tok.text),
            "smooth" => raw_smooth = Some(tok.text),
            "mat" => raw_mat = Some(tok.text),
            other => return Err(format!("unknown mesh option '{other}'")),
        }
    }

    if let Some(uv_str) = raw_uvs {
        let uv_vals = parse_floats(&uv_str)?;
        let corners: usize = mesh.faces.iter().map(|f| f.len()).sum();
        if uv_vals.len() != corners * 2 {
            return Err(format!("uvs holds {} floats for {corners} corners", uv_vals.len()));
        }
        let mut k = 0;
        for f in &mesh.faces {
            let mut face_uvs = Vec::with_capacity(f.len());
            for _ in 0..f.len() {
                face_uvs.push(Vec2::new(uv_vals[k], uv_vals[k + 1]));
                k += 2;
            }
            mesh.uvs.push(face_uvs);
        }
    }

    if let Some(s_str) = raw_smooth {
        let s = s_str.trim();
        if s == "1" || s.eq_ignore_ascii_case("true") {
            mesh.smooth = alloc::vec![true; mesh.faces.len()];
        } else if s == "0" || s.eq_ignore_ascii_case("false") {
            mesh.smooth = alloc::vec![false; mesh.faces.len()];
        } else if s.len() == mesh.faces.len() && s.bytes().all(|b| b == b'0' || b == b'1') {
            mesh.smooth = s.bytes().map(|b| b == b'1').collect();
        } else {
            return Err(format!("invalid smooth specification '{s}'"));
        }
    }

    if let Some(m_str) = raw_mat {
        let slots = parse_uints(&m_str)?;
        if slots.len() != mesh.faces.len() {
            return Err("mat slot count does not match face count".into());
        }
        mesh.materials = slots
            .into_iter()
            .map(|v| u16::try_from(v).map_err(|_| "material slot exceeds u16".to_string()))
            .collect::<Result<Vec<_>, _>>()?;
    }

    Ok(mesh)
}

/// Extract the payload from a `@ID ok ...` reply line.
pub fn parse_tagged_reply<'a>(line: &'a str, expected_id: &str) -> Result<&'a str, String> {
    let trimmed = line.trim();
    let tag = format!("@{expected_id}");
    let rest = trimmed
        .strip_prefix(&tag)
        .ok_or_else(|| format!("expected reply for {tag}, got '{trimmed}'"))?
        .trim_start();
    if rest == "ok" {
        Ok("")
    } else if let Some(payload) = rest.strip_prefix("ok ") {
        Ok(payload.trim())
    } else {
        Err(format!("command {tag} failed: {rest}"))
    }
}
