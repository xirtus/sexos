# Vellum signing source envelope — 2026-11-25

Vellum now sends a bounded source PDF path and signature-field name with each signing request. The native PDX route transmits these bytes before CMS attributes. Silk rejects missing or oversized lengths and out-of-order chunks; Shibari refuses incomplete or malformed metadata. Vellum uses its saved source path and selected field.

This is a protocol step toward independent document consent. Shibari still signs caller-supplied attributes after the existing approval key; it does not yet re-read the PDF or prove that those attributes came from the named document. The next change must use the native Files picker to give Shibari its own read grant, reproduce the attributes from the chosen PDF, compare them byte-for-byte, and refuse mismatch or source mutation.

Focused checks: `git diff --check`; `rustc --test crates/sex-pdx/src/pdf_sign.rs` (2 passed). Cross-crate compilation, full build, QEMU gate, and screenshot are required from exact-head GitHub CI.
