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
    /// Linear location/scale and spherical rotation interpolation; `t`
    /// outside 0..1 extrapolates.
    pub fn lerp(&self, o: &Transform, t: f32) -> Transform {
        Transform { location: self.location.lerp(o.location, t), rotation: self.rotation.slerp(o.rotation, t), scale: self.scale.lerp(o.scale, t) }
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
    /// Shader node tree (Use Nodes). When set, the renderer shades from the
    /// tree and the fields above mirror its Principled BSDF for the viewport
    /// and exporters.
    pub node_tree: Option<crate::nodes::NodeTree>,
    /// Blender's material "Pass Index" (the Material Index render pass).
    pub pass_index: u32,
    /// Homogeneous volume inside closed meshes using this material (a
    /// Principled Volume's inputs). A material with a volume renders with no
    /// surface: its faces only bound the medium.
    pub volume: Option<VolumeSettings>,
}

/// Principled Volume inputs, for homogeneous media in the path tracer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeSettings {
    /// Scattering colour (linear RGB).
    pub color: Vec3,
    pub density: f32,
    /// Henyey-Greenstein anisotropy, -1 (back) to 1 (forward).
    pub anisotropy: f32,
    /// Extra absorption tint: black absorbs only what `color` does not scatter.
    pub absorption_color: Vec3,
    pub emission_color: Vec3,
    pub emission_strength: f32,
}

impl Default for VolumeSettings {
    fn default() -> Self {
        VolumeSettings { color: Vec3::splat(0.5), density: 1.0, anisotropy: 0.0, absorption_color: Vec3::ZERO, emission_color: Vec3::ONE, emission_strength: 0.0 }
    }
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
            node_tree: None,
            pass_index: 0,
            volume: None,
        }
    }
}

impl Material {
    /// Copy the node tree's Principled BSDF (the one feeding the active
    /// Material Output's Surface, through reroutes) back into the legacy
    /// fields the viewport and exporters read. Linked inputs keep the field's
    /// value, except an Image Texture on Base Color, which sets
    /// `base_color_texture` (its image) and a white base colour. Returns
    /// false when there is no such node.
    pub fn sync_from_tree(&mut self) -> bool {
        use crate::nodes::{Prop, Value};
        let Some(t) = &self.node_tree else { return false };
        let out = t.nodes.iter().enumerate().filter(|(_, n)| n.idname == "ShaderNodeOutputMaterial" && matches!(n.enum_prop("target"), "" | "ALL" | "CYCLES")).max_by_key(|(_, n)| n.bool_prop("is_active_output")).map(|(i, _)| i);
        let Some(out) = out else { return false };
        // Follow the Surface link through reroutes.
        let source = |node: usize, sock: usize| -> Option<usize> {
            let mut l = t.input_link(node, sock).filter(|l| !l.muted)?;
            for _ in 0..64 {
                if t.nodes[l.from_node].idname != "NodeReroute" {
                    return Some(l.from_node);
                }
                l = t.input_link(l.from_node, 0).filter(|l| !l.muted)?;
            }
            None
        };
        let Some(si) = t.nodes[out].input("Surface") else { return false };
        let Some(p) = source(out, si) else { return false };
        let n = &t.nodes[p];
        if n.idname != "ShaderNodeBsdfPrincipled" || n.mute {
            return false;
        }
        let linked = |id: &str| n.input(id).is_some_and(|i| t.input_link(p, i).is_some_and(|l| !l.muted));
        let val = |id: &str| n.input(id).map(|i| n.inputs[i].value.clone()).unwrap_or(Value::None);
        let f = |id: &str, d: f32| match val(id) {
            Value::None => d,
            v => v.as_float(),
        };
        if let Some(bi) = n.input("Base Color") {
            let tex = source(p, bi).filter(|&s| t.nodes[s].idname == "ShaderNodeTexImage").and_then(|s| match t.nodes[s].prop("image") {
                Some(Prop::Id(i)) => *i,
                _ => None,
            });
            match tex {
                Some(img) => {
                    self.base_color_texture = Some(img);
                    self.base_color = [1.0, 1.0, 1.0, self.base_color[3]];
                }
                None if !linked("Base Color") => {
                    let c = val("Base Color").as_color();
                    self.base_color = [c[0], c[1], c[2], self.base_color[3]];
                    self.base_color_texture = None;
                }
                None => {}
            }
        }
        if !linked("Alpha") {
            self.base_color[3] = f("Alpha", self.base_color[3]);
        }
        if !linked("Metallic") {
            self.metallic = f("Metallic", self.metallic);
        }
        if !linked("Roughness") {
            self.roughness = f("Roughness", self.roughness);
        }
        if !linked("IOR") {
            self.ior = f("IOR", self.ior);
        }
        if !linked("Transmission Weight") {
            self.transmission = f("Transmission Weight", self.transmission);
        }
        if !linked("Emission Color") {
            let c = val("Emission Color").as_vector();
            self.emission = Vec3::new(c[0], c[1], c[2]);
        }
        if !linked("Emission Strength") {
            self.emission_strength = f("Emission Strength", self.emission_strength);
        }
        true
    }
}

