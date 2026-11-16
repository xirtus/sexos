// SPDX-License-Identifier: GPL-3.0-only
//! Scene data for the `.blend` writer (see `write.rs` for the format).
use super::write::{Rec, W};
use super::write_rig::StackExtras;
use super::BlendAction;
use crate::armature::{Armature, InheritScale, PoseBone, RotationMode};
use crate::math::EulerOrder;
use crate::nodes::{NodeTree, Prop, Value};
use crate::mesh::Mesh;
use crate::modifiers::Modifier;
use crate::physics::PhysicsModifier;
use crate::scene::{AreaShape, Camera, Light, LightKind, Material, ObjectData, PanoramaType, ParentType, Scene, SensorFit, World};
use std::collections::{HashMap, HashSet};

// eCustomDataType.
const CD_MDEFORMVERT: i64 = 2;
const CD_PROP_INT32: i64 = 11;
const CD_PROP_INT32_2D: i64 = 46;
const CD_PROP_FLOAT3: i64 = 48;
const CD_PROP_FLOAT2: i64 = 49;
const CD_PROP_BOOL: i64 = 50;
const CD_NUMTYPES: usize = 53;
/// `LIB_EMBEDDED_DATA`.
pub(super) const LIB_EMBEDDED_DATA: i64 = 1 << 10;
/// `ID_OB` (`MAKE_ID2('O', 'B')`), an action's `idroot`.
const ID_OB: i64 = b'O' as i64 | (b'B' as i64) << 8;

/// Options for [`export_blend`].
#[derive(Clone, Copy, Debug)]
pub struct BlendExport {
    /// Render resolution (the camera lens is derived from the field of view
    /// with it, as Blender's sensor fit does).
    pub width: u32,
    pub height: u32,
}

impl Default for BlendExport {
    fn default() -> Self {
        BlendExport { width: 1920, height: 1080 }
    }
}

