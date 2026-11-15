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
    ob.extend([("float", "parentinv[4][4]"), ("float", "constinv[4][4]"), ("short", "rotmode"), ("char", "_pad3[6]"), ("ListBase", "modifiers")]);
    match c.layout {
        // 2.8x keeps the "disable in viewports" bit in `restrictflag`.
        Layout::Legacy27 | Layout::Legacy30 => ob.extend([("short", "restrictflag"), ("char", "_pad4[6]")]),
        _ => ob.extend([("char", "visibility_flag"), ("char", "_pad4[7]")]),
    }
    d.def("Object", &ob);

    // Modifiers: every concrete struct starts with `ModifierData modifier`.
    d.def("ModifierData", &[("ModifierData", "*next"), ("ModifierData", "*prev"), ("int", "type"), ("int", "mode"), ("float", "execution_time"), ("short", "flag"), ("short", "ui_expand_flag"), ("char", "name[64]"), ("char", "*error"), ("char", "_pad[8]")]);
    let md = ("ModifierData", "modifier");
    d.def("MirrorModifierData", &[md, ("short", "axis"), ("short", "flag"), ("float", "tolerance"), ("float", "bisect_threshold"), ("char", "use_correct_order_on_merge"), ("char", "_pad[3]"), ("float", "uv_offset[2]"), ("float", "uv_offset_copy[2]"), ("Object", "*mirror_ob")]);
    d.def("SubsurfModifierData", &[md, ("short", "subdivType"), ("short", "levels"), ("short", "renderLevels"), ("short", "flags"), ("short", "uv_smooth"), ("short", "quality"), ("short", "boundary_smooth"), ("char", "_pad[2]"), ("void", "*emCache"), ("void", "*mCache")]);
    d.def(
        "ArrayModifierData",
        &[md, ("Object", "*start_cap"), ("Object", "*end_cap"), ("Object", "*curve_ob"), ("Object", "*offset_ob"), ("float", "offset[3]"), ("float", "scale[3]"), ("float", "length"), ("float", "merge_dist"), ("int", "fit_type"), ("int", "offset_type"), ("int", "flags"), ("int", "count"), ("float", "uv_offset[2]")],
    );
    d.def("SolidifyModifierData", &[md, ("char", "defgrp_name[64]"), ("char", "shell_defgrp_name[64]"), ("char", "rim_defgrp_name[64]"), ("float", "offset"), ("float", "offset_fac"), ("float", "offset_fac_vg"), ("float", "offset_clamp"), ("char", "mode"), ("char", "_pad[3]")]);
    let mut bevel: Vec<(&str, &str)> = vec![md, ("float", "value"), ("int", "res"), ("short", "flags"), ("short", "val_flags"), ("short", "profile_type"), ("short", "lim_flags"), ("short", "e_flags"), ("short", "mat"), ("short", "edge_flags"), ("short", "face_str_mode")];
    if c.layout == Layout::Legacy30 {
        bevel.push(("char", "_pad[4]"));
    } else {
        bevel.extend([("char", "affect_type"), ("char", "_pad[3]")]);
    }
    bevel.extend([("float", "profile"), ("float", "bevel_angle"), ("float", "spread"), ("char", "defgrp_name[64]")]);
    d.def("BevelModifierData", &bevel);
    d.def("TriangulateModifierData", &[md, ("int", "flag"), ("int", "quad_method"), ("int", "ngon_method"), ("int", "min_vertices")]);
    d.def("WeldModifierData", &[md, ("float", "merge_dist"), ("char", "defgrp_name[64]"), ("char", "mode"), ("char", "flag"), ("char", "_pad[2]")]);
    d.def(
        "DisplaceModifierData",
        &[md, ("Tex", "*texture"), ("Object", "*map_object"), ("char", "map_bone[64]"), ("char", "defgrp_name[64]"), ("float", "midlevel"), ("int", "direction"), ("char", "uvlayer_name[64]"), ("int", "uvlayer_tmp"), ("int", "texmapping"), ("float", "strength"), ("int", "flag"), ("short", "space"), ("char", "_pad[6]")],
    );
    d.def("SmoothModifierData", &[md, ("float", "fac"), ("char", "defgrp_name[64]"), ("short", "flag"), ("short", "repeat")]);
    d.def("WaveModifierData", &[md, ("float", "height"), ("float", "width"), ("float", "narrow"), ("float", "speed")]);

    // Collections (2.8+).
    d.def("CollectionObject", &[("CollectionObject", "*next"), ("CollectionObject", "*prev"), ("Object", "*ob"), ("void", "*_pad")]);
    d.def("CollectionChild", &[("CollectionChild", "*next"), ("CollectionChild", "*prev"), ("Collection", "*collection"), ("void", "*_pad")]);
    let col_flag = if c.layout == Layout::Legacy30 { ("short", "flag") } else { ("uchar", "flag") };
    d.def("Collection", &[("ID", "id"), ("ID", "*owner_id"), ("ListBase", "gobject"), ("ListBase", "children"), ("char", "_pad0[4]"), ("int", "color_tag"), col_flag, ("char", "_pad1[5]")]);

    // World.
    d.def("World", &[("ID", "id"), ("AnimData", "*adt"), ("short", "colormodel"), ("short", "totex"), ("short", "texact"), ("short", "mistype"), ("float", "horr"), ("float", "horg"), ("float", "horb"), ("float", "exposure"), ("bNodeTree", "*nodetree"), ("short", "use_nodes"), ("char", "_pad[6]")]);

    // Animation.
    let mut adt: Vec<(&str, &str)> = vec![("bAction", "*action"), ("bAction", "*tmpact"), ("ListBase", "nla_tracks"), ("NlaTrack", "*act_track"), ("int", "flag"), ("int", "recalc")];
    if c.layout == Layout::Attr50 {
        adt.extend([("int", "slot_handle"), ("char", "last_slot_identifier[66]"), ("char", "_pad[2]")]);
    }
    d.def("AnimData", &adt);
    d.def("BezTriple", &[("float", "vec[3][3]"), ("float", "tilt"), ("float", "weight"), ("float", "radius"), ("char", "ipo"), ("char", "h1"), ("char", "h2"), ("char", "f1"), ("char", "f2"), ("char", "f3"), ("char", "hide"), ("char", "easing"), ("float", "back"), ("float", "amplitude"), ("float", "period"), ("char", "auto_handle_type"), ("char", "_pad[3]")]);
    d.def("FPoint", &[("float", "vec[2]"), ("int", "flag"), ("int", "_pad")]);
    d.def(
        "FCurve",
        &[("FCurve", "*next"), ("FCurve", "*prev"), ("bActionGroup", "*grp"), ("ChannelDriver", "*driver"), ("ListBase", "modifiers"), ("BezTriple", "*bezt"), ("FPoint", "*fpt"), ("uint", "totvert"), ("int", "active_keyframe_index"), ("float", "curval"), ("short", "flag"), ("short", "extend"), ("char", "auto_smoothing"), ("char", "_pad[3]"), ("int", "array_index"), ("char", "*rna_path"), ("int", "color_mode"), ("float", "color[3]")],
    );
    let mut act: Vec<(&str, &str)> = vec![("ID", "id"), ("ListBase", "curves"), ("ListBase", "chanbase"), ("ListBase", "groups"), ("ListBase", "markers"), ("int", "flag"), ("int", "active_marker"), ("int", "idroot"), ("char", "_pad[4]")];
    if c.layout == Layout::Attr50 {
        d.def("ActionChannelbag", &[("int", "slot_handle"), ("int", "fcurve_array_num"), ("FCurve", "**fcurve_array"), ("bActionGroup", "**group_array"), ("int", "group_array_num"), ("char", "_pad[4]")]);
        d.def("ActionStripKeyframeData", &[("ActionChannelbag", "**channelbag_array"), ("int", "channelbag_array_num"), ("char", "_pad[4]")]);
        act.extend([
            ("ActionLayer", "**layer_array"),
            ("int", "layer_array_num"),
            ("int", "layer_active_index"),
            ("ActionSlot", "**slot_array"),
            ("int", "slot_array_num"),
            ("int", "last_slot_handle"),
            ("ActionStripKeyframeData", "**strip_keyframe_data_array"),
            ("int", "strip_keyframe_data_array_num"),
            ("char", "_pad1[4]"),
        ]);
    }
    d.def("bAction", &act);

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
    d.def("Scene", &[("ID", "id"), ("AnimData", "*adt"), ("Object", "*camera"), ("World", "*world"), ("Scene", "*set"), ("ListBase", "base"), ("RenderData", "r"), ("Collection", "*master_collection")]);
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
// Modifiers, collections, parenting, world and animation

