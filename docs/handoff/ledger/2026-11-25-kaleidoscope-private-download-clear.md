# Kaleidoscope: private download-history clear

- Ctrl+Alt+J in a private tab or standalone private window clears only the current window's private completed rows and failed transfers. It leaves ordinary saved download history, resumable transfers, and downloaded files untouched.
- The private panel offers per-row removal only for private rows. A typed action targeting an ordinary row from a private tab is rejected without a Files state write.
- A standalone private window does not load the ordinary profile. The previous clear path replaced its saved `downloads.tsv` with an empty file, erasing ordinary history. The private path now makes no Files state write.
- The GitHub CI downloads guest gate clears a mixed normal/private tab and a standalone private window, captures `browser-download-private-cleared.ppm`, and reopens the ordinary browser to check its four saved rows. Full builds and QEMU run in GitHub CI.
