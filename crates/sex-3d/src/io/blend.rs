// SPDX-License-Identifier: GPL-3.0-only
//! Native reader for Blender `.blend` files.
//!
//! A clean-room reader written from the publicly documented structure of the
//! format (the "blend file" chapter of Blender's developer docs, the
//! `mystery of the blend` write-up and observation of the files themselves).
//! No Blender source code is used. Everything is bounds-checked: corrupt or
//! truncated input yields `Err`, never a panic.
//!
//! # Format knowledge this reader relies on
//!
//! **Container.** A file is either raw, gzip-compressed (magic `1f 8b`, Blender
//! before 3.0) or zstd-compressed (magic `28 b5 2f fd`, Blender 3.0+). Blender's
//! zstd files are a sequence of ordinary frames followed by a skippable frame
//! (magic `0x184D2A5?`) holding a seek table; all frames are concatenated.
//!
//! **Header.** Classic layout, 12 bytes: `BLENDER`, a pointer-size char (`_` =
//! 4 bytes, `-` = 8 bytes), an endianness char (`v` = little, `V` = big) and a
//! three-digit version (`279`, `402`, ...). Blender 5.0 introduced a longer
//! header: `BLENDER`, a two-digit header size (`17`), `-`, a two-digit file
//! format version (`01`), the endianness char and a four-digit version
//! (`0500`). Pointers are always 8 bytes there, and format version 1 uses the
//! "large" block header below.
//!
//! **Blocks.** After the header comes a sequence of blocks, ending with one whose
//! code is `ENDB`. Each block header (BHead) is, in the file's endianness:
//! - classic: `code[4]`, `i32 len`, `old pointer` (4 or 8 bytes), `i32 SDNAnr`,
//!   `i32 nr` (20 or 24 bytes);
//! - large (5.0+): `code[4]`, `i32 SDNAnr`, `u64 old pointer`, `i64 len`,
//!   `i64 nr` (32 bytes).
//!
//! `len` bytes of payload follow: `nr` consecutive instances of the struct
//! numbered `SDNAnr` (or raw arrays). The old pointer is the address the data
//! had when it was saved; other blocks refer to it by that value, so a map from
//! old pointer to block resolves every pointer. ID blocks use a two-letter code
//! (`OB`, `ME`, `MA`, `LA`, `CA`, `SC`), the rest are `DATA`; `GLOB` holds the
//! `FileGlobal` struct (current scene) and `DNA1` the type catalogue.
//!
//! **SDNA (`DNA1`).** `SDNA`, then `NAME` + `i32` count + NUL-terminated member
//! names, padded to 4 bytes; `TYPE` + count + NUL-terminated type names, padded;
//! `TLEN` + one `u16` size per type, padded; `STRC` + count + per struct a `u16`
//! type index, a `u16` member count and that many (`u16` type, `u16` name)
//! pairs. Member names carry their declarator: `*next` (pointer), `**mat`,
//! `(*func)()` (function pointer), `co[3]`, `obmat[4][4]`. Members are laid out
//! back to back with no implicit padding (Blender's DNA requires explicit `_pad`
//! members), so offsets are running sums of `size × array length`. This reader
//! looks every member up by name, so it does not care where fields are, which
//! padding exists or which unrelated fields were added between versions.
//!
//! **Data model** (names as they appear in SDNA; aliases from renames listed):
//! - `ID`: `name[66]` (`[258]` in 5.0) whose first two chars are the ID code.
//! - `Object`: `id`, `type` (0 empty, 1 mesh, 10 light, 11 camera, ...),
//!   `data`, `parent`, `parentinv[4][4]`, `loc`, `dloc`, `rot`, `drot`,
//!   `quat`, `dquat`, `rotAxis`, `rotAngle`, `rotmode` (0 quaternion, 1..6 Euler
//!   XYZ XZY YXZ YZX ZXY ZYX, -1 axis-angle), `scale` (`size` before 2.8 rename),
//!   `dscale`, `obmat`/`object_to_world` (4.2+ keeps it only at runtime, so the
//!   loc/rot/scale basis is preferred), `mat` (Material**), `matbits` (char*,
//!   nonzero = slot linked to the object rather than the mesh), `totcol`.
//!   Local transform = `parentinv × T × R × S`.
//! - `Mesh`: element counts `totvert`/`verts_num`, `totedge`/`edges_num`,
//!   `totpoly`/`faces_num`, `totloop`/`corners_num`, `totface` (tessfaces);
//!   `CustomData vdata, edata, pdata, ldata, fdata` (4.0+: `vert_data`,
//!   `edge_data`, `face_data`, `corner_data`, `fdata_legacy`), each with `layers`
//!   (CustomDataLayer*: `type`, `name`, `active_rnd`, `data`) and `totlayer`;
//!   `mat`, `totcol`.
//!   - Legacy layout (up to 3.4): `MVert {co[3], ...}` (type 0), `MEdge {v1,
//!     v2, ...}` (3), `MPoly {loopstart, totloop, mat_nr, flag}` (25, flag bit 1
//!     = smooth), `MLoop {v, e}` (26), `MLoopUV {uv[2], flag}` (16); older files
//!     also have direct `mvert`/`medge`/`mpoly`/`mloop`/`mloopuv` pointers, and
//!     pre-2.63 files only `MFace {v1..v4, mat_nr, flag}` tessfaces.
//!   - Attribute layout (3.5 .. 5.x): generic layers by name: `position` (float3,
//!     type 48) on points, `.edge_verts` (int2, 53) on edges, `.corner_vert`
//!     (int, 11) on corners, `material_index` (int) and `sharp_face` (bool, 50)
//!     on faces, UV maps as float2 (49) corner layers whose names do not start
//!     with `.`; face sizes come from the `face_offset_indices` /
//!     `poly_offset_indices` int array of `faces + 1` offsets.
//!   The pieces are read independently with fallbacks, so transitional files
//!   (e.g. 3.5: `position` attribute plus `MPoly`/`MLoop`) also load.
//! - `Material` (`Material`): `r g b a`, `metallic`, `roughness`, and, when it
//!   has one, an embedded `nodetree` (`bNodeTree.nodes` ListBase of `bNode` with
//!   `idname`, `inputs` ListBase of `bNodeSocket` with `name`, `identifier` and
//!   `default_value` pointing at `bNodeSocketValueRGBA {value[4]}` or
//!   `bNodeSocketValueFloat {..., value, ...}`). The `ShaderNodeBsdfPrincipled`
//!   inputs win over the viewport colour unless `use_nodes` is 0.
//! - `Light` (`Lamp` before 2.8): `type` (0 point, 1 sun, 2 spot, 3 hemi, 4
//!   area), `r g b`, `energy` (W), `spotsize` (full cone angle, radians),
//!   `spotblend`, `area_size`, `radius` (4.x; `area_size` doubled as the point
//!   radius before), `sun_angle`.
//! - `Camera`: `type` (0 perspective, 1 orthographic), `lens` (mm), `sensor_x`,
//!   `sensor_y`, `sensor_fit` (0 auto, 1 horizontal, 2 vertical), `ortho_scale`
//!   (along the fitted axis), `clip_start`/`clipsta`, `clip_end`/`clipend`.
//! - `Scene`: `id`, `camera`, `r` (`RenderData`: `xsch`, `ysch`, `xasp`,
//!   `yasp`, `sfra`, `efra`, `frs_sec`, `frs_sec_base`). The current scene is
//!   `FileGlobal.curscene` in the `GLOB` block, else the first `SC` block.
//!
//! Not imported yet: modifiers, collections and visibility, shape keys,
//! textures/images, world, animation, library links, custom split normals.
use crate::math::{Mat4, Quat, Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::scene::{Camera, Light, LightKind, Material, Object, ObjectData, Scene, Transform};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::Read;

/// Largest decompressed file accepted (guards against decompression bombs).
const MAX_DECOMPRESSED: u64 = 4 << 30;

// CustomData layer types (eCustomDataType values stored in files).
const CD_MVERT: i64 = 0;
const CD_MEDGE: i64 = 3;
const CD_MFACE: i64 = 4;
const CD_MLOOPUV: i64 = 16;
const CD_MPOLY: i64 = 25;
const CD_MLOOP: i64 = 26;
const CD_PROP_FLOAT2: i64 = 49;

/// Read a `.blend` file (raw, gzip or zstd) into a [`Scene`].
pub fn import_blend(bytes: &[u8]) -> Result<Scene, String> {
    let raw = decompress(bytes)?;
    let file = BlendFile::parse(&raw)?;
    Importer::new(&file).run()
}

// ---------------------------------------------------------------------------
// Decompression

fn decompress(bytes: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    if bytes.starts_with(b"BLENDER") {
        return Ok(Cow::Borrowed(bytes));
    }
    let out = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(bytes)
            .take(MAX_DECOMPRESSED + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("blend: gzip: {e}"))?;
        out
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        zstd_decode(bytes)?
    } else {
        return Err("blend: not a .blend file (unknown magic)".into());
    };
    if out.len() as u64 > MAX_DECOMPRESSED {
        return Err("blend: decompressed file too large".into());
    }
    if !out.starts_with(b"BLENDER") {
        return Err("blend: compressed data is not a .blend file".into());
    }
    Ok(Cow::Owned(out))
}

