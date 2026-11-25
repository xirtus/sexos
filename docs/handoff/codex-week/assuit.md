# Assuit gap list

Baseline: a daily word processor such as LibreOffice Writer or Pages. Checked against `apps/assuit` at `aea4ba33a`.

- **Already present:** DOCX and ODT open/save, page layout, paragraph styles, find/replace, undo/redo, PDF export, and recovery copies. Ctrl+C/X/V publish and read text through SexOS's focused-window clipboard; the QEMU gate transfers a selected word into Jot and back. WordCraft retains rich formatting when the shared text still matches its own copy.
- **Save safety:** ordinary save, Save As, PDF export, and recovery copies stage complete bytes and replace through Files rename. A two-boot CI gate now cuts power as soon as the synced-stage / Files-promotion receipt appears during ordinary save, then reopens the actual document and accepts only the complete old or complete new version. Host serial observation cannot guarantee the cut landed inside Files' rename operation; deterministic mid-rename injection remains open.
  An idle recovery write that fails now stays pending and retries after five seconds without another keystroke. A failed close stays open with a visible warning until retry succeeds or Ctrl+Shift+Q explicitly discards the work. A host test blocks staged promotion, then reopens the retried recovery copy and its DOCX/ODT saves; the two-boot guest recovery gate captures the restored page.
- **Editing UI:** table insertion (Ctrl+T), table row/column selection, insertion and deletion, merge/split cells, cell shading, DOCX table borders and named table styles (Ctrl+Shift+T), image insertion (Ctrl+Shift+I), selected-picture alt text (Ctrl+Shift+R), proportional picture width (Ctrl+Shift+O), crop percentages (Ctrl+Shift+C), and inline/square/top-and-bottom picture text flow (Ctrl+Shift+W) are exposed. Structural edits, shading, alt text and picture formatting have DOCX/ODT round-trip tests. ODT table borders and named table styles are refused because WordCraft drops them on save. Entering `style` in the table edit prompt now opens a native visual gallery of the document's actual DOCX table styles, with header, band and border samples. Up/Down or a click chooses a style. More precise floating image position and tight/contour wrapping remain.
- **Repeating table headers:** Ctrl+Shift+T now accepts `header on` and
  `header off` for the row at the caret. WordCraft repeats consecutive leading
  header rows when a table spans pages. A host test checks the row property and
  DOCX/ODT persistence. Marking a non-leading row alone cannot make it repeat.
- **Paragraph spacing:** WordCraft stores line spacing and paragraph space
  before/after, but Assuit currently has no native control for these common
  document edits. This slice adds a bounded paragraph-spacing prompt and checks
  DOCX/ODT save and reopen. The prompt now also accepts `exact` and `minimum`
  line heights in points, with DOCX/ODT round trips and a guest screenshot gate.
- **Paragraph indents:** WordCraft stores left, right and relative first-line
  indents. Ctrl+Alt+I now opens a bounded inch prompt, including negative
  first-line values for hanging indents. One-step undo and DOCX/ODT persistence
  are covered by a host test; CI captures the guest prompt and result.
- **Footnotes:** Ctrl+Shift+N now inserts a linked note at the body caret
  through a bounded prompt. The note remains editable on the page; Esc returns
  to the body caret. Ctrl+Alt+E now inserts a linked editable endnote through
  the same bounded flow. DOCX/ODT round trips are covered. Native note
  placement and numbering controls remain. Ctrl+Alt+Shift+N now converts
  footnotes to endnotes, endnotes to footnotes, or swaps both kinds across the
  document. It keeps linked note text, supports one-step undo, and has DOCX/ODT
  round-trip coverage plus a guest screenshot gate.
  Ctrl+Alt+Shift+E now reopens a linked note from its body reference after a
  document is saved and reopened. Editing and Esc return have DOCX/ODT tests
  and a guest screenshot gate. Ctrl+Alt+Shift+F now sets document-wide note
  placement, number format, first number, and restart behavior through a
  bounded prompt. DOCX/ODT persistence and guest screenshots cover it; ODT
  placement choices that WordCraft cannot preserve are refused.
  Per-section note options and custom note marks remain.
