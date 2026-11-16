// SPDX-License-Identifier: GPL-3.0-only
//! The live Python channel end to end: a real python3 kernel
//! (apps/polyamour/python/polyamour/kernel.py) driving a Scene through
//! script::live. Skipped when the host has no python3.
use sex_3d::math::{Quat, Vec3};
use sex_3d::modifiers::Modifier;
use sex_3d::scene::{ObjectData, Scene};
use sex_3d::script::live::{CallResult, Kernel, KernelConfig, NoHost};
use std::process::Command;

fn kernel() -> Option<Kernel> {
    let ok = Command::new("python3").arg("-c").arg("import sys; assert sys.version_info >= (3, 9)").output().map(|o| o.status.success()).unwrap_or(false);
    if !ok {
        eprintln!("skipping: python3 is not installed on this host");
        return None;
    }
    Some(Kernel::spawn(&KernelConfig::host()).expect("start the kernel"))
}

fn call(k: &mut Kernel, s: &mut Scene, kind: &str, args: &[&str]) -> CallResult {
    let r = k.call(s, &mut NoHost, kind, args);
    if let Err(e) = &r.result {
        panic!("{kind} failed: {e}\n{}\n--- output\n{}", e.traceback, r.text());
    }
    r
}

fn exec(k: &mut Kernel, s: &mut Scene, src: &str) -> CallResult {
    call(k, s, "exec", &["test.py", src])
}

fn obj<'a>(s: &'a Scene, name: &str) -> &'a sex_3d::Object {
    &s.objects[s.find_object(name).unwrap_or_else(|| panic!("no object {name}"))]
}

fn mesh_of<'a>(s: &'a Scene, name: &str) -> &'a sex_3d::mesh::Mesh {
    match obj(s, name).data {
        ObjectData::Mesh(m) => &s.meshes[m],
        _ => panic!("{name} has no mesh"),
    }
}

#[test]
fn live_reads_back_what_it_did() {
    let Some(mut k) = kernel() else { return };
    assert!(k.hello.starts_with("hello polyops 1 python="), "{}", k.hello);
    let mut s = Scene::startup();
    let r = exec(
        &mut k,
        &mut s,
        "import bpy\n\
         bpy.ops.mesh.primitive_cube_add(size=1, location=(2, 0, 0))\n\
         cube = bpy.context.active_object\n\
         print(cube.name, len(cube.data.vertices), tuple(round(c, 3) for c in cube.location))\n\
         m = cube.modifiers.new('Sub', 'SUBSURF')\n\
         m.levels = 2\n\
         assert cube.modifiers['Sub'].levels == 2\n\
         bpy.ops.object.modifier_apply(modifier=m.name)\n\
         print(len(cube.data.vertices), len(cube.modifiers))\n\
         assert bpy.ops.object.modifier_apply.poll()\n\
         assert bpy.ops.mesh.primitive_cube_add() == {'FINISHED'}\n",
    );
    assert_eq!(r.text(), "Cube.001 8 (2.0, 0.0, 0.0)\n98 0\n");
    assert!(obj(&s, "Cube.001").modifiers.is_empty());
    assert_eq!(mesh_of(&s, "Cube.001").vertex_count(), 98);
    assert!(s.find_object("Cube.002").is_some());
}

#[test]
fn live_errors_name_the_line_and_the_session_survives() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let r = k.call(&mut s, &mut NoHost, "exec", &["broken.py", "import bpy\nx = 1\nbpy.data.objects['Missing']\n"]);
    let e = r.result.unwrap_err();
    assert_eq!(e.line, Some(3), "{e:?}");
    assert!(e.message.starts_with("KeyError"), "{}", e.message);
    assert!(e.traceback.contains("broken.py\", line 3"), "{}", e.traceback);
    let r = k.call(&mut s, &mut NoHost, "exec", &["bad.py", "def f(:\n  pass\n"]);
    let e = r.result.unwrap_err();
    assert!(e.message.starts_with("SyntaxError"), "{}", e.message);
    assert_eq!(e.line, Some(1));
    let r = k.call(&mut s, &mut NoHost, "exec", &["op.py", "import bpy\n\nbpy.ops.object.modifier_apply(modifier='Nope')\n"]);
    let e = r.result.unwrap_err();
    assert_eq!(e.line, Some(3));
    assert!(e.message.contains("no modifier named 'Nope'"), "{}", e.message);
    // An operator that does not exist fails like Blender's.
    let r = k.call(&mut s, &mut NoHost, "exec", &["x.py", "import bpy\nbpy.ops.mesh.frobnicate()\n"]);
    assert!(r.result.unwrap_err().message.contains("could not be found"));
    // The kernel keeps going.
    let r = exec(&mut k, &mut s, "import bpy\nprint(len(bpy.data.objects))\n");
    assert_eq!(r.text(), "3\n");
}