/// Decode every zstd frame in `input`, skipping skippable frames.
fn zstd_decode(mut input: &[u8]) -> Result<Vec<u8>, String> {
    use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
    use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
    let err = |e: FrameDecoderError| format!("blend: zstd: {e}");
    let mut out = Vec::new();
    let mut dec = FrameDecoder::new();
    while !input.is_empty() {
        match dec.init(&mut input) {
            Ok(()) => {}
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame { length, .. })) => {
                input = input.get(length as usize..).ok_or("blend: zstd: truncated skippable frame")?;
                continue;
            }
            Err(e) => return Err(err(e)),
        }
        loop {
            let before = input.len();
            dec.decode_blocks(&mut input, BlockDecodingStrategy::UptoBytes(1 << 20)).map_err(err)?;
            let got = dec.collect().unwrap_or_default();
            if (out.len() + got.len()) as u64 > MAX_DECOMPRESSED {
                return Err("blend: decompressed file too large".into());
            }
            let progressed = !got.is_empty() || input.len() != before;
            out.extend_from_slice(&got);
            if dec.is_finished() {
                break;
            }
            if !progressed {
                return Err("blend: zstd: truncated frame".into());
            }
        }
        out.extend_from_slice(&dec.collect().unwrap_or_default());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Byte reading

/// Endian-aware, bounds-checked primitive reads.
#[derive(Clone, Copy)]
struct Endian {
    big: bool,
}

impl Endian {
    fn bytes<const N: usize>(&self, d: &[u8], off: usize) -> Option<[u8; N]> {
        let s = d.get(off..off.checked_add(N)?)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        if self.big {
            a.reverse();
        }
        Some(a) // now little-endian
    }
    fn u16(&self, d: &[u8], off: usize) -> Option<u16> {
        self.bytes::<2>(d, off).map(u16::from_le_bytes)
    }
    fn i32(&self, d: &[u8], off: usize) -> Option<i32> {
        self.bytes::<4>(d, off).map(i32::from_le_bytes)
    }
    fn u32(&self, d: &[u8], off: usize) -> Option<u32> {
        self.bytes::<4>(d, off).map(u32::from_le_bytes)
    }
    fn i64(&self, d: &[u8], off: usize) -> Option<i64> {
        self.bytes::<8>(d, off).map(i64::from_le_bytes)
    }
    fn u64(&self, d: &[u8], off: usize) -> Option<u64> {
        self.bytes::<8>(d, off).map(u64::from_le_bytes)
    }
    fn f32(&self, d: &[u8], off: usize) -> Option<f32> {
        self.bytes::<4>(d, off).map(f32::from_le_bytes)
    }
    fn prim(&self, d: &[u8], off: usize, p: Prim) -> Option<f64> {
        Some(match p {
            Prim::I8 => *d.get(off)? as i8 as f64,
            Prim::U8 => *d.get(off)? as f64,
            Prim::I16 => self.bytes::<2>(d, off).map(i16::from_le_bytes)? as f64,
            Prim::U16 => self.u16(d, off)? as f64,
            Prim::I32 => self.i32(d, off)? as f64,
            Prim::U32 => self.u32(d, off)? as f64,
            Prim::I64 => self.i64(d, off)? as f64,
            Prim::U64 => self.u64(d, off)? as f64,
            Prim::F32 => self.f32(d, off)? as f64,
            Prim::F64 => self.bytes::<8>(d, off).map(f64::from_le_bytes)?,
            Prim::Other => return None,
        })
    }
}

