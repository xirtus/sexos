// SPDX-License-Identifier: GPL-3.0-only
//! Scene files: glTF 2.0 (`.gltf` and `.glb`), Wavefront OBJ (+ MTL), STL and
//! PLY, plus the shared plumbing every format uses.
//!
//! The entry points are format-agnostic:
//! - [`import`] / [`import_into`] read one main file. Files it refers to (an
//!   OBJ's `.mtl`, a glTF's `.bin` buffers, textures) are fetched through a
//!   [`Resolver`] given the path exactly as written in the main file, so the
//!   caller decides how relative paths resolve (and tests need no disk).
//! - [`export`] writes the main file only; [`export_files`] also returns the
//!   companion files a format needs (an OBJ's `.mtl` and its textures).
//!
//! Axis conventions: the scene is Z-up, like Blender. OBJ and glTF are Y-up and
//! are converted with Blender's default mapping (file -Z forward / +Y up become
//! scene +Y / +Z, i.e. a +90° turn about X); STL and PLY are written and read
//! as-is, like Blender's importers do.
use crate::math::Vec3;
use crate::mesh::Mesh;
use crate::scene::{Image, ObjectData, Scene};
use std::collections::HashMap;
use std::fmt;

pub mod blend;
pub mod gltf;
pub mod obj;
pub mod ply;
pub mod stl;

/// A file format the suite can read or write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Obj,
    Stl,
    Ply,
    /// glTF 2.0 JSON; exported with every buffer and texture embedded as data URIs.
    Gltf,
    /// Binary glTF 2.0.
    Glb,
    /// Blender's native `.blend`.
    Blend,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Obj, Format::Stl, Format::Ply, Format::Gltf, Format::Glb, Format::Blend];

    /// Format for a file extension (no dot, any case).
    pub fn from_extension(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "obj" => Format::Obj,
            "stl" => Format::Stl,
            "ply" => Format::Ply,
            "gltf" => Format::Gltf,
            "glb" => Format::Glb,
            "blend" => Format::Blend,
            _ => return None,
        })
    }

    /// Format from a path's extension (`"models/Box.glb"` -> [`Format::Glb`]).
    pub fn from_path(path: &str) -> Option<Format> {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let (_, ext) = name.rsplit_once('.')?;
        Format::from_extension(ext)
    }

    /// Guess the format from the first bytes of a file, for files with a
    /// missing or wrong extension. OBJ has no signature and is never guessed.
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        if bytes.starts_with(b"glTF") {
            return Some(Format::Glb);
        }
        if bytes.starts_with(b"BLENDER") || bytes.starts_with(&[0x1f, 0x8b]) || bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            // Compressed .blend files are gzip (old) or zstd (4.x).
            return Some(Format::Blend);
        }
        if bytes.starts_with(b"ply") {
            return Some(Format::Ply);
        }
        if stl::is_binary(bytes) || trim_ascii_start(bytes).starts_with(b"solid") {
            return Some(Format::Stl);
        }
        let t = trim_ascii_start(bytes);
        if t.starts_with(b"{") && t.windows(7).take(4096).any(|w| w == b"\"asset\"") {
            return Some(Format::Gltf);
        }
        None
    }

    /// Lower-case extension without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Obj => "obj",
            Format::Stl => "stl",
            Format::Ply => "ply",
            Format::Gltf => "gltf",
            Format::Glb => "glb",
            Format::Blend => "blend",
        }
    }

    /// Display name used in menus and error messages.
    pub fn name(self) -> &'static str {
        match self {
            Format::Obj => "OBJ",
            Format::Stl => "STL",
            Format::Ply => "PLY",
            Format::Gltf => "glTF",
            Format::Glb => "GLB",
            Format::Blend => ".blend",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a file could not be read or written.
#[derive(Clone, Debug, PartialEq)]
pub enum IoError {
    /// The data is malformed. `line` is 1-based and set for text formats.
    Parse { format: Format, line: Option<usize>, message: String },
    /// A file the main file needs could not be loaded through the resolver.
    MissingFile { format: Format, path: String },
    /// Well-formed data using a feature this reader or writer lacks.
    Unsupported { format: Format, message: String },
}

impl IoError {
    pub(crate) fn parse(format: Format, line: Option<usize>, message: impl Into<String>) -> IoError {
        IoError::Parse { format, line, message: message.into() }
    }
    pub(crate) fn at_line(format: Format, line: usize, message: impl Into<String>) -> IoError {
        IoError::Parse { format, line: Some(line), message: message.into() }
    }
    /// The 1-based line of a text-format parse error.
    pub fn line(&self) -> Option<usize> {
        match self {
            IoError::Parse { line, .. } => *line,
            _ => None,
        }
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::Parse { format, line: Some(l), message } => write!(f, "{format} line {l}: {message}"),
            IoError::Parse { format, line: None, message } => write!(f, "{format}: {message}"),
            IoError::MissingFile { format, path } => write!(f, "{format}: cannot load referenced file \"{path}\""),
            IoError::Unsupported { format, message } => write!(f, "{format}: unsupported: {message}"),
        }
    }
}

