# Vellum unsaved PDF recovery

- Vellum keeps a complete incremental PDF of unsaved committed markup and form edits every three seconds when the bytes change. It stages, syncs and replaces the recovery copy through Files; the source PDF remains intact until Ctrl+S.
- On reopen, it restores a recovery copy only when it is a parseable PDF whose bytes start with the exact current source. A recovered copy remains pending until Ctrl+S commits it to the source, then the copy is removed.
- An exact `@/file.pdf` grant stores the copy in Vellum's own folder under a source-path hash, avoiding a write beside the granted file. The document path remains the source path in the UI.
- A focused host test checks source-prefix rejection and path isolation. The Programs QEMU gate kills the guest after an unsaved note is kept, reboots the same disk, verifies the restored annotation, captures before/after screenshots and commits it.
- Limitation: text still being typed into a note or field is not committed until Enter or a field switch. The proof covers a power cut after a completed recovery write, not physical media flush ordering.
- Validation: the focused recovery module test passed with `rustc --test` and its temporary binary was removed; Python syntax and `git diff --check` passed. Full image build and QEMU gate run only on GitHub CI. No local full build or QEMU run.