/// NUL-terminated string from a byte slice (lossy UTF-8).
fn c_string(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// A count read from the file: non-negative and finite, else 0.
fn count(v: f64) -> usize {
    if v.is_finite() && v > 0.0 && v < 1e12 {
        v as usize
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// SDNA

/// Primitive kind of a member's type, for numeric reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prim {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Other,
}

fn prim_of(ty: &str) -> Prim {
    match ty {
        // `char` holds both text and small enums/flags in DNA; unsigned reads suit both.
        "char" | "uchar" | "uint8_t" | "bool" => Prim::U8,
        "int8_t" => Prim::I8,
        "short" | "int16_t" => Prim::I16,
        "ushort" | "uint16_t" => Prim::U16,
        "int" | "int32_t" => Prim::I32,
        "uint" | "uint32_t" => Prim::U32,
        "long" | "int64_t" => Prim::I64,
        "ulong" | "uint64_t" => Prim::U64,
        "float" => Prim::F32,
        "double" => Prim::F64,
        _ => Prim::Other,
    }
}

#[derive(Debug)]
struct Member {
    /// Type index into [`Sdna::types`].
    ty: usize,
    prim: Prim,
    is_ptr: bool,
    /// Product of the array dimensions (1 for scalars).
    len: usize,
    /// Size of one element (the pointer size for pointers).
    elem: usize,
    offset: usize,
}

#[derive(Debug)]
struct StructDef {
    size: usize,
    members: HashMap<String, Member>,
}

#[derive(Debug)]
struct Sdna {
    structs: Vec<StructDef>,
    /// Type index -> struct index, for nested struct members.
    struct_of_type: Vec<Option<usize>>,
    by_name: HashMap<String, usize>,
}

/// Cursor over the DNA1 payload.
struct Cursor<'a> {
    d: &'a [u8],
    pos: usize,
    e: Endian,
}

impl<'a> Cursor<'a> {
    fn tag(&mut self, t: &[u8; 4]) -> Result<(), String> {
        if self.d.get(self.pos..self.pos + 4) != Some(&t[..]) {
            return Err(format!("blend: SDNA: expected {}", String::from_utf8_lossy(t)));
        }
        self.pos += 4;
        Ok(())
    }
    fn u32(&mut self) -> Result<usize, String> {
        let v = self.e.u32(self.d, self.pos).ok_or("blend: SDNA truncated")?;
        self.pos += 4;
        Ok(v as usize)
    }
    fn u16(&mut self) -> Result<usize, String> {
        let v = self.e.u16(self.d, self.pos).ok_or("blend: SDNA truncated")?;
        self.pos += 2;
        Ok(v as usize)
    }
    fn cstr(&mut self) -> Result<String, String> {
        let rest = self.d.get(self.pos..).ok_or("blend: SDNA truncated")?;
        let end = rest.iter().position(|&c| c == 0).ok_or("blend: SDNA unterminated string")?;
        self.pos += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
    fn align4(&mut self) {
        self.pos = (self.pos + 3) & !3;
    }
    /// Read a count that must fit in the remaining bytes (each item >= `min` bytes).
    fn count(&mut self, min: usize) -> Result<usize, String> {
        let n = self.u32()?;
        if n.saturating_mul(min) > self.d.len().saturating_sub(self.pos) {
            return Err("blend: SDNA count exceeds data".into());
        }
        Ok(n)
    }
}

/// Split a DNA member declarator into (base name, is pointer, array length).
fn parse_decl(decl: &str) -> Option<(String, bool, usize)> {
    let is_ptr = decl.starts_with('*') || decl.starts_with("(*");
    let core = if let Some(rest) = decl.strip_prefix("(*") {
        rest.split(')').next().unwrap_or(rest)
    } else {
        decl.trim_start_matches('*')
    };
    let base = core.split('[').next().unwrap_or(core).to_string();
    let mut len: usize = 1;
    // Array dimensions only matter outside a function-pointer's argument list.
    let dims = if decl.starts_with("(*") { "" } else { core };
    for part in dims.split('[').skip(1) {
        let n: usize = part.trim_end_matches(']').trim().parse().ok()?;
        len = len.checked_mul(n)?;
    }
    if base.is_empty() || len > 1 << 28 {
        return None;
    }
    Some((base, is_ptr, len))
}

impl Sdna {
    fn parse(d: &[u8], e: Endian, ptr_size: usize) -> Result<Sdna, String> {
        let mut c = Cursor { d, pos: 0, e };
        c.tag(b"SDNA")?;
        c.tag(b"NAME")?;
        let n = c.count(1)?;
        let names = (0..n).map(|_| c.cstr()).collect::<Result<Vec<_>, _>>()?;
        c.align4();
        c.tag(b"TYPE")?;
        let n = c.count(1)?;
        let types = (0..n).map(|_| c.cstr()).collect::<Result<Vec<_>, _>>()?;
        c.align4();
        c.tag(b"TLEN")?;
        let tlen = (0..types.len()).map(|_| c.u16()).collect::<Result<Vec<_>, _>>()?;
        c.align4();
        c.tag(b"STRC")?;
        let n = c.count(4)?;
        let mut structs = Vec::with_capacity(n);
        let mut struct_of_type = vec![None; types.len()];
        let mut by_name = HashMap::new();
        for si in 0..n {
            let ty = c.u16()?;
            let nm = c.u16()?;
            let size = *tlen.get(ty).ok_or("blend: SDNA struct type out of range")?;
            let mut members = HashMap::new();
            let mut offset = 0usize;
            for _ in 0..nm {
                let mty = c.u16()?;
                let mname = c.u16()?;
                let decl = names.get(mname).ok_or("blend: SDNA member name out of range")?;
                let tname = types.get(mty).ok_or("blend: SDNA member type out of range")?;
                let (base, is_ptr, len) = parse_decl(decl).ok_or("blend: SDNA bad member name")?;
                let elem = if is_ptr { ptr_size } else { tlen[mty] };
                let total = elem.checked_mul(len).ok_or("blend: SDNA member too large")?;
                let prim = if is_ptr { Prim::Other } else { prim_of(tname) };
                members.entry(base).or_insert(Member { ty: mty, prim, is_ptr, len, elem, offset });
                offset = offset.checked_add(total).ok_or("blend: SDNA struct too large")?;
            }
            if offset > size {
                return Err(format!("blend: SDNA struct {} larger than its declared size", types[ty]));
            }
            struct_of_type[ty] = Some(si);
            by_name.entry(types[ty].clone()).or_insert(si);
            structs.push(StructDef { size, members });
        }
        Ok(Sdna { structs, struct_of_type, by_name })
    }
}

// ---------------------------------------------------------------------------
// File and blocks

struct Block<'a> {
    code: [u8; 4],
    sdna: usize,
    old: u64,
    nr: usize,
    data: &'a [u8],
}

struct BlendFile<'a> {
    e: Endian,
    ptr_size: usize,
    blocks: Vec<Block<'a>>,
    sdna: Sdna,
    by_ptr: HashMap<u64, usize>,
}

/// Parse an ASCII decimal field.
fn digits(b: &[u8]) -> Option<usize> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(b).ok()?.parse().ok()
}