/// Unique, length-limited ID names per ID type.
#[derive(Default)]
pub(super) struct Names(HashMap<&'static str, HashSet<String>>);

impl Names {
    pub(super) fn unique(&mut self, code: &'static str, name: &str) -> String {
        let fit = |s: &str| {
            let mut n = s.len().min(63);
            while !s.is_char_boundary(n) {
                n -= 1;
            }
            s[..n].to_string()
        };
        let base = fit(if name.is_empty() { "Unnamed" } else { name });
        let used = self.0.entry(code).or_default();
        let mut n = base.clone();
        let mut k = 1;
        while used.contains(&n) {
            n = fit(&format!("{}.{k:03}", &base[..base.len().min(55)]));
            k += 1;
        }
        used.insert(n.clone());
        n
    }
}

/// Addresses of every ID, assigned before any block is written so IDs can
/// point at each other in any order.
pub(super) struct Ids {
    pub(super) scene: u64,
    pub(super) objects: Vec<u64>,
    pub(super) obdata: Vec<u64>,
    pub(super) meshes: Vec<u64>,
    pub(super) materials: Vec<u64>,
    pub(super) images: Vec<u64>,
    pub(super) armatures: Vec<u64>,
    pub(super) collections: Vec<u64>,
    pub(super) groups: Vec<u64>,
    pub(super) world: u64,
    pub(super) actions: Vec<u64>,
    pub(super) lattices: Vec<u64>,
    pub(super) gpencils: Vec<u64>,
    /// Per Grease Pencil data: its materials (`MA` IDs with `gp_style`).
    pub(super) gp_materials: Vec<Vec<u64>>,
    /// Per mesh: its shape key `Key` (0 without shape keys).
    pub(super) keys: Vec<u64>,
    /// `ParticleSettings` IDs by (object, modifier index).
    pub(super) particles: HashMap<(usize, usize), u64>,
    pub(super) object_by_name: HashMap<String, u64>,
    pub(super) collection_by_name: HashMap<String, u64>,
}

pub(super) fn euler_mode(o: EulerOrder) -> i64 {
    match o {
        EulerOrder::XYZ => 1,
        EulerOrder::XZY => 2,
        EulerOrder::YXZ => 3,
        EulerOrder::YZX => 4,
        EulerOrder::ZXY => 5,
        EulerOrder::ZYX => 6,
    }
}

fn inherit_scale(s: InheritScale) -> i64 {
    match s {
        InheritScale::Full => 0,
        InheritScale::FixShear => 1,
        InheritScale::Average => 2,
        InheritScale::None => 3,
        InheritScale::NoneLegacy => 4,
        InheritScale::Aligned => 5,
    }
}

impl W {
    /// An ID struct with its name and one user.
    pub(super) fn id(&mut self, sname: &str, code: &str, name: &str) -> Rec {
        let mut r = self.rec(sname);
        self.s(&mut r, "id.name", &format!("{code}{name}"));
        self.i(&mut r, "id.us", 1);
        r
    }

    /// A `ListBase` of the blocks at `addrs` (already linked by next/prev).
    pub(super) fn listbase(&mut self, r: &mut Rec, path: &str, addrs: &[u64]) {
        self.p(r, &format!("{path}.first"), addrs.first().copied().unwrap_or(0));
        self.p(r, &format!("{path}.last"), addrs.last().copied().unwrap_or(0));
    }

    /// Link `recs` (which start with `next`/`prev`) and write each as its own
    /// block at `addrs`.
    pub(super) fn chain(&mut self, mut recs: Vec<Rec>, addrs: &[u64]) {
        for (k, r) in recs.iter_mut().enumerate() {
            let next = addrs.get(k + 1).copied().unwrap_or(0);
            let prev = if k > 0 { addrs[k - 1] } else { 0 };
            self.p(r, "next", next);
            self.p(r, "prev", prev);
        }
        for (r, a) in recs.iter().zip(addrs) {
            self.put(b"DATA", *a, std::slice::from_ref(r));
        }
    }

    // ------------------------------------------------------------- Scene

    pub(super) fn scene(&mut self, scene: &Scene, ids: &Ids, names: &mut Names, opts: &BlendExport) -> (u64, String) {
        self.start_id();
        let name = names.unique("SC", &scene.name);
        let mut sc = self.id("Scene", "SC", &name);
        self.p(&mut sc, "camera", scene.active_camera.and_then(|i| ids.objects.get(i).copied()).unwrap_or(0));
        self.p(&mut sc, "world", ids.world);
        self.i(&mut sc, "r.xsch", opts.width.max(4) as i64);
        self.i(&mut sc, "r.ysch", opts.height.max(4) as i64);
        self.i(&mut sc, "r.sfra", scene.frame_start as i64);
        self.i(&mut sc, "r.efra", scene.frame_end as i64);
        self.i(&mut sc, "r.cfra", scene.frame_current as i64);
        // fps = frs_sec / frs_sec_base, frs_sec a short.
        let fps = if scene.fps.is_finite() && scene.fps > 0.0 { scene.fps } else { 24.0 };
        let whole = fps.round().clamp(1.0, 32767.0);
        self.i(&mut sc, "r.frs_sec", whole as i64);
        self.f(&mut sc, "r.frs_sec_base", &[whole / fps]);
        self.s(&mut sc, "view_settings.view_transform", "Standard");
        self.s(&mut sc, "view_settings.look", "None");
        self.f(&mut sc, "view_settings.gamma", &[1.0]);
        self.s(&mut sc, "display_settings.display_device", "sRGB");
        self.s(&mut sc, "sequencer_colorspace_settings.name", "sRGB");
        self.f(&mut sc, "unit.scale_length", &[1.0]);
        self.i(&mut sc, "unit.system", 1); // USER_UNIT_METRIC

        // Embedded data, written after the scene block.
        let ts_addr = self.addr();
        self.p(&mut sc, "toolsettings", ts_addr);
        let master = self.addr();
        self.p(&mut sc, "master_collection", master);
        let vl = self.addr();
        self.listbase(&mut sc, "view_layers", &[vl]);
        let tree = scene.compositor.as_ref().map(|_| self.addr()).unwrap_or(0);
        self.p(&mut sc, "nodetree", tree);
        self.i(&mut sc, "use_nodes", scene.compositor.is_some() as i64);
        self.scene_physics(&mut sc, &scene.physics, ids);
        self.put(b"SC", ids.scene, &[sc]);

        let ts = self.rec("ToolSettings");
        self.put(b"DATA", ts_addr, &[ts]);

        // The master collection: objects in collection 0, child collections.
        let mut mc = self.rec("Collection");
        self.s(&mut mc, "id.name", "GRScene Collection");
        self.i(&mut mc, "id.flag", LIB_EMBEDDED_DATA);
        self.i(&mut mc, "flag", 1 << 5); // COLLECTION_IS_MASTER
        self.collection_contents(&mut mc, scene, ids, 0);
        self.put(b"DATA", master, &[mc]);
        self.collection_lists(scene, ids, 0);

        // View layer and its layer collection tree.
        let mut v = self.rec("ViewLayer");
        self.s(&mut v, "name", "ViewLayer");
        self.i(&mut v, "flag", 1 | 1 << 2); // VIEW_LAYER_RENDER | VIEW_LAYER_FREESTYLE
        self.i(&mut v, "layflag", (1 << 15) - 1);
        self.i(&mut v, "passflag", 1);
        self.f(&mut v, "pass_alpha_threshold", &[0.5]);
        self.i(&mut v, "cryptomatte_levels", 6);
        let lc = self.layer_collection(scene, 0, master, ids);
        self.listbase(&mut v, "layer_collections", &[lc]);
        self.p(&mut v, "active_collection", lc);
        self.put(b"DATA", vl, &[v]);

        if let Some(t) = &scene.compositor {
            self.node_tree_at(tree, b"DATA", t, "Compositing Nodetree", true, ids);
        }
        (vl, name)
    }

    /// `LayerCollection` for collection `ci` (and its children), returning
    /// its address. Must be written in the scene's data.
    fn layer_collection(&mut self, scene: &Scene, ci: usize, col_addr: u64, ids: &Ids) -> u64 {
        let a = self.addr();
        let kids: Vec<usize> = (0..scene.collections.len()).filter(|&k| k != 0 && scene.collections[k].parent.unwrap_or(0) == ci && k != ci).collect();
        let kid_addrs: Vec<u64> = kids.iter().map(|&k| self.layer_collection(scene, k, ids.collections[k], ids)).collect();
        let mut r = self.rec("LayerCollection");
        self.p(&mut r, "collection", col_addr);
        self.listbase(&mut r, "layer_collections", &kid_addrs);
        // Sibling links are set by `link_layer_collections` once all exist.
        self.put(b"DATA", a, &[r]);
        self.link_siblings(&kid_addrs);
        a
    }

    /// Fix next/prev of already-written sibling blocks.
    fn link_siblings(&mut self, addrs: &[u64]) {
        for (k, &a) in addrs.iter().enumerate() {
            let next = addrs.get(k + 1).copied().unwrap_or(0);
            let prev = if k > 0 { addrs[k - 1] } else { 0 };
            self.patch_ptr(a, "next", next);
            self.patch_ptr(a, "prev", prev);
        }
    }

    /// The child collections of `ci` and objects in it.
    fn collection_contents(&mut self, r: &mut Rec, scene: &Scene, ids: &Ids, ci: usize) {
        let obs: Vec<usize> = (0..scene.objects.len()).filter(|&o| scene.objects[o].collection == ci || (ci == 0 && scene.objects[o].collection >= scene.collections.len())).collect();
        let kids: Vec<usize> = (1..scene.collections.len()).filter(|&k| scene.collections[k].parent.unwrap_or(0) == ci && k != ci).collect();
        let co: Vec<u64> = obs.iter().map(|_| self.addr()).collect();
        let cc: Vec<u64> = kids.iter().map(|_| self.addr()).collect();
        self.listbase(r, "gobject", &co);
        self.listbase(r, "children", &cc);
        // Remember for `collection_lists`.
        self.pending.push((co, obs.iter().map(|&o| ids.objects[o]).collect(), cc, kids.iter().map(|&k| ids.collections[k]).collect()));
    }

    /// Write the `CollectionObject`/`CollectionChild` lists queued by the last
    /// `collection_contents`.
    fn collection_lists(&mut self, _scene: &Scene, _ids: &Ids, _ci: usize) {
        let Some((co, obs, cc, kids)) = self.pending.pop() else { return };
        let recs: Vec<Rec> = obs
            .iter()
            .map(|&o| {
                let mut r = self.rec("CollectionObject");
                self.p(&mut r, "ob", o);
                r
            })
            .collect();
        self.chain(recs, &co);
        let recs: Vec<Rec> = kids
            .iter()
            .map(|&k| {
                let mut r = self.rec("CollectionChild");
                self.p(&mut r, "collection", k);
                r
            })
            .collect();
        self.chain(recs, &cc);
    }

    /// A child collection ID (`GR`).
    pub(super) fn collection(&mut self, scene: &Scene, ids: &Ids, names: &mut Names, ci: usize) {
        self.start_id();
        let c = &scene.collections[ci];
        let name = names.unique("GR", &c.name);
        let mut r = self.id("Collection", "GR", &name);
        if !c.visible {
            self.i(&mut r, "flag", 1); // COLLECTION_HIDE_VIEWPORT
        }
        self.collection_contents(&mut r, scene, ids, ci);
        self.put(b"GR", ids.collections[ci], &[r]);
        self.collection_lists(scene, ids, ci);
    }

    // ------------------------------------------------------------ Objects

    pub(super) fn object(&mut self, scene: &Scene, ids: &Ids, names: &mut Names, oi: usize, action: Option<u64>) {
        self.start_id();
        let o = &scene.objects[oi];
        let name = names.unique("OB", &o.name);
        let mut r = self.id("Object", "OB", &name);
        let (ty, data) = match &o.data {
            // Empties keep the DNA defaults: plain axes of size 1.
            ObjectData::Empty => (0, 0),
            ObjectData::Lattice(l) => (22, ids.lattices.get(*l).copied().unwrap_or(0)),
            ObjectData::GreasePencil(g) => (26, ids.gpencils.get(*g).copied().unwrap_or(0)),
            ObjectData::Mesh(m) => (1, ids.meshes.get(*m).copied().unwrap_or(0)),
            ObjectData::Light(_) => (10, ids.obdata[oi]),
            ObjectData::Camera(_) => (11, ids.obdata[oi]),
            ObjectData::Armature(a) => (25, ids.armatures.get(*a).copied().unwrap_or(0)),
        };
        self.i(&mut r, "type", ty);
        self.p(&mut r, "data", data);
        if let Some(p) = o.parent.and_then(|p| ids.objects.get(p).copied()) {
            self.p(&mut r, "parent", p);
            match o.parent_type {
                ParentType::Object => {}
                ParentType::Armature => self.i(&mut r, "partype", 4),
                ParentType::Bone => {
                    self.i(&mut r, "partype", 7);
                    self.s(&mut r, "parsubstr", &o.parent_bone);
                }
            }
        }
        let t = &o.transform;
        self.f(&mut r, "loc", &t.location.to_array());
        self.f(&mut r, "rot", &t.rotation.normalize().to_euler(EulerOrder::XYZ));
        self.f(&mut r, "size", &t.scale.to_array());
        self.i(&mut r, "rotmode", 1);
        // visibility_flag: OB_HIDE_VIEWPORT 1, OB_HIDE_CAMERA 1<<3 .. SHADOW 1<<8, OB_HOLDOUT 1<<9.
        let v = &o.visibility;
        let mut vis = 0;
        for (bit, on) in [(3, v.camera), (4, v.diffuse), (5, v.glossy), (6, v.transmission), (7, v.volume_scatter), (8, v.shadow)] {
            if !on {
                vis |= 1 << bit;
            }
        }
        if o.holdout {
            vis |= 1 << 9;
        }
        if !o.visible {
            vis |= 1;
        }
        self.i(&mut r, "restrictflag", vis);
        self.i(&mut r, "index", o.pass_index.min(32767) as i64);
        // Material slots linked to the data, as Blender's default (Grease
        // Pencil objects: the data's own materials).
        let gp_mats = match o.data {
            ObjectData::GreasePencil(g) => ids.gp_materials.get(g).cloned(),
            _ => None,
        };
        let n = gp_mats.as_ref().map_or(o.materials.len(), |m| m.len());
        if n > 0 {
            let mats = self.ptrs(&gp_mats.unwrap_or_else(|| vec![0u64; n]));
            let bits = self.raw(vec![0u8; n]);
            self.p(&mut r, "mat", mats);
            self.p(&mut r, "matbits", bits);
            self.i(&mut r, "totcol", n as i64);
            self.i(&mut r, "actcol", 1);
        }
        let pose = matches!(o.data, ObjectData::Armature(_)).then(|| self.addr());
        if let Some(pa) = pose {
            self.p(&mut r, "pose", pa);
        }
        let adt = action.map(|_| self.addr());
        if let Some(a) = adt {
            self.p(&mut r, "adt", a);
        }
        // Modifiers, constraints and physics (their blocks follow the object's).
        let mut extras = StackExtras::default();
        self.modifier_stack(scene, oi, ids, &mut extras);
        self.listbase(&mut r, "modifiers", &extras.modifiers);
        let cons = self.constraints(&o.constraints, ids);
        self.listbase(&mut r, "constraints", &cons);
        let collision = o.modifiers.iter().find_map(|m| match m {
            Modifier::Physics(PhysicsModifier::Collision(c)) => Some(c),
            _ => None,
        });
        self.object_physics(&mut r, &o.physics, collision, ids);
        if let Some(sb) = o.modifiers.iter().find_map(|m| match m {
            Modifier::Physics(PhysicsModifier::SoftBody(s)) => Some(s),
            _ => None,
        }) {
            self.soft_body(&mut r, sb, ids);
        }
        self.listbase(&mut r, "particlesystem", &extras.particle_systems);
        // Show Emitter (render): OB_DUPLI_FLAG_VIEWPORT 1, OB_DUPLI_FLAG_RENDER 2.
        let emitters: Vec<bool> = o.modifiers.iter().filter_map(|m| match m {
            Modifier::Physics(PhysicsModifier::ParticleSystem(p)) => Some(p.settings.use_render_emitter),
            _ => None,
        }).collect();
        if !emitters.is_empty() {
            self.i(&mut r, "duplicator_visibility_flag", 1 | (emitters.iter().all(|e| *e) as i64) << 1);
        }
        let psys: Vec<Rec> = extras.pending_psys.drain(..).map(|(_, r)| r).collect();
        self.chain(psys, &extras.particle_systems);
        if let ObjectData::Mesh(m) = o.data {
            if scene.meshes.get(m).is_some_and(|m| !m.shape_keys.is_empty()) {
                self.i(&mut r, "shapenr", 1);
            }
        }
        self.put(b"OB", ids.objects[oi], &[r]);
        if let (Some(pa), ObjectData::Armature(ai)) = (pose, &o.data) {
            self.pose(pa, &o.pose, scene.armatures.get(*ai), ids);
        }
        if let (Some(a), Some(act)) = (adt, action) {
            let mut ad = self.rec("AnimData");
            self.p(&mut ad, "action", act);
            self.f(&mut ad, "act_influence", &[1.0]);
            self.put(b"DATA", a, &[ad]);
        }
    }

    fn pose(&mut self, addr: u64, pose: &[PoseBone], arm: Option<&Armature>, ids: &Ids) {
        // One channel per bone: posed values where the object has them.
        let names: Vec<String> = match arm {
            Some(a) => a.bones.iter().map(|b| b.name.clone()).collect(),
            None => pose.iter().map(|p| p.name.clone()).collect(),
        };
        let addrs: Vec<u64> = names.iter().map(|_| self.addr()).collect();
        let mut p = self.rec("bPose");
        self.listbase(&mut p, "chanbase", &addrs);
        self.i(&mut p, "flag", 1); // POSE_RECALC: Blender links channels to bones on load
        self.i(&mut p, "iksolver", 0);
        self.put(b"DATA", addr, &[p]);
        let recs: Vec<Rec> = names
            .iter()
            .map(|n| {
                let pb = pose.iter().find(|p| &p.name == n).cloned().unwrap_or_else(|| PoseBone::new(n));
                let mut c = self.pose_channel(&pb);
                let cons = self.constraints(&pb.constraints, ids);
                self.listbase(&mut c, "constraints", &cons);
                c
            })
            .collect();
        self.chain(recs, &addrs);
    }

    fn pose_channel(&mut self, pb: &PoseBone) -> Rec {
        let mut c = self.rec("bPoseChannel");
        self.s(&mut c, "name", &pb.name);
        self.f(&mut c, "loc", &pb.location.to_array());
        self.f(&mut c, "size", &pb.scale.to_array());
        self.f(&mut c, "eul", &pb.rotation_euler.to_array());
        let q = pb.rotation_quaternion;
        self.f(&mut c, "quat", &[q.w, q.x, q.y, q.z]);
        let aa = pb.rotation_axis_angle;
        self.f(&mut c, "rotAxis", &[aa[1], aa[2], aa[3]]);
        self.f(&mut c, "rotAngle", &[aa[0]]);
        let mode = match pb.rotation_mode {
            RotationMode::Quaternion => 0,
            RotationMode::AxisAngle => -1,
            RotationMode::Euler(o) => euler_mode(o),
        };
        self.i(&mut c, "rotmode", mode);
        let ik = &pb.ik;
        let mut flag = 0;
        for k in 0..3 {
            if ik.lock[k] {
                flag |= 1 << k;
            }
            if ik.use_limit[k] {
                flag |= 8 << k;
            }
        }
        self.i(&mut c, "ikflag", flag);
        self.f(&mut c, "limitmin", &ik.min);
        self.f(&mut c, "limitmax", &ik.max);
        self.f(&mut c, "stiffness", &ik.stiffness);
        self.f(&mut c, "ikstretch", &[ik.stretch]);
        // BKE_pose_channel_ensure's values.
        self.f(&mut c, "ikrotweight", &[1.0]);
        self.f(&mut c, "custom_scale", &[1.0]);
        self.f(&mut c, "custom_scale_xyz", &[1.0, 1.0, 1.0]);
        self.f(&mut c, "custom_shape_wire_width", &[1.0]);
        self.f(&mut c, "scale_in_y", &[1.0]);
        self.f(&mut c, "scale_out_y", &[1.0]);
        self.f(&mut c, "scale_in", &[1.0, 1.0, 1.0]);
        self.f(&mut c, "scale_out", &[1.0, 1.0, 1.0]);
        self.i(&mut c, "protectflag", 1 << 4); // OB_LOCK_ROT4D
        c
    }

    // ------------------------------------------------------------- Meshes

    pub(super) fn mesh(&mut self, addr: u64, name: &str, m: &Mesh, slots: &[u64], key: u64) {
        self.start_id();
        let mut r = self.id("Mesh", "ME", name);
        let nv = m.positions.len();
        // Edges: face edges in corner order, then loose edges.
        let mut edge_of: HashMap<(u32, u32), usize> = HashMap::new();
        let mut edges: Vec<[u32; 2]> = Vec::new();
        let mut add_edge = |a: u32, b: u32, edges: &mut Vec<[u32; 2]>| -> usize {
            *edge_of.entry((a.min(b), a.max(b))).or_insert_with(|| {
                edges.push([a, b]);
                edges.len() - 1
            })
        };
        let mut offsets = vec![0i32];
        let mut corner_vert: Vec<i32> = Vec::new();
        let mut corner_edge: Vec<i32> = Vec::new();
        let mut faces = Vec::new();
        for f in &m.faces {
            let n = f.verts.len();
            if n < 3 || f.verts.iter().any(|&v| v as usize >= nv) {
                continue;
            }
            for k in 0..n {
                let (a, b) = (f.verts[k], f.verts[(k + 1) % n]);
                corner_vert.push(a as i32);
                corner_edge.push(add_edge(a, b, &mut edges) as i32);
            }
            offsets.push(corner_vert.len() as i32);
            faces.push(f);
        }
        for &[a, b] in &m.loose_edges {
            if a != b && (a as usize) < nv && (b as usize) < nv {
                add_edge(a, b, &mut edges);
            }
        }
        let (ne, nf, nl) = (edges.len(), faces.len(), corner_vert.len());
        self.i(&mut r, "totvert", nv as i64);
        self.i(&mut r, "totedge", ne as i64);
        self.i(&mut r, "totpoly", nf as i64);
        self.i(&mut r, "totloop", nl as i64);
        self.f(&mut r, "smoothresh", &[std::f32::consts::PI / 6.0]);

        // Layer data blocks are written after the mesh block, so collect
        // them first: (customdata path, layers).
        let mut lay: Vec<(&str, Vec<(i64, String, Vec<u8>, &str)>)> = Vec::new();
        let f32s = |xs: &mut dyn Iterator<Item = f32>| xs.flat_map(f32::to_le_bytes).collect::<Vec<u8>>();
        let i32s = |xs: &[i32]| xs.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let pos = f32s(&mut m.positions.iter().flat_map(|p| p.to_array()));
        let mut vlayers = vec![(CD_PROP_FLOAT3, "position".to_string(), pos, "vec3f")];
        if !m.weights.is_empty() && !m.vertex_groups.is_empty() {
            vlayers.push((CD_MDEFORMVERT, String::new(), Vec::new(), "MDeformVert"));
        }
        lay.push(("vdata", vlayers));
        let ev: Vec<i32> = edges.iter().flat_map(|e| [e[0] as i32, e[1] as i32]).collect();
        let mut elayers = vec![(CD_PROP_INT32_2D, ".edge_verts".to_string(), i32s(&ev), "vec2i")];
        if !m.sharp_edges.is_empty() {
            let mut sharp = vec![0u8; ne];
            for &(a, b) in &m.sharp_edges {
                if let Some(&e) = edge_of.get(&(a.min(b), a.max(b))) {
                    sharp[e] = 1;
                }
            }
            elayers.push((CD_PROP_BOOL, "sharp_edge".to_string(), sharp, "MBoolProperty"));
        }
        lay.push(("edata", elayers));
        let mut players = Vec::new();
        if faces.iter().any(|f| f.material != 0) {
            let mi: Vec<i32> = faces.iter().map(|f| f.material as i32).collect();
            players.push((CD_PROP_INT32, "material_index".to_string(), i32s(&mi), "MIntProperty"));
        }
        if faces.iter().any(|f| !f.smooth) {
            players.push((CD_PROP_BOOL, "sharp_face".to_string(), faces.iter().map(|f| (!f.smooth) as u8).collect(), "MBoolProperty"));
        }
        lay.push(("pdata", players));
        let mut llayers = vec![(CD_PROP_INT32, ".corner_vert".to_string(), i32s(&corner_vert), "MIntProperty"), (CD_PROP_INT32, ".corner_edge".to_string(), i32s(&corner_edge), "MIntProperty")];
        if faces.iter().any(|f| !f.uvs.is_empty()) {
            let uv = f32s(&mut faces.iter().flat_map(|f| (0..f.verts.len()).flat_map(move |k| f.uvs.get(k).map_or([0.0, 0.0], |u| [u.x, u.y]))));
            llayers.push((CD_PROP_FLOAT2, "UVMap".to_string(), uv, "vec2f"));
        }
        lay.push(("ldata", llayers));

        // Addresses for everything the mesh points at.
        let mut layer_arrays = Vec::new();
        for (path, ls) in &lay {
            let a = if ls.is_empty() { 0 } else { self.addr() };
            self.p(&mut r, &format!("{path}.layers"), a);
            self.i(&mut r, &format!("{path}.totlayer"), ls.len() as i64);
            self.i(&mut r, &format!("{path}.maxlayer"), ls.len() as i64);
            let mut map = [-1i64; CD_NUMTYPES];
            for (k, l) in ls.iter().enumerate() {
                if let Some(slot) = map.get_mut(l.0 as usize) {
                    if *slot < 0 {
                        *slot = k as i64;
                    }
                }
            }
            self.set(&mut r, &format!("{path}.typemap"), super::write::V::I(&map));
            layer_arrays.push(a);
        }
        let offs_addr = if nf > 0 { self.addr() } else { 0 };
        self.p(&mut r, "poly_offset_indices", offs_addr);
        let groups: Vec<u64> = m.vertex_groups.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "vertex_group_names", &groups);
        if !groups.is_empty() {
            self.i(&mut r, "vertex_group_active_index", 1);
        }
        let mat_addr = if slots.is_empty() { 0 } else { self.addr() };
        self.p(&mut r, "mat", mat_addr);
        self.i(&mut r, "totcol", slots.len() as i64);
        self.p(&mut r, "key", key);
        self.put(b"ME", addr, &[r]);

        // Data blocks.
        if mat_addr != 0 {
            let bytes = slots.iter().flat_map(|p| p.to_le_bytes()).collect();
            self.put_raw_at(mat_addr, bytes);
        }
        for ((_, ls), arr) in lay.iter().zip(&layer_arrays) {
            if ls.is_empty() {
                continue;
            }
            let mut recs = Vec::new();
            let mut datas = Vec::new();
            for (ty, name, bytes, sname) in ls {
                let mut l = self.rec("CustomDataLayer");
                self.i(&mut l, "type", *ty);
                self.s(&mut l, "name", name);
                let d = if *ty == CD_MDEFORMVERT { self.addr() } else { self.addr() };
                self.p(&mut l, "data", d);
                recs.push(l);
                datas.push((d, *ty, bytes, *sname));
            }
            self.put(b"DATA", *arr, &recs);
            for (d, ty, bytes, sname) in datas {
                if ty == CD_MDEFORMVERT {
                    self.deform_verts(d, m, nv);
                } else {
                    let size = self.size(sname).max(1);
                    let n = bytes.len() / size;
                    self.put_struct_bytes(d, sname, n, bytes.clone());
                }
            }
        }
        if offs_addr != 0 {
            self.put_raw_at(offs_addr, i32s(&offsets));
        }
        let recs: Vec<Rec> = m
            .vertex_groups
            .iter()
            .map(|g| {
                let mut r = self.rec("bDeformGroup");
                self.s(&mut r, "name", g);
                r
            })
            .collect();
        self.chain(recs, &groups);
    }

    fn deform_verts(&mut self, addr: u64, m: &Mesh, nv: usize) {
        let ng = m.vertex_groups.len() as u32;
        let mut recs = Vec::with_capacity(nv);
        let mut dws = Vec::new();
        for v in 0..nv {
            let ws: Vec<(u32, f32)> = m.weights.get(v).map(|w| w.iter().copied().filter(|(g, _)| *g < ng).collect()).unwrap_or_default();
            let mut r = self.rec("MDeformVert");
            if !ws.is_empty() {
                let a = self.addr();
                self.p(&mut r, "dw", a);
                self.i(&mut r, "totweight", ws.len() as i64);
                dws.push((a, ws));
            }
            recs.push(r);
        }
        self.put(b"DATA", addr, &recs);
        for (a, ws) in dws {
            let recs: Vec<Rec> = ws
                .iter()
                .map(|(g, w)| {
                    let mut r = self.rec("MDeformWeight");
                    self.i(&mut r, "def_nr", *g as i64);
                    self.f(&mut r, "weight", &[*w]);
                    r
                })
                .collect();
            self.put(b"DATA", a, &recs);
        }
    }

    // ----------------------------------------------------- Camera, Light

    pub(super) fn camera(&mut self, addr: u64, name: &str, c: &Camera, opts: &BlendExport) {
        self.start_id();
        let mut r = self.id("Camera", "CA", name);
        let (w, h) = (opts.width.max(1) as f32, opts.height.max(1) as f32);
        let (sx, sy) = (if c.sensor_width > 0.0 { c.sensor_width } else { 36.0 }, if c.sensor_height > 0.0 { c.sensor_height } else { 24.0 });
        // The sensor size and axis the lens is measured against (Blender's
        // sensor fit; Auto measures sensor_x along the larger side).
        let vertical = match c.sensor_fit {
            SensorFit::Horizontal => None,
            SensorFit::Vertical => Some(sy),
            SensorFit::Auto if w >= h => None,
            SensorFit::Auto => Some(sx),
        };
        let half_y = (c.fov_y * 0.5).clamp(1e-4, 1.5);
        let lens = match vertical {
            Some(s) => s * 0.5 / half_y.tan(),
            None => sx * 0.5 / (half_y.tan() * w / h),
        };
        self.f(&mut r, "lens", &[lens]);
        self.f(&mut r, "sensor_x", &[sx]);
        self.f(&mut r, "sensor_y", &[sy]);
        self.i(&mut r, "sensor_fit", match c.sensor_fit {
            SensorFit::Auto => 0,
            SensorFit::Horizontal => 1,
            SensorFit::Vertical => 2,
        });
        self.f(&mut r, "clipsta", &[c.near]);
        self.f(&mut r, "clipend", &[c.far]);
        self.f(&mut r, "shiftx", &[c.shift_x]);
        self.f(&mut r, "shifty", &[c.shift_y]);
        if let Some(os) = c.ortho_scale {
            self.i(&mut r, "type", 1);
            self.f(&mut r, "ortho_scale", &[if vertical.is_some() { os } else { os * w / h }]);
        }
        if let Some(p) = c.panorama {
            self.i(&mut r, "type", 2);
            self.i(&mut r, "panorama_type", match p {
                PanoramaType::Equirectangular => 0,
                PanoramaType::FisheyeEquidistant => 1,
                PanoramaType::FisheyeEquisolid => 2,
                #[allow(unreachable_patterns)]
                _ => 0,
            });
            for (k, v) in [("fisheye_fov", c.fisheye_fov), ("fisheye_lens", c.fisheye_lens), ("latitude_min", c.latitude_min), ("latitude_max", c.latitude_max), ("longitude_min", c.longitude_min), ("longitude_max", c.longitude_max)] {
                self.f(&mut r, k, &[v]);
            }
        }
        let d = &c.dof;
        self.i(&mut r, "dof.flag", d.enabled as i64);
        self.f(&mut r, "dof.focus_distance", &[d.focus_distance]);
        self.f(&mut r, "dof.aperture_fstop", &[d.fstop]);
        self.f(&mut r, "dof.aperture_rotation", &[d.rotation]);
        self.f(&mut r, "dof.aperture_ratio", &[d.ratio]);
        self.i(&mut r, "dof.aperture_blades", d.blades as i64);
        self.put(b"CA", addr, &[r]);
    }

    pub(super) fn light(&mut self, addr: u64, name: &str, l: &Light) {
        self.start_id();
        let mut r = self.id("Lamp", "LA", name);
        let ty = match l.kind {
            LightKind::Point => 0,
            LightKind::Sun => 1,
            LightKind::Spot { angle, blend } => {
                self.f(&mut r, "spotsize", &[angle]);
                self.f(&mut r, "spotblend", &[blend]);
                2
            }
            LightKind::Area { size } => {
                self.f(&mut r, "area_size", &[size]);
                self.f(&mut r, "area_sizey", &[l.size_y]);
                self.f(&mut r, "area_spread", &[l.spread]);
                self.i(&mut r, "area_shape", match l.shape {
                    AreaShape::Square => 0,
                    AreaShape::Rectangle => 1,
                    AreaShape::Disk => 4,
                    AreaShape::Ellipse => 5,
                });
                4
            }
        };
        self.i(&mut r, "type", ty);
        self.f(&mut r, "r", &[l.color.x]);
        self.f(&mut r, "g", &[l.color.y]);
        self.f(&mut r, "b", &[l.color.z]);
        // 4.x reads `energy_new`; older versions `energy`.
        self.f(&mut r, "energy_new", &[l.power]);
        self.f(&mut r, "energy", &[l.power]);
        match l.kind {
            LightKind::Sun => self.f(&mut r, "sun_angle", &[l.radius]),
            LightKind::Point | LightKind::Spot { .. } => self.f(&mut r, "radius", &[l.radius]),
            LightKind::Area { .. } => {}
        }
        // LA_SHADOW (bit 0) with the default LA_USE_SOFT_FALLOFF.
        self.i(&mut r, "mode", if l.cast_shadow { 1 | 1 << 16 } else { 1 << 16 });
        self.put(b"LA", addr, &[r]);
    }

    // ---------------------------------------------------------- Armature

    pub(super) fn armature(&mut self, addr: u64, name: &str, arm: &Armature) {
        self.start_id();
        let mut r = self.id("bArmature", "AR", name);
        let bone_addrs: Vec<u64> = arm.bones.iter().map(|_| self.addr()).collect();
        let roots: Vec<u64> = (0..arm.bones.len()).filter(|&i| arm.parent(i).is_none()).map(|i| bone_addrs[i]).collect();
        self.listbase(&mut r, "bonebase", &roots);
        let coll_addrs: Vec<u64> = arm.collections.iter().map(|_| self.addr()).collect();
        let arr = if coll_addrs.is_empty() { 0 } else { self.addr() };
        self.p(&mut r, "collection_array", arr);
        self.i(&mut r, "collection_array_num", coll_addrs.len() as i64);
        self.i(&mut r, "collection_root_count", coll_addrs.len() as i64);
        if let Some(c) = arm.collections.first() {
            self.s(&mut r, "active_collection_name", c);
        }
        self.put(b"AR", addr, &[r]);

        // Bones: siblings linked, children listed under their parent.
        let mut recs = Vec::new();
        for (i, b) in arm.bones.iter().enumerate() {
            let loc = arm.bone_local(i);
            let arm_mat = b.rest_matrix();
            let kids: Vec<u64> = (0..arm.bones.len()).filter(|&k| arm.parent(k) == Some(i)).map(|k| bone_addrs[k]).collect();
            let sibs: Vec<u64> = (0..arm.bones.len()).filter(|&k| arm.parent(k) == arm.parent(i)).map(|k| bone_addrs[k]).collect();
            let at = sibs.iter().position(|&a| a == bone_addrs[i]).unwrap_or(0);
            let mut r = self.rec("Bone");
            self.p(&mut r, "next", sibs.get(at + 1).copied().unwrap_or(0));
            self.p(&mut r, "prev", if at > 0 { sibs[at - 1] } else { 0 });
            self.p(&mut r, "parent", arm.parent(i).map_or(0, |p| bone_addrs[p]));
            self.listbase(&mut r, "childbase", &kids);
            self.s(&mut r, "name", &b.name);
            self.f(&mut r, "roll", &[loc.roll]);
            self.f(&mut r, "head", &loc.head.to_array());
            self.f(&mut r, "tail", &loc.tail.to_array());
            let bm = loc.bone_mat.cols.iter().flat_map(|c| c.to_array()).collect::<Vec<f32>>();
            self.f(&mut r, "bone_mat", &bm);
            self.f(&mut r, "arm_head", &b.head.to_array());
            self.f(&mut r, "arm_tail", &b.tail.to_array());
            self.f(&mut r, "arm_mat", &arm_mat.to_cols_array());
            self.f(&mut r, "arm_roll", &[b.roll]);
            self.f(&mut r, "length", &[b.length()]);
            let mut flag = 0;
            for (bit, on) in [(4, b.use_connect), (9, !b.use_inherit_rotation), (11, b.use_envelope_multiply), (12, !b.use_deform), (22, !b.use_local_location), (23, b.use_relative_parent)] {
                if on {
                    flag |= 1i64 << bit;
                }
            }
            self.i(&mut r, "flag", flag);
            self.i(&mut r, "inherit_scale_mode", inherit_scale(b.inherit_scale));
            self.f(&mut r, "dist", &[b.envelope_distance]);
            self.f(&mut r, "weight", &[b.envelope_weight]);
            self.f(&mut r, "rad_head", &[b.head_radius]);
            self.f(&mut r, "rad_tail", &[b.tail_radius]);
            self.i(&mut r, "segments", b.bbone_segments.clamp(1, 32) as i64);
            // ED_armature_from_edit's values for the B-Bone and display fields.
            let w = b.length() * 0.1;
            self.f(&mut r, "xwidth", &[w]);
            self.f(&mut r, "zwidth", &[w]);
            self.f(&mut r, "ease1", &[1.0]);
            self.f(&mut r, "ease2", &[1.0]);
            self.f(&mut r, "scale_in_y", &[1.0]);
            self.f(&mut r, "scale_out_y", &[1.0]);
            self.f(&mut r, "scale_in", &[1.0, 1.0, 1.0]);
            self.f(&mut r, "scale_out", &[1.0, 1.0, 1.0]);
            self.f(&mut r, "size", &[1.0, 1.0, 1.0]);
            self.i(&mut r, "layer", 1);
            recs.push(r);
        }
        for (r, a) in recs.iter().zip(&bone_addrs) {
            self.put(b"DATA", *a, std::slice::from_ref(r));
        }
        if arr != 0 {
            self.put_raw_at(arr, coll_addrs.iter().flat_map(|p| p.to_le_bytes()).collect());
            for (ci, cname) in arm.collections.iter().enumerate() {
                let members: Vec<usize> = (0..arm.bones.len()).filter(|&b| arm.bones[b].collections.contains(cname)).collect();
                let maddrs: Vec<u64> = members.iter().map(|_| self.addr()).collect();
                let mut c = self.rec("BoneCollection");
                self.s(&mut c, "name", cname);
                self.listbase(&mut c, "bones", &maddrs);
                self.i(&mut c, "flags", 1 | 2 | 8); // visible, selectable, ancestors visible
                self.i(&mut c, "child_index", 0);
                self.put(b"DATA", coll_addrs[ci], &[c]);
                let recs: Vec<Rec> = members
                    .iter()
                    .map(|&b| {
                        let mut m = self.rec("BoneCollectionMember");
                        self.p(&mut m, "bone", bone_addrs[b]);
                        m
                    })
                    .collect();
                self.chain(recs, &maddrs);
            }
        }
    }

    // ------------------------------------------------------------ Images

    pub(super) fn image(&mut self, addr: u64, name: &str, img: &crate::scene::Image) {
        self.start_id();
        let mut r = self.id("Image", "IM", name);
        let file = format!("//{name}.png");
        self.s(&mut r, "name", &file);
        self.i(&mut r, "source", 1); // IMA_SRC_FILE
        self.i(&mut r, "type", 0); // IMA_TYPE_IMAGE
        self.s(&mut r, "colorspace_settings.name", "sRGB");
        let ipf = self.addr();
        self.listbase(&mut r, "packedfiles", &[ipf]);
        let tile = self.addr();
        self.listbase(&mut r, "tiles", &[tile]);
        self.put(b"IM", addr, &[r]);
        let png = crate::io::encode_png(img);
        let len = png.len();
        let data = self.raw(png);
        let mut pf = self.rec("PackedFile");
        self.i(&mut pf, "size", len as i64);
        self.p(&mut pf, "data", data);
        let pfa = self.data(&[pf]);
        let mut i = self.rec("ImagePackedFile");
        self.p(&mut i, "packedfile", pfa);
        self.s(&mut i, "filepath", &file);
        self.i(&mut i, "tile_number", 1001);
        self.put(b"DATA", ipf, &[i]);
        let mut t = self.rec("ImageTile");
        self.i(&mut t, "tile_number", 1001);
        self.i(&mut t, "gen_x", img.width as i64);
        self.i(&mut t, "gen_y", img.height as i64);
        self.f(&mut t, "gen_color", &[0.0, 0.0, 0.0, 1.0]);
        self.put(b"DATA", tile, &[t]);
    }

    // ----------------------------------------------------------- Actions

    pub(super) fn action(&mut self, addr: u64, name: &str, a: &BlendAction) {
        self.start_id();
        let mut r = self.id("bAction", "AC", name);
        let fc_addrs: Vec<u64> = a.curves.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "curves", &fc_addrs);
        self.i(&mut r, "idroot", ID_OB);
        self.put(b"AC", addr, &[r]);
        let mut recs = Vec::new();
        let mut later = Vec::new();
        for c in &a.curves {
            let mut f = self.rec("FCurve");
            let path = self.string(&c.path);
            self.p(&mut f, "rna_path", path);
            self.i(&mut f, "array_index", c.index as i64);
            self.i(&mut f, "totvert", c.keys.len() as i64);
            self.i(&mut f, "flag", 1 | 2 | 4); // visible, selected, active
            self.i(&mut f, "auto_smoothing", 1); // FCURVE_SMOOTH_CONT_ACCEL
            self.i(&mut f, "color_mode", 0);
            let bz = if c.keys.is_empty() { 0 } else { self.addr() };
            self.p(&mut f, "bezt", bz);
            recs.push(f);
            later.push((bz, c));
        }
        self.chain(recs, &fc_addrs);
        for (bz, c) in later {
            if bz == 0 {
                continue;
            }
            let n = c.keys.len();
            let recs: Vec<Rec> = (0..n)
                .map(|k| {
                    let key = c.keys[k];
                    // Given handles stay as they are (HD_ALIGN); otherwise
                    // auto-clamped ones (HD_AUTO_ANIM) a third of the way to
                    // the neighbours, flat at extremes and ends.
                    let (l, r, ty) = match key.handles {
                        Some([l, r]) => (l, r, 3),
                        None => {
                            let prev = (k > 0).then(|| c.keys[k - 1]);
                            let next = c.keys.get(k + 1).copied();
                            let dl = prev.map_or(1.0, |p| (key.frame - p.frame) / 3.0);
                            let dr = next.map_or(1.0, |q| (q.frame - key.frame) / 3.0);
                            let slope = match (prev, next) {
                                (Some(p), Some(q)) if (key.value - p.value) * (q.value - key.value) > 0.0 && q.frame > p.frame => (q.value - p.value) / (q.frame - p.frame),
                                _ => 0.0,
                            };
                            ([key.frame - dl, key.value - slope * dl], [key.frame + dr, key.value + slope * dr], 4)
                        }
                    };
                    let mut b = self.rec("BezTriple");
                    self.f(&mut b, "vec", &[l[0], l[1], 0.0, key.frame, key.value, 0.0, r[0], r[1], 0.0]);
                    self.i(&mut b, "ipo", key.interpolation as i64);
                    self.i(&mut b, "h1", ty);
                    self.i(&mut b, "h2", ty);
                    self.f(&mut b, "back", &[1.70158]);
                    self.f(&mut b, "amplitude", &[0.8]);
                    self.f(&mut b, "period", &[4.1]);
                    b
                })
                .collect();
            self.put(b"DATA", bz, &recs);
        }
    }
}

