# Assuit table-style gallery

- Added a native table-style gallery reached by Ctrl+Shift+T, `style`, Enter. It lists actual table styles in the open DOCX and samples their inherited header, band, body and border formatting.
- Up/Down, PageUp/PageDown, Enter and a pointer click choose a style through Assuit's existing undoable named-style command. The ODT refusal remains because the writer does not preserve named styles.
- Added a focused DOCX save/reopen and ODT refusal host test. The Programs QEMU gate checks gallery opening and application and captures `assuit-table-gallery.ppm`.
- Full build and QEMU screenshot verification run on GitHub CI. The sample is a compact colour and border preview, not a complete rendering of all table-style options.