impl<'a> BlendFile<'a> {
    fn parse(d: &'a [u8]) -> Result<BlendFile<'a>, String> {
        let bad = || "blend: bad file header".to_string();
        if !d.starts_with(b"BLENDER") || d.len() < 12 {
            return Err(bad());
        }
        // (pointer size, big endian, large bheads, header length)
        let (ptr_size, big, large, mut pos) = match d[7] {
            b'_' | b'-' => {
                digits(&d[9..12]).ok_or_else(bad)?;
                (if d[7] == b'_' { 4 } else { 8 }, endian_char(d[8]).ok_or_else(bad)?, false, 12)
            }
            _ => {
                // Blender 5.0+: BLENDER + "17" + '-' + "01" + endianness + "0500".
                let hs = digits(d.get(7..9).ok_or_else(bad)?).ok_or_else(bad)?;
                let h = d.get(..hs.max(17)).ok_or_else(bad)?;
                if hs < 17 || h[9] != b'-' {
                    return Err(bad());
                }
                let fmt = digits(&h[10..12]).ok_or_else(bad)?;
                let big = endian_char(h[12]).ok_or_else(bad)?;
                digits(&h[13..17]).ok_or_else(bad)?;
                match fmt {
                    0 => (8, big, false, hs),
                    1 => (8, big, true, hs),
                    _ => return Err(format!("blend: unsupported file format version {fmt}")),
                }
            }
        };
        let e = Endian { big };
        let mut blocks = Vec::new();
        let mut dna = None;
        loop {
            let trunc = || "blend: truncated file (no ENDB block)".to_string();
            let code: [u8; 4] = d.get(pos..pos + 4).ok_or_else(trunc)?.try_into().map_err(|_| trunc())?;
            let (sdna, old, len, nr, hlen) = if large {
                let sdna = e.i32(d, pos + 4).ok_or_else(trunc)? as i64;
                let old = e.u64(d, pos + 8).ok_or_else(trunc)?;
                let len = e.i64(d, pos + 16).ok_or_else(trunc)?;
                let nr = e.i64(d, pos + 24).ok_or_else(trunc)?;
                (sdna, old, len, nr, 32)
            } else {
                let len = e.i32(d, pos + 4).ok_or_else(trunc)? as i64;
                let old = if ptr_size == 4 { e.u32(d, pos + 8).map(u64::from) } else { e.u64(d, pos + 8) }.ok_or_else(trunc)?;
                let sdna = e.i32(d, pos + 8 + ptr_size).ok_or_else(trunc)? as i64;
                let nr = e.i32(d, pos + 12 + ptr_size).ok_or_else(trunc)? as i64;
                (sdna, old, len, nr, 16 + ptr_size)
            };
            pos += hlen;
            if &code == b"ENDB" {
                break;
            }
            if len < 0 || sdna < 0 || nr < 0 {
                return Err("blend: corrupt block header".into());
            }
            let end = pos.checked_add(usize::try_from(len).map_err(|_| trunc())?).ok_or_else(trunc)?;
            let data = d.get(pos..end).ok_or_else(trunc)?;
            pos = end;
            if &code == b"DNA1" {
                dna = Some(data);
            }
            blocks.push(Block { code, sdna: sdna as usize, old, nr: nr as usize, data });
        }
        let sdna = Sdna::parse(dna.ok_or("blend: no DNA1 block")?, e, ptr_size)?;
        let mut by_ptr = HashMap::new();
        for (i, b) in blocks.iter().enumerate() {
            if b.old != 0 && &b.code != b"DNA1" {
                by_ptr.entry(b.old).or_insert(i);
            }
        }
        Ok(BlendFile { e, ptr_size, blocks, sdna, by_ptr })
    }

    fn block(&self, ptr: u64) -> Option<&Block<'a>> {
        if ptr == 0 {
            return None;
        }
        self.by_ptr.get(&ptr).map(|&i| &self.blocks[i])
    }

    /// First of `names` that the file's SDNA defines.
    fn struct_idx(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.sdna.by_name.get(*n).copied())
    }

    fn view(&self, s: usize, data: &'a [u8]) -> Option<View<'_, 'a>> {
        let size = self.sdna.structs.get(s)?.size;
        Some(View { f: self, s, data: data.get(..size)? })
    }

    /// View of block `b` as its own SDNA struct (element 0).
    fn block_view(&self, b: &Block<'a>) -> Option<View<'_, 'a>> {
        self.view(b.sdna, b.data)
    }

    /// The struct a pointer refers to, read as the first known type of `names`.
    fn view_ptr(&self, ptr: u64, names: &[&str]) -> Option<View<'_, 'a>> {
        self.view(self.struct_idx(names)?, self.block(ptr)?.data)
    }

    /// `n` consecutive structs of type `names` at `ptr`.
    fn view_array(&self, ptr: u64, names: &[&str], n: usize) -> Option<Vec<View<'_, 'a>>> {
        let s = self.struct_idx(names)?;
        let size = self.sdna.structs[s].size;
        if size == 0 {
            return None;
        }
        let data = self.raw_array(ptr, size, n)?;
        (0..n).map(|i| self.view(s, &data[i * size..])).collect()
    }

    /// `n * elem` bytes at `ptr`, if the block is large enough.
    fn raw_array(&self, ptr: u64, elem: usize, n: usize) -> Option<&'a [u8]> {
        let need = elem.checked_mul(n)?;
        self.block(ptr)?.data.get(..need)
    }

    fn read_ptr(&self, d: &[u8], off: usize) -> Option<u64> {
        if self.ptr_size == 4 {
            self.e.u32(d, off).map(u64::from)
        } else {
            self.e.u64(d, off)
        }
    }

    /// An array of `n` pointers stored at `ptr`.
    fn ptr_array(&self, ptr: u64, n: usize) -> Vec<u64> {
        match self.raw_array(ptr, self.ptr_size, n) {
            Some(d) => (0..n).filter_map(|i| self.read_ptr(d, i * self.ptr_size)).collect(),
            None => Vec::new(),
        }
    }

    /// Follow a `ListBase` (`first` pointer, `next` links) of structs `names`.
    fn list(&self, lb: Option<View<'_, 'a>>, names: &[&str]) -> Vec<View<'_, 'a>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut p = lb.map_or(0, |l| l.ptr("first"));
        while p != 0 && seen.insert(p) && out.len() < 100_000 {
            match self.view_ptr(p, names) {
                Some(v) => {
                    p = v.ptr("next");
                    out.push(v);
                }
                None => break,
            }
        }
        out
    }
}

fn endian_char(c: u8) -> Option<bool> {
    match c {
        b'v' => Some(false),
        b'V' => Some(true),
        _ => None,
    }
}

/// One struct instance, with members found by name.
#[derive(Clone, Copy)]
struct View<'f, 'a> {
    f: &'f BlendFile<'a>,
    s: usize,
    data: &'a [u8],
}