/// Panoramic lens projections (Blender's "Panorama Type").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanoramaType {
    #[default]
    Equirectangular,
    FisheyeEquidistant,
    FisheyeEquisolid,
}

/// Which sensor dimension the focal length refers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SensorFit {
    /// The sensor width spans the larger image dimension.
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

/// The lens type, as Blender's camera "Type".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraType {
    Perspective,
    Orthographic,
    Panoramic,
}

/// Thin-lens depth of field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    pub enabled: bool,
    /// Focus on this object's origin (index into [`Scene::objects`]) instead
    /// of `focus_distance`.
    pub focus_object: Option<usize>,
    /// Metres along the view axis.
    pub focus_distance: f32,
    /// F-stop: the aperture diameter is the focal length over this.
    pub fstop: f32,
    /// Polygonal bokeh with this many blades (fewer than 3: round).
    pub blades: u32,
    /// Blade rotation in radians.
    pub rotation: f32,
    /// Anamorphic squeeze (1 = round bokeh).
    pub ratio: f32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        DepthOfField { enabled: false, focus_object: None, focus_distance: 10.0, fstop: 2.8, blades: 0, rotation: 0.0, ratio: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Vertical field of view in radians (perspective).
    pub fov_y: f32,
    /// `Some(height)` for orthographic cameras.
    pub ortho_scale: Option<f32>,
    /// Clip start and end (metres).
    pub near: f32,
    pub far: f32,
    /// `Some` makes the camera panoramic (overrides perspective/orthographic).
    pub panorama: Option<PanoramaType>,
    /// Sensor size in millimetres (Blender: 36 x 24).
    pub sensor_width: f32,
    pub sensor_height: f32,
    pub sensor_fit: SensorFit,
    /// Lens shift, in units of the larger frame dimension.
    pub shift_x: f32,
    pub shift_y: f32,
    /// Fisheye field of view (radians) and equisolid focal length (mm).
    pub fisheye_fov: f32,
    pub fisheye_lens: f32,
    /// Equirectangular ranges (radians).
    pub latitude_min: f32,
    pub latitude_max: f32,
    pub longitude_min: f32,
    pub longitude_max: f32,
    pub dof: DepthOfField,
}

impl Default for Camera {
    fn default() -> Self {
        // Blender's default 50 mm lens on a 36 mm sensor, as a vertical FOV at 16:9.
        Camera {
            fov_y: 2.0 * (18.0f32 / 50.0 * 9.0 / 16.0).atan(),
            ortho_scale: None,
            near: 0.1,
            far: 1000.0,
            panorama: None,
            sensor_width: 36.0,
            sensor_height: 24.0,
            sensor_fit: SensorFit::Auto,
            shift_x: 0.0,
            shift_y: 0.0,
            fisheye_fov: core::f32::consts::PI,
            fisheye_lens: 10.5,
            latitude_min: -core::f32::consts::FRAC_PI_2,
            latitude_max: core::f32::consts::FRAC_PI_2,
            longitude_min: -core::f32::consts::PI,
            longitude_max: core::f32::consts::PI,
            dof: DepthOfField::default(),
        }
    }
}

