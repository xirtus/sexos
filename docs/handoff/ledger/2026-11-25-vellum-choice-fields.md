# Vellum single-choice PDF forms

- Vellum's form focus order now includes noneditable AcroForm combo boxes and single-select list boxes with finite options. Up/Down previews their display labels; Enter or moving to the next field commits the export value through `vellum-edit` and its existing undo/save/recovery path. Read-only, editable combo and multi-select fields remain outside this selector.
- Added a real one-page PDF fixture with paired export/display values. Host tests read the actual PDF, select options, save incrementally, reopen and check the stored export values. The QEMU edit gate selects both kinds, captures `pdf-choice-fields.ppm`, confirms Files sync before save success, reopens and captures `pdf-choice-reopened.ppm` with both selections retained.
- Existing signature restrictions and confirmed PDF save path are unchanged. Full image build and QEMU run belong to GitHub CI.
