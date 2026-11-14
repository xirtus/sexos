// SPDX-License-Identifier: GPL-3.0-only
//! Scenes: objects in collections, sharing mesh, material and image data.
//! Z is up and units are metres, like Blender; glTF's Y-up is converted on
//! import/export.
use crate::math::{Aabb, Mat4, Quat, Vec3};
use crate::mesh::Mesh;
use crate::modifiers::Modifier;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub location: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { location: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE }
    }
}

impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_trs(self.location, self.rotation, self.scale)
    }
    pub fn from_matrix(m: &Mat4) -> Transform {
        let (location, rotation, scale) = m.to_trs();
        Transform { location, rotation, scale }
    }
}

/// Linear-light RGBA image (textures, HDRIs, render results).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Row-major from the top-left, linear RGBA.
    pub pixels: Vec<[f32; 4]>,
}

impl Image {
    pub fn new(name: &str, width: u32, height: u32) -> Image {
        Image { name: name.into(), width, height, pixels: vec![[0.0, 0.0, 0.0, 1.0]; (width * height) as usize] }
    }
    /// Bilinear sample with wrap-around; `v` = 0 is the bottom row (UV convention).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [1.0, 0.0, 1.0, 1.0];
        }
        let x = (u - u.floor()) * self.width as f32 - 0.5;
        let y = (1.0 - (v - v.floor())) * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let px = |xi: f32, yi: f32| {
            let xi = (xi as i64).rem_euclid(self.width as i64) as usize;
            let yi = (yi as i64).rem_euclid(self.height as i64) as usize;
            self.pixels[yi * self.width as usize + xi]
        };
        let (a, b, c, d) = (px(x0, y0), px(x0 + 1.0, y0), px(x0, y0 + 1.0), px(x0 + 1.0, y0 + 1.0));
        let mut o = [0.0; 4];
        for i in 0..4 {
            o[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
        }
        o
    }
}

