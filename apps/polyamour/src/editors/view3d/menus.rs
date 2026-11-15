// SPDX-License-Identifier: GPL-3.0-only
//! The 3D Viewport's menus with Blender 4.x's entries: the header menus of
//! Object and Edit Mode, the context menus, and the hotkey menus (Shift+S,
//! Ctrl+A, M, X, Ctrl+P, the ` view pie, Z shading pie...). Entries we cannot
//! do yet keep Blender's operator names and report "not available".
use crate::doc::{Doc, Mode, SelectMode};
use crate::transform::{Falloff, Orientation, Pivot, SnapTarget};
use crate::ui::{MenuItem, Op};

fn op(l: &str, k: &str, id: &str) -> MenuItem {
    MenuItem::op(l, k, Op::new(id))
}
fn opa(l: &str, k: &str, id: &str, a: &str) -> MenuItem {
    MenuItem::op(l, k, Op::with(id, a))
}
fn sub(l: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem::Sub(l.into(), items)
}
const SEP: MenuItem = MenuItem::Separator;

/// Flags the View menu shows as checkboxes.
pub struct ViewFlags {
    pub toolbar: bool,
    pub sidebar: bool,
}

pub fn view_menu(_d: &Doc, f: &ViewFlags) -> Vec<MenuItem> {
    vec![
        MenuItem::Check("Toolbar".into(), f.toolbar, Op::new("view3d.toolbar")),
        MenuItem::Check("Sidebar".into(), f.sidebar, Op::new("view3d.sidebar")),
        MenuItem::Check("Tool Settings".into(), false, Op::new("screen.region_toggle")),
        SEP,
        op("Adjust Last Operation", "F9", "screen.redo_last"),
        SEP,
        op("Frame Selected", "Numpad .", "view3d.view_selected"),
        op("Frame All", "Home", "view3d.view_all"),
        SEP,
        op("Perspective/Orthographic", "Numpad 5", "view3d.view_persportho"),
        sub("Local View", vec![op("Toggle Local View", "Numpad /", "view3d.localview")]),
        SEP,
        sub(
            "Cameras",
            vec![
                op("Set Active Object as Camera", "Ctrl Numpad 0", "view3d.object_as_camera"),
                op("Active Camera", "Numpad 0", "view3d.view_camera"),
                op("Align Active Camera to View", "Ctrl Alt Numpad 0", "view3d.camera_to_view"),
            ],
        ),
        sub("Viewpoint", viewpoint_items()),
        sub(
            "Navigation",
            vec![
                opa("Orbit Left", "Numpad 4", "view3d.view_orbit", "LEFT"),
                opa("Orbit Right", "Numpad 6", "view3d.view_orbit", "RIGHT"),
                opa("Orbit Up", "Numpad 8", "view3d.view_orbit", "UP"),
                opa("Orbit Down", "Numpad 2", "view3d.view_orbit", "DOWN"),
                SEP,
                opa("Orbit Opposite", "Numpad 9", "view3d.view_orbit", "OPPOSITE"),
                SEP,
                opa("Zoom In", "Numpad +", "view3d.zoom", "IN"),
                opa("Zoom Out", "Numpad -", "view3d.zoom", "OUT"),
                SEP,
                op("Fly Navigation", "", "view3d.fly"),
                op("Walk Navigation", "", "view3d.walk"),
            ],
        ),
        sub(
            "Align View",
            vec![
                op("Center Cursor and Frame All", "Shift C", "view3d.view_center_cursor"),
                op("Center View to Cursor", "", "view3d.view_center_cursor"),
                op("Frame Selected", "Numpad .", "view3d.view_selected"),
            ],
        ),
        SEP,
        sub("Area", vec![op("Toggle Maximize Area", "Ctrl Space", "screen.screen_full_area")]),
    ]
}

pub fn viewpoint_items() -> Vec<MenuItem> {
    vec![
        op("Camera", "Numpad 0", "view3d.view_camera"),
        SEP,
        opa("Top", "Numpad 7", "view3d.view_axis", "TOP"),
        opa("Bottom", "Ctrl Numpad 7", "view3d.view_axis", "BOTTOM"),
        SEP,
        opa("Front", "Numpad 1", "view3d.view_axis", "FRONT"),
        opa("Back", "Ctrl Numpad 1", "view3d.view_axis", "BACK"),
        SEP,
        opa("Right", "Numpad 3", "view3d.view_axis", "RIGHT"),
        opa("Left", "Ctrl Numpad 3", "view3d.view_axis", "LEFT"),
    ]
}

