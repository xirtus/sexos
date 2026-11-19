// SPDX-License-Identifier: GPL-3.0-only
//! Sculpt and paint data in `.poly` files: masks, face sets, hidden faces,
//! color attributes, Multires levels and the scene's images.
//!
//! Images are embedded in the file as base64 PNG (`#@image` lines) rather
//! than written as sidecar files: a `.poly` stays one self-contained file
//! that the save dialog, the file manager and the Python package can move
//! around without losing its textures, and painting a texture never leaves a
//! half-saved pair behind. PNG is 8-bit sRGB, so an HDR image loses its
//! range above 1; a painted texture does not have one.
//!
//! Every line is a `#@` comment, so polyops readers skip it. Meshes are named
//! by their data name, Multires modifiers by object name and stack index.

use super::{esc, unesc};
use sex_3d::mesh::{AttrDomain, ColorAttribute};
use sex_3d::modifiers::{Modifier, MultiresLevel, MultiresSubdiv};
use sex_3d::scene::Scene;
use sex_3d::Vec3;

pub fn save(scene: &Scene, out: &mut String) {
    for img in &scene.images {
        let png = sex_3d::io::encode_png(img);
        out.push_str(&format!("#@image {}\t{}\n", esc(&img.name), b64_encode(&png)));
    }
    for (mi, m) in scene.meshes.iter().enumerate() {
        let name = esc(&scene.mesh_names[mi]);
        if m.mask.len() == m.positions.len() && m.mask.iter().any(|&v| v != 0.0) {
            out.push_str(&format!("#@mask {name}\t{}\n", b64_encode(&f32_bytes(&m.mask))));
        }
        if m.face_sets.len() == m.faces.len() && m.face_sets.iter().any(|&s| s != 1) {
            let bytes: Vec<u8> = m.face_sets.iter().flat_map(|s| s.to_le_bytes()).collect();
            out.push_str(&format!("#@face_sets {name}\t{}\n", b64_encode(&bytes)));
        }
        if m.hidden_faces.len() == m.faces.len() && m.hidden_faces.iter().any(|&h| h) {
            let s: String = m.hidden_faces.iter().map(|&h| if h { '1' } else { '0' }).collect();
            out.push_str(&format!("#@hidden_faces {name}\t{s}\n"));
        }
        for a in &m.color_attributes {
            let flat: Vec<f32> = a.data.iter().flat_map(|c| *c).collect();
            out.push_str(&format!("#@color_attribute {name}\t{}\t{}\t{}\t{}\n", esc(&a.name), a.domain.id(), a.byte as u8, b64_encode(&f32_bytes(&flat))));
        }
        if !m.color_attributes.is_empty() {
            out.push_str(&format!("#@color_active {name}\t{}\t{}\n", m.active_color, m.render_color));
        }
    }
    for o in &scene.objects {
        for (i, md) in o.modifiers.iter().enumerate() {
            if let Modifier::Multires { levels, sculpt_levels, render_levels, optimal_display, data } = md {
                let on = esc(&o.name);
                out.push_str(&format!("#@multires {on}\t{i}\t{levels}\t{sculpt_levels}\t{render_levels}\t{}\n", *optimal_display as u8));
                for l in &data.levels {
                    let flat: Vec<f32> = l.disp.iter().flat_map(|d| d.to_array()).collect();
                    out.push_str(&format!("#@multires_level {on}\t{i}\t{}\t{}\n", l.kind.id(), b64_encode(&f32_bytes(&flat))));
                }
            }
        }
    }
}

/// Read the embedded images, in file order, into a scene before polyops
/// runs: its material trees name them, and the `#@ntree` lines hold their
/// indexes.
pub fn load_images(scene: &mut Scene, text: &str) {
    for line in text.lines() {
        let Some(body) = line.strip_prefix("#@image ") else { continue };
        let Some((name, data)) = body.split_once('\t') else { continue };
        let Some(png) = b64_decode(data) else { continue };
        if let Ok(mut img) = sex_3d::io::decode_image(&unesc(name), &png) {
            img.name = unesc(name);
            scene.images.push(img);
        }
    }
}

