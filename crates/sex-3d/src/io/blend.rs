// SPDX-License-Identifier: GPL-3.0-only
//! Native reader for Blender `.blend` files.
//!
//! A clean-room reader written from the publicly documented structure of the
//! format (the "blend file" chapter of Blender's developer docs, the
//! `mystery of the blend` write-up and observation of the files themselves).
//! No Blender source code is used (the modifier mapping only relies on the
//! DNA member names and flag values noted below). Everything is bounds-checked: corrupt or
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
//! - `Scene`: `id`, `camera`, `world`, `master_collection` (2.8+), `r`
//!   (`RenderData`: `xsch`, `ysch`, `xasp`, `yasp`, `sfra`, `efra`, `frs_sec`,
//!   `frs_sec_base`). The current scene is `FileGlobal.curscene` in the `GLOB`
//!   block, else the first `SC` block.
//! - `World`: `horr horg horb` (viewport colour) and, when `use_nodes` is set,
//!   the `ShaderNodeBackground` node's `Color` and `Strength` inputs.
//! - `Collection` (2.8+): `id`, `gobject` (ListBase of `CollectionObject {ob}`),
//!   `children` (ListBase of `CollectionChild {collection}`), `flag` (bit 0 =
//!   disabled in viewports). Objects: `visibility_flag` (2.9+) or
//!   `restrictflag` (2.8 and older), bit 0 = disabled in viewports.
//! - Modifiers: `Object.modifiers` is a ListBase of blocks whose SDNA struct is
//!   the concrete type (`MirrorModifierData`, `SubsurfModifierData`, ...), each
//!   starting with `ModifierData modifier {next, prev, type, mode, name}`;
//!   `mode` bit 0 = shown in the viewport. The concrete struct is taken from
//!   each block's own SDNA index, so no modifier type numbers are hard-coded.
//!   Settings read: Mirror `flag` (axis bits 3..5, bit 7 = no merge),
//!   `tolerance`; Subsurf `subdivType` (1 = simple), `levels`; Array `count`,
//!   `offset_type` (bit 0 constant, bit 1 relative), `offset`, `scale`,
//!   `flags` (bit 0 merge); Solidify `offset` (thickness), `offset_fac`; Bevel
//!   `value`, `res`, `lim_flags` (bit 3 angle, 4 weight, 5 vertex group),
//!   `bevel_angle`, `affect_type` (0 vertices) or `flags` bit 1 before 2.90;
//!   Weld `merge_dist`; Displace `strength`, `midlevel`, `direction` (3 normal,
//!   5 custom normal), `texture`; Smooth `fac`, `repeat`.
//! - Animation: `Object.adt` -> `AnimData {action, slot_handle}` -> `bAction`.
//!   Up to 4.3 the F-curves are the `curves` ListBase of `FCurve {bezt, fpt,
//!   totvert, array_index, rna_path}`; 4.4+ "slotted" actions keep them in
//!   `strip_keyframe_data_array[] -> channelbag_array[] -> fcurve_array[]`,
//!   picking the channelbag whose `slot_handle` matches the AnimData's.
//!   `BezTriple {vec[3][3], ipo}`: `vec[1]` is the (frame, value) key.
//!
//! - Modifiers: `Object.modifiers` is a ListBase of per-type structs
//!   (`SubsurfModifierData`, `DecimateModifierData`, ...), each starting with
//!   a `ModifierData modifier` header (`next`, `type`, `mode`, `name`); the
//!   block's SDNA struct name says which type it is. Field names and flag
//!   bits are those of Blender's `DNA_modifier_types.h` (GPL-2+); types
//!   Polyamour has no equivalent for are skipped, as are entries switched off
//!   in the viewport (`mode` bit 0 clear).
//!
//! - Vertex groups: names from `Mesh.vertex_group_names` (3.0+) or
//!   `Object.defbase` (ListBase of `bDeformGroup {name}`); weights from
//!   `Mesh.dvert` or the `CD_MDEFORMVERT` (2) vertex layer: `MDeformVert {dw,
//!   totweight}` with `MDeformWeight {def_nr, weight}`.
//! - Shape keys: `Mesh.key` -> `Key {block, type (1 relative), ctime}` with a
//!   ListBase of `KeyBlock {name, data (float[3] x totelem), curval, pos,
//!   slidermin, slidermax, relative, flag (bit 0 mute), vgroup, type (0
//!   linear, 1 cardinal, 2 B-spline, 3 Catmull-Rom)}`.
//! - Armatures (object type 25): `bArmature {bonebase, collection_array,
//!   collection_array_num}`, nested `Bone {name, childbase, arm_head,
//!   arm_tail, arm_roll, arm_mat, flag, inherit_scale_mode, dist, weight,
//!   rad_head, rad_tail, segments}` (flag bits: 4 connected, 9 no inherit
//!   rotation, 11 envelope multiply, 12 no deform, 15 legacy no scale, 22 no
//!   local location, 23 relative parenting); `BoneCollection {name, bones}`
//!   of `BoneCollectionMember {bone}`. `Object.pose` -> `bPose {chanbase}` of
//!   `bPoseChannel {name, loc, size, eul, quat, rotAxis, rotAngle, rotmode,
//!   ikflag (bits 0-2 lock, 3-5 limit), limitmin, limitmax, stiffness,
//!   ikstretch, constraints}`. Bone parenting: `Object.partype` 7 with
//!   `parsubstr`; 4 is armature (deform) parenting.
//! - Constraints: `Object.constraints` / `bPoseChannel.constraints`, ListBase
//!   of `bConstraint {data, name, flag (bit 9 muted), ownspace, tarspace,
//!   enforce, headtail, space_object, space_subtarget}`; `data`'s struct says
//!   the type: `bLocateLikeConstraint`, `bRotateLikeConstraint`,
//!   `bSizeLikeConstraint`, `bTransLikeConstraint`, `bDistLimitConstraint`,
//!   `bLocLimitConstraint`, `bRotLimitConstraint`, `bSizeLimitConstraint`,
//!   `bSameVolumeConstraint`, `bTransformConstraint`, `bChildOfConstraint`,
//!   `bTrackToConstraint`, `bDampTrackConstraint`, `bLockTrackConstraint`,
//!   `bStretchToConstraint`, `bMinMaxConstraint` (Floor),
//!   `bArmatureConstraint` and `bKinematicConstraint` (IK), each with `tar` and
//!   `subtarget` plus its own members (see `blend/rig.rs`).
//! - Lattices (object type 22): `Lattice {pntsu, pntsv, pntsw, typeu, typev,
//!   typew, flag (bit 1 outside), def}` with `BPoint {vec[4]}`.
//! - Rigging modifiers: `ArmatureModifierData {object, deformflag (1 groups, 2
//!   envelopes, 4 preserve volume, 16 invert), multi, defgrp_name}`,
//!   `HookModifierData {object, subtarget, flag, falloff_type, falloff,
//!   parentinv, cent, indexar, indexar_num/totindex, force, name}`,
//!   `LatticeModifierData {object, name, strength, flag}`, `WarpModifierData
//!   {object_from, object_to, strength, falloff_radius, falloff_type, flag,
//!   defgrp_name}` and `WeightVGEdit/Mix/ProximityModifierData`; the deform
//!   modifiers' `defgrp_name`/`vgroup_name` and invert flags.
//!
//! Not imported yet: textures/images, view-layer visibility (the eye icon),
//! drivers, NLA, library links, custom split normals, modifier textures,
//! custom falloff curves, B-Bone shape settings and lattice vertex groups.
use crate::math::{Mat4, Quat, Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::modifiers::{BooleanOp, CastType, DecimateMode, Modifier, RemeshMode, ShrinkwrapMethod, SimpleDeformMode};
use crate::scene::{AreaShape, Camera, Light, LightKind, Material, Object, ObjectData, PanoramaType, ParentType, RayVisibility, Scene, SensorFit, Transform};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::Read;
mod nodes;

mod rig;

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
    import_blend_full(bytes).map(|(scene, _)| scene)
}

