# Kaleidoscope: remove one completed download from history

- The downloads panel gives each completed row a Remove from history link. It updates the bounded app-owned history durably and retires the old recovery copy, then refreshes the panel. The saved file remains in Files.
- The action is accepted only while the top-level downloads panel is open; a site or embedded frame cannot clear a row by navigating to the internal action address.
- The GitHub CI downloads guest gate removes one row, captures `browser-download-item-forgotten.ppm`, checks the file remains in Downloads, and reopens the browser to verify the row stays absent. Full build and QEMU run only in GitHub CI.