/// Read the mesh and Multires lines back into a scene that polyops just
/// built.
pub fn load(scene: &mut Scene, text: &str) {
    let mesh = |scene: &Scene, name: &str| scene.mesh_names.iter().position(|n| *n == unesc(name));
    let object = |scene: &Scene, name: &str| scene.objects.iter().position(|o| o.name == unesc(name));
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("#@") else { continue };
        let (tag, body) = rest.split_once(' ').unwrap_or((rest, ""));
        let f: Vec<&str> = body.split('\t').collect();
        match (tag, f.as_slice()) {
            ("mask", [name, data]) => {
                if let (Some(mi), Some(v)) = (mesh(scene, name), b64_decode(data).map(|b| bytes_f32(&b))) {
                    if v.len() == scene.meshes[mi].positions.len() {
                        scene.meshes[mi].mask = v;
                    }
                }
            }
            ("face_sets", [name, data]) => {
                if let (Some(mi), Some(b)) = (mesh(scene, name), b64_decode(data)) {
                    let v: Vec<i32> = b.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                    if v.len() == scene.meshes[mi].faces.len() {
                        scene.meshes[mi].face_sets = v;
                    }
                }
            }
            ("hidden_faces", [name, s]) => {
                if let Some(mi) = mesh(scene, name) {
                    if s.len() == scene.meshes[mi].faces.len() {
                        scene.meshes[mi].hidden_faces = s.bytes().map(|c| c == b'1').collect();
                    }
                }
            }
            ("color_attribute", [name, aname, domain, byte, data]) => {
                let (Some(mi), Some(domain), Some(b)) = (mesh(scene, name), AttrDomain::from_id(domain), b64_decode(data)) else { continue };
                let v = bytes_f32(&b);
                let data: Vec<[f32; 4]> = v.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                let m = &mut scene.meshes[mi];
                let want = match domain {
                    AttrDomain::Point => m.positions.len(),
                    _ => m.faces.iter().map(|f| f.verts.len()).sum(),
                };
                if data.len() == want {
                    m.color_attributes.push(ColorAttribute { name: unesc(aname), domain, data, byte: *byte == "1" });
                }
            }
            ("color_active", [name, a, r]) => {
                if let Some(mi) = mesh(scene, name) {
                    let m = &mut scene.meshes[mi];
                    let n = m.color_attributes.len().max(1);
                    m.active_color = a.parse::<usize>().unwrap_or(0).min(n - 1);
                    m.render_color = r.parse::<usize>().unwrap_or(0).min(n - 1);
                }
            }
            ("multires", [name, i, levels, sculpt, render, optimal]) => {
                let (Some(oi), Ok(i)) = (object(scene, name), i.parse::<usize>()) else { continue };
                if let Some(Modifier::Multires { levels: l, sculpt_levels: s, render_levels: r, optimal_display: o, data }) = scene.objects[oi].modifiers.get_mut(i) {
                    *l = levels.parse().unwrap_or(0);
                    *s = sculpt.parse().unwrap_or(0);
                    *r = render.parse().unwrap_or(0);
                    *o = *optimal == "1";
                    data.levels.clear();
                }
            }
            ("multires_level", [name, i, kind, data]) => {
                let (Some(oi), Ok(i), Some(kind), Some(b)) = (object(scene, name), i.parse::<usize>(), MultiresSubdiv::from_id(kind), b64_decode(data)) else { continue };
                if let Some(Modifier::Multires { data: md, .. }) = scene.objects[oi].modifiers.get_mut(i) {
                    let disp = bytes_f32(&b).chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
                    md.levels.push(MultiresLevel { kind, disp });
                }
            }
            _ => {}
        }
    }
    // Levels past what the file held would index nothing.
    for o in &mut scene.objects {
        for md in &mut o.modifiers {
            if let Modifier::Multires { levels, sculpt_levels, render_levels, data, .. } = md {
                let n = data.levels.len() as u32;
                *levels = (*levels).min(n);
                *sculpt_levels = (*sculpt_levels).min(n);
                *render_levels = (*render_levels).min(n);
            }
        }
    }
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn bytes_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn b64_encode(data: &[u8]) -> String {
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                o.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

pub(crate) fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().trim_end_matches('=');
    let mut o = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            o.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for n in 0..10 {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&data)).unwrap(), data);
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
    }
}
