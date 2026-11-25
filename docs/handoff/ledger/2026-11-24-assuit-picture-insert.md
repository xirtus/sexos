# Assuit picture insertion

Ctrl+Shift+I now accepts a picture path in the program's granted folder. Assuit reads up to 16 MiB, gives the bytes to WordCraft's `insert.picture` command, and sets the image's alternative text to its filename. First-run `welcome.docx` also comes with a small `assuitmark.png` drawn from SexOS's own brand asset so the feature has a real QEMU fixture.

The host test inserts the mark and checks its media survives DOCX and ODT save/reopen. The Assuit CI QEMU gate inserts that same PNG and captures `assuit-picture.ppm`. Full build and guest evidence come from GitHub CI; no local build or QEMU run. No kernel, Wi-Fi driver, or tether changes.
