# Kaleidoscope failed download recovery

- A streamed response with a declared length must deliver every byte before its hidden `.part` file is promoted. The network flow can report peer closure as complete while short, so this check prevents a truncated file from appearing finished. Failed streams now clear the progress status, remove their partial file, and add a bounded failure row to Ctrl+J's downloads panel. The row shows a safely escaped reason and an exact-address **Try again** link. A successful retry removes the failure row and takes the original free file name.
- The browser download QEMU gate serves a truncated attachment, checks the failure and panel count, captures `browser-download-retry.ppm`, retries successfully, and captures `browser-download-recovered.ppm`.
- Validation: Python syntax and focused app compile locally; full image build and QEMU test only on GitHub CI. No target or disk image is kept locally.
