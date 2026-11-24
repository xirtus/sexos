# Assuit staged saves

Assuit stages document, PDF, and recovery bytes in a new file in the same program folder, syncs the staged file, and then replaces the destination through Files' rename operation. This preserves the last saved document if encoding or writing fails before replacement. The application's existing recovery copy still holds unsaved edits.

Scope: `apps/assuit` only. No kernel, driver, or tether changes. The app uses `sex_std::fs::rename` on SexOS because the pinned Hermit `std::fs::rename` is unsupported; host tests use the host rename. Files' own rename is atomic at its next commit.

Evidence required: the Assuit host tests and full image/program QEMU gates on GitHub CI, including the Assuit screenshots from `program_assuit_gate.py`. CI result and artifact link belong in the PR.
