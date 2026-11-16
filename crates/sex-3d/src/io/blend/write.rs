// SPDX-License-Identifier: GPL-3.0-only
//! `.blend` writer targeting Blender 4.2 (`BLENDER-v402`, file subversion 66).
//!
//! # Container
//!
//! A 12-byte header `BLENDER-v402` (8-byte pointers, little endian), then
//! blocks with the 24-byte classic header `code[4]`, `i32 len`, `u64 old
//! address`, `i32 SDNA index`, `i32 count`, ending with `ENDB`. Blocks come in
//! the order Blender's reader expects: `REND` (frame range and scene name),
//! `GLOB` (`FileGlobal`), then each ID block directly followed by the `DATA`
//! blocks it owns (Blender reads an ID's data from the blocks after it up to
//! the next ID), then `DNA1` and `ENDB`. Old addresses are unique fake values;
//! every pointer refers to the start of a block.
//!
//! # SDNA
//!
//! `DNA1` describes exactly the structs written (and the structs they embed by
//! value), generated into `sdna42.rs` from Blender 4.2's DNA headers by a port
//! of `makesdna`: static (file) member names from `dna_rename_defs.h`
//! (`Mesh.totvert`, `vdata`, `Object.size`, `Lamp`, `Lamp.energy_new`, ...),
//! type sizes for 64-bit, and makesdna's alignment rules (checked by a test).
//! Blender reconstructs structs by member name, so a field this file lacks
//! reads as zero: every struct is therefore filled with the DNA defaults of
//! `DNA_*_defaults.h` before the writer sets its own values.
//!
//! # Data written
//!
//! - `Scene` with `RenderData` (resolution, frame range, fps), colour
//!   management, `ToolSettings`, one `ViewLayer` with its `LayerCollection`
//!   tree, the embedded master `Collection`, the world, the active camera and,
//!   when present, the embedded compositing tree (`use_nodes`).
//! - `Collection` IDs with `CollectionObject`/`CollectionChild` lists.
//! - `Object`: type, data, parent (`partype` object/armature/bone,
//!   `parsubstr`), loc/rot (Euler XYZ)/scale, visibility flags, pass index,
//!   material slots, pose (armatures) and the active action.
//! - `Mesh` in the 4.x attribute layout: `position` (float3) on points,
//!   `.edge_verts` (int2) on edges, `poly_offset_indices`, `.corner_vert` and
//!   `.corner_edge` (int) on corners, `material_index`, `sharp_face`,
//!   `sharp_edge`, a `UVMap` float2 corner layer, vertex groups
//!   (`vertex_group_names` + `MDeformVert`), material slots.
//! - `Material` and `World` with embedded node trees, node groups (`NT`),
//!   node storage structs, socket default values and group interfaces.
//! - `Camera` (lens from the field of view), `Lamp` (`energy_new`, `energy`).
//! - `bArmature` with the `Bone` tree in Blender's local and armature-space
//!   form, bone collections, and `bPose` channels.
//! - Legacy `bAction`s (`curves` list of `FCurve` with `BezTriple` keys) on
//!   object `AnimData`: given handles as aligned handles, otherwise
//!   auto-clamped ones placed here (Blender recomputes them on the next edit).
//! - `Image` IDs with their pixels packed as PNG.
//!
//! Not written: modifiers, constraints, shape keys, lattices (as empties),
//! geometry node groups, NLA and drivers, view-layer visibility, volume
//! materials, nodes newer than 4.2 (Gabor Texture) and UI data (windows,
//! screens, workspaces: Blender's loader keeps its current UI for a file
//! saved without one).
//!
//! # Not verified against Blender
//!
//! There is no Blender here to open the files: they are checked against the
//! reader in this crate (every data type round-trips), the makesdna layout
//! rules (a test) and the DNA of a Blender-saved 3.6 file for the structs
//! that did not change since. Still unverified: that 4.2 accepts each
//! struct as filled (defaults cover the `DNA_*_defaults.h` structs; others
//! start zeroed apart from what is set here), socket `default_value`
//! subtypes, the `MBoolProperty` layer struct for bool attributes, embedded
//! trees written as `DATA` blocks, and the absence of UI, `Base` lists and
//! runtime data (Blender rebuilds them on load).
use super::sdna42::{Dv, DEFAULTS, STRUCTS, TYPES};
use super::{BlendAction, Endian, Prim, Sdna};
use std::collections::HashMap;

