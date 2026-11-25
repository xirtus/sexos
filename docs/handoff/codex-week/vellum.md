# Vellum gap list

Baseline: a daily PDF reader and editor such as Preview or Acrobat. Checked against `apps/pdf` and `crates/vellum-edit` at `d48bcc919`.

- **Already present:** native page rendering, zoom and thumbnails, full-document search, outline navigation, highlights and notes, AcroForm text/checkbox filling, incremental PDF updates, and signature field inspection. CI has host tests and QEMU viewer/edit gates.
- **Save safety:** edited PDF saves overwrite the destination directly; thumbnail output can delete an existing destination on a partial write. First slice: stage and sync complete bytes, then replace through Files' rename.
- **Signing:** Ctrl+Shift+G places an explicitly labelled typed visual signature. Ctrl+Shift+V inspects stored digital signature fields and flags malformed byte ranges, while explicitly saying the cryptographic content and signer are unverified. Vellum still cannot create or cryptographically verify a CMS/PDF digital signature; this needs a certificate/key path through Shibari and a CMS implementation.
- **Editing depth:** committed form-field edits and added or deleted highlights,
  notes, and visual signatures now have bounded Ctrl+Z / Ctrl+Shift+Z histories until
  save. A deleted annotation restores its original PDF object and appearance.
  Precise pointer selection and appearance controls remain.
- **Performance and polish:** the viewer chrome now follows Settings' light,
  dark and high-contrast colors. This slice retains up to two retired rendered
  pages under a 48 MiB cache cap, so a same-zoom revisit avoids Hayro rendering.
  Document the cold-render latency target with a fixture. Vellum now keeps
  complete unsaved markup/form edits in a staged recovery PDF and checks the
  source prefix before restoring it after a crash. The QEMU gate cuts power,
  reboots the same disk and commits a restored note. Ctrl+Shift+/ opens an in-app shortcut
  reference; `docs/apps/vellum-shortcuts.md` lists the active keys.
  Thumbnail pairs now share Hayro's glyph/interpreter cache within one panel
  draw; the QEMU fixture records cold-batch milliseconds without a runner-speed
  threshold. A representative cold full-page latency target remains open.

Each remaining feature needs a focused host test and CI guest evidence before being called complete.
