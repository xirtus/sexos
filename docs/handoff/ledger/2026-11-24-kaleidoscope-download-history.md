# Kaleidoscope completed download history

- The Ctrl+J panel now reloads up to 128 completed public downloads from `Kaleidoscope/downloads.tsv` on startup, including source URL and byte count. It checks whether each file is still in Downloads before linking it. Private-tab downloads remain visible in the current window but never enter the persisted history.
- A versioned, bounded record format hex-encodes names and URLs so a site cannot forge extra rows. The existing staged Files replacement keeps a previous copy; malformed live records fall back to it. Host tests cover round-trip, bounds, private exclusion and malformed records.
- The downloads QEMU gate closes and reopens Kaleidoscope after four transfers, asserts the panel's restored count, and captures `browser-download-history-reopened.ppm`. Full build and QEMU run only in GitHub CI.