/// Scene-level ID addresses.
pub(super) fn assign_ids(w: &mut W, scene: &Scene, actions: &[&BlendAction]) -> Ids {
    let mut many = |n: usize| (0..n).map(|_| w.addr()).collect::<Vec<u64>>();
    let objects = many(scene.objects.len());
    let obdata = many(scene.objects.len());
    let meshes = many(scene.meshes.len());
    let materials = many(scene.materials.len());
    let images = many(scene.images.len());
    let armatures = many(scene.armatures.len());
    let collections = many(scene.collections.len());
    let groups = many(scene.node_groups.len());
    let actions = many(actions.len());
    let lattices = many(scene.lattices.len());
    let gpencils = many(scene.grease_pencils.len());
    let gp_materials = scene.grease_pencils.iter().map(|g| many(g.materials.len())).collect();
    let keys = scene.meshes.iter().map(|m| if m.shape_keys.is_empty() { 0 } else { w.addr() }).collect();
    let mut particles = HashMap::new();
    for (oi, o) in scene.objects.iter().enumerate() {
        for (k, m) in o.modifiers.iter().enumerate() {
            if let Modifier::Physics(PhysicsModifier::ParticleSystem(_)) = m {
                particles.insert((oi, k), w.addr());
            }
        }
    }
    let mut object_by_name = HashMap::new();
    for (o, &a) in scene.objects.iter().zip(&objects) {
        object_by_name.entry(o.name.clone()).or_insert(a);
    }
    let mut collection_by_name = HashMap::new();
    for (c, &a) in scene.collections.iter().zip(&collections).skip(1) {
        collection_by_name.entry(c.name.clone()).or_insert(a);
    }
    Ids {
        scene: w.addr(),
        objects,
        obdata,
        meshes,
        materials,
        images,
        armatures,
        collections,
        groups,
        world: w.addr(),
        actions,
        lattices,
        gpencils,
        gp_materials,
        keys,
        particles,
        object_by_name,
        collection_by_name,
    }
}

