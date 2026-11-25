# Assuit: section running text

- **Change:** Ctrl+Alt+Shift+H starts an independent default header or footer in the section at the body caret with `header TEXT` or `footer TEXT`. Existing section stories are not overwritten. Ctrl+Alt+H/F remains the rich editor.
- **Why:** A document with sections needs different running text while retaining the previous section's header/footer. WordCraft's ordinary edit command follows inherited text; its `insert.header`/`insert.footer` command creates a new current-section story.
- **Scope:** Assuit only. DOCX and ODT host regression checks two sections, text persistence, refusal to replace an existing story, and one-step undo/redo. A guest prompt screenshot is requested in CI. No kernel, Wi-Fi driver, or tether change.
- **Verification:** `git diff --check` locally; focused host regression, full build, QEMU, and screenshot pending GitHub CI. No local full build or QEMU. A regular page in the section is required; first/even page and relink controls remain open.