impl std::error::Error for IoError {}

/// Loads a file referenced by the main file (path as written in it, e.g.
/// `"textures/wood.png"`), relative to the main file's folder.
pub type Resolver<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// A resolver that finds nothing: for self-contained files.
pub fn no_files(_: &str) -> Option<Vec<u8>> {
    None
}

/// Read a file into a new scene. The first imported camera becomes the active
/// camera and the first imported object the active one.
pub fn import(bytes: &[u8], format: Format, resolver: Resolver) -> Result<Scene, IoError> {
    let mut scene = Scene::new();
    let added = import_into(&mut scene, bytes, format, resolver)?;
    scene.active_camera = added.iter().copied().find(|&i| matches!(scene.objects[i].data, ObjectData::Camera(_)));
    scene.active_object = added.first().copied();
    Ok(scene)
}

/// Append a file's objects (with their meshes, materials and images) to
/// `scene`, in its first collection. Returns the new objects' indices in file
/// order (glTF: node order; parents are set by index, not by position).
pub fn import_into(scene: &mut Scene, bytes: &[u8], format: Format, resolver: Resolver) -> Result<Vec<usize>, IoError> {
    match format {
        Format::Obj => obj::import_into(scene, bytes, resolver),
        Format::Stl => stl::import_into(scene, bytes),
        Format::Ply => ply::import_into(scene, bytes),
        Format::Gltf | Format::Glb => gltf::import_into(scene, bytes, resolver),
        Format::Blend => {
            let other = blend::import_blend(bytes).map_err(|message| IoError::Parse { format, line: None, message })?;
            Ok(append_scene(scene, other))
        }
    }
}

/// Move every object of `other` (with its meshes, materials and images) into
/// `scene`'s first collection, fixing up indices. Returns the new object indices.
pub fn append_scene(scene: &mut Scene, other: Scene) -> Vec<usize> {
    let img_off = scene.images.len();
    scene.images.extend(other.images);
    let mat_off = scene.materials.len();
    for mut m in other.materials {
        m.base_color_texture = m.base_color_texture.map(|i| i + img_off);
        scene.materials.push(m);
    }
    let mesh_ids: Vec<usize> = other.meshes.into_iter().zip(other.mesh_names).map(|(m, n)| scene.add_mesh(&n, m)).collect();
    let obj_off = scene.objects.len();
    let mut out = Vec::new();
    for mut o in other.objects {
        if let ObjectData::Mesh(mi) = &mut o.data {
            *mi = mesh_ids[*mi];
        }
        for slot in &mut o.materials {
            if *slot != usize::MAX {
                *slot += mat_off;
            }
        }
        o.parent = o.parent.map(|p| p + obj_off);
        o.collection = 0;
        out.push(scene.add_object(o));
    }
    if scene.active_camera.is_none() {
        scene.active_camera = other.active_camera.map(|c| c + obj_off);
    }
    out
}

