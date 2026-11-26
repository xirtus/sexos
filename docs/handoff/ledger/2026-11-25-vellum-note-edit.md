# Vellum sticky note editing — 2026-11-25

Enter on a selected sticky note in Ctrl+L opens its text for editing. Enter commits, Escape cancels, and Ctrl+Z/Ctrl+Shift+Z undo or redo before save. The annotation engine updates `/Contents` for highlights and sticky notes without changing their appearance or placement, and it still refuses edits to signed PDFs.

Focused tests check note save/reopen and unchanged icon rendering. The QEMU edit gate appends text to a note, captures `pdf-note-edited.ppm`, checks undo/redo, and reads the note back from the guest-saved PDF. No protected path or compatibility behavior changes.
