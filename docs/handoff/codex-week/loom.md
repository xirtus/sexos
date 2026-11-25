# Loom daily-driver gaps

Compared with Finder and Dolphin, Loom already has folder browsing, search,
thumbnail grids for pictures/PDF/RAW, archive browsing and extraction, batch
rename, trash and undo, and network places. The current quick-look gap is
reading a text file in place without launching an editor. F3 now opens a
read-only, bounded UTF-8 preview with line scrolling; the QEMU gate captures
the actual preview.
F3 now also shows a selected PNG, JPEG, GIF or WebP at window size inside Loom,
using the bounded native image decoder. Escape returns to the folder without
starting another app. Picture inputs over 16 MiB are refused.

Remaining gaps: multiple folder tabs, richer document previews, archive
creation beyond zip, drag selection and multi-item operations, and a full
keyboard shortcut reference. The preview shows the first 32 KiB only and
rejects binary or control-bearing files; Enter still opens the chosen app.