pub fn select_menu(d: &Doc) -> Vec<MenuItem> {
    if d.mode == Mode::Edit {
        return vec![
            opa("All", "A", "mesh.select_all", "SELECT"),
            opa("None", "Alt A", "mesh.select_all", "DESELECT"),
            opa("Invert", "Ctrl I", "mesh.select_all", "INVERT"),
            SEP,
            op("Box Select", "B", "view3d.select_box"),
            op("Circle Select", "C", "view3d.select_circle"),
            op("Lasso Select", "", "view3d.select_lasso"),
            SEP,
            op("Select Random", "", "mesh.select_random"),
            op("Checker Deselect", "", "mesh.select_nth"),
            SEP,
            sub("Select More/Less", vec![op("More", "Ctrl Numpad +", "mesh.select_more"), op("Less", "Ctrl Numpad -", "mesh.select_less")]),
            sub(
                "Select All by Trait",
                vec![op("Non Manifold", "", "mesh.select_non_manifold"), op("Loose Geometry", "", "mesh.select_loose"), op("Interior Faces", "", "mesh.select_interior_faces")],
            ),
            SEP,
            sub("Select Loops", vec![op("Edge Loops", "", "mesh.loop_multi_select"), op("Edge Rings", "", "mesh.loop_multi_select_ring")]),
            sub("Select Linked", vec![op("Linked", "Ctrl L", "mesh.select_linked"), op("Shortest Path", "", "mesh.shortest_path_select")]),
            SEP,
            op("Select Mirror", "Shift Ctrl M", "mesh.select_mirror"),
        ];
    }
    vec![
        opa("All", "A", "object.select_all", "SELECT"),
        opa("None", "Alt A", "object.select_all", "DESELECT"),
        opa("Invert", "Ctrl I", "object.select_all", "INVERT"),
        SEP,
        op("Box Select", "B", "view3d.select_box"),
        op("Circle Select", "C", "view3d.select_circle"),
        op("Lasso Select", "", "view3d.select_lasso"),
        SEP,
        sub(
            "Select All by Type",
            vec![
                opa("Mesh", "", "object.select_by_type", "MESH"),
                opa("Camera", "", "object.select_by_type", "CAMERA"),
                opa("Light", "", "object.select_by_type", "LIGHT"),
                opa("Empty", "", "object.select_by_type", "EMPTY"),
            ],
        ),
        op("Select Active Camera", "", "object.select_camera"),
        op("Select Mirror", "Shift Ctrl M", "object.select_mirror"),
        op("Select Random", "", "object.select_random"),
        SEP,
        sub("Select More/Less", vec![op("More", "Ctrl Numpad +", "object.select_more"), op("Less", "Ctrl Numpad -", "object.select_less")]),
        op("Select Grouped", "Shift G", "object.select_grouped"),
        op("Select Linked", "Shift L", "object.select_linked"),
        op("Select Pattern...", "", "object.select_pattern"),
    ]
}

