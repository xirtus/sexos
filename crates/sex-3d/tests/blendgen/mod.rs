// SPDX-License-Identifier: GPL-3.0-only
//! A tiny synthetic `.blend` writer for the importer tests.
//!
//! It emits a real file header, BHead blocks, a DNA1 catalogue describing just
//! the structs the fixtures use (with Blender's member names and types and
//! explicit `_pad` members, which it fills with junk the reader must skip),
//! and the ENDB terminator, in either byte order, with 4- or 8-byte pointers,
//! and in the classic or the Blender 5.0 "large BHead" flavour.
#![allow(dead_code)]

use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;

// ---------------------------------------------------------------------------
// SDNA description

/// Split a member declarator into (base name, is pointer, array length).
fn decl(d: &str) -> (String, bool, usize) {
    let is_ptr = d.starts_with('*') || d.starts_with("(*");
    let core = d.trim_start_matches(['*', '(']);
    let base = core.split(['[', ')']).next().unwrap().to_string();
    let len = if d.starts_with("(*") { 1 } else { core.split('[').skip(1).map(|p| p.trim_end_matches(']').parse::<usize>().unwrap()).product() };
    (base, is_ptr, len)
}

pub struct Dna {
    pub ptr: usize,
    names: Vec<String>,
    types: Vec<String>,
    tlen: Vec<usize>,
    structs: Vec<(usize, Vec<(usize, usize)>)>,
}

/// Where a member lives inside a struct.
#[derive(Clone, Debug)]
pub struct Slot {
    pub offset: usize,
    pub ty: String,
    pub elem: usize,
    pub len: usize,
    pub is_ptr: bool,
}

impl Dna {
    pub fn new(ptr: usize) -> Dna {
        let mut d = Dna { ptr, names: Vec::new(), types: Vec::new(), tlen: Vec::new(), structs: Vec::new() };
        for (t, l) in [("char", 1), ("uchar", 1), ("short", 2), ("ushort", 2), ("int", 4), ("uint", 4), ("float", 4), ("double", 8), ("int64_t", 8), ("uint64_t", 8), ("int8_t", 1), ("void", 0)] {
            d.types.push(t.into());
            d.tlen.push(l);
        }
        d
    }

    fn ty(&mut self, name: &str) -> usize {
        if let Some(i) = self.types.iter().position(|t| t == name) {
            return i;
        }
        self.types.push(name.into());
        self.tlen.push(0); // opaque until defined
        self.types.len() - 1
    }

    fn name(&mut self, n: &str) -> usize {
        if let Some(i) = self.names.iter().position(|x| x == n) {
            return i;
        }
        self.names.push(n.into());
        self.names.len() - 1
    }

    /// Define a struct from (type, declarator) pairs, in order.
    pub fn def(&mut self, sname: &str, members: &[(&str, &str)]) {
        let st = self.ty(sname);
        let mut size = 0;
        let mut ms = Vec::new();
        for &(t, n) in members {
            let (_, is_ptr, len) = decl(n);
            let ti = self.ty(t);
            let elem = if is_ptr { self.ptr } else { self.tlen[ti] };
            assert!(is_ptr || elem > 0, "{sname}.{n}: type {t} not defined yet");
            size += elem * len;
            ms.push((ti, self.name(n)));
        }
        self.tlen[st] = size;
        self.structs.push((st, ms));
    }

    pub fn struct_index(&self, sname: &str) -> usize {
        self.structs.iter().position(|(t, _)| self.types[*t] == sname).unwrap_or_else(|| panic!("no struct {sname}"))
    }

    pub fn size(&self, sname: &str) -> usize {
        self.tlen[self.types.iter().position(|t| t == sname).unwrap()]
    }

    /// Members of a struct with their offsets.
    fn members(&self, sname: &str) -> Vec<(String, Slot)> {
        let (_, ms) = &self.structs[self.struct_index(sname)];
        let mut off = 0;
        ms.iter()
            .map(|&(t, n)| {
                let (base, is_ptr, len) = decl(&self.names[n]);
                let elem = if is_ptr { self.ptr } else { self.tlen[t] };
                let s = Slot { offset: off, ty: self.types[t].clone(), elem, len, is_ptr };
                off += elem * len;
                (base, s)
            })
            .collect()
    }

    /// Find a dotted member path such as `id.name` or `r.xsch`.
    pub fn slot(&self, sname: &str, path: &str) -> Slot {
        let (head, rest) = match path.split_once('.') {
            Some((h, r)) => (h, Some(r)),
            None => (path, None),
        };
        let (_, s) = self.members(sname).into_iter().find(|(n, _)| n == head).unwrap_or_else(|| panic!("no member {sname}.{head}"));
        match rest {
            None => s,
            Some(r) => {
                let mut inner = self.slot(&s.ty, r);
                inner.offset += s.offset;
                inner
            }
        }
    }

