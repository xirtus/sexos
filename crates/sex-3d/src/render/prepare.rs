// SPDX-License-Identifier: GPL-3.0-only
//! Scene preparation: flatten visible mesh objects into a world-space triangle
//! BVH with shading data (objects moving during the shutter get their own
//! BVH placed at each ray's time), resolve materials and their media, record
//! per-object ray visibility, and build the light list (lights, emissive
//! triangles, world) with its power-based selection distribution.
use super::bvh::{Bvh, Hit, TriGeom};
use super::luminance;
use super::rng::Rng;
use super::sampling::{concentric_disk, onb, uniform_cone, uniform_sphere, Distribution1D};
use crate::math::{Aabb, Mat4, Vec2, Vec3};
use super::camera::{CameraRig, MotionChain};
use super::volume::VolumeCoeffs;
use crate::scene::{AreaShape, Camera, Image, LightKind, Material, ObjectData, RayVisibility, Scene};
use super::shader::surface::{self as shading, ShadeData, WorldShader};
use super::shader::{compile_material, compile_world, Program, RayState};
use core::f32::consts::{FRAC_1_PI, PI, TAU};
use std::sync::Arc;

/// Per-triangle shading data (in BVH order).
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriShade {
    pub n: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub material: u32,
    /// Index into the light list for emissive triangles, else `u32::MAX`.
    pub light: u32,
    pub object: u32,
    /// Corner colors of the mesh's render color attribute (white without one).
    pub col: [[f32; 4]; 3],
}

/// A material reduced to what the BSDF needs.
#[derive(Clone, Debug)]
pub(crate) struct ShadeMaterial {
    pub base: Vec3,
    pub texture: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    pub ior: f32,
    pub transmission: f32,
    /// Emitted radiance.
    pub emission: Vec3,
    /// The compiled node tree, when the material uses nodes.
    pub program: Option<Arc<Program>>,
}