/// Inverse of `Root`'s world matrix T(1,0,0) Rz(90°), stored as `Mid`'s parentinv.
pub const ROOT_INV: [f64; 16] = [0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 1., 0., 1.];

/// World colour and strength the extras scene should import with: the
/// Background node, except for the 2.8-era file whose world uses no nodes.
pub fn extras_world(c: Cfg) -> ([f32; 3], f32) {
    if c.layout == Layout::Legacy30 {
        ([0.05, 0.06, 0.07], 1.0)
    } else {
        ([0.1, 0.2, 0.3], 2.5)
    }
}

/// F-curves written on `Anim`'s action: (rna path, index, keys (frame, value,
/// ipo), stored as baked FPoints). Only the transform ones are imported.
pub fn extras_curves() -> Vec<(&'static str, usize, Vec<(f64, f64, u8)>, bool)> {
    vec![
        ("location", 0, vec![(1., 0., 2), (10., 5., 1)], false),
        ("hide_viewport", 0, vec![(1., 0., 0)], false),
        ("rotation_euler", 2, vec![(1., 0., 0), (20., 3., 2)], false),
        ("scale", 1, vec![(1., 1., 1), (2., 2., 1)], true),
        ("location", 1, vec![], false),
    ]
}

fn version_of(c: &Cfg) -> u32 {
    match c.layout {
        Layout::Legacy27 => 279,
        Layout::Legacy30 => 300,
        Layout::Attr36 => 306,
        Layout::Attr42 => 402,
        Layout::Attr50 => 500,
    }
}