    fn encode(&self, big: bool) -> Vec<u8> {
        let mut o = b"SDNA".to_vec();
        let u32b = |o: &mut Vec<u8>, v: usize| o.extend(if big { (v as u32).to_be_bytes() } else { (v as u32).to_le_bytes() });
        let u16b = |o: &mut Vec<u8>, v: usize| o.extend(if big { (v as u16).to_be_bytes() } else { (v as u16).to_le_bytes() });
        let pad = |o: &mut Vec<u8>| {
            while o.len() % 4 != 0 {
                o.push(0);
            }
        };
        o.extend(b"NAME");
        u32b(&mut o, self.names.len());
        for n in &self.names {
            o.extend(n.as_bytes());
            o.push(0);
        }
        pad(&mut o);
        o.extend(b"TYPE");
        u32b(&mut o, self.types.len());
        for n in &self.types {
            o.extend(n.as_bytes());
            o.push(0);
        }
        pad(&mut o);
        o.extend(b"TLEN");
        for &l in &self.tlen {
            u16b(&mut o, l);
        }
        pad(&mut o);
        o.extend(b"STRC");
        u32b(&mut o, self.structs.len());
        for (t, ms) in &self.structs {
            u16b(&mut o, *t);
            u16b(&mut o, ms.len());
            for &(a, b) in ms {
                u16b(&mut o, a);
                u16b(&mut o, b);
            }
        }
        o
    }
}

// ---------------------------------------------------------------------------
// Values and the file writer

/// A member value.
pub enum V {
    /// Numbers, converted to the member's declared type.
    N(Vec<f64>),
    /// Pointers.
    P(Vec<u64>),
    /// A NUL-terminated string in a char array.
    S(String),
}

pub fn n(x: f64) -> V {
    V::N(vec![x])
}
pub fn nv(x: &[f64]) -> V {
    V::N(x.to_vec())
}
pub fn p(x: u64) -> V {
    V::P(vec![x])
}
pub fn s(x: &str) -> V {
    V::S(x.into())
}

pub struct Writer {
    pub dna: Dna,
    pub big: bool,
    /// Blender 5.0 header and 32-byte block headers.
    pub large: bool,
    pub version: u32,
    blocks: Vec<([u8; 4], usize, u64, usize, Vec<u8>)>,
    next: u64,
}

impl Writer {
    pub fn new(dna: Dna, big: bool, large: bool, version: u32) -> Writer {
        Writer { dna, big, large, version, blocks: Vec::new(), next: 0x10_0000 }
    }

    /// A fresh old-pointer value (fits in 32 bits).
    pub fn alloc(&mut self) -> u64 {
        self.next += 0x140;
        self.next
    }

    fn num(&self, ty: &str, v: f64) -> Vec<u8> {
        macro_rules! e {
            ($x:expr) => {
                if self.big { $x.to_be_bytes().to_vec() } else { $x.to_le_bytes().to_vec() }
            };
        }
        match ty {
            "char" | "uchar" | "int8_t" => vec![v as i64 as u8],
            "short" | "ushort" => e!(v as i64 as i16),
            "int" | "uint" => e!(v as i64 as i32),
            "float" => e!(v as f32),
            "double" => e!(v),
            "int64_t" | "uint64_t" => e!(v as i64),
            t => panic!("not numeric: {t}"),
        }
    }

    pub fn ptr_bytes(&self, v: u64) -> Vec<u8> {
        match (self.dna.ptr, self.big) {
            (4, false) => (v as u32).to_le_bytes().to_vec(),
            (4, true) => (v as u32).to_be_bytes().to_vec(),
            (_, false) => v.to_le_bytes().to_vec(),
            (_, true) => v.to_be_bytes().to_vec(),
        }
    }

    /// One instance of `sname`. Unset members are zero; `_pad` members get junk.
    pub fn inst<K: AsRef<str>>(&self, sname: &str, fields: &[(K, V)]) -> Vec<u8> {
        let mut o = vec![0u8; self.dna.size(sname)];
        for (name, s) in self.dna.members(sname) {
            if name.starts_with("_pad") {
                o[s.offset..s.offset + s.elem * s.len].fill(0xA5);
            }
        }
        for (path, v) in fields {
            let path = path.as_ref();
            let s = self.dna.slot(sname, path);
            let bytes: Vec<u8> = match v {
                V::N(xs) => {
                    assert!(!s.is_ptr && xs.len() <= s.len, "{sname}.{path}");
                    xs.iter().flat_map(|&x| self.num(&s.ty, x)).collect()
                }
                V::P(xs) => {
                    assert!(s.is_ptr && xs.len() <= s.len, "{sname}.{path}");
                    xs.iter().flat_map(|&x| self.ptr_bytes(x)).collect()
                }
                V::S(t) => {
                    assert!(t.len() < s.len && s.elem == 1, "{sname}.{path}");
                    t.as_bytes().to_vec()
                }
            };
            o[s.offset..s.offset + bytes.len()].copy_from_slice(&bytes);
        }
        o
    }

    /// Add a block of `nr` structs (`data` already encoded) at address `ptr`.
    pub fn put(&mut self, ptr: u64, code: &str, sname: &str, nr: usize, data: Vec<u8>) -> u64 {
        let mut c = [0u8; 4];
        c[..code.len()].copy_from_slice(code.as_bytes());
        let sdna = if sname.is_empty() { 0 } else { self.dna.struct_index(sname) };
        self.blocks.push((c, sdna, ptr, nr, data));
        ptr
    }

    /// A struct block at a new address.
    pub fn add(&mut self, code: &str, sname: &str, fields: &[(&str, V)]) -> u64 {
        let ptr = self.alloc();
        let d = self.inst(sname, fields);
        self.put(ptr, code, sname, 1, d)
    }

    /// A raw DATA block (arrays of numbers or pointers).
    pub fn raw(&mut self, data: Vec<u8>, nr: usize) -> u64 {
        let ptr = self.alloc();
        self.put(ptr, "DATA", "", nr, data)
    }