impl ShadeMaterial {
    fn from(m: &Material, images: usize) -> ShadeMaterial {
        let c = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let e = m.emission * m.emission_strength;
        ShadeMaterial {
            base: Vec3::new(c(m.base_color[0]), c(m.base_color[1]), c(m.base_color[2])),
            texture: m.base_color_texture.filter(|&t| t < images),
            metallic: c(m.metallic).min(1.0),
            roughness: c(m.roughness).min(1.0),
            ior: if m.ior.is_finite() && m.ior > 0.0 { m.ior } else { 1.5 },
            transmission: c(m.transmission).min(1.0),
            emission: Vec3::new(c(e.x), c(e.y), c(e.z)),
            program: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum LightShape {
    /// Point or spot light; `spot` = (axis, cos of half angle, blend width in cos).
    Point { pos: Vec3, radius: f32, intensity: Vec3, spot: Option<(Vec3, f32, f32)> },
    Sun { to_light: Vec3, cos_max: f32, irradiance: Vec3 },
    /// One-sided parallelogram `center ± u ± v`, emitting along `normal`.
    /// `ellipse`: the ellipse inscribed in it instead. `spread`: see [`spread_params`].
    Area { center: Vec3, u: Vec3, v: Vec3, normal: Vec3, area: f32, radiance: Vec3, ellipse: bool, spread: Option<(f32, f32)> },
    /// Emissive triangle (two-sided), by BVH triangle index.
    Tri { prim: u32, area: f32, radiance: Vec3 },
    World,
}

/// A light sample seen from a shading point.
pub(crate) struct LightSample {
    pub wi: Vec3,
    /// Distance to the sampled point (infinite for sun and world).
    pub dist: f32,
    /// Incident radiance (intensity / d² for delta lights).
    pub li: Vec3,
    /// Solid-angle pdf including the light selection probability.
    pub pdf: f32,
    /// Delta light: cannot be hit by BSDF sampling, so no MIS.
    pub delta: bool,
}

/// The environment: uniform colour or equirectangular HDRI.
#[derive(Clone, Debug)]
pub(crate) struct Environment {
    pub color: Vec3,
    pub hdri: Option<Image>,
    pub strength: f32,
    /// Importance sampling over a (possibly downsampled) grid.
    dist: Option<EnvDistribution>,
    /// The world node tree, replacing the fields above.
    shader: Option<WorldShader>,
    /// Average luminance of the world shader.
    shader_avg: f32,
}

#[derive(Clone, Debug)]
struct EnvDistribution {
    w: usize,
    h: usize,
    rows: Distribution1D,
    cols: Vec<Option<Distribution1D>>,
}

/// Blender's equirectangular mapping: direction to (u, v), v = 0 at the bottom.
pub(crate) fn dir_to_equirect(d: Vec3) -> (f32, f32) {
    let u = -d.y.atan2(d.x) / TAU + 0.5;
    let v = d.z.atan2((d.x * d.x + d.y * d.y).sqrt()) / PI + 0.5;
    (u, v)
}

fn equirect_to_dir(u: f32, v: f32) -> Vec3 {
    let phi = (0.5 - u) * TAU;
    let el = (v - 0.5) * PI;
    Vec3::new(phi.cos() * el.cos(), phi.sin() * el.cos(), el.sin())
}

impl Environment {
    fn new(scene: &Scene, data: Option<&Arc<ShadeData>>) -> Environment {
        let w = &scene.world;
        let strength = if w.strength.is_finite() { w.strength.max(0.0) } else { 0.0 };
        let program = w.node_tree.as_ref().and_then(|t| compile_world(t, &scene.node_groups));
        if let (Some(program), Some(data)) = (program, data) {
            let shader = WorldShader { program: Arc::new(program), data: data.clone() };
            let (gw, gh, grid) = shader.grid();
            let shader_avg = shading::grid_average(gw, gh, &grid);
            let dist = if shader.constant() { None } else { shading::grid_rows(gw, gh, &grid).map(|(rows, cols)| EnvDistribution { w: gw, h: gh, rows, cols }) };
            let color = shader.eval(Vec3::Z, RayState::camera());
            return Environment { color, hdri: None, strength: 1.0, dist, shader: Some(shader), shader_avg };
        }
        let hdri = w.hdri.and_then(|i| scene.images.get(i)).filter(|im| im.width > 0 && im.height > 0 && im.pixels.len() == (im.width * im.height) as usize).cloned();
        let dist = hdri.as_ref().and_then(EnvDistribution::new);
        Environment { color: w.color * strength, hdri, strength, dist, shader: None, shader_avg: 0.0 }
    }

    /// Radiance towards `d` for a ray of type `ray` (the world tree's Light
    /// Path node sees it).
    pub fn eval_ray(&self, d: Vec3, ray: RayState) -> Vec3 {
        match &self.shader {
            Some(s) => s.eval(d, ray),
            None => self.eval(d),
        }
    }

    pub fn eval(&self, d: Vec3) -> Vec3 {
        if let Some(s) = &self.shader {
            return s.eval(d, RayState::default());
        }
        match &self.hdri {
            Some(img) => {
                let (u, v) = dir_to_equirect(d);
                let p = img.sample(u, v);
                Vec3::new(p[0], p[1], p[2]) * self.strength
            }
            None => self.color,
        }
    }

    /// Rough average radiance, for light selection.
    fn average(&self) -> f32 {
        if self.shader.is_some() {
            return self.shader_avg;
        }
        match &self.hdri {
            Some(img) => {
                // Solid-angle weighted average over rows.
                let (w, h) = (img.width as usize, img.height as usize);
                let mut sum = 0.0f64;
                let mut wsum = 0.0f64;
                for y in 0..h {
                    let s = ((y as f32 + 0.5) / h as f32 * PI).sin() as f64;
                    let row: f64 = img.pixels[y * w..(y + 1) * w].iter().map(|p| luminance(Vec3::new(p[0], p[1], p[2])).max(0.0) as f64).sum();
                    sum += row * s;
                    wsum += w as f64 * s;
                }
                (sum / wsum.max(1e-30)) as f32 * self.strength
            }
            None => luminance(self.color),
        }
    }

    fn sample(&self, u1: f32, u2: f32) -> (Vec3, f32) {
        match &self.dist {
            Some(d) => {
                let (v, pdf_v, row) = d.rows.sample_continuous(u1);
                let Some(cols) = &d.cols[row] else { return (Vec3::Z, 0.0) };
                let (u, pdf_u, _) = cols.sample_continuous(u2);
                // Grid row 0 is the top of the image (v = 1).
                let (uu, vv) = (u, 1.0 - v);
                let sin_theta = (v * PI).sin();
                let pdf = if sin_theta > 0.0 { pdf_v * pdf_u / (2.0 * PI * PI * sin_theta) } else { 0.0 };
                (equirect_to_dir(uu, vv), pdf)
            }
            None => (uniform_sphere(u1, u2), 0.25 * FRAC_1_PI),
        }
    }

    fn pdf(&self, dir: Vec3) -> f32 {
        match &self.dist {
            Some(d) => {
                let (u, v) = dir_to_equirect(dir);
                let gy = ((1.0 - v) * d.h as f32).clamp(0.0, d.h as f32 - 1.0) as usize;
                let gx = (u * d.w as f32).clamp(0.0, d.w as f32 - 1.0) as usize;
                let Some(cols) = &d.cols[gy] else { return 0.0 };
                let sin_theta = ((1.0 - v) * PI).sin();
                if sin_theta <= 0.0 {
                    return 0.0;
                }
                d.rows.density(gy) * cols.density(gx) / (2.0 * PI * PI * sin_theta)
            }
            None => 0.25 * FRAC_1_PI,
        }
    }
}

impl EnvDistribution {
    fn new(img: &Image) -> Option<EnvDistribution> {
        let (iw, ih) = (img.width as usize, img.height as usize);
        let (w, h) = (iw.min(512), ih.min(256));
        let mut grid = vec![0.0f32; w * h];
        for y in 0..ih {
            let gy = y * h / ih;
            for x in 0..iw {
                let p = img.pixels[y * iw + x];
                grid[gy * w + x * w / iw] += luminance(Vec3::new(p[0], p[1], p[2])).max(0.0);
            }
        }
        let mut row_w = Vec::with_capacity(h);
        let mut cols = Vec::with_capacity(h);
        for gy in 0..h {
            let sin_theta = ((gy as f32 + 0.5) / h as f32 * PI).sin();
            // A small floor keeps every direction samplable (bilinear lookups
            // leak into dark cells).
            let row: Vec<f32> = grid[gy * w..(gy + 1) * w].iter().map(|&l| (l + 1e-3) * sin_theta).collect();
            let d = Distribution1D::new(&row);
            row_w.push(d.as_ref().map_or(0.0, |d| d.total()));
            cols.push(d);
        }
        Some(EnvDistribution { w, h, rows: Distribution1D::new(&row_w)?, cols })
    }
}

/// Ray-type bits for [`crate::scene::RayVisibility`].
pub(crate) const VIS_CAMERA: u8 = 1;
pub(crate) const VIS_DIFFUSE: u8 = 2;
pub(crate) const VIS_GLOSSY: u8 = 4;
pub(crate) const VIS_TRANSMISSION: u8 = 8;
pub(crate) const VIS_VOLUME: u8 = 16;
pub(crate) const VIS_SHADOW: u8 = 32;
pub(crate) const VIS_ALL: u8 = 63;

fn vis_mask(v: &RayVisibility) -> u8 {
    let b = |on: bool, bit: u8| if on { bit } else { 0 };
    b(v.camera, VIS_CAMERA) | b(v.diffuse, VIS_DIFFUSE) | b(v.glossy, VIS_GLOSSY) | b(v.transmission, VIS_TRANSMISSION) | b(v.volume_scatter, VIS_VOLUME) | b(v.shadow, VIS_SHADOW)
}

/// Per-light settings beyond its shape.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LightInfo {
    /// Scene object index (light object, or the emissive mesh).
    pub object: u32,
    pub max_bounces: u32,
    pub cast_shadow: bool,
    pub use_mis: bool,
}

/// A mesh object that moves during the shutter: its own BVH in object space,
/// placed by its transform chain at the ray's time.
#[derive(Clone, Debug)]
pub(crate) struct MotionInstance {
    bvh: Bvh,
    /// Global index of its first triangle in [`PreparedScene::shade`].
    base: u32,
    chain: MotionChain,
}

/// The media a path is inside (material indices), innermost last.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MediumStack {
    ids: [u32; 4],
    n: u8,
}

impl MediumStack {
    /// Cross the boundary of material `m`'s medium: enter when `entering`.
    pub fn cross(&mut self, m: u32, entering: bool) {
        if entering {
            if (self.n as usize) < self.ids.len() {
                self.ids[self.n as usize] = m;
                self.n += 1;
            }
        } else if let Some(i) = self.ids[..self.n as usize].iter().rposition(|&x| x == m) {
            self.ids.copy_within(i + 1..self.n as usize, i);
            self.n -= 1;
        }
    }
}

/// A scene flattened for path tracing. Build once, render many times (the
/// progressive viewport and the render farm reuse it). Owns all its data.
#[derive(Clone, Debug)]
pub struct PreparedScene {
    pub(crate) bvh: Bvh,
    /// Shading data of every triangle: the static BVH's, then each motion instance's.
    pub(crate) shade: Vec<TriShade>,
    pub(crate) materials: Vec<ShadeMaterial>,
    /// Homogeneous medium of each material (same indices as `materials`).
    pub(crate) material_volume: Vec<Option<VolumeCoeffs>>,
    pub(crate) material_pass_index: Vec<u32>,
    pub(crate) images: Vec<Image>,
    pub(crate) lights: Vec<LightShape>,
    pub(crate) light_info: Vec<LightInfo>,
    pub(crate) light_dist: Option<Distribution1D>,
    /// Indices of [`LightShape::Area`] lights, which rays can hit.
    pub(crate) area_lights: Vec<u32>,
    pub(crate) world_light: Option<u32>,
    pub(crate) env: Environment,
    pub(crate) world_volume: Option<VolumeCoeffs>,
    pub(crate) camera: CameraRig,
    /// Per scene object: ray visibility bits, holdout, pass index.
    pub(crate) object_vis: Vec<u8>,
    pub(crate) object_holdout: Vec<bool>,
    pub(crate) object_pass_index: Vec<u32>,
    /// Some object hides from some ray type (rays must skip it).
    pub(crate) any_hidden: bool,
    /// Some material has a medium (rays must track volumes).
    pub(crate) any_volume: bool,
    /// Some node material lets light through (shadow rays shade them).
    pub(crate) any_transparent: bool,
    /// What node programs read (present when any material or the world
    /// uses nodes).
    pub(crate) shade_data: Option<Arc<ShadeData>>,
    instances: Vec<MotionInstance>,
    bounds: Aabb,
}

/// What a ray hit.
pub(crate) enum SceneHit {
    Tri(Hit),
    /// An area light (light index) at distance t.
    Area(u32, f32),
}

/// Triangles of one object in world space at time 0, with shading data.
fn object_triangles(scene: &Scene, oi: usize, mesh_to: Option<&Mat4>, default_mat: u32) -> (Vec<TriGeom>, Vec<TriShade>) {
    let mut geoms = Vec::new();
    let mut shades = Vec::new();
    let Some(mut mesh) = scene.evaluated_mesh(oi) else { return (geoms, shades) };
    if let Some(m) = mesh_to {
        mesh.transform(m);
    }
    let slots = &scene.objects[oi].materials;
    let smooth = mesh.faces.iter().any(|f| f.smooth);
    let vn = if smooth { mesh.vertex_normals() } else { Vec::new() };
    let offsets = if mesh.color_attributes.is_empty() { Vec::new() } else { mesh.corner_offsets() };
    for t in mesh.triangulate() {
        let face = &mesh.faces[t.face as usize];
        let p = t.verts.map(|v| mesh.positions[v as usize]);
        let g = TriGeom::new(p[0], p[1], p[2]);
        let ng = g.e1.cross(g.e2).normalize();
        if ng == Vec3::ZERO || !(p[0].x.is_finite() && p[1].x.is_finite() && p[2].x.is_finite()) {
            continue; // degenerate
        }
        let n = if face.smooth { t.verts.map(|v| if vn[v as usize] == Vec3::ZERO { ng } else { vn[v as usize] }) } else { [ng; 3] };
        let uv = if face.uvs.len() == face.verts.len() { t.corners.map(|c| face.uvs[c as usize]) } else { [Vec2::ZERO; 3] };
        let material = slots.get(face.material as usize).copied().filter(|&m| m < scene.materials.len()).map_or(default_mat, |m| m as u32);
        geoms.push(g);
        let col = if offsets.is_empty() { [[1.0; 4]; 3] } else { t.corners.map(|c| mesh.corner_color(mesh.render_color, &offsets, t.face as usize, c as usize)) };
        shades.push(TriShade { n, uv, material, light: u32::MAX, object: oi as u32, col });
    }
    (geoms, shades)
}

impl PreparedScene {
    /// Flatten `scene` for rendering from its active camera.
    pub fn new(scene: &Scene) -> PreparedScene {
        let mut materials: Vec<ShadeMaterial> = scene.materials.iter().map(|m| ShadeMaterial::from(m, scene.images.len())).collect();
        // Node trees replace the legacy fields.
        for (sm, m) in materials.iter_mut().zip(&scene.materials) {
            if let Some(t) = &m.node_tree {
                sm.program = compile_material(t, &scene.node_groups).map(Arc::new);
                if sm.program.is_some() {
                    sm.emission = Vec3::ZERO;
                }
            }
        }
        let default_mat = materials.len() as u32;
        materials.push(ShadeMaterial::from(&Material::default(), 0));
        let any_nodes = materials.iter().any(|m| m.program.is_some()) || scene.world.node_tree.is_some();
        let mut shade_data = any_nodes.then(|| ShadeData::new(scene, true));
        let mut material_volume: Vec<Option<VolumeCoeffs>> = scene
            .materials
            .iter()
            .zip(&materials)
            .map(|(m, sm)| match (&sm.program, &shade_data) {
                (Some(p), Some(d)) => shading::volume_coeffs(p, d),
                _ => m.volume.as_ref().map(VolumeCoeffs::from_settings),
            })
            .collect();
        material_volume.push(None);
        let mut material_pass_index: Vec<u32> = scene.materials.iter().map(|m| m.pass_index).collect();
        material_pass_index.push(0);
        let any_volume = material_volume.iter().any(Option::is_some);

        let object_vis: Vec<u8> = scene.objects.iter().map(|o| vis_mask(&o.visibility)).collect();
        let object_holdout: Vec<bool> = scene.objects.iter().map(|o| o.holdout).collect();
        let object_pass_index: Vec<u32> = scene.objects.iter().map(|o| o.pass_index).collect();
        let any_hidden = scene.objects.iter().enumerate().any(|(i, o)| scene.is_visible(i) && (object_vis[i] != VIS_ALL || o.holdout));

        let mut geoms = Vec::new();
        let mut shades = Vec::new();
        let mut moving = Vec::new();
        for oi in 0..scene.objects.len() {
            if !scene.is_visible(oi) || !matches!(scene.objects[oi].data, ObjectData::Mesh(_)) {
                continue;
            }
            if scene.has_motion(oi) {
                moving.push(oi);
                continue;
            }
            let (g, s) = object_triangles(scene, oi, Some(&scene.world_matrix(oi)), default_mat);
            geoms.extend(g);
            shades.extend(s);
        }
        let (bvh, order) = Bvh::build(geoms);
        let mut shade: Vec<TriShade> = order.iter().map(|&i| shades[i as usize]).collect();
        let mut instances = Vec::new();
        let mut bounds = bvh.bounds();
        for oi in moving {
            let (g, s) = object_triangles(scene, oi, None, default_mat);
            if g.is_empty() {
                continue;
            }
            let (ib, iorder) = Bvh::build(g);
            let base = shade.len() as u32;
            shade.extend(iorder.iter().map(|&i| s[i as usize]));
            let chain = MotionChain::from_scene(scene, oi);
            let lb = ib.bounds();
            for t in [0.0, 1.0] {
                let m = chain.matrix(t);
                for c in 0..8 {
                    let p = Vec3::new(if c & 1 == 0 { lb.min.x } else { lb.max.x }, if c & 2 == 0 { lb.min.y } else { lb.max.y }, if c & 4 == 0 { lb.min.z } else { lb.max.z });
                    bounds.grow(m.transform_point(p));
                }
            }
            instances.push(MotionInstance { bvh: ib, base, chain });
        }
        let radius = if bounds.is_empty() { 1.0 } else { (bounds.size().length() * 0.5).max(1e-3) };

        let camera = match scene.active_camera.filter(|&c| c < scene.objects.len()) {
            Some(ci) => {
                let cam = match &scene.objects[ci].data {
                    ObjectData::Camera(c) => *c,
                    _ => Camera::default(),
                };
                let focus = cam.dof.focus_object.filter(|&f| f < scene.objects.len()).map(|f| scene.world_matrix(f).transform_point(Vec3::ZERO));
                let motion = scene.has_motion(ci).then(|| MotionChain::from_scene(scene, ci));
                CameraRig::new(&scene.world_matrix(ci), &cam, focus, motion)
            }
            None => CameraRig::new(&Mat4::look_at(Vec3::new(7.36, -6.93, 4.96), Vec3::ZERO, Vec3::Z).inverse(), &Camera::default(), None, None),
        };

        // Lights and their selection weights (estimated emitted power).
        let mut lights = Vec::new();
        let mut light_info = Vec::new();
        let mut power = Vec::new();
        for oi in 0..scene.objects.len() {
            let ObjectData::Light(l) = &scene.objects[oi].data else { continue };
            if !scene.is_visible(oi) || !(l.power > 0.0) || !l.power.is_finite() {
                continue;
            }
            let m = scene.world_matrix(oi);
            let pos = m.transform_point(Vec3::ZERO);
            let axis = m.transform_vector(-Vec3::Z).normalize();
            let color = l.color.max(Vec3::ZERO);
            let lum = luminance(color);
            let info = LightInfo { object: oi as u32, max_bounces: l.max_bounces, cast_shadow: l.cast_shadow, use_mis: l.use_mis };
            match l.kind {
                LightKind::Point | LightKind::Spot { .. } => {
                    let intensity = color * (l.power / (4.0 * PI));
                    let spot = match l.kind {
                        LightKind::Spot { angle, blend } => {
                            let cos_half = (angle.clamp(1e-4, PI) * 0.5).cos();
                            Some((axis, cos_half, (1.0 - cos_half) * blend.clamp(0.0, 1.0)))
                        }
                        _ => None,
                    };
                    lights.push(LightShape::Point { pos, radius: l.radius.max(0.0), intensity, spot });
                    power.push(l.power * lum);
                }
                LightKind::Sun => {
                    let half = (l.radius.clamp(0.0, PI) * 0.5).min(PI * 0.5);
                    lights.push(LightShape::Sun { to_light: -axis, cos_max: half.cos(), irradiance: color * l.power });
                    power.push(l.power * lum * PI * radius * radius);
                }
                LightKind::Area { .. } => {
                    let (sx, sy) = l.area_size();
                    let u = m.transform_vector(Vec3::X) * (sx * 0.5);
                    let v = m.transform_vector(Vec3::Y) * (sy * 0.5);
                    let ellipse = matches!(l.shape, AreaShape::Disk | AreaShape::Ellipse);
                    let area = u.cross(v).length() * if ellipse { PI } else { 4.0 };
                    if !(area > 0.0) || !area.is_finite() {
                        continue;
                    }
                    let spread = spread_params(l.spread);
                    lights.push(LightShape::Area { center: pos, u, v, normal: axis, area, radiance: color * (l.power / (area * PI)), ellipse, spread });
                    power.push(l.power * lum);
                }
            }
            light_info.push(info);
        }
        for (prim, s) in shade.iter_mut().enumerate().take(bvh.tris.len()) {
            let mat = &materials[s.material as usize];
            let e = match (&mat.program, &shade_data) {
                (Some(p), Some(d)) => shading::emission_estimate(d, p, &bvh.tris[prim], s, material_pass_index.get(s.material as usize).copied().unwrap_or(0) as f32),
                _ => mat.emission,
            };
            if e == Vec3::ZERO {
                continue;
            }
            let g = &bvh.tris[prim];
            let area = 0.5 * g.e1.cross(g.e2).length();
            s.light = lights.len() as u32;
            lights.push(LightShape::Tri { prim: prim as u32, area, radiance: e });
            light_info.push(LightInfo { object: s.object, max_bounces: u32::MAX, cast_shadow: true, use_mis: true });
            // Two-sided emitter: power = L * A * pi per side.
            power.push(luminance(e) * area * 2.0 * PI);
        }
        // Node programs read the camera (Texture Coordinate, Camera Data).
        if let Some(d) = shade_data.as_mut() {
            d.camera = Some(camera.pose);
            d.tan_half = match camera.lens {
                super::camera::Lens::Perspective { tan_half } => tan_half,
                _ => 0.4,
            };
        }
        let shade_data = shade_data.map(Arc::new);
        let any_transparent = materials.iter().any(|m| m.program.as_ref().is_some_and(|p| p.info.transparent));
        let env = Environment::new(scene, shade_data.as_ref());
        let env_avg = env.average();
        let world_light = (env_avg > 0.0).then(|| {
            lights.push(LightShape::World);
            light_info.push(LightInfo { object: u32::MAX, max_bounces: u32::MAX, cast_shadow: true, use_mis: true });
            power.push(env_avg * 4.0 * PI * PI * radius * radius);
            lights.len() as u32 - 1
        });
        let area_lights = lights.iter().enumerate().filter(|(_, l)| matches!(l, LightShape::Area { .. })).map(|(i, _)| i as u32).collect();
        let light_dist = Distribution1D::new(&power);
        let world_volume = scene.world.volume.as_ref().map(VolumeCoeffs::from_settings).filter(|v| !v.is_empty());

        PreparedScene {
            bvh,
            shade,
            materials,
            material_volume,
            material_pass_index,
            images: scene.images.clone(),
            lights,
            light_info,
            light_dist,
            area_lights,
            world_light,
            env,
            world_volume,
            camera,
            object_vis,
            object_holdout,
            object_pass_index,
            any_hidden,
            any_volume: any_volume || world_volume.is_some(),
            any_transparent,
            shade_data,
            instances,
            bounds,
        }
    }