impl<'f, 'a> View<'f, 'a> {
    fn member(&self, name: &str) -> Option<&'f Member> {
        self.f.sdna.structs[self.s].members.get(name)
    }
    fn has(&self, name: &str) -> bool {
        self.member(name).is_some()
    }
    /// Element `i` of a numeric member.
    fn num_at(&self, name: &str, i: usize) -> Option<f64> {
        let m = self.member(name)?;
        if m.is_ptr || i >= m.len {
            return None;
        }
        self.f.e.prim(self.data, m.offset + i * m.elem, m.prim)
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.num_at(name, 0)
    }
    /// First of several alternative member names.
    fn num_any(&self, names: &[&str]) -> Option<f64> {
        names.iter().find_map(|n| self.num(n))
    }
    fn f32(&self, name: &str) -> Option<f32> {
        self.num(name).map(|v| v as f32)
    }
    fn vec<const N: usize>(&self, name: &str) -> Option<[f32; N]> {
        let mut a = [0.0; N];
        for (i, x) in a.iter_mut().enumerate() {
            *x = self.num_at(name, i)? as f32;
        }
        Some(a)
    }
    fn vec3(&self, name: &str) -> Option<Vec3> {
        self.vec::<3>(name).map(Vec3::from_array)
    }
    fn ptr(&self, name: &str) -> u64 {
        match self.member(name) {
            Some(m) if m.is_ptr => self.f.read_ptr(self.data, m.offset).unwrap_or(0),
            _ => 0,
        }
    }
    fn string(&self, name: &str) -> Option<String> {
        let m = self.member(name)?;
        if m.is_ptr || m.elem != 1 {
            return None;
        }
        Some(c_string(self.data.get(m.offset..m.offset + m.len)?))
    }
    /// A nested (by-value) struct member.
    fn sub(&self, name: &str) -> Option<View<'f, 'a>> {
        let m = self.member(name)?;
        if m.is_ptr {
            return None;
        }
        let s = (*self.f.sdna.struct_of_type.get(m.ty)?)?;
        self.f.view(s, self.data.get(m.offset..)?)
    }
    /// The ID name without its two-letter type code.
    fn id_name(&self) -> String {
        let n = self.sub("id").and_then(|id| id.string("name")).unwrap_or_default();
        n.chars().skip(2).collect()
    }
}

// ---------------------------------------------------------------------------
// Scene building

/// Render settings that camera FOVs depend on.
struct Render {
    width: f32,
    height: f32,
}

struct Importer<'f, 'a> {
    f: &'f BlendFile<'a>,
    scene: Scene,
    materials: HashMap<u64, usize>,
    meshes: HashMap<u64, usize>,
    render: Render,
}

impl<'f, 'a> Importer<'f, 'a> {
    fn new(f: &'f BlendFile<'a>) -> Self {
        Importer { f, scene: Scene::new(), materials: HashMap::new(), meshes: HashMap::new(), render: Render { width: 1920.0, height: 1080.0 } }
    }

    /// Every ID block with a two-letter `code`, with its old address.
    fn id_blocks(&self, code: &[u8; 2]) -> Vec<(u64, View<'f, 'a>)> {
        self.f
            .blocks
            .iter()
            .filter(|b| b.code[..2] == code[..] && b.code[2] == 0 && b.nr >= 1)
            .filter_map(|b| Some((b.old, self.f.block_view(b)?)))
            .collect()
    }

    fn run(mut self) -> Result<Scene, String> {
        let f = self.f;
        // Current scene: FileGlobal.curscene, else the first SC block.
        let glob = f.blocks.iter().find(|b| &b.code == b"GLOB").and_then(|b| f.view(f.struct_idx(&["FileGlobal"])?, b.data));
        let sc = f.view_ptr(glob.map_or(0, |g| g.ptr("curscene")), &["Scene"]).or_else(|| self.id_blocks(b"SC").into_iter().next().map(|(_, v)| v));
        let mut camera_ptr = 0;
        if let Some(sc) = sc {
            self.scene.name = sc.id_name();
            camera_ptr = sc.ptr("camera");
            if let Some(r) = sc.sub("r") {
                let (xs, ys) = (r.num("xsch").unwrap_or(0.0), r.num("ysch").unwrap_or(0.0));
                let (xa, ya) = (r.num("xasp").unwrap_or(1.0), r.num("yasp").unwrap_or(1.0));
                let (w, h) = ((xs * xa) as f32, (ys * ya) as f32);
                if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
                    self.render = Render { width: w, height: h };
                }
                if let (Some(s), Some(e)) = (r.num("sfra"), r.num("efra")) {
                    self.scene.frame_start = s as i32;
                    self.scene.frame_end = e as i32;
                }
                if let Some(fps) = r.num("frs_sec") {
                    let base = r.num("frs_sec_base").filter(|b| *b > 0.0).unwrap_or(1.0);
                    let fps = (fps / base) as f32;
                    if fps.is_finite() && fps > 0.0 {
                        self.scene.fps = fps;
                    }
                }
            }
        }

        for (old, ma) in self.id_blocks(b"MA") {
            let m = self.material(&ma);
            let idx = self.scene.add_material(m);
            self.materials.insert(old, idx);
        }

