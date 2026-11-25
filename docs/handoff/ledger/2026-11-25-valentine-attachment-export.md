# Valentine attachment export — 2026-11-25

- Gap: Valentine's native local mail viewer showed attachment counts but could not save decoded MIME attachments. Live IMAP/SMTP still lacks a boot-routed Multipass service and TLS flow.
- Change: Ctrl+Alt+A cycles attachments in an opened local message; Ctrl+Alt+S asks Loom for an exact Save grant, creates without replacing an existing file, writes decoded bytes, syncs, and reports/cleans failures. The sender's MIME filename is only a sanitized picker suggestion.
- Evidence: focused host tests cover malicious MIME filenames, malformed picker paths, and the exact PDF fixture bytes. The Valentine QEMU gate saves that PDF through Loom, tries the same name again without replacing it, checks its bytes, and captures the picker and result screens. Full guest build/QEMU and screenshots remain pending exact-head GitHub CI.
- Limit: the 4 MiB local-message cap remains; a power cut mid-export may leave an incomplete target because the picked grant is exact-file only. This does not claim live mail sync, sending, or attachment opening.
