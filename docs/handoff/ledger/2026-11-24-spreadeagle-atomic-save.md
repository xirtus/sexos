# Spreadeagle staged saves

Spreadeagle stages workbook, export, and recovery bytes in a new file, syncs the complete staged file, then replaces the destination through Files' rename. Ordinary paths stage beside the destination; an exact `@/file.xlsx` grant stages in Spreadeagle's own folder, because it cannot create a temporary sibling. Recovery for an exact grant uses a stable path hash in that folder. Exports from exact grants go to the app folder. Failed encoding or writing leaves the prior copy intact. On SexOS the app calls `sex_std::fs::rename`, since the pinned Hermit `std::fs::rename` is unsupported. Files commits same-volume replacements atomically; a cross-volume exact grant needs a future Files operation. Host tests use host rename.

Scope: `apps/spreadeagle` only. The existing GitHub CI host tests and QEMU gate exercise save/reopen and capture screenshots. No local full build or QEMU run; no kernel, Wi-Fi driver, or tether changes.