        // Objects, then parents once every object has an index.
        let mut by_ptr = HashMap::new();
        let mut pending = Vec::new();
        for (old, ob) in self.id_blocks(b"OB") {
            let idx = self.object(&ob)?;
            by_ptr.insert(old, idx);
            pending.push((idx, ob));
        }
        for (idx, ob) in pending {
            let Some(&p) = by_ptr.get(&ob.ptr("parent")) else { continue };
            if p == idx {
                continue;
            }
            self.scene.objects[idx].parent = Some(p);
            if let Some(inv) = ob.vec::<16>("parentinv") {
                let inv = Mat4::from_cols_array(inv);
                if inv != Mat4::IDENTITY && inv.to_cols_array().iter().all(|v| v.is_finite()) {
                    let local = inv * self.scene.objects[idx].transform.matrix();
                    self.scene.objects[idx].transform = Transform::from_matrix(&local);
                }
            }
        }
        self.scene.active_camera = by_ptr.get(&camera_ptr).copied();
        if self.scene.active_camera.is_none() {
            self.scene.active_camera = self.scene.objects.iter().position(|o| matches!(o.data, ObjectData::Camera(_)));
        }
        Ok(self.scene)
    }

    fn object(&mut self, ob: &View<'f, 'a>) -> Result<usize, String> {
        let f = self.f;
        let data = ob.ptr("data");
        let ty = ob.num("type").unwrap_or(0.0) as i64;
        let mut mesh_view = None;
        let od = match ty {
            1 => match f.view_ptr(data, &["Mesh"]) {
                Some(me) => {
                    mesh_view = Some(me);
                    ObjectData::Mesh(self.mesh(data, &me)?)
                }
                None => ObjectData::Empty,
            },
            10 => f.view_ptr(data, &["Light", "Lamp"]).map_or(ObjectData::Empty, |la| ObjectData::Light(light(&la))),
            11 => f.view_ptr(data, &["Camera"]).map_or(ObjectData::Empty, |ca| ObjectData::Camera(self.camera(&ca))),
            _ => ObjectData::Empty,
        };
        let mut o = Object::new(&ob.id_name(), od);
        o.transform = basis(ob);
        o.materials = self.slots(ob, mesh_view.as_ref());
        Ok(self.scene.add_object(o))
    }

    /// Material slots: the object's own link where `matbits` says so, else the mesh's.
    fn slots(&self, ob: &View<'f, 'a>, me: Option<&View<'f, 'a>>) -> Vec<usize> {
        let f = self.f;
        let ob_n = count(ob.num("totcol").unwrap_or(0.0)).min(32767);
        let me_n = me.map_or(0, |m| count(m.num("totcol").unwrap_or(0.0)).min(32767));
        let n = ob_n.max(me_n);
        let ob_mat = f.ptr_array(ob.ptr("mat"), ob_n);
        let me_mat = me.map_or_else(Vec::new, |m| f.ptr_array(m.ptr("mat"), me_n));
        let bits = f.raw_array(ob.ptr("matbits"), 1, ob_n).unwrap_or(&[]);
        (0..n)
            .map(|i| {
                let on_object = bits.get(i).is_some_and(|&b| b != 0);
                let p = if on_object { ob_mat.get(i) } else { me_mat.get(i).or(ob_mat.get(i)) };
                p.and_then(|p| self.materials.get(p)).copied().unwrap_or(usize::MAX)
            })
            .collect()
    }

    fn camera(&self, ca: &View<'f, 'a>) -> Camera {
        let mut c = Camera::default();
        let (w, h) = (self.render.width, self.render.height);
        let lens = ca.f32("lens").filter(|l| *l > 0.0).unwrap_or(50.0);
        let sx = ca.f32("sensor_x").filter(|s| *s > 0.0).unwrap_or(36.0);
        let sy = ca.f32("sensor_y").filter(|s| *s > 0.0).unwrap_or(24.0);
        // Fit: 0 auto (sensor_x spans the larger side), 1 horizontal, 2 vertical.
        let fit = match ca.num("sensor_fit").unwrap_or(0.0) as i64 {
            1 => Fit::Horizontal(sx),
            2 => Fit::Vertical(sy),
            _ if w >= h => Fit::Horizontal(sx),
            _ => Fit::Vertical(sx),
        };
        let half = |size: f32| (size * 0.5 / lens).atan();
        c.fov_y = match fit {
            Fit::Vertical(s) => 2.0 * half(s),
            Fit::Horizontal(s) => 2.0 * (half(s).tan() * h / w).atan(),
        };
        if ca.num("type").unwrap_or(0.0) as i64 == 1 {
            let os = ca.f32("ortho_scale").unwrap_or(6.0);
            c.ortho_scale = Some(match fit {
                Fit::Vertical(_) => os,
                Fit::Horizontal(_) => os * h / w,
            });
        }
        if let Some(n) = ca.num_any(&["clip_start", "clipsta"]) {
            c.near = n as f32;
        }
        if let Some(n) = ca.num_any(&["clip_end", "clipend"]) {
            c.far = n as f32;
        }
        c
    }

    fn material(&self, ma: &View<'f, 'a>) -> Material {
        let mut m = Material { name: ma.id_name(), ..Material::default() };
        if let (Some(r), Some(g), Some(b)) = (ma.f32("r"), ma.f32("g"), ma.f32("b")) {
            m.base_color = [r, g, b, ma.f32("a").unwrap_or(1.0)];
        }
        if let Some(v) = ma.f32("metallic") {
            m.metallic = v;
        }
        if let Some(v) = ma.f32("roughness") {
            m.roughness = v;
        }
        if ma.num("use_nodes") != Some(0.0) {
            self.principled(ma, &mut m);
        }
        m
    }

    /// Overlay the inputs of the first Principled BSDF node, if any.
    fn principled(&self, ma: &View<'f, 'a>, m: &mut Material) {
        let f = self.f;
        let Some(tree) = f.view_ptr(ma.ptr("nodetree"), &["bNodeTree"]) else { return };
        let Some(node) = f.list(tree.sub("nodes"), &["bNode"]).into_iter().find(|n| n.string("idname").as_deref() == Some("ShaderNodeBsdfPrincipled")) else { return };
        let mut emission_color = None;
        let mut emission_strength = None;
        for s in f.list(node.sub("inputs"), &["bNodeSocket"]) {
            let name = s.string("name").unwrap_or_default();
            let dv = s.ptr("default_value");
            let rgba = || f.view_ptr(dv, &["bNodeSocketValueRGBA"]).and_then(|v| v.vec::<4>("value"));
            let float = || f.view_ptr(dv, &["bNodeSocketValueFloat"]).and_then(|v| v.f32("value"));
            match name.as_str() {
                "Base Color" => {
                    if let Some(c) = rgba() {
                        m.base_color = [c[0], c[1], c[2], m.base_color[3]];
                    }
                }
                "Alpha" => m.base_color[3] = float().unwrap_or(m.base_color[3]),
                "Metallic" => m.metallic = float().unwrap_or(m.metallic),
                "Roughness" => m.roughness = float().unwrap_or(m.roughness),
                "IOR" => m.ior = float().unwrap_or(m.ior),
                "Transmission" | "Transmission Weight" => m.transmission = float().unwrap_or(m.transmission),
                "Emission" | "Emission Color" => emission_color = rgba(),
                "Emission Strength" => emission_strength = float(),
                _ => {}
            }
        }
        if let Some(c) = emission_color {
            m.emission = Vec3::new(c[0], c[1], c[2]);
            // Before 2.91 there was no strength input: the colour was the radiance.
            m.emission_strength = emission_strength.unwrap_or(1.0);
        }
    }

    /// Build the mesh at `ptr` once; objects sharing it share the scene mesh.
    fn mesh(&mut self, ptr: u64, me: &View<'f, 'a>) -> Result<usize, String> {
        if let Some(&i) = self.meshes.get(&ptr) {
            return Ok(i);
        }
        let name = me.id_name();
        let mesh = read_mesh(self.f, me).map_err(|e| format!("blend: mesh {name:?}: {e}"))?;
        let idx = self.scene.add_mesh(&name, mesh);
        self.meshes.insert(ptr, idx);
        Ok(idx)
    }
}

enum Fit {
    Horizontal(f32),
    Vertical(f32),
}

fn light(la: &View<'_, '_>) -> Light {
    let mut l = Light::default();
    let ty = la.num("type").unwrap_or(0.0) as i64;
    l.kind = match ty {
        1 | 3 => LightKind::Sun,
        2 => LightKind::Spot { angle: la.f32("spotsize").unwrap_or(std::f32::consts::FRAC_PI_4), blend: la.f32("spotblend").unwrap_or(0.15) },
        4 => LightKind::Area { size: la.f32("area_size").unwrap_or(1.0) },
        _ => LightKind::Point,
    };
    if let (Some(r), Some(g), Some(b)) = (la.f32("r"), la.f32("g"), la.f32("b")) {
        l.color = Vec3::new(r, g, b);
    }
    if let Some(e) = la.f32("energy") {
        l.power = e;
    }
    match l.kind {
        LightKind::Sun => {
            if let Some(a) = la.f32("sun_angle") {
                l.radius = a;
            }
        }
        LightKind::Point | LightKind::Spot { .. } => {
            let r = if la.has("radius") { la.f32("radius") } else { la.f32("area_size") };
            if let Some(r) = r {
                l.radius = r;
            }
        }
        LightKind::Area { .. } => {}
    }
    l
}

