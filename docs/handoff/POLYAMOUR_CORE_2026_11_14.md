# Polyamour core (`crates/sex-3d`), 2026-11-14

Workstream Z of docs/handoff/APP_PARITY_PLAN_2026_11_14.md: the CPU-side core of
Polyamour, SexOS's native Blender-class 3D suite. Plain Rust, host-tested, builds for
`x86_64-sex-std.json`. CI: `cargo test --locked --release -p sex-3d`.

| Module | Slice | What it has |
| --- | --- | --- |
| `math`, `mesh`, `primitives`, `scene`, `undo` | Z1 | n-gon meshes + half-edge view (neighbours, boundaries, edge loops, manifold), weld, ear-clip triangulation; 8 primitives; objects/parents/collections/materials/images/lights/cameras/world; snapshot undo |
| `modifiers` | Z2 | Mirror, Catmull-Clark/simple subdivision, Array, Solidify, Bevel, Triangulate, Weld, Displace, Smooth |
| `ops` | Z2 | extrude, inset, bevel edges, loop cut, bisect/knife, booleans (BSP), delete, merge, recalc normals, fill hole, bridge loops, spin/screw, limited dissolve |
| `render` | Z5 | path tracer (SAH BVH, GGX metal/dielectric/glass, NEE+MIS, HDRI importance sampling, deterministic per-pixel RNG so farm regions match bit for bit), progressive accumulator, à-trous denoiser, AgX/Standard view transforms |
| `render::viewport` | Z1 viewport | multithreaded CPU Solid-mode rasterizer with grid, wireframe, selection outlines, picking (~2 ms at 720p for the startup scene, ~12 ms for 100k tris on 4 cores) |
| `io` | Z1 | glTF 2.0/GLB, OBJ+MTL, STL, PLY import/export (round-trip goldens); native `.blend` import (legacy and 3.5+/4.x attribute layouts, 5.0 header, gzip/zstd) |
| `script` + `apps/polyamour/python` | Z7 | `polyops` command language, `bpy`-style Python package frozen into SexOS `python`; see POLYAMOUR_PYTHON_2026_11_14.md |

Host demo: `cargo run --release -p sex-3d --example render_png -- [scene file] out.png [samples]`.

Known gaps (next):
- App shell `apps/polyamour` on sex-egui (PR #220) — viewport, outliner, properties, edit mode.
- `.blend` fixtures are synthetic; add real Blender-saved files (2.79, 3.6, 4.2, 5.0).
- Bevel skips edges at boundaries/non-manifold verts; booleans use an epsilon BSP, not exact predicates.
- Renderer: rough glass not light-sampled, point/spot lights invisible to camera, BVH build single-threaded.
- glTF: no skins/animation, quads come back as triangles. Python: replies don't flow back to scripts yet.
- Z3 sculpt/paint, Z4 node materials, Z6 animation, Z8 physics, Z9 USD/FBX not started.
