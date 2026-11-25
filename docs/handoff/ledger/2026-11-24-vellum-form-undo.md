# Vellum form-field undo and redo

Vellum records up to 32 committed form-field changes. Ctrl+Z restores the
previous value and Ctrl+Shift+Z reapplies it. A new edit clears redo. Saving
or changing annotations clears this field-only history, so the commands never
claim to reverse an annotation they did not record. The history stays in the
window and adds no Files grant.

The host test applies undo/redo to a real PDF form and verifies the incremental
save. The QEMU edit gate checks the shortcuts and captures the undone form.
Full builds and QEMU run only on GitHub CI.
