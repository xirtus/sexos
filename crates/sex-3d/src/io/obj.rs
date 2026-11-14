// SPDX-License-Identifier: GPL-3.0-only
//! Wavefront OBJ and MTL.
//!
//! Import reads `v`/`vt`/`f` (n-gons, `v/vt/vn` corners, negative relative
//! indices), `l` polylines as loose edges, `o` and `g` as object splits,
//! `usemtl` as material slots, `s` as smooth shading and `mtllib` through the
//! resolver. Vertex normals are validated but not kept: shading comes from `s`.
//! MTL maps to the Principled material the way Blender does: `Kd` base colour,
//! `Ns` roughness (`1 - sqrt(Ns / 1000)`), `Pr`/`Pm` PBR roughness and
//! metallic, `Ke` emission, `d`/`Tr` alpha, `Ni` IOR, `map_Kd` texture.
//!
//! OBJ is Y-up: positions are converted to and from the scene's Z-up.
//! Export writes every mesh object in world space with UVs, normals,
//! smoothing groups and material names.
use super::{parse_floats, world_meshes, y_up_to_z_up, z_up_to_y_up, Format, IoError, Resolver};
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::scene::{unique_name, Material, Scene};
use std::collections::HashMap;
use std::fmt::Write as _;

const F: Format = Format::Obj;

/// One `newmtl` block.
struct MtlEntry {
    material: Material,
    texture: Option<String>,
}

/// Parse MTL text into named materials (texture paths unresolved).
fn parse_mtl(text: &str) -> Result<Vec<(String, MtlEntry)>, IoError> {
    let mut out: Vec<(String, MtlEntry)> = Vec::new();
    for (ln, raw) in text.lines().enumerate() {
        let line = ln + 1;
        let l = raw.split('#').next().unwrap_or("").trim();
        let mut toks = l.split_whitespace();
        let Some(kw) = toks.next() else { continue };
        if kw == "newmtl" {
            let name = l[kw.len()..].trim();
            let material = Material { name: if name.is_empty() { "Material".into() } else { name.into() }, ..Default::default() };
            out.push((material.name.clone(), MtlEntry { material, texture: None }));
            continue;
        }
        let Some((_, cur)) = out.last_mut() else { continue };
        let m = &mut cur.material;
        let mtl_err = |msg: &str| IoError::Parse { format: F, line: Some(line), message: format!("in .mtl: {msg}") };
        let one = |toks: &mut std::str::SplitWhitespace| -> Result<f32, IoError> {
            let t = toks.next().ok_or_else(|| mtl_err(&format!("\"{kw}\" needs a value")))?;
            t.parse::<f32>().map_err(|_| mtl_err(&format!("invalid number \"{t}\"")))
        };
        match kw {
            "Kd" => {
                let [r, g, b] = parse_floats::<3>(toks, 0.0, F, line).map_err(|_| mtl_err("invalid Kd colour"))?;
                m.base_color = [r, g, b, m.base_color[3]];
            }
            "Ns" => m.roughness = 1.0 - (one(&mut toks)?.clamp(0.0, 1000.0) / 1000.0).sqrt(),
            "Pr" => m.roughness = one(&mut toks)?.clamp(0.0, 1.0),
            "Pm" => m.metallic = one(&mut toks)?.clamp(0.0, 1.0),
            "Ke" => {
                let [r, g, b] = parse_floats::<3>(toks, 0.0, F, line).map_err(|_| mtl_err("invalid Ke colour"))?;
                m.emission = Vec3::new(r, g, b);
                m.emission_strength = if m.emission == Vec3::ZERO { 0.0 } else { 1.0 };
            }
            "d" => m.base_color[3] = one(&mut toks)?.clamp(0.0, 1.0),
            "Tr" => m.base_color[3] = 1.0 - one(&mut toks)?.clamp(0.0, 1.0),
            "Ni" => m.ior = one(&mut toks)?,
            "map_Kd" => cur.texture = texture_path(&l[kw.len()..]),
            _ => {}
        }
    }
    Ok(out)
}