/// A material's tree: its own, or one built from its fields (Principled BSDF,
/// with an Image Texture on Base Color when it has one).
fn material_tree(m: &Material) -> NodeTree {
    if let Some(t) = &m.node_tree {
        return t.clone();
    }
    let mut t = NodeTree::default_material("Shader Nodetree");
    if let Some(p) = t.nodes.iter().position(|n| n.idname == "ShaderNodeBsdfPrincipled") {
        let n = &mut t.nodes[p];
        n.set_input("Base Color", Value::Color(m.base_color));
        n.set_input("Alpha", Value::Float(m.base_color[3]));
        n.set_input("Metallic", Value::Float(m.metallic));
        n.set_input("Roughness", Value::Float(m.roughness));
        n.set_input("IOR", Value::Float(m.ior));
        n.set_input("Transmission Weight", Value::Float(m.transmission));
        n.set_input("Emission Color", Value::Color([m.emission.x, m.emission.y, m.emission.z, 1.0]));
        n.set_input("Emission Strength", Value::Float(m.emission_strength));
        if let Some(img) = m.base_color_texture {
            if let Some(tex) = t.add("ShaderNodeTexImage", [-600.0, 300.0]) {
                t.nodes[tex].set_prop("image", Prop::Id(Some(img)));
                let _ = t.link(tex, "Color", p, "Base Color");
            }
        }
    }
    t
}