impl Camera {
    pub fn camera_type(&self) -> CameraType {
        if self.panorama.is_some() {
            CameraType::Panoramic
        } else if self.ortho_scale.is_some() {
            CameraType::Orthographic
        } else {
            CameraType::Perspective
        }
    }

    /// Tangent of half the vertical field of view for focal length `mm` at
    /// frame `aspect` (width / height), following the sensor fit.
    fn tan_half_y_for(&self, mm: f32, aspect: f32) -> f32 {
        let mm = mm.max(1e-3);
        let aspect = if aspect > 0.0 && aspect.is_finite() { aspect } else { 1.0 };
        match self.sensor_fit {
            SensorFit::Auto if aspect >= 1.0 => self.sensor_width * 0.5 / mm / aspect,
            SensorFit::Auto => self.sensor_width * 0.5 / mm,
            SensorFit::Horizontal => self.sensor_width * 0.5 / mm / aspect,
            SensorFit::Vertical => self.sensor_height * 0.5 / mm,
        }
    }

    /// Focal length in millimetres matching `fov_y` at frame `aspect`.
    pub fn focal_length(&self, aspect: f32) -> f32 {
        let t = (self.fov_y * 0.5).tan().max(1e-6);
        self.tan_half_y_for(1.0, aspect) / t
    }

    /// Set `fov_y` from a focal length in millimetres at frame `aspect`.
    pub fn set_focal_length(&mut self, mm: f32, aspect: f32) {
        self.fov_y = 2.0 * self.tan_half_y_for(mm, aspect).atan();
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

/// Area light outline (Blender's "Shape").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AreaShape {
    #[default]
    Square,
    Rectangle,
    Disk,
    Ellipse,
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
    /// Area light outline; `Square`/`Disk` use the kind's `size` both ways,
    /// `Rectangle`/`Ellipse` take `size_y` as the second dimension.
    pub shape: AreaShape,
    pub size_y: f32,
    /// Area light emission spread (radians, pi = Lambertian).
    pub spread: f32,
    /// Paths with more bounces than this do not see the light.
    pub max_bounces: u32,
    pub cast_shadow: bool,
    /// Multiple importance sampling: when off, only light sampling sees the
    /// light (rays never hit it).
    pub use_mis: bool,
}

impl Default for Light {
    fn default() -> Self {
        Light { kind: LightKind::Point, color: Vec3::ONE, power: 1000.0, radius: 0.1, shape: AreaShape::Square, size_y: 1.0, spread: core::f32::consts::PI, max_bounces: 1024, cast_shadow: true, use_mis: true }
    }
}

impl Light {
    /// Area light extent (x, y) in metres, after the shape.
    pub fn area_size(&self) -> (f32, f32) {
        let s = match self.kind {
            LightKind::Area { size } => size,
            _ => 0.0,
        };
        match self.shape {
            AreaShape::Square | AreaShape::Disk => (s, s),
            AreaShape::Rectangle | AreaShape::Ellipse => (s, self.size_y),
        }
    }
}

/// Which kinds of rays see an object (Blender's Object > Visibility > Ray Visibility).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayVisibility {
    pub camera: bool,
    pub diffuse: bool,
    pub glossy: bool,
    pub transmission: bool,
    pub volume_scatter: bool,
    pub shadow: bool,
}