/// Write `scene` as one file. OBJ written this way references no `.mtl`
/// (faces still name their materials); use [`export_files`] to get it.
/// STL is written binary and PLY binary little-endian; see
/// [`stl::export_ascii`] and [`ply::export_ascii`] for the text variants.
pub fn export(scene: &Scene, format: Format) -> Result<Vec<u8>, IoError> {
    match format {
        Format::Obj => Ok(obj::export(scene, None)),
        Format::Stl => Ok(stl::export_binary(scene)),
        Format::Ply => Ok(ply::export_binary(scene)),
        Format::Gltf => gltf::export(scene, false),
        Format::Glb => gltf::export(scene, true),
        Format::Blend => Err(IoError::Unsupported { format, message: "no .blend writer in this build".into() }),
    }
}

/// Write `scene` as `stem.<ext>` plus its companion files, main file first:
/// for OBJ that is `stem.mtl` and one PNG per texture used by a material.
pub fn export_files(scene: &Scene, format: Format, stem: &str) -> Result<Vec<(String, Vec<u8>)>, IoError> {
    match format {
        Format::Obj => Ok(obj::export_files(scene, stem)),
        _ => Ok(vec![(format!("{stem}.{}", format.extension()), export(scene, format)?)]),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers.

/// Y-up file space (OBJ, glTF) to the scene's Z-up space: +90° about X.
pub(crate) fn y_up_to_z_up(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}

/// Inverse of [`y_up_to_z_up`].
pub(crate) fn z_up_to_y_up(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// Merge vertices with bit-identical positions (treating -0 as 0), keeping
/// per-corner UVs on the faces. Linear time. Returns how many were removed.
///
/// Formats that store triangle soups or split vertices along UV and normal
/// seams (STL, glTF) need this so the half-edge mesh is connected.
pub(crate) fn weld_exact(m: &mut Mesh) -> usize {
    let key = |p: Vec3| [p.x + 0.0, p.y + 0.0, p.z + 0.0].map(f32::to_bits);
    let mut first: HashMap<[u32; 3], u32> = HashMap::with_capacity(m.positions.len());
    let mut remap = Vec::with_capacity(m.positions.len());
    let mut kept = Vec::with_capacity(m.positions.len());
    for &p in &m.positions {
        let next = kept.len() as u32;
        let j = *first.entry(key(p)).or_insert(next);
        if j == next {
            kept.push(p);
        }
        remap.push(j);
    }
    let removed = m.positions.len() - kept.len();
    if removed > 0 {
        m.positions = kept;
        m.remap_vertices(&remap);
    }
    removed
}

/// A mesh object flattened into world space, as OBJ/STL/PLY export it.
pub(crate) struct WorldMesh {
    pub name: String,
    pub mesh: Mesh,
    /// The object's material slots (scene material indices).
    pub slots: Vec<usize>,
}

/// Every mesh object's evaluated mesh in world space, in object order.
pub(crate) fn world_meshes(scene: &Scene) -> Vec<WorldMesh> {
    (0..scene.objects.len())
        .filter_map(|i| scene.world_mesh(i).map(|(mesh, slots)| WorldMesh { name: scene.objects[i].name.clone(), mesh, slots }))
        .collect()
}

/// Bytes after leading ASCII whitespace.
pub(crate) fn trim_ascii_start(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

/// Parse a float token of a text format, with a line-numbered error.
pub(crate) fn parse_f32(tok: &str, format: Format, line: usize) -> Result<f32, IoError> {
    tok.parse::<f32>().map_err(|_| IoError::at_line(format, line, format!("invalid number \"{tok}\"")))
}

/// Read up to `N` floats from `toks`; missing ones take `default`.
pub(crate) fn parse_floats<'a, const N: usize>(toks: impl Iterator<Item = &'a str>, default: f32, format: Format, line: usize) -> Result<[f32; N], IoError> {
    let mut out = [default; N];
    for (o, t) in out.iter_mut().zip(toks) {
        *o = parse_f32(t, format, line)?;
    }
    Ok(out)
}

/// sRGB transfer function, encoded 0..1 to linear 0..1.
pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear 0..1 to sRGB-encoded 0..1.
pub(crate) fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Decode a PNG or JPEG (detected by signature) into a linear-light image.
/// Colour channels are treated as sRGB, alpha as linear.
pub(crate) fn decode_image(name: &str, bytes: &[u8]) -> Result<Image, String> {
    let (w, h, channels, data) = if bytes.starts_with(b"\x89PNG") {
        decode_png(bytes)?
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        decode_jpeg(bytes)?
    } else {
        return Err("not a PNG or JPEG image".into());
    };
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let px = |c: &[u8]| -> [f32; 4] {
        match channels {
            1 => {
                let l = lut[c[0] as usize];
                [l, l, l, 1.0]
            }
            2 => {
                let l = lut[c[0] as usize];
                [l, l, l, c[1] as f32 / 255.0]
            }
            3 => [lut[c[0] as usize], lut[c[1] as usize], lut[c[2] as usize], 1.0],
            _ => [lut[c[0] as usize], lut[c[1] as usize], lut[c[2] as usize], c[3] as f32 / 255.0],
        }
    };
    let pixels: Vec<[f32; 4]> = data.chunks_exact(channels).take(w as usize * h as usize).map(px).collect();
    if pixels.len() != w as usize * h as usize {
        return Err("image data is truncated".into());
    }
    Ok(Image { name: name.to_string(), width: w, height: h, pixels })
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, usize, Vec<u8>), String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| format!("PNG: {e}"))?;
    let size = reader.output_buffer_size().ok_or("PNG: image too large")?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("PNG: {e}"))?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("PNG: palette was not expanded".into()),
    };
    // Rows may be padded; repack to tightly packed pixels.
    let row = info.width as usize * channels;
    let mut data = Vec::with_capacity(row * info.height as usize);
    for r in buf.chunks(info.line_size).take(info.height as usize) {
        data.extend_from_slice(&r[..row.min(r.len())]);
    }
    Ok((info.width, info.height, channels, data))
}

