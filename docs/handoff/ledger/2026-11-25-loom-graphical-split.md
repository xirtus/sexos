# Loom graphical split folders

- Branch: `codex/sexfiles-graphical-split`, stacked on `codex/sexfiles-batch-copy` (#672).
- Gap: split folders only rendered as text; the thumbnail grid discarded the second folder.
- Change: draw two independent folder tile grids with each tab's path, cursor and scroll row. Ctrl+G changes between text and grid while split remains on. Ctrl+PageUp/PageDown switches the active tab; a pointer click focuses either pane and can select a tile. If a resized window cannot hold one full tile in each pane, the text split view appears until space returns.
- Focused host test: `cargo test -p loom-core split::tests` checks pointer geometry and independent scroll offsets. Targeted `cargo check -p files --target x86_64-unknown-none` checks the native UI; the full build stays on CI.
- QEMU gate: `scripts/files_loom_gate.py` opens two distinct folders, switches into the split grid, verifies the render log, captures `folder-split-grid.ppm`, clicks the other pane, and verifies keyboard focus can switch back. The full build and QEMU run on GitHub CI.
- Remaining: inactive folders with uncached pictures show file icons until focused; a places sidebar and grid batch selection remain open.
