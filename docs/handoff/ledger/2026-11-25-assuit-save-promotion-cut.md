# Assuit save promotion power-cut gate

- Assuit records a promotion-start receipt with byte count and FNV hash immediately after syncing a complete staged document and just before requesting Files rename, plus a success receipt after the rename replies. This adds observability without changing save semantics.
- The GitHub CI QEMU gate saves a baseline DOCX, edits it again, cuts guest power on the next promotion-start receipt, and reboots the same disk. Spindle reads and hashes the actual saved file, which must exactly match either complete staged version. Assuit then opens it, after discarding any unsaved recovery overlay, and its text must match that hash. The gate captures `assuit-promotion-reopened.ppm`.
- The gate targets the narrowest observable boundary without a Files failure hook. Serial polling and host termination cannot guarantee the cut falls inside the rename request. Deterministic interruption of Files' internal promotion remains an open test gap.