    /// World-space bounds of the renderable geometry.
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// Number of triangles (static and moving).
    pub fn triangle_count(&self) -> usize {
        self.shade.len()
    }

    /// Object index of the nearest surface along a ray, if any.
    pub fn pick_ray(&self, o: Vec3, d: Vec3) -> Option<usize> {
        self.nearest_tri(o, d.normalize(), f32::INFINITY, 0.0).map(|h| self.shade[h.prim as usize].object as usize)
    }

    /// Nearest triangle (static or moving at `time`).
    fn nearest_tri(&self, o: Vec3, d: Vec3, tmax: f32, time: f32) -> Option<Hit> {
        let mut best = self.bvh.intersect(o, d, tmax);
        let mut tmax = best.map_or(tmax, |h| h.t);
        for inst in &self.instances {
            let inv = inst.chain.matrix(time).inverse();
            // Unnormalised local direction keeps t in world units.
            if let Some(h) = inst.bvh.intersect(inv.transform_point(o), inv.transform_vector(d), tmax) {
                tmax = h.t;
                best = Some(Hit { prim: inst.base + h.prim, ..h });
            }
        }
        best
    }

    /// The world-space triangle and shading data of global triangle `prim` at `time`.
    pub(crate) fn triangle(&self, prim: u32, time: f32) -> (TriGeom, TriShade) {
        let sh = self.shade[prim as usize];
        if (prim as usize) < self.bvh.tris.len() {
            return (self.bvh.tris[prim as usize], sh);
        }
        let inst = self.instances.iter().rev().find(|i| prim >= i.base).expect("motion instance");
        let g = inst.bvh.tris[(prim - inst.base) as usize];
        let m = inst.chain.matrix(time);
        let nm = m.inverse().transpose();
        let v0 = m.transform_point(g.v0);
        let wg = TriGeom::new(v0, m.transform_point(g.v0 + g.e1), m.transform_point(g.v0 + g.e2));
        let n = sh.n.map(|n| nm.transform_vector(n).normalize());
        (wg, TriShade { n, ..sh })
    }

