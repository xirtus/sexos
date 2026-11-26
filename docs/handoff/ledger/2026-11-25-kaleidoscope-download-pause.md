# Kaleidoscope live download pause and resume

- Ctrl+Shift+J pauses a live download by stopping Exchange receive calls and file writes. The status bar shows the committed offset; another Ctrl+Shift+J resumes the same connection at that offset. Escape still cancels and removes the hidden partial file.
- Exchange excludes the person's paused interval from its no-byte timeout. A bounded transfer state refuses writes while paused or beyond the maximum size.
- A focused host test checks exact offsets, pause, resume, and size limits. The Browser QEMU gate pauses a paced attachment, verifies no progress while paused, resumes it, hashes the complete saved file, and captures `browser-download-paused.ppm`. Full build and QEMU execution run on GitHub CI.
- This is a live connection pause. If the server closes while paused, Kaleidoscope offers a fresh retry; durable Range/ETag continuation remains open.