pub fn add_menu(d: &Doc) -> Vec<MenuItem> {
    let p = |l: &str, a: &str| opa(l, "", "mesh.primitive_add", a);
    let mesh = vec![
        p("Plane", "plane"),
        p("Cube", "cube"),
        p("Circle", "circle"),
        p("UV Sphere", "uv_sphere"),
        p("Ico Sphere", "ico_sphere"),
        p("Cylinder", "cylinder"),
        p("Cone", "cone"),
        p("Torus", "torus"),
        SEP,
        p("Grid", "grid"),
        op("Monkey", "", "mesh.primitive_monkey_add"),
    ];
    if d.mode == Mode::Edit {
        return mesh;
    }
    vec![
        sub("Mesh", mesh),
        sub("Curve", vec![op("Bezier", "", "curve.primitive_bezier_curve_add"), op("Circle", "", "curve.primitive_bezier_circle_add"), op("Path", "", "curve.primitive_nurbs_path_add")]),
        sub("Surface", vec![op("NURBS Surface", "", "surface.primitive_nurbs_surface_surface_add")]),
        sub("Metaball", vec![op("Ball", "", "object.metaball_add")]),
        op("Text", "", "object.text_add"),
        sub("Volume", vec![op("Empty", "", "object.volume_add")]),
        sub("Grease Pencil", vec![op("Blank", "", "object.grease_pencil_add")]),
        SEP,
        sub("Armature", vec![op("Single Bone", "", "object.armature_add")]),
        op("Lattice", "", "object.add"),
        sub(
            "Empty",
            vec![
                op("Plain Axes", "", "object.empty_add"),
                op("Arrows", "", "object.empty_add"),
                op("Single Arrow", "", "object.empty_add"),
                op("Circle", "", "object.empty_add"),
                op("Cube", "", "object.empty_add"),
                op("Sphere", "", "object.empty_add"),
                op("Cone", "", "object.empty_add"),
            ],
        ),
        op("Image", "", "object.empty_image_add"),
        SEP,
        sub(
            "Light",
            vec![
                opa("Point", "", "object.light_add", "POINT"),
                opa("Sun", "", "object.light_add", "SUN"),
                opa("Spot", "", "object.light_add", "SPOT"),
                opa("Area", "", "object.light_add", "AREA"),
            ],
        ),
        sub("Light Probe", vec![op("Sphere", "", "object.lightprobe_add"), op("Plane", "", "object.lightprobe_add"), op("Volume", "", "object.lightprobe_add")]),
        SEP,
        op("Camera", "", "object.camera_add"),
        op("Speaker", "", "object.speaker_add"),
        SEP,
        sub("Force Field", vec![op("Force", "", "object.effector_add"), op("Wind", "", "object.effector_add")]),
        op("Collection Instance", "", "object.collection_instance_add"),
    ]
}

pub fn snap_items() -> Vec<MenuItem> {
    vec![
        op("Selection to Grid", "", "view3d.snap_selected_to_grid"),
        op("Selection to Cursor", "", "view3d.snap_selected_to_cursor"),
        opa("Selection to Cursor (Keep Offset)", "", "view3d.snap_selected_to_cursor", "OFFSET"),
        op("Selection to Active", "", "view3d.snap_selected_to_active"),
        SEP,
        op("Cursor to Selected", "", "view3d.snap_cursor_to_selected"),
        op("Cursor to World Origin", "", "view3d.snap_cursor_to_center"),
        op("Cursor to Grid", "", "view3d.snap_cursor_to_grid"),
        op("Cursor to Active", "", "view3d.snap_cursor_to_active"),
    ]
}

pub fn apply_items() -> Vec<MenuItem> {
    vec![
        opa("Location", "", "object.transform_apply", "LOCATION"),
        opa("Rotation", "", "object.transform_apply", "ROTATION"),
        opa("Scale", "", "object.transform_apply", "SCALE"),
        opa("All Transforms", "", "object.transform_apply", "ALL"),
        opa("Rotation & Scale", "", "object.transform_apply", "ROTSCALE"),
        SEP,
        op("Location to Deltas", "", "object.transforms_to_deltas"),
        SEP,
        op("Visual Transform", "", "object.visual_transform_apply"),
        opa("Visual Geometry to Mesh", "", "object.convert", "MESH"),
        op("Make Instances Real", "", "object.duplicates_make_real"),
        op("Parent Inverse", "", "object.parent_inverse_apply"),
    ]
}

pub fn parent_items() -> Vec<MenuItem> {
    vec![
        op("Object", "Ctrl P", "object.parent_set"),
        op("Object (Keep Transform)", "", "object.parent_set"),
        op("Object (Without Inverse)", "", "object.parent_no_inverse_set"),
        SEP,
        opa("Clear Parent", "Alt P", "object.parent_clear", "CLEAR"),
        opa("Clear and Keep Transformation", "", "object.parent_clear", "CLEAR_KEEP_TRANSFORM"),
        opa("Clear Parent Inverse", "", "object.parent_clear", "CLEAR_INVERSE"),
    ]
}

pub fn clear_parent_items() -> Vec<MenuItem> {
    vec![opa("Clear Parent", "", "object.parent_clear", "CLEAR"), opa("Clear and Keep Transformation", "", "object.parent_clear", "CLEAR_KEEP_TRANSFORM")]
}

pub fn collection_items(d: &Doc) -> Vec<MenuItem> {
    let mut v: Vec<MenuItem> = d.scene.collections.iter().enumerate().map(|(i, c)| opa(&c.name, "", "object.move_to_collection", &i.to_string())).collect();
    v.push(SEP);
    v.push(opa("+ New Collection", "", "object.move_to_collection", "NEW"));
    v
}

