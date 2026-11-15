# Polyamour Python scripting: live kernel, bpy, add-ons (polyops 1)

Python scripts drive Polyamour through `import bpy` (or `import polyamour`),
`mathutils`, `bmesh` and `bpy_extras`. Python runs in its own process and
talks to Polyamour in **polyops**, a line language that
`crates/sex-3d/src/script.rs` executes against a `Scene`. Since phase 5 the
channel is **live**: a long-running Python *kernel* sends a request for every
read and write and waits for the reply, so a script reads back what it just
did, as in Blender.

| Piece | Where |
|---|---|
| polyops interpreter, scene writer | `crates/sex-3d/src/script.rs` (+ `script/{rig,nodes}.rs`) |
| bpy commands (dumps, Blender-idname operators, keyframes, registry, reports, noise) | `crates/sex-3d/src/script/bpy.rs` |
| Live channel, batch fallback | `crates/sex-3d/src/script/live.rs` |
| The kernel | `apps/polyamour/python/polyamour/kernel.py` |
| Python API | `apps/polyamour/python/{polyamour,bpy,bpy_extras,mathutils,bmesh}/` |
| App integration (console, Run Script, add-ons, panels, F3, handlers) | `apps/polyamour/src/python.rs`, `editors/{simple,text}.rs`, hooks in `app.rs`, `doc.rs`, `editors/view3d/sidebar.rs`, `editors/properties/mod.rs` |
| Templates (ours) and examples | `apps/polyamour/python/templates/`, `apps/polyamour/python/examples/` |
| Tests | `crates/sex-3d/tests/python_live.rs` (10), `crates/sex-3d/tests/script.rs` (`python_*`, 4), `apps/polyamour/src/python.rs` (4), `editors/simple.rs` (console, 2) |
| Freezing into SexOS's `python` | `scripts/python_freeze.py` (`POLYAMOUR` list) -> `apps/python/frozen/` |

## The live protocol (`sex_3d::script::live`)

Polyamour starts `python3 -u -I -c BOOT` (BOOT puts the package folders on
`sys.path` and runs `polyamour.kernel.main()`), with `POLYAMOUR_LIVE=1`. The
kernel keeps the original stdin/stdout for the protocol and gives running code
an empty stdin and `sys.stdout`/`sys.stderr` that become `out`/`err` lines.
Every line is one message; tokens are polyops tokens (quoted when needed).

```text
python -> host   hello polyops 1 python=3.13.18
host   -> python call 7 exec "script.py" "import bpy\n..."
python -> host   @1 op mesh.primitive_cube_add size=2      (a request: any polyops command)
host   -> python @1 ok Cube.001                           (or: @1 err "message")
python -> host   @2 dump_object Cube.001
host   -> python @2 ok =json {"name":"Cube.001",...}
python -> host   out "made Cube.001\n"                    (err "..." for stderr)
python -> host   done 7 ok                                (done 7 json {...} / done 7 error MSG line=N traceback=TEXT)
host   -> python quit
```

* Call kinds: `exec NAME SOURCE`, `console LINE` (answers `ok`, `more` or
  `error`), `complete TEXT` (json `{prefix, completions}`), `draw PANEL`,
  `menu MENU`, `panels` (registered panels, operators, menus, appended menus,
  keymaps, handler counts, timer count), `operator IDNAME invoke|execute|poll
  PROPS-JSON`, `set_prop PANEL REF VALUE-JSON`, `modal EVENT-JSON`,
  `handler NAME`, `frame N`, `timers` (json `{next}`), `addons DIR...`,
  `addon_enable M`, `addon_disable M`, `reset`, `ping`.
* While a call runs, each request gets exactly one reply before Python goes
  on. One `Session` lives as long as the kernel, so `as=$N` handles and the
  `bpy::State` (selection, edit-mode selection, render props, registered
  classes, reports) persist across calls. A `Host` gets the first look at each
  request; `NoHost` serves everything from the scene.
* The kernel caches read replies (JSON dumps) until the next write, and drops
  the cache at the start of every call (the host may have undone or rolled
  back in between). Local mesh edits (`vertices[i].co`, `foreach_set`, uv,
  smooth, seams...) are kept in Python and flushed as one `mesh_set` /
  `mesh_attr` / `mesh_select` before the next request.
* Rust API: `Kernel::spawn(&KernelConfig) -> io::Result<Kernel>`,
  `Kernel::with_wire(Box<dyn Wire>)` (any line transport), `kernel.call(&mut
  scene, &mut dyn Host, kind, &[args]) -> CallResult { printed, result:
  Result<Vec<String>, CallError { message, line, traceback }>, json, requests }`,
  `kernel.session` (the `Session`), `run_script(&mut scene, name, src)` (one
  shot, all or nothing), `live::json` (serde_json re-exported).