- **Hyperlinks:** Ctrl+K now opens a native URL prompt for selected text or a
  new display label. Web and mail links are bounded, undoable, and covered by
  DOCX/ODT round trips. Ctrl+Alt+K now removes a link from selected text while
  keeping the words; undo and DOCX/ODT round trips are covered. Ctrl+K at a
  caret inside an existing single-paragraph link now selects its whole label
  and prefills the address, so its target or display text can be changed without
  manual reselection. Ctrl+Alt+K also removes a complete single-paragraph link
  from a caret inside its label, preserving the words and one-step undo; DOCX/ODT
  round trips are checked. An explicit selection spanning paragraphs now
  prefills Ctrl+K when every selected run has the same target, and can retarget
  or remove that link while keeping the words; DOCX/ODT round trips and undo
  are covered. Automatic caret expansion across paragraphs remains because
  WordCraft does not distinguish separate adjacent links with the same URL.
- **Table of contents:** Ctrl+Alt+T now inserts a generated contents table
  from heading styles (levels 1–9), refreshes it after edits, or removes it.
  A DOCX/ODT host test covers insert, update, removal and save/reopen.
- **Page numbering:** WordCraft can insert dynamic page fields in a header or
  footer, but Assuit has no native control for it. This slice exposes bounded
  top/bottom and left/center/right choices, checks the visible page and
  DOCX/ODT save/reopen. This slice adds safe removal of a generated page-number
  header/footer in a single-section document, refusing unrelated header/footer
  content. This slice also exposes a bounded start number or continuation for
  the section at the caret, preserved through DOCX/ODT. This slice adds decimal,
  Roman and letter number formats for the current section. This slice exposes
  different-first-page and different-odd/even header/footer options, with
  DOCX/ODT persistence and guest screenshots. First-page choice is per section;
  odd/even choice is document-wide. Rich header/footer editing remains: the
  page-number command can add a field. Ctrl+Alt+H/F now enters the header or
  footer shown on the caret's page for rich text edits around existing fields;
  Esc returns to the body. DOCX and ODT round trips cover the text and field.
  Ctrl+Alt+Shift+H now starts a distinct default header or footer in the
  section at the body caret, leaving earlier sections' text intact. DOCX/ODT
  round trips and one-step undo are covered. Other first/even-page variants,
  relinking, and richer per-section header/footer management remain.
- **List editing:** WordCraft can create bullet and numbered lists, but Assuit
  exposes no native way to choose numbered formats, remove a list, or restart
  numbering. This slice adds a bounded list prompt for those document edits and
  verifies DOCX/ODT round trips. The same prompt now applies nested levels 1–9
  to an existing list. A host test checks nested-level DOCX/ODT persistence,
  return to the outer level, and refusal outside a list. This slice adds direct
  Ctrl+Alt+Right/Left gestures for a list item at the caret, with bounded levels
  and DOCX/ODT persistence. Custom list definitions remain.
- **Writing aids:** Ctrl+Shift+Y selects the next issue and offers corrections. A adds the selected issue to Assuit's app-owned persistent dictionary. Ctrl+Shift+D lists that dictionary and adds or removes words with a durable save. Ctrl+Shift+K sets the document default and selected text's en-US, en-GB, en-AU or en-CA proofing language; DOCX also accepts `none` for text that should not be proofed. English dialect tags round-trip in DOCX and ODT; ODT refuses `none` because WordCraft cannot preserve it there. Ctrl+Shift+U shows or hides WordCraft's actual continuous spelling marks on the editable page without changing the document or undo history. The display choice persists across app restarts. Assuit now accepts non-English tags when a matching, readable Hunspell `.aff`/`.dic` pair is in its app-owned `assuit-dictionaries` folder; a host test verifies actual German spelling and DOCX/ODT persistence. Automatic language-pack download and bundled non-English dictionaries remain gaps.
- **Print workflow:** PDF export and a selected-page range are now available.
  Letter, Legal and A4 paper with portrait/landscape orientation are now available.
  Margin presets and custom top/bottom/left/right inches are now available.
  Ctrl+Alt+P now opens a separate read-only, whole-page print preview with page
  navigation and PDF export. It uses WordCraft's page renderer, the same page
  layout shown by the editor. Printer device output remains outside SexOS's
  PDF-only print path.
- **Polish:** the app now follows Settings' light, dark and high-contrast
  appearance for its page desk, status bar and style picker. A keyboard shortcut
  reference is available in the app and `docs/apps/assuit-shortcuts.md`. Verified
  recovery after an interrupted run is covered by a two-boot QEMU power-cut
  gate for idle unsaved edits. This proves the recovery copy survives an abrupt
  stop after Assuit reports it kept the edit. A second two-boot gate cuts near
  Files promotion of an ordinary save and checks that a complete document
  survives; exact mid-rename interruption remains unproved.

Each remaining feature should have a focused host test and a CI guest gate before it is claimed complete.