/// `BLENDER_FILE_SUBVERSION` of 4.2.
pub(super) const SUBVERSION: i64 = 66;
/// `BLENDER_FILE_MIN_VERSION` / `MIN_SUBVERSION` of 4.2.
const MIN_VERSION: i64 = 306;
const MIN_SUBVERSION: i64 = 13;

/// The SDNA of the written structs: the `DNA1` payload and its parsed form.
pub(super) struct Dna {
    pub(super) bytes: Vec<u8>,
    pub(super) sdna: Sdna,
}

fn align4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

/// Build `DNA1` from the generated tables: `Link` first, as in Blender, so
/// raw data blocks (SDNA index 0) name it.
pub(super) fn build_dna() -> Dna {
    let mut order: Vec<usize> = (0..STRUCTS.len()).collect();
    order.sort_by_key(|&i| (STRUCTS[i].0 != "Link", STRUCTS[i].0 != "ListBase", i));
    let tidx: HashMap<&str, usize> = TYPES.iter().enumerate().map(|(i, t)| (t.0, i)).collect();
    let mut names: Vec<&str> = Vec::new();
    let mut nidx: HashMap<&str, usize> = HashMap::new();
    for &i in &order {
        for (_, n) in STRUCTS[i].1 {
            if !nidx.contains_key(n) {
                nidx.insert(n, names.len());
                names.push(n);
            }
        }
    }
    let mut b = Vec::new();
    b.extend_from_slice(b"SDNANAME");
    b.extend_from_slice(&(names.len() as u32).to_le_bytes());
    for n in &names {
        b.extend_from_slice(n.as_bytes());
        b.push(0);
    }
    align4(&mut b);
    b.extend_from_slice(b"TYPE");
    b.extend_from_slice(&(TYPES.len() as u32).to_le_bytes());
    for (t, _) in TYPES {
        b.extend_from_slice(t.as_bytes());
        b.push(0);
    }
    align4(&mut b);
    b.extend_from_slice(b"TLEN");
    for (_, s) in TYPES {
        b.extend_from_slice(&s.to_le_bytes());
    }
    align4(&mut b);
    b.extend_from_slice(b"STRC");
    b.extend_from_slice(&(order.len() as u32).to_le_bytes());
    for &i in &order {
        let (name, members) = STRUCTS[i];
        b.extend_from_slice(&(tidx[name] as u16).to_le_bytes());
        b.extend_from_slice(&(members.len() as u16).to_le_bytes());
        for (t, n) in members.iter() {
            b.extend_from_slice(&(tidx[t] as u16).to_le_bytes());
            b.extend_from_slice(&(nidx[n] as u16).to_le_bytes());
        }
    }
    // The tables are generated, so this cannot fail.
    let sdna = Sdna::parse(&b, Endian { big: false }, 8).expect("generated SDNA parses");
    Dna { bytes: b, sdna }
}

/// One struct instance being filled.
pub(super) struct Rec {
    pub(super) s: usize,
    pub(super) data: Vec<u8>,
}

