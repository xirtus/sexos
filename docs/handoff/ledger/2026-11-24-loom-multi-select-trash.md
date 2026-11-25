# Loom contiguous selection and batch trash

- Branch: `codex/sexfiles-multi-select` (stacked on `codex/loom-shortcuts-doc`).
- Text folder lists now mark a contiguous range with Shift+Up/Down/Home/End.
  Delete moves that range to trash and one Ctrl+Z restores it. The batch move
  rolls back earlier items if a later move or origin-note write fails.
- Copy, cut and F2 explicitly explain their single-item scope when multiple
  rows are selected. Grid and noncontiguous selection are future work.
- Focused host test: `cargo test --locked -p loom-core selection::tests`.
- Guest gate: `scripts/files_loom_gate.py` takes `loom-multi-selection.ppm`,
  checks the batch-trash log and item counts, undoes the batch, and verifies
  the restored files on the FAT32 stick.
- Full build and QEMU gate run in GitHub CI only.
