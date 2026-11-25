# Vellum: find phrases across line breaks

- Find now collapses runs of PDF-extracted whitespace while matching. A phrase typed with one space reaches words on adjacent visible lines. Each hit retains its source span, so the viewer's search overlay, Ctrl+H highlight and Ctrl+N note placement use the exact extracted characters.
- Pure search tests cover case, overlapping matches, whitespace runs and source spans. The PDF text host test checks a phrase across the rendered fixture's line break. The QEMU viewer gate searches the same fixture, captures `pdf-find-across-lines.ppm` and checks the hit and painted window.
- Search still follows the PDF's extracted content order. It does not reflow columns or join words hyphenated at line ends. Full builds and QEMU execution run on GitHub CI.