/// Like [`import_blend`], also returning the transform animation of every
/// object whose action animates `location`, `rotation_euler` or `scale`.
pub fn import_blend_full(bytes: &[u8]) -> Result<(Scene, Vec<BlendAction>), String> {
    let raw = decompress(bytes)?;
    let file = BlendFile::parse(&raw)?;
    Importer::new(&file).run()
}

/// Constant interpolation (Blender's `BEZT_IPO_CONST`).
pub const INTERP_CONSTANT: u8 = 0;
/// Linear interpolation (`BEZT_IPO_LIN`); also used for baked sample points.
pub const INTERP_LINEAR: u8 = 1;
/// Bezier interpolation (`BEZT_IPO_BEZ`), Blender's default. Larger values
/// are the easing curves (sine, quad, ..., elastic) in Blender's order.
pub const INTERP_BEZIER: u8 = 2;

/// F-curve properties of an object that are imported.
const ANIMATED_PATHS: [&str; 3] = ["location", "rotation_euler", "scale"];

/// One keyframe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendKey {
    pub frame: f32,
    pub value: f32,
    /// Interpolation towards the next key: [`INTERP_CONSTANT`],
    /// [`INTERP_LINEAR`], [`INTERP_BEZIER`] or a Blender easing mode.
    pub interpolation: u8,
}

/// One animated channel: element `index` of the property `path`.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendFCurve {
    /// RNA path, e.g. `"location"`.
    pub path: String,
    /// Array element (0 = X, 1 = Y, 2 = Z).
    pub index: usize,
    /// Keys in file order (Blender keeps them sorted by frame).
    pub keys: Vec<BlendKey>,
}

/// The transform F-curves an object's action holds.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendAction {
    /// Name of the object in the imported [`Scene`].
    pub object: String,
    /// Name of the Blender action.
    pub action: String,
    pub curves: Vec<BlendFCurve>,
    /// The object's parent inverse matrix (column-major) when it has a parent
    /// and the matrix is not identity. The imported transform already has it
    /// folded in; animated channel values do not, so a user of the curves
    /// applies it to each evaluated pose.
    pub parent_inverse: Option<[f32; 16]>,
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
    name: String,
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
            structs.push(StructDef { name: types[ty].clone(), size, members });
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

    /// The block at `ptr` read as its own SDNA struct, whatever that is.
    fn view_own(&self, ptr: u64) -> Option<View<'_, 'a>> {
        self.block_view(self.block(ptr)?)
    }

    /// The NUL-terminated string a `char *` points at.
    fn cstr_at(&self, ptr: u64) -> Option<String> {
        self.block(ptr).map(|b| c_string(b.data))
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
    /// Name of this view's SDNA struct.
    fn struct_name(&self) -> &'f str {
        &self.f.sdna.structs[self.s].name
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
    armatures: HashMap<u64, usize>,
    lattices: HashMap<u64, usize>,
    /// Imported `IM` and `NT` blocks by old address.
    images: HashMap<u64, usize>,
    groups: HashMap<u64, usize>,
    render: Render,
    actions: Vec<BlendAction>,
}