    /// Visibility of whatever `h` is for ray-type bits `mask`.
    fn hit_visible(&self, h: &SceneHit, mask: u8) -> bool {
        let object = match h {
            SceneHit::Tri(h) => self.shade[h.prim as usize].object,
            SceneHit::Area(li, _) => self.light_info[*li as usize].object,
        };
        self.object_vis.get(object as usize).is_none_or(|v| v & mask != 0)
    }

    /// Nearest hit within `tmax` visible to rays of type `mask` (objects
    /// hidden from it are passed through).
    pub(crate) fn intersect_ray(&self, o: Vec3, d: Vec3, tmax: f32, time: f32, mask: u8) -> Option<SceneHit> {
        let mut origin = o;
        let mut travelled = 0.0f32;
        for _ in 0..256 {
            let h = self.nearest_any(origin, d, tmax - travelled, time)?;
            if !self.any_hidden || self.hit_visible(&h, mask) {
                return Some(match h {
                    SceneHit::Tri(mut hit) => {
                        hit.t += travelled;
                        SceneHit::Tri(hit)
                    }
                    SceneHit::Area(li, t) => SceneHit::Area(li, t + travelled),
                });
            }
            let t = match h {
                SceneHit::Tri(hit) => hit.t,
                SceneHit::Area(_, t) => t,
            };
            let p = origin + d * t;
            let step = 1e-4 * (1.0 + p.abs().max_elem());
            origin = p + d * step;
            travelled += t + step;
        }
        None
    }

