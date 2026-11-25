# Spreadeagle row header resizing

- Branch: `codex/spreadeagle-row-height`, stacked on `codex/spreadeagle-formula-fixture` (#671).
- Gap: the grid let people drag column edges, but offered no row-height control.
- Change: drag a row header's bottom edge to set its height through GridCraft's undoable `home.rowHeight` command. The height is bounded to the workbook format's supported range.
- Focused host verification: `cargo test --manifest-path apps/spreadeagle/Cargo.toml row_height_drag_result_undoes_and_survives_xlsx` checks row isolation, undo, redo, and XLSX reopen.
- CI guest evidence: the Programs gate drags the row edge, checks the native height log, captures `spreadeagle-row-height.ppm`, saves and reopens the actual workbook, and checks the restored height. Full build and QEMU run on GitHub CI.
- Remaining: text wrapping, automatic fitting to text, and multi-row selection sizing.
