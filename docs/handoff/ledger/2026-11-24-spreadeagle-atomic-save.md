# Spreadeagle staged saves

Spreadeagle stages workbook, export, and recovery bytes in a new file beside the destination, syncs the complete staged file, then replaces the destination through Files' rename. Failed encoding or writing leaves the prior copy intact. On SexOS the app calls `sex_std::fs::rename`, since the pinned Hermit `std::fs::rename` is unsupported. Files commits the replacement atomically; host tests use host rename.

Scope: `apps/spreadeagle` only. The existing GitHub CI host tests and QEMU gate exercise save/reopen and capture screenshots. No local full build or QEMU run; no kernel, Wi-Fi driver, or tether changes.