/// A ListBase of `sname` link structs whose `member` points at `targets`.
fn links(w: &mut Writer, sname: &str, member: &str, targets: &[u64]) -> (u64, u64) {
    let addrs: Vec<u64> = targets.iter().map(|_| w.alloc()).collect();
    for (i, &t) in targets.iter().enumerate() {
        let prev = if i > 0 { addrs[i - 1] } else { 0 };
        let d = w.inst(sname, &[("next", p(addrs.get(i + 1).copied().unwrap_or(0))), ("prev", p(prev)), (member, p(t))]);
        w.put(addrs[i], "DATA", sname, 1, d);
    }
    (addrs.first().copied().unwrap_or(0), addrs.last().copied().unwrap_or(0))
}

#[allow(clippy::too_many_arguments)]
fn collection(w: &mut Writer, ptr: u64, code: &str, name: &str, flag: f64, obs: &[u64], kids: &[u64]) {
    let (of, ol) = links(w, "CollectionObject", "ob", obs);
    let (cf, cl) = links(w, "CollectionChild", "collection", kids);
    let d = w.inst(
        "Collection",
        &[("id.name", s(&format!("GR{name}"))), ("gobject.first", p(of)), ("gobject.last", p(ol)), ("children.first", p(cf)), ("children.last", p(cl)), ("flag", n(flag))],
    );
    w.put(ptr, code, "Collection", 1, d);
}

