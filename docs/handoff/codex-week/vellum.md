# Vellum gap list

Baseline: a daily PDF reader and editor such as Preview or Acrobat. Checked against `apps/pdf` and `crates/vellum-edit` at `d48bcc919`.

- **Already present:** native page rendering, zoom and thumbnails, full-document search, outline navigation, highlights and notes, AcroForm text/checkbox filling, incremental PDF updates, and signature field inspection. CI has host tests and QEMU viewer/edit gates.
- **Choice fields:** the viewer now includes noneditable combo boxes and single-select lists in form focus order. Up/Down chooses a displayed option, Enter commits its PDF export value, and incremental save/reopen keeps it. Editable combos and multi-select lists still need distinct controls.
- **Advanced choices:** editable combo boxes now accept custom text or an Up/Down option (Ctrl+A clears the current text). Multi-select lists show each option's selected state; Up/Down moves and Space toggles it. Both use the editor's PDF export values and incremental save path. A real-PDF host test and guest save/reopen gate cover both; richer pointer controls remain open.
- **Choice pointer menu:** the next slice gives a focused choice field a bounded pointer menu in Vellum's own chrome. A click on its value opens the options, a click chooses or toggles the displayed export value, and the wheel reaches longer lists. PDF widget hit testing and other pointer form controls remain separate gaps.
- **Widget pointer focus:** PDF form widgets still require Ctrl+E and keyboard field traversal before editing. The next slice maps a click through the current page zoom, scroll, crop and rotation to the editable widget, focuses it, opens a choice menu where appropriate, and toggles a checkbox directly. It keeps read-only/password widgets inaccessible. Drag selection and richer text controls remain open.
- **Form text caret:** text fields and editable combo boxes now show a caret in Vellum's form bar. Left/Right and Home/End move it; Backspace/Delete and typing edit at that position, including Unicode character boundaries.
- **Form text selection:** Shift+Left/Right and Shift+Home/End extend a text selection. Ctrl+A selects an entire text field; typing or deletion replaces the selected span. Vellum highlights the range in its form bar. Pointer drag now selects a text-field span in the bar and typing replaces it. Editable-combo drag selection remains open.
- **Form text pointer caret:** a click on the focused text field's value in the form bar places the caret at the nearest character boundary and clears a keyboard selection. The guest gate inserts text at that position. Drag selection and editable-combo text pointer placement remain open.
- **Annotation region drag:** after Ctrl+R enters region mode, dragging on the rendered page defines a bounded rectangle in shown page coordinates. Enter turns it into a PDF highlight through the existing edit/save path. The QEMU gate captures the dragged outline before the highlight and checks the saved annotation count. Direct drag initiation without choosing the region tool and richer markup appearance controls remain open.
- **Highlight colors:** Ctrl+Shift+H cycles yellow, green, pink, and blue before Ctrl+H or Enter marks found text or a selected region. The color is written into the PDF annotation and its appearance; the QEMU gate captures a green highlight and checks its color after guest save. Other markup appearance controls remain open.
- **Highlight opacity:** Ctrl+Shift+O cycles 100%, 75%, 50%, and 25% before placing a highlight. The PDF annotation stores `/CA` and the generated appearance applies the same alpha. A focused test compares opaque and translucent renders; the QEMU gate captures a green 75% highlight and checks its guest-saved opacity. Other markup appearance controls remain open.
- **Highlight comments:** Ctrl+L lists annotations; Enter on a highlight edits its comment in the top bar, with Ctrl+Z/Ctrl+Shift+Z before save. The edit updates only `/Contents`, retaining the highlight's geometry and appearance. A host round trip and QEMU save/reopen gate check the text; comments on other annotation kinds and richer metadata editing remain open.
- **Search across layout breaks:** Find currently treats extracted line breaks as
  literal characters, so a phrase typed with a space misses words split across
  two visible lines. This slice makes whitespace runs match one query space and
  highlights the exact source character span. Hyphenated word joins and
  multi-column reading order remain separate gaps.
- **Save safety:** edited PDF saves overwrite the destination directly; thumbnail output can delete an existing destination on a partial write. First slice: stage and sync complete bytes, then replace through Files' rename.
  Form and markup saves now require Files' final sync after the staged rename;
  a failed sync retains unsaved edits and the recovery copy and reports the
  save as unconfirmed instead of falsely clearing the form's edit state.
- **Signing:** Ctrl+Shift+G places an explicitly labelled typed visual signature. Ctrl+Shift+V inspects stored digital signature fields and checks exact covered bytes for the supported P-256/SHA-256 detached CMS shape. Ctrl+Shift+B selects a saved empty field and requires Enter in Vellum plus per-request Y approval in the separate Shibari window; Escape also cancels while waiting for approval. Shibari imports a public DER certificate, accepts a matching private scalar only in its own protection view, and returns bounded DER signatures through Shell; Vellum constructs CMS and verifies content before and after saving. The QEMU gate signs a PDF in the guest and an independent OpenSSL host check validates its saved bytes. Certificate-chain trust, signer identity, other CMS algorithms, DocMDP/FieldMDP permissions, and multiple signatures still need implementation. Vellum refuses an already signed PDF until those permission rules exist.
- **Signed markup guard:** annotation add, remove, and restore now refuse edits on any signed PDF until DocMDP/FieldMDP policy is evaluated. The editor leaves the PDF unchanged; the QEMU gate attempts a highlight after signing and captures the refusal while signature content stays intact. This is a fail-closed guard, not permission support.
- **Signature field placement:** Ctrl+Shift+D places a visible, empty PDF signature widget at the selected region (or a default location). It persists through incremental save and can be signed after reopening. Adding a field to an already signed PDF is refused until DocMDP and FieldMDP permissions can be evaluated.
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