    fn nearest_any(&self, o: Vec3, d: Vec3, tmax: f32, time: f32) -> Option<SceneHit> {
        let tri = self.nearest_tri(o, d, tmax, time);
        let mut best = tri.map(SceneHit::Tri);
        let mut tmax = tri.map_or(tmax, |h| h.t);
        for &li in &self.area_lights {
            if let LightShape::Area { center, u, v, normal, ellipse, .. } = &self.lights[li as usize] {
                let dn = d.dot(*normal);
                if dn >= 0.0 {
                    continue; // back side
                }
                let t = (*center - o).dot(*normal) / dn;
                if !(t > 0.0 && t < tmax) {
                    continue;
                }
                let q = o + d * t - *center;
                let (a, b) = (q.dot(*u) / u.length_squared(), q.dot(*v) / v.length_squared());
                let inside = if *ellipse { a * a + b * b <= 1.0 } else { a.abs() <= 1.0 && b.abs() <= 1.0 };
                if inside {
                    tmax = t;
                    best = Some(SceneHit::Area(li, t));
                }
            }
        }
        best
    }

    /// Whether the material's surface is a pass-through boundary of its medium.
    pub(crate) fn null_surface(&self, material: u32) -> bool {
        self.material_volume.get(material as usize).is_some_and(Option::is_some)
    }