/// Euler rotation with Blender's rotation-mode axis order (first letter applied first).
fn euler(r: Vec3, mode: i64) -> Quat {
    let ax = |i: usize| match i {
        0 => Quat::from_axis_angle(Vec3::X, r.x),
        1 => Quat::from_axis_angle(Vec3::Y, r.y),
        _ => Quat::from_axis_angle(Vec3::Z, r.z),
    };
    let order: [usize; 3] = match mode {
        2 => [0, 2, 1], // XZY
        3 => [1, 0, 2], // YXZ
        4 => [1, 2, 0], // YZX
        5 => [2, 0, 1], // ZXY
        6 => [2, 1, 0], // ZYX
        _ => [0, 1, 2], // XYZ
    };
    ax(order[2]) * ax(order[1]) * ax(order[0])
}

/// The object's local basis from loc/rot/scale (plus deltas), falling back to its stored matrix.
fn basis(ob: &View<'_, '_>) -> Transform {
    let Some(loc) = ob.vec3("loc") else {
        let m = ob.vec::<16>("object_to_world").or_else(|| ob.vec::<16>("obmat"));
        return m.map_or_else(Transform::default, |m| Transform::from_matrix(&Mat4::from_cols_array(m)));
    };
    let location = loc + ob.vec3("dloc").unwrap_or(Vec3::ZERO);
    let mut scale = ob.vec3("scale").or_else(|| ob.vec3("size")).unwrap_or(Vec3::ONE);
    if let Some(ds) = ob.vec3("dscale").filter(|d| *d != Vec3::ZERO) {
        scale = scale.mul_elem(ds);
    }
    let mode = ob.num("rotmode").unwrap_or(1.0) as i64;
    let rotation = match mode {
        0 => {
            let q = |n: &str| ob.vec::<4>(n).map(|q| Quat { w: q[0], x: q[1], y: q[2], z: q[3] }.normalize());
            q("dquat").unwrap_or(Quat::IDENTITY) * q("quat").unwrap_or(Quat::IDENTITY)
        }
        -1 => {
            let axis = ob.vec3("rotAxis").unwrap_or(Vec3::Y);
            let angle = ob.f32("rotAngle").unwrap_or(0.0);
            let dq = euler(ob.vec3("drot").unwrap_or(Vec3::ZERO), 1);
            if axis.length_squared() > 0.0 { dq * Quat::from_axis_angle(axis, angle) } else { dq }
        }
        _ => {
            let rot = ob.vec3("rot").unwrap_or(Vec3::ZERO);
            let drot = ob.vec3("drot").unwrap_or(Vec3::ZERO);
            euler(drot, mode) * euler(rot, mode)
        }
    };
    Transform { location, rotation, scale }
}

// ---------------------------------------------------------------------------
// Meshes

/// One CustomData layer.
struct Layer {
    ty: i64,
    name: String,
    active_rnd: usize,
    data: u64,
}

/// The layers of the first CustomData member of `names` (4.0 renamed `vdata` to `vert_data`, ...).
fn layers(f: &BlendFile<'_>, me: &View<'_, '_>, names: &[&str]) -> Vec<Layer> {
    let Some(cd) = names.iter().find_map(|n| me.sub(n)) else { return Vec::new() };
    let n = count(cd.num("totlayer").unwrap_or(0.0));
    let Some(s) = f.struct_idx(&["CustomDataLayer"]) else { return Vec::new() };
    let size = f.sdna.structs[s].size;
    let ptr = cd.ptr("layers");
    let Some(block) = f.block(ptr) else { return Vec::new() };
    let n = n.min(block.data.len() / size.max(1));
    (0..n)
        .filter_map(|i| f.view(s, &block.data[i * size..]))
        .map(|l| Layer {
            ty: l.num("type").unwrap_or(-1.0) as i64,
            name: l.string("name").unwrap_or_default(),
            active_rnd: count(l.num("active_rnd").unwrap_or(0.0)),
            data: l.ptr("data"),
        })
        .collect()
}

fn named<'l>(ls: &'l [Layer], name: &str) -> Option<&'l Layer> {
    ls.iter().find(|l| l.name == name && l.data != 0)
}
fn typed(ls: &[Layer], ty: i64) -> Option<&Layer> {
    ls.iter().find(|l| l.ty == ty && l.data != 0)
}

/// `n` items decoded by `read` at a stride of `elem` bytes.
fn read_items<T>(data: &[u8], n: usize, elem: usize, read: impl Fn(&[u8]) -> Option<T>) -> Option<Vec<T>> {
    (0..n).map(|i| read(data.get(i * elem..)?)).collect()
}