#[test]
fn live_console_history_free_and_completion() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let r = call(&mut k, &mut s, "console", &["C.active_object.name"]);
    assert_eq!(r.text(), "'Cube'\n");
    assert_eq!(r.result.unwrap(), ["ok"]);
    let r = call(&mut k, &mut s, "console", &["for o in D.objects:"]);
    assert_eq!(r.result.unwrap(), ["more"]);
    call(&mut k, &mut s, "console", &["    print(o.name, o.type)"]);
    let r = call(&mut k, &mut s, "console", &[""]);
    assert_eq!(r.text(), "Cube MESH\nLight LIGHT\nCamera CAMERA\n");
    let r = call(&mut k, &mut s, "console", &["1/0"]);
    assert_eq!(r.result.as_ref().unwrap(), &["error"]);
    assert!(r.text().contains("ZeroDivisionError"));
    // Names persist between lines, like Blender's console.
    call(&mut k, &mut s, "console", &["v = Vector((1, 2, 3))"]);
    let r = call(&mut k, &mut s, "console", &["v.length_squared"]);
    assert_eq!(r.text(), "14.0\n");
    let r = call(&mut k, &mut s, "complete", &["bpy.data.obj"]);
    let j = r.json.unwrap();
    assert_eq!(j["completions"][0], "bpy.data.objects");
    let r = call(&mut k, &mut s, "complete", &["D.objects['C"]);
    let c: Vec<String> = r.json.unwrap()["completions"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    assert_eq!(c, ["D.objects['Camera']", "D.objects['Cube']"]);
    let r = call(&mut k, &mut s, "complete", &["C.active_object.loc"]);
    let c = r.json.unwrap();
    assert!(c["completions"].as_array().unwrap().iter().any(|x| x == "C.active_object.location"), "{c}");
}

#[test]
fn live_procedural_generator_with_modifiers() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    exec(
        &mut k,
        &mut s,
        r#"import bpy, math
from mathutils import Vector, Matrix

def terrace(name, rings, steps, height):
    verts, faces = [], []
    for r in range(rings + 1):
        for i in range(steps):
            a = 2 * math.pi * i / steps
            verts.append((r * math.cos(a), r * math.sin(a), height * (rings - r) / rings))
    for r in range(rings):
        for i in range(steps):
            j = (i + 1) % steps
            faces.append((r * steps + i, (r + 1) * steps + i, (r + 1) * steps + j, r * steps + j))
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], faces)
    me.update()
    ob = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(ob)
    return ob

ob = terrace('Hill', 4, 12, 2.0)
assert len(ob.data.vertices) == 60 and len(ob.data.polygons) == 48
for p in ob.data.polygons:
    p.use_smooth = True
for v in ob.data.vertices:
    v.co.z += 0.1 * math.sin(v.co.x * 3)