    pub fn f32s(&self, xs: &[f32]) -> Vec<u8> {
        xs.iter().flat_map(|&x| self.num("float", x as f64)).collect()
    }
    pub fn i32s(&self, xs: &[i64]) -> Vec<u8> {
        xs.iter().flat_map(|&x| self.num("int", x as f64)).collect()
    }
    pub fn ptrs(&self, xs: &[u64]) -> Vec<u8> {
        xs.iter().flat_map(|&x| self.ptr_bytes(x)).collect()
    }

    fn bhead(&self, o: &mut Vec<u8>, code: [u8; 4], sdna: usize, old: u64, len: usize, nr: usize) {
        o.extend(code);
        let i32b = |v: i64| if self.big { (v as i32).to_be_bytes() } else { (v as i32).to_le_bytes() };
        let i64b = |v: i64| if self.big { v.to_be_bytes() } else { v.to_le_bytes() };
        if self.large {
            o.extend(i32b(sdna as i64));
            o.extend(i64b(old as i64));
            o.extend(i64b(len as i64));
            o.extend(i64b(nr as i64));
        } else {
            o.extend(i32b(len as i64));
            o.extend(self.ptr_bytes(old));
            o.extend(i32b(sdna as i64));
            o.extend(i32b(nr as i64));
        }
    }

    pub fn finish(&self) -> Vec<u8> {
        let mut o = b"BLENDER".to_vec();
        let endian = if self.big { b'V' } else { b'v' };
        if self.large {
            o.extend(b"17-01");
            o.push(endian);
            o.extend(format!("{:04}", self.version).as_bytes());
        } else {
            o.push(if self.dna.ptr == 4 { b'_' } else { b'-' });
            o.push(endian);
            o.extend(format!("{:03}", self.version).as_bytes());
        }
        for (c, sdna, old, nr, data) in &self.blocks {
            self.bhead(&mut o, *c, *sdna, *old, data.len(), *nr);
            o.extend(data);
        }
        let dna = self.dna.encode(self.big);
        self.bhead(&mut o, *b"DNA1", 0, 0x7f00, dna.len(), 1);
        o.extend(dna);
        self.bhead(&mut o, *b"ENDB", 0, 0, 0, 0);
        o
    }
}

// ---------------------------------------------------------------------------
// The test scene in several Blender generations

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Blender 2.4x/2.7x-style: `Lamp`, `size`, `clipsta`; mesh arrays only via
    /// direct `mvert`/`mpoly`/... pointers, no CustomData layers.
    Legacy27,
    /// Blender 2.8 .. 3.4: MVert/MEdge/MPoly/MLoop/MLoopUV as CustomData layers
    /// (and the direct pointers).
    Legacy30,
    /// Blender 3.6: generic attributes, `poly_offset_indices`, `vdata`/`totvert` names.
    Attr36,
    /// Blender 4.2: `vert_data`/`verts_num` names, `face_offset_indices`, no stored `obmat`.
    Attr42,
    /// Blender 5.0: large header and block heads, 258-byte ID names.
    Attr50,
}

#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    pub ptr: usize,
    pub big: bool,
    pub layout: Layout,
}

impl Cfg {
    fn attr(&self) -> bool {
        matches!(self.layout, Layout::Attr36 | Layout::Attr42 | Layout::Attr50)
    }
    fn new_names(&self) -> bool {
        matches!(self.layout, Layout::Attr42 | Layout::Attr50)
    }
}

