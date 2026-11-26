# Assuit note conversion

Assuit now exposes whole-document footnote/endnote conversion through
Ctrl+Alt+Shift+N. Enter `endnote`, `footnote`, or `swap`. The action preserves
linked note text and is a single undoable edit. It refuses invalid choices and
conversions with no source notes.

Verification: focused host test for both DOCX and ODT save/reopen, linked note
text, all three conversion choices, one-step undo, and no-op refusal. The CI
QEMU gate captures the prompt and converted document after converting a guest
footnote to an endnote and saving it. Full build and QEMU run only on GitHub CI.