/// Principled (metallic-roughness) material, the common ground of glTF,
/// Blender's Principled BSDF and the path tracer.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    pub base_color_texture: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: Vec3,
    pub emission_strength: f32,
    pub ior: f32,
    /// 0 = opaque, 1 = fully transmissive (glass).
    pub transmission: f32,
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            name: "Material".into(),
            base_color: [0.8, 0.8, 0.8, 1.0],
            base_color_texture: None,
            metallic: 0.0,
            roughness: 0.5,
            emission: Vec3::ZERO,
            emission_strength: 0.0,
            ior: 1.5,
            transmission: 0.0,
            double_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Vertical field of view in radians (perspective).
    pub fov_y: f32,
    /// `Some(height)` for orthographic cameras.
    pub ortho_scale: Option<f32>,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        // Blender's default 50 mm lens on a 36 mm sensor, as a vertical FOV at 16:9.
        Camera { fov_y: 2.0 * (18.0f32 / 50.0 * 9.0 / 16.0).atan(), ortho_scale: None, near: 0.1, far: 1000.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Point,
    /// Directional light along the object's -Z.
    Sun,
    /// Cone along -Z with full angle `angle` (radians) and soft `blend` 0..1.
    Spot { angle: f32, blend: f32 },
    /// Square emitter of side `size` facing -Z.
    Area { size: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    /// Linear RGB.
    pub color: Vec3,
    /// Watts for point/spot/area (Blender convention); irradiance W/m² for sun.
    pub power: f32,
    /// Soft shadow radius for point/spot, angular diameter (radians) for sun.
    pub radius: f32,
}

impl Default for Light {
    fn default() -> Self {
        Light { kind: LightKind::Point, color: Vec3::ONE, power: 1000.0, radius: 0.1 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectData {
    Empty,
    /// Index into [`Scene::meshes`]; several objects may share one mesh.
    Mesh(usize),
    Camera(Camera),
    Light(Light),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub name: String,
    pub transform: Transform,
    pub parent: Option<usize>,
    pub data: ObjectData,
    /// Non-destructive modifier stack, applied top to bottom.
    pub modifiers: Vec<Modifier>,
    /// Material slots: indices into [`Scene::materials`]; a face's `material`
    /// picks a slot.
    pub materials: Vec<usize>,
    pub collection: usize,
    pub visible: bool,
}

impl Object {
    pub fn new(name: &str, data: ObjectData) -> Object {
        Object { name: name.into(), transform: Transform::default(), parent: None, data, modifiers: Vec::new(), materials: Vec::new(), collection: 0, visible: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    pub name: String,
    pub parent: Option<usize>,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// Uniform background radiance when there is no HDRI.
    pub color: Vec3,
    pub strength: f32,
    /// Equirectangular environment image in [`Scene::images`].
    pub hdri: Option<usize>,
}

impl Default for World {
    fn default() -> Self {
        World { color: Vec3::splat(0.05), strength: 1.0, hdri: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub name: String,
    pub objects: Vec<Object>,
    pub collections: Vec<Collection>,
    pub meshes: Vec<Mesh>,
    pub mesh_names: Vec<String>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    pub world: World,
    pub active_camera: Option<usize>,
    pub active_object: Option<usize>,
    pub selected: Vec<usize>,
    pub frame_start: i32,
    pub frame_end: i32,
    pub fps: f32,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            name: "Scene".into(),
            objects: Vec::new(),
            collections: vec![Collection { name: "Collection".into(), parent: None, visible: true }],
            meshes: Vec::new(),
            mesh_names: Vec::new(),
            materials: Vec::new(),
            images: Vec::new(),
            world: World::default(),
            active_camera: None,
            active_object: None,
            selected: Vec::new(),
            frame_start: 1,
            frame_end: 250,
            fps: 24.0,
        }
    }
}

impl Scene {
    pub fn new() -> Scene {
        Scene::default()
    }

    /// The scene a new Polyamour file opens with: a cube, a light and a camera.
    pub fn startup() -> Scene {
        let mut s = Scene::new();
        let mat = s.add_material(Material::default());
        let cube = s.add_mesh_object("Cube", crate::primitives::cube(2.0));
        s.objects[cube].materials.push(mat);
        let light = s.add_object(Object::new("Light", ObjectData::Light(Light::default())));
        s.objects[light].transform.location = Vec3::new(4.08, 1.0, 5.9);
        let cam = s.add_object(Object::new("Camera", ObjectData::Camera(Camera::default())));
        s.objects[cam].transform = Transform::from_matrix(&Mat4::look_at(Vec3::new(7.36, -6.93, 4.96), Vec3::ZERO, Vec3::Z).inverse());
        s.active_camera = Some(cam);
        s.active_object = Some(cube);
        s
    }

    /// Blender-style unique name: "Cube", then "Cube.001", "Cube.002", ...
    pub fn unique_object_name(&self, base: &str) -> String {
        unique_name(base, |n| self.objects.iter().any(|o| o.name == n))
    }

    pub fn add_object(&mut self, mut o: Object) -> usize {
        o.name = self.unique_object_name(&o.name);
        if o.collection >= self.collections.len() {
            o.collection = 0;
        }
        self.objects.push(o);
        self.objects.len() - 1
    }

    pub fn add_mesh(&mut self, name: &str, m: Mesh) -> usize {
        let n = unique_name(name, |n| self.mesh_names.iter().any(|x| x == n));
        self.meshes.push(m);
        self.mesh_names.push(n);
        self.meshes.len() - 1
    }

    pub fn add_mesh_object(&mut self, name: &str, m: Mesh) -> usize {
        let mi = self.add_mesh(name, m);
        self.add_object(Object::new(name, ObjectData::Mesh(mi)))
    }

    pub fn add_material(&mut self, m: Material) -> usize {
        self.materials.push(m);
        self.materials.len() - 1
    }

    pub fn add_collection(&mut self, name: &str, parent: Option<usize>) -> usize {
        let n = unique_name(name, |n| self.collections.iter().any(|c| c.name == n));
        self.collections.push(Collection { name: n, parent, visible: true });
        self.collections.len() - 1
    }

    pub fn find_object(&self, name: &str) -> Option<usize> {
        self.objects.iter().position(|o| o.name == name)
    }

    /// Remove an object; children are re-parented to its parent and indices
    /// held by the scene (parents, camera, selection) are fixed up.
    pub fn remove_object(&mut self, idx: usize) -> Object {
        let gone_parent = self.objects[idx].parent;
        let o = self.objects.remove(idx);
        let fix = |i: usize| if i > idx { i - 1 } else { i };
        for ob in &mut self.objects {
            ob.parent = match ob.parent {
                Some(p) if p == idx => gone_parent.map(fix),
                Some(p) => Some(fix(p)),
                None => None,
            };
        }
        self.active_camera = self.active_camera.and_then(|c| (c != idx).then(|| fix(c)));
        self.active_object = self.active_object.and_then(|c| (c != idx).then(|| fix(c)));
        self.selected.retain(|&s| s != idx);
        for s in &mut self.selected {
            *s = fix(*s);
        }
        o
    }

    /// Object-to-world matrix through the parent chain.
    pub fn world_matrix(&self, idx: usize) -> Mat4 {
        let mut m = self.objects[idx].transform.matrix();
        let mut p = self.objects[idx].parent;
        let mut guard = 0;
        while let Some(pi) = p {
            m = self.objects[pi].transform.matrix() * m;
            p = self.objects[pi].parent;
            guard += 1;
            if guard > self.objects.len() {
                break; // parent cycle
            }
        }
        m
    }

    /// The object's mesh after its modifier stack, in object space.
    pub fn evaluated_mesh(&self, idx: usize) -> Option<Mesh> {
        match self.objects[idx].data {
            ObjectData::Mesh(mi) => Some(crate::modifiers::apply_stack(&self.meshes[mi], &self.objects[idx].modifiers)),
            _ => None,
        }
    }

    /// Evaluated mesh in world space with face materials resolved to scene
    /// material indices (`None` slot -> default material, index `usize::MAX`).
    pub fn world_mesh(&self, idx: usize) -> Option<(Mesh, Vec<usize>)> {
        let mut m = self.evaluated_mesh(idx)?;
        m.transform(&self.world_matrix(idx));
        let slots = self.objects[idx].materials.clone();
        Some((m, slots))
    }

    /// True when the object and every collection up its chain is visible.
    pub fn is_visible(&self, idx: usize) -> bool {
        let o = &self.objects[idx];
        if !o.visible {
            return false;
        }
        let mut c = Some(o.collection);
        let mut guard = 0;
        while let Some(ci) = c {
            let col = match self.collections.get(ci) {
                Some(col) => col,
                None => break,
            };
            if !col.visible {
                return false;
            }
            c = col.parent;
            guard += 1;
            if guard > self.collections.len() {
                break;
            }
        }
        true
    }

    /// World-space bounds of every visible mesh object.
    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for i in 0..self.objects.len() {
            if !self.is_visible(i) {
                continue;
            }
            if let Some((m, _)) = self.world_mesh(i) {
                b = b.union(m.bounds());
            }
        }
        b
    }
}

pub fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    // Strip an existing ".NNN" suffix so "Cube.001" duplicates to "Cube.002".
    let stem = match base.rsplit_once('.') {
        Some((s, n)) if n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()) => s,
        _ => base,
    };
    (1..).map(|i| format!("{stem}.{i:03}")).find(|n| !taken(n)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parents_and_removal() {
        let mut s = Scene::startup();
        let a = s.add_mesh_object("Cube", crate::primitives::cube(1.0));
        assert_eq!(s.objects[a].name, "Cube.001");
        let b = s.add_mesh_object("Cube.001", crate::primitives::cube(1.0));
        assert_eq!(s.objects[b].name, "Cube.002");
        s.objects[b].parent = Some(a);
        s.objects[a].transform.location = Vec3::new(1.0, 0.0, 0.0);
        s.objects[b].transform.location = Vec3::new(0.0, 2.0, 0.0);
        assert!(s.world_matrix(b).transform_point(Vec3::ZERO).approx_eq(Vec3::new(1.0, 2.0, 0.0), 1e-6));
        let cam = s.active_camera.unwrap();
        s.remove_object(0);
        assert_eq!(s.active_camera, Some(cam - 1));
        assert_eq!(s.objects[b - 1].parent, Some(a - 1));
    }

    #[test]
    fn startup_camera_looks_at_cube() {
        let s = Scene::startup();
        let cam = s.active_camera.unwrap();
        let m = s.world_matrix(cam);
        let forward = m.transform_vector(-Vec3::Z).normalize();
        let to_origin = (Vec3::ZERO - m.transform_point(Vec3::ZERO)).normalize();
        assert!(forward.approx_eq(to_origin, 1e-4));
    }
}