/// A world tree for an environment image: Environment Texture into
/// Background into World Output.
fn world_tree(w: &World, img: usize) -> NodeTree {
    let mut t = NodeTree::default_world("Shader Nodetree");
    if let Some(b) = t.nodes.iter().position(|n| n.idname == "ShaderNodeBackground") {
        t.nodes[b].set_input("Strength", Value::Float(w.strength));
        if let Some(e) = t.add("ShaderNodeTexEnvironment", [-500.0, 300.0]) {
            t.nodes[e].set_prop("image", Prop::Id(Some(img)));
            let _ = t.link(e, "Color", b, "Color");
        }
    }
    t
}

impl W {
    // ---------------------------------------------------- Material, World

    pub(super) fn material(&mut self, addr: u64, name: &str, m: &Material, ids: &Ids) {
        self.start_id();
        let mut r = self.id("Material", "MA", name);
        let c = m.base_color;
        self.f(&mut r, "r", &[c[0]]);
        self.f(&mut r, "g", &[c[1]]);
        self.f(&mut r, "b", &[c[2]]);
        self.f(&mut r, "a", &[c[3]]);
        self.f(&mut r, "metallic", &[m.metallic]);
        self.f(&mut r, "roughness", &[m.roughness]);
        self.i(&mut r, "index", m.pass_index.min(32767) as i64);
        // EEVEE: Render Method (MA_SURFACE_METHOD_DEFERRED 0 / FORWARD 1),
        // blend_flag MA_BL_CULL_BACKFACE, _SS_REFRACTION, _CULL_BACKFACE_SHADOW,
        // _TRANSPARENT_SHADOW, and the refraction thickness.
        let e = &m.eevee;
        self.i(&mut r, "surface_render_method", (e.render_method == crate::scene::RenderMethod::Blended) as i64);
        let keep = self.get(&r, "blend_flag") & !(1 | 4 | 8 | 16);
        let bf = [(e.backface_culling, 1), (e.screen_refraction, 4), (e.backface_culling_shadow, 8), (e.transparent_shadows, 16)].iter().fold(keep, |a, &(on, b)| if on { a | b } else { a });
        self.i(&mut r, "blend_flag", bf);
        self.f(&mut r, "refract_depth", &[e.refraction_depth]);
        self.i(&mut r, "use_nodes", 1);
        let tree = self.addr();
        self.p(&mut r, "nodetree", tree);
        self.put(b"MA", addr, &[r]);
        self.node_tree_at(tree, b"DATA", &material_tree(m), "Shader Nodetree", true, ids);
    }