    /// Combined coefficients of the media in `stack` and the world's.
    pub(crate) fn medium(&self, stack: &MediumStack) -> Option<VolumeCoeffs> {
        let mut m = self.world_volume;
        for &id in &stack.ids[..stack.n as usize] {
            if let Some(Some(v)) = self.material_volume.get(id as usize) {
                m = Some(match m {
                    Some(a) => a.add(v),
                    None => *v,
                });
            }
        }
        m
    }

    /// Transmittance of a shadow ray: zero when an opaque surface blocks it,
    /// else the product of medium transmittances through volume boundaries
    /// and objects hidden from shadows (at most `max_transparent` crossings).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shadow(&self, o: Vec3, d: Vec3, tmax: f32, time: f32, stack: MediumStack, max_transparent: u32) -> Vec3 {
        if !self.any_hidden && !self.any_volume && !self.any_transparent && self.instances.is_empty() {
            return if self.bvh.occluded(o, d, tmax) { Vec3::ZERO } else { Vec3::ONE };
        }
        let mut stack = stack;
        let mut origin = o;
        let mut left = tmax;
        let mut tr = Vec3::ONE;
        let mut crossings = 0;
        loop {
            let hit = self.nearest_tri(origin, d, left, time);
            let seg = hit.map_or(left, |h| h.t);
            if let Some(m) = self.medium(&stack) {
                tr = tr.mul_elem(m.transmittance(seg));
            }
            let Some(h) = hit else { return tr };
            let sh = &self.shade[h.prim as usize];
            let hidden = self.object_vis.get(sh.object as usize).is_some_and(|v| v & VIS_SHADOW == 0);
            let null = self.null_surface(sh.material);
            if !hidden && !null {
                // Transparent node materials let some light through.
                let t = if self.any_transparent { self.shadow_transparency(&h, d, time) } else { Vec3::ZERO };
                if t == Vec3::ZERO {
                    return Vec3::ZERO;
                }
                tr = tr.mul_elem(t);
            }
            crossings += 1;
            if crossings > max_transparent || tr.max_elem() <= 1e-6 {
                return Vec3::ZERO;
            }
            if null && !hidden {
                let (g, _) = self.triangle(h.prim, time);
                stack.cross(sh.material, g.e1.cross(g.e2).dot(d) < 0.0);
            }
            let p = origin + d * h.t;
            let step = 1e-4 * (1.0 + p.abs().max_elem());
            origin = p + d * step;
            left -= h.t + step;
            if left <= 0.0 {
                return tr;
            }
        }
    }