impl Default for RayVisibility {
    fn default() -> Self {
        RayVisibility { camera: true, diffuse: true, glossy: true, transmission: true, volume_scatter: true, shadow: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectData {
    Empty,
    /// Index into [`Scene::meshes`]; several objects may share one mesh.
    Mesh(usize),
    Camera(Camera),
    Light(Light),
    /// Index into [`Scene::armatures`].
    Armature(usize),
    /// Index into [`Scene::lattices`].
    Lattice(usize),
}

/// Which vertex groups [`Scene::parent_with_armature`] creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmatureWeights {
    /// Plain armature parenting with the modifier, no groups.
    None,
    Empty,
    Envelope,
    Automatic,
}

/// How an object hangs off its parent (Blender's `partype`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParentType {
    /// Plain object parenting.
    #[default]
    Object,
    /// Armature parenting (`PARSKEL`): object parenting, deformation comes
    /// from an Armature modifier.
    Armature,
    /// Parented to the bone `Object::parent_bone` of an armature parent:
    /// follows the posed bone, at its tail (or root with Relative Parenting).
    Bone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub name: String,
    pub transform: Transform,
    pub parent: Option<usize>,
    pub parent_type: ParentType,
    /// Bone name for [`ParentType::Bone`].
    pub parent_bone: String,
    /// Object constraints, evaluated after parenting.
    pub constraints: Vec<crate::constraints::Constraint>,
    /// Armature objects: the pose (bones without an entry are at rest).
    pub pose: Vec<crate::armature::PoseBone>,
    pub data: ObjectData,
    /// Non-destructive modifier stack, applied top to bottom.
    pub modifiers: Vec<Modifier>,
    /// Material slots: indices into [`Scene::materials`]; a face's `material`
    /// picks a slot.
    pub materials: Vec<usize>,
    pub collection: usize,
    pub visible: bool,
    /// Ray visibility for the path tracer.
    pub visibility: RayVisibility,
    /// Render as a hole in the image (alpha 0).
    pub holdout: bool,
    /// Blender's object "Pass Index" (the Object Index render pass).
    pub pass_index: u32,
    /// The local transform one frame later, for motion blur (linear motion
    /// from `transform`); `None` means static.
    pub motion: Option<Transform>,
    /// Rigid body, rigid body constraint and force field settings.
    pub physics: crate::physics::ObjectPhysics,
}

impl Object {
    pub fn new(name: &str, data: ObjectData) -> Object {
        Object {
            name: name.into(),
            transform: Transform::default(),
            parent: None,
            parent_type: ParentType::Object,
            parent_bone: String::new(),
            constraints: Vec::new(),
            pose: Vec::new(),
            data,
            modifiers: Vec::new(),
            materials: Vec::new(),
            collection: 0,
            visible: true,
            visibility: RayVisibility::default(),
            holdout: false,
            pass_index: 0,
            motion: None,
            physics: crate::physics::ObjectPhysics::default(),
        }
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
    /// World shader node tree; when set it replaces colour/strength/HDRI.
    pub node_tree: Option<crate::nodes::NodeTree>,
    /// A homogeneous medium filling the whole world.
    pub volume: Option<VolumeSettings>,
}

impl Default for World {
    fn default() -> Self {
        World { color: Vec3::splat(0.05), strength: 1.0, hdri: None, node_tree: None, volume: None }
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
    /// The current frame, for modifiers that animate (Wave, Build).
    pub frame_current: i32,
    /// Node groups (Blender's node group data-blocks), shared by trees.
    pub node_groups: Vec<crate::nodes::NodeTree>,
    /// The compositing node tree (Compositor's Use Nodes).
    pub compositor: Option<crate::nodes::NodeTree>,
    /// Armature data, shared by armature objects like meshes.
    pub armatures: Vec<crate::armature::Armature>,
    pub armature_names: Vec<String>,
    pub lattices: Vec<crate::lattice::Lattice>,
    pub lattice_names: Vec<String>,
    /// Gravity, the rigid body world and the simulation caches.
    pub physics: crate::physics::ScenePhysics,
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
            frame_current: 1,
            node_groups: Vec::new(),
            compositor: None,
            armatures: Vec::new(),
            armature_names: Vec::new(),
            lattices: Vec::new(),
            lattice_names: Vec::new(),
            physics: crate::physics::ScenePhysics::default(),
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
        for ob in &mut self.objects {
            if let ObjectData::Camera(c) = &mut ob.data {
                c.dof.focus_object = c.dof.focus_object.and_then(|f| (f != idx).then(|| fix(f)));
            }
        }
        self.selected.retain(|&s| s != idx);
        for s in &mut self.selected {
            *s = fix(*s);
        }
        o
    }

