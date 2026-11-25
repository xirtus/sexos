# Assuit paragraph spacing

- **Gap:** Assuit could set page margins but could not change a paragraph's line
  multiple or space before/after, despite WordCraft storing those properties.
- **Change:** Ctrl+Alt+S opens a native prompt for three bounded values: line
  multiple (0.5–3), before points (0–72), and after points (0–72). The engine
  applies them to the current or selected paragraphs and supports undo. Invalid
  values are refused before any document command runs.
- **Verification:** Focused host test rejects bad values and checks DOCX/ODT
  save/reopen. CI's Assuit QEMU gate captures the prompt and edited page and
  requires the document edit record. Full build and QEMU run on GitHub CI.
- **Limit:** Exact and at-least line heights are not in this prompt.