fn set_origin_items() -> Vec<MenuItem> {
    vec![
        opa("Geometry to Origin", "", "object.origin_set", "GEOMETRY_ORIGIN"),
        opa("Origin to Geometry", "", "object.origin_set", "ORIGIN_GEOMETRY"),
        opa("Origin to 3D Cursor", "", "object.origin_set", "ORIGIN_CURSOR"),
        op("Origin to Center of Mass (Surface)", "", "object.origin_set_mass"),
    ]
}

fn mirror_items() -> Vec<MenuItem> {
    vec![op("Interactive Mirror", "Ctrl M", "transform.mirror"), SEP, op("X Global", "", "transform.mirror"), op("Y Global", "", "transform.mirror"), op("Z Global", "", "transform.mirror")]
}

fn showhide_items() -> Vec<MenuItem> {
    vec![
        op("Show Hidden Objects", "Alt H", "object.hide_view_clear"),
        op("Hide Selected", "H", "object.hide_view_set"),
        opa("Hide Unselected", "Shift H", "object.hide_view_set", "UNSELECTED"),
    ]
}

pub fn object_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        sub(
            "Transform",
            vec![
                op("Move", "G", "transform.translate"),
                op("Rotate", "R", "transform.rotate"),
                op("Scale", "S", "transform.resize"),
                SEP,
                op("Align to Transform Orientation", "", "transform.transform"),
                op("Randomize Transform", "", "object.randomize_transform"),
                op("Align Objects", "", "object.align"),
            ],
        ),
        sub("Set Origin", set_origin_items()),
        sub("Mirror", mirror_items()),
        sub(
            "Clear",
            vec![
                op("Location", "Alt G", "object.location_clear"),
                op("Rotation", "Alt R", "object.rotation_clear"),
                op("Scale", "Alt S", "object.scale_clear"),
                op("Origin", "Alt O", "object.origin_clear"),
            ],
        ),
        sub("Apply", apply_items()),
        sub("Snap", snap_items()),
        SEP,
        op("Duplicate Objects", "Shift D", "object.duplicate_move"),
        op("Duplicate Linked", "Alt D", "object.duplicate_move_linked"),
        op("Join", "Ctrl J", "object.join"),
        SEP,
        op("Copy Objects", "Ctrl C", "view3d.copybuffer"),
        op("Paste Objects", "Ctrl V", "view3d.pastebuffer"),
        SEP,
        sub("Parent", parent_items()),
        sub("Collection", vec![op("Move to Collection", "M", "object.move_to_collection_menu"), op("Link to Collection", "Shift M", "object.link_to_collection")]),
        sub("Relations", vec![op("Make Single User", "", "object.make_single_user")]),
        sub("Constraint", vec![op("Add Constraint (with Targets)", "", "object.constraint_add_with_targets")]),
        sub("Track", vec![op("Damped Track Constraint", "", "object.track_set")]),
        sub("Link/Transfer Data", vec![op("Link Objects to Scene...", "Ctrl L", "object.make_links_scene")]),
        SEP,
        op("Shade Smooth", "", "object.shade_smooth"),
        op("Shade Auto Smooth", "", "object.shade_auto_smooth"),
        op("Shade Flat", "", "object.shade_flat"),
        SEP,
        sub("Animation", vec![op("Insert Keyframe", "I", "anim.keyframe_insert"), op("Clear Keyframes...", "Alt I", "anim.keyframe_clear_v3d")]),
        SEP,
        sub("Convert", vec![opa("Mesh", "", "object.convert", "MESH"), op("Curve", "", "object.convert_curve")]),
        SEP,
        sub("Show/Hide", showhide_items()),
        SEP,
        op("Delete", "X", "object.delete"),
        op("Delete Globally", "", "object.delete"),
    ]
}