/// A value for [`W::set`].
pub(super) enum V<'a> {
    F(&'a [f32]),
    I(&'a [i64]),
    P(u64),
    S(&'a str),
    /// Defaults (generated as f64).
    D(&'a [f64]),
}

struct Blk {
    code: [u8; 4],
    sdna: usize,
    old: u64,
    nr: usize,
    data: Vec<u8>,
}

/// The writer: SDNA, blocks in file order, the next free fake address and
/// member paths that did not resolve (a bug; tests check it stays empty).
pub(super) struct W {
    pub(super) dna: Dna,
    blocks: Vec<Blk>,
    next: u64,
    pub(super) missing: Vec<String>,
    /// Collection object/child lists waiting to be written:
    /// (CollectionObject addresses, objects, CollectionChild addresses, collections).
    pub(super) pending: Vec<(Vec<u64>, Vec<u64>, Vec<u64>, Vec<u64>)>,
    /// Where the current ID's block goes: ahead of the data written for it.
    group_start: usize,
}

fn put_num(d: &mut [u8], off: usize, prim: Prim, x: f64) {
    let bytes: Vec<u8> = match prim {
        Prim::I8 | Prim::U8 => vec![x as i64 as u8],
        Prim::I16 | Prim::U16 => (x as i64 as i16).to_le_bytes().to_vec(),
        Prim::I32 | Prim::U32 => (x as i64 as i32).to_le_bytes().to_vec(),
        Prim::I64 | Prim::U64 => (x as i64).to_le_bytes().to_vec(),
        Prim::F32 => (x as f32).to_le_bytes().to_vec(),
        Prim::F64 => x.to_le_bytes().to_vec(),
        Prim::Other => return,
    };
    if let Some(s) = d.get_mut(off..off + bytes.len()) {
        s.copy_from_slice(&bytes);
    }
}

impl W {
    pub(super) fn new() -> W {
        W { dna: build_dna(), blocks: Vec::new(), next: 0x1000_0000, missing: Vec::new(), pending: Vec::new(), group_start: 0 }
    }

    /// A fresh old address.
    pub(super) fn addr(&mut self) -> u64 {
        self.next += 0x100;
        self.next
    }

    fn struct_idx(&self, name: &str) -> usize {
        self.dna.sdna.by_name.get(name).copied().unwrap_or_else(|| panic!("struct {name} not in the writer's SDNA"))
    }

    pub(super) fn size(&self, name: &str) -> usize {
        self.dna.sdna.structs[self.struct_idx(name)].size
    }

    /// A zeroed `name` with its DNA defaults applied.
    pub(super) fn rec(&mut self, name: &str) -> Rec {
        let s = self.struct_idx(name);
        let mut r = Rec { s, data: vec![0; self.dna.sdna.structs[s].size] };
        for (st, path, v) in DEFAULTS {
            if *st == name {
                let v = match v {
                    Dv::N(n) => V::D(n),
                    Dv::S(s) => V::S(s),
                };
                self.set_quiet(&mut r, path, &v);
            }
        }
        r
    }

    /// Offset and member of a dotted path (`r.im_format.depth`).
    fn locate(&self, s: usize, path: &str) -> Option<(usize, &super::Member)> {
        let mut s = s;
        let mut off = 0;
        let parts: Vec<&str> = path.split('.').collect();
        for (k, p) in parts.iter().enumerate() {
            let m = self.dna.sdna.structs[s].members.get(*p)?;
            if k + 1 == parts.len() {
                return Some((off + m.offset, m));
            }
            if m.is_ptr {
                return None;
            }
            off += m.offset;
            s = (*self.dna.sdna.struct_of_type.get(m.ty)?)?;
        }
        None
    }

    fn set_quiet(&self, r: &mut Rec, path: &str, v: &V<'_>) -> bool {
        let Some((off, m)) = self.locate(r.s, path) else { return false };
        match v {
            V::P(p) if m.is_ptr => r.data[off..off + 8].copy_from_slice(&p.to_le_bytes()),
            V::S(s) if !m.is_ptr && m.elem == 1 => {
                let n = s.len().min(m.len.saturating_sub(1));
                let mut cut = n;
                while cut > 0 && !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                r.data[off..off + m.len].fill(0);
                r.data[off..off + cut].copy_from_slice(&s.as_bytes()[..cut]);
            }
            V::F(xs) if !m.is_ptr => {
                for (i, x) in xs.iter().enumerate().take(m.len) {
                    put_num(&mut r.data, off + i * m.elem, m.prim, *x as f64);
                }
            }
            V::D(xs) if !m.is_ptr => {
                for (i, x) in xs.iter().enumerate().take(m.len) {
                    put_num(&mut r.data, off + i * m.elem, m.prim, *x);
                }
            }
            V::I(xs) if !m.is_ptr => {
                for (i, x) in xs.iter().enumerate().take(m.len) {
                    put_num(&mut r.data, off + i * m.elem, m.prim, *x as f64);
                }
            }
            _ => return false,
        }
        true
    }

    /// Set a member; an unknown path is recorded in `missing`.
    pub(super) fn set(&mut self, r: &mut Rec, path: &str, v: V<'_>) {
        if !self.set_quiet(r, path, &v) {
            let name = self.dna.sdna.structs[r.s].name.clone();
            self.missing.push(format!("{name}.{path}"));
        }
    }
    pub(super) fn f(&mut self, r: &mut Rec, path: &str, xs: &[f32]) {
        self.set(r, path, V::F(xs));
    }
    pub(super) fn i(&mut self, r: &mut Rec, path: &str, x: i64) {
        self.set(r, path, V::I(&[x]));
    }
    pub(super) fn p(&mut self, r: &mut Rec, path: &str, x: u64) {
        self.set(r, path, V::P(x));
    }
    pub(super) fn s(&mut self, r: &mut Rec, path: &str, x: &str) {
        self.set(r, path, V::S(x));
    }

    /// Copy `sub` into element `index` of the struct-array member `path`.
    pub(super) fn put_sub(&mut self, r: &mut Rec, path: &str, index: usize, sub: &Rec) {
        match self.locate(r.s, path) {
            Some((off, m)) if !m.is_ptr && index < m.len && m.elem == sub.data.len() => {
                let at = off + index * m.elem;
                r.data[at..at + m.elem].copy_from_slice(&sub.data);
            }
            _ => {
                let name = self.dna.sdna.structs[r.s].name.clone();
                self.missing.push(format!("{name}.{path}[{index}]"));
            }
        }
    }

    /// Start writing an ID: its block will precede everything written from
    /// here on (Blender reads an ID's data from the blocks that follow it).
    pub(super) fn start_id(&mut self) {
        self.group_start = self.blocks.len();
    }

    /// A block of struct instances at `addr`; an ID code (two letters) puts
    /// it ahead of the data written since [`W::start_id`].
    pub(super) fn put(&mut self, code: &[u8], addr: u64, recs: &[Rec]) {
        let Some(first) = recs.first() else { return };
        let mut c = [0u8; 4];
        c[..code.len()].copy_from_slice(code);
        let data = recs.iter().flat_map(|r| r.data.iter().copied()).collect();
        let b = Blk { code: c, sdna: first.s, old: addr, nr: recs.len(), data };
        if code.len() == 2 {
            self.blocks.insert(self.group_start.min(self.blocks.len()), b);
        } else {
            self.blocks.push(b);
        }
    }

    /// A raw block at a chosen address.
    pub(super) fn put_raw_at(&mut self, addr: u64, data: Vec<u8>) {
        self.blocks.push(Blk { code: *b"DATA", sdna: 0, old: addr, nr: 1, data });
    }

    /// `n` instances of struct `sname` given as bytes, at `addr`.
    pub(super) fn put_struct_bytes(&mut self, addr: u64, sname: &str, n: usize, data: Vec<u8>) {
        let s = self.struct_idx(sname);
        self.blocks.push(Blk { code: *b"DATA", sdna: s, old: addr, nr: n.max(1), data });
    }

    /// Overwrite pointer `member` of the (first) struct in the block at `addr`.
    pub(super) fn patch_ptr(&mut self, addr: u64, member: &str, value: u64) {
        let Some(bi) = self.blocks.iter().position(|b| b.old == addr) else { return };
        let s = self.blocks[bi].sdna;
        let Some((off, m)) = self.locate(s, member) else { return };
        if m.is_ptr {
            self.blocks[bi].data[off..off + 8].copy_from_slice(&value.to_le_bytes());
        }
    }

    /// A `DATA` block of struct instances at a new address (0 when empty).
    pub(super) fn data(&mut self, recs: &[Rec]) -> u64 {
        if recs.is_empty() {
            return 0;
        }
        let a = self.addr();
        self.put(b"DATA", a, recs);
        a
    }

    /// A raw `DATA` block (SDNA index 0, as `BLO_write_raw` writes them).
    pub(super) fn raw(&mut self, bytes: Vec<u8>) -> u64 {
        if bytes.is_empty() {
            return 0;
        }
        let a = self.addr();
        self.blocks.push(Blk { code: *b"DATA", sdna: 0, old: a, nr: 1, data: bytes });
        a
    }

    /// A NUL-terminated string block.
    pub(super) fn string(&mut self, s: &str) -> u64 {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        self.raw(b)
    }

    /// An array of pointers.
    pub(super) fn ptrs(&mut self, ps: &[u64]) -> u64 {
        self.raw(ps.iter().flat_map(|p| p.to_le_bytes()).collect())
    }

    /// The file: header, blocks, `DNA1`, `ENDB`.
    pub(super) fn finish(self, glob: Rec, rend: (i32, i32, &str)) -> Vec<u8> {
        let mut out = b"BLENDER-v402".to_vec();
        let head = |out: &mut Vec<u8>, code: &[u8; 4], len: usize, old: u64, sdna: usize, nr: usize| {
            out.extend_from_slice(code);
            out.extend_from_slice(&(len as i32).to_le_bytes());
            out.extend_from_slice(&old.to_le_bytes());
            out.extend_from_slice(&(sdna as i32).to_le_bytes());
            out.extend_from_slice(&(nr as i32).to_le_bytes());
        };
        // REND: `int sfra, efra; char scene_name[64]`.
        let mut r = Vec::with_capacity(72);
        r.extend_from_slice(&rend.0.to_le_bytes());
        r.extend_from_slice(&rend.1.to_le_bytes());
        let mut name = [0u8; 64];
        let n = rend.2.len().min(63);
        name[..n].copy_from_slice(&rend.2.as_bytes()[..n]);
        r.extend_from_slice(&name);
        head(&mut out, b"REND", r.len(), 0, 0, 1);
        out.extend_from_slice(&r);
        head(&mut out, b"GLOB", glob.data.len(), 0x0F00_0000, glob.s, 1);
        out.extend_from_slice(&glob.data);
        for b in &self.blocks {
            head(&mut out, &b.code, b.data.len(), b.old, b.sdna, b.nr);
            out.extend_from_slice(&b.data);
        }
        head(&mut out, b"DNA1", self.dna.bytes.len(), 0x0E00_0000, 0, 1);
        out.extend_from_slice(&self.dna.bytes);
        head(&mut out, b"ENDB", 0, 0, 0, 0);
        out
    }

    /// `FileGlobal` for the given scene and view layer.
    pub(super) fn glob(&mut self, scene: u64, view_layer: u64) -> Rec {
        let mut g = self.rec("FileGlobal");
        self.s(&mut g, "subvstr", &format!("{SUBVERSION:4}"));
        self.i(&mut g, "subversion", SUBVERSION);
        self.i(&mut g, "minversion", MIN_VERSION);
        self.i(&mut g, "minsubversion", MIN_SUBVERSION);
        self.p(&mut g, "curscene", scene);
        self.p(&mut g, "cur_view_layer", view_layer);
        g
    }
}

/// Every action the writer can attach: by object name.
pub(super) fn actions_by_object(actions: &[BlendAction]) -> HashMap<&str, &BlendAction> {
    actions.iter().map(|a| (a.object.as_str(), a)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (pointer?, array length) of a DNA member name such as `*mat`, `vec[3][3]`, `(*func)()`.
    fn shape(name: &str) -> (bool, usize) {
        let ptr = name.starts_with('*') || name.starts_with("(*");
        let mut len = 1;
        let mut rest = name;
        while let Some(i) = rest.find('[') {
            let j = rest[i..].find(']').unwrap() + i;
            len *= rest[i + 1..j].parse::<usize>().unwrap();
            rest = &rest[j + 1..];
        }
        (ptr, len)
    }

    /// The alignment makesdna gives a struct: the largest of its members'
    /// (pointers and 8-byte scalars 8, 4-byte 4, 2-byte 2, structs their own).
    fn struct_align(sname: &str, size: &HashMap<&str, usize>) -> usize {
        let members = STRUCTS.iter().find(|s| s.0 == sname).unwrap().1;
        members
            .iter()
            .map(|(ty, name)| match shape(name) {
                (true, _) => 8,
                _ if STRUCTS.iter().any(|s| s.0 == *ty) => struct_align(ty, size),
                _ => size[ty].clamp(1, 8),
            })
            .max()
            .unwrap_or(1)
    }

    /// makesdna's checks for 64-bit files (`calculate_struct_sizes`,
    /// `check_field_alignment`): members sum to the TLEN size; pointers,
    /// embedded structs and 8-byte scalars start on 8 bytes, 4-byte scalars on
    /// 4, 2-byte on 2; a struct's size is a multiple of its alignment and of 4
    /// (sizes 1 and 2 excepted).
    #[test]
    fn sdna_follows_makesdna_layout_rules() {
        let size: HashMap<&str, usize> = TYPES.iter().map(|(t, s)| (*t, *s as usize)).collect();
        for (sname, members) in STRUCTS {
            let mut off = 0;
            for (ty, name) in members.iter() {
                let (ptr, len) = shape(name);
                let is_struct = STRUCTS.iter().any(|s| s.0 == *ty);
                let (elem, align) = match (ptr, is_struct) {
                    (true, _) => (8, 8),
                    (false, true) => (size[ty], 8),
                    (false, false) => (size[ty], size[ty].clamp(1, 8)),
                };
                assert_eq!(off % align, 0, "{sname}.{name} at {off} is not {align}-aligned");
                off += elem * len;
            }
            assert_eq!(off, size[sname], "{sname}: members sum to {off}, TLEN says {}", size[sname]);
            let a = struct_align(sname, &size);
            assert_eq!(off % a, 0, "{sname}: size {off} is not a multiple of {a}");
            assert!(off % 4 == 0 || off == 1 || off == 2, "{sname}: size {off} is not a multiple of 4");
        }
    }

    /// The parsed SDNA agrees with the tables, and every default resolves.
    #[test]
    fn dna_parses_and_defaults_resolve() {
        let mut w = W::new();
        assert_eq!(w.dna.sdna.structs[0].name, "Link");
        for (sname, _) in STRUCTS {
            let s = w.dna.sdna.by_name[*sname];
            assert_eq!(w.dna.sdna.structs[s].size, TYPES.iter().find(|t| t.0 == *sname).unwrap().1 as usize);
        }
        for (st, path, v) in DEFAULTS {
            let mut r = w.rec(st);
            let v = match v {
                Dv::N(n) => V::D(n),
                Dv::S(s) => V::S(s),
            };
            assert!(w.set_quiet(&mut r, path, &v), "default {st}.{path} does not resolve");
        }
        assert!(w.missing.is_empty());
    }
}
