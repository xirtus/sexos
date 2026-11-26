# Spreadeagle wrapped cells

- Added Ctrl+Shift+U for wrapping selected cells through GridCraft's `home.wrapText` command. Its automatic row height grows for wrapped content; the native grid draws visible lines inside the cell instead of overflowing into adjacent cells.
- Preserve per-cell wrap on ODS import and export with `fo:wrap-option="wrap"`; XLSX uses GridCraft's existing wrap style serialization. This does not add a general AutoFit command for manually sized rows.
- Host test covers line layout, row growth, undo/redo, XLSX and ODS save/reopen; Programs QEMU gate captures `spreadeagle-wrapped-cell.ppm` after toggling wrap and checks reported row growth. Full build and guest run belong to GitHub CI.