/// The file name of a `map_*` statement, skipping options such as `-s 1 1 1`
/// or `-clamp on`. File names may contain spaces.
fn texture_path(args: &str) -> Option<String> {
    let toks: Vec<&str> = args.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() && toks[i].starts_with('-') && toks[i].len() > 1 {
        i += 1;
        // Option arguments are numbers or on/off.
        while i < toks.len() && (toks[i].parse::<f32>().is_ok() || toks[i] == "on" || toks[i] == "off") {
            i += 1;
        }
    }
    let name = toks[i.min(toks.len())..].join(" ");
    (!name.is_empty()).then_some(name)
}

/// The object being collected between `o`/`g` statements.
#[derive(Default)]
struct Builder {
    name: String,
    /// Faces with file-global vertex indices.
    faces: Vec<Face>,
    edges: Vec<[u32; 2]>,
    /// Material names in slot order; a slot is added when a face uses it.
    slots: Vec<String>,
    /// The material named by the last `usemtl` (carries over between objects).
    active: Option<String>,
    smooth: bool,
}

impl Builder {
    fn is_empty(&self) -> bool {
        self.faces.is_empty() && self.edges.is_empty()
    }
    /// Slot of the active material, adding it on first use.
    fn slot(&mut self) -> u16 {
        let Some(name) = &self.active else { return 0 };
        match self.slots.iter().position(|s| s == name) {
            Some(i) => i as u16,
            None => {
                self.slots.push(name.clone());
                (self.slots.len() - 1) as u16
            }
        }
    }
}

/// Resolve an OBJ index (1-based, or negative relative to the end) against
/// `count` elements defined so far.
fn resolve(tok: &str, count: usize, what: &str, line: usize) -> Result<u32, IoError> {
    let i: i64 = tok.parse().map_err(|_| IoError::at_line(F, line, format!("invalid {what} index \"{tok}\"")))?;
    let idx = if i > 0 { i - 1 } else { count as i64 + i };
    if i == 0 || idx < 0 || idx >= count as i64 {
        return Err(IoError::at_line(F, line, format!("{what} index {i} out of range ({count} defined)")));
    }
    Ok(idx as u32)
}

