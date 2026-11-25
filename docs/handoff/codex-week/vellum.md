# Vellum gap list

Baseline: a daily PDF reader and editor such as Preview or Acrobat. Checked against `apps/pdf` and `crates/vellum-edit` at `d48bcc919`.

- **Already present:** native page rendering, zoom and thumbnails, full-document search, outline navigation, highlights and notes, AcroForm text/checkbox filling, incremental PDF updates, and signature field inspection. CI has host tests and QEMU viewer/edit gates.
- **Save safety:** edited PDF saves overwrite the destination directly; thumbnail output can delete an existing destination on a partial write. First slice: stage and sync complete bytes, then replace through Files' rename.
- **Signing:** Ctrl+Shift+G places an explicitly labelled typed visual signature. Ctrl+Shift+V inspects stored digital signature fields, flags malformed byte ranges, and checks exact covered bytes for the supported P-256/SHA-256 detached CMS shape. Vellum reports intact, changed, or unsupported content separately from unverified signer trust. The editing engine can append a detached-CMS signature slot to an existing empty signature field and hand the exact covered byte slices to a signer callback. Shibari has a bounded P-256/SHA-256 signing operation that returns only a DER signature. The CMS builder checks the certificate's P-256 public point against Shibari's, builds signed attributes and SignedData, and OpenSSL verifies the host fixture. These are prerequisites, not a complete PDF signing feature: no user certificate/key import, booted Shibari PDX signing service, certificate-chain validation, or signature UI is present. Other CMS algorithms/layouts are unsupported. PDF permission transforms and field locking also need policy checks before exposing signing.
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