* Undo is the caller's: Polyamour runs a call that may change the scene on a
  copy and keeps the copy as **one undo step** when the call succeeded and the
  scene changed; a failed call changes nothing.

## SexOS: starting another program and talking to it

What a SexOS program can do today (`patches/rust-std/process_sexos.rs`, over
`crates/sex-std/src/process.rs` and `sex_app::launch`):

* `std::process::Command::new("python")` starts `/bin/python` (a bare name,
  `/bin/NAME`, or `@/path` of a program file the run holds). **Arguments**
  are passed (as one quoted line). `spawn()`, `status()`, `wait()`,
  `try_wait()`, `kill()` and `id()` work; the exit code comes back.
* **No environment variables** and **no `current_dir`** reach the child.
  `SEXOS_RUN_WINDOWED=1` / `SEXOS_RUN_NETWORK=1` in `Command::env` are read as
  flags (a window; network), nothing else.
* **No pipes**: `Stdio::piped()` fails with `Unsupported`, so `output()` fails
  unless both streams are `null()` or `inherit()`. A child's output otherwise
  goes to the serial record only.
* So the only data channel is **files**, named in the arguments, read after
  the child ended (or polled while it runs). Whether a child sees every file
  of its parent depends on what the run holds; Polyamour keeps its files under
  `/data/polyamour/run`.

Consequences for Polyamour on SexOS: `Kernel::spawn` fails with `Unsupported`
(no pipes), and Polyamour uses the **batch path**:

1. `live::batch_snapshot(&scene)` writes the replies to the start-of-run reads
   (`ids`, `dump_scene`, `op_list`, every object, material, material node tree,
   mesh up to 100k vertices, armature) as JSON `[[request tokens], reply]`.
2. `live::batch_command(&cfg, script, ops, snapshot)` runs
   `python -I -c BOOT OPS SNAPSHOT SCRIPT` (BOOT = `live::batch_boot`, calling
   `polyops.run_batch`); everything travels as arguments.
3. The script's commands go to OPS; creating commands get `as=$N` handles in
   place of names; `print` output goes into OPS as `#> "text"` (stdout) /
   `#! "text"` (stderr) comment lines (`live::batch_output`); an uncaught
   exception ends the stream with `abort "line N: Kind: message"`.
4. `live::batch_apply(&mut scene, ops)` applies it all or nothing; Polyamour
   pushes one undo step.

