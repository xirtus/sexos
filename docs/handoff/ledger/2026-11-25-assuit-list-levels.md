# Assuit nested list levels

- The native Ctrl+Alt+L list prompt now accepts `level 1` through `level 9` for a current bullet or numbered list. It rejects out-of-range input and a paragraph outside a list. WordCraft changes the paragraph's actual list level, so the edit is undoable and saved with the document.
- A focused host test creates an outer item and a nested item, saves and reopens DOCX and ODT, verifies both list levels, and moves the nested item back to level 1.
- The Programs QEMU gate changes a second numbered item to level 2, captures `assuit-nested-list.ppm`, checks that the document view changed, and saves the DOCX. Full build and QEMU execution run on GitHub CI.
- Custom list definitions and dedicated indent/outdent gestures remain open.
