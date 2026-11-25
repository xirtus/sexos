# Assuit caret link removal

Assuit's Ctrl+Alt+K now removes the complete hyperlink when the caret is inside
its label. The words stay in place, and one undo restores the link. Selection-based
removal continues to work. The app's shortcut reference describes both gestures.

At a boundary between two different links, the caret chooses the link to its
right; a regression covers removal and editing there. The focused regression
checks DOCX and ODT save/reopen and undo/redo. Full build,
QEMU, and screenshot evidence are required from GitHub CI for this branch.

The link lookup is limited to one paragraph. Cross-paragraph links remain a gap.