/// Import an OBJ file. `.mtl` libraries and textures come from `resolver`;
/// missing ones leave default materials rather than failing the import.
pub fn import_into(scene: &mut Scene, bytes: &[u8], resolver: Resolver) -> Result<Vec<usize>, IoError> {
    let text = String::from_utf8_lossy(bytes);
    let mut positions: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut normal_count = 0usize;
    let mut library: HashMap<String, MtlEntry> = HashMap::new();
    let mut cur = Builder { name: "Mesh".into(), ..Default::default() };
    let mut done: Vec<Builder> = Vec::new();
    let mut pending = String::new();
    let mut start_line = 0;
    for (ln, raw) in text.lines().enumerate() {
        // Backslash continues a statement on the next line.
        if pending.is_empty() {
            start_line = ln + 1;
        }
        let raw = raw.split('#').next().unwrap_or("");
        if let Some(stripped) = raw.trim_end().strip_suffix('\\') {
            pending.push_str(stripped);
            pending.push(' ');
            continue;
        }
        pending.push_str(raw);
        let stmt = std::mem::take(&mut pending);
        let line = start_line;
        let l = stmt.trim();
        let mut toks = l.split_whitespace();
        let Some(kw) = toks.next() else { continue };
        let rest = l[kw.len()..].trim();
        match kw {
            "v" => {
                let [x, y, z] = parse_floats::<3>(toks, 0.0, F, line)?;
                positions.push(y_up_to_z_up(Vec3::new(x, y, z)));
            }
            "vt" => {
                let [u, v] = parse_floats::<2>(toks, 0.0, F, line)?;
                uvs.push(Vec2::new(u, v));
            }
            "vn" => {
                parse_floats::<3>(toks, 0.0, F, line)?;
                normal_count += 1;
            }
            "f" | "fo" => {
                let mut face = Face { material: cur.slot(), smooth: cur.smooth, ..Default::default() };
                let mut all_uv = true;
                for corner in toks {
                    let mut parts = corner.split('/');
                    let v = resolve(parts.next().unwrap_or(""), positions.len(), "vertex", line)?;
                    face.verts.push(v);
                    match parts.next().filter(|t| !t.is_empty()) {
                        Some(t) => face.uvs.push(uvs[resolve(t, uvs.len(), "texture coordinate", line)? as usize]),
                        None => all_uv = false,
                    }
                    if let Some(t) = parts.next().filter(|t| !t.is_empty()) {
                        resolve(t, normal_count, "normal", line)?;
                    }
                }
                if face.verts.len() < 3 {
                    return Err(IoError::at_line(F, line, format!("face has {} vertices, needs at least 3", face.verts.len())));
                }
                if !all_uv {
                    face.uvs.clear();
                }
                cur.faces.push(face);
            }
            "l" => {
                let mut prev = None;
                for corner in toks {
                    let v = resolve(corner.split('/').next().unwrap_or(""), positions.len(), "vertex", line)?;
                    if let Some(p) = prev {
                        cur.edges.push([p, v]);
                    }
                    prev = Some(v);
                }
            }
            "o" | "g" => {
                let name = if rest.is_empty() { "Mesh".to_string() } else { rest.to_string() };
                if cur.is_empty() {
                    cur.name = name;
                } else {
                    let next = Builder { name, smooth: cur.smooth, active: cur.active.clone(), ..Default::default() };
                    done.push(std::mem::replace(&mut cur, next));
                }
            }
            "usemtl" => {
                cur.active = Some(rest.to_string());
            }
            "s" => cur.smooth = !matches!(rest, "off" | "0" | ""),
            "mtllib" => {
                // Prefer the whole rest as one name (names may contain spaces).
                let files = match resolver(rest) {
                    Some(b) => vec![b],
                    None => rest.split_whitespace().filter_map(resolver).collect(),
                };
                for b in files {
                    for (name, e) in parse_mtl(&String::from_utf8_lossy(&b))? {
                        library.insert(name, e);
                    }
                }
            }
            _ => {}
        }
    }
    if !cur.is_empty() {
        done.push(cur);
    }

    let mut material_index: HashMap<String, usize> = HashMap::new();
    let mut texture_index: HashMap<String, Option<usize>> = HashMap::new();
    let mut added = Vec::with_capacity(done.len());
    for b in done {
        // File-global vertex indices to object-local ones, keeping file order.
        let mut used: Vec<u32> = b.faces.iter().flat_map(|f| f.verts.iter().copied()).chain(b.edges.iter().flatten().copied()).collect();
        used.sort_unstable();
        used.dedup();
        let local: HashMap<u32, u32> = used.iter().enumerate().map(|(l, &g)| (g, l as u32)).collect();
        let mut mesh = Mesh { positions: used.iter().map(|&g| positions[g as usize]).collect(), ..Default::default() };
        for mut f in b.faces {
            for v in &mut f.verts {
                *v = local[v];
            }
            mesh.faces.push(f);
        }
        mesh.loose_edges.extend(b.edges.iter().map(|e| [local[&e[0]], local[&e[1]]]).filter(|e| e[0] != e[1]));
        let mut slots = Vec::with_capacity(b.slots.len());
        for name in &b.slots {
            let idx = match material_index.get(name) {
                Some(&i) => i,
                None => {
                    let (mut mat, tex) = match library.get(name) {
                        Some(e) => (e.material.clone(), e.texture.clone()),
                        None => (Material { name: name.clone(), ..Default::default() }, None),
                    };
                    if let Some(path) = tex {
                        mat.base_color_texture = *texture_index.entry(path.clone()).or_insert_with(|| {
                            let bytes = resolver(&path)?;
                            let img = super::decode_image(&path, &bytes).ok()?;
                            scene.images.push(img);
                            Some(scene.images.len() - 1)
                        });
                    }
                    let i = scene.add_material(mat);
                    material_index.insert(name.clone(), i);
                    i
                }
            };
            slots.push(idx);
        }
        let o = scene.add_mesh_object(&b.name, mesh);
        scene.objects[o].materials = slots;
        added.push(o);
    }
    Ok(added)
}

