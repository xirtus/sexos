# Vellum markup creation undo

- Added a bounded history for newly created annotations. Ctrl+Z removes the exact PDF object created by a highlight, note, or typed visual signature; Ctrl+Shift+Z recreates it with its appearance. The history is cleared on save, form edit, or deletion of an existing annotation.
- A host test exercises a real PDF through add, undo, reopen, redo, reopen, and redo invalidation. The Programs QEMU gate undoes and redoes a note and captures `pdf-note-undone.ppm` before continuing its form and save checks.
- This slice does not undo deletion of an existing annotation, and it does not persist unsaved markup through a power cut.
- Validation: the focused host test passed in a temporary harness that compiled the actual history module against `vellum-edit`; its target was removed. Python syntax and `git diff --check` passed. Full image build and QEMU execution are on GitHub CI only.