pub fn object_context_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Shade Smooth", "", "object.shade_smooth"),
        op("Shade Auto Smooth", "", "object.shade_auto_smooth"),
        op("Shade Flat", "", "object.shade_flat"),
        SEP,
        sub("Convert To", vec![opa("Mesh", "", "object.convert", "MESH"), op("Curve", "", "object.convert_curve")]),
        sub("Set Origin", set_origin_items()),
        SEP,
        op("Copy Objects", "Ctrl C", "view3d.copybuffer"),
        op("Paste Objects", "Ctrl V", "view3d.pastebuffer"),
        SEP,
        op("Duplicate Objects", "Shift D", "object.duplicate_move"),
        op("Duplicate Linked", "Alt D", "object.duplicate_move_linked"),
        op("Rename Active Item...", "F2", "wm.call_panel"),
        SEP,
        sub("Mirror", mirror_items()),
        sub("Snap", snap_items()),
        sub("Parent", parent_items()),
        op("Move to Collection", "M", "object.move_to_collection_menu"),
        SEP,
        op("Insert Keyframe", "I", "anim.keyframe_insert"),
        SEP,
        op("Delete", "X", "object.delete"),
    ]
}

fn mesh_showhide() -> Vec<MenuItem> {
    vec![op("Reveal Hidden", "Alt H", "mesh.reveal"), op("Hide Selected", "H", "mesh.hide"), opa("Hide Unselected", "Shift H", "mesh.hide", "UNSELECTED")]
}

pub fn merge_items() -> Vec<MenuItem> {
    vec![
        opa("At Center", "", "mesh.merge", "CENTER"),
        opa("At Cursor", "", "mesh.merge", "CURSOR"),
        opa("Collapse", "", "mesh.merge", "COLLAPSE"),
        opa("At First", "", "mesh.merge_first", "FIRST"),
        opa("At Last", "", "mesh.merge_last", "LAST"),
        SEP,
        op("By Distance", "", "mesh.remove_doubles"),
    ]
}

pub fn delete_items() -> Vec<MenuItem> {
    vec![
        opa("Vertices", "", "mesh.delete", "VERT"),
        opa("Edges", "", "mesh.delete", "EDGE"),
        opa("Faces", "", "mesh.delete", "FACE"),
        opa("Only Edges & Faces", "", "mesh.delete", "EDGE_FACE"),
        opa("Only Faces", "", "mesh.delete", "ONLY_FACE"),
        SEP,
        op("Dissolve Vertices", "", "mesh.dissolve_verts"),
        op("Dissolve Edges", "", "mesh.dissolve_edges"),
        op("Dissolve Faces", "", "mesh.dissolve_faces"),
        op("Limited Dissolve", "", "mesh.dissolve_limited"),
        SEP,
        op("Edge Collapse", "", "mesh.edge_collapse"),
        op("Edge Loops", "", "mesh.delete_edgeloop"),
    ]
}

pub fn separate_items() -> Vec<MenuItem> {
    vec![opa("Selection", "", "mesh.separate", "SELECTED"), opa("By Material", "", "mesh.separate", "MATERIAL"), opa("By Loose Parts", "", "mesh.separate", "LOOSE")]
}

pub fn normals_items() -> Vec<MenuItem> {
    vec![
        op("Flip", "", "mesh.flip_normals"),
        op("Recalculate Outside", "Shift N", "mesh.normals_make_consistent"),
        opa("Recalculate Inside", "Shift Ctrl N", "mesh.normals_make_consistent", "INSIDE"),
    ]
}

pub fn mesh_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        sub(
            "Transform",
            vec![
                op("Move", "G", "transform.translate"),
                op("Rotate", "R", "transform.rotate"),
                op("Scale", "S", "transform.resize"),
                SEP,
                op("To Sphere", "Shift Alt S", "transform.tosphere"),
                op("Shear", "Shift Ctrl Alt S", "transform.shear"),
                op("Push/Pull", "", "transform.push_pull"),
            ],
        ),
        sub("Mirror", vec![op("Interactive Mirror", "Ctrl M", "transform.mirror")]),
        sub("Snap", snap_items()),
        SEP,
        op("Duplicate", "Shift D", "mesh.duplicate_move"),
        sub("Extrude", extrude_items()),
        SEP,
        sub("Merge", merge_items()),
        sub("Split", vec![op("Selection", "Y", "mesh.split"), op("Faces by Edges", "", "mesh.edge_split")]),
        sub("Separate", separate_items()),
        SEP,
        op("Bisect", "", "mesh.knife_tool"),
        op("Knife Project", "", "mesh.knife_project"),
        op("Knife Topology Tool", "K", "mesh.knife_tool"),
        op("Convex Hull", "", "mesh.convex_hull"),
        SEP,
        op("Symmetrize", "", "mesh.symmetrize"),
        op("Snap to Symmetry", "", "mesh.symmetry_snap"),
        SEP,
        sub("Normals", normals_items()),
        sub("Shading", vec![op("Smooth Faces", "", "mesh.faces_shade_smooth"), op("Flat Faces", "", "mesh.faces_shade_flat")]),
        sub(
            "Clean Up",
            vec![
                op("Delete Loose", "", "mesh.delete_loose"),
                op("Decimate Geometry", "", "mesh.decimate"),
                op("Degenerate Dissolve", "", "mesh.dissolve_degenerate"),
                op("Limited Dissolve", "", "mesh.dissolve_limited"),
                op("Make Planar Faces", "", "mesh.face_make_planar"),
                op("Split Non-Planar Faces", "", "mesh.vert_connect_nonplanar"),
                op("Split Concave Faces", "", "mesh.vert_connect_concave"),
                op("Fill Holes", "", "mesh.fill_holes"),
                SEP,
                op("Merge by Distance", "", "mesh.remove_doubles"),
            ],
        ),
        sub("Delete", delete_items()),
        SEP,
        sub("Show/Hide", mesh_showhide()),
    ]
}