/// Shortest text that reads back as the same `f32`.
fn num(out: &mut String, v: f32) {
    let _ = write!(out, " {}", if v == 0.0 { 0.0 } else { v });
}

/// Unique OBJ/MTL-safe names for the scene's materials (no whitespace).
fn material_names(scene: &Scene) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(scene.materials.len());
    for m in &scene.materials {
        let base: String = m.name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect();
        let base = if base.is_empty() { "Material".to_string() } else { base };
        let n = unique_name(&base, |n| names.iter().any(|x| x == n));
        names.push(n);
    }
    names
}

/// Write the OBJ text. With `mtllib`, a `mtllib` line references that file.
pub fn export(scene: &Scene, mtllib: Option<&str>) -> Vec<u8> {
    let names = material_names(scene);
    let mut out = String::from("# Polyamour OBJ\n");
    if let Some(lib) = mtllib {
        let _ = writeln!(out, "mtllib {lib}");
    }
    // OBJ indices are file-global.
    let (mut v_base, mut vt_base, mut vn_base) = (1usize, 1usize, 1usize);
    for wm in world_meshes(scene) {
        let m = &wm.mesh;
        let _ = writeln!(out, "o {}", wm.name);
        for &p in &m.positions {
            let p = z_up_to_y_up(p);
            out.push('v');
            for c in [p.x, p.y, p.z] {
                num(&mut out, c);
            }
            out.push('\n');
        }
        // Deduplicated UVs and normals.
        let mut uv_ids: HashMap<[u32; 2], usize> = HashMap::new();
        let mut uv_list: Vec<Vec2> = Vec::new();
        let mut n_ids: HashMap<[u32; 3], usize> = HashMap::new();
        let mut n_list: Vec<Vec3> = Vec::new();
        let vnormals = m.vertex_normals();
        let mut corners: Vec<Vec<(usize, Option<usize>, usize)>> = Vec::with_capacity(m.faces.len());
        for (fi, f) in m.faces.iter().enumerate() {
            let fnorm = m.face_normal(fi);
            let mut cs = Vec::with_capacity(f.verts.len());
            for (ci, &v) in f.verts.iter().enumerate() {
                let uv = (f.uvs.len() == f.verts.len()).then(|| {
                    let uv = f.uvs[ci];
                    *uv_ids.entry([uv.x.to_bits(), uv.y.to_bits()]).or_insert_with(|| {
                        uv_list.push(uv);
                        uv_list.len() - 1
                    })
                });
                let n = z_up_to_y_up(if f.smooth { vnormals[v as usize] } else { fnorm });
                let ni = *n_ids.entry([n.x.to_bits(), n.y.to_bits(), n.z.to_bits()]).or_insert_with(|| {
                    n_list.push(n);
                    n_list.len() - 1
                });
                cs.push((v as usize, uv, ni));
            }
            corners.push(cs);
        }
        for uv in &uv_list {
            out.push_str("vt");
            num(&mut out, uv.x);
            num(&mut out, uv.y);
            out.push('\n');
        }
        for n in &n_list {
            out.push_str("vn");
            for c in [n.x, n.y, n.z] {
                num(&mut out, (c * 1e6).round() / 1e6);
            }
            out.push('\n');
        }
        let (mut cur_mat, mut cur_smooth) = (None, None);
        for (f, cs) in m.faces.iter().zip(&corners) {
            let mat = wm.slots.get(f.material as usize).or(wm.slots.last()).copied();
            if mat.is_some() && mat != cur_mat {
                if let Some(name) = mat.and_then(|i| names.get(i)) {
                    let _ = writeln!(out, "usemtl {name}");
                }
                cur_mat = mat;
            }
            if cur_smooth != Some(f.smooth) {
                out.push_str(if f.smooth { "s 1\n" } else { "s off\n" });
                cur_smooth = Some(f.smooth);
            }
            out.push('f');
            for &(v, uv, n) in cs {
                match uv {
                    Some(t) => {
                        let _ = write!(out, " {}/{}/{}", v + v_base, t + vt_base, n + vn_base);
                    }
                    None => {
                        let _ = write!(out, " {}//{}", v + v_base, n + vn_base);
                    }
                }
            }
            out.push('\n');
        }
        for e in &m.loose_edges {
            let _ = writeln!(out, "l {} {}", e[0] as usize + v_base, e[1] as usize + v_base);
        }
        v_base += m.positions.len();
        vt_base += uv_list.len();
        vn_base += n_list.len();
    }
    out.into_bytes()
}

