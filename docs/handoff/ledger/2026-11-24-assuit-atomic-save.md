# Assuit staged saves

Assuit stages document, PDF, and recovery bytes in a new file, syncs the staged file, and then replaces the destination through Files' rename operation. Ordinary paths stage beside the destination; an exact `@/file.docx` grant stages in Assuit's own folder, because the grant does not authorize a temporary sibling. Such a file's recovery copy also lives in Assuit's folder under a stable path hash. This preserves the last saved document if encoding or writing fails before replacement. A cross-volume exact grant needs a future Files operation for atomic replacement.

Scope: `apps/assuit` only. No kernel, driver, or tether changes. The app uses `sex_std::fs::rename` on SexOS because the pinned Hermit `std::fs::rename` is unsupported; host tests use the host rename. Files' own rename is atomic at its next commit.

Evidence required: the Assuit host tests and full image/program QEMU gates on GitHub CI, including the Assuit screenshots from `program_assuit_gate.py`. CI result and artifact link belong in the PR.