pub fn extrude_items() -> Vec<MenuItem> {
    vec![
        op("Extrude Faces", "E", "mesh.extrude_region_move"),
        op("Extrude Faces Along Normals", "", "mesh.extrude_region_shrink_fatten"),
        op("Extrude Individual Faces", "", "mesh.extrude_faces_indiv_move"),
        op("Extrude Manifold", "", "mesh.extrude_manifold"),
        op("Extrude Edges", "", "mesh.extrude_region_move"),
        op("Extrude Vertices", "", "mesh.extrude_region_move"),
        op("Extrude Repeat", "", "mesh.extrude_repeat"),
        op("Spin", "", "mesh.spin"),
    ]
}

pub fn vertex_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Extrude Vertices", "", "mesh.extrude_region_move"),
        op("Bevel Vertices", "Shift Ctrl B", "mesh.bevel_vertices"),
        SEP,
        op("New Edge/Face from Vertices", "F", "mesh.edge_face_add"),
        op("Connect Vertex Path", "J", "mesh.vert_connect_path"),
        op("Connect Vertex Pairs", "", "mesh.vert_connect"),
        SEP,
        op("Rip Vertices", "V", "mesh.rip_move"),
        op("Rip Vertices and Fill", "", "mesh.rip_move_fill"),
        op("Rip Vertices and Extend", "Alt D", "mesh.rip_edge_move"),
        SEP,
        op("Slide Vertices", "", "transform.vert_slide"),
        op("Smooth Vertices", "", "mesh.vertices_smooth"),
        op("Smooth Vertices (Laplacian)", "", "mesh.vertices_smooth_laplacian"),
        SEP,
        op("Blend from Shape", "", "mesh.blend_from_shape"),
        op("Propagate to Shapes", "", "mesh.shape_propagate_to_all"),
        SEP,
        op("Vertex Groups", "", "object.vertex_group_assign"),
        op("Hooks", "", "object.hook_add_newob"),
        SEP,
        op("Set Parent to Vertices", "Ctrl P", "object.vertex_parent_set"),
    ]
}

pub fn edge_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Extrude Edges", "", "mesh.extrude_region_move"),
        op("Bevel Edges", "Ctrl B", "mesh.bevel"),
        op("Bridge Edge Loops", "", "mesh.bridge_edge_loops"),
        op("Screw", "", "mesh.screw"),
        SEP,
        op("Subdivide", "", "mesh.subdivide"),
        op("Subdivide Edge-Ring", "", "mesh.subdivide_edgering"),
        op("Un-Subdivide", "", "mesh.unsubdivide"),
        SEP,
        opa("Rotate Edge CW", "", "mesh.edge_rotate", "CW"),
        opa("Rotate Edge CCW", "", "mesh.edge_rotate", "CCW"),
        SEP,
        op("Edge Slide", "", "transform.edge_slide"),
        op("Offset Edge Slide", "", "mesh.offset_edge_loops_slide"),
        op("Loop Cut and Slide", "Ctrl R", "mesh.loopcut_slide"),
        SEP,
        op("Mark Seam", "", "mesh.mark_seam"),
        op("Clear Seam", "", "mesh.mark_seam"),
        SEP,
        op("Mark Sharp", "", "mesh.mark_sharp"),
        op("Clear Sharp", "", "mesh.mark_sharp"),
    ]
}