/// One FCurve block at `ptr` with its keys and RNA path.
fn fcurve(w: &mut Writer, ptr: u64, next: u64, prev: u64, path: &str, index: usize, keys: &[(f64, f64, u8)], baked: bool) {
    let rna = w.raw(format!("{path}\0").into_bytes(), 1);
    let mut f = vec![("next", p(next)), ("prev", p(prev)), ("array_index", n(index as f64)), ("rna_path", p(rna)), ("totvert", n(keys.len() as f64))];
    if !keys.is_empty() {
        let addr = w.alloc();
        let mut data = Vec::new();
        if baked {
            for &(fr, v, _) in keys {
                data.extend(w.inst("FPoint", &[("vec", nv(&[fr, v]))]));
            }
            w.put(addr, "DATA", "FPoint", keys.len(), data);
            f.push(("fpt", p(addr)));
        } else {
            for &(fr, v, ipo) in keys {
                data.extend(w.inst("BezTriple", &[("vec", nv(&[fr - 1., v, 0., fr, v, 0., fr + 1., v, 0.])), ("ipo", n(ipo as f64)), ("h1", n(1.)), ("h2", n(1.))]));
            }
            w.put(addr, "DATA", "BezTriple", keys.len(), data);
            f.push(("bezt", p(addr)));
        }
    }
    let d = w.inst("FCurve", &f);
    w.put(ptr, "DATA", "FCurve", 1, d);
}

