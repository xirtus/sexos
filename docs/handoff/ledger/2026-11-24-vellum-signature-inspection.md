# Vellum signature inspection

- Added Ctrl+Shift+V to show the count of stored PDF digital signature fields and report malformed signed byte ranges. The UI explicitly says cryptographic content and identity remain unverified. A typed visual signature remains clearly labelled as markup.
- The parser now requires exactly two non-overlapping, in-bounds signed byte spans starting at zero. Host tests check the existing fixture and a tampered range. The Documents QEMU gate captures `pdf-signature-inspection.ppm` for the unsigned form fixture.
- This is a structural inspection step. Signing and CMS/X.509 verification still require certificate and private-key handling through Shibari plus a CMS implementation; no trust claim is made from PDF metadata alone.
- Validation: focused host tests and Python syntax check locally; full image build and QEMU only in GitHub CI. No target or disk image is kept locally.