fn read_mesh(f: &BlendFile<'_>, me: &View<'_, '_>) -> Result<Mesh, String> {
    let e = f.e;
    let nv = count(me.num_any(&["verts_num", "totvert"]).unwrap_or(0.0));
    let ne = count(me.num_any(&["edges_num", "totedge"]).unwrap_or(0.0));
    let nf = count(me.num_any(&["faces_num", "totpoly", "polys_num"]).unwrap_or(0.0));
    let nl = count(me.num_any(&["corners_num", "totloop"]).unwrap_or(0.0));
    let ntess = count(me.num_any(&["totface_legacy", "totface"]).unwrap_or(0.0));
    let (vd, ed, pd, ld) = (
        layers(f, me, &["vert_data", "vdata"]),
        layers(f, me, &["edge_data", "edata"]),
        layers(f, me, &["face_data", "pdata"]),
        layers(f, me, &["corner_data", "ldata"]),
    );
    let vec3_at = |d: &[u8]| Some(Vec3::new(e.f32(d, 0)?, e.f32(d, 4)?, e.f32(d, 8)?));
    let i32_at = |d: &[u8]| e.i32(d, 0);

    // Positions.
    let positions = if let Some(l) = named(&vd, "position") {
        f.raw_array(l.data, 12, nv).and_then(|d| read_items(d, nv, 12, vec3_at))
    } else {
        let p = typed(&vd, CD_MVERT).map_or(me.ptr("mvert"), |l| l.data);
        f.view_array(p, &["MVert"], nv).and_then(|vs| vs.iter().map(|v| v.vec3("co")).collect())
    }
    .ok_or("missing or short vertex positions")?;

    let vert = |v: i64| -> Result<u32, String> {
        if v >= 0 && (v as usize) < nv {
            Ok(v as u32)
        } else {
            Err(format!("vertex index {v} out of range"))
        }
    };

    // Face corners: (start, size, material, smooth) per face, plus corner vertices.
    let mut faces = Vec::new();
    let offsets_ptr = [me.ptr("face_offset_indices"), me.ptr("poly_offset_indices")].into_iter().find(|&p| p != 0);
    let mpoly = typed(&pd, CD_MPOLY).map_or(me.ptr("mpoly"), |l| l.data);
    let mut polys: Vec<(usize, usize, i64, Option<bool>)> = Vec::new();
    if nf > 0 {
        if let Some(op) = offsets_ptr {
            let offs = f.raw_array(op, 4, nf + 1).and_then(|d| read_items(d, nf + 1, 4, i32_at)).ok_or("short face offsets")?;
            for w in offs.windows(2) {
                if w[0] < 0 || w[1] < w[0] {
                    return Err("bad face offsets".into());
                }
                polys.push((w[0] as usize, (w[1] - w[0]) as usize, 0, None));
            }
        } else if let Some(ps) = f.view_array(mpoly, &["MPoly"], nf) {
            for p in ps {
                let start = p.num("loopstart").unwrap_or(-1.0);
                let len = p.num("totloop").unwrap_or(-1.0);
                if start < 0.0 || len < 0.0 {
                    return Err("bad polygon".into());
                }
                let smooth = p.num("flag").map(|fl| fl as i64 & 1 != 0);
                polys.push((start as usize, len as usize, p.num("mat_nr").unwrap_or(0.0) as i64, smooth));
            }
        } else {
            return Err("missing faces".into());
        }
    }
    if !polys.is_empty() {
        let corner_verts: Vec<i64> = if let Some(l) = named(&ld, ".corner_vert") {
            f.raw_array(l.data, 4, nl).and_then(|d| read_items(d, nl, 4, i32_at)).map(|v| v.into_iter().map(i64::from).collect())
        } else {
            let p = typed(&ld, CD_MLOOP).map_or(me.ptr("mloop"), |l| l.data);
            f.view_array(p, &["MLoop"], nl).and_then(|ls| ls.iter().map(|l| l.num("v").map(|v| v as i64)).collect())
        }
        .ok_or("missing or short face corners")?;

        // UVs: the active float2 corner attribute, else MLoopUV.
        let uv_layers: Vec<&Layer> = ld.iter().filter(|l| l.ty == CD_PROP_FLOAT2 && !l.name.starts_with('.') && l.data != 0).collect();
        let uvs: Option<Vec<Vec2>> = if let Some(first) = uv_layers.first() {
            let l = uv_layers.get(first.active_rnd).unwrap_or(first);
            f.raw_array(l.data, 8, nl).and_then(|d| read_items(d, nl, 8, |d| Some(Vec2::new(e.f32(d, 0)?, e.f32(d, 4)?))))
        } else {
            let p = ld.iter().filter(|l| l.ty == CD_MLOOPUV && l.data != 0).collect::<Vec<_>>();
            let p = p.first().map_or(me.ptr("mloopuv"), |first| p.get(first.active_rnd).unwrap_or(first).data);
            f.view_array(p, &["MLoopUV"], nl).and_then(|ls| ls.iter().map(|l| l.vec::<2>("uv").map(|a| Vec2::new(a[0], a[1]))).collect())
        };
        let mat_idx = named(&pd, "material_index").and_then(|l| f.raw_array(l.data, 4, nf)).and_then(|d| read_items(d, nf, 4, i32_at));
        let sharp = named(&pd, "sharp_face").and_then(|l| f.raw_array(l.data, 1, nf));

        for (i, &(start, len, mat_nr, smooth)) in polys.iter().enumerate() {
            let end = start.checked_add(len).filter(|&e| e <= corner_verts.len()).ok_or("face corners out of range")?;
            let verts = corner_verts[start..end].iter().map(|&v| vert(v)).collect::<Result<Vec<_>, _>>()?;
            let uv = uvs.as_ref().map_or_else(Vec::new, |u| u[start..end].to_vec());
            let material = mat_idx.as_ref().map_or(mat_nr, |m| m[i] as i64).clamp(0, u16::MAX as i64) as u16;
            let smooth = match (sharp, smooth) {
                (Some(s), _) => s[i] == 0,
                (None, Some(sm)) => sm,
                (None, None) => true,
            };
            faces.push(Face { verts, uvs: uv, material, smooth });
        }
    } else if ntess > 0 {
        // Pre-2.63 meshes: only tessellated triangles and quads.
        let p = typed(&layers(f, me, &["fdata_legacy", "fdata"]), CD_MFACE).map_or(me.ptr("mface"), |l| l.data);
        for t in f.view_array(p, &["MFace"], ntess).ok_or("missing tessfaces")? {
            let ids = ["v1", "v2", "v3", "v4"].map(|n| t.num(n).unwrap_or(-1.0) as i64);
            let k = if ids[3] == 0 { 3 } else { 4 };
            let verts = ids[..k].iter().map(|&v| vert(v)).collect::<Result<Vec<_>, _>>()?;
            let material = t.num("mat_nr").unwrap_or(0.0).clamp(0.0, u16::MAX as f64) as u16;
            let smooth = t.num("flag").is_some_and(|fl| fl as i64 & 1 != 0);
            faces.push(Face { verts, uvs: Vec::new(), material, smooth });
        }
    }

    // Edges not used by any face become loose (wire) edges.
    let edges: Option<Vec<[i64; 2]>> = if let Some(l) = named(&ed, ".edge_verts") {
        f.raw_array(l.data, 8, ne).and_then(|d| read_items(d, ne, 8, |d| Some([e.i32(d, 0)? as i64, e.i32(d, 4)? as i64])))
    } else {
        let p = typed(&ed, CD_MEDGE).map_or(me.ptr("medge"), |l| l.data);
        f.view_array(p, &["MEdge"], ne).and_then(|es| es.iter().map(|x| Some([x.num("v1")? as i64, x.num("v2")? as i64])).collect())
    };
    let mut mesh = Mesh { positions, faces, loose_edges: Vec::new() };
    if let Some(edges) = edges {
        let used: HashSet<(u32, u32)> = mesh.faces.iter().flat_map(|fc| fc.edges().map(|(a, b)| (a.min(b), a.max(b)))).collect();
        for [a, b] in edges {
            let (a, b) = (vert(a)?, vert(b)?);
            if a != b && !used.contains(&(a.min(b), a.max(b))) {
                mesh.loose_edges.push([a, b]);
            }
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_modes() {
        let r = Vec3::new(0.3, -0.5, 0.7);
        let q = |a: Vec3, b: Quat| (0..3).all(|i| (a[i] - b.rotate(Vec3::X)[i]).abs() < 1e-5);
        // XYZ is Blender's default order, which the math module already matches.
        assert!(q(Quat::from_euler_xyz(r.x, r.y, r.z).rotate(Vec3::X), euler(r, 1)));
        // ZYX applies Z first, so X is outermost.
        let zyx = Quat::from_axis_angle(Vec3::X, r.x) * Quat::from_axis_angle(Vec3::Y, r.y) * Quat::from_axis_angle(Vec3::Z, r.z);
        assert!(q(zyx.rotate(Vec3::X), euler(r, 6)));
    }

    #[test]
    fn member_declarators() {
        assert_eq!(parse_decl("*next"), Some(("next".into(), true, 1)));
        assert_eq!(parse_decl("**mat"), Some(("mat".into(), true, 1)));
        assert_eq!(parse_decl("obmat[4][4]"), Some(("obmat".into(), false, 16)));
        assert_eq!(parse_decl("(*func)(void *, int)"), Some(("func".into(), true, 1)));
        assert_eq!(parse_decl("*mtex[18]"), Some(("mtex".into(), true, 18)));
        assert_eq!(parse_decl("bad[x]"), None);
    }
}