    pub(super) fn world(&mut self, addr: u64, w: &World, ids: &Ids) {
        self.start_id();
        let mut r = self.id("World", "WO", "World");
        let c = w.color * w.strength;
        self.f(&mut r, "horr", &[c.x]);
        self.f(&mut r, "horg", &[c.y]);
        self.f(&mut r, "horb", &[c.z]);
        let tree = match (&w.node_tree, w.hdri) {
            (Some(t), _) => Some(t.clone()),
            (None, Some(img)) => Some(world_tree(w, img)),
            (None, None) => None,
        };
        let ta = tree.as_ref().map(|_| self.addr()).unwrap_or(0);
        self.i(&mut r, "use_nodes", tree.is_some() as i64);
        self.p(&mut r, "nodetree", ta);
        self.put(b"WO", addr, &[r]);
        if let Some(t) = &tree {
            self.node_tree_at(ta, b"DATA", t, "Shader Nodetree", true, ids);
        }
    }
}

/// Write `scene` (and the given legacy actions, attached to the objects they
/// name) as a Blender 4.2 `.blend` file. See `write.rs` for what is written.
pub fn export_blend(scene: &Scene, actions: &[BlendAction], opts: BlendExport) -> Vec<u8> {
    export_blend_checked(scene, actions, opts).0
}

