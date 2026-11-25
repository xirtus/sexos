# Loom folder tabs

- Gap: Loom offered one folder view per window while Finder and Dolphin can keep several locations open for repeated work.
- Change: Ctrl+Shift+T opens a second tab at the current folder, Ctrl+Tab cycles, and Ctrl+W closes the current tab. Up to eight tabs keep independent folder, cursor and scroll positions. The native view shows the active tab. Tab switching waits until a picker, archive, preview, or file operation is finished.
- Tests: The `loom-core` tab model test checks independent positions, cycling, closing and the eight-tab bound. The Files QEMU gate opens a second tab, enters a folder, switches back and forth, captures `folder-tabs.ppm`, closes it, and continues its existing archive checks. Full builds and QEMU run only on GitHub CI.