fn define(c: &Cfg) -> Dna {
    let mut d = Dna::new(c.ptr);
    let id_name = if c.layout == Layout::Attr50 { "name[258]" } else { "name[66]" };
    d.def("Link", &[("Link", "*next"), ("Link", "*prev")]);
    d.def("ListBase", &[("void", "*first"), ("void", "*last")]);
    d.def("ID", &[("void", "*next"), ("void", "*prev"), ("ID", "*newid"), ("Library", "*lib"), ("char", id_name), ("short", "flag"), ("int", "tag"), ("int", "us"), ("int", "icon_id"), ("char", "_pad[4]")]);
    let layer_name = if c.layout == Layout::Attr50 { "name[68]" } else { "name[64]" };
    d.def(
        "CustomDataLayer",
        &[("int", "type"), ("int", "offset"), ("int", "flag"), ("int", "active"), ("int", "active_rnd"), ("int", "active_clone"), ("int", "active_mask"), ("int", "uid"), ("char", layer_name), ("void", "*data")],
    );
    d.def("CustomData", &[("CustomDataLayer", "*layers"), ("int", "typemap[53]"), ("int", "_pad[1]"), ("int", "totlayer"), ("int", "maxlayer"), ("int", "totsize"), ("BLI_mempool", "*pool"), ("CustomDataExternal", "*external")]);
    d.def("MVert", &[("float", "co[3]"), ("short", "no[3]"), ("char", "flag"), ("char", "bweight")]);
    d.def("MEdge", &[("uint", "v1"), ("uint", "v2"), ("char", "crease"), ("char", "bweight"), ("short", "flag")]);
    d.def("MPoly", &[("int", "loopstart"), ("int", "totloop"), ("short", "mat_nr"), ("char", "flag"), ("char", "_pad")]);
    d.def("MLoop", &[("uint", "v"), ("uint", "e")]);
    d.def("MLoopUV", &[("float", "uv[2]"), ("int", "flag")]);
    let (vd, ed, pd, ld) = if c.new_names() { ("vert_data", "edge_data", "face_data", "corner_data") } else { ("vdata", "edata", "pdata", "ldata") };
    let (tv, te, tp, tl) = if c.new_names() { ("verts_num", "edges_num", "faces_num", "corners_num") } else { ("totvert", "totedge", "totpoly", "totloop") };
    let mut mesh: Vec<(&str, &str)> = vec![("ID", "id"), ("AnimData", "*adt"), ("Key", "*key"), ("Material", "**mat")];
    match c.layout {
        Layout::Legacy27 | Layout::Legacy30 => mesh.extend([("MPoly", "*mpoly"), ("MLoop", "*mloop"), ("MLoopUV", "*mloopuv"), ("MLoopCol", "*mloopcol"), ("MFace", "*mface"), ("MTFace", "*mtface"), ("MVert", "*mvert"), ("MEdge", "*medge")]),
        Layout::Attr36 => mesh.push(("int", "*poly_offset_indices")),
        _ => mesh.push(("int", "*face_offset_indices")),
    }
    mesh.extend([("CustomData", vd), ("CustomData", ed), ("CustomData", if c.new_names() { "fdata_legacy" } else { "fdata" }), ("CustomData", pd), ("CustomData", ld)]);
    mesh.extend([("int", tv), ("int", te), ("int", if c.new_names() { "totface_legacy" } else { "totface" }), ("int", "totselect"), ("int", tp), ("int", tl), ("short", "totcol"), ("char", "_pad2[6]")]);
    d.def("Mesh", &mesh);

    let mut ob: Vec<(&str, &str)> = vec![("ID", "id"), ("AnimData", "*adt"), ("SculptSession", "*sculpt"), ("short", "type"), ("short", "partype"), ("int", "par1"), ("int", "par2"), ("int", "par3"), ("char", "parsubstr[64]"), ("Object", "*parent"), ("Object", "*track"), ("void", "*data"), ("Material", "**mat"), ("char", "*matbits"), ("int", "totcol"), ("int", "actcol")];
    ob.extend([("float", "loc[3]"), ("float", "dloc[3]")]);
    ob.push(("float", if c.layout == Layout::Legacy27 { "size[3]" } else { "scale[3]" }));
    if c.layout != Layout::Legacy27 {
        ob.push(("float", "dscale[3]"));
    } else {
        ob.push(("float", "dsize[3]"));
    }
    ob.extend([("float", "rot[3]"), ("float", "drot[3]"), ("float", "quat[4]"), ("float", "dquat[4]"), ("float", "rotAxis[3]"), ("float", "drotAxis[3]"), ("float", "rotAngle"), ("float", "drotAngle")]);
    match c.layout {
        Layout::Legacy27 | Layout::Legacy30 | Layout::Attr36 => ob.push(("float", "obmat[4][4]")),
        _ => {}
    }
    ob.extend([("float", "parentinv[4][4]"), ("float", "constinv[4][4]"), ("short", "rotmode"), ("char", "_pad3[6]")]);
    d.def("Object", &ob);

    d.def("bNodeSocketValueRGBA", &[("float", "value[4]")]);
    d.def("bNodeSocketValueFloat", &[("int", "subtype"), ("float", "value"), ("float", "min"), ("float", "max")]);
    d.def("bNodeSocket", &[("bNodeSocket", "*next"), ("bNodeSocket", "*prev"), ("IDProperty", "*prop"), ("char", "identifier[64]"), ("char", "name[64]"), ("void", "*storage"), ("short", "type"), ("short", "flag"), ("char", "_pad[4]"), ("void", "*default_value")]);
    d.def("bNode", &[("bNode", "*next"), ("bNode", "*prev"), ("ListBase", "inputs"), ("ListBase", "outputs"), ("char", "name[64]"), ("int", "flag"), ("short", "type"), ("char", "_pad[2]"), ("char", "idname[64]")]);
    d.def("bNodeTree", &[("ID", "id"), ("AnimData", "*adt"), ("ListBase", "nodes"), ("ListBase", "links"), ("int", "type"), ("char", "_pad[4]")]);
    d.def(
        "Material",
        &[("ID", "id"), ("AnimData", "*adt"), ("short", "flag"), ("char", "_pad1[2]"), ("float", "r"), ("float", "g"), ("float", "b"), ("float", "a"), ("float", "specr"), ("float", "specg"), ("float", "specb"), ("float", "alpha"), ("float", "ray_mirror"), ("float", "spec"), ("float", "gloss_mir"), ("float", "roughness"), ("float", "metallic"), ("char", "use_nodes"), ("char", "pr_type"), ("short", "pr_texture"), ("bNodeTree", "*nodetree")],
    );
    let mut la: Vec<(&str, &str)> = vec![("ID", "id"), ("AnimData", "*adt"), ("short", "type"), ("short", "flag"), ("int", "mode"), ("float", "r"), ("float", "g"), ("float", "b"), ("float", "k"), ("float", "energy"), ("float", "dist"), ("float", "spotsize"), ("float", "spotblend"), ("float", "area_size"), ("float", "area_sizey"), ("float", "area_sizez"), ("float", "sun_angle")];
    if c.new_names() {
        la.push(("float", "radius"));
    }
    la.push(("char", "_pad2[4]"));
    d.def(if c.layout == Layout::Legacy27 { "Lamp" } else { "Light" }, &la);
    let (cs, ce) = if c.layout == Layout::Legacy27 { ("clipsta", "clipend") } else { ("clip_start", "clip_end") };
    d.def(
        "Camera",
        &[("ID", "id"), ("AnimData", "*adt"), ("char", "type"), ("char", "dtx"), ("short", "flag"), ("float", "passepartalpha"), ("float", cs), ("float", ce), ("float", "lens"), ("float", "ortho_scale"), ("float", "drawsize"), ("float", "sensor_x"), ("float", "sensor_y"), ("float", "shiftx"), ("float", "shifty"), ("char", "sensor_fit"), ("char", "_pad[7]")],
    );
    d.def("RenderData", &[("int", "cfra"), ("int", "sfra"), ("int", "efra"), ("short", "frs_sec"), ("char", "_pad[2]"), ("int", "xsch"), ("int", "ysch"), ("float", "xasp"), ("float", "yasp"), ("float", "frs_sec_base")]);
    d.def("Scene", &[("ID", "id"), ("AnimData", "*adt"), ("Object", "*camera"), ("World", "*world"), ("Scene", "*set"), ("ListBase", "base"), ("RenderData", "r")]);
    d.def("FileGlobal", &[("char", "subvstr[4]"), ("short", "subversion"), ("short", "minversion"), ("short", "minsubversion"), ("char", "_pad[6]"), ("bScreen", "*curscreen"), ("Scene", "*curscene")]);
    d
}

