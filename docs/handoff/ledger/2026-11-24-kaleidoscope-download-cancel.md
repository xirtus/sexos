# Kaleidoscope download cancellation

- Escape now cancels an active attachment transfer while the browser streams to Files. It removes the hidden partial file, leaves completed download history intact, and shows the canceled address with a retry link. Close is delivered after cleanup; other key and pointer input is held in a bounded queue for the browser event loop.
- The Browser QEMU gate serves a paced 8 MiB attachment, waits for progress, presses Escape, checks the partial file is gone, and captures `browser-download-cancelled.ppm`. Full image build and QEMU run on GitHub CI.
- Targeted `web` app check, Rust parser, gate syntax and diff checks run locally. The temporary target and image files are removed afterward.
