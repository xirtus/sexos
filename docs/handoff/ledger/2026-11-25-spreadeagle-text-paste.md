# Spreadeagle: system clipboard and guarded text paste

- Ctrl+C/X publishes tab-separated cell text to the SexOS system clipboard. Ctrl+V uses text from another app when it differs from Spreadeagle's own copy; Ctrl+Shift+V forces a plain-cell paste. Internal copy/paste continues to carry workbook formatting and formulas.
- Plain text paste uses the stop-rule preflight from the previous change, including multi-cell atomic rejection. A focused host test checks invalid and valid TSV blocks and unchanged undo history after rejection. The Programs QEMU gate checks system clipboard copy, invalid plain paste, accepted plain paste, and captures `spreadeagle-text-paste.ppm`.
- Other bulk commands still need validation coverage.
