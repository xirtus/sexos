# Loom text preview

- Gap: Loom had thumbnails, but no quick text preview inside its file manager. Opening a note required launching another program.
- Change: F3 shows the selected UTF-8 file in a read-only preview with line numbers and Up/Down/Home/End scrolling. Loom requests desktop keys from Shell so F3 reaches the program. Input is limited to 32 KiB, refuses binary and display controls, and never opens an editor or changes the file. The UI uses the Loom name.
- Tests: `loom-core` host tests cover paging, truncation, tabs, UTF-8 rejection, and control rejection. The GitHub Files QEMU gate opens `alpha.txt` with F3, checks the guest marker, captures `text-preview.ppm`, and returns to the folder before its archive checks. Full builds and QEMU run only on GitHub CI.
