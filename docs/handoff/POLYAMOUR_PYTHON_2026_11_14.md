# Polyamour Python scripting (Z7, first cut): polyops 1

Python scripts drive Polyamour through `import polyamour` (or plain `import bpy`).
A script cannot link into the Polyamour process, so the package writes a text
command stream, **polyops**, that `crates/sex-3d/src/script.rs` executes against a
`Scene`.

| Piece | Where |
|---|---|
| Interpreter, scene writer | `crates/sex-3d/src/script.rs` |
| Tests (every command, errors, round trip, python3 end to end) | `crates/sex-3d/tests/script.rs` |
| Python API | `apps/polyamour/python/polyamour/{__init__,polyops,types}.py` |
| Blender aliases | `apps/polyamour/python/bpy/`, `apps/polyamour/python/mathutils/` |
| Examples | `apps/polyamour/python/examples/{spiral_staircase,blender_pyramid}.py` |
| Freezing into `python` | `scripts/python_freeze.py` (`POLYAMOUR` list) → `apps/python/frozen/` |

## How Polyamour runs a script

1. Write the current scene with `script::scene_to_polyops(&scene)` to a file and
   pass its path as `POLYAMOUR_SCENE`. The package loads it at import, so
   `bpy.data.objects['Cube']`, `obj.location`, `context.active_object`, mesh
   vertices etc. read real values.
2. Start `python SCRIPT` with `POLYAMOUR_OPS=<path>`. Without `POLYAMOUR_OPS` the
   commands go to standard output and the script's own `print` output is moved
   to standard error, so either stream can be the channel.
3. When the process exits, read the file and call `script::run(&mut scene, &text)`
   after `undo.push("Run Script", &scene)`. `run` is atomic (it works on a copy),
   so one script is one undo step, and a failed run changes nothing.
4. Check `report.complete`: the package always ends the stream with `end`, or with
   `abort "KeyError: 'Missing'"` when the script raised (which makes `run` fail).
   A stream without either was cut short (the interpreter died): do not apply it.
5. `report.render` holds `render k=v` settings (the `Scene` has no render settings
   yet), `report.created` the names of new objects.

## Rust API (`sex_3d::script`)

```rust
pub const VERSION: u32 = 1;
pub fn run(scene: &mut Scene, src: &str) -> Result<RunReport, ScriptError>;
pub fn scene_to_polyops(scene: &Scene) -> String;
pub fn mesh_to_options(m: &Mesh) -> String;        // " verts=... faces=... uvs=..."
pub fn quote(s: &str) -> String;                   // a name as one token
pub fn quat_to_euler_xyz(q: Quat) -> [f32; 3];     // Blender XYZ Euler
pub struct RunReport { version, commands, created: Vec<String>, replies: Vec<String>,
                       render: BTreeMap<String, String>, complete: bool }
pub struct ScriptError { line: usize, message: String }   // Display: "line N: message"
pub struct Session;  // new(), exec_line(&mut scene, line), take_replies(), finish()
```

`Session` is the line-at-a-time engine under `run`, for a future live channel
(feed a line, take its reply lines, send them back).

## The language

* UTF-8 text, one command per line (`\n` or `\r\n`). Blank lines are ignored; an
  unquoted token starting with `#` comments out the rest of the line.
* The first command is `polyops 1`. A higher version is refused with an error.
* Tokens are separated by blanks. `"..."` quotes, with escapes `\" \\ \n \t \r`;
  bare and quoted pieces join (`a"b c"` is `ab c`). `key=value` (key an
  identifier, `=` outside quotes) is an option, anything else a positional
  argument. Unknown or repeated options are errors, so typos never pass silently.
* Lists are comma-separated: `location=1,2,3`. Numbers must be finite.
  Booleans are `1/0`, `true/false`, `yes/no`, `on/off`.
* Mesh arrays are text (`verts=0,0,0,1,0,0,...`, `faces=0,1,2;2,3,0`,
  `edges=0,1;1,2`) or `b64:` + base64 of little-endian `f32` (verts, uvs) /
  `u32` (faces as runs `n, v0..vn-1`; edges as pairs). Writers switch to base64
  above 64 vertices.
* Names: objects, meshes, materials and collections are referenced by name, or by
  a **handle**: `as=$7` on a creating command binds the unquoted token `$7` to
  what was made, whatever unique name it received (`Cube.001`). Handles follow
  renames and are unbound by `delete`. Quoted text is never a handle, never the
  keyword `none` and never an option. The Python package always uses handles for
  what it creates, so its predictions of unique names can never mis-address.
* Properties are written `OBJECT.property`, split at the **last** dot, so
  `Cube.001.location` is object `Cube.001`, property `location`.
* Request/response: `@ID` before the command word (`@7 get Cube.location`) asks
  for a reply `@7 ok 1 2 3`; creating commands answer with the name they gave.
  Queries (`get`, `list`, `exists`, `ping`) always reply (`ok ...`). Replies are
  collected in `RunReport::replies`; today's file stream is one-way, so scripts
  read the `POLYAMOUR_SCENE` mirror instead. Any error stops the run and names
  its 1-based line: `line 12: no object named 'Cueb'`.

