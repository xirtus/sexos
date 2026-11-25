# Vellum gap list

Baseline: a daily PDF reader and editor such as Preview or Acrobat. Checked against `apps/pdf` and `crates/vellum-edit` at `d48bcc919`.

- **Already present:** native page rendering, zoom and thumbnails, full-document search, outline navigation, highlights and notes, AcroForm text/checkbox filling, incremental PDF updates, and signature field inspection. CI has host tests and QEMU viewer/edit gates.
- **Save safety:** edited PDF saves overwrite the destination directly; thumbnail output can delete an existing destination on a partial write. First slice: stage and sync complete bytes, then replace through Files' rename.
- **Signing:** Ctrl+Shift+G now places an explicitly labelled typed visual signature. Vellum can inspect stored signature fields, but still cannot create or cryptographically verify a CMS/PDF digital signature.
- **Editing depth:** annotation tools need more types, precise pointer selection, appearance controls, and undo/redo for editing sessions.
- **Performance and polish:** document the render latency target with a fixture, add a shortcut reference, follow light/dark Settings themes, and prove crash recovery on interrupted saves.

Each remaining feature needs a focused host test and CI guest evidence before being called complete.