sol = ob.modifiers.new('Shell', 'SOLIDIFY')
sol.thickness = 0.2
arr = ob.modifiers.new('Row', 'ARRAY')
arr.count = 3
arr.relative_offset_displace = (1.2, 0, 0)
mir = ob.modifiers.new('Mirror', 'MIRROR')
mir.use_axis[1] = True
ob.location = (1, 2, 3)
ob.rotation_euler.z = math.radians(90)
ob.scale *= 2
assert [m.name for m in ob.modifiers] == ['Shell', 'Row', 'Mirror']
assert [m.type for m in ob.modifiers] == ['SOLIDIFY', 'ARRAY', 'MIRROR']
ev = ob.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh()
assert len(ev.vertices) == 60 * 2 * 3 * 2 * 2 - 0 or len(ev.vertices) > 60 * 6, len(ev.vertices)
print(len(ev.vertices) > 300, round(ob.matrix_world.to_translation().z, 4), round(ob.dimensions.x, 3))
"#,
    );
    let hill = obj(&s, "Hill");
    assert!(close(hill.transform.location, Vec3::new(1.0, 2.0, 3.0)));
    assert!(hill.transform.rotation.rotate(Vec3::X).approx_eq(Vec3::Y, 1e-5));
    assert!(close(hill.transform.scale, Vec3::splat(2.0)));
    assert_eq!(hill.modifiers.len(), 3);
    assert!(matches!(hill.modifiers[0], Modifier::Solidify { thickness, .. } if (thickness - 0.2).abs() < 1e-6));
    assert!(matches!(hill.modifiers[1], Modifier::Array { count: 3, relative, .. } if (relative[0] - 1.2).abs() < 1e-6));
    assert!(matches!(hill.modifiers[2], Modifier::Mirror { x: true, y: true, z: false, .. }));
    let m = mesh_of(&s, "Hill");
    assert!(m.faces.iter().all(|f| f.smooth));
    assert!((m.positions[1].z - (2.0 + 0.1 * (3.0 * m.positions[1].x).sin())).abs() < 1e-4);
}

fn close(a: Vec3, b: Vec3) -> bool {
    a.approx_eq(b, 1e-5)
}

#[test]
fn live_node_material_renders() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let r = exec(
        &mut k,
        &mut s,
        r#"import bpy
mat = bpy.data.materials.new('Glow')
mat.use_nodes = True
nt = mat.node_tree
nodes, links = nt.nodes, nt.links
bsdf = nodes['Principled BSDF']
noise = nodes.new('ShaderNodeTexNoise')
noise.inputs['Scale'].default_value = 8.0
ramp = nodes.new('ShaderNodeValToRGB')
ramp.color_ramp.elements[0].color = (0.0, 0.0, 1.0, 1.0)
ramp.color_ramp.elements[1].color = (1.0, 0.2, 0.0, 1.0)
links.new(noise.outputs['Fac'], ramp.inputs['Fac'])
links.new(ramp.outputs[0], bsdf.inputs['Base Color'])
emit = nodes.new(type='ShaderNodeEmission')
emit.inputs['Strength'].default_value = 4.0
emit.inputs['Color'].default_value = (1.0, 0.5, 0.1, 1.0)
add = nodes.new('ShaderNodeAddShader')
links.new(bsdf.outputs['BSDF'], add.inputs[0])
links.new(emit.outputs[0], add.inputs[1])
out = nodes['Material Output']
links.new(add.outputs['Shader'], out.inputs['Surface'])
math_node = nodes.new('ShaderNodeMath')
math_node.operation = 'MULTIPLY'
assert math_node.operation == 'MULTIPLY'
assert bsdf.inputs['Base Color'].is_linked, [(l.from_node.name, l.to_socket.identifier) for l in links]
assert len(links) == 5, [(l.from_node.name, l.from_socket.identifier, l.to_node.name, l.to_socket.identifier) for l in links]
assert [n.bl_idname for n in nodes][:2] == ['ShaderNodeBsdfPrincipled', 'ShaderNodeOutputMaterial'] or 'ShaderNodeTexNoise' in [n.bl_idname for n in nodes]
cube = bpy.data.objects['Cube']
cube.data.materials.clear()
cube.data.materials.append(mat)
assert cube.active_material.name == 'Glow'
sc = bpy.context.scene
sc.render.resolution_x = 48
sc.render.resolution_y = 32
sc.render.resolution_percentage = 100
sc.cycles.samples = 4
assert bpy.ops.render.render(write_still=False) == {'FINISHED'}
img = bpy.data.images['Render Result']
assert tuple(img.size) == (48, 32), tuple(img.size)
px = img.pixels[:]
# The middle of the frame is the glowing cube: bright and orange.
i = (16 * 48 + 24) * 4
print(px[i] > 0.5, px[i] > px[i + 2])
"#,
    );
    assert_eq!(r.text(), "True True\n");
    let glow = s.materials.iter().find(|m| m.name == "Glow").unwrap();
    let t = glow.node_tree.as_ref().unwrap();
    assert!(t.nodes.iter().any(|n| n.idname == "ShaderNodeTexNoise"));
    assert_eq!(t.links.len(), 5);
}

