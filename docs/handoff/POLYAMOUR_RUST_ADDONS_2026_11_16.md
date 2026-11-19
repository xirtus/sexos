# Polyamour pure-Rust add-ons, `polyops` extensions & native desktop builds, 2026-11-16

Strategy: **keep what works (`import bpy` Python scripts over `polyops 1`), and add pure Rust strictly where Python hurts** (compute-heavy procedural geometry, capability-sandboxed third-party `.wasm` add-ons, live mesh readback, bounded host resource limits, and native macOS/Linux desktop builds).

## 1. Why Python stays for `.py` scripts (`apps/polyamour/python`)

* Polyamour's 3D backend (`crates/sex-3d`) and UI (`apps/polyamour`) are **100% pure Rust** and link zero C or `libpython` code.
* `apps/polyamour/python` (`polyamour`, `bpy`, `bpy_extras`, `mathutils`, `bmesh`) uses **zero PyPI libraries**, communicating with `sex-3d` over the `polyops 1` text command stream.
* Keeping it preserves zero-shot Blender script & LLM procedural generation compatibility and no-compile edit-and-run workflows, while SexOS already builds `apps/python` (`cpython-3.13.18`) for Qupid's Kodi add-ons.

## 2. What landed on `feat/polyamour-rust-sdk`

| Component | Where | What it adds |
| --- | --- | --- |
| Live mesh readback, declarative panels & host caps | `crates/sex-3d/src/script.rs` | `mesh_get OBJ\|MESH [evaluated=0\|1]` (base or `Scene::evaluated_mesh`), `mesh_from_options()`, `ui_panel` / `ui_prop` (`RunReport::panels`), and `SessionLimits` (`max_commands`, `max_created_objects`, monotonic `max_reply_bytes`, and pre/post-modifier `max_mesh_vertices` / `max_mesh_faces`) |
| Python `bpy` parity | `apps/polyamour/python/` + `apps/python/frozen/` | `bpy.mesh_get(target, evaluated=False)`, `bpy.ui_panel`, and `bpy.ui_prop` (with updated Python 3.13 frozen `.bin` tables) |
| `#![no_std]` pure-Rust guest SDK | `crates/polyamour-sdk` | `no_std + alloc` SDK (`Vec2`, `Vec3`, `Euler`, `Color`, `Quat`, `MeshData` with `seams`/`sharp` and >64-vertex `b64:` packing, `Client<T>`, `BufferTransport`, and `wasm32-unknown-unknown` `SexWasmTransport` + `guest_args()`) |
| Sandboxed `.wasm` add-on host & compiled `wasm32` guest | `crates/polyamour-wasm` | Runs untrusted `.wasm` plugins in-process via `sex-wasm` (`wasmi 2.0.0`) with WASM fuel/memory caps + `SessionLimits::sandboxed()` + `max_log_bytes`, zero disk/network grants, atomic scene rollback on failure, and a real `wasm32-unknown-unknown` Rust guest (`guests/spiral_rust`) |
| Polyamour UI & native macOS/Linux window | `apps/polyamour/src/{python,desktop,main}.rs` | Discovers & toggles `*.wasm` add-ons in `Preferences > Add-ons` (even without Python), draws interactive `ui_panel`/`ui_prop` widgets + rebuild buttons in the 3D View Sidebar (`N`), supports `--wasm FILE.wasm`, and opens a live interactive `winit` + `softbuffer` desktop window on macOS (Cocoa/CoreGraphics) and Linux (Wayland/X11) |
| Canonical spec update | `docs/handoff/POLYAMOUR_PYTHON_2026_11_14.md` | Documents `mesh_get`, `ui_panel`, `ui_prop`, `SessionLimits`, `bpy.mesh_get`, and `.wasm` add-ons |
