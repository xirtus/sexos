// SPDX-License-Identifier: GPL-3.0-only
//! STL, ASCII and binary.
//!
//! Binary files are recognised by their size (84 + 50 bytes per triangle), not
//! by the "solid" prefix, since many binary writers start their header with it.
//! STL is a triangle soup, so import welds bit-identical positions into a
//! connected mesh. Each ASCII `solid` becomes its own object. No axis
//! conversion is applied (like Blender's default). Export writes every mesh
//! object in world space, triangulated.
use super::{parse_floats, weld_exact, world_meshes, Format, IoError};
use crate::math::Vec3;
use crate::mesh::{Face, Mesh};
use crate::scene::Scene;
use std::fmt::Write as _;

const F: Format = Format::Stl;

/// True when `bytes` has exactly the size a binary STL with its declared
/// triangle count has.
pub fn is_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as u64;
    84 + 50 * n == bytes.len() as u64
}

/// Import an STL file (ASCII or binary).
pub fn import_into(scene: &mut Scene, bytes: &[u8]) -> Result<Vec<usize>, IoError> {
    let solids = if is_binary(bytes) {
        vec![read_binary(bytes)]
    } else if super::trim_ascii_start(bytes).starts_with(b"solid") && !bytes.iter().take(1024).any(|&b| b == 0) {
        // Text without NUL bytes; a truncated binary file with a "solid"
        // header almost always has some in its triangle count or data.
        read_ascii(&String::from_utf8_lossy(bytes))?
    } else if bytes.len() >= 84 {
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]);
        return Err(IoError::parse(F, None, format!("binary STL declares {n} triangles ({} bytes) but the file has {} bytes", 84 + 50 * n as u64, bytes.len())));
    } else {
        return Err(IoError::parse(F, None, "file is too short for binary STL and does not start with \"solid\""));
    };
    let mut added = Vec::with_capacity(solids.len());
    for (name, mut mesh) in solids {
        weld_exact(&mut mesh);
        added.push(scene.add_mesh_object(&name, mesh));
    }
    Ok(added)
}

fn read_binary(b: &[u8]) -> (String, Mesh) {
    let n = u32::from_le_bytes([b[80], b[81], b[82], b[83]]) as usize;
    let mut m = Mesh { positions: Vec::with_capacity(n * 3), faces: Vec::with_capacity(n), loose_edges: Vec::new() };
    let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    for t in 0..n {
        let base = 84 + 50 * t + 12; // skip the stored normal
        let i = m.positions.len() as u32;
        for c in 0..3 {
            let o = base + c * 12;
            m.positions.push(Vec3::new(f(o), f(o + 4), f(o + 8)));
        }
        m.faces.push(Face::new(vec![i, i + 1, i + 2]));
    }
    // The 80-byte header is free text (often junk), so it is not used as a name.
    ("STL".to_string(), m)
}

fn read_ascii(text: &str) -> Result<Vec<(String, Mesh)>, IoError> {
    let mut solids = Vec::new();
    let mut cur: Option<(String, Mesh)> = None;
    let mut facet: Vec<u32> = Vec::new();
    let mut in_facet = false;
    for (ln, raw) in text.lines().enumerate() {
        let line = ln + 1;
        let l = raw.trim();
        let mut toks = l.split_whitespace();
        let Some(kw) = toks.next() else { continue };
        match kw {
            "solid" => {
                if let Some(s) = cur.take() {
                    solids.push(s);
                }
                let name = l[5..].trim();
                cur = Some((if name.is_empty() { "STL".into() } else { name.into() }, Mesh::new()));
            }
            "facet" => {
                if cur.is_none() {
                    return Err(IoError::at_line(F, line, "\"facet\" outside a solid"));
                }
                in_facet = true;
                facet.clear();
            }
            "vertex" => {
                let (_, m) = cur.as_mut().filter(|_| in_facet).ok_or_else(|| IoError::at_line(F, line, "\"vertex\" outside a facet"))?;
                let n = toks.clone().count();
                if n != 3 {
                    return Err(IoError::at_line(F, line, format!("vertex has {n} coordinates, expected 3")));
                }
                let [x, y, z] = parse_floats::<3>(toks, 0.0, F, line)?;
                facet.push(m.add_vertex(Vec3::new(x, y, z)));
            }
            "endfacet" => {
                let (_, m) = cur.as_mut().filter(|_| in_facet).ok_or_else(|| IoError::at_line(F, line, "\"endfacet\" without \"facet\""))?;
                if facet.len() < 3 {
                    return Err(IoError::at_line(F, line, format!("facet has {} vertices, expected 3", facet.len())));
                }
                m.faces.push(Face::new(std::mem::take(&mut facet)));
                in_facet = false;
            }
            "endsolid" => {
                if let Some(s) = cur.take() {
                    solids.push(s);
                }
            }
            "outer" | "endloop" => {}
            _ => return Err(IoError::at_line(F, line, format!("unexpected \"{kw}\""))),
        }
    }
    if in_facet {
        return Err(IoError::parse(F, None, "file ends inside a facet"));
    }
    if let Some(s) = cur {
        solids.push(s);
    }
    Ok(solids)
}

/// Binary STL of every mesh object in world space.
pub fn export_binary(scene: &Scene) -> Vec<u8> {
    let meshes = world_meshes(scene);
    let tris: Vec<_> = meshes.iter().map(|w| w.mesh.triangulate()).collect();
    let count: usize = tris.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(84 + 50 * count);
    // Must not start with "solid", or naive readers take it for ASCII.
    let mut header = [b' '; 80];
    header[..20].copy_from_slice(b"Polyamour binary STL");
    out.extend_from_slice(&header);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for (w, ts) in meshes.iter().zip(&tris) {
        for t in ts {
            let [a, b, c] = t.verts.map(|i| w.mesh.positions[i as usize]);
            let n = (b - a).cross(c - a).normalize();
            for v in [n, a, b, c] {
                for x in [v.x, v.y, v.z] {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            out.extend_from_slice(&[0, 0]);
        }
    }
    out
}

/// ASCII STL, one `solid` per mesh object, in world space.
pub fn export_ascii(scene: &Scene) -> Vec<u8> {
    let mut out = String::new();
    for w in world_meshes(scene) {
        let name: String = w.name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect();
        let _ = writeln!(out, "solid {name}");
        for t in w.mesh.triangulate() {
            let [a, b, c] = t.verts.map(|i| w.mesh.positions[i as usize]);
            let n = (b - a).cross(c - a).normalize();
            let _ = writeln!(out, "  facet normal {} {} {}\n    outer loop", n.x, n.y, n.z);
            for p in [a, b, c] {
                let _ = writeln!(out, "      vertex {} {} {}", p.x, p.y, p.z);
            }
            out.push_str("    endloop\n  endfacet\n");
        }
        let _ = writeln!(out, "endsolid {name}");
    }
    out.into_bytes()
}