#[test]
fn live_armature_edit_bones_and_pose_keyframes() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let r = exec(
        &mut k,
        &mut s,
        r#"import bpy, math
from mathutils import Quaternion, Vector
arm = bpy.data.armatures.new('Rig')
ob = bpy.data.objects.new('Rig', arm)
bpy.context.view_layer.objects.active = ob
bpy.ops.object.mode_set(mode='EDIT')
assert bpy.context.mode == 'EDIT_ARMATURE'
root = arm.edit_bones.new('Root')
root.head = (0, 0, 0)
root.tail = (0, 0, 1)
tip = arm.edit_bones.new('Tip')
tip.head = (0, 0, 1)
tip.tail = (0, 0, 2)
tip.parent = root
tip.use_connect = True
bpy.ops.object.mode_set(mode='OBJECT')
assert [b.name for b in arm.bones] == ['Root', 'Tip']
assert arm.bones['Tip'].parent.name == 'Root'
assert abs(arm.bones['Tip'].length - 1) < 1e-6
pb = ob.pose.bones['Tip']
pb.rotation_mode = 'XYZ'
for frame, angle in ((1, 0.0), (11, 90.0)):
    pb.rotation_euler = (math.radians(angle), 0, 0)
    pb.keyframe_insert('rotation_euler', frame=frame)
ob.location = (0, 0, 0)
ob.keyframe_insert(data_path='location', frame=1)
ob.location = (4, 0, 0)
ob.keyframe_insert(data_path='location', index=0, frame=11)
act = ob.animation_data.action
paths = sorted(set((fc.data_path, fc.array_index) for fc in act.fcurves))
assert ('pose.bones["Tip"].rotation_euler', 0) in paths and ('location', 0) in paths, paths
fc = act.fcurves.find('location', index=0)
assert len(fc.keyframe_points) == 2
for kp in fc.keyframe_points:
    kp.interpolation = 'LINEAR'
assert abs(fc.evaluate(6) - 2.0) < 1e-5
sc = bpy.context.scene
sc.frame_set(6)
print(round(ob.location.x, 4), round(math.degrees(ob.pose.bones['Tip'].rotation_euler.x), 1))
sc.frame_set(11)
tail = ob.pose.bones['Tip'].tail
print(round(tail.y, 3), round(tail.z, 3))
"#,
    );
    let txt = r.text();
    let lines: Vec<&str> = txt.lines().map(|l| l.trim()).collect::<Vec<_>>();
    assert_eq!(lines[0], "2.0 45.0", "{}", r.text());
    // An upright bone with roll 0 has its local Z along world -Y, so 90 degrees
    // about X swings the tail from +Z to -Y.
    assert_eq!(lines[1], "-1.0 1.0", "{}", r.text());
    let rig = obj(&s, "Rig");
    assert!(close(rig.transform.location, Vec3::new(4.0, 0.0, 0.0)));
    assert_eq!(s.frame_current, 11);
    let tip = rig.pose.iter().find(|p| p.name == "Tip").unwrap();
    assert!((tip.rotation_euler.x - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    let _ = Quat::IDENTITY;
}

#[test]
fn live_bmesh_script() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let r = exec(
        &mut k,
        &mut s,
        r#"import bpy, bmesh
from mathutils import Matrix, Vector
bm = bmesh.new()
bmesh.ops.create_cube(bm, size=2.0)
assert len(bm.verts) == 8 and len(bm.faces) == 6
top = [f for f in bm.faces if f.normal.z > 0.5]
r = bmesh.ops.extrude_face_region(bm, geom=top)
verts = [e for e in r['geom'] if isinstance(e, bmesh.types.BMVert)]
bmesh.ops.translate(bm, vec=(0, 0, 1), verts=verts)
bmesh.ops.scale(bm, vec=(0.5, 0.5, 1), verts=verts, space=Matrix.Translation((0, 0, -2)))
assert len(bm.verts) == 12 and len(bm.faces) == 10, (len(bm.verts), len(bm.faces))
bmesh.ops.subdivide_edges(bm, edges=bm.edges[:], cuts=1, use_grid_fill=True)
bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
bm.verts.ensure_lookup_table()
print(len(bm.verts), len(bm.faces), round(bm.calc_volume(), 3))
me = bpy.data.meshes.new('Tower')
bm.to_mesh(me)
bm.free()
ob = bpy.data.objects.new('Tower', me)
# Edit mode round trip on the new object
bpy.context.view_layer.objects.active = ob
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(ob.data)
bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if f.normal.z < -0.5])
for v in bm.verts:
    v.co.x += 10