pub fn face_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Extrude Faces", "E", "mesh.extrude_region_move"),
        op("Extrude Faces Along Normals", "", "mesh.extrude_region_shrink_fatten"),
        op("Extrude Individual Faces", "", "mesh.extrude_faces_indiv_move"),
        SEP,
        op("Inset Faces", "I", "mesh.inset"),
        op("Poke Faces", "", "mesh.poke"),
        op("Triangulate Faces", "Ctrl T", "mesh.quads_convert_to_tris"),
        op("Tris to Quads", "Alt J", "mesh.tris_convert_to_quads"),
        op("Solidify Faces", "", "mesh.solidify"),
        op("Wireframe", "", "mesh.wireframe"),
        SEP,
        op("Fill", "Alt F", "mesh.fill"),
        op("Grid Fill", "", "mesh.fill_grid"),
        op("Beautify Faces", "Shift Alt F", "mesh.beautify_fill"),
        SEP,
        op("Intersect (Knife)", "", "mesh.intersect"),
        op("Intersect (Boolean)", "", "mesh.intersect_boolean"),
        op("Weld Edges into Faces", "", "mesh.face_split_by_edges"),
        SEP,
        op("Shade Smooth", "", "mesh.faces_shade_smooth"),
        op("Shade Flat", "", "mesh.faces_shade_flat"),
        SEP,
        sub("Face Data", vec![op("Rotate Colors", "", "mesh.colors_rotate"), op("Rotate UVs", "", "mesh.uvs_rotate")]),
    ]
}

pub fn uv_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        op("Unwrap", "U", "uv.unwrap"),
        op("Smart UV Project", "", "uv.smart_project"),
        op("Lightmap Pack", "", "uv.lightmap_pack"),
        op("Follow Active Quads", "", "uv.follow_active_quads"),
        SEP,
        op("Cube Projection", "", "uv.cube_project"),
        op("Cylinder Projection", "", "uv.cylinder_project"),
        op("Sphere Projection", "", "uv.sphere_project"),
        SEP,
        op("Project from View", "", "uv.project_from_view"),
        op("Project from View (Bounds)", "", "uv.project_from_view"),
        SEP,
        op("Mark Seam", "", "mesh.mark_seam"),
        op("Clear Seam", "", "mesh.mark_seam"),
        SEP,
        op("Reset", "", "uv.reset"),
    ]
}

/// Right-click in Edit Mode: the vertex, edge or face context menu.
pub fn edit_context_menu(d: &Doc) -> (String, Vec<MenuItem>) {
    match d.select_mode {
        SelectMode::Vertex => (
            "Vertex Context Menu".into(),
            vec![
                op("Subdivide", "", "mesh.subdivide"),
                op("Extrude Vertices", "", "mesh.extrude_region_move"),
                op("Bevel Vertices", "Shift Ctrl B", "mesh.bevel_vertices"),
                SEP,
                op("New Edge/Face from Vertices", "F", "mesh.edge_face_add"),
                op("Connect Vertex Path", "J", "mesh.vert_connect_path"),
                op("Connect Vertex Pairs", "", "mesh.vert_connect"),
                SEP,
                op("Rip Vertices", "V", "mesh.rip_move"),
                op("Slide Vertices", "", "transform.vert_slide"),
                op("Smooth Vertices", "", "mesh.vertices_smooth"),
                SEP,
                sub("Mirror", vec![op("Interactive Mirror", "Ctrl M", "transform.mirror")]),
                sub("Snap", snap_items()),
                sub("Merge Vertices", merge_items()),
                op("Split", "Y", "mesh.split"),
                sub("Separate", separate_items()),
                SEP,
                op("Delete Vertices", "", "mesh.delete"),
            ],
        ),
        SelectMode::Edge => (
            "Edge Context Menu".into(),
            vec![
                op("Subdivide", "", "mesh.subdivide"),
                op("Extrude Edges", "", "mesh.extrude_region_move"),
                op("Bevel Edges", "Ctrl B", "mesh.bevel"),
                op("Bridge Edge Loops", "", "mesh.bridge_edge_loops"),
                SEP,
                op("New Face from Edges", "F", "mesh.edge_face_add"),
                op("Loop Cut and Slide", "Ctrl R", "mesh.loopcut_slide"),
                op("Edge Slide", "", "transform.edge_slide"),
                SEP,
                op("Mark Seam", "", "mesh.mark_seam"),
                op("Mark Sharp", "", "mesh.mark_sharp"),
                SEP,
                sub("Snap", snap_items()),
                sub("Merge Vertices", merge_items()),
                sub("Separate", separate_items()),
                SEP,
                opa("Delete Edges", "", "mesh.delete", "EDGE"),
            ],
        ),
        SelectMode::Face => (
            "Face Context Menu".into(),
            vec![
                op("Subdivide", "", "mesh.subdivide"),
                op("Extrude Faces", "E", "mesh.extrude_region_move"),
                op("Extrude Individual Faces", "", "mesh.extrude_faces_indiv_move"),
                op("Inset Faces", "I", "mesh.inset"),
                op("Bridge Faces", "", "mesh.bridge_edge_loops"),
                op("Poke Faces", "", "mesh.poke"),
                op("Triangulate Faces", "Ctrl T", "mesh.quads_convert_to_tris"),
                SEP,
                op("Shade Smooth", "", "mesh.faces_shade_smooth"),
                op("Shade Flat", "", "mesh.faces_shade_flat"),
                SEP,
                sub("Snap", snap_items()),
                sub("Merge Vertices", merge_items()),
                sub("Separate", separate_items()),
                SEP,
                opa("Delete Faces", "", "mesh.delete", "FACE"),
            ],
        ),
    }
}