/// `stem.obj`, `stem.mtl` and one PNG per texture the materials use.
pub fn export_files(scene: &Scene, stem: &str) -> Vec<(String, Vec<u8>)> {
    let mtl_name = format!("{stem}.mtl");
    let obj = export(scene, Some(&mtl_name));
    let names = material_names(scene);
    let mut textures: Vec<(String, Vec<u8>)> = Vec::new();
    let mut texture_file: HashMap<usize, String> = HashMap::new();
    let mut mtl = String::from("# Polyamour MTL\n");
    for (m, name) in scene.materials.iter().zip(&names) {
        let _ = writeln!(mtl, "\nnewmtl {name}");
        let [r, g, b, a] = m.base_color;
        let _ = writeln!(mtl, "Kd {r} {g} {b}");
        let _ = writeln!(mtl, "Ns {}", (1.0 - m.roughness.clamp(0.0, 1.0)).powi(2) * 1000.0);
        let _ = writeln!(mtl, "Pr {}\nPm {}", m.roughness, m.metallic);
        let e = m.emission * m.emission_strength;
        let _ = writeln!(mtl, "Ke {} {} {}", e.x, e.y, e.z);
        let _ = writeln!(mtl, "Ni {}\nd {a}", m.ior);
        let _ = writeln!(mtl, "illum {}", if m.metallic > 0.0 || m.transmission > 0.0 { 3 } else { 2 });
        if let Some(ti) = m.base_color_texture.filter(|&t| t < scene.images.len()) {
            let file = texture_file
                .entry(ti)
                .or_insert_with(|| {
                    let base: String = scene.images[ti].name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
                    let base = base.trim_end_matches("_png").to_string();
                    let file = unique_name(&format!("{stem}_{base}.png"), |n| textures.iter().any(|(t, _)| t == n));
                    textures.push((file.clone(), super::encode_png(&scene.images[ti])));
                    file
                })
                .clone();
            let _ = writeln!(mtl, "map_Kd {file}");
        }
    }
    let mut files = vec![(format!("{stem}.obj"), obj), (mtl_name, mtl.into_bytes())];
    files.extend(textures);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_path_skips_options() {
        assert_eq!(texture_path(" -s 1 1 1 -clamp on my wood.png").as_deref(), Some("my wood.png"));
        assert_eq!(texture_path(" tex.jpg").as_deref(), Some("tex.jpg"));
        assert_eq!(texture_path(" "), None);
    }

    #[test]
    fn negative_index_resolves() {
        assert_eq!(resolve("-1", 4, "vertex", 1).unwrap(), 3);
        assert_eq!(resolve("1", 4, "vertex", 1).unwrap(), 0);
        assert!(resolve("0", 4, "vertex", 1).is_err());
        assert!(resolve("5", 4, "vertex", 1).is_err());
    }

    #[test]
    fn floats_print_round_trip() {
        let mut s = String::new();
        num(&mut s, 0.1);
        num(&mut s, -0.0);
        assert_eq!(s, " 0.1 0");
    }
}
