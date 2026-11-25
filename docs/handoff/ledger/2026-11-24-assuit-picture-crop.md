# Assuit picture crop

- Added Ctrl+Shift+C to crop the selected image with left, top, right and bottom whole percentages. Input leaves at least five percent visible on each axis; `0 0 0 0` resets the crop. WordCraft applies this as an undoable command.
- A focused host test checks invalid percentages and DOCX/ODT save/reopen persistence. The Programs QEMU gate crops the picture and captures `assuit-picture-cropped.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