/// [`export_blend`] plus the member paths that did not resolve against the
/// writer's SDNA (always empty unless the writer has a bug).
pub(super) fn export_blend_checked(scene: &Scene, actions: &[BlendAction], opts: BlendExport) -> (Vec<u8>, Vec<String>) {
    let mut w = W::new();
    let by_object = super::write::actions_by_object(actions);
    let used: Vec<&BlendAction> = scene.objects.iter().filter_map(|o| by_object.get(o.name.as_str()).copied()).collect();
    let ids = assign_ids(&mut w, scene, &used);
    let mut names = Names::default();

    let (view_layer, scene_name) = w.scene(scene, &ids, &mut names, &opts);
    for ci in 1..scene.collections.len() {
        w.collection(scene, &ids, &mut names, ci);
    }
    let mut act_k = 0;
    for oi in 0..scene.objects.len() {
        let act = by_object.get(scene.objects[oi].name.as_str()).map(|_| {
            act_k += 1;
            ids.actions[act_k - 1]
        });
        w.object(scene, &ids, &mut names, oi, act);
    }
    for (mi, m) in scene.meshes.iter().enumerate() {
        let name = names.unique("ME", scene.mesh_names.get(mi).map_or("Mesh", |s| s.as_str()));
        // Material slots of the first object using the mesh.
        let slots: Vec<u64> = scene
            .objects
            .iter()
            .find(|o| matches!(o.data, ObjectData::Mesh(k) if k == mi))
            .map(|o| o.materials.iter().map(|&k| ids.materials.get(k).copied().unwrap_or(0)).collect())
            .unwrap_or_default();
        w.mesh(ids.meshes[mi], &name, m, &slots, ids.keys[mi]);
        if ids.keys[mi] != 0 {
            let kname = names.unique("KE", &format!("Key {name}"));
            w.shape_key(ids.keys[mi], &kname, m, ids.meshes[mi]);
        }
    }
    for (li, l) in scene.lattices.iter().enumerate() {
        let name = names.unique("LT", scene.lattice_names.get(li).map_or("Lattice", |s| s.as_str()));
        w.lattice(ids.lattices[li], &name, l);
    }
    for (gi, g) in scene.grease_pencils.iter().enumerate() {
        let name = names.unique("GD", scene.grease_pencil_names.get(gi).map_or("GPencil", |s| s.as_str()));
        w.gpencil(ids.gpencils[gi], &name, g, &ids.gp_materials[gi], &ids);
        for (k, m) in g.materials.iter().enumerate() {
            let mname = names.unique("MA", &m.name);
            w.gp_material(ids.gp_materials[gi][k], &mname, m, &ids);
        }
    }
    for (oi, o) in scene.objects.iter().enumerate() {
        for (k, m) in o.modifiers.iter().enumerate() {
            if let (Modifier::Physics(PhysicsModifier::ParticleSystem(ps)), Some(&a)) = (m, ids.particles.get(&(oi, k))) {
                let name = names.unique("PA", if ps.settings.name.is_empty() { "ParticleSettings" } else { &ps.settings.name });
                w.particle_settings(a, &name, &ps.settings, &ids);
            }
        }
    }
    for (oi, o) in scene.objects.iter().enumerate() {
        match &o.data {
            ObjectData::Camera(c) => {
                let name = names.unique("CA", &o.name);
                w.camera(ids.obdata[oi], &name, c, &opts);
            }
            ObjectData::Light(l) => {
                let name = names.unique("LA", &o.name);
                w.light(ids.obdata[oi], &name, l);
            }
            _ => {}
        }
    }
    for (ai, a) in scene.armatures.iter().enumerate() {
        let name = names.unique("AR", scene.armature_names.get(ai).map_or("Armature", |s| s.as_str()));
        w.armature(ids.armatures[ai], &name, a);
    }
    for (mi, m) in scene.materials.iter().enumerate() {
        let name = names.unique("MA", &m.name);
        w.material(ids.materials[mi], &name, m, &ids);
    }
    w.world(ids.world, &scene.world, &ids);
    for (gi, g) in scene.node_groups.iter().enumerate() {
        w.start_id();
        w.node_tree_at(ids.groups[gi], b"NT", g, "NodeGroup", false, &ids);
    }
    for (ii, img) in scene.images.iter().enumerate() {
        let name = names.unique("IM", if img.name.is_empty() { "Image" } else { &img.name });
        w.image(ids.images[ii], &name, img);
    }
    for (k, a) in used.iter().enumerate() {
        let name = names.unique("AC", if a.action.is_empty() { "Action" } else { &a.action });
        w.action(ids.actions[k], &name, a);
    }
    let glob = w.glob(ids.scene, view_layer);
    let missing = std::mem::take(&mut w.missing);
    let bytes = w.finish(glob, (scene.frame_start, scene.frame_end, &scene_name));
    (bytes, missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::registry::{new_node, node_types, update_sockets};
    use crate::nodes::{enums, Node, SocketType, TreeKind};

    /// Every registered node of `kind` (no group nodes) in one tree.
    fn every_node(kind: TreeKind) -> NodeTree {
        let mut t = NodeTree::new("All", kind);
        for ty in node_types().iter().filter(|t| t.tree == kind) {
            if matches!(ty.idname, "ShaderNodeGroup" | "NodeGroupInput" | "NodeGroupOutput" | "CompositorNodeGroup" | "GeometryNodeGroup") {
                continue;
            }
            t.nodes.push((ty.build)());
        }
        t
    }

    fn scene_with(material: NodeTree, comp: NodeTree) -> Scene {
        let mut s = Scene::new();
        s.images.push(crate::scene::Image::new("Img", 2, 2));
        s.add_material(Material { name: "All".into(), node_tree: Some(material), ..Material::default() });
        s.compositor = Some(comp);
        s
    }

    /// Props that differ after a round trip, as "node.prop: before vs after".
    fn prop_diffs(a: &NodeTree, b: &NodeTree) -> Vec<String> {
        let mut out = Vec::new();
        assert_eq!(a.nodes.len(), b.nodes.len());
        for (x, y) in a.nodes.iter().zip(&b.nodes) {
            assert_eq!(x.idname, y.idname);
            for (k, v) in &x.props {
                let w = y.prop(k);
                let same = match (v, w) {
                    (Prop::Float(p), Some(Prop::Float(q))) => (p - q).abs() <= 1e-5 * (1.0 + p.abs()),
                    (Prop::Ramp(_) | Prop::Curve(_), Some(_)) => true,
                    (_, Some(w)) => v == w,
                    (_, None) => false,
                };
                if !same {
                    out.push(format!("{}.{k}: {v:?} vs {w:?}", x.idname));
                }
            }
        }
        out
    }

    /// `t` as a geometry node group used by a Nodes modifier (the importer
    /// reads the groups that something uses).
    fn use_geometry_group(s: &mut Scene, mut t: NodeTree) {
        t.name = "Geo".into();
        s.node_groups.push(t);
        let o = s.add_mesh_object("Host", crate::Mesh::new());
        s.objects[o].modifiers.push(crate::modifiers::Modifier::Nodes { group: "Geo".into(), inputs: Vec::new() });
    }

    fn round_trip(s: &Scene) -> Scene {
        let (bytes, missing) = export_blend_checked(s, &[], BlendExport::default());
        assert!(missing.is_empty(), "unresolved members: {missing:?}");
        super::super::import_blend(&bytes).expect("imports")
    }

    /// Every shader, compositor and geometry node writes only members the
    /// SDNA has, reads back as the same type, and keeps its default
    /// properties.
    #[test]
    fn every_node_round_trips() {
        let mut s = scene_with(every_node(TreeKind::Shader), every_node(TreeKind::Compositor));
        use_geometry_group(&mut s, every_node(TreeKind::Geometry));
        let r = round_trip(&s);
        let mut diffs = prop_diffs(s.materials[0].node_tree.as_ref().unwrap(), r.materials[0].node_tree.as_ref().unwrap());
        diffs.extend(prop_diffs(s.compositor.as_ref().unwrap(), r.compositor.as_ref().unwrap()));
        diffs.extend(prop_diffs(&s.node_groups[0], &r.node_groups[0]));
        assert!(diffs.is_empty(), "{} props changed:\n{}", diffs.len(), diffs.join("\n"));
    }

    /// Modifiers, constraints, shape keys, lattices, Grease Pencil and
    /// physics write only members the writer's SDNA has.
    #[test]
    fn every_kind_writes_known_members() {
        use crate::constraints::Constraint;
        use crate::modifiers::Modifier;
        let mut s = Scene::new();
        let mut m = crate::Mesh::new();
        m.positions = vec![crate::Vec3::ZERO, crate::Vec3::new(1.0, 0.0, 0.0), crate::Vec3::new(0.0, 1.0, 0.0)];
        m.faces = vec![crate::Face::new(vec![0, 1, 2])];
        m.vertex_groups = vec!["G".into()];
        m.weights = vec![vec![(0, 1.0)], vec![], vec![]];
        m.shape_keys = vec![crate::mesh::ShapeKey::new("Basis", m.positions.clone()), crate::mesh::ShapeKey::new("Key", m.positions.clone())];
        let o = s.add_mesh_object("Body", m);
        for ty in [
            "mirror", "subsurf", "array", "solidify", "bevel", "triangulate", "weld", "displace", "smooth", "decimate", "remesh", "screw", "wireframe", "edgesplit", "build", "boolean",
            "simpledeform", "cast", "wave", "shrinkwrap", "laplaciansmooth", "correctivesmooth", "weightednormal", "nodes", "armature", "hook", "lattice", "warp", "vertexweightedit",
            "vertexweightmix", "vertexweightproximity", "multires", "cloth", "softbody", "collision", "particlesystem", "fluid",
        ] {
            s.objects[o].modifiers.push(Modifier::with_defaults(ty).unwrap_or_else(|| panic!("{ty}")));
        }
        for ty in [
            "copylocation", "copyrotation", "copyscale", "copytransforms", "limitdistance", "limitlocation", "limitrotation", "limitscale", "maintainvolume", "transformation", "childof",
            "trackto", "dampedtrack", "lockedtrack", "stretchto", "floor", "armature", "ik",
        ] {
            s.objects[o].constraints.push(Constraint::with_defaults(ty, false).unwrap());
        }
        s.objects[o].physics.rigid_body = Some(Default::default());
        s.objects[o].physics.rigid_body_constraint = Some(Default::default());
        s.objects[o].physics.field = Some(Default::default());
        s.physics.rigidbody_world = Some(Default::default());
        s.add_lattice_object("Cage", crate::lattice::Lattice::new(2, 3, 2));
        let mut g = crate::grease_pencil::GreasePencil::blank();
        g.materials.push(crate::grease_pencil::GpMaterial::fill("Fill", [1.0; 4]));
        g.vertex_groups.push("G".into());
        let mut p = crate::grease_pencil::GpPoint::new(crate::Vec3::ZERO, 0.01);
        p.weights = vec![(0, 0.5)];
        g.layers[0].frames.push(crate::grease_pencil::GpFrame {
            frame: 1,
            drawing: crate::grease_pencil::Drawing { strokes: vec![crate::grease_pencil::GpStroke::new(vec![p.clone(), p], 0)] },
            key_type: Default::default(),
            select: false,
        });
        g.layers.push(crate::grease_pencil::GpLayer::new("Masked"));
        g.layers[1].masks = vec!["Layer".into()];
        s.add_grease_pencil_object("Sketch", g);
        let r = round_trip(&s);
        assert_eq!(r.objects.len(), 3);
    }

    /// Geometry node settings (storage, `custom1`/`custom2`): every choice
    /// the importer knows survives a round trip, and zones keep their
    /// pairing and items.
    #[test]
    fn geometry_node_settings_round_trip() {
        const DOM: &[&str] = &["POINT", "EDGE", "FACE", "CORNER", "CURVE", "INSTANCE"];
        const DT: &[&str] = &["FLOAT", "INT", "FLOAT_VECTOR", "FLOAT_COLOR", "BYTE_COLOR", "BOOLEAN", "FLOAT2", "INT8", "QUATERNION", "FLOAT4X4"];
        const ST: &[&str] = &["FLOAT", "INT", "BOOLEAN", "VECTOR", "RGBA", "ROTATION", "MATRIX", "STRING", "GEOMETRY", "OBJECT", "COLLECTION", "MATERIAL", "IMAGE", "MENU"];
        let cases: &[(&str, &str, &[&str])] = &[
            ("GeometryNodeMeshCylinder", "fill_type", &["NONE", "NGON", "TRIANGLE_FAN"]),
            ("GeometryNodeMeshCone", "fill_type", &["NONE", "NGON", "TRIANGLE_FAN"]),
            ("GeometryNodeMeshCircle", "fill_type", &["NONE", "NGON", "TRIANGLE_FAN"]),
            ("GeometryNodeMeshLine", "mode", &["END_POINTS", "OFFSET"]),
            ("GeometryNodeMeshLine", "count_mode", &["TOTAL", "RESOLUTION"]),
            ("GeometryNodeExtrudeMesh", "mode", &["VERTICES", "EDGES", "FACES"]),
            ("GeometryNodeStoreNamedAttribute", "data_type", DT),
            ("GeometryNodeStoreNamedAttribute", "domain", DOM),
            ("GeometryNodeInputNamedAttribute", "data_type", DT),
            ("GeometryNodeCaptureAttribute", "domain", DOM),
            ("GeometryNodeViewer", "data_type", DT),
            ("GeometryNodeViewer", "domain", &["AUTO", "POINT", "FACE"]),
            ("GeometryNodeAccumulateField", "domain", DOM),
            ("GeometryNodeSampleIndex", "domain", DOM),
            ("GeometryNodeSampleIndex", "data_type", DT),
            ("GeometryNodeRaycast", "mapping", &["INTERPOLATED", "NEAREST"]),
            ("GeometryNodeProximity", "target_element", &["POINTS", "EDGES", "FACES"]),
            ("GeometryNodeDeleteGeometry", "mode", &["ALL", "EDGE_FACE", "ONLY_FACE"]),
            ("GeometryNodeDeleteGeometry", "domain", DOM),
            ("GeometryNodeSeparateGeometry", "domain", DOM),
            ("GeometryNodeDuplicateElements", "domain", DOM),
            ("GeometryNodeMergeByDistance", "mode", &["ALL", "CONNECTED"]),
            ("GeometryNodeObjectInfo", "transform_space", &["ORIGINAL", "RELATIVE"]),
            ("GeometryNodeCollectionInfo", "transform_space", &["ORIGINAL", "RELATIVE"]),
            ("GeometryNodeMeshToPoints", "mode", &["VERTICES", "EDGES", "FACES", "CORNERS"]),
            ("GeometryNodeCurvePrimitiveCircle", "mode", &["POINTS", "RADIUS"]),
            ("GeometryNodeCurvePrimitiveLine", "mode", &["POINTS", "DIRECTION"]),
            ("GeometryNodeCurveArc", "mode", &["POINTS", "RADIUS"]),
            ("GeometryNodeCurvePrimitiveQuadrilateral", "mode", &["RECTANGLE", "PARALLELOGRAM", "TRAPEZOID", "KITE", "POINTS"]),
            ("GeometryNodeCurvePrimitiveBezierSegment", "mode", &["POSITION", "OFFSET"]),
            ("GeometryNodeCurveToPoints", "mode", &["EVALUATED", "COUNT", "LENGTH"]),
            ("GeometryNodeFillCurve", "mode", &["TRIANGLES", "NGONS"]),
            ("GeometryNodeFilletCurve", "mode", &["BEZIER", "POLY"]),
            ("GeometryNodeResampleCurve", "mode", &["EVALUATED", "COUNT", "LENGTH"]),
            ("GeometryNodeTrimCurve", "mode", &["FACTOR", "LENGTH"]),
            ("GeometryNodeSetCurveHandlePositions", "mode", &["LEFT", "RIGHT"]),
            ("GeometryNodeCurveSetHandles", "handle_type", &["FREE", "AUTO", "VECTOR", "ALIGN"]),
            ("GeometryNodeCurveSetHandles", "mode", &["LEFT_RIGHT", "LEFT", "RIGHT"]),
            ("GeometryNodeCurveSplineType", "spline_type", &["CATMULL_ROM", "POLY", "BEZIER", "NURBS"]),
            ("GeometryNodeSampleCurve", "mode", &["FACTOR", "LENGTH"]),
            ("GeometryNodeSubdivisionSurface", "uv_smooth", &["NONE", "PRESERVE_CORNERS", "PRESERVE_BOUNDARIES", "SMOOTH_ALL"]),
            ("GeometryNodeSubdivisionSurface", "boundary_smooth", &["ALL", "PRESERVE_CORNERS"]),
            ("GeometryNodeSwitch", "input_type", ST),
            ("GeometryNodeIndexSwitch", "data_type", ST),
            ("GeometryNodeMenuSwitch", "data_type", ST),
            ("FunctionNodeRandomValue", "data_type", &["FLOAT", "INT", "FLOAT_VECTOR", "BOOLEAN"]),
            ("FunctionNodeCompare", "operation", &["LESS_THAN", "LESS_EQUAL", "GREATER_THAN", "GREATER_EQUAL", "EQUAL", "NOT_EQUAL", "BRIGHTER", "DARKER"]),
            ("FunctionNodeCompare", "mode", &["ELEMENT", "LENGTH", "AVERAGE", "DOT_PRODUCT", "DIRECTION"]),
            ("FunctionNodeCombineColor", "mode", &["RGB", "HSV", "HSL"]),
            ("FunctionNodeSeparateColor", "mode", &["RGB", "HSV", "HSL"]),
            ("GeometryNodeAttributeStatistic", "data_type", &["FLOAT", "FLOAT_VECTOR"]),
            ("GeometryNodeAttributeStatistic", "domain", DOM),
            ("GeometryNodeBlurAttribute", "data_type", &["FLOAT", "INT", "FLOAT_VECTOR", "FLOAT_COLOR"]),
            ("GeometryNodeFieldAtIndex", "domain", DOM),
            ("GeometryNodeFieldAtIndex", "data_type", DT),
            ("GeometryNodeFieldOnDomain", "domain", DOM),
            ("GeometryNodeSampleNearest", "domain", &["POINT", "EDGE", "FACE", "CORNER"]),
            ("GeometryNodeSortElements", "domain", &["POINT", "EDGE", "FACE"]),
            ("GeometryNodeSetShadeSmooth", "domain", &["EDGE", "FACE"]),
            ("GeometryNodeScaleElements", "domain", &["EDGE", "FACE"]),
            ("GeometryNodeScaleElements", "scale_mode", &["UNIFORM", "SINGLE_AXIS"]),
            ("GeometryNodeAttributeDomainSize", "component", &["MESH", "POINTCLOUD", "CURVE", "INSTANCES"]),
            ("GeometryNodeDistributePointsOnFaces", "distribute_method", &["RANDOM", "POISSON"]),
            ("GeometryNodeTriangulate", "quad_method", &["BEAUTY", "FIXED", "FIXED_ALTERNATE", "SHORTEST_DIAGONAL", "LONGEST_DIAGONAL"]),
            ("GeometryNodeTriangulate", "ngon_method", &["BEAUTY", "CLIP"]),
            ("GeometryNodeMeshBoolean", "operation", &["INTERSECT", "UNION", "DIFFERENCE"]),
            ("GeometryNodeMeshBoolean", "solver", &["EXACT", "FLOAT"]),
            ("FunctionNodeBooleanMath", "operation", &["AND", "OR", "NOT", "NAND", "NOR", "XNOR", "XOR", "IMPLY", "NIMPLY"]),
            ("FunctionNodeFloatToInt", "rounding_mode", &["ROUND", "FLOOR", "CEILING", "TRUNCATE"]),
        ];
        let mut t = NodeTree::new("Geo", TreeKind::Geometry);
        for (idname, prop, choices) in cases {
            for c in *choices {
                let mut n = new_node(idname).unwrap_or_else(|| panic!("no node {idname}"));
                n.set_prop(prop, Prop::Enum((*c).to_string()));
                update_sockets(&mut n);
                t.nodes.push(n);
            }
        }
        // Value inputs and flags.
        let mut n = new_node("FunctionNodeInputInt").unwrap();
        n.set_prop("integer", Prop::Int(-7));
        t.nodes.push(n);
        let mut n = new_node("FunctionNodeInputBool").unwrap();
        n.set_prop("boolean", Prop::Bool(true));
        t.nodes.push(n);
        let mut n = new_node("FunctionNodeInputVector").unwrap();
        n.set_prop("vector", Prop::Vector([1.0, -2.0, 3.5]));
        t.nodes.push(n);
        let mut n = new_node("FunctionNodeInputColor").unwrap();
        n.set_prop("value", Prop::Color([0.1, 0.2, 0.3, 0.4]));
        t.nodes.push(n);
        let mut n = new_node("FunctionNodeInputString").unwrap();
        n.set_prop("string", Prop::String("hello".into()));
        t.nodes.push(n);
        let mut n = new_node("GeometryNodeSampleIndex").unwrap();
        n.set_prop("clamp", Prop::Bool(true));
        t.nodes.push(n);
        // A repeat zone with a second state item.
        let rin = t.nodes.len();
        t.nodes.push(new_node("GeometryNodeRepeatInput").unwrap());
        t.nodes.push(new_node("GeometryNodeRepeatOutput").unwrap());
        t.nodes[rin + 1].name = "Repeat Output".into();
        t.nodes[rin].set_prop("paired_output", Prop::String("Repeat Output".into()));
        crate::nodes::geo_registry::add_zone_item(&mut t, rin, rin + 1, "Count", SocketType::Int);
        let mut s = Scene::new();
        use_geometry_group(&mut s, t);
        let r = round_trip(&s);
        let (a, b) = (&s.node_groups[0], &r.node_groups[0]);
        let mut diffs = prop_diffs(a, b);
        diffs.sort();
        diffs.dedup();
        assert!(diffs.is_empty(), "{} props changed:\n{}", diffs.len(), diffs.join("\n"));
        let out = &b.nodes[rin + 1];
        assert_eq!(out.inputs.iter().map(|s| s.identifier.as_str()).filter(|i| i.starts_with("Item_")).collect::<Vec<_>>(), ["Item_0", "Item_1"]);
    }

    /// Every choice of every enum property survives a round trip.
    #[test]
    fn every_enum_choice_round_trips() {
        let mut shader = NodeTree::new("Enums", TreeKind::Shader);
        let mut comp = NodeTree::new("Enums", TreeKind::Compositor);
        let mut geo = NodeTree::new("Enums", TreeKind::Geometry);
        for kind in [TreeKind::Shader, TreeKind::Compositor, TreeKind::Geometry] {
            for ty in node_types().iter().filter(|t| t.tree == kind) {
                let n: Node = (ty.build)();
                for (k, v) in &n.props {
                    if !matches!(v, Prop::Enum(_)) {
                        continue;
                    }
                    // Gabor Texture is a 4.3 node: a 4.2 file has no storage for it.
                    if ty.idname == "ShaderNodeTexGabor" {
                        continue;
                    }
                    for (item, _) in enums::items(ty.idname, k) {
                        let mut m = new_node(ty.idname).unwrap();
                        m.set_prop(k, Prop::Enum((*item).to_string()));
                        update_sockets(&mut m);
                        match kind {
                            TreeKind::Shader => shader.nodes.push(m),
                            TreeKind::Compositor => comp.nodes.push(m),
                            _ => geo.nodes.push(m),
                        }
                    }
                }
            }
        }
        let mut s = scene_with(shader, comp);
        use_geometry_group(&mut s, geo);
        let r = round_trip(&s);
        let mut diffs = prop_diffs(s.materials[0].node_tree.as_ref().unwrap(), r.materials[0].node_tree.as_ref().unwrap());
        diffs.extend(prop_diffs(s.compositor.as_ref().unwrap(), r.compositor.as_ref().unwrap()));
        diffs.extend(prop_diffs(&s.node_groups[0], &r.node_groups[0]));
        diffs.sort();
        diffs.dedup();
        assert!(diffs.is_empty(), "{} props changed:\n{}", diffs.len(), diffs.join("\n"));
    }
}