    fn select_pmf(&self, light: u32) -> f32 {
        self.light_dist.as_ref().map_or(0.0, |d| d.pmf(light as usize))
    }

    /// Pdf (solid angle, with selection) that NEE would have sampled the
    /// point at distance `t` along `d` on emissive triangle / area light `light`
    /// with geometric normal `n`.
    pub(crate) fn surface_light_pdf(&self, light: u32, d: Vec3, t: f32, n: Vec3) -> f32 {
        let area = match &self.lights[light as usize] {
            LightShape::Tri { area, .. } | LightShape::Area { area, .. } => *area,
            _ => return 0.0,
        };
        let cos = n.dot(d).abs();
        if cos <= 0.0 {
            return 0.0;
        }
        self.select_pmf(light) * t * t / (area * cos)
    }

    /// Pdf that NEE would have sampled world direction `d`.
    pub(crate) fn world_pdf(&self, d: Vec3) -> f32 {
        self.world_light.map_or(0.0, |w| self.select_pmf(w) * self.env.pdf(d))
    }

    pub(crate) fn has_lights(&self) -> bool {
        self.light_dist.is_some()
    }

    /// Pick a light by power and sample a point on it as seen from `p`.
    /// Returns the light index with the sample.
    pub(crate) fn sample_light(&self, p: Vec3, rng: &mut Rng) -> Option<(u32, LightSample)> {
        let dist = self.light_dist.as_ref()?;
        let (li, sel) = dist.sample_discrete(rng.f32());
        let (u1, u2) = (rng.f32(), rng.f32());
        let s = match &self.lights[li] {
            LightShape::Point { pos, radius, intensity, spot } => {
                let mut target = *pos;
                if *radius > 0.0 {
                    let axis = (*pos - p).normalize();
                    let (t, b) = onb(axis);
                    let (dx, dy) = concentric_disk(u1, u2);
                    target += (t * dx + b * dy) * *radius;
                }
                let to = target - p;
                let d2 = to.length_squared();
                if d2 <= 1e-12 {
                    return None;
                }
                let dist = d2.sqrt();
                let wi = to / dist;
                let mut li = *intensity / d2;
                if let Some((axis, cos_half, blend)) = spot {
                    let c = axis.dot(-wi);
                    let x = if *blend > 0.0 { ((c - cos_half) / blend).clamp(0.0, 1.0) } else if c >= *cos_half { 1.0 } else { 0.0 };
                    li *= x * x * (3.0 - 2.0 * x);
                }
                Some(LightSample { wi, dist, li, pdf: sel, delta: true })
            }
            LightShape::Sun { to_light, cos_max, irradiance } => {
                let wi = if *cos_max < 1.0 {
                    let (t, b) = onb(*to_light);
                    let l = uniform_cone(u1, u2, *cos_max);
                    (t * l.x + b * l.y + *to_light * l.z).normalize()
                } else {
                    *to_light
                };
                Some(LightSample { wi, dist: f32::INFINITY, li: *irradiance, pdf: sel, delta: true })
            }
            LightShape::Area { center, u, v, normal, area, radiance, ellipse, spread } => {
                let (a, b) = if *ellipse { concentric_disk(u1, u2) } else { (2.0 * u1 - 1.0, 2.0 * u2 - 1.0) };
                let q = *center + *u * a + *v * b;
                self.area_sample(p, q, *normal, *area, *radiance, sel, false).map(|mut s| {
                    s.li *= spread_attenuation(*spread, -s.wi.dot(*normal));
                    s
                })
            }
            LightShape::Tri { prim, area, radiance } => {
                let g = &self.bvh.tris[*prim as usize];
                let su = u1.sqrt();
                let q = g.v0 + g.e1 * (1.0 - su) + g.e2 * (u2 * su);
                self.area_sample(p, q, g.e1.cross(g.e2).normalize(), *area, *radiance, sel, true).map(|mut s| {
                    // Node emission varies over the surface: evaluate it here.
                    if let Some(e) = self.node_emission(*prim, q, s.wi) {
                        s.li = e;
                    }
                    s
                })
            }
            LightShape::World => {
                let (wi, pdf) = self.env.sample(u1, u2);
                (pdf > 0.0).then(|| LightSample { wi, dist: f32::INFINITY, li: self.env.eval(wi), pdf: pdf * sel, delta: false })
            }
        };
        s.map(|s| (li as u32, s))
    }

