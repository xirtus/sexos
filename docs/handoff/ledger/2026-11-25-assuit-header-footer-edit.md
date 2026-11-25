# Assuit header and footer editing

- **Gap:** The page-number controls inserted fields, but a writer could not add rich text around them in the native window.
- **Change:** Ctrl+Alt+H/F enters the header/footer visible on the caret's page. Esc returns to the body. WordCraft owns the story, layout, undo and DOCX/ODT serialization; Assuit tracks nested creation/removal revisions for autosave.
- **Evidence:** A focused host test edits both regions, verifies a dynamic PAGE field remains, checks rendered change and saves/reopens DOCX and ODT. `program_assuit_gate.py` captures guest screenshots before and after footer editing and a header editing screenshot; full build and QEMU run on GitHub CI.
- **Limit:** Complex per-section header/footer management and exact power-cut timing during Files promotion remain open.