/// A 2.8+ scene exercising modifiers, collections, parenting, the world and
/// object animation. Not available for `Legacy27` (no collections then).
///
/// Objects: `Mod` (mesh with a modifier stack, in `Props` and `Nested`),
/// `Root` (empty at (1,0,0) turned 90° about Z, in the master collection),
/// `Mid` (child of `Root` with its parent inverse, disabled in viewports, in
/// `Props`), `Leaf` (child of `Mid`, identity parent inverse, in `Hidden`),
/// `Anim` (empty with an action, in `Nested`).
/// Collections: master -> `Props` -> `Nested` (-> `Hidden` again), master ->
/// `Hidden` (disabled in viewports).
pub fn extras_file(c: Cfg) -> Vec<u8> {
    assert!(c.layout != Layout::Legacy27);
    let mut w = Writer::new(define(&c), c.big, c.layout == Layout::Attr50, version_of(&c));
    let id = |code: &str, name: &str| ("id.name", s(&format!("{code}{name}")));
    let [sc, world, me, mod_ob, root, mid, leaf, anim, master, props, nested, hidden, action, adt] = [(); 14].map(|_| w.alloc());

    let glob = w.alloc();
    let g = w.inst("FileGlobal", &[("curscene", p(sc))]);
    w.put(glob, "GLOB", "FileGlobal", 1, g);
    let d = w.inst(
        "Scene",
        &[id("SC", "Extras"), ("camera", p(0)), ("world", p(world)), ("master_collection", p(master)), ("r.sfra", n(1.)), ("r.efra", n(48.)), ("r.frs_sec", n(25.)), ("r.frs_sec_base", n(1.)), ("r.xsch", n(800.)), ("r.ysch", n(600.)), ("r.xasp", n(1.)), ("r.yasp", n(1.))],
    );
    w.put(sc, "SC", "Scene", 1, d);

    // World: viewport colour plus, from 2.9 on, a Background node.
    let nodes = c.layout != Layout::Legacy30;
    let mut wf = vec![id("WO", "World"), ("horr", n(0.05)), ("horg", n(0.06)), ("horb", n(0.07)), ("use_nodes", n(nodes as u8 as f64))];
    let tree = w.alloc();
    let (bg, s_col, s_str) = (w.alloc(), w.alloc(), w.alloc());
    let t = w.inst("bNodeTree", &[id("NT", "Shader Nodetree"), ("nodes.first", p(bg)), ("nodes.last", p(bg))]);
    w.put(tree, "DATA", "bNodeTree", 1, t);
    let nd = w.inst("bNode", &[("inputs.first", p(s_col)), ("inputs.last", p(s_str)), ("name", s("Background")), ("idname", s("ShaderNodeBackground"))]);
    w.put(bg, "DATA", "bNode", 1, nd);
    let dv = w.add("DATA", "bNodeSocketValueRGBA", &[("value", nv(&[0.1, 0.2, 0.3, 1.0]))]);
    let sk = w.inst("bNodeSocket", &[("next", p(s_str)), ("identifier", s("Color")), ("name", s("Color")), ("default_value", p(dv))]);
    w.put(s_col, "DATA", "bNodeSocket", 1, sk);
    let dv = w.add("DATA", "bNodeSocketValueFloat", &[("value", n(2.5))]);
    let sk = w.inst("bNodeSocket", &[("prev", p(s_col)), ("identifier", s("Strength")), ("name", s("Strength")), ("default_value", p(dv))]);
    w.put(s_str, "DATA", "bNodeSocket", 1, sk);
    wf.push(("nodetree", p(tree)));
    let d = w.inst("World", &wf);
    w.put(world, "WO", "World", 1, d);

    write_mesh(&mut w, &c, me, 0, 0);

    // Modifier stack; the Wave, render-only, vertex-bevel and textured
    // displace entries must be skipped.
    let edge_bevel: Vec<(&str, V)> = if c.layout == Layout::Legacy30 { vec![("flags", n(0.))] } else { vec![("affect_type", n(1.))] };
    let vert_bevel: Vec<(&str, V)> = if c.layout == Layout::Legacy30 { vec![("flags", n(2.))] } else { vec![("affect_type", n(0.))] };
    let mut bevel = vec![("value", n(0.05)), ("res", n(3.)), ("lim_flags", n(8.)), ("bevel_angle", n(0.6))];
    bevel.extend(edge_bevel);
    let mods: Vec<(&str, Vec<(&str, V)>)> = vec![
        ("MirrorModifierData", vec![("flag", n((1 << 3 | 1 << 5 | 1 << 6) as f64)), ("tolerance", n(0.01))]),
        ("SubsurfModifierData", vec![("levels", n(2.)), ("renderLevels", n(3.))]),
        ("WaveModifierData", vec![("height", n(0.5))]),
        ("SubsurfModifierData", vec![("modifier.mode", n(2.)), ("levels", n(4.))]),
        ("ArrayModifierData", vec![("count", n(3.)), ("offset_type", n(3.)), ("scale", nv(&[1.5, 0., 0.])), ("offset", nv(&[0., 0., 2.])), ("flags", n(1.))]),
        ("SolidifyModifierData", vec![("offset", n(0.2)), ("offset_fac", n(0.5))]),
        ("BevelModifierData", bevel),
        ("BevelModifierData", vert_bevel),
        ("TriangulateModifierData", vec![]),
        ("WeldModifierData", vec![("merge_dist", n(0.02))]),
        ("DisplaceModifierData", vec![("strength", n(2.)), ("midlevel", n(0.25)), ("direction", n(3.))]),
        ("DisplaceModifierData", vec![("texture", p(0xDEAD_0)), ("strength", n(2.)), ("direction", n(3.))]),
        ("SmoothModifierData", vec![("fac", n(0.7)), ("repeat", n(4.))]),
    ];
    let addrs: Vec<u64> = mods.iter().map(|_| w.alloc()).collect();
    for (i, (sname, fields)) in mods.into_iter().enumerate() {
        let prev = if i > 0 { addrs[i - 1] } else { 0 };
        let mut f: Vec<(&str, V)> = vec![("modifier.mode", n(3.)), ("modifier.type", n(i as f64 + 1.)), ("modifier.name", s(sname)), ("modifier.next", p(addrs.get(i + 1).copied().unwrap_or(0))), ("modifier.prev", p(prev))];
        f.extend(fields);
        let d = w.inst(sname, &f);
        w.put(addrs[i], "DATA", sname, 1, d);
    }

    // Objects.
    let ident = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
    let has_obmat = !c.new_names();
    let hide_name = if c.layout == Layout::Legacy30 { "restrictflag" } else { "visibility_flag" };
    // "Disable in renders" alone: bit 2 of restrictflag, bit 1 of visibility_flag.
    let render_off = if c.layout == Layout::Legacy30 { 4. } else { 2. };
    let object = |w: &mut Writer, ptr: u64, name: &str, ty: f64, data: u64, extra: Vec<(&str, V)>| {
        let mut f = vec![id("OB", name), ("type", n(ty)), ("data", p(data)), ("scale", nv(&[1., 1., 1.])), ("dscale", nv(&[1., 1., 1.])), ("parentinv", nv(&ident)), ("rotmode", n(1.))];
        if has_obmat {
            f.push(("obmat", nv(&ident))); // stale; the loc/rot/scale basis wins
        }
        f.extend(extra);
        let d = w.inst("Object", &f);
        w.put(ptr, "OB", "Object", 1, d);
    };
    object(&mut w, mod_ob, "Mod", 1., me, vec![("modifiers.first", p(addrs[0])), ("modifiers.last", p(*addrs.last().unwrap())), (hide_name, n(render_off))]);
    object(&mut w, root, "Root", 0., 0, vec![("loc", nv(&[1., 0., 0.])), ("rot", nv(&[0., 0., FRAC_PI_2]))]);
    object(&mut w, mid, "Mid", 0., 0, vec![("loc", nv(&[0., 2., 0.])), ("parent", p(root)), ("parentinv", nv(&ROOT_INV)), (hide_name, n(1.))]);
    object(&mut w, leaf, "Leaf", 0., 0, vec![("loc", nv(&[0., 0., 3.])), ("parent", p(mid))]);
    object(&mut w, anim, "Anim", 0., 0, vec![("adt", p(adt))]);

    // Collections.
    collection(&mut w, master, "DATA", "Scene Collection", 0., &[root], &[props, hidden]);
    collection(&mut w, props, "GR", "Props", 0., &[mod_ob, mid], &[nested]);
    collection(&mut w, nested, "GR", "Nested", 0., &[anim, mod_ob], &[hidden]);
    collection(&mut w, hidden, "GR", "Hidden", 1., &[leaf], &[]);

    // Animation.
    let curves = extras_curves();
    let fc: Vec<u64> = curves.iter().map(|_| w.alloc()).collect();
    let slotted = c.layout == Layout::Attr50;
    for (i, (path, index, keys, baked)) in curves.iter().enumerate() {
        let (next, prev) = if slotted { (0, 0) } else { (fc.get(i + 1).copied().unwrap_or(0), if i > 0 { fc[i - 1] } else { 0 }) };
        fcurve(&mut w, fc[i], next, prev, path, *index, keys, *baked);
    }
    let mut af = vec![id("AC", "AnimAction")];
    let mut adf = vec![("action", p(action))];
    if slotted {
        // A decoy channelbag for another slot comes first.
        let decoy_fc = w.alloc();
        fcurve(&mut w, decoy_fc, 0, 0, "location", 0, &[(1., 99., 1)], false);
        let decoy_arr = w.raw(w.ptrs(&[decoy_fc]), 1);
        let decoy = w.add("DATA", "ActionChannelbag", &[("slot_handle", n(3.)), ("fcurve_array", p(decoy_arr)), ("fcurve_array_num", n(1.))]);
        let arr = w.raw(w.ptrs(&fc), fc.len());
        let bag = w.add("DATA", "ActionChannelbag", &[("slot_handle", n(7.)), ("fcurve_array", p(arr)), ("fcurve_array_num", n(fc.len() as f64))]);
        let bags = w.raw(w.ptrs(&[decoy, bag]), 2);
        let kd = w.add("DATA", "ActionStripKeyframeData", &[("channelbag_array", p(bags)), ("channelbag_array_num", n(2.))]);
        let kds = w.raw(w.ptrs(&[kd]), 1);
        af.extend([("strip_keyframe_data_array", p(kds)), ("strip_keyframe_data_array_num", n(1.))]);
        adf.push(("slot_handle", n(7.)));
    } else {
        af.extend([("curves.first", p(fc[0])), ("curves.last", p(*fc.last().unwrap()))]);
    }
    let d = w.inst("bAction", &af);
    w.put(action, "AC", "bAction", 1, d);
    let d = w.inst("AnimData", &adf);
    w.put(adt, "DATA", "AnimData", 1, d);
    w.finish()
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