/// The ` (accent grave) view pie, as a menu.
pub fn view_pie() -> Vec<MenuItem> {
    vec![
        opa("Left", "", "view3d.view_axis", "LEFT"),
        opa("Right", "", "view3d.view_axis", "RIGHT"),
        opa("Bottom", "", "view3d.view_axis", "BOTTOM"),
        opa("Top", "", "view3d.view_axis", "TOP"),
        opa("Back", "", "view3d.view_axis", "BACK"),
        opa("Front", "", "view3d.view_axis", "FRONT"),
        SEP,
        op("View Camera", "", "view3d.view_camera"),
        op("View Selected", "", "view3d.view_selected"),
    ]
}

/// The Z shading pie, as a menu.
pub fn shading_pie() -> Vec<MenuItem> {
    vec![
        opa("Wireframe", "", "view3d.shading", "WIREFRAME"),
        opa("Solid", "", "view3d.shading", "SOLID"),
        opa("Material Preview", "", "view3d.shading", "MATERIAL"),
        opa("Rendered", "", "view3d.shading", "RENDERED"),
        SEP,
        op("Toggle X-Ray", "Alt Z", "view3d.toggle_xray"),
    ]
}

pub fn mode_items() -> Vec<MenuItem> {
    Mode::ALL
        .iter()
        .map(|m| {
            let arg = match m {
                Mode::Object => "OBJECT",
                Mode::Edit => "EDIT",
                Mode::Sculpt => "SCULPT",
                Mode::VertexPaint => "VERTEX_PAINT",
                Mode::WeightPaint => "WEIGHT_PAINT",
                Mode::TexturePaint => "TEXTURE_PAINT",
            };
            opa(m.label(), "", "object.mode_set", arg)
        })
        .collect()
}

pub fn orientation_items(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Transform Orientation".into())];
    v.extend(Orientation::ALL.iter().map(|o| MenuItem::Check(o.label().into(), d.orientation == *o, Op::with("wm.context_set", &format!("orientation={}", o.id())))));
    v
}

pub fn pivot_items(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Pivot Point".into())];
    v.extend(Pivot::ALL.iter().map(|p| MenuItem::Check(p.label().into(), d.pivot == *p, Op::with("wm.context_set", &format!("pivot={}", p.id())))));
    v
}

pub fn snap_target_items(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Snap To".into())];
    v.extend(SnapTarget::ALL.iter().map(|s| MenuItem::Check(s.label().into(), d.snap_target == *s, Op::with("wm.context_set", &format!("snap_target={}", s.id())))));
    v
}

pub fn falloff_items(d: &Doc) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Heading("Proportional Editing Falloff".into())];
    v.extend(Falloff::ALL.iter().map(|f| MenuItem::Check(f.label().into(), d.falloff == *f, Op::with("wm.context_set", &format!("falloff={}", f.id())))));
    v
}

/// The X confirmation in Object Mode.
pub fn delete_confirm() -> Vec<MenuItem> {
    vec![op("Delete", "", "object.delete")]
}