Batch limits: reads see the scene as it was when the script started (a script
cannot read back an object it just made, e.g. `context.active_object` after an
operator, or a new material's node tree); the console, add-ons, script panels,
handlers and timers need the live kernel and say so. Info and the status bar
say "Python runs scripts in batch mode: live Python needs pipes between
programs, which this system does not have yet".

**Dropping the live channel in later:** the protocol only needs a line
transport (`trait Wire { send, recv, diagnostics, close }`). When SexOS gets
pipes, `Kernel::spawn` works unchanged. Without pipes, a `Wire` over two files
(the host appends calls to one, the kernel appends requests to the other, each
side polling with `try_wait` for liveness) would carry the same protocol;
`kernel.main()` takes any pair of text streams.

## In the app

* **Text Editor**: Run Script (header button, Text > Run Script, **Alt+P**, or
  Ctrl+Enter). A text starting with `polyops 1` runs in-process; anything else
  runs in the kernel as one undo step. An error changes nothing; the traceback
  goes to Info as output lines and `script.py: line N: KeyError: ...` to Info
  and the status bar. Templates > Python: Operator Simple, UI Panel Simple,
  Add-on Add Object, Bmesh Simple, Handlers and Timers, Pyramid, Spiral
  Staircase (all ours).
* **Python Console**: Blender's banner and colours, `>>> ` / `... ` prompts
  with auto-indent (an indent-only line ends the block), Up/Down history,
  **Tab** (indent at line start, otherwise complete) and **Ctrl+Space**
  completion from the live namespace (attributes, `D.objects['` keys,
  builtins), `C = bpy.context`, `D = bpy.data`, `from mathutils import *`,
  `from math import *`. Results are echoed (`repr`), errors print the
  traceback in red. Each line that changed the scene is one undo step. Without
  live Python the console falls back to polyops lines.
* **Add-ons** (Preferences > Add-ons): add-ons are `.py` modules or package
  folders with `bl_info` (or `blender_manifest.toml`) in the add-ons folder:
  `/data/polyamour/addons` on SexOS, `$XDG_DATA_HOME/polyamour/addons` or
  `~/.local/share/polyamour/addons` on a host (`POLYAMOUR_DATA` overrides).
  The checkbox enables/disables (`register()`/`unregister()`), the list is kept
  in `addons_enabled.txt` next to it and enabled again at the next start.
* **F3** lists script operators after the built-in entries; running one is
  `bpy.ops.X.Y('INVOKE_DEFAULT')` (poll, invoke, execute, modal).
* **Script panels**: `bl_space_type='VIEW_3D'`, `bl_region_type='UI'` panels
  appear as sidebar (N) tabs by `bl_category` (default "Misc");
  `bl_space_type='PROPERTIES'` panels appear under the tab of their
  `bl_context`. `draw()` is recorded into a layout description (rows, columns,
  boxes, splits, labels, props with kind/min/max/enum items, operator buttons
  with properties, menus, lists) that Polyamour draws with its own widgets;
  editing a prop sends `set_prop`, a button runs the operator. Sub-panels
  (`bl_parent_id`), `poll()`, `HIDE_HEADER`, `DEFAULT_CLOSED` are honoured.
* **Handlers**: `frame_change_pre/post` run when the frame changes (only when a
  handler is registered), `depsgraph_update_post` after a call changed
  something; `load_*`/`save_*` can be called with `handler NAME`.
  **`bpy.app.timers`** run from the window loop (the window keeps ticking
  while timers wait). Handlers and timers make no undo steps, as in Blender.

## Python API coverage

* `bpy.data`: objects, meshes, materials, collections, images, node_groups,
  armatures, lattices, actions, lights, cameras, scenes, worlds, texts with
  `new/remove/get/[name]/[index]/iteration/foreach_get`. `bpy.context`:
  scene, view_layer, collection, active_object/object, selected_objects,
  mode, active_bone, selected pose bones, area, `temp_override`,
  `evaluated_depsgraph_get`.
* Objects: transforms (all rotation modes), `matrix_world/local/basis`,
  parent, `dimensions`, hide/select, modifiers (Blender types and attribute
  names), material slots, constraints, vertex groups, shape keys, pose,
  `animation_data`, `keyframe_insert/delete`, `evaluated_get`, `ray_cast`,
  `copy()`, `to_mesh()`.
* Meshes: `from_pydata`, vertices/edges/polygons/loops/loop_triangles with
  `foreach_get/set`, normals, uv layers, attributes (lite), materials,
  `transform`, `update/validate`, shape keys, select/hide flags, seams, sharp.
* Animation: actions, fcurves (`evaluate`, keyframe points with handles and
  interpolation), groups; `scene.frame_set` runs handlers.
* Rigging: armatures, `edit_bones` (head/tail/roll/matrix, Blender's
  vec_roll conventions), bones, pose bones (`matrix`, rotation modes,
  constraints), lattices.
* Shading: node trees for materials, world, groups and the compositor;
  nodes/links/sockets (`default_value`, `is_linked`), interface sockets, color
  ramps; Principled inputs also set the material's own colour and numbers;
  images (`pixels`, `save`), `render.render()` with `write_still`.
* `bpy.ops`: Polyamour's operators under Blender idnames with `poll()` and
  `{'FINISHED'}`/`{'CANCELLED'}`; unknown idnames raise like Blender.
* `bpy.types`: Operator (+ Macro), Panel, Menu (with `append/prepend` on
  built-in menus), Header, UIList, PropertyGroup, AddonPreferences;
  `bpy.props` (Bool/Int/Float/String/Enum/Pointer/Collection and vectors with
  update/get/set callbacks); `bpy.utils.register_class`,
  `register_classes_factory`, keymaps (recorded).
* `bpy.app`: `version` (4, 2, 0), `version_string`, handlers (all of Blender's
  lists, `persistent`), timers, translations stub, `binary_path`, `debug`.
* `mathutils`: Vector, Matrix, Quaternion, Euler, Color with Blender's
  operators and conversions; `geometry`, `kdtree`, `bvhtree`,
  `noise` (Polyamour's own noise through the `noise` command, values agree with
  the renderer to float precision), `interpolate`.
* `bmesh`: BMesh/BMVert/BMEdge/BMFace/BMLoop, layers (uv, deform, float/int),
  `from_mesh/to_mesh/from_edit_mesh/update_edit_mesh`, and ops
  create_cube/grid/uvsphere/icosphere/circle/cone, translate/scale/rotate/
  transform, reverse_faces, recalc_face_normals, triangulate, remove_doubles,
  weld_verts, delete, duplicate, contextual_create, holes_fill, poke,
  extrude_face_region/discrete_faces/edge_only/vert_indiv,
  inset_individual/region, subdivide_edges (grid fill), spin, mirror,
  dissolve_verts/edges/faces, bevel (runs Polyamour's bevel on a copy),
  collapse, pointmerge, join_triangles, smooth_vert.

## Missing compared with Blender's Python API

* No pipes on SexOS: live features (console, add-ons, panels, handlers,
  timers, read-after-write) need the host build until SexOS has pipes or the
  file `Wire` is written; batch reads see the start-of-run scene.
* `gpu`, `blf`, `bgl`, `aud`, `imbuf`, `freestyle`, `bl_math`, `idprop`
  modules; drawing callbacks (`SpaceView3D.draw_handler_add`) are accepted but
  never called.
* `bpy.msgbus` subscriptions are recorded but never fire; `bpy.app.handlers`
  for render, undo/redo, load/save run only when the host calls them.
* Modal operators get the events the host sends (`modal` call); the app does
  not forward viewport events to script modal operators yet. Operator redo
  panels (Adjust Last Operation) are not shown.
* Layout widgets not drawn: icons, `template_*` beyond `template_list`,
  popovers, pie menus (drawn as rows), `operator_enum`, color pickers for
  script color props (drawn as number fields).
* One scene per file (`bpy.data.scenes.new` raises); no linked libraries,
  `bpy.data.libraries`, `bpy.data.batch_remove`, `bpy.data.user_map`.
* Node trees: no copying a tree, no removing group interface sockets; Geometry
  Nodes trees through bpy are limited to what the polyops node commands carry.
* Images: no `scale`, no packing; textures (`bpy.data.textures`) absent.
* Lights cannot change type after creation; an object cannot switch data to a
  different data type.
* bmesh: `convex_hull`, `bridge_loops`, `connect_verts`, `BMFace.copy`,
  `utils.vert_separate`, `utils.loop_separate` raise `NotImplementedError`;
  `face_split` ignores `coords`; custom-data layers are kept in Python only for
  uv, deform and simple float/int.
* Drivers (`driver_add`) and the expression namespace are absent.
* Grease Pencil, curves/text objects, particles, physics, sequencer, and
  `bpy.ops` for editors Polyamour does not have.

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

`Session` is the line-at-a-time engine under `run` and under the live
channel (feed a line, take its reply lines, send them back).

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
  collected in `RunReport::replies`, and the live channel sends them back to
  the script. Any error stops the run and names
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
| `ids`, `dump_object/mesh/material/tree/scene/armature/lattice/image/action OBJ`, `eval_mesh`, `op_list`, `op_poll`, `op IDNAME [opts]`, `noise KIND POINTS`, ... | the bpy commands of `script/bpy.rs` (JSON replies), used by the Python package |

`scene_to_polyops` uses `scene`, `world`, `collection_new/_set`, `material_new`,
`mesh_new`, `add_object/add_empty/light_add/camera_add` (with `quaternion=` so
rotations are exact), `parent`, `material_assign slot=`, `modifier_add`,
`select`, `active`, `active_camera`, `end`. Floats are written in Rust's shortest
round-trip form, so `run(Scene::new(), scene_to_polyops(s)) == s` (tested on
`Scene::startup()` plus collections, nested parents, UVs, smooth flags, face
materials, loose edges, every light kind, an ortho camera, modifiers, hidden
objects). Not carried: images, material textures, the world HDRI; duplicate
material names come back made unique.


## Getting it onto SexOS

`apps/python` freezes pure-Python modules into the binary from
`apps/python/frozen/*.bin` + `MANIFEST` (build.rs turns them into CPython's
frozen tables). `scripts/python_freeze.py` compiles every module of
`polyamour` (with the kernel), `bpy`, `bpy_extras`, `mathutils` and `bmesh`
(the `POLYAMOUR` list, which must name every module) and `codeop` for the
console; the regenerated `apps/python/frozen/` is committed, so every
`python` on SexOS can `import bpy` with nothing in its folder. The
standard-library `.bin` files come out byte-identical.

**After editing anything under `apps/polyamour/python/`, refreeze** with any
CPython 3.13 (bytecode is per release series), and add new modules to
`POLYAMOUR`:

    mkdir -p "$SCRATCH/cp" && ln -sfn /usr/local/lib/python3.13 "$SCRATCH/cp/Lib"   # or a v3.13.18 checkout
    python3.13 scripts/python_freeze.py "$SCRATCH/cp"
    cargo build -p python --target <SexOS target>   # rebuilds the frozen tables

The frozen copies were checked on the host by importing all 39 modules through
a finder that loads the `.bin` files (as FrozenImporter does) and running
bmesh and mathutils code. Without refreezing, an edited package still works
when its folder is next to a script, but frozen modules are found first, so a
stale frozen copy wins.

Not yet run inside SexOS itself: the build host has no musl headers
(`MUSL_INCLUDE`), so `cargo build -p python` builds the stub without CPython;
a gate like `scripts/python_gate.py` typing `run polyamour` and Alt+P would
check the batch path end to end.
