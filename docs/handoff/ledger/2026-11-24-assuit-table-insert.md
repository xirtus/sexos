# Assuit table insertion

The native Assuit page view now opens a rows × columns prompt with Ctrl+T. It calls WordCraft's existing `insert.table` command, leaves the caret in the first cell, and bounds dimensions to 20 rows and 10 columns so accidental input cannot create an enormous layout. The status line advertises the shortcut.

A host test inserts a 2 × 2 table, types into two cells, and reopens it after DOCX and ODT saves. The Assuit QEMU gate inserts a table and captures `assuit-table.ppm`; its existing screenshots and save/reopen checks remain. Full build and QEMU evidence come from GitHub CI. No kernel, Wi-Fi driver, or tether changes.
