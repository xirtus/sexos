# Gestalt on VectorCraft's engine (V4)

**`apps/gestalt`, `crates/sex-pdx/src/program_store.rs`, `crates/sex-std/src/sys.rs`.**

- **Gestalt** replaces `apps/shapes` (the 59-line `sex-canvas` demo). It runs VectorCraft's
  engine unchanged (storytold/vectorcraft at 0995c13, MIT OR Apache-2.0) under a native canvas:
  - A tool strip with Selection, Direct Selection, Pen, Pencil, Rectangle, Ellipse, Line Segment,
    Type, Eyedropper and Hand, plus Shift+M for Shape Builder.
  - The tools' overlays (pen rubber band, marquees, anchors, handles, Smart Guides, measures) and
    the selection's box.
  - Undo and redo, copy, cut, paste, duplicate, select all, group and ungroup, nudge, delete,
    and Pathfinder (unite, minus front, intersect, exclude).
  - Zoom, fit, pan with the middle button or Hand, wheel scroll.
  - Opens SVG, PDF, `.ai`, EPS and `.vectorcraft`; saves SVG (or what the name says); exports
    PNG and PDF.
- **Program image limit 32 → 64 MiB** (`IMAGE_BYTES`, `sex_pkg::PROGRAM_MAX`,
  `install_program.py`): the engine is 59 MB of code with its crates at opt-level "s" and the
  program stripped. Files' load buffer, reserved at boot, grows by 32 MiB. ABI hash updated.
- **`sys_secure_rand32/64`** in sex-std, over `sys_read_entropy`, for `getrandom` 0.4 on Hermit.
- **Layout retries 24 → 64** in `sexos_build_trace.sh`: a 59 MB program holds WRPKRU bytes by
  chance at most layouts (1–8 hits per seed; seed 21 had none), so the build tries more.
- **Tests:** host tests `draws_moves_undoes_and_saves_svg`, `every_tool_on_the_strip_exists`,
  `pathfinder_unites_two_overlapping_shapes`, `welcome_opens`; gate
  `scripts/program_gestalt_gate.py` (welcome colours on screen, Rectangle drag, undo/redo,
  select all, group, save, PNG export, reopen). `program_shapes_gate.py` is gone with the demo.