/// Cube corner positions plus one extra vertex for a loose edge.
pub const CUBE_VERTS: [[f32; 3]; 9] = [[-1., -1., -1.], [1., -1., -1.], [1., 1., -1.], [-1., 1., -1.], [-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.], [0., 0., 3.]];
pub const CUBE_FACES: [[u32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];

/// UV of face `f`, corner `c`: unique per corner so tests can check order.
pub fn cube_uv(f: usize, c: usize) -> [f32; 2] {
    [c as f32 * 0.25, f as f32 * 0.125]
}
/// Faces 0..3 use slot 0, the rest slot 1.
pub fn cube_mat(f: usize) -> i64 {
    (f >= 3) as i64
}
/// Only face 0 is smooth-shaded.
pub fn cube_smooth(f: usize) -> bool {
    f == 0
}

/// Unique cube edges (face edges in order) plus the loose edge 8-0.
pub fn cube_edges() -> Vec<[u32; 2]> {
    let mut e: Vec<[u32; 2]> = Vec::new();
    for f in CUBE_FACES {
        for i in 0..4 {
            let (a, b) = (f[i], f[(i + 1) % 4]);
            if !e.iter().any(|x| (x[0] == a && x[1] == b) || (x[0] == b && x[1] == a)) {
                e.push([a, b]);
            }
        }
    }
    e.push([8, 0]);
    e
}

/// Column-major world matrix of the cube object: T(1,2,3) * Rz(90°) * S(2).
pub fn cube_world() -> [f64; 16] {
    [0., 2., 0., 0., -2., 0., 0., 0., 0., 0., 2., 0., 1., 2., 3., 1.]
}
/// Its inverse (what Blender stores as the child's `parentinv`).
pub fn cube_world_inv() -> [f64; 16] {
    // R^T / 2 and -R^T t / 2 with R = Rz(90°), t = (1, 2, 3).
    [0., -0.5, 0., 0., 0.5, 0., 0., 0., 0., 0., 0.5, 0., -1., 0.5, -1.5, 1.]
}

/// Build the shared test scene in the layout `c` describes.
///
/// Objects: `Cube` (mesh, 2 slots, child `Child`), `Instance` (same mesh),
/// `Camera` (active, 50 mm), `OrthoCam`, `Spot`, `Area`, `Sun`.
/// Materials: `Red` (Principled node), `Blue` (no nodes), `Green` (object-linked slot).
pub fn scene_file(c: Cfg) -> Vec<u8> {
    let version = match c.layout {
        Layout::Legacy27 => 279,
        Layout::Legacy30 => 300,
        Layout::Attr36 => 306,
        Layout::Attr42 => 402,
        Layout::Attr50 => 500,
    };
    let mut w = Writer::new(define(&c), c.big, c.layout == Layout::Attr50, version);
    let id = |code: &str, name: &str| ("id.name", s(&format!("{code}{name}")));

    // Addresses of the IDs, so blocks can refer to each other in any order.
    let [sc, cube, child, inst, cam, ortho, spot, area, sun, me, red, blue, green, cam_d, ortho_d, spot_d, area_d, sun_d] = [(); 18].map(|_| w.alloc());

    let glob = w.alloc();
    let g = w.inst("FileGlobal", &[("curscene", p(sc))]);
    w.put(glob, "GLOB", "FileGlobal", 1, g);
    let scene = w.inst("Scene", &[id("SC", "Main"), ("camera", p(cam)), ("r.sfra", n(10.)), ("r.efra", n(120.)), ("r.frs_sec", n(30.)), ("r.frs_sec_base", n(1.)), ("r.xsch", n(1920.)), ("r.ysch", n(1080.)), ("r.xasp", n(1.)), ("r.yasp", n(1.))]);
    w.put(sc, "SC", "Scene", 1, scene);

    // Materials. Red carries a Principled BSDF node tree.
    let tree = w.alloc();
    let (out_node, bsdf) = (w.alloc(), w.alloc());
    let (s_base, s_metal, s_rough) = (w.alloc(), w.alloc(), w.alloc());
    let mat = w.inst("Material", &[id("MA", "Red"), ("r", n(0.8)), ("g", n(0.8)), ("b", n(0.8)), ("a", n(1.)), ("roughness", n(0.5)), ("use_nodes", n(1.)), ("nodetree", p(tree))]);
    w.put(red, "MA", "Material", 1, mat);
    let t = w.inst("bNodeTree", &[id("NT", "Shader Nodetree"), ("nodes.first", p(out_node)), ("nodes.last", p(bsdf))]);
    w.put(tree, "DATA", "bNodeTree", 1, t);
    let nd = w.inst("bNode", &[("next", p(bsdf)), ("name", s("Material Output")), ("idname", s("ShaderNodeOutputMaterial"))]);
    w.put(out_node, "DATA", "bNode", 1, nd);
    let nd = w.inst("bNode", &[("prev", p(out_node)), ("inputs.first", p(s_base)), ("inputs.last", p(s_rough)), ("name", s("Principled BSDF")), ("idname", s("ShaderNodeBsdfPrincipled"))]);
    w.put(bsdf, "DATA", "bNode", 1, nd);
    let sockets = [(s_base, "Base Color", Some(s_metal), true, vec![0.8, 0.1, 0.1, 1.0]), (s_metal, "Metallic", Some(s_rough), false, vec![0.5]), (s_rough, "Roughness", None, false, vec![0.3])];
    for (ptr, name, next, rgba, val) in sockets {
        let dv = if rgba {
            w.add("DATA", "bNodeSocketValueRGBA", &[("value", nv(&val))])
        } else {
            w.add("DATA", "bNodeSocketValueFloat", &[("value", n(val[0])), ("max", n(1.))])
        };
        let ident = name.replace(' ', "_");
        let sk = w.inst("bNodeSocket", &[("next", p(next.unwrap_or(0))), ("identifier", s(&ident)), ("name", s(name)), ("default_value", p(dv))]);
        w.put(ptr, "DATA", "bNodeSocket", 1, sk);
    }
    let mat = w.inst("Material", &[id("MA", "Blue"), ("r", n(0.1)), ("g", n(0.2)), ("b", n(0.9)), ("a", n(1.)), ("metallic", n(1.)), ("roughness", n(0.2))]);
    w.put(blue, "MA", "Material", 1, mat);
    let mat = w.inst("Material", &[id("MA", "Green"), ("r", n(0.1)), ("g", n(0.8)), ("b", n(0.1)), ("a", n(1.))]);
    w.put(green, "MA", "Material", 1, mat);

    // Mesh.
    write_mesh(&mut w, &c, me, red, blue);

    // Objects.
    let rot_fields = |mode: i64| -> Vec<(&'static str, V)> {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let mut f = vec![("rotmode", n(mode as f64))];
        match mode {
            0 => f.push(("quat", nv(&[h, 0., 0., h]))),
            -1 => {
                f.push(("rotAxis", nv(&[0., 0., 1.])));
                f.push(("rotAngle", n(FRAC_PI_2)));
            }
            _ => f.push(("rot", nv(&[0., 0., FRAC_PI_2]))),
        }
        f
    };
    let mode = match c.layout {
        Layout::Legacy27 => 0,
        Layout::Legacy30 | Layout::Attr50 => 1,
        Layout::Attr36 => -1,
        Layout::Attr42 => 6,
    };
    let scale_name = if c.layout == Layout::Legacy27 { "size" } else { "scale" };
    let has_obmat = !c.new_names();
    let ident = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
    let object = |w: &mut Writer, ptr: u64, name: &str, ty: f64, data: u64, extra: Vec<(&str, V)>, world: [f64; 16]| {
        let mut f = vec![id("OB", name), ("type", n(ty)), ("data", p(data)), (scale_name, nv(&[1., 1., 1.])), ("parentinv", nv(&ident)), ("rotmode", n(1.))];
        if scale_name == "scale" {
            f.push(("dscale", nv(&[1., 1., 1.])));
        }
        if has_obmat {
            f.push(("obmat", nv(&world)));
        }
        f.extend(extra);
        let d = w.inst("Object", &f);
        w.put(ptr, "OB", "Object", 1, d);
    };

    let ob_mat = w.raw(w.ptrs(&[0, green]), 2);
    let bits = w.raw(vec![0, 1], 2);
    let mut cube_f: Vec<(&str, V)> = vec![("loc", nv(&[1., 2., 3.])), (scale_name, nv(&[2., 2., 2.])), ("mat", p(ob_mat)), ("matbits", p(bits)), ("totcol", n(2.))];
    cube_f.extend(rot_fields(mode));
    object(&mut w, cube, "Cube", 1., me, cube_f, cube_world());
    object(&mut w, child, "Child", 0., 0, vec![("loc", nv(&[0., 0., 1.])), ("parent", p(cube)), ("parentinv", nv(&cube_world_inv()))], [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 1., 1.]);
    let inst_mat = w.raw(w.ptrs(&[0, 0]), 2);
    let inst_bits = w.raw(vec![0, 0], 2);
    object(&mut w, inst, "Instance", 1., me, vec![("loc", nv(&[-5., 0., 0.])), ("mat", p(inst_mat)), ("matbits", p(inst_bits)), ("totcol", n(2.))], ident);
    let mut tw = ident;
    tw[12..15].copy_from_slice(&[7., -7., 5.]);
    object(&mut w, cam, "Camera", 11., cam_d, vec![("loc", nv(&[7., -7., 5.]))], tw);
    object(&mut w, ortho, "OrthoCam", 11., ortho_d, vec![], ident);
    object(&mut w, spot, "Spot", 10., spot_d, vec![], ident);
    object(&mut w, area, "Area", 10., area_d, vec![], ident);
    object(&mut w, sun, "Sun", 10., sun_d, vec![], ident);

    // Camera and light data.
    let (cs, ce) = if c.layout == Layout::Legacy27 { ("clipsta", "clipend") } else { ("clip_start", "clip_end") };
    let d = w.inst("Camera", &[id("CA", "Camera"), ("lens", n(50.)), ("sensor_x", n(36.)), ("sensor_y", n(24.)), (cs, n(0.1)), (ce, n(100.)), ("ortho_scale", n(7.))]);
    w.put(cam_d, "CA", "Camera", 1, d);
    let d = w.inst("Camera", &[id("CA", "Ortho"), ("type", n(1.)), ("lens", n(50.)), ("sensor_x", n(36.)), (cs, n(0.5)), (ce, n(50.)), ("ortho_scale", n(10.))]);
    w.put(ortho_d, "CA", "Camera", 1, d);
    let la = if c.layout == Layout::Legacy27 { "Lamp" } else { "Light" };
    let radius = if c.new_names() { ("radius", n(0.25)) } else { ("area_size", n(0.25)) };
    let d = w.inst(la, &[id("LA", "Spot"), ("type", n(2.)), ("r", n(1.)), ("g", n(0.5)), ("b", n(0.25)), ("energy", n(1000.)), ("spotsize", n(0.785398)), ("spotblend", n(0.2)), radius]);
    w.put(spot_d, "LA", la, 1, d);
    let d = w.inst(la, &[id("LA", "Area"), ("type", n(4.)), ("r", n(1.)), ("g", n(1.)), ("b", n(1.)), ("energy", n(500.)), ("area_size", n(2.))]);
    w.put(area_d, "LA", la, 1, d);
    let d = w.inst(la, &[id("LA", "Sun"), ("type", n(1.)), ("r", n(1.)), ("g", n(1.)), ("b", n(0.9)), ("energy", n(3.)), ("sun_angle", n(0.01))]);
    w.put(sun_d, "LA", la, 1, d);
    w.finish()
}

/// The cube mesh in the layout's representation.
fn write_mesh(w: &mut Writer, c: &Cfg, me: u64, m0: u64, m1: u64) {
    let edges = cube_edges();
    let nverts = CUBE_VERTS.len();
    let (ne, nf, nl) = (edges.len(), CUBE_FACES.len(), CUBE_FACES.len() * 4);
    let corners: Vec<u32> = CUBE_FACES.iter().flatten().copied().collect();
    let edge_of = |a: u32, b: u32| edges.iter().position(|e| (e[0] == a && e[1] == b) || (e[0] == b && e[1] == a)).unwrap() as f64;
    let mut uvs = Vec::new();
    for f in 0..nf {
        for k in 0..4 {
            uvs.extend(cube_uv(f, k));
        }
    }
    let mat = w.raw(w.ptrs(&[m0, m1]), 2);
    let mut fields: Vec<(String, V)> = vec![("id.name".into(), s("MECube")), ("mat".into(), p(mat)), ("totcol".into(), n(2.))];
    let (vd, ed, pd, ld) = if c.new_names() { ("vert_data", "edge_data", "face_data", "corner_data") } else { ("vdata", "edata", "pdata", "ldata") };
    let (tv, te, tp, tl) = if c.new_names() { ("verts_num", "edges_num", "faces_num", "corners_num") } else { ("totvert", "totedge", "totpoly", "totloop") };
    for (k, v) in [(tv, nverts), (te, ne), (tp, nf), (tl, nl)] {
        fields.push((k.into(), n(v as f64)));
    }
    // (CustomData member, [(type, name, data ptr)])
    let mut cds: Vec<(&str, Vec<(i64, &str, u64)>)> = Vec::new();
    if c.attr() {
        let pos = w.raw(w.f32s(&CUBE_VERTS.concat()), nverts);
        let ev: Vec<i64> = edges.iter().flat_map(|e| [e[0] as i64, e[1] as i64]).collect();
        let ev = w.raw(w.i32s(&ev), ne);
        let cv = w.raw(w.i32s(&corners.iter().map(|&v| v as i64).collect::<Vec<_>>()), nl);
        let ce: Vec<i64> = CUBE_FACES.iter().flat_map(|f| (0..4).map(move |i| (f[i], f[(i + 1) % 4]))).map(|(a, b)| edge_of(a, b) as i64).collect();
        let ce = w.raw(w.i32s(&ce), nl);
        // A decoy float2 layer that is not a UV map (dot name) comes first.
        let decoy = w.raw(w.f32s(&vec![9.0; nl * 2]), nl);
        let uv = w.raw(w.f32s(&uvs), nl);
        let pin = w.raw(vec![0; nl], nl);
        let mi = w.raw(w.i32s(&(0..nf).map(cube_mat).collect::<Vec<_>>()), nf);
        let sharp = w.raw((0..nf).map(|f| (!cube_smooth(f)) as u8).collect(), nf);
        let offs: Vec<i64> = (0..=nf).map(|i| i as i64 * 4).collect();
        let offs = w.raw(w.i32s(&offs), nf + 1);
        let off_name = if c.layout == Layout::Attr36 { "poly_offset_indices" } else { "face_offset_indices" };
        fields.push((off_name.into(), p(offs)));
        cds.push((vd, vec![(48, "position", pos)]));
        cds.push((ed, vec![(53, ".edge_verts", ev)]));
        cds.push((pd, vec![(11, "material_index", mi), (50, "sharp_face", sharp)]));
        cds.push((ld, vec![(11, ".corner_vert", cv), (11, ".corner_edge", ce), (49, ".decoy", decoy), (49, "UVMap", uv), (50, ".pn.UVMap", pin)]));
    } else {
        let mut verts = Vec::new();
        for v in CUBE_VERTS {
            verts.extend(w.inst("MVert", &[("co", nv_f32(&v)), ("no", nv(&[0., 0., 32767.])), ("flag", n(1.))]));
        }
        let mvert = w.alloc();
        w.put(mvert, "DATA", "MVert", nverts, verts);
        let mut eb = Vec::new();
        for e in &edges {
            eb.extend(w.inst("MEdge", &[("v1", n(e[0] as f64)), ("v2", n(e[1] as f64)), ("flag", n(3.))]));
        }
        let medge = w.alloc();
        w.put(medge, "DATA", "MEdge", ne, eb);
        let mut pb = Vec::new();
        for f in 0..nf {
            pb.extend(w.inst("MPoly", &[("loopstart", n(f as f64 * 4.)), ("totloop", n(4.)), ("mat_nr", n(cube_mat(f) as f64)), ("flag", n((cube_smooth(f) as u8 | 2) as f64))]));
        }
        let mpoly = w.alloc();
        w.put(mpoly, "DATA", "MPoly", nf, pb);
        let mut lb = Vec::new();
        for (i, &v) in corners.iter().enumerate() {
            let (f, k) = (i / 4, i % 4);
            let next = CUBE_FACES[f][(k + 1) % 4];
            lb.extend(w.inst("MLoop", &[("v", n(v as f64)), ("e", n(edge_of(v, next)))]));
        }
        let mloop = w.alloc();
        w.put(mloop, "DATA", "MLoop", nl, lb);
        let mut ub = Vec::new();
        for uv in uvs.chunks(2) {
            ub.extend(w.inst("MLoopUV", &[("uv", nv(&[uv[0] as f64, uv[1] as f64]))]));
        }
        let mloopuv = w.alloc();
        w.put(mloopuv, "DATA", "MLoopUV", nl, ub);
        for (k, v) in [("mvert", mvert), ("medge", medge), ("mpoly", mpoly), ("mloop", mloop), ("mloopuv", mloopuv)] {
            fields.push((k.into(), p(v)));
        }
        if c.layout == Layout::Legacy30 {
            cds.push((vd, vec![(0, "", mvert)]));
            cds.push((ed, vec![(3, "", medge)]));
            cds.push((pd, vec![(25, "", mpoly)]));
            cds.push((ld, vec![(26, "", mloop), (16, "UVMap", mloopuv)]));
        }
    }
    for (cd, ls) in cds {
        let mut lb = Vec::new();
        for (ty, name, data) in &ls {
            lb.extend(w.inst("CustomDataLayer", &[("type", n(*ty as f64)), ("name", s(name)), ("data", p(*data)), ("active_rnd", n(0.))]));
        }
        let lp = w.alloc();
        w.put(lp, "DATA", "CustomDataLayer", ls.len(), lb);
        fields.push((format!("{cd}.layers"), p(lp)));
        fields.push((format!("{cd}.totlayer"), n(ls.len() as f64)));
        fields.push((format!("{cd}.maxlayer"), n(ls.len() as f64)));
    }
    let d = w.inst("Mesh", &fields);
    w.put(me, "ME", "Mesh", 1, d);
}

fn nv_f32(v: &[f32]) -> V {
    V::N(v.iter().map(|&x| x as f64).collect())
}

// ---------------------------------------------------------------------------
// Compression helpers

pub fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// zstd via ruzstd's encoder.
pub fn zstd(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

/// Hand-made zstd: two frames of raw (stored) blocks, then a skippable frame,
/// the shape of Blender's seekable zstd files.
pub fn zstd_raw_frames(data: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    let mid = data.len() / 2;
    for part in [&data[..mid], &data[mid..]] {
        o.extend(0xFD2F_B528u32.to_le_bytes());
        // Frame header: single segment, 4-byte content size, no checksum.
        o.push(0b1010_0000);
        o.extend((part.len() as u32).to_le_bytes());
        let chunks: Vec<&[u8]> = if part.is_empty() { vec![&[][..]] } else { part.chunks(50_000).collect() };
        for (i, ch) in chunks.iter().enumerate() {
            let last = (i + 1 == chunks.len()) as u32;
            let h = last | ((ch.len() as u32) << 3); // block type 0 = raw
            o.extend(&h.to_le_bytes()[..3]);
            o.extend(*ch);
        }
    }
    o.extend(0x184D_2A5Eu32.to_le_bytes());
    o.extend(8u32.to_le_bytes());
    o.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    o
}

/// Build a lookup of object names to indices.
pub fn names(scene: &sex_3d::Scene) -> HashMap<String, usize> {
    scene.objects.iter().enumerate().map(|(i, o)| (o.name.clone(), i)).collect()
}