    /// Object-to-world matrix at `time` frames after the current one, with
    /// each object in the parent chain moving linearly towards its
    /// [`Object::motion`] transform (extrapolating outside 0..1).
    pub fn world_matrix_at(&self, idx: usize, time: f32) -> Mat4 {
        let local = |i: usize| {
            let o = &self.objects[i];
            match o.motion {
                Some(m) if time != 0.0 => o.transform.lerp(&m, time).matrix(),
                _ => o.transform.matrix(),
            }
        };
        let mut m = local(idx);
        let mut p = self.objects[idx].parent;
        let mut guard = 0;
        while let Some(pi) = p {
            m = local(pi) * m;
            p = self.objects[pi].parent;
            guard += 1;
            if guard > self.objects.len() {
                break;
            }
        }
        m
    }

    /// True when the object or one of its parents moves (has a
    /// [`Object::motion`] different from its transform).
    pub fn has_motion(&self, idx: usize) -> bool {
        let mut i = Some(idx);
        let mut guard = 0;
        while let Some(c) = i {
            if self.objects[c].motion.is_some_and(|m| m != self.objects[c].transform) {
                return true;
            }
            i = self.objects[c].parent;
            guard += 1;
            if guard > self.objects.len() {
                break;
            }
        }
        false
    }

    /// Object-to-world matrix through the parent chain.
    ///
    /// Bone parents follow the posed bone and object constraints are applied
    /// after parenting (see [`crate::constraints`]).
    pub fn world_matrix(&self, idx: usize) -> Mat4 {
        self.world_matrix_depth(idx, 0)
    }

    pub fn add_armature(&mut self, name: &str, a: crate::armature::Armature) -> usize {
        let n = unique_name(name, |n| self.armature_names.iter().any(|x| x == n));
        self.armatures.push(a);
        self.armature_names.push(n);
        self.armatures.len() - 1
    }

    pub fn add_armature_object(&mut self, name: &str, a: crate::armature::Armature) -> usize {
        let ai = self.add_armature(name, a);
        self.add_object(Object::new(name, ObjectData::Armature(ai)))
    }

    pub fn add_lattice(&mut self, name: &str, l: crate::lattice::Lattice) -> usize {
        let n = unique_name(name, |n| self.lattice_names.iter().any(|x| x == n));
        self.lattices.push(l);
        self.lattice_names.push(n);
        self.lattices.len() - 1
    }

    pub fn add_lattice_object(&mut self, name: &str, l: crate::lattice::Lattice) -> usize {
        let li = self.add_lattice(name, l);
        self.add_object(Object::new(name, ObjectData::Lattice(li)))
    }

    /// The armature data of object `idx`, if it is an armature.
    pub fn armature_of(&self, idx: usize) -> Option<&crate::armature::Armature> {
        match self.objects.get(idx)?.data {
            ObjectData::Armature(a) => self.armatures.get(a),
            _ => None,
        }
    }

    /// Parent `child` to bone `bone` of armature object `armature` (Ctrl+P
    /// > Bone). With `keep_transform` the child stays where it is.
    pub fn parent_to_bone(&mut self, child: usize, armature: usize, bone: &str, keep_transform: bool) -> bool {
        if child == armature || self.armature_of(armature).and_then(|a| a.find(bone)).is_none() {
            return false;
        }
        let world = self.world_matrix(child);
        let o = &mut self.objects[child];
        o.parent = Some(armature);
        o.parent_type = ParentType::Bone;
        o.parent_bone = bone.to_string();
        if keep_transform {
            let p = self.parent_matrix_depth(child, 0).unwrap_or(Mat4::IDENTITY);
            self.objects[child].transform = Transform::from_matrix(&(p.inverse() * world));
        }
        true
    }

