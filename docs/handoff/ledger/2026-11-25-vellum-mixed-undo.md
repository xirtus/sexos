# Vellum mixed edit undo — 2026-11-25

- Gap: Vellum cleared markup history when a form field changed, and cleared
  form history when markup changed. A person could not undo across the two kinds.
- Change: one bounded edit-order stack coordinates the existing field and
  annotation payload histories. Undo and redo visit committed changes in time
  order; a new edit clears redo for both kinds. Save and signing still reset
  the timeline, as before.
- Verification: `field_and_markup_share_one_undo_order_and_branch` uses a real
  PDF and checks saved field and annotation state. The PDF edit QEMU gate now
  undoes a form change and the preceding note edit, captures
  `pdf-mixed-undo.ppm`, then redoes both. Full build and QEMU run on GitHub CI.
- Scope: app/editor history only; no kernel, Wi-Fi driver, or tether changes.