fn decode_jpeg(bytes: &[u8]) -> Result<(u32, u32, usize, Vec<u8>), String> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), opts);
    let data = dec.decode().map_err(|e| format!("JPEG: {e:?}"))?;
    let (w, h) = dec.dimensions().ok_or("JPEG: no image header")?;
    let channels = dec.output_colorspace().map_or(3, |c| c.num_components());
    if !(1..=4).contains(&channels) {
        return Err("JPEG: unsupported colour space".into());
    }
    Ok((w as u32, h as u32, channels, data))
}

/// Encode a linear-light image as an 8-bit sRGB RGBA PNG.
pub(crate) fn encode_png(img: &Image) -> Vec<u8> {
    let mut data = Vec::with_capacity(img.pixels.len() * 4);
    for p in &img.pixels {
        let e = |c: f32| (linear_to_srgb(c) * 255.0 + 0.5) as u8;
        data.extend_from_slice(&[e(p[0]), e(p[1]), e(p[2]), (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
    }
    let mut out = Vec::new();
    let (w, h) = if img.pixels.len() == img.width as usize * img.height as usize && img.width > 0 && img.height > 0 {
        (img.width, img.height)
    } else {
        // A malformed image still yields a valid file: one magenta pixel.
        data = vec![255, 0, 255, 255];
        (1, 1)
    };
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    // Writing into a Vec cannot fail for valid dimensions.
    if let Ok(mut wr) = enc.write_header() {
        let _ = wr.write_image_data(&data);
        let _ = wr.finish();
    }
    out
}
