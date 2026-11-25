# Vellum highlight comments — 2026-11-25

Enter on a selected highlight in Vellum's Ctrl+L annotation list opens its comment for editing. Enter commits, Escape cancels, and Backspace removes a character. Edits update the PDF annotation's `/Contents` while retaining its geometry and appearance. Ctrl+Z and Ctrl+Shift+Z undo and redo the comment before save. Existing signed-PDF markup guards still apply.

Focused host tests check save/reopen, appearance preservation, and undo/redo. The QEMU edit gate types `Review green`, captures `pdf-highlight-comment.ppm`, exercises undo/redo, and checks the guest-saved PDF comment. No protected paths or compatibility behavior change.