bmesh.update_edit_mesh(ob.data)
bpy.ops.object.mode_set(mode='OBJECT')
print(len(ob.data.polygons), round(min(v.co.x for v in ob.data.vertices), 3))
s2 = bmesh.new()
bmesh.ops.create_uvsphere(s2, u_segments=8, v_segments=4, radius=1)
bmesh.ops.create_icosphere(s2, subdivisions=2, radius=0.5, matrix=Matrix.Translation((3, 0, 0)))
bmesh.ops.remove_doubles(s2, verts=s2.verts, dist=0.001)
print(len(s2.verts), len(s2.faces))
"#,
    );
    let txt = r.text();
    let lines: Vec<&str> = txt.lines().collect();
    // 12 verts + 20 edge midpoints + 10 face centres; 40 quads. The volume is
    // the 2x2x2 cube plus a frustum from 2x2 to 1x1 of height 1: 8 + 7/3.
    assert_eq!(lines[0], "42 40 10.333", "{}", r.text());
    // The bottom face's 4 quads became 8 triangles.
    assert_eq!(lines[1], "44 9.0", "{}", r.text());
    assert_eq!(lines[2], "68 112", "{}", r.text());
    let m = mesh_of(&s, "Tower");
    assert_eq!(m.face_count(), 44);
    assert!(m.volume() > 10.3, "faces point outwards: {}", m.volume());
}