impl<'f, 'a> Importer<'f, 'a> {
    fn new(f: &'f BlendFile<'a>) -> Self {
        Importer {
            f,
            scene: Scene::new(),
            materials: HashMap::new(),
            meshes: HashMap::new(),
            armatures: HashMap::new(),
            images: HashMap::new(),
            groups: HashMap::new(),
            lattices: HashMap::new(),
            render: Render { width: 1920.0, height: 1080.0 },
            actions: Vec::new(),
        }
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

    fn run(mut self) -> Result<(Scene, Vec<BlendAction>), String> {
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
                    self.scene.frame_current = r.num("cfra").map_or(s as i32, |c| c as i32);
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
            // partype (low 4 bits): 4 armature (deform), 7 bone (parsubstr).
            match ob.num("partype").unwrap_or(0.0) as i64 & 15 {
                4 => self.scene.objects[idx].parent_type = ParentType::Armature,
                7 => {
                    let bone = ob.string("parsubstr").unwrap_or_default();
                    if !bone.is_empty() {
                        let o = &mut self.scene.objects[idx];
                        o.parent_type = ParentType::Bone;
                        o.parent_bone = bone;
                    }
                }
                _ => {}
            }
            if let Some(inv) = ob.vec::<16>("parentinv") {
                let inv = Mat4::from_cols_array(inv);
                if inv != Mat4::IDENTITY && inv.to_cols_array().iter().all(|v| v.is_finite()) {
                    let local = inv * self.scene.objects[idx].transform.matrix();
                    self.scene.objects[idx].transform = Transform::from_matrix(&local);
                    let name = &self.scene.objects[idx].name;
                    for a in self.actions.iter_mut().filter(|a| &a.object == name) {
                        a.parent_inverse = Some(inv.to_cols_array());
                    }
                }
            }
        }
        self.scene.active_camera = by_ptr.get(&camera_ptr).copied();
        if self.scene.active_camera.is_none() {
            self.scene.active_camera = self.scene.objects.iter().position(|o| matches!(o.data, ObjectData::Camera(_)));
        }
        if let Some(sc) = sc {
            self.collections(&sc, &by_ptr);
            self.world(&sc);
        }
        self.resolve_object_refs(&by_ptr);
        Ok((self.scene, self.actions))
    }

    /// The scene's collection tree (master collection = collection 0), in
    /// depth-first order. An object linked into several collections is placed
    /// in the first one met.
    fn collections(&mut self, sc: &View<'f, 'a>, objects: &HashMap<u64, usize>) {
        let f = self.f;
        let master_ptr = sc.ptr("master_collection");
        let Some(master) = f.view_ptr(master_ptr, &["Collection"]) else { return };
        let name = master.id_name();
        self.scene.collections[0].name = if name.is_empty() { "Scene Collection".into() } else { name };
        self.scene.collections[0].visible = collection_visible(&master);
        let mut placed = HashSet::new();
        let mut seen = HashSet::from([master_ptr]);
        let mut stack = vec![(master, 0usize)];
        while let Some((col, ci)) = stack.pop() {
            for co in f.list(col.sub("gobject"), &["CollectionObject"]) {
                if let Some(&oi) = objects.get(&co.ptr("ob")) {
                    if placed.insert(oi) {
                        self.scene.objects[oi].collection = ci;
                    }
                }
            }
            let mut children = Vec::new();
            for cc in f.list(col.sub("children"), &["CollectionChild"]) {
                let cp = cc.ptr("collection");
                if !seen.insert(cp) {
                    continue;
                }
                let Some(child) = f.view_ptr(cp, &["Collection"]) else { continue };
                let idx = self.scene.add_collection(&child.id_name(), Some(ci));
                self.scene.collections[idx].visible = collection_visible(&child);
                children.push((child, idx));
            }
            // Reversed so the first child is visited next (pre-order).
            stack.extend(children.into_iter().rev());
        }
    }

    /// World colour and strength: the Background node when nodes are used,
    /// else the viewport horizon colour.
    fn world(&mut self, sc: &View<'f, 'a>) {
        let f = self.f;
        let Some(wo) = f.view_ptr(sc.ptr("world"), &["World"]) else { return };
        if let (Some(r), Some(g), Some(b)) = (wo.f32("horr"), wo.f32("horg"), wo.f32("horb")) {
            self.scene.world.color = Vec3::new(r, g, b);
            self.scene.world.strength = 1.0;
        }
        if wo.num("use_nodes").unwrap_or(0.0) == 0.0 {
            return;
        }
        if let Some(tree) = f.view_ptr(wo.ptr("nodetree"), &["bNodeTree"]) {
            self.scene.world.node_tree = self.node_tree(&tree);
        }
        for (name, s) in self.node_inputs(&wo, "ShaderNodeBackground") {
            match name.as_str() {
                "Color" => {
                    if let Some(c) = socket_rgba(f, &s) {
                        self.scene.world.color = Vec3::new(c[0], c[1], c[2]);
                    }
                }
                "Strength" => {
                    if let Some(v) = socket_float(f, &s) {
                        self.scene.world.strength = v;
                    }
                }
                _ => {}
            }
        }
    }

