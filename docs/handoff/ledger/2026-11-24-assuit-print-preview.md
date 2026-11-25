# Assuit print preview

- Ctrl+Alt+P opens a read-only, whole-page view using WordCraft's rendered page. Left/Right, Up/Down, PageUp/PageDown and the wheel navigate; Ctrl+P exports PDF, Ctrl+Shift+P accepts a page range, and Esc returns to editing without moving the caret.
- The in-app and repository shortcut references document the preview. The QEMU Assuit gate opens a two-page document in preview, checks page navigation and return to editing, and captures `assuit-print-preview-page2.ppm`.
- Full image build and QEMU validation run on GitHub CI. Local checks cover host Assuit tests, Rust syntax and the Python gate syntax.