### Commands

Placement options `location=x,y,z rotation=x,y,z` (XYZ Euler, radians)
`quaternion=w,x,y,z scale=s|x,y,z collection=C visible=0|1 as=$H` are accepted by
every command that creates an object. A new object becomes active and the only
selected one (Blender's rule).

| Command | Meaning |
|---|---|
| `polyops 1` | header |
| `end` / `abort MESSAGE...` | complete stream / fail the whole run |
| `add_primitive KIND [name=]` | `cube size=2`, `plane size=2`, `grid x_subdivisions=10 y_subdivisions=10 size=2`, `uv_sphere segments=32 rings=16 radius=1`, `ico_sphere subdivisions=2 radius=1`, `cylinder segments=32 radius=1 depth=2`, `cone segments=32 radius1=1 radius2=0 depth=2`, `torus major_segments=48 minor_segments=12 major_radius=1 minor_radius=0.25` |
| `add_mesh [NAME\|name=] verts= [faces=] [edges=] [uvs=] [smooth=] [mat=]` | new mesh + object; `uvs` per face corner, `smooth` `1`/`0` or one digit per face, `mat` slot per face |
| `mesh_new NAME [as=] [mesh options]` | mesh datablock only (replies its name) |
| `add_object NAME data=MESH\|none` / `add_empty NAME` | object using an existing mesh / an empty |
| `light_add point\|sun\|spot\|area [name=] power= color=r,g,b radius= angle= blend= size=` | light object (spot: angle/blend, area: size) |
| `camera_add [name=] fov= \| lens= ortho_scale= near= far= active=0\|1` | camera; becomes the scene camera if there is none (or `active=1`) |
| `mesh_set OBJ\|MESH [mesh options]` | replace a mesh's data (every user sees it) |
| `mesh_edit OBJ\|MESH OP ... [indices=i,j] [center=x,y,z]` | `translate_verts dx dy dz`, `scale_verts s\|sx sy sz`, `rotate_verts rx ry rz`, `set_verts verts=...`, `flip_normals`, `shade_smooth`, `shade_flat`, `weld [dist]`, `set_material SLOT [faces=...]` |
| `set OBJ.PROP VALUES` | `location`, `rotation_euler`, `rotation_quaternion` (w x y z), `scale`, `visible`, `name`, `collection`, `parent`; lights `power color radius angle blend size`; cameras `fov lens near far ortho_scale` (`none` = perspective). Values as `1 2 3` or `1,2,3` |
| `get OBJ.PROP` | the same plus `type`, `data`, `vertex_count`, `face_count`, `materials`, `modifiers`, `matrix_world` |
| `parent CHILD PARENT\|none [keep_transform=1]` | default keeps the local transform (like `obj.parent = p` in Python); cycles are errors |
| `material_new NAME [as=] base_color=r,g,b[,a] metallic= roughness= emission=r,g,b emission_strength= ior= transmission= double_sided=` | names made unique |
| `material_set MAT [name=] [same options]` | change a material |
| `material_assign OBJ MAT [slot=N]` / `material_clear OBJ` | append (or set) a material slot / clear slots |
| `modifier_add OBJ TYPE [options]` | `Mirror x y z merge merge_distance`, `Subdivision levels simple` (also `SUBSURF`), `Array count relative=x,y,z constant=x,y,z merge`, `Solidify thickness offset`, `Bevel width segments angle`, `Triangulate`, `Weld distance`, `Displace strength`, `Smooth factor iterations`; Blender defaults |
| `modifier_set OBJ INDEX [options]` / `modifier_remove OBJ INDEX` / `modifier_clear OBJ` | edit the stack |
| `select OBJ...` / `select_add OBJ...` / `deselect [OBJ...]` / `select_all` | selection (deselect with no names clears) |
| `active OBJ\|none` / `active_camera CAM\|none` | active object / scene camera |
| `delete OBJ...` | remove objects (children move to the parent) |
| `collection_new NAME [parent=C] [visible=] [as=]` / `collection_set C [name=] [parent=C\|none] [visible=]` / `link OBJ C` | collections (an object is in one) |
| `world color=r,g,b strength=` / `scene name= frame_start= frame_end= fps=` | world, scene settings |
| `render key=value...` | stored into `RunReport::render` |
| `get` `list objects\|meshes\|materials\|collections\|selected` `exists OBJ` `ping` | queries |

`scene_to_polyops` uses `scene`, `world`, `collection_new/_set`, `material_new`,
`mesh_new`, `add_object/add_empty/light_add/camera_add` (with `quaternion=` so
rotations are exact), `parent`, `material_assign slot=`, `modifier_add`,
`select`, `active`, `active_camera`, `end`. Floats are written in Rust's shortest
round-trip form, so `run(Scene::new(), scene_to_polyops(s)) == s` (tested on
`Scene::startup()` plus collections, nested parents, UVs, smooth flags, face
materials, loose edges, every light kind, an ortho camera, modifiers, hidden
objects). Not carried: images, material textures, the world HDRI; duplicate
material names come back made unique.

## Python API

`import polyamour as pa` (or `import bpy`; `from mathutils import Vector`):

* `pa.data.objects` (`[name]`, `.get`, iteration, `.new(name, data)`,
  `.remove(obj)`), `meshes.new`, `materials.new`, `collections.new`,
  `lights.new(name, type)`, `cameras.new`.
* Objects: `name`, `type`, `data`, `location` / `rotation_euler` / `scale`
  (live vectors: `obj.location.x = 2`, `obj.location += Vector(...)`), `parent`,
  `children`, `modifiers.new(name, type)` with Blender types `SUBSURF MIRROR
  ARRAY SOLIDIFY BEVEL TRIANGULATE WELD DISPLACE SMOOTH` and their Blender
  attribute names (`levels`, `use_axis[1]`, `relative_offset_displace`,
  `merge_threshold`, `angle_limit`, ...), `active_material`, `material_slots`,
  `select_set/select_get`, `hide_set/hide_viewport`.
* Meshes: `from_pydata(verts, edges, faces)`, `vertices[i].co` (read/write),
  `polygons`, `materials.append(mat)`, `shade_smooth()`.
* Materials: `base_color` / `diffuse_color`, `metallic`, `roughness`, `ior`,
  `transmission`, `emission_color`, `emission_strength`, and
  `node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value`.
* Lights: `energy`, `color`, `shadow_soft_size`, `spot_size`, `spot_blend`,
  `size`. Cameras: `lens`, `angle`, `clip_start`, `clip_end`, `type`, `ortho_scale`.
* `pa.context`: `scene` (`objects`, `collection`, `camera`, `frame_start/end`,
  `render.*`, `world.color/strength`), `active_object`/`object`,
  `selected_objects`, `collection`, `view_layer.objects.active`.
* `pa.ops.mesh.primitive_{cube,plane,grid,uv_sphere,ico_sphere,cylinder,cone,torus}_add`
  (Blender arguments incl. `location`, `rotation`, `scale`), `pa.ops.object.`
  `light_add`, `camera_add`, `empty_add`, `select_all`, `delete`, `shade_smooth`,
  `shade_flat`, `modifier_add`.

Only the stdlib modules in `apps/python/frozen/MANIFEST` are used (`os`, `sys`,
`atexit`, `math`, `base64`, `struct`).

## Getting it onto SexOS

`apps/python` freezes pure-Python modules into the binary from
`apps/python/frozen/*.bin` + `MANIFEST` (build.rs turns them into CPython's
frozen tables). `scripts/python_freeze.py` now also compiles `polyamour`,
`polyamour.polyops`, `polyamour.types`, `bpy` and `mathutils` from
`apps/polyamour/python`, and the regenerated `apps/python/frozen/` is committed:
every `python` on SexOS can `import bpy` with nothing in its folder. The
standard-library `.bin` files came out byte-identical (checked), so only five
new files and five `MANIFEST` lines changed.

**After editing anything under `apps/polyamour/python/polyamour|bpy|mathutils`,
refreeze**, with any CPython 3.13 (bytecode is per release series):

    mkdir -p /tmp/cp && ln -sfn /usr/local/lib/python3.13 /tmp/cp/Lib   # or a v3.13.18 checkout
    python3.13 scripts/python_freeze.py /tmp/cp
    cargo build -p python --target <SexOS target>   # rebuilds the frozen tables

Without refreezing, an edited package still works when its folder is next to a
script: sys.path holds the script's directory and `/lib`, but frozen modules are
found first, so a stale frozen copy wins. (Frozen import of the packages was
checked on the host by loading the `.bin` files through a finder like
FrozenImporter and running `examples/blender_pyramid.py`.)

## Weak spots / next steps

* One-way today: replies exist (`@ID`, queries) but nothing carries them back to
  the script yet; reads come from the `POLYAMOUR_SCENE` snapshot taken at start.
* Primitives created by `pa.ops` have no geometry in the Python mirror
  (`len(obj.data.vertices) == 0`) until the script sets it.
* `bpy.data.objects.new` adds the object to the scene at once (Polyamour has no
  unlinked objects); `collection.objects.link` *moves* it (one collection per object).
* Material slots live on objects; `mesh.materials.append` updates every current
  user of the mesh and later users created through `objects.new`.
* `sys.exit(1)` is not treated as failure (no `abort`); only uncaught exceptions are.
* No `matrix_world`/`Matrix`, keyframes, bmesh, edit-mode operators, drivers,
  node graphs beyond the Principled shim, or images/textures.
* Not yet run inside SexOS itself (needs a `python` rebuild with musl headers and
  a gate like `scripts/python_gate.py` typing `run python stairs.py`).
