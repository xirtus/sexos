# Assuit picture width

- Added Ctrl+Shift+O for a selected picture's width in inches (0.1–20). WordCraft keeps the picture's proportions and stores the resulting size in the document.
- A focused host test checks bounds, aspect ratio and DOCX/ODT save/reopen persistence. The Programs QEMU gate resizes the picture and captures `assuit-picture-sized.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