    /// Name and socket of each input of the first `idname` node in the
    /// embedded node tree of `owner` (a material or world).
    fn node_inputs(&self, owner: &View<'f, 'a>, idname: &str) -> Vec<(String, View<'f, 'a>)> {
        let f = self.f;
        let Some(tree) = f.view_ptr(owner.ptr("nodetree"), &["bNodeTree"]) else { return Vec::new() };
        let Some(node) = f.list(tree.sub("nodes"), &["bNode"]).into_iter().find(|n| n.string("idname").as_deref() == Some(idname)) else { return Vec::new() };
        f.list(node.sub("inputs"), &["bNodeSocket"]).into_iter().map(|s| (s.string("name").unwrap_or_default(), s)).collect()
    }

    /// Transform F-curves of the object's active action.
    fn action(&self, ob: &View<'f, 'a>, object: &str) -> Option<BlendAction> {
        let f = self.f;
        let adt = f.view_ptr(ob.ptr("adt"), &["AnimData"])?;
        let act = f.view_ptr(adt.ptr("action"), &["bAction"])?;
        let mut fcurves = Vec::new();
        // 4.4+ slotted actions.
        let slot = adt.num("slot_handle");
        let kd_n = count(act.num("strip_keyframe_data_array_num").unwrap_or(0.0));
        for kd in f.ptr_array(act.ptr("strip_keyframe_data_array"), kd_n).into_iter().filter_map(|p| f.view_own(p)) {
            let cb_n = count(kd.num("channelbag_array_num").unwrap_or(0.0));
            for cb in f.ptr_array(kd.ptr("channelbag_array"), cb_n).into_iter().filter_map(|p| f.view_own(p)) {
                if slot.is_some() && cb.num("slot_handle") != slot {
                    continue;
                }
                let n = count(cb.num("fcurve_array_num").unwrap_or(0.0));
                fcurves.extend(f.ptr_array(cb.ptr("fcurve_array"), n).into_iter().filter_map(|p| f.view_ptr(p, &["FCurve"])));
            }
        }
        // Up to 4.3 (and 4.4's forward-compatible copy): the `curves` list.
        if fcurves.is_empty() {
            fcurves = f.list(act.sub("curves"), &["FCurve"]);
        }
        let curves: Vec<BlendFCurve> = fcurves.iter().filter_map(|fc| fcurve(f, fc)).collect();
        (!curves.is_empty()).then(|| BlendAction { object: object.to_string(), action: act.id_name(), curves, parent_inverse: None })
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
            22 => f.view_ptr(data, &["Lattice"]).map_or(ObjectData::Empty, |lt| {
                let name = lt.id_name();
                let i = *self.lattices.entry(data).or_insert_with(|| self.scene.add_lattice(&name, rig::lattice(f, &lt)));
                ObjectData::Lattice(i)
            }),
            25 => f.view_ptr(data, &["bArmature"]).map_or(ObjectData::Empty, |ar| {
                let name = ar.id_name();
                let i = *self.armatures.entry(data).or_insert_with(|| self.scene.add_armature(&name, rig::armature(f, &ar)));
                ObjectData::Armature(i)
            }),
            _ => ObjectData::Empty,
        };
        // Vertex group names lived on the object before 3.0.
        if let ObjectData::Mesh(mi) = od {
            let m = &mut self.scene.meshes[mi];
            if m.vertex_groups.is_empty() {
                m.vertex_groups = rig::group_names(f, ob.sub("defbase"));
            }
        }
        let mut o = Object::new(&ob.id_name(), od);
        o.transform = basis(ob);
        // visibility_flag: OB_HIDE_CAMERA 1<<3 .. OB_HIDE_SHADOW 1<<8, OB_HOLDOUT 1<<9.
        if let Some(v) = ob.num("visibility_flag") {
            let v = v as i64;
            let shown = |bit: i64| v & (1 << bit) == 0;
            o.visibility = sex_vis(shown(3), shown(4), shown(5), shown(6), shown(7), shown(8));
            o.holdout = v & (1 << 9) != 0;
        }
        if let Some(i) = ob.num("index") {
            o.pass_index = i.max(0.0) as u32;
        }
        o.materials = self.slots(ob, mesh_view.as_ref());
        if mesh_view.is_some() {
            o.modifiers = modifiers(f, ob);
        }
        o.constraints = rig::constraints(f, ob.sub("constraints"));
        if matches!(o.data, ObjectData::Armature(_)) {
            o.pose = rig::pose(f, ob);
        }
        let hide = ob.num_any(&["visibility_flag", "restrictflag"]).unwrap_or(0.0) as i64;
        o.visible = hide & 1 == 0;
        let idx = self.scene.add_object(o);
        if let Some(a) = self.action(ob, &self.scene.objects[idx].name) {
            self.actions.push(a);
        }
        Ok(idx)
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
        c.sensor_width = sx;
        c.sensor_height = sy;
        c.sensor_fit = match ca.num("sensor_fit").unwrap_or(0.0) as i64 {
            1 => SensorFit::Horizontal,
            2 => SensorFit::Vertical,
            _ => SensorFit::Auto,
        };
        c.shift_x = ca.f32("shiftx").unwrap_or(0.0);
        c.shift_y = ca.f32("shifty").unwrap_or(0.0);
        if ca.num("type").unwrap_or(0.0) as i64 == 2 {
            c.panorama = Some(match ca.num("panorama_type").unwrap_or(0.0) as i64 {
                1 => PanoramaType::FisheyeEquidistant,
                2 => PanoramaType::FisheyeEquisolid,
                _ => PanoramaType::Equirectangular,
            });
            c.ortho_scale = None;
            for (name, dst) in [("fisheye_fov", &mut c.fisheye_fov), ("fisheye_lens", &mut c.fisheye_lens), ("latitude_min", &mut c.latitude_min), ("latitude_max", &mut c.latitude_max), ("longitude_min", &mut c.longitude_min), ("longitude_max", &mut c.longitude_max)] {
                if let Some(v) = ca.f32(name) {
                    *dst = v;
                }
            }
        }
        if let Some(dof) = ca.sub("dof") {
            c.dof.enabled = dof.num("flag").unwrap_or(0.0) as i64 & 1 != 0;
            c.dof.focus_distance = dof.f32("focus_distance").unwrap_or(c.dof.focus_distance);
            c.dof.fstop = dof.f32("aperture_fstop").unwrap_or(c.dof.fstop);
            c.dof.rotation = dof.f32("aperture_rotation").unwrap_or(0.0);
            c.dof.ratio = dof.f32("aperture_ratio").unwrap_or(1.0);
            c.dof.blades = dof.num("aperture_blades").unwrap_or(0.0).max(0.0) as u32;
        }
        c
    }

    fn material(&mut self, ma: &View<'f, 'a>) -> Material {
        let mut m = Material { name: ma.id_name(), ..Material::default() };
        if let Some(i) = ma.num("index") {
            m.pass_index = i.max(0.0) as u32;
        }
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
            if let Some(tree) = self.f.view_ptr(ma.ptr("nodetree"), &["bNodeTree"]) {
                m.node_tree = self.node_tree(&tree);
                // An image on the Principled base colour shows in the viewport.
                if let Some(t) = &m.node_tree {
                    let mut probe = Material { node_tree: Some(t.clone()), ..m.clone() };
                    if probe.sync_from_tree() && probe.base_color_texture.is_some() {
                        m.base_color_texture = probe.base_color_texture;
                        m.base_color = probe.base_color;
                    }
                }
            }
        }
        m
    }

    /// Overlay the inputs of the first Principled BSDF node, if any.
    fn principled(&self, ma: &View<'f, 'a>, m: &mut Material) {
        let f = self.f;
        let mut emission_color = None;
        let mut emission_strength = None;
        for (name, s) in self.node_inputs(ma, "ShaderNodeBsdfPrincipled") {
            let rgba = || socket_rgba(f, &s);
            let float = || socket_float(f, &s);
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

fn socket_rgba(f: &BlendFile<'_>, s: &View<'_, '_>) -> Option<[f32; 4]> {
    f.view_ptr(s.ptr("default_value"), &["bNodeSocketValueRGBA"]).and_then(|v| v.vec::<4>("value"))
}

fn socket_float(f: &BlendFile<'_>, s: &View<'_, '_>) -> Option<f32> {
    f.view_ptr(s.ptr("default_value"), &["bNodeSocketValueFloat"]).and_then(|v| v.f32("value"))
}

/// `Collection.flag` bit 0: disabled in viewports.
fn collection_visible(c: &View<'_, '_>) -> bool {
    c.num("flag").unwrap_or(0.0) as i64 & 1 == 0
}

/// A non-negative integer setting, clamped to `max`.
fn setting_u32(v: &View<'_, '_>, name: &str, default: u32, max: u32) -> u32 {
    match v.num(name) {
        Some(x) if x.is_finite() => x.clamp(0.0, max as f64) as u32,
        _ => default,
    }
}

/// A finite float setting.
fn setting_f32(v: &View<'_, '_>, name: &str, default: f32) -> f32 {
    v.f32(name).filter(|x| x.is_finite()).unwrap_or(default)
}

/// An imported transform F-curve, or `None` for other properties.
fn fcurve(f: &BlendFile<'_>, fc: &View<'_, '_>) -> Option<BlendFCurve> {
    let path = f.cstr_at(fc.ptr("rna_path"))?;
    if !ANIMATED_PATHS.contains(&path.as_str()) {
        return None;
    }
    let index = count(fc.num("array_index").unwrap_or(0.0));
    let n = count(fc.num("totvert").unwrap_or(0.0));
    let keys: Vec<BlendKey> = if let Some(bz) = f.view_array(fc.ptr("bezt"), &["BezTriple"], n) {
        // vec[3][3]: left handle, key, right handle.
        bz.iter()
            .filter_map(|b| Some(BlendKey { frame: b.num_at("vec", 3)? as f32, value: b.num_at("vec", 4)? as f32, interpolation: b.num("ipo").unwrap_or(2.0) as u8 }))
            .collect()
    } else if let Some(pts) = f.view_array(fc.ptr("fpt"), &["FPoint"], n) {
        pts.iter().filter_map(|p| Some(BlendKey { frame: p.num_at("vec", 0)? as f32, value: p.num_at("vec", 1)? as f32, interpolation: INTERP_LINEAR })).collect()
    } else {
        Vec::new()
    };
    let keys: Vec<BlendKey> = keys.into_iter().filter(|k| k.frame.is_finite() && k.value.is_finite()).collect();
    (!keys.is_empty() && index < 4).then_some(BlendFCurve { path, index, keys })
}

/// The object's modifier stack (`Object.modifiers`, a ListBase of
/// `*ModifierData` structs, each starting with a `ModifierData modifier`
/// header). Stack entries switched off in the viewport (`mode` bit 0) and
/// types Polyamour lacks are skipped.
fn modifiers(f: &BlendFile<'_>, ob: &View<'_, '_>) -> Vec<Modifier> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut p = ob.sub("modifiers").map_or(0, |l| l.ptr("first"));
    while p != 0 && seen.insert(p) && out.len() < 1000 {
        let Some(v) = f.block(p).and_then(|b| f.block_view(b)) else { break };
        let Some(md) = v.sub("modifier") else { break };
        p = md.ptr("next");
        if md.num("mode").is_some_and(|m| (m as i64) & 1 == 0) {
            continue;
        }
        if let Some(m) = modifier(f, &v) {
            out.push(m);
        }
    }
    out
}

/// Map one modifier block (read as its concrete SDNA struct) to a
/// [`Modifier`]; `None` for types or settings sex-3d cannot evaluate.
fn modifier_classic(md: &View<'_, '_>) -> Option<Modifier> {
    let flags = |name: &str| md.num(name).unwrap_or(0.0) as i64;
    Some(match md.struct_name() {
        "MirrorModifierData" => {
            let fl = flags("flag");
            Modifier::Mirror { x: fl & 1 << 3 != 0, y: fl & 1 << 4 != 0, z: fl & 1 << 5 != 0, merge: fl & 1 << 7 == 0, merge_distance: setting_f32(md, "tolerance", 0.001).max(0.0) }
        }
        "SubsurfModifierData" => Modifier::Subdivision { levels: setting_u32(md, "levels", 1, crate::modifiers::MAX_SUBDIVISION_LEVELS), simple: flags("subdivType") == 1 },
        "ArrayModifierData" => {
            let ot = md.num("offset_type").map_or(2, |v| v as i64);
            let vec = |name: &str, on: bool| if on { md.vec::<3>(name).filter(|v| v.iter().all(|x| x.is_finite())).unwrap_or([0.0; 3]) } else { [0.0; 3] };
            Modifier::Array { count: setting_u32(md, "count", 2, 10_000).max(1), relative: vec("scale", ot & 2 != 0), constant: vec("offset", ot & 1 != 0), merge: flags("flags") & 1 != 0 }
        }
        "SolidifyModifierData" => Modifier::Solidify { thickness: setting_f32(md, "offset", 0.01), offset: setting_f32(md, "offset_fac", -1.0) },
        "BevelModifierData" => {
            let vertex_only = if md.has("affect_type") { flags("affect_type") == 0 } else { flags("flags") & 1 << 1 != 0 };
            let lim = flags("lim_flags");
            if vertex_only || lim & (1 << 4 | 1 << 5) != 0 {
                return None; // vertex bevels and weight / vertex-group limits
            }
            let angle = if lim & 1 << 3 != 0 { setting_f32(md, "bevel_angle", 30f32.to_radians()) } else { 0.0 };
            Modifier::Bevel { width: setting_f32(md, "value", 0.1), segments: setting_u32(md, "res", 1, 1000).max(1), angle }
        }
        "TriangulateModifierData" => Modifier::Triangulate,
        "WeldModifierData" => Modifier::Weld { distance: setting_f32(md, "merge_dist", 0.001).max(0.0) },
        "DisplaceModifierData" => {
            // Without a texture Blender offsets by (1 - midlevel) * strength.
            let dir = md.num("direction").map_or(3, |v| v as i64);
            if md.ptr("texture") != 0 || !(dir == 3 || dir == 5) {
                return None;
            }
            Modifier::Displace { strength: (1.0 - setting_f32(md, "midlevel", 0.5)) * setting_f32(md, "strength", 1.0), vertex_group: None, invert_vertex_group: false }
        }
        "SmoothModifierData" => Modifier::Smooth { factor: setting_f32(md, "fac", 0.5), iterations: setting_u32(md, "repeat", 1, 10_000), vertex_group: None, invert_vertex_group: false },
        _ => return None,
    })
}

/// One `*ModifierData` struct as a [`Modifier`], with its vertex group.
fn modifier(f: &BlendFile<'_>, v: &View<'_, '_>) -> Option<Modifier> {
    let mut m = rig::modifier(f, v).or_else(|| modifier_base(f, v))?;
    if let (Some((g, inv)), Some((mg, minv))) = (rig::modifier_vertex_group(v), m.vertex_group_mut()) {
        *mg = (!g.is_empty()).then_some(g);
        *minv = inv;
    }
    Some(m)
}

/// One `*ModifierData` struct as a [`Modifier`], by its DNA struct name.
/// Member names and flag bits follow Blender's `DNA_modifier_types.h`.
fn modifier_base(f: &BlendFile<'_>, v: &View<'_, '_>) -> Option<Modifier> {
    let flag = |name: &str| v.num(name).unwrap_or(0.0) as i64;
    let fl = |name: &str, d: f32| v.f32(name).filter(|x| x.is_finite()).unwrap_or(d);
    let int = |name: &str, d: u32| v.num(name).filter(|x| x.is_finite() && *x >= 0.0).map_or(d, |x| x.min(u32::MAX as f64) as u32);
    let bit = |name: &str, b: i64| flag(name) & b != 0;
    let object_name = |name: &str| f.view_ptr(v.ptr(name), &["Object"]).map(|o| o.id_name()).unwrap_or_default();
    // The nine original types keep their stricter mapping (vertex bevels,
    // textured or axis displaces and weight limits are skipped).
    if matches!(
        v.struct_name(),
        "MirrorModifierData" | "SubsurfModifierData" | "ArrayModifierData" | "SolidifyModifierData" | "BevelModifierData" | "TriangulateModifierData" | "WeldModifierData" | "DisplaceModifierData" | "SmoothModifierData"
    ) {
        return modifier_classic(v);
    }
    let mut m = match f.sdna.structs[v.s].name.as_str() {
        "SubsurfModifierData" => Modifier::Subdivision { levels: int("levels", 1).min(6), simple: flag("subdivType") == 1 },
        "MirrorModifierData" => {
            let fg = flag("flag");
            let (mut x, y, z) = (fg & (1 << 3) != 0, fg & (1 << 4) != 0, fg & (1 << 5) != 0);
            if !(x || y || z) {
                x = true;
            }
            Modifier::Mirror { x, y, z, merge: fg & (1 << 7) == 0, merge_distance: fl("tolerance", 0.001) }
        }
        "ArrayModifierData" => {
            let ot = flag("offset_type");
            let rel = if ot & 2 != 0 { v.vec::<3>("scale").unwrap_or([1.0, 0.0, 0.0]) } else { [0.0; 3] };
            let con = if ot & 1 != 0 { v.vec::<3>("offset").unwrap_or([0.0; 3]) } else { [0.0; 3] };
            Modifier::Array { count: int("count", 2).clamp(1, 10_000), relative: rel, constant: con, merge: bit("flags", 1) }
        }
        "SolidifyModifierData" => Modifier::Solidify { thickness: fl("offset", 0.01), offset: fl("offset_fac", -1.0) },
        "BevelModifierData" => Modifier::Bevel { width: fl("value", 0.1), segments: int("res", 1).max(1), angle: if bit("lim_flags", 1 << 3) { fl("bevel_angle", 0.5236) } else { 0.0 } },
        "TriangulateModifierData" => Modifier::Triangulate,
        "WeldModifierData" => Modifier::Weld { distance: fl("merge_dist", 0.001) },
        "DisplaceModifierData" => Modifier::Displace { strength: fl("strength", 1.0), vertex_group: None, invert_vertex_group: false },
        "SmoothModifierData" => Modifier::Smooth { factor: fl("fac", 0.5), iterations: int("repeat", 1), vertex_group: None, invert_vertex_group: false },
        "DecimateModifierData" => Modifier::Decimate {
            mode: match flag("mode") {
                1 => DecimateMode::Unsubdivide,
                2 => DecimateMode::Planar,
                _ => DecimateMode::Collapse,
            },
            ratio: fl("percent", 1.0),
            iterations: int("iter", 0),
            angle: fl("angle", 0.0873),
            triangulate: bit("flag", 1 << 1),
        },
        "RemeshModifierData" => Modifier::Remesh {
            mode: match flag("mode") {
                0 => RemeshMode::Blocks,
                1 | 2 => RemeshMode::Smooth,
                _ => RemeshMode::Voxel,
            },
            voxel_size: fl("voxel_size", 0.1),
            octree_depth: int("depth", 4).clamp(1, 12),
            scale: fl("scale", 0.9),
            smooth_shade: bit("flag", 1 << 1),
        },
        "ScrewModifierData" => Modifier::Screw {
            axis: int("axis", 2).min(2) as u8,
            angle: fl("angle", std::f32::consts::TAU),
            steps: int("steps", 16).clamp(1, 10_000),
            screw_offset: fl("screw_ofs", 0.0),
            iterations: int("iter", 1).clamp(1, 10_000),
            merge: bit("flag", 1 << 8),
            merge_distance: fl("merge_dist", 0.01),
            flip: bit("flag", 1),
            smooth_shade: bit("flag", 1 << 5),
        },
        "WireframeModifierData" => Modifier::Wireframe {
            thickness: fl("offset", 0.02),
            offset: fl("offset_fac", -1.0),
            even: bit("flag", 1 << 3),
            relative: bit("flag", 1 << 4),
            boundary: bit("flag", 1 << 2),
            replace: bit("flag", 1 << 1),
        },
        "EdgeSplitModifierData" => Modifier::EdgeSplit { angle: fl("split_angle", 0.5236), use_angle: bit("flags", 1 << 1) },
        "BuildModifierData" => Modifier::Build { start: fl("start", 1.0), length: fl("length", 100.0), reverse: bit("flag", 1 << 1), random: bit("flag", 1), seed: int("seed", 0) },
        "BooleanModifierData" => Modifier::Boolean {
            object: object_name("object"),
            operation: match flag("operation") {
                0 => BooleanOp::Intersect,
                1 => BooleanOp::Union,
                _ => BooleanOp::Difference,
            },
        },
        "SimpleDeformModifierData" => Modifier::SimpleDeform {
            mode: match flag("mode") {
                2 => SimpleDeformMode::Bend,
                3 => SimpleDeformMode::Taper,
                4 => SimpleDeformMode::Stretch,
                _ => SimpleDeformMode::Twist,
            },
            axis: int("deform_axis", 2).min(2) as u8,
            factor: fl("factor", 0.7854),
            limits: v.vec::<2>("limit").unwrap_or([0.0, 1.0]),
            lock: [bit("axis", 1), bit("axis", 2), bit("axis", 4)],
            vertex_group: None,
            invert_vertex_group: false,
        },
        "CastModifierData" => Modifier::Cast {
            kind: match flag("type") {
                1 => CastType::Cylinder,
                2 => CastType::Cuboid,
                _ => CastType::Sphere,
            },
            factor: fl("fac", 0.5),
            radius: fl("radius", 0.0),
            size: fl("size", 0.0),
            axes: [bit("flag", 1 << 1), bit("flag", 1 << 2), bit("flag", 1 << 3)],
            size_from_radius: bit("flag", 1 << 5),
            vertex_group: None,
            invert_vertex_group: false,
        },
        "WaveModifierData" => Modifier::Wave {
            x: bit("flag", 1 << 1),
            y: bit("flag", 1 << 2),
            cyclic: bit("flag", 1 << 3),
            normals: bit("flag", 1 << 4),
            falloff: fl("falloff", 0.0),
            height: fl("height", 0.5),
            width: fl("width", 1.5),
            narrowness: fl("narrow", 1.5),
            speed: fl("speed", 0.25),
            time_offset: fl("timeoffs", 0.0),
            lifetime: fl("lifetime", 0.0),
            damping: fl("damp", 10.0),
            start: [fl("startx", 0.0), fl("starty", 0.0)],
            vertex_group: None,
            invert_vertex_group: false,
        },
        "ShrinkwrapModifierData" => Modifier::Shrinkwrap {
            target: object_name("target"),
            method: match flag("shrinkType") {
                1 | 3 => ShrinkwrapMethod::Project,
                2 => ShrinkwrapMethod::NearestVertex,
                _ => ShrinkwrapMethod::NearestSurfacePoint,
            },
            offset: fl("keepDist", 0.0),
            axes: [bit("projAxis", 1), bit("projAxis", 2), bit("projAxis", 4)],
            positive: bit("shrinkOpts", 1),
            negative: bit("shrinkOpts", 2),
            vertex_group: None,
            invert_vertex_group: false,
        },
        "LaplacianSmoothModifierData" => Modifier::LaplacianSmooth {
            iterations: int("repeat", 1),
            factor: fl("lambda", 0.01),
            border: fl("lambda_border", 0.01),
            axes: [bit("flag", 1 << 1), bit("flag", 1 << 2), bit("flag", 1 << 3)],
            preserve_volume: bit("flag", 1 << 4),
            normalized: bit("flag", 1 << 5),
            vertex_group: None,
            invert_vertex_group: false,
        },
        "CorrectiveSmoothModifierData" => Modifier::CorrectiveSmooth {
            factor: fl("lambda", 0.5),
            iterations: int("repeat", 5),
            scale: fl("scale", 1.0),
            length_weighted: flag("smooth_type") == 1,
            only_smooth: bit("flag", 1 << 1),
            vertex_group: None,
            invert_vertex_group: false,
        },
        "WeightedNormalModifierData" => Modifier::WeightedNormal { keep_sharp: bit("flag", 1) },
        _ => return None,
    };
    // Keep counts in the ranges the UI and polyops accept.
    if let Modifier::Smooth { iterations, .. } | Modifier::LaplacianSmooth { iterations, .. } | Modifier::CorrectiveSmooth { iterations, .. } = &mut m {
        *iterations = (*iterations).min(10_000);
    }
    Some(m)
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
        LightKind::Area { .. } => {
            // Square 0, Rectangle 1, Disk 4, Ellipse 5.
            l.shape = match la.num("area_shape").unwrap_or(0.0) as i64 {
                1 => AreaShape::Rectangle,
                4 => AreaShape::Disk,
                5 => AreaShape::Ellipse,
                _ => AreaShape::Square,
            };
            l.size_y = la.f32("area_sizey").unwrap_or(l.size_y);
            l.spread = la.f32("area_spread").unwrap_or(l.spread);
        }
    }
    // LA_SHADOW.
    if let Some(m) = la.num("mode") {
        l.cast_shadow = m as i64 & 1 != 0;
    }
    l
}

#[allow(clippy::fn_params_excessive_bools)]
fn sex_vis(camera: bool, diffuse: bool, glossy: bool, transmission: bool, volume_scatter: bool, shadow: bool) -> RayVisibility {
    RayVisibility { camera, diffuse, glossy, transmission, volume_scatter, shadow }
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
    let mut mesh = Mesh { positions, faces, ..Default::default() };
    rig::mesh_extras(f, me, &vd, &mut mesh);
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