    /// Ctrl+P > "With Empty Groups" / "With Envelope Weights" / "With
    /// Automatic Weights": parent mesh object `child` to armature object
    /// `armature` (keeping its transform), add an Armature modifier and
    /// create the vertex groups.
    pub fn parent_with_armature(&mut self, child: usize, armature: usize, weights: ArmatureWeights) -> bool {
        let ObjectData::Mesh(mi) = self.objects[child].data else { return false };
        let Some(arm) = self.armature_of(armature).cloned() else { return false };
        if child == armature {
            return false;
        }
        let world = self.world_matrix(child);
        let arm_world = self.world_matrix(armature);
        let arm_to_mesh = world.inverse() * arm_world;
        {
            let mesh = &mut self.meshes[mi];
            match weights {
                ArmatureWeights::Empty => crate::armature::empty_groups(mesh, &arm),
                ArmatureWeights::Envelope => crate::armature::envelope_weights(mesh, &arm, &arm_to_mesh),
                ArmatureWeights::Automatic => crate::armature::auto_weights(mesh, &arm, &arm_to_mesh),
                ArmatureWeights::None => {}
            }
        }
        let name = self.objects[armature].name.clone();
        let o = &mut self.objects[child];
        o.parent = Some(armature);
        o.parent_type = ParentType::Armature;
        o.parent_bone.clear();
        o.transform = Transform::from_matrix(&(arm_world.inverse() * world));
        if !o.modifiers.iter().any(|m| matches!(m, Modifier::Armature { object, .. } if *object == name)) {
            let mut md = Modifier::with_defaults("Armature").expect("armature modifier");
            if let Modifier::Armature { object, .. } = &mut md {
                *object = name;
            }
            o.modifiers.insert(0, md);
        }
        true
    }

    /// The object's mesh after its modifier stack, in object space.
    pub fn evaluated_mesh(&self, idx: usize) -> Option<Mesh> {
        self.evaluated_mesh_depth(idx, 0)
    }

    /// Apply `stack` to `mesh` as if it were object `idx`'s stack: at the
    /// current frame, with other objects (Boolean, Shrinkwrap) resolved by
    /// name to their evaluated meshes in this object's space.
    pub fn apply_modifiers(&self, idx: usize, mesh: &Mesh, stack: &[Modifier]) -> Mesh {
        self.apply_modifiers_depth(idx, mesh, stack, 0)
    }

    /// Objects referring to each other (A booleans B, B shrinkwraps to A)
    /// are cut off at this depth instead of recursing forever.
    const MAX_MODIFIER_DEPTH: u32 = 4;

    pub(crate) fn evaluated_mesh_depth(&self, idx: usize, depth: u32) -> Option<Mesh> {
        match self.objects[idx].data {
            ObjectData::Mesh(mi) => Some(self.apply_modifiers_depth(idx, &self.meshes[mi], &self.objects[idx].modifiers, depth)),
            _ => None,
        }
    }

    fn apply_modifiers_depth(&self, idx: usize, mesh: &Mesh, stack: &[Modifier], depth: u32) -> Mesh {
        let lookup = |name: &str| -> Option<Mesh> {
            let j = self.find_object(name)?;
            if j == idx || depth >= Self::MAX_MODIFIER_DEPTH {
                return None;
            }
            let mut m = self.evaluated_mesh_depth(j, depth + 1)?;
            m.transform(&(self.world_matrix(idx).inverse() * self.world_matrix(j)));
            Some(m)
        };
        // Shape keys come first (Blender evaluates the key block before the
        // modifier stack); Corrective Smooth's rest shape stays the Basis.
        let keyed;
        let (input, rest) = if mesh.shape_keys.len() > 1 {
            let mut m = mesh.clone();
            m.positions = mesh.evaluated_shape();
            m.shape_keys.clear();
            keyed = m;
            (&keyed, Some(mesh.positions.as_slice()))
        } else {
            (mesh, None)
        };
        let cx = crate::modifiers::EvalContext { frame: self.frame_current as f32, object: Some(&lookup), rest, scene: Some(self), owner: idx, depth };
        crate::modifiers::apply_stack_with(input, stack, &cx)
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
