# Assuit PDF page choices — 2026-11-25

Assuit's PDF range prompt accepted page numbers only. It now also accepts
`current`, `selection`, `odd`, and `even`; the engine chooses laid-out document
pages and refuses an empty result. Selected pages are only available with an
active text selection. Each choice gets its own PDF beside the document, leaving
the editing file and unsaved changes in place. The prompt, shortcut reference,
host test, and QEMU screenshot gate cover the new choices.

Branch: `codex/assuit-print-page-choices` (stacked on
`codex/assuit-print-preview`). No kernel, Wi-Fi, or tether code changed.

Verification: focused host test and guest target check locally; full build and
QEMU test on GitHub CI. Screenshot artifact:
`assuit-print-even-pages.ppm` from the Programs job.
