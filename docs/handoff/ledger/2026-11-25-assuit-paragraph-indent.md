# Assuit paragraph indents

- **Gap:** Assuit could change line and paragraph spacing, but left, right, first-line and hanging indents had no native control.
- **Change:** Ctrl+Alt+I sets three bounded inch values on the current or selected paragraphs. A negative relative first-line value creates a hanging indent. WordCraft owns layout, undo and serialization.
- **Evidence:** A focused host test checks input rejection, rendering, one-step undo and DOCX/ODT round trips. The Assuit guest gate captures the prompt and applied page; full build and QEMU run in GitHub CI.
- **Limit:** The prompt uses numeric values; an on-page ruler remains open.
