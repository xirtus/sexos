// SPDX-License-Identifier: GPL-3.0-only
//! Scene preparation: flatten visible mesh objects into a world-space triangle
//! BVH with shading data, resolve materials, and build the light list (lights,
//! emissive triangles, world) with its power-based selection distribution.
use super::bvh::{Bvh, Hit, TriGeom};
use super::luminance;
use super::rng::Rng;
use super::sampling::{concentric_disk, onb, uniform_cone, uniform_sphere, Distribution1D};
use crate::math::{Aabb, Mat4, Vec2, Vec3};
use crate::scene::{Camera, Image, LightKind, Material, ObjectData, Scene};
use core::f32::consts::{FRAC_1_PI, PI, TAU};

/// Per-triangle shading data (in BVH order).
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriShade {
    pub n: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub material: u32,
    /// Index into the light list for emissive triangles, else `u32::MAX`.
    pub light: u32,
    pub object: u32,
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
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum LightShape {
    /// Point or spot light; `spot` = (axis, cos of half angle, blend width in cos).
    Point { pos: Vec3, radius: f32, intensity: Vec3, spot: Option<(Vec3, f32, f32)> },
    Sun { to_light: Vec3, cos_max: f32, irradiance: Vec3 },
    /// One-sided parallelogram `center ± u ± v`, emitting along `normal`.
    Area { center: Vec3, u: Vec3, v: Vec3, normal: Vec3, area: f32, radiance: Vec3 },
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
    fn new(scene: &Scene) -> Environment {
        let w = &scene.world;
        let strength = if w.strength.is_finite() { w.strength.max(0.0) } else { 0.0 };
        let hdri = w.hdri.and_then(|i| scene.images.get(i)).filter(|im| im.width > 0 && im.height > 0 && im.pixels.len() == (im.width * im.height) as usize).cloned();
        let dist = hdri.as_ref().and_then(EnvDistribution::new);
        Environment { color: w.color * strength, hdri, strength, dist }
    }

    pub fn eval(&self, d: Vec3) -> Vec3 {
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

/// Camera rig for generating primary rays.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CameraRig {
    pub origin: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
    pub tan_half: f32,
    pub ortho: Option<f32>,
}

impl CameraRig {
    fn new(m: &Mat4, cam: &Camera) -> CameraRig {
        CameraRig {
            origin: m.transform_point(Vec3::ZERO),
            right: m.transform_vector(Vec3::X).normalize(),
            up: m.transform_vector(Vec3::Y).normalize(),
            forward: m.transform_vector(-Vec3::Z).normalize(),
            tan_half: (cam.fov_y.clamp(1e-4, 3.1) * 0.5).tan(),
            ortho: cam.ortho_scale.filter(|s| *s > 0.0),
        }
    }

    /// Ray through film position (`fx`, `fy`) in pixels of a `w` x `h` frame.
    pub fn ray(&self, fx: f32, fy: f32, w: u32, h: u32) -> (Vec3, Vec3) {
        let aspect = w as f32 / h.max(1) as f32;
        let sx = 2.0 * fx / w as f32 - 1.0;
        let sy = 1.0 - 2.0 * fy / h as f32;
        match self.ortho {
            Some(scale) => {
                let o = self.origin + self.right * (sx * scale * 0.5 * aspect) + self.up * (sy * scale * 0.5);
                (o, self.forward)
            }
            None => {
                let d = self.forward + self.right * (sx * self.tan_half * aspect) + self.up * (sy * self.tan_half);
                (self.origin, d.normalize())
            }
        }
    }
}

/// A scene flattened for path tracing. Build once, render many times (the
/// progressive viewport and the render farm reuse it). Owns all its data.
#[derive(Clone, Debug)]
pub struct PreparedScene {
    pub(crate) bvh: Bvh,
    pub(crate) shade: Vec<TriShade>,
    pub(crate) materials: Vec<ShadeMaterial>,
    pub(crate) images: Vec<Image>,
    pub(crate) lights: Vec<LightShape>,
    pub(crate) light_dist: Option<Distribution1D>,
    /// Indices of [`LightShape::Area`] lights, which rays can hit.
    pub(crate) area_lights: Vec<u32>,
    pub(crate) world_light: Option<u32>,
    pub(crate) env: Environment,
    pub(crate) camera: CameraRig,
    bounds: Aabb,
}

/// What a ray hit.
pub(crate) enum SceneHit {
    Tri(Hit),
    /// An area light (light index) at distance t.
    Area(u32, f32),
}

impl PreparedScene {
    /// Flatten `scene` for rendering from its active camera.
    pub fn new(scene: &Scene) -> PreparedScene {
        let mut materials: Vec<ShadeMaterial> = scene.materials.iter().map(|m| ShadeMaterial::from(m, scene.images.len())).collect();
        let default_mat = materials.len() as u32;
        materials.push(ShadeMaterial::from(&Material::default(), 0));

        let mut geoms = Vec::new();
        let mut shades = Vec::new();
        for oi in 0..scene.objects.len() {
            if !scene.is_visible(oi) {
                continue;
            }
            let Some((mesh, slots)) = scene.world_mesh(oi) else { continue };
            let smooth = mesh.faces.iter().any(|f| f.smooth);
            let vn = if smooth { mesh.vertex_normals() } else { Vec::new() };
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
                shades.push(TriShade { n, uv, material, light: u32::MAX, object: oi as u32 });
            }
        }
        let (bvh, order) = Bvh::build(geoms);
        let mut shade: Vec<TriShade> = order.iter().map(|&i| shades[i as usize]).collect();
        let bounds = bvh.bounds();
        let radius = if bounds.is_empty() { 1.0 } else { (bounds.size().length() * 0.5).max(1e-3) };

        // Lights and their selection weights (estimated emitted power).
        let mut lights = Vec::new();
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
                LightKind::Area { size } => {
                    let u = m.transform_vector(Vec3::X) * (size * 0.5);
                    let v = m.transform_vector(Vec3::Y) * (size * 0.5);
                    let area = 4.0 * u.cross(v).length();
                    if area <= 0.0 {
                        continue;
                    }
                    lights.push(LightShape::Area { center: pos, u, v, normal: axis, area, radiance: color * (l.power / (area * PI)) });
                    power.push(l.power * lum);
                }
            }
        }
        for (prim, s) in shade.iter_mut().enumerate() {
            let e = materials[s.material as usize].emission;
            if e == Vec3::ZERO {
                continue;
            }
            let g = &bvh.tris[prim];
            let area = 0.5 * g.e1.cross(g.e2).length();
            s.light = lights.len() as u32;
            lights.push(LightShape::Tri { prim: prim as u32, area, radiance: e });
            // Two-sided emitter: power = L * A * pi per side.
            power.push(luminance(e) * area * 2.0 * PI);
        }
        let env = Environment::new(scene);
        let env_avg = env.average();
        let world_light = (env_avg > 0.0).then(|| {
            lights.push(LightShape::World);
            power.push(env_avg * 4.0 * PI * PI * radius * radius);
            lights.len() as u32 - 1
        });
        let area_lights = lights.iter().enumerate().filter(|(_, l)| matches!(l, LightShape::Area { .. })).map(|(i, _)| i as u32).collect();
        let light_dist = Distribution1D::new(&power);

        let camera = match scene.active_camera.filter(|&c| c < scene.objects.len()) {
            Some(ci) => {
                let cam = match &scene.objects[ci].data {
                    ObjectData::Camera(c) => *c,
                    _ => Camera::default(),
                };
                CameraRig::new(&scene.world_matrix(ci), &cam)
            }
            None => CameraRig::new(&Mat4::look_at(Vec3::new(7.36, -6.93, 4.96), Vec3::ZERO, Vec3::Z).inverse(), &Camera::default()),
        };

        PreparedScene { bvh, shade, materials, images: scene.images.clone(), lights, light_dist, area_lights, world_light, env, camera, bounds }
    }

    /// World-space bounds of the renderable geometry.
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// Number of triangles in the BVH.
    pub fn triangle_count(&self) -> usize {
        self.bvh.tris.len()
    }

    /// Object index of the nearest surface along a ray, if any.
    pub fn pick_ray(&self, o: Vec3, d: Vec3) -> Option<usize> {
        self.bvh.intersect(o, d.normalize(), f32::INFINITY).map(|h| self.shade[h.prim as usize].object as usize)
    }

    /// Nearest hit: triangles or the front of an area light.
    pub(crate) fn intersect(&self, o: Vec3, d: Vec3) -> Option<SceneHit> {
        let tri = self.bvh.intersect(o, d, f32::INFINITY);
        let mut best = tri.map(SceneHit::Tri);
        let mut tmax = tri.map_or(f32::INFINITY, |h| h.t);
        for &li in &self.area_lights {
            if let LightShape::Area { center, u, v, normal, .. } = &self.lights[li as usize] {
                let dn = d.dot(*normal);
                if dn >= 0.0 {
                    continue; // back side
                }
                let t = (*center - o).dot(*normal) / dn;
                if !(t > 0.0 && t < tmax) {
                    continue;
                }
                let q = o + d * t - *center;
                if q.dot(*u).abs() <= u.length_squared() && q.dot(*v).abs() <= v.length_squared() {
                    tmax = t;
                    best = Some(SceneHit::Area(li, t));
                }
            }
        }
        best
    }

    pub(crate) fn occluded(&self, o: Vec3, d: Vec3, tmax: f32) -> bool {
        self.bvh.occluded(o, d, tmax)
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
    pub(crate) fn sample_light(&self, p: Vec3, rng: &mut Rng) -> Option<LightSample> {
        let dist = self.light_dist.as_ref()?;
        let (li, sel) = dist.sample_discrete(rng.f32());
        let (u1, u2) = (rng.f32(), rng.f32());
        match &self.lights[li] {
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
            LightShape::Area { center, u, v, normal, area, radiance } => {
                let q = *center + *u * (2.0 * u1 - 1.0) + *v * (2.0 * u2 - 1.0);
                self.area_sample(p, q, *normal, *area, *radiance, sel, false)
            }
            LightShape::Tri { prim, area, radiance } => {
                let g = &self.bvh.tris[*prim as usize];
                let su = u1.sqrt();
                let q = g.v0 + g.e1 * (1.0 - su) + g.e2 * (u2 * su);
                self.area_sample(p, q, g.e1.cross(g.e2).normalize(), *area, *radiance, sel, true)
            }
            LightShape::World => {
                let (wi, pdf) = self.env.sample(u1, u2);
                (pdf > 0.0).then(|| LightSample { wi, dist: f32::INFINITY, li: self.env.eval(wi), pdf: pdf * sel, delta: false })
            }
        }
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
