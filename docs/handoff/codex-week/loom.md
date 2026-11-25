# Loom daily-driver gaps

Compared with Finder and Dolphin, Loom already has folder browsing, search,
thumbnail grids for pictures/PDF/RAW, archive browsing and extraction, batch
rename, trash and undo, and network places. The current quick-look gap is
reading a text file in place without launching an editor. F3 now opens a
read-only, bounded UTF-8 preview with line scrolling; the QEMU gate captures
the actual preview.

Loom can now also create tar.gz archives from a selected file or folder using
Ctrl+Shift+Z, with the same staged output and undo as zip. A QEMU gate browses
the result and checks every file on the host.

Remaining gaps: multiple folder tabs, richer document previews, more archive
creation formats, drag selection and multi-item operations, and a full
keyboard shortcut reference. The preview shows the first 32 KiB only and
rejects binary or control-bearing files; Enter still opens the chosen app.
