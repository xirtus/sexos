# Assuit picture alternative text

- Added Ctrl+Shift+R to set 1–500 characters of alternative text on the selected picture, using WordCraft's undoable picture command. The prompt explains its accessibility purpose.
- A focused host test checks the selected-picture requirement and DOCX/ODT save/reopen persistence. The Programs QEMU gate opens the prompt, captures `assuit-picture-alt-prompt.ppm`, and records successful entry.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
