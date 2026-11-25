# Spreadeagle ODS frozen panes (2026-11-25)

- **Change:** Preserve row and column freezes in ODS `settings.xml` and restore per-sheet view state on import. A movable split is never misread as frozen.
- **Format:** LibreOffice stores fixed panes in per-table `HorizontalSplitMode` / `VerticalSplitMode` value `2` and zero-based positions. The package manifest includes settings.xml. See [LibreOffice SheetSettings](https://docs.libreoffice.org/xmerge/html/SheetSettings_8java_source.html) and [Calc view settings](https://docs.libreoffice.org/sc/html/viewdata_8cxx_source.html). This is application view metadata within ODF settings, not core cell data.
- **Tests:** Host round trip saves, reopens and unfreezes an ODS workbook. A generated LibreOffice-format fixture checks that mode `1` is ignored. Full build, QEMU, and screenshot are GitHub CI gates; not run locally.
- **Limit:** External LibreOffice-produced fixture and visual interoperability proof remain pending. Other ODS view state is not yet preserved.