    #[allow(clippy::too_many_arguments)]
    fn area_sample(&self, p: Vec3, q: Vec3, n: Vec3, area: f32, radiance: Vec3, sel: f32, two_sided: bool) -> Option<LightSample> {
        let to = q - p;
        let d2 = to.length_squared();
        if d2 <= 1e-12 {
            return None;
        }
        let dist = d2.sqrt();
        let wi = to / dist;
        let cos = -n.dot(wi);
        let cos = if two_sided { cos.abs() } else { cos };
        if cos <= 1e-6 {
            return None;
        }
        Some(LightSample { wi, dist, li: radiance, pdf: sel * d2 / (area * cos), delta: false })
    }
}

/// Spread of an area light: (tan of half the spread, normalisation), or
/// `None` for the full Lambertian hemisphere. The normalisation keeps the
/// emitted power: the attenuated cosine lobe integrates to pi.
fn spread_params(spread: f32) -> Option<(f32, f32)> {
    if !spread.is_finite() || spread >= PI - 1e-4 {
        return None;
    }
    let half = (spread.max(1e-3) * 0.5).min(PI * 0.5 - 1e-4);
    let t = half.tan();
    // 2 pi k [t sin²θm / 2 - θm / 2 + sin 2θm / 4] = pi, θm = atan t = half.
    let i = t * half.sin().powi(2) * 0.5 - half * 0.5 + (2.0 * half).sin() * 0.25;
    Some((t, 1.0 / (2.0 * i.max(1e-9))))
}

/// Cycles' soft-box spread falloff for emission at angle `cos` from the normal.
pub(crate) fn spread_attenuation(spread: Option<(f32, f32)>, cos: f32) -> f32 {
    match spread {
        None => 1.0,
        Some((tan_half, norm)) => {
            if cos <= 0.0 {
                return 0.0;
            }
            let tan_a = (1.0 - cos * cos).max(0.0).sqrt() / cos;
            ((tan_half - tan_a) * norm).max(0.0)
        }
    }
}