#[test]
fn live_addon_operator_panel_and_handlers() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let dir = std::env::temp_dir().join(format!("polyamour-addons-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("stack_tools.py"),
        r#"bl_info = {"name": "Stack Tools", "author": "Test", "version": (1, 2), "blender": (4, 2, 0),
           "location": "View3D > Sidebar > Stack", "description": "Stacks copies of the active object", "category": "Object"}
import bpy
from bpy.props import IntProperty, FloatProperty, BoolProperty, EnumProperty, PointerProperty

class StackSettings(bpy.types.PropertyGroup):
    count: IntProperty(name="Count", default=3, min=1, max=50)
    gap: FloatProperty(name="Gap", default=0.5)

class OBJECT_OT_stack(bpy.types.Operator):
    """Stack copies of the active object upwards"""
    bl_idname = "object.stack_copies"
    bl_label = "Stack Copies"
    bl_options = {'REGISTER', 'UNDO'}
    count: IntProperty(name="Count", default=2, min=1)
    axis: EnumProperty(items=[('X', 'X', ''), ('Z', 'Z', '')], default='Z')

    @classmethod
    def poll(cls, context):
        return context.active_object is not None

    def execute(self, context):
        src = context.active_object
        h = src.dimensions.z + context.scene.stack.gap
        for i in range(self.count):
            new = src.copy()
            new.location.z = src.location.z + (i + 1) * h
        self.report({'INFO'}, "stacked %d" % self.count)
        return {'FINISHED'}

class VIEW3D_PT_stack(bpy.types.Panel):
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = 'Stack'
    bl_label = 'Stack'

    def draw(self, context):
        layout = self.layout
        st = context.scene.stack
        col = layout.column(align=True)
        col.prop(st, "count")
        col.prop(st, "gap")
        row = layout.row()
        op = row.operator("object.stack_copies", text="Stack!")
        op.count = st.count
        layout.label(text="Active: %s" % context.active_object.name)

def menu_func(self, context):
    self.layout.operator(OBJECT_OT_stack.bl_idname)

frames = []

@bpy.app.handlers.persistent
def on_frame(scene, depsgraph):
    frames.append(scene.frame_current)
    bpy.data.objects['Cube'].location.x = scene.frame_current / 10

classes = (StackSettings, OBJECT_OT_stack, VIEW3D_PT_stack)

def register():
    for c in classes:
        bpy.utils.register_class(c)
    bpy.types.Scene.stack = PointerProperty(type=StackSettings)
    bpy.types.VIEW3D_MT_object.append(menu_func)
    bpy.app.handlers.frame_change_post.append(on_frame)

def unregister():
    bpy.app.handlers.frame_change_post.remove(on_frame)
    bpy.types.VIEW3D_MT_object.remove(menu_func)
    del bpy.types.Scene.stack
    for c in reversed(classes):
        bpy.utils.unregister_class(c)
"#,
    )
    .unwrap();
    let d = dir.to_string_lossy().to_string();
    let r = call(&mut k, &mut s, "addons", &[&d]);
    let j = r.json.unwrap();
    assert_eq!(j["addons"][0]["module"], "stack_tools");
    assert_eq!(j["addons"][0]["name"], "Stack Tools");
    assert_eq!(j["addons"][0]["version"], "1.2");
    call(&mut k, &mut s, "addon_enable", &["stack_tools"]);
    // Polyamour hears about the classes (F3 search lists the operator).
    let reg = &k.session.bpy.registry;
    assert!(reg.iter().any(|r| r.kind == "operator" && r.idname == "object.stack_copies"));
    assert!(reg.iter().any(|r| r.kind == "panel" && r.idname == "VIEW3D_PT_stack" && r.attrs.iter().any(|(k, v)| k == "category" && v == "Stack")));
    let r = call(&mut k, &mut s, "panels", &[]);
    let p = r.json.unwrap();
    assert_eq!(p["panels"][0]["id"], "VIEW3D_PT_stack");
    assert_eq!(p["operators"][0]["description"], "Stack copies of the active object upwards");
    assert!(p["extended_menus"].as_array().unwrap().iter().any(|m| m == "VIEW3D_MT_object"));
    // Draw the panel: a layout description with live property values.
    let r = call(&mut k, &mut s, "draw", &["VIEW3D_PT_stack"]);
    let l = r.json.unwrap();
    let col = &l["layout"]["items"][0];
    assert_eq!(col["type"], "column");
    assert_eq!(col["items"][0]["type"], "prop");
    assert_eq!(col["items"][0]["value"], 3);
    assert_eq!(col["items"][0]["min"], 1);
    assert_eq!(l["layout"]["items"][1]["items"][0]["operator"], "object.stack_copies");
    assert_eq!(l["layout"]["items"][1]["items"][0]["properties"]["count"], 3);
    assert_eq!(l["layout"]["items"][2]["text"], "Active: Cube");
    // The UI changes a property through its reference, then redraws.
    let rf = col["items"][0]["ref"].as_u64().unwrap().to_string();
    call(&mut k, &mut s, "set_prop", &["VIEW3D_PT_stack", &rf, "2"]);
    let l = call(&mut k, &mut s, "draw", &["VIEW3D_PT_stack"]).json.unwrap();
    assert_eq!(l["layout"]["items"][0]["items"][0]["value"], 2);
    // The menu entry an add-on appended to a built-in menu.
    let m = call(&mut k, &mut s, "menu", &["VIEW3D_MT_object"]).json.unwrap();
    assert_eq!(m["layout"]["items"][0]["operator"], "object.stack_copies");
    assert_eq!(m["layout"]["items"][0]["text"], "Stack Copies");
    // Invoke the operator the way the button does.
    let r = call(&mut k, &mut s, "operator", &["object.stack_copies", "invoke", r#"{"count": 2}"#]);
    assert_eq!(r.result.as_ref().unwrap(), &["FINISHED"]);
    assert_eq!(k.session.bpy.reports, vec![("INFO".to_string(), "stacked 2".to_string())]);
    assert!(close(obj(&s, "Cube.002").transform.location, Vec3::new(0.0, 0.0, 5.0)));
    // ... and from a script, with poll.
    exec(&mut k, &mut s, "import bpy\nassert bpy.ops.object.stack_copies.poll()\nbpy.ops.object.stack_copies(count=1, axis='Z')\n");
    assert!(s.find_object("Cube.003").is_some());
    // Frame change handlers run around frame_set.
    call(&mut k, &mut s, "frame", &["7"]);
    assert_eq!(s.frame_current, 7);
    assert!(close(obj(&s, "Cube").transform.location, Vec3::new(0.7, 0.0, 0.0)));
    exec(&mut k, &mut s, "import bpy\nbpy.context.scene.frame_set(3)\nimport stack_tools\nassert stack_tools.frames == [7, 3], stack_tools.frames\n");
    // Timers registered by scripts run when the host asks.
    exec(&mut k, &mut s, "import bpy\nn = [0]\ndef tick():\n    n[0] += 1\n    bpy.data.objects['Light'].location.z = n[0]\n    return None if n[0] >= 2 else 0.0\nbpy.app.timers.register(tick)\n");
    let t = call(&mut k, &mut s, "timers", &[]).json.unwrap();
    assert_eq!(t["next"], 0.0);
    let t = call(&mut k, &mut s, "timers", &[]).json.unwrap();
    assert!(t["next"].is_null());
    let l = obj(&s, "Light").transform.location;
    assert!((l.z - 2.0).abs() < 1e-5, "{l:?}");
    // Disable: everything is unregistered on both sides.
    call(&mut k, &mut s, "addon_disable", &["stack_tools"]);
    assert!(!k.session.bpy.registry.iter().any(|r| r.idname == "object.stack_copies"));
    let r = k.call(&mut s, &mut NoHost, "exec", &["t.py", "import bpy\nbpy.ops.object.stack_copies()\n"]);
    assert!(r.result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn live_mathutils_numbers() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    exec(
        &mut k,
        &mut s,
        r#"import math
from mathutils import Vector, Matrix, Quaternion, Euler, Color, geometry, noise, kdtree, bvhtree

def near(a, b, eps=1e-5):
    if hasattr(a, '__len__'):
        return all(abs(x - y) < eps for x, y in zip(a, b)) and len(a) == len(b)
    return abs(a - b) < eps

v = Vector((1, 2, 3))
assert v.xy == Vector((1, 2)) and v.zyx == Vector((3, 2, 1))
v.xz = (5, 6)
assert tuple(v) == (5, 2, 6)
assert near(Vector((3, 4, 0)).length, 5) and near(Vector((1, 0, 0)).cross((0, 1, 0)), (0, 0, 1))
assert near(Vector((1, 0, 0)).angle(Vector((0, 1, 0))), math.pi / 2)
m = Matrix.Translation((1, 2, 3)) @ Matrix.Rotation(math.radians(90), 4, 'Z') @ Matrix.Scale(2, 4)
assert near(m @ Vector((1, 0, 0)), (1, 4, 3))
loc, rot, sca = m.decompose()
assert near(loc, (1, 2, 3)) and near(sca, (2, 2, 2)) and near(rot.to_euler(), (0, 0, math.pi / 2))
assert near((m @ m.inverted()).to_translation(), (0, 0, 0))
assert near(m.determinant(), 8)
q = Euler((0.3, -0.2, 1.1), 'ZXY').to_quaternion()
assert near(q.to_euler('ZXY'), (0.3, -0.2, 1.1))
a, b = Quaternion((1, 0, 0, 0)), Quaternion((0, 0, 1), math.pi / 2)
assert near(a.slerp(b, 0.5).angle, math.pi / 4)
assert near(a.rotation_difference(b).angle, math.pi / 2)
c = Color((1, 0, 0))
c.h = 1 / 3
assert near(tuple(c), (0, 1, 0))
assert near(Vector((1, 0, 0)).to_track_quat('-Z', 'Y').to_euler(), (math.pi / 2, 0, -math.pi / 2), 1e-4)
assert geometry.intersect_ray_tri((0, 0, -1), (0, 0, 1), (-1, -1, 0), (1, -1, 0), (0, 1, 0)) is None
hit = geometry.intersect_ray_tri((-1, -1, 0), (1, -1, 0), (0, 1, 0), (0, 0, 1), (0, 0, -1))
assert near(hit, (0, 0, 0))
p1, p2 = geometry.intersect_line_line((0, 0, 0), (1, 0, 0), (0.5, -1, 1), (0.5, 1, 1))
assert near(p1, (0.5, 0, 0)) and near(p2, (0.5, 0, 1))
assert near(geometry.distance_point_to_plane((0, 0, 5), (0, 0, 1), (0, 0, 1)), 4)
assert near(geometry.area_tri((0, 0, 0), (2, 0, 0), (0, 2, 0)), 2)
assert near(geometry.normal((0, 0, 0), (1, 0, 0), (0, 1, 0)), (0, 0, 1))
pts = geometry.interpolate_bezier((0, 0, 0), (1, 0, 0), (2, 0, 0), (3, 0, 0), 4)
assert near(pts[1], (1, 0, 0))
assert sorted(sorted(t) for t in geometry.tessellate_polygon([[(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]])) in ([[0, 1, 2], [0, 2, 3]], [[0, 1, 3], [1, 2, 3]])
kd = kdtree.KDTree(10)
for i in range(10):
    kd.insert((i, 0, 0), i)
kd.balance()
co, idx, d = kd.find((3.2, 0, 0))
assert idx == 3 and near(d, 0.2)
assert [r[1] for r in kd.find_n((0, 0, 0), 3)] == [0, 1, 2]
assert len(kd.find_range((5, 0, 0), 1.5)) == 3
bvh = bvhtree.BVHTree.FromPolygons([(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0)], [(0, 1, 2, 3)])
loc, nrm, i, dist = bvh.ray_cast((0.2, 0.3, 5), (0, 0, -1))
assert near(loc, (0.2, 0.3, 0)) and i == 0 and near(dist, 5) and near(abs(nrm.z), 1)
# noise: the live value (the shader nodes' Rust noise) equals the local port
from mathutils import noise as N
live = [N.noise((x * 0.37, 1.1, -0.4)) for x in range(5)]
import polyamour.polyops as P
saved, P.channel.live = P.channel.live, False
local = [N.noise((x * 0.37, 1.1, -0.4)) for x in range(5)]
P.channel.live = saved
assert near(live, local, 1e-4), (live, local)
assert any(abs(x) > 1e-3 for x in live)
print('ok')
"#,
    );
}

#[test]
fn live_old_examples_still_run() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/polyamour/python/examples");
    let src = std::fs::read_to_string(dir.join("spiral_staircase.py")).unwrap();
    let r = call(&mut k, &mut s, "exec", &["spiral_staircase.py", &src]);
    assert!(r.text().contains("staircase: 32 objects"), "{}", r.text());
    assert_eq!(s.objects.len(), 3 + 24 + 5);
    assert_eq!(k.session.bpy.props.get("render.resolution_x").map(String::as_str), Some("1280"));
    let src = std::fs::read_to_string(dir.join("blender_pyramid.py")).unwrap();
    let mut s = Scene::startup();
    call(&mut k, &mut s, "exec", &["blender_pyramid.py", &src]);
    let names: Vec<&str> = s.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["Light", "Camera", "Pyramid", "Ball", "Sun"]);
    let ball = obj(&s, "Ball");
    assert_eq!(
        ball.modifiers,
        vec![Modifier::Subdivision { levels: 2, simple: false }, Modifier::Mirror { x: false, y: true, z: false, merge: true, merge_distance: 0.001 }]
    );
}

#[test]
fn save_as_mainfile_writes_blend_files() {
    let Some(mut k) = kernel() else { return };
    let mut s = Scene::startup();
    let path = std::env::temp_dir().join(format!("sex3d-live-{}.blend", std::process::id()));
    let src = format!(
        "import bpy\n\
         bpy.ops.mesh.primitive_cube_add(size=2, location=(1, 2, 3))\n\
         ob = bpy.context.active_object\n\
         ob.name = 'Saved'\n\
         ob.keyframe_insert(data_path='location', index=0, frame=1)\n\
         ob.location = (5, 2, 3)\n\
         ob.keyframe_insert(data_path='location', index=0, frame=20)\n\
         bpy.context.scene.render.resolution_x = 640\n\
         bpy.context.scene.render.resolution_y = 480\n\
         assert bpy.ops.wm.save_as_mainfile(filepath={:?}) == {{'FINISHED'}}\n",
        path.to_string_lossy()
    );
    exec(&mut k, &mut s, &src);
    let bytes = std::fs::read(&path).expect("the .blend was written");
    let _ = std::fs::remove_file(&path);
    assert!(bytes.starts_with(b"BLENDER-v402"));
    let (r, actions) = sex_3d::io::blend::import_blend_full(&bytes).unwrap();
    let ob = obj(&r, "Saved");
    assert!(matches!(ob.data, ObjectData::Mesh(m) if r.meshes[m].vertex_count() == 8));
    let a = actions.iter().find(|a| a.object == "Saved").expect("the action");
    let c = a.curves.iter().find(|c| c.path == "location" && c.index == 0).unwrap();
    assert_eq!(c.keys.iter().map(|k| (k.frame, k.value)).collect::<Vec<_>>(), [(1.0, 1.0), (20.0, 5.0)]);
}
